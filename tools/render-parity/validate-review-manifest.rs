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
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    allow_failed: bool,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    let text = fs::read_to_string(&options.manifest)?;
    let manifest = serde_json::from_str::<Value>(&text)?;
    let base_dir = options
        .manifest
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    validate_manifest(&manifest, base_dir, options.allow_failed)?;
    println!(
        "validated render parity review manifest: {}",
        display_path(&options.manifest)
    );
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
    validate_artifact_group(
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
            &format!("{source}.comparisons[{comparison_index}]"),
            allow_failed,
        )?;
    }
    Ok(())
}

fn validate_comparison(
    comparison: &Value,
    base_dir: &Path,
    source: &str,
    allow_failed: bool,
) -> Result<(), Box<dyn Error>> {
    expect_object(comparison, source)?;
    require_string(comparison, "renderer")?;
    validate_artifact_group(
        required_object(comparison, "capture", source)?,
        base_dir,
        &format!("{source}.capture"),
    )?;
    let numeric_report_path = require_existing_path(comparison, base_dir, "numericReport")?;
    require_existing_path(comparison, base_dir, "diagnosticReport")?;
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
    Ok(())
}

fn validate_artifact_group(
    group: &Value,
    base_dir: &Path,
    source: &str,
) -> Result<(), Box<dyn Error>> {
    expect_object(group, source)?;
    for field in ["rgbaJson", "imqraw", "png"] {
        require_existing_path(group, base_dir, field)?;
    }
    Ok(())
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
