#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

//! Export the small, portable acceptance-repeat evidence files for another machine.

use clap::Parser;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "export-acceptance-evidence-bundle",
    about = "Copy portable render-parity acceptance evidence files into a transfer bundle"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/render-parity-acceptance-repeat"
    )]
    acceptance_root: PathBuf,
    #[arg(
        long,
        default_value = ".external-fixtures/render-parity-acceptance-bundle"
    )]
    out_dir: PathBuf,
    #[arg(long)]
    include_visual_contact_sheets: bool,
    #[arg(long)]
    require_accepted_signoff: bool,
    #[arg(long)]
    apply: bool,
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
    let bundle = build_bundle_plan(
        &options.acceptance_root,
        options.include_visual_contact_sheets,
        options.require_accepted_signoff,
    )?;
    if options.apply {
        write_bundle(&bundle, &options.out_dir)?;
        println!(
            "wrote acceptance evidence bundle: {}",
            display_path(&options.out_dir)
        );
    } else {
        println!(
            "dry run: would write {} files to {}",
            bundle.files.len() + 1,
            display_path(&options.out_dir)
        );
        println!("rerun with --apply to write the bundle");
    }
    Ok(())
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from("target/acceptance-evidence-bundle-self-test/source");
    let out = PathBuf::from("target/acceptance-evidence-bundle-self-test/bundle");
    let _ = fs::remove_dir_all(root.parent().unwrap_or(Path::new("target")));
    fs::create_dir_all(root.join("run-1"))?;
    fs::create_dir_all(root.join("run-2"))?;
    fs::create_dir_all(root.join("run-3"))?;
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        test_summary_text(),
    )?;
    fs::write(root.join("acceptance-repeat-summary.md"), "# summary\n")?;
    fs::write(
        root.join("acceptance-signoff.md"),
        test_accepted_signoff_text("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    )?;
    for run in ["run-1", "run-2", "run-3"] {
        fs::write(root.join(run).join("review-manifest.json"), "{}\n")?;
        fs::write(root.join(run).join("summary.md"), "# run\n")?;
    }

    let plan = build_bundle_plan(&root, false, false)?;
    if plan.files.len() != 9 {
        return Err(format!(
            "expected 9 files in self-test plan, got {}",
            plan.files.len()
        )
        .into());
    }
    write_bundle(&plan, &out)?;
    let manifest = read_json(&out.join("bundle-manifest.json"))?;
    if required_array(&manifest, "files")?.len() != 9 {
        return Err("bundle manifest should record copied files".into());
    }
    if !out.join("acceptance-repeat-summary.json").exists() {
        return Err("summary JSON was not copied".into());
    }
    fs::write(out.join("stale-file.txt"), "stale")?;
    write_bundle(&plan, &out)?;
    if out.join("stale-file.txt").exists() {
        return Err("bundle writer should prune stale output files".into());
    }

    let without_signoff =
        PathBuf::from("target/acceptance-evidence-bundle-self-test/source-without-signoff");
    fs::create_dir_all(&without_signoff)?;
    copy_tree(&root, &without_signoff)?;
    fs::write(
        without_signoff.join("acceptance-repeat-summary.json"),
        test_summary_text().replace(
            "target/acceptance-evidence-bundle-self-test/source/",
            "target/acceptance-evidence-bundle-self-test/source-without-signoff/",
        ),
    )?;
    fs::remove_file(without_signoff.join("acceptance-signoff.md"))?;
    let without_signoff_plan = build_bundle_plan(&without_signoff, false, false)?;
    if without_signoff_plan.files.len() != 8 {
        return Err("signoff should be optional in bundle plan".into());
    }
    if build_bundle_plan(&without_signoff, false, true).is_ok() {
        return Err("missing signoff should be rejected when strict export is requested".into());
    }

    fs::write(root.join("acceptance-signoff.md"), "# signoff\n")?;
    if build_bundle_plan(&root, false, true).is_ok() {
        return Err("draft signoff should be rejected when strict export is requested".into());
    }
    fs::write(
        root.join("acceptance-signoff.md"),
        test_accepted_signoff_text("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    )?;
    let strict_plan = build_bundle_plan(&root, false, true)?;
    if !strict_plan.require_accepted_signoff {
        return Err("strict bundle plan should record accepted signoff requirement".into());
    }
    let strict_manifest = bundle_manifest(&strict_plan, &out)?;
    if strict_manifest
        .get("acceptedSignoffRequired")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("strict bundle manifest should record acceptedSignoffRequired=true".into());
    }

    fs::write(
        root.join("run-1").join("visual-contact-sheet.png"),
        [137, 80, 78, 71],
    )?;
    let visual_plan = build_bundle_plan(&root, true, false)?;
    if visual_plan.files.len() != 10 {
        return Err("visual contact sheets should be included when requested".into());
    }

    let mut bad_summary = serde_json::from_str::<Value>(&test_summary_text())?;
    bad_summary["runCount"] = Value::from(4);
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        serde_json::to_string(&bad_summary)?,
    )?;
    if build_bundle_plan(&root, false, false).is_ok() {
        return Err("summary runCount mismatch should be rejected".into());
    }
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        test_summary_text(),
    )?;

    let mut artifact_mismatch = serde_json::from_str::<Value>(&test_summary_text())?;
    artifact_mismatch["runs"][1]["artifacts"] = Value::String("other-run".to_owned());
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        serde_json::to_string(&artifact_mismatch)?,
    )?;
    if build_bundle_plan(&root, false, false).is_ok() {
        return Err("runs[].artifacts mismatch should be rejected".into());
    }
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        test_summary_text(),
    )?;

    fs::remove_dir_all(root.join("run-3"))?;
    if build_bundle_plan(&root, false, false).is_ok() {
        return Err("missing run directory should be rejected".into());
    }
    fs::create_dir_all(root.join("run-3"))?;
    fs::write(root.join("run-3").join("review-manifest.json"), "{}\n")?;
    fs::write(root.join("run-3").join("summary.md"), "# run\n")?;

    let mut bad_environment = serde_json::from_str::<Value>(&test_summary_text())?;
    bad_environment["environmentLock"]["gpuAdapters"] = Value::String("bad".to_owned());
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        serde_json::to_string(&bad_environment)?,
    )?;
    if build_bundle_plan(&root, false, false).is_ok() {
        return Err("bad environmentLock should be rejected".into());
    }
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        test_summary_text(),
    )?;

    let mut bad_lane = serde_json::from_str::<Value>(&test_summary_text())?;
    bad_lane["laneConfig"]["metric"] = Value::String("rgb-all".to_owned());
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        serde_json::to_string(&bad_lane)?,
    )?;
    if build_bundle_plan(&root, false, false).is_ok() {
        return Err("bad laneConfig should be rejected".into());
    }
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        test_summary_text(),
    )?;

    let mut bad_comparison_runs = serde_json::from_str::<Value>(&test_summary_text())?;
    bad_comparison_runs["comparisons"][0]["runs"] = Value::from(2);
    fs::write(
        root.join("acceptance-repeat-summary.json"),
        serde_json::to_string(&bad_comparison_runs)?,
    )?;
    if build_bundle_plan(&root, false, false).is_ok() {
        return Err("comparison run mismatch should be rejected".into());
    }
    if write_bundle(&plan, &root).is_ok() {
        return Err("bundle writer should reject out_dir equal to acceptance_root".into());
    }
    if write_bundle(&plan, &root.join("run-1")).is_ok() {
        return Err("bundle writer should reject out_dir inside acceptance_root".into());
    }
    if write_bundle(
        &plan,
        root.parent().ok_or("self-test source has no parent")?,
    )
    .is_ok()
    {
        return Err("bundle writer should reject out_dir above acceptance_root".into());
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct BundlePlan {
    acceptance_root: PathBuf,
    summary: Value,
    files: Vec<BundleFile>,
    require_accepted_signoff: bool,
}

#[derive(Clone, Debug)]
struct BundleFile {
    source: PathBuf,
    relative: PathBuf,
    bytes: u64,
}

fn build_bundle_plan(
    acceptance_root: &Path,
    include_visual_contact_sheets: bool,
    require_accepted_signoff: bool,
) -> Result<BundlePlan, Box<dyn Error>> {
    let summary_path = acceptance_root.join("acceptance-repeat-summary.json");
    let summary = read_json(&summary_path)?;
    validate_summary(&summary)?;

    let mut files = Vec::new();
    add_required_file(
        &mut files,
        acceptance_root,
        Path::new("acceptance-repeat-summary.json"),
    )?;
    add_required_file(
        &mut files,
        acceptance_root,
        Path::new("acceptance-repeat-summary.md"),
    )?;
    add_optional_file(
        &mut files,
        acceptance_root,
        Path::new("acceptance-signoff.md"),
    )?;
    if require_accepted_signoff {
        validate_accepted_signoff(acceptance_root, &summary)?;
    }
    let run_count = required_u64(&summary, "runCount")?;
    let runs = required_array(&summary, "runs")?;
    for run_number in 1..=run_count {
        let run = expected_run_dir(acceptance_root, runs, run_number)?;
        add_required_file(
            &mut files,
            acceptance_root,
            &Path::new(&run).join("review-manifest.json"),
        )?;
        add_required_file(
            &mut files,
            acceptance_root,
            &Path::new(&run).join("summary.md"),
        )?;
        if include_visual_contact_sheets {
            add_optional_file(
                &mut files,
                acceptance_root,
                &Path::new(&run).join("visual-contact-sheet.png"),
            )?;
            add_optional_file(
                &mut files,
                acceptance_root,
                &Path::new(&run).join("visual-diff-contact-sheet.png"),
            )?;
        }
    }

    Ok(BundlePlan {
        acceptance_root: acceptance_root.to_path_buf(),
        summary,
        files,
        require_accepted_signoff,
    })
}

fn validate_summary(summary: &Value) -> Result<(), Box<dyn Error>> {
    if required_string(summary, "runMode")? != "acceptance" {
        return Err("acceptance-repeat summary must have runMode=acceptance".into());
    }
    if summary.get("referenceClean").and_then(Value::as_bool) != Some(true) {
        return Err("acceptance-repeat summary must have referenceClean=true".into());
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
    let run_count = required_u64(summary, "runCount")?;
    let runs = required_array(summary, "runs")?;
    if run_count as usize != runs.len() {
        return Err(format!(
            "summary.runCount {run_count} does not match runs length {}",
            runs.len()
        )
        .into());
    }
    for run in runs {
        required_string(run, "artifacts")?;
    }
    let comparisons = required_array(summary, "comparisons")?;
    for comparison in comparisons {
        required_string(comparison, "name")?;
        let comparison_runs = required_u64(comparison, "runs")?;
        if comparison_runs != run_count {
            return Err(format!(
                "comparison.runs {comparison_runs} does not match summary.runCount {run_count}"
            )
            .into());
        }
        required_metric_f64(comparison, "minSelectedPsnr")?;
        required_u64(comparison, "maxChannelDelta")?;
        required_u64(comparison, "maxAlphaMismatches")?;
        required_u64(comparison, "maxAlphaDelta")?;
    }
    Ok(())
}

fn validate_accepted_signoff(
    acceptance_root: &Path,
    summary: &Value,
) -> Result<(), Box<dyn Error>> {
    let path = acceptance_root.join("acceptance-signoff.md");
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("accepted signoff is missing: {}: {err}", display_path(&path)))?;
    let source_lock = required_object_value(summary, "sourceLock", "summary")?;
    let vrm_rs_head = required_string(source_lock, "vrmRsGitHead")?;
    let three_vrm_head = required_string(source_lock, "threeVrmGitHead")?;
    for needle in [
        format!("- vrm-rs HEAD: `{vrm_rs_head}`"),
        "- current-source gate: `required`".to_owned(),
        format!("- three-vrm HEAD: `{three_vrm_head}`"),
        "- numeric gate: pass".to_owned(),
        "- visual review: accepted".to_owned(),
        "- signoff status: complete".to_owned(),
    ] {
        if !text.contains(&needle) {
            return Err(format!(
                "accepted signoff {} is missing required line {needle:?}",
                display_path(&path)
            )
            .into());
        }
    }
    let reviewer_prefix = "- reviewer: `";
    let reviewer_line = text
        .lines()
        .find(|line| line.starts_with(reviewer_prefix))
        .ok_or("accepted signoff must record a reviewer")?;
    if reviewer_line == "- reviewer: ``" {
        return Err("accepted signoff reviewer must not be empty".into());
    }
    Ok(())
}

fn expected_run_dir(
    acceptance_root: &Path,
    runs: &[Value],
    run_number: u64,
) -> Result<String, Box<dyn Error>> {
    let run = runs
        .get((run_number - 1) as usize)
        .ok_or_else(|| format!("runs[{run_number}] is missing"))?;
    let artifacts = required_string(run, "artifacts")?;
    let run_dir = format!("run-{run_number}");
    let artifact_name = Path::new(artifacts)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("runs[{run_number}].artifacts must end with {run_dir}"))?;
    if artifact_name != run_dir {
        return Err(format!(
            "runs[{run_number}].artifacts must end with {run_dir}, got {artifact_name}"
        )
        .into());
    }
    let expected = acceptance_root.join(&run_dir);
    if canonical_or_absolute(Path::new(artifacts))? != canonical_or_absolute(&expected)? {
        return Err(format!(
            "runs[{run_number}].artifacts must match {}",
            display_path(&expected)
        )
        .into());
    }
    Ok(run_dir)
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
    let normal_map_scale = required_metric_f64(lane_config, "normalMapScale")?;
    if (normal_map_scale - 1.0).abs() > f64::EPSILON {
        return Err("laneConfig.normalMapScale must be 1.0".into());
    }
    Ok(())
}

fn add_required_file(
    files: &mut Vec<BundleFile>,
    root: &Path,
    relative: &Path,
) -> Result<(), Box<dyn Error>> {
    let source = root.join(relative);
    if !source.is_file() {
        return Err(format!(
            "required evidence file is missing: {}",
            display_path(&source)
        )
        .into());
    }
    add_file(files, source, relative)
}

fn add_optional_file(
    files: &mut Vec<BundleFile>,
    root: &Path,
    relative: &Path,
) -> Result<(), Box<dyn Error>> {
    let source = root.join(relative);
    if source.is_file() {
        add_file(files, source, relative)?;
    }
    Ok(())
}

fn add_file(
    files: &mut Vec<BundleFile>,
    source: PathBuf,
    relative: &Path,
) -> Result<(), Box<dyn Error>> {
    let bytes = source.metadata()?.len();
    files.push(BundleFile {
        source,
        relative: relative.to_path_buf(),
        bytes,
    });
    Ok(())
}

fn write_bundle(plan: &BundlePlan, out_dir: &Path) -> Result<(), Box<dyn Error>> {
    prepare_output_dir(out_dir, &plan.acceptance_root)?;
    fs::create_dir_all(out_dir)?;
    for file in &plan.files {
        let destination = out_dir.join(&file.relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&file.source, destination)?;
    }
    let manifest = bundle_manifest(plan, out_dir)?;
    fs::write(
        out_dir.join("bundle-manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    Ok(())
}

fn prepare_output_dir(out_dir: &Path, acceptance_root: &Path) -> Result<(), Box<dyn Error>> {
    let out_abs = canonical_or_absolute(out_dir)?;
    let acceptance_abs = canonical_or_absolute(acceptance_root)?;
    if out_abs == acceptance_abs
        || out_abs.starts_with(&acceptance_abs)
        || acceptance_abs.starts_with(&out_abs)
    {
        return Err("refusing to write a bundle inside, above, or over the acceptance evidence source directory".into());
    }
    if !out_dir.exists() {
        return Ok(());
    }
    if out_dir.is_file() {
        return Err(format!("output path is a file: {}", display_path(out_dir)).into());
    }
    if out_dir.parent().is_none() {
        return Err("refusing to replace a filesystem root output directory".into());
    }
    let current_dir = std::env::current_dir()?;
    if out_dir.canonicalize()? == current_dir.canonicalize()? {
        return Err("refusing to replace the current working directory".into());
    }
    fs::remove_dir_all(out_dir)?;
    Ok(())
}

fn canonical_or_absolute(path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        if parent.exists() {
            return Ok(parent
                .canonicalize()?
                .join(path.file_name().unwrap_or_default()));
        }
    }
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    })
}

fn bundle_manifest(plan: &BundlePlan, out_dir: &Path) -> Result<Value, Box<dyn Error>> {
    let comparisons = required_array(&plan.summary, "comparisons")?;
    let min_psnr = comparisons
        .iter()
        .map(|comparison| required_metric_f64(comparison, "minSelectedPsnr"))
        .try_fold(f64::INFINITY, |acc, value| {
            value.map(|value| acc.min(value))
        })?;
    let max_alpha_mismatches = comparisons
        .iter()
        .map(|comparison| required_u64(comparison, "maxAlphaMismatches"))
        .try_fold(0, |acc, value| value.map(|value| acc.max(value)))?;
    let max_alpha_delta = comparisons
        .iter()
        .map(|comparison| required_u64(comparison, "maxAlphaDelta"))
        .try_fold(0, |acc, value| value.map(|value| acc.max(value)))?;
    Ok(serde_json::json!({
        "bundleFormat": "vrm-rs.render-parity.acceptance-evidence.v1",
        "acceptanceRoot": display_path(&plan.acceptance_root),
        "bundleRoot": display_path(out_dir),
        "acceptedSignoffRequired": plan.require_accepted_signoff,
        "sourceLock": required_object(&plan.summary, "sourceLock", "summary")?,
        "environmentLock": required_object(&plan.summary, "environmentLock", "summary")?,
        "runCount": required_u64(&plan.summary, "runCount")?,
        "comparisonCount": comparisons.len(),
        "minSelectedPsnr": metric_json(min_psnr),
        "maxAlphaMismatches": max_alpha_mismatches,
        "maxAlphaDelta": max_alpha_delta,
        "files": plan.files.iter().map(|file| {
            serde_json::json!({
                "path": file.relative.display().to_string().replace('\\', "/"),
                "source": display_path(&file.source),
                "bytes": file.bytes,
            })
        }).collect::<Vec<_>>(),
    }))
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn required_object<'a>(
    value: &'a Value,
    field: &str,
    source: &str,
) -> Result<&'a serde_json::Map<String, Value>, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{source}.{field} must be an object").into())
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

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_tree(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

fn test_accepted_signoff_text(vrm_rs_head: &str) -> String {
    format!(
        "\
# Render Parity Acceptance Signoff

- vrm-rs HEAD: `{vrm_rs_head}`
- current-source gate: `required`
- three-vrm HEAD: `9d125586f6d7da094b0ac5f204cebf19586f2397`
- numeric gate: pass
- visual review: accepted
- signoff status: complete
- reviewer: `self-test`
"
    )
}

fn test_summary_text() -> String {
    serde_json::json!({
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
            "gpuAdapters": [{"Name": "Adapter", "DriverVersion": "1.2.3"}]
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
        "runs": [
            {"artifacts": "target/acceptance-evidence-bundle-self-test/source/run-1"},
            {"artifacts": "target/acceptance-evidence-bundle-self-test/source/run-2"},
            {"artifacts": "target/acceptance-evidence-bundle-self-test/source/run-3"}
        ],
        "comparisons": [
            {
                "name": "Seed-san.vrm/wgpu",
                "runs": 3,
                "minSelectedPsnr": 34.25,
                "maxChannelDelta": 255,
                "maxAlphaMismatches": 0,
                "maxAlphaDelta": 0
            }
        ]
    })
    .to_string()
}
