#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Audit whether an owner/sample manifest is reflected in a rendered image.
//!
//! This is intentionally a post-render diagnostic. It never selects samples or
//! changes renderer behavior; it only compares each manifest entry's target
//! pixel with a concrete `.rgba.json` readback.

use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "audit-owner-sample-execution",
    about = "Audit owner/sample manifest execution against a rendered RGBA JSON image"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    manifest: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    actual_rgba_json: Option<PathBuf>,
    #[arg(long)]
    expected_rgba_json: Option<PathBuf>,
    #[arg(long, default_value_t = 1.5)]
    tolerance: f64,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
struct RgbaJsonImage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl RgbaJsonImage {
    fn pixel(&self, x: u64, y: u64) -> Option<[u8; 4]> {
        let x = usize::try_from(x).ok()?;
        let y = usize::try_from(y).ok()?;
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = (y * self.width + x) * 4;
        Some([
            *self.rgba.get(index)?,
            *self.rgba.get(index + 1)?,
            *self.rgba.get(index + 2)?,
            *self.rgba.get(index + 3)?,
        ])
    }
}

#[derive(Clone, Debug, Serialize)]
struct AuditReport {
    manifest: String,
    actual_rgba_json: String,
    expected_rgba_json: Option<String>,
    tolerance: f64,
    totals: AuditBucket,
    by_selection_source: BTreeMap<String, AuditBucket>,
    by_material: BTreeMap<String, AuditBucket>,
    sample_closer_by_material: Vec<SampleCloserBucket>,
    sample_closer_by_material_source: Vec<SampleCloserBucket>,
    top_actual_sample_misses: Vec<AuditRow>,
    top_sample_closer_to_expected: Vec<AuditRow>,
}

#[derive(Clone, Debug, Serialize)]
struct SampleCloserBucket {
    label: String,
    entries: u64,
    mean_sample_expected_margin: f64,
    max_sample_expected_margin: f64,
    mean_expected_sample_distance: f64,
    mean_actual_expected_distance: f64,
    top_pixel: Option<AuditRow>,
}

#[derive(Clone, Debug, Default)]
struct SampleCloserAccumulator {
    entries: u64,
    margin_sum: f64,
    expected_sample_distance_sum: f64,
    actual_expected_distance_sum: f64,
    max_margin: f64,
    top_pixel: Option<AuditRow>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct AuditBucket {
    manifest_entries: u64,
    actual_pixels_present: u64,
    expected_pixels_present: u64,
    actual_sample_exact: u64,
    actual_sample_within_tolerance: u64,
    expected_sample_exact: u64,
    expected_sample_within_tolerance: u64,
    actual_expected_exact: u64,
    actual_expected_within_tolerance: u64,
    actual_closer_to_expected_than_sample: u64,
    sample_closer_to_expected_than_actual: u64,
    actual_sample_expected_tie: u64,
    mean_actual_sample_distance: Option<f64>,
    mean_expected_sample_distance: Option<f64>,
    mean_actual_expected_distance: Option<f64>,
    max_actual_sample_distance: Option<f64>,
    max_expected_sample_distance: Option<f64>,
    max_actual_expected_distance: Option<f64>,
}

#[derive(Clone, Debug, Default)]
struct BucketAccumulator {
    bucket: AuditBucket,
    actual_sample_distance_sum: f64,
    expected_sample_distance_sum: f64,
    actual_expected_distance_sum: f64,
}

#[derive(Clone, Debug, Serialize)]
struct AuditRow {
    x: u64,
    y: u64,
    material_name: String,
    triangle: Option<u64>,
    selection_source: String,
    manifest_rgba: [u8; 4],
    actual_rgba: Option<[u8; 4]>,
    expected_rgba: Option<[u8; 4]>,
    actual_sample_distance: Option<f64>,
    expected_sample_distance: Option<f64>,
    actual_expected_distance: Option<f64>,
    expected_closeness: Option<ExpectedCloseness>,
}

#[derive(Clone, Debug)]
struct ManifestEntry {
    x: u64,
    y: u64,
    rgba: [u8; 4],
    material_name: String,
    triangle: Option<u64>,
    selection_source: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ExpectedCloseness {
    Actual,
    Sample,
    Tie,
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
    let manifest_path = options.manifest.as_ref().ok_or("missing --manifest")?;
    let actual_path = options
        .actual_rgba_json
        .as_ref()
        .ok_or("missing --actual-rgba-json")?;
    let manifest = serde_json::from_str::<Value>(&fs::read_to_string(manifest_path)?)?;
    let actual = read_rgba_json(actual_path)?;
    let expected = options
        .expected_rgba_json
        .as_ref()
        .map(|path| read_rgba_json(path))
        .transpose()?;
    let report = audit(
        manifest_path,
        actual_path,
        options.expected_rgba_json.as_deref(),
        options.tolerance,
        &manifest,
        &actual,
        expected.as_ref(),
    )?;
    let json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = &options.json_out {
        write_file(path, &json)?;
    } else {
        print!("{json}");
    }
    if let Some(path) = &options.markdown_out {
        write_file(path, &markdown(&report))?;
    }
    Ok(())
}

fn audit(
    manifest_path: &Path,
    actual_path: &Path,
    expected_path: Option<&Path>,
    tolerance: f64,
    manifest: &Value,
    actual: &RgbaJsonImage,
    expected: Option<&RgbaJsonImage>,
) -> Result<AuditReport, Box<dyn Error>> {
    let entries = manifest_entries(manifest)?;
    let mut totals = BucketAccumulator::default();
    let mut by_selection_source = BTreeMap::<String, BucketAccumulator>::new();
    let mut by_material = BTreeMap::<String, BucketAccumulator>::new();
    let mut rows = Vec::<AuditRow>::with_capacity(entries.len());

    for entry in entries {
        let actual_rgba = actual.pixel(entry.x, entry.y);
        let expected_rgba = expected.and_then(|image| image.pixel(entry.x, entry.y));
        let row = AuditRow {
            x: entry.x,
            y: entry.y,
            material_name: entry.material_name.clone(),
            triangle: entry.triangle,
            selection_source: entry.selection_source.clone(),
            manifest_rgba: entry.rgba,
            actual_rgba,
            expected_rgba,
            actual_sample_distance: actual_rgba.map(|actual| rgb_distance(actual, entry.rgba)),
            expected_sample_distance: expected_rgba.map(|expected| rgb_distance(expected, entry.rgba)),
            actual_expected_distance: actual_rgba
                .zip(expected_rgba)
                .map(|(actual, expected)| rgb_distance(actual, expected)),
            expected_closeness: expected_closeness(
                actual_rgba,
                Some(entry.rgba),
                expected_rgba,
            ),
        };
        totals.push(&row, tolerance);
        by_selection_source
            .entry(row.selection_source.clone())
            .or_default()
            .push(&row, tolerance);
        by_material
            .entry(row.material_name.clone())
            .or_default()
            .push(&row, tolerance);
        rows.push(row);
    }

    let sample_closer_by_material =
        sample_closer_buckets(&rows, |row| row.material_name.clone());
    let sample_closer_by_material_source = sample_closer_buckets(&rows, |row| {
        format!("{} / {}", row.material_name, row.selection_source)
    });

    let mut top_sample_closer_to_expected = rows
        .iter()
        .filter(|row| row.expected_closeness == Some(ExpectedCloseness::Sample))
        .cloned()
        .collect::<Vec<_>>();
    top_sample_closer_to_expected.sort_by(|left, right| {
        right
            .sample_expected_margin()
            .partial_cmp(&left.sample_expected_margin())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    top_sample_closer_to_expected.truncate(16);

    rows.sort_by(|left, right| {
        right
            .actual_sample_distance
            .partial_cmp(&left.actual_sample_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(16);

    Ok(AuditReport {
        manifest: display_path(manifest_path),
        actual_rgba_json: display_path(actual_path),
        expected_rgba_json: expected_path.map(display_path),
        tolerance,
        totals: totals.finish(),
        by_selection_source: by_selection_source
            .into_iter()
            .map(|(key, value)| (key, value.finish()))
            .collect(),
        by_material: by_material
            .into_iter()
            .map(|(key, value)| (key, value.finish()))
            .collect(),
        sample_closer_by_material,
        sample_closer_by_material_source,
        top_actual_sample_misses: rows,
        top_sample_closer_to_expected,
    })
}

impl AuditRow {
    fn sample_expected_margin(&self) -> Option<f64> {
        Some(self.actual_expected_distance? - self.expected_sample_distance?)
    }
}

impl SampleCloserAccumulator {
    fn push(&mut self, row: &AuditRow) {
        let Some(margin) = row.sample_expected_margin() else {
            return;
        };
        self.entries += 1;
        self.margin_sum += margin;
        self.max_margin = self.max_margin.max(margin);
        self.expected_sample_distance_sum += row.expected_sample_distance.unwrap_or_default();
        self.actual_expected_distance_sum += row.actual_expected_distance.unwrap_or_default();
        if self
            .top_pixel
            .as_ref()
            .and_then(AuditRow::sample_expected_margin)
            .is_none_or(|current_margin| margin > current_margin)
        {
            self.top_pixel = Some(row.clone());
        }
    }

    fn finish(self, label: String) -> SampleCloserBucket {
        let entries = self.entries.max(1);
        SampleCloserBucket {
            label,
            entries: self.entries,
            mean_sample_expected_margin: self.margin_sum / entries as f64,
            max_sample_expected_margin: self.max_margin,
            mean_expected_sample_distance: self.expected_sample_distance_sum / entries as f64,
            mean_actual_expected_distance: self.actual_expected_distance_sum / entries as f64,
            top_pixel: self.top_pixel,
        }
    }
}

fn sample_closer_buckets(
    rows: &[AuditRow],
    key: impl Fn(&AuditRow) -> String,
) -> Vec<SampleCloserBucket> {
    let mut accumulators = BTreeMap::<String, SampleCloserAccumulator>::new();
    for row in rows
        .iter()
        .filter(|row| row.expected_closeness == Some(ExpectedCloseness::Sample))
    {
        accumulators.entry(key(row)).or_default().push(row);
    }
    let mut buckets = accumulators
        .into_iter()
        .map(|(label, accumulator)| accumulator.finish(label))
        .collect::<Vec<_>>();
    buckets.sort_by(|left, right| {
        right
            .entries
            .cmp(&left.entries)
            .then_with(|| {
                right
                    .mean_sample_expected_margin
                    .partial_cmp(&left.mean_sample_expected_margin)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.label.cmp(&right.label))
    });
    buckets
}

impl BucketAccumulator {
    fn push(&mut self, row: &AuditRow, tolerance: f64) {
        self.bucket.manifest_entries += 1;
        if let Some(distance) = row.actual_sample_distance {
            self.bucket.actual_pixels_present += 1;
            self.actual_sample_distance_sum += distance;
            self.bucket.max_actual_sample_distance =
                Some(max_f64(self.bucket.max_actual_sample_distance, distance));
            if distance == 0.0 {
                self.bucket.actual_sample_exact += 1;
            }
            if distance <= tolerance {
                self.bucket.actual_sample_within_tolerance += 1;
            }
        }
        if let Some(distance) = row.expected_sample_distance {
            self.bucket.expected_pixels_present += 1;
            self.expected_sample_distance_sum += distance;
            self.bucket.max_expected_sample_distance =
                Some(max_f64(self.bucket.max_expected_sample_distance, distance));
            if distance == 0.0 {
                self.bucket.expected_sample_exact += 1;
            }
            if distance <= tolerance {
                self.bucket.expected_sample_within_tolerance += 1;
            }
        }
        if let Some(distance) = row.actual_expected_distance {
            self.actual_expected_distance_sum += distance;
            self.bucket.max_actual_expected_distance =
                Some(max_f64(self.bucket.max_actual_expected_distance, distance));
            if distance == 0.0 {
                self.bucket.actual_expected_exact += 1;
            }
            if distance <= tolerance {
                self.bucket.actual_expected_within_tolerance += 1;
            }
        }
        match row.expected_closeness {
            Some(ExpectedCloseness::Actual) => self.bucket.actual_closer_to_expected_than_sample += 1,
            Some(ExpectedCloseness::Sample) => self.bucket.sample_closer_to_expected_than_actual += 1,
            Some(ExpectedCloseness::Tie) => self.bucket.actual_sample_expected_tie += 1,
            None => {}
        }
    }

    fn finish(mut self) -> AuditBucket {
        if self.bucket.actual_pixels_present > 0 {
            self.bucket.mean_actual_sample_distance =
                Some(self.actual_sample_distance_sum / self.bucket.actual_pixels_present as f64);
        }
        if self.bucket.expected_pixels_present > 0 {
            self.bucket.mean_expected_sample_distance =
                Some(self.expected_sample_distance_sum / self.bucket.expected_pixels_present as f64);
        }
        if self.bucket.actual_pixels_present > 0 && self.bucket.expected_pixels_present > 0 {
            let paired_count = self
                .bucket
                .actual_pixels_present
                .min(self.bucket.expected_pixels_present);
            self.bucket.mean_actual_expected_distance =
                Some(self.actual_expected_distance_sum / paired_count as f64);
        }
        self.bucket
    }
}

fn manifest_entries(value: &Value) -> Result<Vec<ManifestEntry>, Box<dyn Error>> {
    let corrections = value
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or("manifest corrections must be an array")?;
    corrections
        .iter()
        .enumerate()
        .map(|(index, value)| manifest_entry(value, index))
        .collect()
}

fn manifest_entry(value: &Value, index: usize) -> Result<ManifestEntry, Box<dyn Error>> {
    Ok(ManifestEntry {
        x: value
            .get("x")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("correction {index}: missing x"))?,
        y: value
            .get("y")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("correction {index}: missing y"))?,
        rgba: rgba_at(value, "/rgba")
            .ok_or_else(|| format!("correction {index}: missing rgba"))?,
        material_name: value
            .pointer("/surface/materialName")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        triangle: value.pointer("/surface/triangle").and_then(Value::as_u64),
        selection_source: value
            .get("selection_source")
            .and_then(Value::as_str)
            .unwrap_or("unspecified")
            .to_owned(),
    })
}

fn rgba_at(value: &Value, pointer: &str) -> Option<[u8; 4]> {
    let array = value.pointer(pointer)?.as_array()?;
    if array.len() != 4 {
        return None;
    }
    Some([
        u8::try_from(array[0].as_u64()?).ok()?,
        u8::try_from(array[1].as_u64()?).ok()?,
        u8::try_from(array[2].as_u64()?).ok()?,
        u8::try_from(array[3].as_u64()?).ok()?,
    ])
}

fn read_rgba_json(path: &Path) -> Result<RgbaJsonImage, Box<dyn Error>> {
    let image = serde_json::from_str::<RgbaJsonImage>(&fs::read_to_string(path)?)?;
    let expected_len = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("RGBA JSON dimensions overflow")?;
    if image.rgba.len() != expected_len {
        return Err(format!(
            "{}: rgba length {} does not match dimensions {}x{}",
            path.display(),
            image.rgba.len(),
            image.width,
            image.height
        )
        .into());
    }
    Ok(image)
}

fn rgb_distance(left: [u8; 4], right: [u8; 4]) -> f64 {
    left[..3]
        .iter()
        .zip(right[..3].iter())
        .map(|(left, right)| {
            let delta = f64::from(*left) - f64::from(*right);
            delta * delta
        })
        .sum::<f64>()
        .sqrt()
}

fn expected_closeness(
    actual: Option<[u8; 4]>,
    sample: Option<[u8; 4]>,
    expected: Option<[u8; 4]>,
) -> Option<ExpectedCloseness> {
    let (actual, sample, expected) = (actual?, sample?, expected?);
    let actual_distance = rgb_distance(actual, expected);
    let sample_distance = rgb_distance(sample, expected);
    match actual_distance.partial_cmp(&sample_distance)? {
        std::cmp::Ordering::Less => Some(ExpectedCloseness::Actual),
        std::cmp::Ordering::Greater => Some(ExpectedCloseness::Sample),
        std::cmp::Ordering::Equal => Some(ExpectedCloseness::Tie),
    }
}

fn max_f64(current: Option<f64>, value: f64) -> f64 {
    current.map_or(value, |current| current.max(value))
}

fn markdown(report: &AuditReport) -> String {
    let mut output = String::new();
    output.push_str("# Owner/Sample Execution Audit\n\n");
    output.push_str(&format!("- Manifest: `{}`\n", report.manifest));
    output.push_str(&format!("- Actual: `{}`\n", report.actual_rgba_json));
    if let Some(expected) = &report.expected_rgba_json {
        output.push_str(&format!("- Expected: `{expected}`\n"));
    }
    output.push_str(&format!("- Tolerance: `{:.4}`\n\n", report.tolerance));
    output.push_str("## Totals\n\n");
    output.push_str(&bucket_table_header("Bucket"));
    output.push_str(&bucket_table_row("all", &report.totals));
    output.push_str("\n## By Selection Source\n\n");
    output.push_str(&bucket_table_header("Source"));
    for (source, bucket) in &report.by_selection_source {
        output.push_str(&bucket_table_row(source, bucket));
    }
    output.push_str("\n## By Material\n\n");
    output.push_str(&bucket_table_header("Material"));
    for (material, bucket) in &report.by_material {
        output.push_str(&bucket_table_row(material, bucket));
    }
    output.push_str("\n## Sample-Closer Buckets By Material\n\n");
    output.push_str(&sample_closer_bucket_table());
    for bucket in &report.sample_closer_by_material {
        output.push_str(&sample_closer_bucket_row(bucket));
    }
    output.push_str("\n## Sample-Closer Buckets By Material And Source\n\n");
    output.push_str(&sample_closer_bucket_table());
    for bucket in &report.sample_closer_by_material_source {
        output.push_str(&sample_closer_bucket_row(bucket));
    }
    output.push_str("\n## Top Actual-Sample Misses\n\n");
    output.push_str("| Pixel | Material | Source | Manifest RGBA | Actual RGBA | Expected RGBA | A-S | E-S | A-E | Expected closer |\n");
    output.push_str("| --- | --- | --- | --- | --- | --- | ---: | ---: | ---: | --- |\n");
    for row in &report.top_actual_sample_misses {
        output.push_str(&audit_row_table_row(row));
    }
    output.push_str("\n## Top Sample-Closer Expected Pixels\n\n");
    output.push_str("| Pixel | Material | Source | Manifest RGBA | Actual RGBA | Expected RGBA | A-S | E-S | A-E | Expected closer |\n");
    output.push_str("| --- | --- | --- | --- | --- | --- | ---: | ---: | ---: | --- |\n");
    for row in &report.top_sample_closer_to_expected {
        output.push_str(&audit_row_table_row(row));
    }
    output
}

fn audit_row_table_row(row: &AuditRow) -> String {
    format!(
        "| {},{} | {}:{} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
        row.x,
        row.y,
        row.material_name,
        row.triangle
            .map(|triangle| triangle.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        row.selection_source,
        fmt_rgba(Some(row.manifest_rgba)),
        fmt_rgba(row.actual_rgba),
        fmt_rgba(row.expected_rgba),
        fmt_f64(row.actual_sample_distance),
        fmt_f64(row.expected_sample_distance),
        fmt_f64(row.actual_expected_distance),
        fmt_closeness(row.expected_closeness),
    )
}

fn bucket_table_header(label: &str) -> String {
    format!(
        "| {label} | Entries | Actual pixels | Actual=sample | Actual~sample | Expected=sample | Expected~sample | Actual~expected | Actual closer | Sample closer | Tie | Mean A-S | Mean E-S | Mean A-E | Max A-S |\n\
         | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n"
    )
}

fn bucket_table_row(label: &str, bucket: &AuditBucket) -> String {
    format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
        label,
        bucket.manifest_entries,
        bucket.actual_pixels_present,
        bucket.actual_sample_exact,
        bucket.actual_sample_within_tolerance,
        bucket.expected_sample_exact,
        bucket.expected_sample_within_tolerance,
        bucket.actual_expected_within_tolerance,
        bucket.actual_closer_to_expected_than_sample,
        bucket.sample_closer_to_expected_than_actual,
        bucket.actual_sample_expected_tie,
        fmt_f64(bucket.mean_actual_sample_distance),
        fmt_f64(bucket.mean_expected_sample_distance),
        fmt_f64(bucket.mean_actual_expected_distance),
        fmt_f64(bucket.max_actual_sample_distance),
    )
}

fn sample_closer_bucket_table() -> &'static str {
    "| Bucket | Entries | Mean sample margin | Max sample margin | Mean E-S | Mean A-E | Top pixel |\n\
     | --- | ---: | ---: | ---: | ---: | ---: | --- |\n"
}

fn sample_closer_bucket_row(bucket: &SampleCloserBucket) -> String {
    let top_pixel = bucket.top_pixel.as_ref().map_or_else(
        || "n/a".to_owned(),
        |row| {
            format!(
                "{},{} {}:{}",
                row.x,
                row.y,
                row.material_name,
                row.triangle
                    .map(|triangle| triangle.to_string())
                    .unwrap_or_else(|| "n/a".to_owned())
            )
        },
    );
    format!(
        "| {} | {} | {:.4} | {:.4} | {:.4} | {:.4} | {} |\n",
        bucket.label,
        bucket.entries,
        bucket.mean_sample_expected_margin,
        bucket.max_sample_expected_margin,
        bucket.mean_expected_sample_distance,
        bucket.mean_actual_expected_distance,
        top_pixel,
    )
}

fn fmt_rgba(value: Option<[u8; 4]>) -> String {
    value
        .map(|rgba| format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3]))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_f64(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_closeness(value: Option<ExpectedCloseness>) -> &'static str {
    match value {
        Some(ExpectedCloseness::Actual) => "actual",
        Some(ExpectedCloseness::Sample) => "sample",
        Some(ExpectedCloseness::Tie) => "tie",
        None => "n/a",
    }
}

fn write_file(path: &Path, content: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn self_test() -> Result<(), Box<dyn Error>> {
    let manifest = serde_json::json!({
        "corrections": [
            {
                "x": 1,
                "y": 0,
                "rgba": [10, 20, 30, 255],
                "surface": {"materialName": "mat", "triangle": 7},
                "selection_source": "center"
            },
            {
                "x": 0,
                "y": 1,
                "rgba": [50, 60, 70, 255],
                "surface": {"materialName": "other", "triangle": 2},
                "selection_source": "webgl-coverage"
            }
        ]
    });
    let actual = RgbaJsonImage {
        width: 2,
        height: 2,
        rgba: vec![
            0, 0, 0, 255, 10, 20, 30, 255, 51, 61, 71, 255, 0, 0, 0, 255,
        ],
    };
    let expected = RgbaJsonImage {
        width: 2,
        height: 2,
        rgba: vec![
            0, 0, 0, 255, 10, 20, 30, 255, 50, 60, 70, 255, 0, 0, 0, 255,
        ],
    };
    let report = audit(
        Path::new("manifest.json"),
        Path::new("actual.rgba.json"),
        Some(Path::new("expected.rgba.json")),
        1.75,
        &manifest,
        &actual,
        Some(&expected),
    )?;
    assert_eq!(report.totals.manifest_entries, 2);
    assert_eq!(report.totals.actual_sample_exact, 1);
    assert_eq!(report.totals.actual_sample_within_tolerance, 2);
    assert_eq!(report.totals.expected_sample_exact, 2);
    assert_eq!(report.totals.sample_closer_to_expected_than_actual, 1);
    assert_eq!(report.totals.actual_sample_expected_tie, 1);
    assert_eq!(report.sample_closer_by_material.len(), 1);
    assert_eq!(report.sample_closer_by_material[0].label, "other");
    assert_eq!(report.top_sample_closer_to_expected.len(), 1);
    assert!(markdown(&report).contains("Owner/Sample Execution Audit"));
    assert!(markdown(&report).contains("Sample-Closer Buckets By Material"));
    Ok(())
}
