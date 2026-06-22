#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

//! Validate a render-parity `review-manifest.json` artifact set.

use clap::Parser;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "validate-review-manifest",
    about = "Validate render parity review-manifest.json paths and pass/fail summaries"
)]
struct Options {
    #[arg(long, required_unless_present = "self_test")]
    manifest: Option<PathBuf>,
    #[arg(long)]
    allow_failed: bool,
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

    let manifest_path = options
        .manifest
        .as_ref()
        .ok_or("--manifest is required unless --self-test is supplied")?;
    let text = fs::read_to_string(manifest_path)?;
    let manifest = serde_json::from_str::<Value>(&text)?;
    let base_dir = manifest_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    validate_manifest(&manifest, base_dir, options.allow_failed)?;
    println!(
        "validated render parity review manifest: {}",
        display_path(manifest_path)
    );
    Ok(())
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    validate_run_mode_contract(&serde_json::json!({
        "runMode": "acceptance",
        "referenceClean": true
    }))?;
    validate_run_mode_contract(&serde_json::json!({
        "runMode": "diagnostic",
        "referenceClean": false
    }))?;
    validate_run_mode_contract(&serde_json::json!({
        "runMode": "experiment",
        "referenceClean": false
    }))?;
    validate_source_lock(&serde_json::json!({
        "sourceLock": {
            "vrmRsGitHead": "0123456789abcdef0123456789abcdef01234567",
            "vrmRsGitDirty": false,
            "threeVrmRoot": ".external-fixtures/three-vrm",
            "threeVrmGitHead": null,
            "expectedThreeVrmCommit": "9d125586f6d7da094b0ac5f204cebf19586f2397",
            "expectedThreeVrmViewerCommit": "75ab65c9d4e488521d41bff7f5cfd1976a0b16e8",
            "expectedVrmSpecCommit": "3942748efbc803b258e288e0f6c993c6bb96cebf"
        }
    }))?;
    validate_environment_lock(&serde_json::json!({
        "environmentLock": {
            "os": "windows",
            "family": "windows",
            "arch": "x86_64",
            "rustcVersion": "rustc 1.90.0",
            "cargoVersion": "cargo 1.90.0",
            "nodeVersion": "v25.0.0",
            "npmVersion": null,
            "justVersion": "just 1.42.0",
            "gpuAdapters": [{"Name": "Adapter", "DriverVersion": "1.2.3"}]
        }
    }))?;
    validate_fixture_source_hash(
        &serde_json::json!({
            "sourceSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "sourceSizeBytes": 1
        }),
        "self-test.fixture",
    )?;

    for (label, manifest) in [
        (
            "acceptance without referenceClean",
            serde_json::json!({
                "runMode": "acceptance",
                "referenceClean": false
            }),
        ),
        (
            "diagnostic with referenceClean",
            serde_json::json!({
                "runMode": "diagnostic",
                "referenceClean": true
            }),
        ),
        (
            "unknown runMode",
            serde_json::json!({
                "runMode": "trial",
                "referenceClean": false
            }),
        ),
    ] {
        if validate_run_mode_contract(&manifest).is_ok() {
            return Err(format!("{label} should be rejected").into());
        }
    }

    Ok(())
}

fn validate_manifest(
    manifest: &Value,
    base_dir: &Path,
    allow_failed: bool,
) -> Result<(), Box<dyn Error>> {
    expect_object(manifest, "manifest")?;
    require_existing_path(manifest, base_dir, "summary")?;
    require_existing_path(manifest, base_dir, "visualReview")?;
    require_existing_path(manifest, base_dir, "artifacts")?;
    require_string(manifest, "numericGate")?;
    validate_run_mode_contract(manifest)?;
    validate_source_lock(manifest)?;
    validate_environment_lock(manifest)?;
    require_string(manifest, "metric")?;

    let fixtures = manifest
        .get("fixtures")
        .and_then(Value::as_array)
        .ok_or("manifest.fixtures must be an array")?;
    if fixtures.is_empty() {
        return Err("manifest.fixtures must not be empty".into());
    }
    for (index, fixture) in fixtures.iter().enumerate() {
        validate_fixture(fixture, base_dir, index, allow_failed)?;
    }
    Ok(())
}

fn validate_run_mode_contract(manifest: &Value) -> Result<(), Box<dyn Error>> {
    let run_mode = require_string(manifest, "runMode")?;
    let reference_clean = require_bool(manifest, "referenceClean")?;
    match run_mode {
        "acceptance" if reference_clean => Ok(()),
        "acceptance" => Err("acceptance manifests must have referenceClean=true".into()),
        "diagnostic" | "experiment" if !reference_clean => Ok(()),
        "diagnostic" | "experiment" => {
            Err(format!("{run_mode} manifests must have referenceClean=false").into())
        }
        _ => Err(format!("runMode must be acceptance, diagnostic, or experiment, got {run_mode}")
            .into()),
    }
}

fn validate_source_lock(manifest: &Value) -> Result<(), Box<dyn Error>> {
    let source_lock = required_object(manifest, "sourceLock", "manifest")?;
    require_string_or_null(source_lock, "vrmRsGitHead")?;
    require_bool_or_null(source_lock, "vrmRsGitDirty")?;
    require_string(source_lock, "threeVrmRoot")?;
    require_string_or_null(source_lock, "threeVrmGitHead")?;
    for field in [
        "expectedThreeVrmCommit",
        "expectedThreeVrmViewerCommit",
        "expectedVrmSpecCommit",
    ] {
        require_string(source_lock, field)?;
    }
    Ok(())
}

fn validate_environment_lock(manifest: &Value) -> Result<(), Box<dyn Error>> {
    let environment_lock = required_object(manifest, "environmentLock", "manifest")?;
    for field in ["os", "family", "arch"] {
        require_string(environment_lock, field)?;
    }
    for field in [
        "rustcVersion",
        "cargoVersion",
        "nodeVersion",
        "npmVersion",
        "justVersion",
    ] {
        require_string_or_null(environment_lock, field)?;
    }
    validate_gpu_adapters(environment_lock, "environmentLock")?;
    Ok(())
}

fn validate_gpu_adapters(value: &Value, source: &str) -> Result<(), Box<dyn Error>> {
    match value.get("gpuAdapters") {
        Some(Value::Null | Value::Array(_)) => Ok(()),
        Some(_) => Err(format!("{source}.gpuAdapters must be null or an array").into()),
        None => Err(format!("{source}.gpuAdapters is missing").into()),
    }
}

fn validate_fixture_source_hash(fixture: &Value, source: &str) -> Result<(), Box<dyn Error>> {
    let sha256 = require_string(fixture, "sourceSha256")?;
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{source}.sourceSha256 must be a 64-character hex digest").into());
    }
    let size = fixture
        .get("sourceSizeBytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{source}.sourceSizeBytes must be a nonzero integer"))?;
    if size == 0 {
        return Err(format!("{source}.sourceSizeBytes must be a nonzero integer").into());
    }
    Ok(())
}

fn validate_fixture(
    fixture: &Value,
    base_dir: &Path,
    index: usize,
    allow_failed: bool,
) -> Result<(), Box<dyn Error>> {
    let source = format!("manifest.fixtures[{index}]");
    expect_object(fixture, &source)?;
    require_string(fixture, "name")?;
    require_string(fixture, "stem")?;
    require_existing_path(fixture, base_dir, "source")?;
    validate_fixture_source_hash(fixture, &source)?;
    let reference = artifact_group_paths(
        required_object(fixture, "reference", &source)?,
        base_dir,
        &format!("{source}.reference"),
    )?;

    let comparisons = fixture
        .get("comparisons")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{source}.comparisons must be an array"))?;
    if comparisons.is_empty() {
        return Err(format!("{source}.comparisons must not be empty").into());
    }
    for (comparison_index, comparison) in comparisons.iter().enumerate() {
        validate_comparison(
            comparison,
            base_dir,
            &reference,
            &format!("{source}.comparisons[{comparison_index}]"),
            allow_failed,
        )?;
    }
    Ok(())
}

fn validate_comparison(
    comparison: &Value,
    base_dir: &Path,
    reference: &ArtifactPaths,
    source: &str,
    allow_failed: bool,
) -> Result<(), Box<dyn Error>> {
    expect_object(comparison, source)?;
    require_string(comparison, "renderer")?;
    comparison
        .get("visualParityGate")
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{source}.visualParityGate must be a boolean"))?;
    let capture = artifact_group_paths(
        required_object(comparison, "capture", source)?,
        base_dir,
        &format!("{source}.capture"),
    )?;
    let numeric_report_path = require_existing_path(comparison, base_dir, "numericReport")?;
    let diagnostic_report_path = require_existing_path(comparison, base_dir, "diagnosticReport")?;
    require_existing_path(comparison, base_dir, "diffPng")?;

    let summary = required_object(comparison, "summary", source)?;
    let manifest_pass = summary
        .get("pass")
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{source}.summary.pass must be a boolean"))?;
    if !allow_failed && !manifest_pass {
        return Err(format!("{source}.summary.pass is false").into());
    }
    for field in [
        "selectedPsnr",
        "maxChannelDelta",
        "alphaMismatches",
        "alphaMaxDelta",
    ] {
        require_string(summary, field)?;
    }

    let report_text = fs::read_to_string(&numeric_report_path)?;
    let report = serde_json::from_str::<Value>(&report_text)?;
    validate_report_paths(
        &report,
        &reference.imqraw,
        &capture.imqraw,
        &numeric_report_path,
        source,
    )?;
    let report_pass = report.get("pass").and_then(Value::as_bool).ok_or_else(|| {
        format!(
            "{}: pass must be a boolean",
            display_path(&numeric_report_path)
        )
    })?;
    if report_pass != manifest_pass {
        return Err(format!(
            "{source}.summary.pass ({manifest_pass}) does not match numeric report pass ({report_pass})"
        )
        .into());
    }
    if !allow_failed && !report_pass {
        return Err(format!(
            "{}: numeric report pass is false",
            display_path(&numeric_report_path)
        )
        .into());
    }
    let selected = report
        .get("selectedMetric")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!(
                "{}: selectedMetric must be an object",
                display_path(&numeric_report_path)
            )
        })?;
    for field in ["name", "psnr", "maxChannelDelta"] {
        if !selected.contains_key(field) {
            return Err(format!(
                "{}: selectedMetric.{field} is missing",
                display_path(&numeric_report_path)
            )
            .into());
        }
    }
    validate_summary_matches_report(summary, &report, &numeric_report_path, source)?;

    let diagnostic_text = fs::read_to_string(&diagnostic_report_path)?;
    let diagnostic_report = serde_json::from_str::<Value>(&diagnostic_text)?;
    validate_report_paths(
        &diagnostic_report,
        &reference.rgba_json,
        &capture.rgba_json,
        &diagnostic_report_path,
        source,
    )?;
    Ok(())
}

#[derive(Clone, Debug)]
struct ArtifactPaths {
    rgba_json: PathBuf,
    imqraw: PathBuf,
}

fn artifact_group_paths(
    group: &Value,
    base_dir: &Path,
    source: &str,
) -> Result<ArtifactPaths, Box<dyn Error>> {
    expect_object(group, source)?;
    let rgba_json = require_existing_path(group, base_dir, "rgbaJson")?;
    let imqraw = require_existing_path(group, base_dir, "imqraw")?;
    require_existing_path(group, base_dir, "png")?;
    Ok(ArtifactPaths { rgba_json, imqraw })
}

fn validate_report_paths(
    report: &Value,
    expected: &Path,
    actual: &Path,
    report_path: &Path,
    source: &str,
) -> Result<(), Box<dyn Error>> {
    let report_expected = report_path_field(report, "expected", report_path)?;
    let report_actual = report_path_field(report, "actual", report_path)?;
    ensure_same_path(
        &report_expected,
        expected,
        &format!("{source}.expected report path"),
    )?;
    ensure_same_path(
        &report_actual,
        actual,
        &format!("{source}.actual report path"),
    )?;
    Ok(())
}

fn report_path_field(
    report: &Value,
    field: &str,
    report_path: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    let raw = report
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{}: {field} must be a string", display_path(report_path)))?;
    Ok(PathBuf::from(raw))
}

fn ensure_same_path(left: &Path, right: &Path, source: &str) -> Result<(), Box<dyn Error>> {
    let left = canonical_path(left)?;
    let right = canonical_path(right)?;
    if left == right {
        Ok(())
    } else {
        Err(format!(
            "{source} mismatch: left={}, right={}",
            left.display(),
            right.display()
        )
        .into())
    }
}

fn canonical_path(path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    path.canonicalize()
        .map_err(|err| format!("failed to canonicalize {}: {err}", path.display()).into())
}

fn validate_summary_matches_report(
    summary: &Value,
    report: &Value,
    report_path: &Path,
    source: &str,
) -> Result<(), Box<dyn Error>> {
    let selected = report
        .get("selectedMetric")
        .ok_or_else(|| format!("{}: selectedMetric is missing", display_path(report_path)))?;
    let alpha = report
        .get("alpha")
        .ok_or_else(|| format!("{}: alpha is missing", display_path(report_path)))?;
    ensure_summary_field(
        summary,
        "selectedPsnr",
        &report_f64_string(selected, "psnr", report_path)?,
        source,
    )?;
    ensure_summary_field(
        summary,
        "maxChannelDelta",
        &report_u64_string(selected, "maxChannelDelta", report_path)?,
        source,
    )?;
    ensure_summary_field(
        summary,
        "alphaMismatches",
        &report_u64_string(alpha, "mismatches", report_path)?,
        source,
    )?;
    ensure_summary_field(
        summary,
        "alphaMaxDelta",
        &report_u64_string(alpha, "maxDelta", report_path)?,
        source,
    )?;
    Ok(())
}

fn ensure_summary_field(
    summary: &Value,
    field: &str,
    expected: &str,
    source: &str,
) -> Result<(), Box<dyn Error>> {
    let actual = summary
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{source}.summary.{field} must be a string"))?;
    if actual == expected {
        Ok(())
    } else {
        Err(
            format!("{source}.summary.{field} mismatch: manifest={actual}, report={expected}")
                .into(),
        )
    }
}

fn report_f64_string(
    value: &Value,
    field: &str,
    report_path: &Path,
) -> Result<String, Box<dyn Error>> {
    let Some(value) = value.get(field) else {
        return Err(format!("{}: {field} is missing", display_path(report_path)).into());
    };
    if value.is_null() || value.as_str() == Some("Infinity") {
        return Ok("Infinity".to_owned());
    }
    let value = value
        .as_f64()
        .ok_or_else(|| format!("{}: {field} must be a number", display_path(report_path)))?;
    Ok(format!("{value:.4}"))
}

fn report_u64_string(
    value: &Value,
    field: &str,
    report_path: &Path,
) -> Result<String, Box<dyn Error>> {
    let value = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{}: {field} must be an integer", display_path(report_path)))?;
    Ok(value.to_string())
}

fn required_object<'a>(
    value: &'a Value,
    field: &str,
    source: &str,
) -> Result<&'a Value, Box<dyn Error>> {
    let value = value
        .get(field)
        .ok_or_else(|| format!("{source}.{field} is missing"))?;
    expect_object(value, &format!("{source}.{field}"))?;
    Ok(value)
}

fn expect_object(value: &Value, source: &str) -> Result<(), Box<dyn Error>> {
    if value.is_object() {
        Ok(())
    } else {
        Err(format!("{source} must be an object").into())
    }
}

fn require_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} must be a string").into())
}

fn require_bool(value: &Value, field: &str) -> Result<bool, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{field} must be a boolean").into())
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

fn require_bool_or_null(value: &Value, field: &str) -> Result<Option<bool>, Box<dyn Error>> {
    match value.get(field) {
        Some(Value::Null) => Ok(None),
        Some(field_value) => field_value
            .as_bool()
            .map(Some)
            .ok_or_else(|| format!("{field} must be a boolean or null").into()),
        None => Err(format!("{field} must be a boolean or null").into()),
    }
}

fn require_existing_path(
    value: &Value,
    base_dir: &Path,
    field: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let raw = require_string(value, field)?;
    let path = resolve_manifest_path(base_dir, raw);
    if !path.exists() {
        return Err(format!("{field} path does not exist: {}", path.display()).into());
    }
    Ok(path)
}

fn resolve_manifest_path(base_dir: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() || path.exists() {
        path
    } else {
        base_dir.join(path)
    }
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
