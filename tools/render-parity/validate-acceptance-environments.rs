#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

//! Validate acceptance-repeat summaries collected from multiple runner environments.

use clap::Parser;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "validate-acceptance-environments",
    about = "Validate multi-environment render-parity acceptance-repeat summaries"
)]
struct Options {
    #[arg(long)]
    summary: Vec<PathBuf>,
    #[arg(long)]
    bundle: Vec<PathBuf>,
    #[arg(long, default_value_t = 2)]
    min_environments: usize,
    #[arg(long, default_value_t = 3)]
    min_runs_per_environment: u64,
    #[arg(long, default_value_t = 18)]
    expected_comparisons: usize,
    #[arg(long, default_value_t = 34.0)]
    min_psnr_floor: f64,
    #[arg(long, default_value_t = 0)]
    max_alpha_mismatches: u64,
    #[arg(long, default_value_t = 0)]
    max_alpha_delta: u64,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
    #[arg(long, hide = true)]
    self_test: bool,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    if options.self_test {
        return run_self_test();
    }
    if !options.summary.is_empty() && !options.bundle.is_empty() {
        return Err("do not mix --summary and --bundle inputs in one validation run".into());
    }
    let mut summaries = options
        .summary
        .iter()
        .map(|path| read_summary(path))
        .collect::<Result<Vec<_>, _>>()?;
    summaries.extend(
        options
            .bundle
            .iter()
            .map(|path| read_bundle_summary(path))
            .collect::<Result<Vec<_>, _>>()?,
    );
    if summaries.is_empty() {
        return Err("provide at least one --summary or --bundle".into());
    }
    let report = validate_summaries(&summaries, &options)?;
    if let Some(path) = options.json_out.as_ref() {
        write_json(path, &report)?;
    }
    if let Some(path) = options.markdown_out.as_ref() {
        write_text(path, &markdown_summary(&report)?)?;
    }
    println!(
        "validated {} acceptance environments",
        required_array(&report, "environments")?.len()
    );
    Ok(())
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let options = Options {
        summary: Vec::new(),
        bundle: Vec::new(),
        min_environments: 2,
        min_runs_per_environment: 3,
        expected_comparisons: 2,
        min_psnr_floor: 34.0,
        max_alpha_mismatches: 0,
        max_alpha_delta: 0,
        json_out: None,
        markdown_out: None,
        self_test: true,
    };
    let summaries = vec![
        test_summary("env-a", "Adapter A"),
        test_summary("env-b", "Adapter B"),
    ];
    let report = validate_summaries(&summaries, &options)?;
    let markdown = markdown_summary(&report)?;
    for needle in [
        "Multi-Environment Acceptance Summary",
        "distinct environments",
        "minimum selected PSNR",
        "Adapter A",
        "Adapter B",
    ] {
        if !markdown.contains(needle) {
            return Err(format!("self-test markdown missing {needle:?}").into());
        }
    }

    let duplicate = vec![
        test_summary("env-a", "Adapter A"),
        test_summary("env-a-copy", "Adapter A"),
    ];
    if validate_summaries(&duplicate, &options).is_ok() {
        return Err("duplicate environment locks should be rejected".into());
    }

    let mut failed = summaries.clone();
    failed[1]["comparisons"][0]["minSelectedPsnr"] = Value::from(33.9);
    if validate_summaries(&failed, &options).is_ok() {
        return Err("low environment PSNR should be rejected".into());
    }

    let mut infinity = summaries.clone();
    infinity[1]["comparisons"][1]["minSelectedPsnr"] = Value::String("Infinity".to_owned());
    validate_summaries(&infinity, &options)?;

    let mut source_mismatch = summaries;
    source_mismatch[1]["sourceLock"]["vrmRsGitHead"] = Value::String("different".to_owned());
    if validate_summaries(&source_mismatch, &options).is_ok() {
        return Err("source-lock mismatch should be rejected".into());
    }

    let bundle_root = PathBuf::from("target/acceptance-environments-self-test");
    let _ = fs::remove_dir_all(&bundle_root);
    write_test_bundle(
        &bundle_root.join("env-a"),
        &test_summary("env-a", "Adapter A"),
    )?;
    write_test_bundle(
        &bundle_root.join("env-b"),
        &test_summary("env-b", "Adapter B"),
    )?;
    let bundle_options = Options {
        bundle: vec![bundle_root.join("env-a"), bundle_root.join("env-b")],
        ..options.clone()
    };
    let bundle_summaries = bundle_options
        .bundle
        .iter()
        .map(|path| read_bundle_summary(path))
        .collect::<Result<Vec<_>, _>>()?;
    validate_summaries(&bundle_summaries, &bundle_options)?;
    run(Options {
        self_test: false,
        ..bundle_options
    })?;

    let mixed_options = Options {
        summary: vec![bundle_root
            .join("env-a")
            .join("acceptance-repeat-summary.json")],
        bundle: vec![bundle_root.join("env-b")],
        self_test: false,
        ..options.clone()
    };
    if run(mixed_options).is_ok() {
        return Err("mixed summary and bundle inputs should be rejected".into());
    }

    let mut bad_manifest = read_json(&bundle_root.join("env-a").join("bundle-manifest.json"))?;
    bad_manifest["runCount"] = Value::from(2);
    fs::write(
        bundle_root.join("env-a").join("bundle-manifest.json"),
        serde_json::to_string(&bad_manifest)?,
    )?;
    if read_bundle_summary(&bundle_root.join("env-a")).is_ok() {
        return Err("bundle manifest mismatch should be rejected".into());
    }

    let mut bad_metric_manifest =
        read_json(&bundle_root.join("env-b").join("bundle-manifest.json"))?;
    bad_metric_manifest["maxAlphaDelta"] = Value::from(1);
    fs::write(
        bundle_root.join("env-b").join("bundle-manifest.json"),
        serde_json::to_string(&bad_metric_manifest)?,
    )?;
    if read_bundle_summary(&bundle_root.join("env-b")).is_ok() {
        return Err("bundle metric mismatch should be rejected".into());
    }

    write_test_bundle(
        &bundle_root.join("env-c"),
        &test_summary("env-c", "Adapter C"),
    )?;
    let mut bad_path_manifest = read_json(&bundle_root.join("env-c").join("bundle-manifest.json"))?;
    bad_path_manifest["files"][0]["path"] = Value::String("../escape.json".to_owned());
    fs::write(
        bundle_root.join("env-c").join("bundle-manifest.json"),
        serde_json::to_string(&bad_path_manifest)?,
    )?;
    if read_bundle_summary(&bundle_root.join("env-c")).is_ok() {
        return Err("bundle path traversal should be rejected".into());
    }

    Ok(())
}

fn read_summary(path: &Path) -> Result<Value, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let mut summary = serde_json::from_str::<Value>(&text)?;
    summary["_summaryPath"] = Value::String(display_path(path));
    Ok(summary)
}

fn read_bundle_summary(bundle_dir: &Path) -> Result<Value, Box<dyn Error>> {
    let manifest_path = bundle_dir.join("bundle-manifest.json");
    let manifest = read_json(&manifest_path)?;
    if required_string(&manifest, "bundleFormat")? != "vrm-rs.render-parity.acceptance-evidence.v1"
    {
        return Err("bundle-manifest.json has an unsupported bundleFormat".into());
    }
    let summary_path = bundle_dir.join("acceptance-repeat-summary.json");
    let mut summary = read_summary(&summary_path)?;
    summary["_bundlePath"] = Value::String(display_path(bundle_dir));

    ensure_same_value(
        required_object_value(&manifest, "sourceLock", "bundle-manifest")?,
        required_object_value(&summary, "sourceLock", "summary")?,
        "bundle.sourceLock",
    )?;
    ensure_same_value(
        required_object_value(&manifest, "environmentLock", "bundle-manifest")?,
        required_object_value(&summary, "environmentLock", "summary")?,
        "bundle.environmentLock",
    )?;
    if required_u64(&manifest, "runCount")? != required_u64(&summary, "runCount")? {
        return Err("bundle runCount does not match summary runCount".into());
    }
    if required_u64(&manifest, "comparisonCount")? as usize
        != required_array(&summary, "comparisons")?.len()
    {
        return Err("bundle comparisonCount does not match summary comparisons".into());
    }
    let expected_min_psnr = metric_json(min_comparison_f64(&summary, "minSelectedPsnr")?);
    ensure_same_value(
        &expected_min_psnr,
        manifest
            .get("minSelectedPsnr")
            .ok_or("bundle-manifest.minSelectedPsnr is missing")?,
        "bundle.minSelectedPsnr",
    )?;
    if required_u64(&manifest, "maxAlphaMismatches")?
        != max_comparison_u64(&summary, "maxAlphaMismatches")?
    {
        return Err("bundle maxAlphaMismatches does not match summary comparisons".into());
    }
    if required_u64(&manifest, "maxAlphaDelta")? != max_comparison_u64(&summary, "maxAlphaDelta")? {
        return Err("bundle maxAlphaDelta does not match summary comparisons".into());
    }
    validate_bundle_files(bundle_dir, &manifest, required_u64(&summary, "runCount")?)?;
    Ok(summary)
}

fn validate_bundle_files(
    bundle_dir: &Path,
    manifest: &Value,
    run_count: u64,
) -> Result<(), Box<dyn Error>> {
    let mut has_summary = false;
    let mut listed_files = BTreeSet::new();
    let canonical_bundle_dir = bundle_dir.canonicalize()?;
    for file in required_array(manifest, "files")? {
        let relative = required_string(file, "path")?;
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(format!(
                "bundle file path must be relative and stay inside the bundle: {relative:?}"
            )
            .into());
        }
        if relative == "acceptance-repeat-summary.json" {
            has_summary = true;
        }
        listed_files.insert(relative.replace('\\', "/"));
        let path = bundle_dir.join(relative_path);
        let canonical_path = path
            .canonicalize()
            .map_err(|err| format!("bundle file is missing: {}: {err}", display_path(&path)))?;
        if !canonical_path.starts_with(&canonical_bundle_dir) {
            return Err(format!(
                "bundle file resolves outside the bundle: {}",
                display_path(&path)
            )
            .into());
        }
        let bytes = path
            .metadata()
            .map_err(|err| format!("bundle file is missing: {}: {err}", display_path(&path)))?
            .len();
        let expected_bytes = required_u64(file, "bytes")?;
        if bytes != expected_bytes {
            return Err(format!(
                "bundle file {} has {bytes} bytes, expected {expected_bytes}",
                display_path(&path)
            )
            .into());
        }
    }
    if !has_summary {
        return Err("bundle manifest must list acceptance-repeat-summary.json".into());
    }
    for required in required_bundle_files(run_count) {
        if !listed_files.contains(&required) {
            return Err(format!("bundle manifest must list required file {required}").into());
        }
    }
    Ok(())
}

fn required_bundle_files(run_count: u64) -> Vec<String> {
    let mut paths = vec![
        "acceptance-repeat-summary.json".to_owned(),
        "acceptance-repeat-summary.md".to_owned(),
    ];
    paths.extend((1..=run_count).flat_map(|run| {
        [
            format!("run-{run}/review-manifest.json"),
            format!("run-{run}/summary.md"),
        ]
    }));
    paths
}

fn validate_summaries(summaries: &[Value], options: &Options) -> Result<Value, Box<dyn Error>> {
    if summaries.len() < options.min_environments {
        return Err(format!(
            "expected at least {} acceptance-repeat summaries, got {}",
            options.min_environments,
            summaries.len()
        )
        .into());
    }

    let baseline = summaries
        .first()
        .ok_or("expected at least one acceptance-repeat summary")?;
    validate_summary_shape(baseline, options)?;
    let baseline_source_lock = required_object_value(baseline, "sourceLock", "summary")?.clone();
    let baseline_lane_config = required_object_value(baseline, "laneConfig", "summary")?.clone();
    let baseline_fixtures = required_array_value(baseline, "fixtures")?.clone();
    let baseline_comparison_names = comparison_names(baseline)?;

    let mut environments = Vec::with_capacity(summaries.len());
    let mut environment_signatures = BTreeSet::new();
    let mut comparison_aggregates = BTreeMap::<String, MultiEnvAggregate>::new();

    for (index, summary) in summaries.iter().enumerate() {
        validate_summary_shape(summary, options)?;
        ensure_same_value(
            &baseline_source_lock,
            required_object_value(summary, "sourceLock", "summary")?,
            &format!("summary[{index}].sourceLock"),
        )?;
        ensure_same_value(
            &baseline_lane_config,
            required_object_value(summary, "laneConfig", "summary")?,
            &format!("summary[{index}].laneConfig"),
        )?;
        ensure_same_value(
            &baseline_fixtures,
            required_array_value(summary, "fixtures")?,
            &format!("summary[{index}].fixtures"),
        )?;
        ensure_same_value(
            &baseline_comparison_names,
            &comparison_names(summary)?,
            &format!("summary[{index}].comparisons"),
        )?;

        let environment_lock = required_object_value(summary, "environmentLock", "summary")?;
        let signature = environment_signature(environment_lock)?;
        environment_signatures.insert(signature.clone());
        let min_psnr = min_comparison_f64(summary, "minSelectedPsnr")?;
        let max_alpha_mismatches = max_comparison_u64(summary, "maxAlphaMismatches")?;
        let max_alpha_delta = max_comparison_u64(summary, "maxAlphaDelta")?;
        environments.push(serde_json::json!({
            "summary": summary.get("_summaryPath").and_then(Value::as_str).unwrap_or("<in-memory>"),
            "environmentLock": environment_lock,
            "environmentSignature": signature,
            "runCount": required_u64(summary, "runCount")?,
            "minSelectedPsnr": metric_json(min_psnr),
            "maxAlphaMismatches": max_alpha_mismatches,
            "maxAlphaDelta": max_alpha_delta,
        }));
        collect_comparison_aggregates(summary, &mut comparison_aggregates)?;
    }

    if environment_signatures.len() < options.min_environments {
        return Err(format!(
            "expected at least {} distinct environment locks, got {}",
            options.min_environments,
            environment_signatures.len()
        )
        .into());
    }
    if environment_signatures.len() != summaries.len() {
        return Err(
            "each acceptance environment summary must have a distinct GPU/driver environment signature"
                .into(),
        );
    }

    Ok(serde_json::json!({
        "runMode": "acceptance",
        "referenceClean": true,
        "environmentCount": summaries.len(),
        "distinctEnvironmentCount": environment_signatures.len(),
        "sourceLock": baseline_source_lock,
        "laneConfig": baseline_lane_config,
        "fixtures": baseline_fixtures,
        "environments": environments,
        "comparisons": aggregate_json(comparison_aggregates),
    }))
}

fn validate_summary_shape(summary: &Value, options: &Options) -> Result<(), Box<dyn Error>> {
    if required_string(summary, "runMode")? != "acceptance" {
        return Err("summary.runMode must be acceptance".into());
    }
    if summary.get("referenceClean").and_then(Value::as_bool) != Some(true) {
        return Err("summary.referenceClean must be true".into());
    }
    if required_u64(summary, "runCount")? < options.min_runs_per_environment {
        return Err(format!(
            "summary.runCount must be at least {}",
            options.min_runs_per_environment
        )
        .into());
    }
    let source_lock = required_object_value(summary, "sourceLock", "summary")?;
    if source_lock.get("vrmRsGitDirty").and_then(Value::as_bool) != Some(false) {
        return Err("sourceLock.vrmRsGitDirty must be false".into());
    }
    if required_string(source_lock, "threeVrmGitHead")?
        != required_string(source_lock, "expectedThreeVrmCommit")?
    {
        return Err("sourceLock.threeVrmGitHead must match expectedThreeVrmCommit".into());
    }
    validate_environment_lock(required_object_value(
        summary,
        "environmentLock",
        "summary",
    )?)?;
    validate_lane_config(required_object_value(summary, "laneConfig", "summary")?)?;
    required_array(summary, "fixtures")?;
    let comparisons = required_array(summary, "comparisons")?;
    if comparisons.len() != options.expected_comparisons {
        return Err(format!(
            "expected {} comparisons, got {}",
            options.expected_comparisons,
            comparisons.len()
        )
        .into());
    }
    for comparison in comparisons {
        let name = required_string(comparison, "name")?;
        let min_psnr = required_metric_f64(comparison, "minSelectedPsnr")?;
        if min_psnr < options.min_psnr_floor {
            return Err(format!(
                "{name} minSelectedPsnr {min_psnr:.4} is below {:.4}",
                options.min_psnr_floor
            )
            .into());
        }
        let alpha_mismatches = required_u64(comparison, "maxAlphaMismatches")?;
        if alpha_mismatches > options.max_alpha_mismatches {
            return Err(format!(
                "{name} maxAlphaMismatches {alpha_mismatches} exceeds {}",
                options.max_alpha_mismatches
            )
            .into());
        }
        let alpha_delta = required_u64(comparison, "maxAlphaDelta")?;
        if alpha_delta > options.max_alpha_delta {
            return Err(format!(
                "{name} maxAlphaDelta {alpha_delta} exceeds {}",
                options.max_alpha_delta
            )
            .into());
        }
    }
    Ok(())
}

fn validate_lane_config(lane_config: &Value) -> Result<(), Box<dyn Error>> {
    require_string_eq(lane_config, "metric", "rgb-visible")?;
    require_string_eq(lane_config, "background", "opaque-black")?;
    require_string_eq(lane_config, "mtoonLightAccumulation", "three-vrm")?;
    require_string_eq(lane_config, "diagnosticMode", "shaded")?;
    require_string_eq(lane_config, "frontFace", "ccw")?;
    require_string_eq(lane_config, "normalMapMode", "generated-tangents")?;
    require_string_eq(lane_config, "ownerIdPhaseOrderPolicy", "draw-index")?;
    require_string_eq(lane_config, "ownerIdColorSource", "vertex-color")?;
    require_u64_eq(lane_config, "browserReadyTimeoutMs", 60000)?;
    require_u64_eq(lane_config, "alphaMismatchTolerance", 0)?;
    require_u64_eq(lane_config, "alphaChannelTolerance", 0)?;
    require_bool_eq(lane_config, "disableTextureMips", false)?;
    require_bool_eq(lane_config, "forceNearestTextures", false)?;
    let normal_map_scale = required_f64(lane_config, "normalMapScale")?;
    if (normal_map_scale - 1.0).abs() > f64::EPSILON {
        return Err("laneConfig.normalMapScale must be 1.0".into());
    }
    Ok(())
}

fn validate_environment_lock(environment: &Value) -> Result<(), Box<dyn Error>> {
    for field in ["os", "family", "arch"] {
        required_string(environment, field)?;
    }
    for field in [
        "rustcVersion",
        "cargoVersion",
        "nodeVersion",
        "npmVersion",
        "justVersion",
    ] {
        require_string_or_null(environment, field)?;
    }
    match environment.get("gpuAdapters") {
        Some(Value::Null | Value::Array(_)) => Ok(()),
        Some(_) => Err("environmentLock.gpuAdapters must be null or an array".into()),
        None => Err("environmentLock.gpuAdapters is missing".into()),
    }
}

fn environment_signature(environment: &Value) -> Result<String, Box<dyn Error>> {
    Ok(compact_json(&serde_json::json!({
        "os": required_string(environment, "os")?,
        "family": required_string(environment, "family")?,
        "arch": required_string(environment, "arch")?,
        "gpuAdapters": environment.get("gpuAdapters").ok_or("environmentLock.gpuAdapters is missing")?,
    }))?)
}

fn comparison_names(summary: &Value) -> Result<Value, Box<dyn Error>> {
    let mut names = required_array(summary, "comparisons")?
        .iter()
        .map(|comparison| Ok(required_string(comparison, "name")?.to_owned()))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    names.sort();
    Ok(serde_json::json!(names))
}

fn collect_comparison_aggregates(
    summary: &Value,
    aggregates: &mut BTreeMap<String, MultiEnvAggregate>,
) -> Result<(), Box<dyn Error>> {
    for comparison in required_array(summary, "comparisons")? {
        let name = required_string(comparison, "name")?.to_owned();
        aggregates.entry(name).or_default().push(
            comparison,
            required_string(summary, "_summaryPath").unwrap_or("<in-memory>"),
        )?;
    }
    Ok(())
}

fn aggregate_json(aggregates: BTreeMap<String, MultiEnvAggregate>) -> Vec<Value> {
    aggregates
        .into_iter()
        .map(|(name, aggregate)| {
            serde_json::json!({
                "name": name,
                "environmentCount": aggregate.environment_count,
                "minSelectedPsnr": metric_json(aggregate.min_selected_psnr),
                "maxChannelDelta": aggregate.max_channel_delta,
                "maxAlphaMismatches": aggregate.max_alpha_mismatches,
                "maxAlphaDelta": aggregate.max_alpha_delta,
                "worstPsnrSummary": aggregate.worst_psnr_summary,
            })
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
struct MultiEnvAggregate {
    environment_count: usize,
    min_selected_psnr: f64,
    max_channel_delta: u64,
    max_alpha_mismatches: u64,
    max_alpha_delta: u64,
    worst_psnr_summary: String,
}

impl MultiEnvAggregate {
    fn push(&mut self, comparison: &Value, summary_path: &str) -> Result<(), Box<dyn Error>> {
        let psnr = required_metric_f64(comparison, "minSelectedPsnr")?;
        if self.environment_count == 0 || psnr < self.min_selected_psnr {
            self.min_selected_psnr = psnr;
            self.worst_psnr_summary = summary_path.to_owned();
        }
        self.max_channel_delta = self
            .max_channel_delta
            .max(required_u64(comparison, "maxChannelDelta")?);
        self.max_alpha_mismatches = self
            .max_alpha_mismatches
            .max(required_u64(comparison, "maxAlphaMismatches")?);
        self.max_alpha_delta = self
            .max_alpha_delta
            .max(required_u64(comparison, "maxAlphaDelta")?);
        self.environment_count += 1;
        Ok(())
    }
}

fn min_comparison_f64(summary: &Value, field: &str) -> Result<f64, Box<dyn Error>> {
    required_array(summary, "comparisons")?
        .iter()
        .map(|comparison| required_metric_f64(comparison, field))
        .try_fold(f64::INFINITY, |acc, value| {
            value.map(|value| acc.min(value))
        })
}

fn max_comparison_u64(summary: &Value, field: &str) -> Result<u64, Box<dyn Error>> {
    required_array(summary, "comparisons")?
        .iter()
        .map(|comparison| required_u64(comparison, field))
        .try_fold(0, |acc, value| value.map(|value| acc.max(value)))
}

fn markdown_summary(report: &Value) -> Result<String, Box<dyn Error>> {
    let mut output = String::from("# Multi-Environment Acceptance Summary\n\n");
    let source_lock = required_object_value(report, "sourceLock", "report")?;
    output.push_str(&format!(
        "- vrm-rs HEAD: `{}`\n",
        required_string(source_lock, "vrmRsGitHead")?
    ));
    output.push_str(&format!(
        "- three-vrm HEAD: `{}`\n",
        required_string(source_lock, "threeVrmGitHead")?
    ));
    output.push_str(&format!(
        "- Environments: `{}` summaries / `{}` distinct environments\n\n",
        required_u64(report, "environmentCount")?,
        required_u64(report, "distinctEnvironmentCount")?
    ));

    output.push_str("| Environment | Runs | minimum selected PSNR | Max alpha mismatches | Max alpha delta | GPU adapters |\n");
    output.push_str("| --- | ---: | ---: | ---: | ---: | --- |\n");
    for environment in required_array(report, "environments")? {
        let lock = required_object_value(environment, "environmentLock", "environment")?;
        output.push_str(&format!(
            "| `{}` `{}` `{}` | {} | {:.4} | {} | {} | `{}` |\n",
            required_string(lock, "os")?,
            required_string(lock, "family")?,
            required_string(lock, "arch")?,
            required_u64(environment, "runCount")?,
            required_f64(environment, "minSelectedPsnr")?,
            required_u64(environment, "maxAlphaMismatches")?,
            required_u64(environment, "maxAlphaDelta")?,
            compact_json(
                lock.get("gpuAdapters")
                    .ok_or("environmentLock.gpuAdapters is missing")?
            )?
        ));
    }

    output.push_str("\n| Fixture / renderer | Environments | minimum selected PSNR | Max channel delta | Max alpha mismatches | Max alpha delta |\n");
    output.push_str("| --- | ---: | ---: | ---: | ---: | ---: |\n");
    for comparison in required_array(report, "comparisons")? {
        output.push_str(&format!(
            "| {} | {} | {:.4} | {} | {} | {} |\n",
            required_string(comparison, "name")?,
            required_u64(comparison, "environmentCount")?,
            required_metric_f64(comparison, "minSelectedPsnr")?,
            required_u64(comparison, "maxChannelDelta")?,
            required_u64(comparison, "maxAlphaMismatches")?,
            required_u64(comparison, "maxAlphaDelta")?
        ));
    }
    Ok(output)
}

fn ensure_same_value(expected: &Value, actual: &Value, source: &str) -> Result<(), Box<dyn Error>> {
    if expected == actual {
        Ok(())
    } else {
        Err(format!("{source} differs between acceptance environment summaries").into())
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(value)?))?;
    Ok(())
}

fn write_text(path: &Path, text: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn write_test_bundle(bundle_dir: &Path, summary: &Value) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(bundle_dir)?;
    let summary_text = format!("{}\n", serde_json::to_string_pretty(summary)?);
    let mut files = Vec::new();
    write_test_bundle_file(
        bundle_dir,
        "acceptance-repeat-summary.json",
        &summary_text,
        &mut files,
    )?;
    write_test_bundle_file(
        bundle_dir,
        "acceptance-repeat-summary.md",
        "# acceptance repeat summary\n",
        &mut files,
    )?;
    write_test_bundle_file(
        bundle_dir,
        "acceptance-signoff.md",
        "# acceptance signoff\n",
        &mut files,
    )?;
    for run in 1..=required_u64(summary, "runCount")? {
        write_test_bundle_file(
            bundle_dir,
            &format!("run-{run}/review-manifest.json"),
            "{}\n",
            &mut files,
        )?;
        write_test_bundle_file(
            bundle_dir,
            &format!("run-{run}/summary.md"),
            "# run summary\n",
            &mut files,
        )?;
    }
    let manifest = serde_json::json!({
        "bundleFormat": "vrm-rs.render-parity.acceptance-evidence.v1",
        "sourceLock": required_object_value(summary, "sourceLock", "summary")?,
        "environmentLock": required_object_value(summary, "environmentLock", "summary")?,
        "runCount": required_u64(summary, "runCount")?,
        "comparisonCount": required_array(summary, "comparisons")?.len(),
        "minSelectedPsnr": metric_json(min_comparison_f64(summary, "minSelectedPsnr")?),
        "maxAlphaMismatches": max_comparison_u64(summary, "maxAlphaMismatches")?,
        "maxAlphaDelta": max_comparison_u64(summary, "maxAlphaDelta")?,
        "files": files,
    });
    write_json(&bundle_dir.join("bundle-manifest.json"), &manifest)
}

fn write_test_bundle_file(
    bundle_dir: &Path,
    relative: &str,
    text: &str,
    files: &mut Vec<Value>,
) -> Result<(), Box<dyn Error>> {
    let path = bundle_dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, text)?;
    files.push(serde_json::json!({
        "path": relative,
        "bytes": text.len() as u64,
    }));
    Ok(())
}

fn required_object_value<'a>(
    value: &'a Value,
    field: &str,
    source: &str,
) -> Result<&'a Value, Box<dyn Error>> {
    let field_value = value
        .get(field)
        .ok_or_else(|| format!("{source}.{field} must be an object"))?;
    if field_value.is_object() {
        Ok(field_value)
    } else {
        Err(format!("{source}.{field} must be an object").into())
    }
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field} must be an array").into())
}

fn required_array_value<'a>(value: &'a Value, field: &str) -> Result<&'a Value, Box<dyn Error>> {
    let field_value = value
        .get(field)
        .ok_or_else(|| format!("{field} must be an array"))?;
    if field_value.is_array() {
        Ok(field_value)
    } else {
        Err(format!("{field} must be an array").into())
    }
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} must be a string").into())
}

fn require_string_or_null<'a>(
    value: &'a Value,
    field: &str,
) -> Result<Option<&'a str>, Box<dyn Error>> {
    match value.get(field) {
        Some(Value::Null) => Ok(None),
        Some(field_value) => field_value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("{field} must be a string or null").into()),
        None => Err(format!("{field} must be a string or null").into()),
    }
}

fn required_u64(value: &Value, field: &str) -> Result<u64, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{field} must be an integer").into())
}

fn required_f64(value: &Value, field: &str) -> Result<f64, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{field} must be a number").into())
}

fn required_metric_f64(value: &Value, field: &str) -> Result<f64, Box<dyn Error>> {
    match value.get(field) {
        Some(Value::String(text)) if text == "Infinity" => Ok(f64::INFINITY),
        Some(field_value) => field_value
            .as_f64()
            .ok_or_else(|| format!("{field} must be a number or \"Infinity\"").into()),
        None => Err(format!("{field} must be a number or \"Infinity\"").into()),
    }
}

fn metric_json(value: f64) -> Value {
    if value.is_infinite() {
        Value::String("Infinity".to_owned())
    } else {
        Value::from(value)
    }
}

fn require_string_eq(value: &Value, field: &str, expected: &str) -> Result<(), Box<dyn Error>> {
    let actual = required_string(value, field)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{field} must be {expected:?}, got {actual:?}").into())
    }
}

fn require_u64_eq(value: &Value, field: &str, expected: u64) -> Result<(), Box<dyn Error>> {
    let actual = required_u64(value, field)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{field} must be {expected}, got {actual}").into())
    }
}

fn require_bool_eq(value: &Value, field: &str, expected: bool) -> Result<(), Box<dyn Error>> {
    match value.get(field).and_then(Value::as_bool) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(format!("{field} must be {expected}, got {actual}").into()),
        None => Err(format!("{field} must be a boolean").into()),
    }
}

fn compact_json(value: &Value) -> Result<String, Box<dyn Error>> {
    Ok(serde_json::to_string(value)?)
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn test_summary(label: &str, adapter: &str) -> Value {
    serde_json::json!({
        "_summaryPath": label,
        "runMode": "acceptance",
        "referenceClean": true,
        "runCount": 3,
        "sourceLock": {
            "vrmRsGitHead": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "vrmRsGitDirty": false,
            "threeVrmGitHead": "9d125586f6d7da094b0ac5f204cebf19586f2397",
            "expectedThreeVrmCommit": "9d125586f6d7da094b0ac5f204cebf19586f2397"
        },
        "environmentLock": {
            "os": "windows",
            "family": "windows",
            "arch": "x86_64",
            "rustcVersion": "rustc 1.98.0-nightly",
            "cargoVersion": "cargo 1.98.0-nightly",
            "nodeVersion": "v25.9.0",
            "npmVersion": null,
            "justVersion": "just 1.49.0",
            "gpuAdapters": [{"Name": adapter, "DriverVersion": "1.2.3"}]
        },
        "laneConfig": {
            "alphaChannelTolerance": 0,
            "alphaMismatchTolerance": 0,
            "background": "opaque-black",
            "browserReadyTimeoutMs": 60000,
            "diagnosticMode": "shaded",
            "disableTextureMips": false,
            "forceNearestTextures": false,
            "frontFace": "ccw",
            "metric": "rgb-visible",
            "mtoonLightAccumulation": "three-vrm",
            "normalMapMode": "generated-tangents",
            "normalMapScale": 1.0,
            "ownerIdColorSource": "vertex-color",
            "ownerIdPhaseOrderPolicy": "draw-index"
        },
        "fixtures": [
            {
                "source": "Seed-san.vrm",
                "sourceSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "sourceBytes": 1
            }
        ],
        "comparisons": [
            {
                "name": "Seed-san.vrm/wgpu",
                "minSelectedPsnr": 34.25,
                "maxChannelDelta": 255,
                "maxAlphaMismatches": 0,
                "maxAlphaDelta": 0
            },
            {
                "name": "Seed-san.vrm/bevy",
                "minSelectedPsnr": 34.50,
                "maxChannelDelta": 128,
                "maxAlphaMismatches": 0,
                "maxAlphaDelta": 0
            }
        ]
    })
}
