#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Audit how much of a delta report is covered by an owner/sample manifest.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "audit-owner-sample-selection",
    about = "Measure whether render residual pixels are covered by an owner/sample selection manifest"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    manifest: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    deltas: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long, default_value_t = 16)]
    top_missing: usize,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionCoverageReport {
    manifest: String,
    deltas: String,
    delta_count: u64,
    manifest_count: u64,
    selected_delta_count: u64,
    missing_delta_count: u64,
    selected_delta_percent: f64,
    selected_rgb_distance_mean: Option<f64>,
    missing_rgb_distance_mean: Option<f64>,
    selected_max_channel_delta_max: Option<u64>,
    missing_max_channel_delta_max: Option<u64>,
    top_missing: Vec<MissingDelta>,
}

#[derive(Clone, Debug, Serialize)]
struct MissingDelta {
    x: u64,
    y: u64,
    expected: [u8; 4],
    actual: [u8; 4],
    max_channel_delta: u64,
    rgb_distance: f64,
}

#[derive(Clone, Copy, Debug)]
struct DeltaStats {
    count: u64,
    rgb_distance_sum: f64,
    max_channel_delta: Option<u64>,
}

impl DeltaStats {
    fn add(&mut self, delta: &Value) {
        self.count += 1;
        self.rgb_distance_sum += delta.get("rgbDistance").and_then(Value::as_f64).unwrap_or(0.0);
        if let Some(max_channel_delta) = delta.get("maxChannelDelta").and_then(Value::as_u64) {
            self.max_channel_delta = Some(
                self.max_channel_delta
                    .map(|current| current.max(max_channel_delta))
                    .unwrap_or(max_channel_delta),
            );
        }
    }

    fn mean(self) -> Option<f64> {
        (self.count > 0).then_some(self.rgb_distance_sum / self.count as f64)
    }
}

impl Default for DeltaStats {
    fn default() -> Self {
        Self {
            count: 0,
            rgb_distance_sum: 0.0,
            max_channel_delta: None,
        }
    }
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
    let manifest_path = options
        .manifest
        .as_deref()
        .ok_or("missing --manifest")?;
    let deltas_path = options.deltas.as_deref().ok_or("missing --deltas")?;
    let manifest = serde_json::from_str::<Value>(&fs::read_to_string(manifest_path)?)?;
    let deltas = serde_json::from_str::<Value>(&fs::read_to_string(deltas_path)?)?;
    let report = audit(manifest_path, deltas_path, &manifest, &deltas, options.top_missing)?;
    let json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = options.out {
        write_file(&path, &json)?;
    } else {
        print!("{json}");
    }
    Ok(())
}

fn audit(
    manifest_path: &Path,
    deltas_path: &Path,
    manifest: &Value,
    deltas: &Value,
    top_missing: usize,
) -> Result<SelectionCoverageReport, Box<dyn Error>> {
    let corrections = manifest
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or("manifest corrections must be an array")?;
    let selected = corrections
        .iter()
        .filter_map(|correction| pixel_key(correction).map(|pixel| (pixel, ())))
        .collect::<HashMap<_, _>>();
    let deltas = deltas
        .get("top")
        .and_then(Value::as_array)
        .ok_or("deltas top must be an array")?;

    let mut selected_stats = DeltaStats::default();
    let mut missing_stats = DeltaStats::default();
    let mut missing = Vec::new();

    for delta in deltas {
        let Some(pixel) = pixel_key(delta) else {
            continue;
        };
        if selected.contains_key(&pixel) {
            selected_stats.add(delta);
        } else {
            missing_stats.add(delta);
            if let Some(missing_delta) = missing_delta(delta) {
                missing.push(missing_delta);
            }
        }
    }
    missing.sort_by(|left, right| {
        right
            .max_channel_delta
            .cmp(&left.max_channel_delta)
            .then_with(|| {
                right
                    .rgb_distance
                    .partial_cmp(&left.rgb_distance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.y.cmp(&right.y))
            .then_with(|| left.x.cmp(&right.x))
    });
    missing.truncate(top_missing);

    let delta_count = deltas.len() as u64;
    let selected_delta_count = selected_stats.count;
    let missing_delta_count = missing_stats.count;
    Ok(SelectionCoverageReport {
        manifest: display_path(manifest_path),
        deltas: display_path(deltas_path),
        delta_count,
        manifest_count: corrections.len() as u64,
        selected_delta_count,
        missing_delta_count,
        selected_delta_percent: if delta_count == 0 {
            100.0
        } else {
            selected_delta_count as f64 * 100.0 / delta_count as f64
        },
        selected_rgb_distance_mean: selected_stats.mean(),
        missing_rgb_distance_mean: missing_stats.mean(),
        selected_max_channel_delta_max: selected_stats.max_channel_delta,
        missing_max_channel_delta_max: missing_stats.max_channel_delta,
        top_missing: missing,
    })
}

fn pixel_key(value: &Value) -> Option<(u64, u64)> {
    Some((value.get("x")?.as_u64()?, value.get("y")?.as_u64()?))
}

fn missing_delta(value: &Value) -> Option<MissingDelta> {
    Some(MissingDelta {
        x: value.get("x")?.as_u64()?,
        y: value.get("y")?.as_u64()?,
        expected: rgba(value.get("expected")?)?,
        actual: rgba(value.get("actual")?)?,
        max_channel_delta: value.get("maxChannelDelta")?.as_u64()?,
        rgb_distance: value.get("rgbDistance")?.as_f64()?,
    })
}

fn rgba(value: &Value) -> Option<[u8; 4]> {
    let values = value.as_array()?;
    Some([
        u8::try_from(values.first()?.as_u64()?).ok()?,
        u8::try_from(values.get(1)?.as_u64()?).ok()?,
        u8::try_from(values.get(2)?.as_u64()?).ok()?,
        u8::try_from(values.get(3)?.as_u64()?).ok()?,
    ])
}

fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn self_test() -> Result<(), Box<dyn Error>> {
    let manifest = serde_json::json!({
        "corrections": [
            {"x": 1, "y": 2, "rgba": [0, 0, 0, 255], "surface": {"materialName": "a", "triangle": 0}, "sample": [0.5, 0.5]}
        ]
    });
    let deltas = serde_json::json!({
        "top": [
            {"x": 1, "y": 2, "expected": [1, 2, 3, 255], "actual": [4, 5, 6, 255], "maxChannelDelta": 3, "rgbDistance": 5.0},
            {"x": 3, "y": 4, "expected": [10, 20, 30, 255], "actual": [40, 50, 60, 255], "maxChannelDelta": 30, "rgbDistance": 52.0}
        ]
    });
    let report = audit(
        Path::new("manifest.json"),
        Path::new("deltas.json"),
        &manifest,
        &deltas,
        8,
    )?;
    assert_eq!(report.delta_count, 2);
    assert_eq!(report.manifest_count, 1);
    assert_eq!(report.selected_delta_count, 1);
    assert_eq!(report.missing_delta_count, 1);
    assert_eq!(report.selected_delta_percent, 50.0);
    assert_eq!(report.selected_rgb_distance_mean, Some(5.0));
    assert_eq!(report.missing_rgb_distance_mean, Some(52.0));
    assert_eq!(report.selected_max_channel_delta_max, Some(3));
    assert_eq!(report.missing_max_channel_delta_max, Some(30));
    assert_eq!(report.top_missing.len(), 1);
    assert_eq!(report.top_missing[0].x, 3);
    assert_eq!(report.top_missing[0].expected, [10, 20, 30, 255]);
    Ok(())
}
