#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
vrm-adapter = { path = "../../crates/vrm-adapter" }
---

//! Merge source-derived owner/sample selection manifests by target pixel.
//!
//! This tool intentionally does not read expected or actual color images. It
//! only combines manifests that were produced from owner-id/fill diagnostics,
//! preserving a single renderer override per output pixel.

use clap::{Parser, ValueEnum};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use vrm_adapter::RenderOwnerSampleCorrectionPlan;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "merge-owner-sample-selection",
    about = "Merge owner/sample selection manifests without using RGB distance"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long = "manifest", required_unless_present = "self_test")]
    manifests: Vec<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    manifest_out: Option<PathBuf>,
    #[arg(long)]
    report_out: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = ConflictPolicy::First)]
    conflict_policy: ConflictPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum ConflictPolicy {
    /// Keep the first manifest entry for a pixel and report later conflicts.
    First,
    /// Replace earlier manifest entries with later entries for the same pixel.
    Last,
    /// Fail when the same pixel has different owner/sample entries.
    Error,
}

impl ConflictPolicy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Last => "last",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug)]
struct ManifestInput {
    path: String,
    value: Value,
}

#[derive(Clone, Debug, Serialize)]
struct MergedManifest {
    generator: &'static str,
    selection_mode: &'static str,
    conflict_policy: &'static str,
    sources: Vec<MergedManifestSource>,
    corrections: Vec<Value>,
}

#[derive(Clone, Debug, Serialize)]
struct MergedManifestSource {
    path: String,
    correction_count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct MergeReport {
    generator: &'static str,
    conflict_policy: &'static str,
    input_count: u64,
    total_entry_count: u64,
    merged_entry_count: u64,
    duplicate_pixel_count: u64,
    conflicting_pixel_count: u64,
    replaced_pixel_count: u64,
    skipped_duplicate_pixel_count: u64,
    sources: Vec<MergeSourceReport>,
}

#[derive(Clone, Debug, Serialize)]
struct MergeSourceReport {
    path: String,
    input_entry_count: u64,
    added_entry_count: u64,
    replaced_entry_count: u64,
    skipped_duplicate_entry_count: u64,
    conflicting_entry_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PixelKey {
    x: u64,
    y: u64,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    if options.self_test {
        self_test()?;
        return Ok(());
    }
    if options.manifests.is_empty() {
        return Err("at least one --manifest is required".into());
    }
    let inputs = options
        .manifests
        .iter()
        .map(|path| {
            let value = serde_json::from_str::<Value>(&fs::read_to_string(path)?)?;
            Ok(ManifestInput {
                path: display_path(path),
                value,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let (manifest, report) = merge_manifests(&inputs, options.conflict_policy)?;
    let manifest_json = format!("{}\n", serde_json::to_string_pretty(&manifest)?);
    let manifest_out = options
        .manifest_out
        .as_deref()
        .ok_or("missing --manifest-out")?;
    write_file(manifest_out, &manifest_json)?;
    if let Some(path) = options.report_out {
        let report_json = format!("{}\n", serde_json::to_string_pretty(&report)?);
        write_file(&path, &report_json)?;
    }
    Ok(())
}

fn merge_manifests(
    inputs: &[ManifestInput],
    conflict_policy: ConflictPolicy,
) -> Result<(MergedManifest, MergeReport), Box<dyn Error>> {
    let mut corrections = Vec::<Value>::new();
    let mut pixel_to_index = HashMap::<PixelKey, usize>::new();
    let mut report = MergeReport {
        generator: "vrm-rs tools/render-parity/merge-owner-sample-selection.rs",
        conflict_policy: conflict_policy.as_str(),
        input_count: inputs.len() as u64,
        total_entry_count: 0,
        merged_entry_count: 0,
        duplicate_pixel_count: 0,
        conflicting_pixel_count: 0,
        replaced_pixel_count: 0,
        skipped_duplicate_pixel_count: 0,
        sources: Vec::with_capacity(inputs.len()),
    };
    let mut sources = Vec::with_capacity(inputs.len());

    for input in inputs {
        let input_corrections = manifest_corrections(&input.value, &input.path)?;
        let mut source_report = MergeSourceReport {
            path: input.path.clone(),
            input_entry_count: input_corrections.len() as u64,
            added_entry_count: 0,
            replaced_entry_count: 0,
            skipped_duplicate_entry_count: 0,
            conflicting_entry_count: 0,
        };
        sources.push(MergedManifestSource {
            path: input.path.clone(),
            correction_count: input_corrections.len() as u64,
        });
        report.total_entry_count += input_corrections.len() as u64;

        for (entry_index, correction) in input_corrections.iter().enumerate() {
            let pixel = pixel_key(correction)
                .ok_or_else(|| invalid_pixel_message(&input.path, entry_index))?;
            let Some(existing_index) = pixel_to_index.get(&pixel).copied() else {
                pixel_to_index.insert(pixel, corrections.len());
                corrections.push(correction.clone());
                source_report.added_entry_count += 1;
                continue;
            };

            report.duplicate_pixel_count += 1;
            source_report.skipped_duplicate_entry_count += 1;
            let is_conflict = corrections[existing_index] != *correction;
            if is_conflict {
                report.conflicting_pixel_count += 1;
                source_report.conflicting_entry_count += 1;
            }
            match (conflict_policy, is_conflict) {
                (ConflictPolicy::Error, true) => {
                    return Err(format!(
                        "conflicting correction for pixel {},{} in {} entry {}",
                        pixel.x, pixel.y, input.path, entry_index
                    )
                    .into());
                }
                (ConflictPolicy::Last, _) => {
                    corrections[existing_index] = correction.clone();
                    report.replaced_pixel_count += 1;
                    source_report.replaced_entry_count += 1;
                }
                (ConflictPolicy::First | ConflictPolicy::Error, _) => {
                    report.skipped_duplicate_pixel_count += 1;
                }
            }
        }
        report.sources.push(source_report);
    }

    report.merged_entry_count = corrections.len() as u64;
    let manifest = MergedManifest {
        generator: "vrm-rs tools/render-parity/merge-owner-sample-selection.rs",
        selection_mode: "merged-owner-sample-selection",
        conflict_policy: conflict_policy.as_str(),
        sources,
        corrections,
    };
    Ok((manifest, report))
}

fn manifest_corrections<'a>(
    value: &'a Value,
    path: &str,
) -> Result<&'a Vec<Value>, Box<dyn Error>> {
    value
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{path}: manifest corrections must be an array").into())
}

fn pixel_key(value: &Value) -> Option<PixelKey> {
    Some(PixelKey {
        x: value.get("x")?.as_u64()?,
        y: value.get("y")?.as_u64()?,
    })
}

fn invalid_pixel_message(path: &str, entry_index: usize) -> String {
    format!("{path}: correction {entry_index} must contain u64 x and y")
}

fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn self_test() -> Result<(), Box<dyn Error>> {
    let first = ManifestInput {
        path: "first.json".to_owned(),
        value: json!({
            "corrections": [
                correction(1, 2, [10, 20, 30, 255], "body", 7),
                correction(3, 4, [40, 50, 60, 255], "hair", 9)
            ]
        }),
    };
    let second = ManifestInput {
        path: "second.json".to_owned(),
        value: json!({
            "corrections": [
                correction(1, 2, [99, 20, 30, 255], "body", 7),
                correction(5, 6, [70, 80, 90, 255], "face", 11)
            ]
        }),
    };

    let (first_manifest, first_report) =
        merge_manifests(&[first.clone(), second.clone()], ConflictPolicy::First)?;
    assert_eq!(first_manifest.corrections.len(), 3);
    assert_eq!(first_manifest.corrections[0]["rgba"][0], 10);
    assert_eq!(first_report.duplicate_pixel_count, 1);
    assert_eq!(first_report.conflicting_pixel_count, 1);
    assert_eq!(first_report.skipped_duplicate_pixel_count, 1);
    RenderOwnerSampleCorrectionPlan::from_manifest_value(&serde_json::to_value(&first_manifest)?)?;

    let (last_manifest, last_report) =
        merge_manifests(&[first.clone(), second.clone()], ConflictPolicy::Last)?;
    assert_eq!(last_manifest.corrections[0]["rgba"][0], 99);
    assert_eq!(last_report.replaced_pixel_count, 1);
    RenderOwnerSampleCorrectionPlan::from_manifest_value(&serde_json::to_value(&last_manifest)?)?;

    let error = merge_manifests(&[first, second], ConflictPolicy::Error).unwrap_err();
    assert!(error.to_string().contains("conflicting correction"));
    Ok(())
}

fn correction(x: u64, y: u64, rgba: [u8; 4], material_name: &str, triangle: u64) -> Value {
    json!({
        "x": x,
        "y": y,
        "rgba": rgba,
        "surface": {
            "materialName": material_name,
            "triangle": triangle
        },
        "sample": [0.5, 0.5],
        "sample_geometry": {
            "node": 0,
            "mesh": 0,
            "primitive": 0,
            "triangle": triangle,
            "indices": [0, 1, 2],
            "barycentric": [0.2, 0.3, 0.5],
            "raw_uv": [0.25, 0.75],
            "base_uv": [0.25, 0.75],
            "depth": 0.5,
            "pass": "base"
        }
    })
}
