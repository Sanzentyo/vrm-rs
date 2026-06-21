#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Audit base-texture sampling residuals after owner/sample resolve.
//!
//! This tool is intentionally Sans I/O: it joins existing hotspot and optional
//! owner/sample manifest JSON reports, then classifies whether residual colors
//! are best explained by the selected material sample, a texture sampling
//! variant, or unresolved surface ownership.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "audit-texture-sampling-parity",
    about = "Classify render hotspot base-texture sampling residuals"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    hotspots: Option<PathBuf>,
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long)]
    baseline_manifest: Option<PathBuf>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
    #[arg(long, default_value_t = 16)]
    top: usize,
}

#[derive(Clone, Debug, Serialize)]
struct AuditReport {
    hotspots: String,
    manifest: Option<String>,
    baseline_manifest: Option<String>,
    hotspot_count: u64,
    manifest_count: u64,
    baseline_manifest_count: u64,
    selected_count: u64,
    missing_selection_count: u64,
    carried_selection_count: u64,
    new_selection_count: u64,
    all: BucketStats,
    selected: BucketStats,
    missing_selection: BucketStats,
    carried_selection: BucketStats,
    new_selection: BucketStats,
    top_residuals: Vec<ResidualRow>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct BucketStats {
    count: u64,
    actual_cpu_closer: u64,
    expected_cpu_closer: u64,
    cpu_tied: u64,
    mean_cpu_actual_rgb_distance: Option<f64>,
    mean_cpu_expected_rgb_distance: Option<f64>,
    mean_edge_distance_pixels: Option<f64>,
    edge_distance_lte_025px: u64,
    edge_distance_lte_050px: u64,
    edge_distance_lte_100px: u64,
    same_material_as_actual: u64,
    same_material_as_expected: u64,
    same_triangle_as_actual: u64,
    same_triangle_as_expected: u64,
    best_sampling_modes_for_actual: Vec<ModeCount>,
    best_sampling_modes_for_expected: Vec<ModeCount>,
    material_counts: Vec<MaterialCount>,
}

#[derive(Clone, Debug, Serialize)]
struct ModeCount {
    mode: String,
    count: u64,
    mean_rgb_distance: f64,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialCount {
    material_name: String,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct ResidualRow {
    x: u64,
    y: u64,
    selected: bool,
    expected: [u8; 4],
    actual: [u8; 4],
    frontmost: Option<SurfaceLabel>,
    actual_match: Option<SurfaceLabel>,
    expected_match: Option<SurfaceLabel>,
    frontmost_cpu_base_color_rgba: Option<[u8; 4]>,
    cpu_actual_rgb_distance: Option<f64>,
    cpu_expected_rgb_distance: Option<f64>,
    best_actual_sampling_mode: Option<String>,
    best_actual_sampling_rgba: Option<[u8; 4]>,
    best_actual_sampling_rgb_distance: Option<f64>,
    best_expected_sampling_mode: Option<String>,
    best_expected_sampling_rgba: Option<[u8; 4]>,
    best_expected_sampling_rgb_distance: Option<f64>,
    edge_distance_pixels: Option<f64>,
    base_texture_local_rgb_gradient: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
struct SurfaceLabel {
    material_name: String,
    triangle: u64,
}

#[derive(Clone, Debug, Default)]
struct Accumulator {
    count: u64,
    actual_cpu_closer: u64,
    expected_cpu_closer: u64,
    cpu_tied: u64,
    cpu_actual_distance_sum: f64,
    cpu_actual_distance_count: u64,
    cpu_expected_distance_sum: f64,
    cpu_expected_distance_count: u64,
    edge_distance_sum: f64,
    edge_distance_count: u64,
    edge_distance_lte_025px: u64,
    edge_distance_lte_050px: u64,
    edge_distance_lte_100px: u64,
    same_material_as_actual: u64,
    same_material_as_expected: u64,
    same_triangle_as_actual: u64,
    same_triangle_as_expected: u64,
    actual_modes: BTreeMap<String, ModeAccumulator>,
    expected_modes: BTreeMap<String, ModeAccumulator>,
    materials: BTreeMap<String, u64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ModeAccumulator {
    count: u64,
    distance_sum: f64,
}

impl Accumulator {
    fn add(&mut self, hotspot: &Value) {
        self.count += 1;

        let actual_cpu = f64_at(hotspot, "/frontmost_cpu_base_color_actual_rgb_distance");
        let expected_cpu = f64_at(
            hotspot,
            "/frontmost_cpu_base_color_expected_rgb_distance",
        );
        if let Some(distance) = actual_cpu {
            self.cpu_actual_distance_sum += distance;
            self.cpu_actual_distance_count += 1;
        }
        if let Some(distance) = expected_cpu {
            self.cpu_expected_distance_sum += distance;
            self.cpu_expected_distance_count += 1;
        }
        match actual_cpu.zip(expected_cpu).and_then(compare_f64) {
            Some(std::cmp::Ordering::Less) => self.actual_cpu_closer += 1,
            Some(std::cmp::Ordering::Greater) => self.expected_cpu_closer += 1,
            Some(std::cmp::Ordering::Equal) => self.cpu_tied += 1,
            None => {}
        }

        if let Some(edge) = f64_at(hotspot, "/frontmost_visible/edge_distance_pixels") {
            self.edge_distance_sum += edge;
            self.edge_distance_count += 1;
            self.edge_distance_lte_025px += u64::from(edge <= 0.25);
            self.edge_distance_lte_050px += u64::from(edge <= 0.50);
            self.edge_distance_lte_100px += u64::from(edge <= 1.00);
        }

        let frontmost = surface_at(hotspot, "/frontmost_visible");
        let actual = surface_at(hotspot, "/nearest_visible_actual");
        let expected = surface_at(hotspot, "/nearest_visible_expected");
        if same_material(frontmost.as_ref(), actual.as_ref()) {
            self.same_material_as_actual += 1;
        }
        if same_material(frontmost.as_ref(), expected.as_ref()) {
            self.same_material_as_expected += 1;
        }
        if frontmost.as_ref().zip(actual.as_ref()).is_some_and(|(left, right)| {
            left.material_name == right.material_name && left.triangle == right.triangle
        }) {
            self.same_triangle_as_actual += 1;
        }
        if frontmost.as_ref().zip(expected.as_ref()).is_some_and(|(left, right)| {
            left.material_name == right.material_name && left.triangle == right.triangle
        }) {
            self.same_triangle_as_expected += 1;
        }
        if let Some(surface) = frontmost {
            *self.materials.entry(surface.material_name).or_default() += 1;
        }

        if let Some(best) = best_sampling_mode(hotspot, "/frontmost_texture_sampling_variants", "actual_rgb_distance") {
            self.actual_modes
                .entry(best.mode)
                .and_modify(|entry| {
                    entry.count += 1;
                    entry.distance_sum += best.distance;
                })
                .or_insert(ModeAccumulator {
                    count: 1,
                    distance_sum: best.distance,
                });
        }
        if let Some(best) = best_sampling_mode(
            hotspot,
            "/frontmost_texture_sampling_variants",
            "expected_rgb_distance",
        ) {
            self.expected_modes
                .entry(best.mode)
                .and_modify(|entry| {
                    entry.count += 1;
                    entry.distance_sum += best.distance;
                })
                .or_insert(ModeAccumulator {
                    count: 1,
                    distance_sum: best.distance,
                });
        }
    }

    fn finish(self) -> BucketStats {
        BucketStats {
            count: self.count,
            actual_cpu_closer: self.actual_cpu_closer,
            expected_cpu_closer: self.expected_cpu_closer,
            cpu_tied: self.cpu_tied,
            mean_cpu_actual_rgb_distance: mean(
                self.cpu_actual_distance_sum,
                self.cpu_actual_distance_count,
            ),
            mean_cpu_expected_rgb_distance: mean(
                self.cpu_expected_distance_sum,
                self.cpu_expected_distance_count,
            ),
            mean_edge_distance_pixels: mean(self.edge_distance_sum, self.edge_distance_count),
            edge_distance_lte_025px: self.edge_distance_lte_025px,
            edge_distance_lte_050px: self.edge_distance_lte_050px,
            edge_distance_lte_100px: self.edge_distance_lte_100px,
            same_material_as_actual: self.same_material_as_actual,
            same_material_as_expected: self.same_material_as_expected,
            same_triangle_as_actual: self.same_triangle_as_actual,
            same_triangle_as_expected: self.same_triangle_as_expected,
            best_sampling_modes_for_actual: mode_counts(self.actual_modes),
            best_sampling_modes_for_expected: mode_counts(self.expected_modes),
            material_counts: material_counts(self.materials),
        }
    }
}

#[derive(Clone, Debug)]
struct BestSamplingMode {
    mode: String,
    rgba: [u8; 4],
    distance: f64,
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
    let hotspots_path = options
        .hotspots
        .as_deref()
        .ok_or("missing --hotspots")?;
    let hotspots = serde_json::from_str::<Value>(&fs::read_to_string(hotspots_path)?)?;
    let manifest = if let Some(path) = options.manifest.as_deref() {
        Some(serde_json::from_str::<Value>(&fs::read_to_string(path)?)?)
    } else {
        None
    };
    let baseline_manifest = if let Some(path) = options.baseline_manifest.as_deref() {
        Some(serde_json::from_str::<Value>(&fs::read_to_string(path)?)?)
    } else {
        None
    };
    let report = audit(
        hotspots_path,
        options.manifest.as_deref(),
        options.baseline_manifest.as_deref(),
        &hotspots,
        manifest.as_ref(),
        baseline_manifest.as_ref(),
        options.top,
    )?;
    if let Some(path) = options.json_out.as_deref() {
        write_file(path, &format!("{}\n", serde_json::to_string_pretty(&report)?))?;
    } else if options.markdown_out.is_none() {
        print!("{}\n", serde_json::to_string_pretty(&report)?);
    }
    if let Some(path) = options.markdown_out.as_deref() {
        write_file(path, &markdown(&report))?;
    }
    Ok(())
}

fn audit(
    hotspots_path: &Path,
    manifest_path: Option<&Path>,
    baseline_manifest_path: Option<&Path>,
    hotspots: &Value,
    manifest: Option<&Value>,
    baseline_manifest: Option<&Value>,
    top: usize,
) -> Result<AuditReport, Box<dyn Error>> {
    let hotspot_items = hotspots
        .get("hotspots")
        .and_then(Value::as_array)
        .ok_or("hotspots.hotspots must be an array")?;
    let selected = manifest
        .map(manifest_pixels)
        .transpose()?
        .unwrap_or_default();
    let baseline_selected = baseline_manifest
        .map(manifest_pixels)
        .transpose()?
        .unwrap_or_default();

    let mut all = Accumulator::default();
    let mut selected_acc = Accumulator::default();
    let mut missing_acc = Accumulator::default();
    let mut carried_acc = Accumulator::default();
    let mut new_acc = Accumulator::default();
    let mut residuals = Vec::new();

    for hotspot in hotspot_items {
        let Some(pixel) = pixel_key(hotspot) else {
            continue;
        };
        let is_selected = selected.is_empty() || selected.contains(&pixel);
        let is_carried = !baseline_selected.is_empty() && baseline_selected.contains(&pixel);
        let is_new = !baseline_selected.is_empty() && is_selected && !is_carried;
        all.add(hotspot);
        if is_selected {
            selected_acc.add(hotspot);
            if is_carried {
                carried_acc.add(hotspot);
            } else if is_new {
                new_acc.add(hotspot);
            }
        } else {
            missing_acc.add(hotspot);
        }
        if let Some(row) = residual_row(hotspot, is_selected) {
            residuals.push(row);
        }
    }
    residuals.sort_by(|left, right| {
        let left_distance = left.cpu_expected_rgb_distance.unwrap_or(f64::INFINITY);
        let right_distance = right.cpu_expected_rgb_distance.unwrap_or(f64::INFINITY);
        right_distance
            .partial_cmp(&left_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.y.cmp(&right.y))
            .then_with(|| left.x.cmp(&right.x))
    });
    residuals.truncate(top);

    let manifest_count = u64::try_from(selected.len()).unwrap_or(u64::MAX);
    let baseline_manifest_count = u64::try_from(baseline_selected.len()).unwrap_or(u64::MAX);
    let selected_count = if selected.is_empty() {
        u64::try_from(hotspot_items.len()).unwrap_or(u64::MAX)
    } else {
        selected_acc.count
    };
    Ok(AuditReport {
        hotspots: display_path(hotspots_path),
        manifest: manifest_path.map(display_path),
        baseline_manifest: baseline_manifest_path.map(display_path),
        hotspot_count: u64::try_from(hotspot_items.len()).unwrap_or(u64::MAX),
        manifest_count,
        baseline_manifest_count,
        selected_count,
        missing_selection_count: if selected.is_empty() {
            0
        } else {
            missing_acc.count
        },
        carried_selection_count: carried_acc.count,
        new_selection_count: new_acc.count,
        all: all.finish(),
        selected: selected_acc.finish(),
        missing_selection: missing_acc.finish(),
        carried_selection: carried_acc.finish(),
        new_selection: new_acc.finish(),
        top_residuals: residuals,
    })
}

fn manifest_pixels(manifest: &Value) -> Result<HashSet<(u64, u64)>, Box<dyn Error>> {
    let corrections = manifest
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or("manifest corrections must be an array")?;
    Ok(corrections.iter().filter_map(pixel_key).collect())
}

fn residual_row(hotspot: &Value, selected: bool) -> Option<ResidualRow> {
    let actual_best = best_sampling_mode(
        hotspot,
        "/frontmost_texture_sampling_variants",
        "actual_rgb_distance",
    );
    let expected_best = best_sampling_mode(
        hotspot,
        "/frontmost_texture_sampling_variants",
        "expected_rgb_distance",
    );
    Some(ResidualRow {
        x: hotspot.get("x")?.as_u64()?,
        y: hotspot.get("y")?.as_u64()?,
        selected,
        expected: rgba_at(hotspot, "/expected")?,
        actual: rgba_at(hotspot, "/actual")?,
        frontmost: surface_at(hotspot, "/frontmost_visible"),
        actual_match: surface_at(hotspot, "/nearest_visible_actual"),
        expected_match: surface_at(hotspot, "/nearest_visible_expected"),
        frontmost_cpu_base_color_rgba: rgba_at(hotspot, "/frontmost_cpu_base_color_rgba"),
        cpu_actual_rgb_distance: f64_at(
            hotspot,
            "/frontmost_cpu_base_color_actual_rgb_distance",
        ),
        cpu_expected_rgb_distance: f64_at(
            hotspot,
            "/frontmost_cpu_base_color_expected_rgb_distance",
        ),
        best_actual_sampling_mode: actual_best.as_ref().map(|best| best.mode.clone()),
        best_actual_sampling_rgba: actual_best.as_ref().map(|best| best.rgba),
        best_actual_sampling_rgb_distance: actual_best.as_ref().map(|best| best.distance),
        best_expected_sampling_mode: expected_best.as_ref().map(|best| best.mode.clone()),
        best_expected_sampling_rgba: expected_best.as_ref().map(|best| best.rgba),
        best_expected_sampling_rgb_distance: expected_best.as_ref().map(|best| best.distance),
        edge_distance_pixels: f64_at(hotspot, "/frontmost_visible/edge_distance_pixels"),
        base_texture_local_rgb_gradient: f64_at(
            hotspot,
            "/frontmost_visible/base_texture_local_rgb_gradient",
        ),
    })
}

fn best_sampling_mode(hotspot: &Value, pointer: &str, distance_field: &str) -> Option<BestSamplingMode> {
    hotspot
        .pointer(pointer)?
        .as_array()?
        .iter()
        .filter_map(|value| {
            Some(BestSamplingMode {
                mode: value.get("mode")?.as_str()?.to_owned(),
                rgba: rgba(value.get("rgba")?)?,
                distance: value.get(distance_field)?.as_f64()?,
            })
        })
        .min_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn surface_at(value: &Value, pointer: &str) -> Option<SurfaceLabel> {
    let value = value.pointer(pointer)?;
    Some(SurfaceLabel {
        material_name: value
            .get("material_name")
            .or_else(|| value.get("materialName"))?
            .as_str()?
            .to_owned(),
        triangle: value.get("triangle")?.as_u64()?,
    })
}

fn same_material(left: Option<&SurfaceLabel>, right: Option<&SurfaceLabel>) -> bool {
    left.zip(right)
        .is_some_and(|(left, right)| left.material_name == right.material_name)
}

fn pixel_key(value: &Value) -> Option<(u64, u64)> {
    Some((value.get("x")?.as_u64()?, value.get("y")?.as_u64()?))
}

fn rgba_at(value: &Value, pointer: &str) -> Option<[u8; 4]> {
    rgba(value.pointer(pointer)?)
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

fn f64_at(value: &Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer)?.as_f64()
}

fn compare_f64((left, right): (f64, f64)) -> Option<std::cmp::Ordering> {
    left.partial_cmp(&right)
}

fn mean(sum: f64, count: u64) -> Option<f64> {
    (count > 0).then_some(sum / count as f64)
}

fn mode_counts(modes: BTreeMap<String, ModeAccumulator>) -> Vec<ModeCount> {
    let mut values = modes
        .into_iter()
        .map(|(mode, acc)| ModeCount {
            mode,
            count: acc.count,
            mean_rgb_distance: acc.distance_sum / acc.count.max(1) as f64,
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.mean_rgb_distance.total_cmp(&right.mean_rgb_distance))
            .then_with(|| left.mode.cmp(&right.mode))
    });
    values
}

fn material_counts(materials: BTreeMap<String, u64>) -> Vec<MaterialCount> {
    let mut values = materials
        .into_iter()
        .map(|(material_name, count)| MaterialCount {
            material_name,
            count,
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.material_name.cmp(&right.material_name))
    });
    values
}

fn markdown(report: &AuditReport) -> String {
    let mut output = String::new();
    output.push_str("# Texture Sampling Parity Audit\n\n");
    output.push_str(&format!("- Hotspots: `{}`\n", report.hotspots));
    if let Some(manifest) = &report.manifest {
        output.push_str(&format!("- Manifest: `{manifest}`\n"));
    }
    if let Some(manifest) = &report.baseline_manifest {
        output.push_str(&format!("- Baseline manifest: `{manifest}`\n"));
    }
    output.push_str(&format!(
        "- Hotspots/manifest/baseline/selected/missing/carried/new: `{}` / `{}` / `{}` / `{}` / `{}` / `{}` / `{}`\n\n",
        report.hotspot_count,
        report.manifest_count,
        report.baseline_manifest_count,
        report.selected_count,
        report.missing_selection_count,
        report.carried_selection_count,
        report.new_selection_count
    ));
    push_bucket_markdown(&mut output, "All", &report.all);
    push_bucket_markdown(&mut output, "Selected", &report.selected);
    push_bucket_markdown(&mut output, "Missing Selection", &report.missing_selection);
    push_bucket_markdown(&mut output, "Carried Selection", &report.carried_selection);
    push_bucket_markdown(&mut output, "New Selection", &report.new_selection);
    output.push_str("## Top Residuals\n\n");
    output.push_str("| Pixel | Sel | Actual | Expected | Frontmost | CPU A/E | Best Sampling A/E | Edge | Gradient |\n");
    output.push_str("| --- | --- | --- | --- | --- | ---: | --- | ---: | ---: |\n");
    for row in &report.top_residuals {
        output.push_str(&format!(
            "| {},{} | {} | {} | {} | {} | {} / {} | {} / {} | {} | {} |\n",
            row.x,
            row.y,
            if row.selected { "yes" } else { "no" },
            fmt_rgba(row.actual),
            fmt_rgba(row.expected),
            fmt_surface(row.frontmost.as_ref()),
            fmt_opt(row.cpu_actual_rgb_distance),
            fmt_opt(row.cpu_expected_rgb_distance),
            row.best_actual_sampling_mode.as_deref().unwrap_or("n/a"),
            row.best_expected_sampling_mode.as_deref().unwrap_or("n/a"),
            fmt_opt(row.edge_distance_pixels),
            fmt_opt(row.base_texture_local_rgb_gradient),
        ));
    }
    output
}

fn push_bucket_markdown(output: &mut String, title: &str, bucket: &BucketStats) {
    output.push_str(&format!("## {title}\n\n"));
    output.push_str(&format!("- Count: `{}`\n", bucket.count));
    output.push_str(&format!(
        "- CPU color closer actual/expected/tie: `{}` / `{}` / `{}`\n",
        bucket.actual_cpu_closer, bucket.expected_cpu_closer, bucket.cpu_tied
    ));
    output.push_str(&format!(
        "- Mean CPU RGB distance actual/expected: `{}` / `{}`\n",
        fmt_opt(bucket.mean_cpu_actual_rgb_distance),
        fmt_opt(bucket.mean_cpu_expected_rgb_distance)
    ));
    output.push_str(&format!(
        "- Edge <=0.25/0.50/1.00px: `{}` / `{}` / `{}`; mean `{}`\n",
        bucket.edge_distance_lte_025px,
        bucket.edge_distance_lte_050px,
        bucket.edge_distance_lte_100px,
        fmt_opt(bucket.mean_edge_distance_pixels)
    ));
    output.push_str(&format!(
        "- Same material actual/expected; same triangle actual/expected: `{}` / `{}`; `{}` / `{}`\n",
        bucket.same_material_as_actual,
        bucket.same_material_as_expected,
        bucket.same_triangle_as_actual,
        bucket.same_triangle_as_expected
    ));
    output.push_str(&format!(
        "- Best actual sampling modes: `{}`\n",
        fmt_modes(&bucket.best_sampling_modes_for_actual)
    ));
    output.push_str(&format!(
        "- Best expected sampling modes: `{}`\n",
        fmt_modes(&bucket.best_sampling_modes_for_expected)
    ));
    output.push_str(&format!(
        "- Top materials: `{}`\n\n",
        fmt_materials(&bucket.material_counts)
    ));
}

fn fmt_modes(modes: &[ModeCount]) -> String {
    modes
        .iter()
        .take(4)
        .map(|mode| format!("{}:{}@{:.4}", mode.mode, mode.count, mode.mean_rgb_distance))
        .collect::<Vec<_>>()
        .join(", ")
}

fn fmt_materials(materials: &[MaterialCount]) -> String {
    materials
        .iter()
        .take(6)
        .map(|material| format!("{}:{}", material.material_name, material.count))
        .collect::<Vec<_>>()
        .join(", ")
}

fn fmt_surface(surface: Option<&SurfaceLabel>) -> String {
    surface
        .map(|surface| format!("{}:tri{}", surface.material_name, surface.triangle))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_rgba(rgba: [u8; 4]) -> String {
    format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3])
}

fn fmt_opt(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "n/a".to_owned())
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
    let hotspots = serde_json::json!({
        "hotspots": [
            {
                "x": 1,
                "y": 2,
                "expected": [10, 10, 10, 255],
                "actual": [100, 100, 100, 255],
                "frontmost_cpu_base_color_rgba": [12, 10, 10, 255],
                "frontmost_cpu_base_color_actual_rgb_distance": 154.0,
                "frontmost_cpu_base_color_expected_rgb_distance": 2.0,
                "frontmost_visible": {
                    "material_name": "body",
                    "triangle": 7,
                    "edge_distance_pixels": 0.2,
                    "base_texture_local_rgb_gradient": 3.0
                },
                "nearest_visible_actual": {"material_name": "body", "triangle": 8},
                "nearest_visible_expected": {"material_name": "body", "triangle": 7},
                "frontmost_texture_sampling_variants": [
                    {
                        "mode": "linear_top_left_half_texel",
                        "rgba": [12, 10, 10, 255],
                        "actual_rgb_distance": 154.0,
                        "expected_rgb_distance": 2.0
                    },
                    {
                        "mode": "linear_bottom_left_half_texel",
                        "rgba": [98, 100, 100, 255],
                        "actual_rgb_distance": 2.0,
                        "expected_rgb_distance": 154.0
                    }
                ]
            },
            {
                "x": 3,
                "y": 4,
                "expected": [20, 20, 20, 255],
                "actual": [21, 20, 20, 255],
                "frontmost_cpu_base_color_rgba": [21, 20, 20, 255],
                "frontmost_cpu_base_color_actual_rgb_distance": 0.0,
                "frontmost_cpu_base_color_expected_rgb_distance": 1.0,
                "frontmost_visible": {
                    "material_name": "hair",
                    "triangle": 1,
                    "edge_distance_pixels": 1.5
                },
                "nearest_visible_actual": {"material_name": "hair", "triangle": 1},
                "nearest_visible_expected": {"material_name": "face", "triangle": 2},
                "frontmost_texture_sampling_variants": [
                    {
                        "mode": "nearest_top_left",
                        "rgba": [21, 20, 20, 255],
                        "actual_rgb_distance": 0.0,
                        "expected_rgb_distance": 1.0
                    }
                ]
            }
        ]
    });
    let manifest = serde_json::json!({
        "corrections": [
            {"x": 1, "y": 2, "rgba": [12, 10, 10, 255]}
        ]
    });
    let report = audit(
        Path::new("hotspots.json"),
        Some(Path::new("manifest.json")),
        None,
        &hotspots,
        Some(&manifest),
        None,
        8,
    )?;
    assert_eq!(report.hotspot_count, 2);
    assert_eq!(report.manifest_count, 1);
    assert_eq!(report.selected_count, 1);
    assert_eq!(report.missing_selection_count, 1);
    assert_eq!(report.all.expected_cpu_closer, 1);
    assert_eq!(report.all.actual_cpu_closer, 1);
    assert_eq!(report.selected.best_sampling_modes_for_expected[0].mode, "linear_top_left_half_texel");
    assert_eq!(report.selected.best_sampling_modes_for_actual[0].mode, "linear_bottom_left_half_texel");
    assert!(markdown(&report).contains("Texture Sampling Parity Audit"));
    let baseline = serde_json::json!({
        "corrections": [
            {"x": 3, "y": 4, "rgba": [21, 20, 20, 255]}
        ]
    });
    let layered_report = audit(
        Path::new("hotspots.json"),
        Some(Path::new("manifest.json")),
        Some(Path::new("baseline.json")),
        &hotspots,
        Some(&manifest),
        Some(&baseline),
        8,
    )?;
    assert_eq!(layered_report.baseline_manifest_count, 1);
    assert_eq!(layered_report.carried_selection_count, 0);
    assert_eq!(layered_report.new_selection_count, 1);
    Ok(())
}
