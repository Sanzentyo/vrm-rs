#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

//! Validate repeated render-parity acceptance manifests as final-evidence candidates.

use clap::Parser;
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "validate-acceptance-repeat",
    about = "Validate repeated reference-clean render parity acceptance manifests"
)]
struct Options {
    #[arg(long, required_unless_present = "self_test")]
    manifest: Vec<PathBuf>,
    #[arg(long, default_value_t = 3)]
    min_runs: usize,
    #[arg(long)]
    allow_dirty_source: bool,
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

    let manifests = options
        .manifest
        .iter()
        .map(|path| read_manifest(path))
        .collect::<Result<Vec<_>, _>>()?;
    let summary = validate_repeated_manifests(
        &manifests,
        options.min_runs,
        options.allow_dirty_source,
    )?;
    if let Some(path) = options.json_out.as_ref() {
        write_json(path, &summary)?;
    }
    if let Some(path) = options.markdown_out.as_ref() {
        write_markdown(path, &summary)?;
    }
    println!(
        "validated {} repeated acceptance manifests",
        summary
            .get("runs")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    );
    Ok(())
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let manifests = ["run-1", "run-2", "run-3"]
        .into_iter()
        .map(test_manifest)
        .collect::<Vec<_>>();
    validate_repeated_manifests(&manifests, 3, false)?;

    let mut dirty = manifests.clone();
    dirty[1]["sourceLock"]["vrmRsGitDirty"] = Value::Bool(true);
    if validate_repeated_manifests(&dirty, 3, false).is_ok() {
        return Err("dirty acceptance source should be rejected".into());
    }
    validate_repeated_manifests(&dirty, 3, true)?;

    let mut mismatch = manifests.clone();
    mismatch[2]["fixtures"][0]["sourceSha256"] =
        Value::String("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned());
    if validate_repeated_manifests(&mismatch, 3, false).is_ok() {
        return Err("fixture hash mismatch should be rejected".into());
    }

    let mut comparison_mismatch = manifests.clone();
    comparison_mismatch[2]["fixtures"][0]["comparisons"][0]["renderer"] =
        Value::String("custom".to_owned());
    if validate_repeated_manifests(&comparison_mismatch, 3, false).is_ok() {
        return Err("comparison mismatch should be rejected".into());
    }

    let mut failed_summary = manifests.clone();
    failed_summary[2]["fixtures"][0]["comparisons"][0]["summary"]["pass"] = Value::Bool(false);
    if validate_repeated_manifests(&failed_summary, 3, false).is_ok() {
        return Err("failed comparison summary should be rejected".into());
    }

    Ok(())
}

fn read_manifest(path: &Path) -> Result<Value, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let mut manifest = serde_json::from_str::<Value>(&text)?;
    manifest["_manifestPath"] = Value::String(display_path(path));
    Ok(manifest)
}

fn validate_repeated_manifests(
    manifests: &[Value],
    min_runs: usize,
    allow_dirty_source: bool,
) -> Result<Value, Box<dyn Error>> {
    if manifests.len() < min_runs {
        return Err(format!(
            "expected at least {min_runs} acceptance manifests, got {}",
            manifests.len()
        )
        .into());
    }

    let baseline = manifests
        .first()
        .ok_or("expected at least one acceptance manifest")?;
    validate_acceptance_manifest(baseline, allow_dirty_source)?;
    let baseline_source_lock = source_lock_signature(baseline, allow_dirty_source)?;
    let baseline_lane_config = lane_config_signature(baseline)?;
    let baseline_fixtures = fixture_signatures(baseline)?;
    let baseline_comparisons = comparison_signatures(baseline)?;
    validate_acceptance_lane_shape(&baseline_fixtures, &baseline_comparisons)?;

    let mut runs = Vec::with_capacity(manifests.len());
    let mut aggregates = BTreeMap::<String, MetricAggregate>::new();
    for (index, manifest) in manifests.iter().enumerate() {
        validate_acceptance_manifest(manifest, allow_dirty_source)?;
        ensure_same_value(
            &baseline_source_lock,
            &source_lock_signature(manifest, allow_dirty_source)?,
            &format!("manifest[{index}].sourceLock"),
        )?;
        ensure_same_value(
            &baseline_lane_config,
            &lane_config_signature(manifest)?,
            &format!("manifest[{index}].laneConfig"),
        )?;
        ensure_same_value(
            &baseline_fixtures,
            &fixture_signatures(manifest)?,
            &format!("manifest[{index}].fixtures"),
        )?;
        ensure_same_value(
            &baseline_comparisons,
            &comparison_signatures(manifest)?,
            &format!("manifest[{index}].comparisons"),
        )?;

        collect_run_aggregates(manifest, &mut aggregates)?;
        runs.push(serde_json::json!({
            "manifest": manifest.get("_manifestPath").and_then(Value::as_str).unwrap_or("<in-memory>"),
            "artifacts": required_string(manifest, "artifacts")?,
        }));
    }

    for (name, aggregate) in &aggregates {
        if aggregate.runs != manifests.len() {
            return Err(format!(
                "{name} collected {} runs, expected {}",
                aggregate.runs,
                manifests.len()
            )
            .into());
        }
    }

    Ok(serde_json::json!({
        "runCount": manifests.len(),
        "runs": runs,
        "sourceLock": baseline_source_lock,
        "laneConfig": baseline_lane_config,
        "fixtures": baseline_fixtures,
        "comparisons": aggregate_json(aggregates),
    }))
}

fn validate_acceptance_manifest(
    manifest: &Value,
    allow_dirty_source: bool,
) -> Result<(), Box<dyn Error>> {
    if required_string(manifest, "runMode")? != "acceptance" {
        return Err("all repeated manifests must have runMode=acceptance".into());
    }
    if !required_bool(manifest, "referenceClean")? {
        return Err("all repeated manifests must have referenceClean=true".into());
    }
    let source_lock = required_object(manifest, "sourceLock", "manifest")?;
    let Some(vrm_rs_head) = source_lock.get("vrmRsGitHead").and_then(Value::as_str) else {
        return Err("sourceLock.vrmRsGitHead must be present for acceptance-repeat evidence".into());
    };
    if vrm_rs_head.len() != 40 {
        return Err("sourceLock.vrmRsGitHead must be a 40-character git commit".into());
    }
    if !allow_dirty_source && source_lock.get("vrmRsGitDirty").and_then(Value::as_bool) != Some(false)
    {
        return Err(
            "sourceLock.vrmRsGitDirty must be false for acceptance-repeat evidence".into(),
        );
    }
    let three_vrm_head = required_string(source_lock, "threeVrmGitHead")?;
    let expected_three_vrm = required_string(source_lock, "expectedThreeVrmCommit")?;
    if three_vrm_head != expected_three_vrm {
        return Err(format!(
            "sourceLock.threeVrmGitHead ({three_vrm_head}) must match expectedThreeVrmCommit ({expected_three_vrm})"
        )
        .into());
    }
    Ok(())
}

fn source_lock_signature(
    manifest: &Value,
    allow_dirty_source: bool,
) -> Result<Value, Box<dyn Error>> {
    let mut source_lock = required_object(manifest, "sourceLock", "manifest")?.clone();
    if allow_dirty_source {
        source_lock["vrmRsGitDirty"] = Value::String("<allowed>".to_owned());
    }
    Ok(source_lock)
}

fn lane_config_signature(manifest: &Value) -> Result<Value, Box<dyn Error>> {
    let mut signature = serde_json::Map::new();
    for field in [
        "metric",
        "background",
        "mtoonLightAccumulation",
        "diagnosticMode",
        "ownerIdPhaseOrderPolicy",
        "ownerIdColorSource",
        "frontFace",
        "browserReadyTimeoutMs",
        "normalMapMode",
        "normalMapScale",
        "disableTextureMips",
        "forceNearestTextures",
        "alphaMismatchTolerance",
        "alphaChannelTolerance",
    ] {
        signature.insert(
            field.to_owned(),
            manifest
                .get(field)
                .ok_or_else(|| format!("manifest.{field} is missing"))?
                .clone(),
        );
    }
    Ok(Value::Object(signature))
}

fn validate_acceptance_lane_shape(
    fixtures: &Value,
    comparisons: &Value,
) -> Result<(), Box<dyn Error>> {
    let fixtures = fixtures
        .as_array()
        .ok_or("fixture signature must be an array")?;
    if fixtures.len() != 6 {
        return Err(format!(
            "acceptance-repeat expects the current six-fixture lane, got {} fixtures",
            fixtures.len()
        )
        .into());
    }

    let comparisons = comparisons
        .as_array()
        .ok_or("comparison signature must be an array")?;
    if comparisons.len() != fixtures.len() * 3 {
        return Err(format!(
            "acceptance-repeat expects wgpu/Bevy/Ash comparisons for each fixture, got {} comparisons",
            comparisons.len()
        )
        .into());
    }

    for fixture in fixtures {
        let fixture_name = required_string(fixture, "name")?;
        for renderer in ["wgpu", "bevy", "ash"] {
            let Some(comparison) = comparisons.iter().find(|comparison| {
                comparison.get("fixture").and_then(Value::as_str) == Some(fixture_name)
                    && comparison.get("renderer").and_then(Value::as_str) == Some(renderer)
            }) else {
                return Err(format!("{fixture_name} is missing gated {renderer} comparison").into());
            };
            if comparison.get("visualParityGate").and_then(Value::as_bool) != Some(true) {
                return Err(
                    format!("{fixture_name}/{renderer} must be in the visual parity gate").into(),
                );
            }
        }
    }
    Ok(())
}

fn fixture_signatures(manifest: &Value) -> Result<Value, Box<dyn Error>> {
    let fixtures = required_array(manifest, "fixtures")?;
    if fixtures.is_empty() {
        return Err("fixtures must not be empty".into());
    }
    let signatures = fixtures
        .iter()
        .enumerate()
        .map(|(index, fixture)| {
            let source = format!("fixtures[{index}]");
            Ok(serde_json::json!({
                "name": required_string(fixture, "name")?,
                "stem": required_string(fixture, "stem")?,
                "source": required_string(fixture, "source")?,
                "sourceSha256": require_sha256(fixture, &source)?,
                "sourceSizeBytes": require_nonzero_u64(fixture, "sourceSizeBytes", &source)?,
            }))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    Ok(Value::Array(signatures))
}

fn comparison_signatures(manifest: &Value) -> Result<Value, Box<dyn Error>> {
    let fixtures = required_array(manifest, "fixtures")?;
    let mut signatures = Vec::new();
    for fixture in fixtures {
        let fixture_name = required_string(fixture, "name")?;
        let comparisons = required_array(fixture, "comparisons")?;
        if comparisons.is_empty() {
            return Err(format!("{fixture_name}.comparisons must not be empty").into());
        }
        for comparison in comparisons {
            signatures.push(serde_json::json!({
                "fixture": fixture_name,
                "renderer": required_string(comparison, "renderer")?,
                "visualParityGate": required_bool(comparison, "visualParityGate")?,
            }));
        }
    }
    Ok(Value::Array(signatures))
}

fn collect_run_aggregates(
    manifest: &Value,
    aggregates: &mut BTreeMap<String, MetricAggregate>,
) -> Result<(), Box<dyn Error>> {
    for fixture in required_array(manifest, "fixtures")? {
        let fixture_name = required_string(fixture, "name")?;
        for comparison in required_array(fixture, "comparisons")? {
            let renderer = required_string(comparison, "renderer")?;
            let summary = required_object(comparison, "summary", "comparison")?;
            if !required_bool(summary, "pass")? {
                return Err(format!("{fixture_name}/{renderer} did not pass").into());
            }
            let key = format!("{fixture_name}/{renderer}");
            aggregates
                .entry(key)
                .or_insert_with(MetricAggregate::default)
                .push(summary)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct MetricAggregate {
    runs: usize,
    min_psnr: Option<f64>,
    max_channel_delta: u64,
    max_alpha_mismatches: u64,
    max_alpha_delta: u64,
}

impl MetricAggregate {
    fn push(&mut self, summary: &Value) -> Result<(), Box<dyn Error>> {
        self.runs += 1;
        let psnr = summary_string(summary, "selectedPsnr")?;
        if psnr != "Infinity" {
            let psnr = psnr.parse::<f64>()?;
            self.min_psnr = Some(self.min_psnr.map_or(psnr, |current| current.min(psnr)));
        }
        self.max_channel_delta = self
            .max_channel_delta
            .max(summary_string(summary, "maxChannelDelta")?.parse::<u64>()?);
        self.max_alpha_mismatches = self
            .max_alpha_mismatches
            .max(summary_string(summary, "alphaMismatches")?.parse::<u64>()?);
        self.max_alpha_delta = self
            .max_alpha_delta
            .max(summary_string(summary, "alphaMaxDelta")?.parse::<u64>()?);
        Ok(())
    }
}

fn aggregate_json(aggregates: BTreeMap<String, MetricAggregate>) -> Value {
    Value::Array(
        aggregates
            .into_iter()
            .map(|(name, aggregate)| {
                serde_json::json!({
                    "name": name,
                    "runs": aggregate.runs,
                    "minSelectedPsnr": aggregate.min_psnr.map_or(Value::String("Infinity".to_owned()), Value::from),
                    "maxChannelDelta": aggregate.max_channel_delta,
                    "maxAlphaMismatches": aggregate.max_alpha_mismatches,
                    "maxAlphaDelta": aggregate.max_alpha_delta,
                })
            })
            .collect(),
    )
}

fn write_json(path: &Path, summary: &Value) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(summary)?)?;
    Ok(())
}

fn write_markdown(path: &Path, summary: &Value) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, markdown_summary(summary)?)?;
    Ok(())
}

fn markdown_summary(summary: &Value) -> Result<String, Box<dyn Error>> {
    let mut output = String::from("# Acceptance Repeat Summary\n\n");
    output.push_str(&format!(
        "- Runs: {}\n",
        summary.get("runCount").and_then(Value::as_u64).unwrap_or(0)
    ));
    let source_lock = required_object(summary, "sourceLock", "summary")?;
    output.push_str(&format!(
        "- vrm-rs HEAD: `{}`\n",
        required_string(source_lock, "vrmRsGitHead")?
    ));
    output.push_str(&format!(
        "- three-vrm HEAD: `{}`\n\n",
        required_string(source_lock, "threeVrmGitHead")?
    ));
    output.push_str("| Fixture / renderer | Runs | Min selected PSNR | Max channel delta | Max alpha mismatches | Max alpha delta |\n");
    output.push_str("| --- | ---: | ---: | ---: | ---: | ---: |\n");
    for comparison in required_array(summary, "comparisons")? {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            required_string(comparison, "name")?,
            comparison.get("runs").and_then(Value::as_u64).unwrap_or(0),
            display_metric(comparison.get("minSelectedPsnr")),
            comparison
                .get("maxChannelDelta")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            comparison
                .get("maxAlphaMismatches")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            comparison
                .get("maxAlphaDelta")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ));
    }
    Ok(output)
}

fn display_metric(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => format!("{:.4}", value.as_f64().unwrap_or(0.0)),
        _ => "n/a".to_owned(),
    }
}

fn ensure_same_value(expected: &Value, actual: &Value, source: &str) -> Result<(), Box<dyn Error>> {
    if expected == actual {
        Ok(())
    } else {
        Err(format!("{source} differs between repeated acceptance manifests").into())
    }
}

fn required_object<'a>(
    value: &'a Value,
    field: &str,
    source: &str,
) -> Result<&'a Value, Box<dyn Error>> {
    value
        .get(field)
        .filter(|field| field.is_object())
        .ok_or_else(|| format!("{source}.{field} must be an object").into())
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

fn required_bool(value: &Value, field: &str) -> Result<bool, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{field} must be a boolean").into())
}

fn require_sha256<'a>(value: &'a Value, source: &str) -> Result<&'a str, Box<dyn Error>> {
    let sha256 = required_string(value, "sourceSha256")?;
    if sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(sha256)
    } else {
        Err(format!("{source}.sourceSha256 must be a 64-character hex digest").into())
    }
}

fn require_nonzero_u64(value: &Value, field: &str, source: &str) -> Result<u64, Box<dyn Error>> {
    let value = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{source}.{field} must be a nonzero integer"))?;
    if value == 0 {
        Err(format!("{source}.{field} must be a nonzero integer").into())
    } else {
        Ok(value)
    }
}

fn summary_string<'a>(summary: &'a Value, field: &str) -> Result<&'a str, Box<dyn Error>> {
    summary
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("summary.{field} must be a string").into())
}

fn test_manifest(artifact: &str) -> Value {
    let fixtures = (0..6)
        .map(|index| {
            let comparisons = ["wgpu", "bevy", "ash"]
                .into_iter()
                .map(|renderer| {
                    serde_json::json!({
                        "renderer": renderer,
                        "visualParityGate": true,
                        "summary": {
                            "pass": true,
                            "selectedPsnr": "34.1234",
                            "maxChannelDelta": "4",
                            "alphaMismatches": "0",
                            "alphaMaxDelta": "0"
                        }
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "name": format!("Fixture-{index}.vrm"),
                "stem": format!("Fixture-{index}"),
                "source": format!(".external-fixtures/official/Fixture-{index}.vrm"),
                "sourceSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "sourceSizeBytes": 1,
                "comparisons": comparisons
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "_manifestPath": format!("{artifact}/review-manifest.json"),
        "artifacts": artifact,
        "runMode": "acceptance",
        "referenceClean": true,
        "metric": "rgb-visible",
        "background": "opaque-black",
        "mtoonLightAccumulation": "three-vrm",
        "diagnosticMode": "final-color",
        "ownerIdPhaseOrderPolicy": "forward",
        "ownerIdColorSource": "vertex",
        "frontFace": "ccw",
        "browserReadyTimeoutMs": 15000,
        "normalMapMode": "generated-tangent",
        "normalMapScale": 1.0,
        "disableTextureMips": false,
        "forceNearestTextures": false,
        "alphaMismatchTolerance": 0,
        "alphaChannelTolerance": 0,
        "sourceLock": {
            "vrmRsGitHead": "0123456789abcdef0123456789abcdef01234567",
            "vrmRsGitDirty": false,
            "threeVrmRoot": ".external-fixtures/three-vrm",
            "threeVrmGitHead": "9d125586f6d7da094b0ac5f204cebf19586f2397",
            "expectedThreeVrmCommit": "9d125586f6d7da094b0ac5f204cebf19586f2397",
            "expectedThreeVrmViewerCommit": "75ab65c9d4e488521d41bff7f5cfd1976a0b16e8",
            "expectedVrmSpecCommit": "3942748efbc803b258e288e0f6c993c6bb96cebf"
        },
        "fixtures": fixtures
    })
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
