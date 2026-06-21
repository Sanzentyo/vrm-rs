#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Join texture/color residual probes by shading model across render backends.
//!
//! This is intentionally a Sans I/O diagnostic. It reads existing
//! `audit-texture-sampling-parity.rs` JSON reports and does not inspect images
//! or choose replacement samples.

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "join-shading-model-residuals",
    about = "Join texture/color residual probes by shading model across backend audit JSON reports"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long = "input", required_unless_present = "self_test")]
    inputs: Vec<NamedInput>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct NamedInput {
    name: String,
    path: PathBuf,
}

impl std::str::FromStr for NamedInput {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((name, path)) = value.split_once('=') else {
            return Err("expected NAME=PATH".to_owned());
        };
        if name.trim().is_empty() {
            return Err("input name must not be empty".to_owned());
        }
        if path.trim().is_empty() {
            return Err("input path must not be empty".to_owned());
        }
        Ok(Self {
            name: name.to_owned(),
            path: PathBuf::from(path),
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
struct TextureAuditReport {
    top_residuals_by_shading_model: Vec<ResidualGroup>,
}

#[derive(Clone, Debug, Deserialize)]
struct ResidualGroup {
    key: String,
    rows: Vec<ResidualRow>,
}

#[derive(Clone, Debug, Deserialize)]
struct ResidualRow {
    x: u64,
    y: u64,
    selected: bool,
    actual: [u8; 4],
    expected: [u8; 4],
    expected_actual_rgb_distance: f64,
    expected_minus_actual_rgb_delta: [f64; 3],
    selection_surface: Option<SurfaceLabel>,
    selection_draw_key: Option<String>,
    selection_rgba: Option<[u8; 4]>,
    selection_actual_rgb_distance: Option<f64>,
    selection_expected_rgb_distance: Option<f64>,
    actual_minus_selection_rgb_delta: Option<[f64; 3]>,
    expected_minus_selection_rgb_delta: Option<[f64; 3]>,
    frontmost_shading_model: Option<String>,
    frontmost: Option<SurfaceLabel>,
}

#[derive(Clone, Debug, Deserialize)]
struct SurfaceLabel {
    material_name: String,
    triangle: u64,
}

#[derive(Clone, Debug, Serialize)]
struct JoinReport {
    inputs: Vec<InputSummary>,
    models: Vec<ModelJoin>,
}

#[derive(Clone, Debug, Serialize)]
struct InputSummary {
    name: String,
    path: String,
}

#[derive(Clone, Debug, Serialize)]
struct ModelJoin {
    model: String,
    backends: Vec<BackendModelSummary>,
    shared_pixel_count: u64,
    pixels: Vec<PixelJoin>,
}

#[derive(Clone, Debug, Serialize)]
struct BackendModelSummary {
    backend: String,
    row_count: u64,
    selected_count: u64,
    mean_expected_actual_rgb_distance: Option<f64>,
    mean_expected_minus_actual_rgb_delta: Option<[f64; 3]>,
    materials: Vec<CountSummary>,
    draw_keys: Vec<CountSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct CountSummary {
    key: String,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct PixelJoin {
    x: u64,
    y: u64,
    material_names: Vec<String>,
    draw_keys: Vec<String>,
    rows: Vec<BackendResidual>,
}

#[derive(Clone, Debug, Serialize)]
struct BackendResidual {
    backend: String,
    selected: bool,
    frontmost_shading_model: Option<String>,
    material_name: Option<String>,
    surface: Option<String>,
    draw_key: Option<String>,
    actual: [u8; 4],
    expected: [u8; 4],
    expected_actual_rgb_distance: f64,
    expected_minus_actual_rgb_delta: [f64; 3],
    selection_rgba: Option<[u8; 4]>,
    selection_actual_rgb_distance: Option<f64>,
    selection_expected_rgb_distance: Option<f64>,
    actual_minus_selection_rgb_delta: Option<[f64; 3]>,
    expected_minus_selection_rgb_delta: Option<[f64; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    let report = join_from_paths(&options.inputs)?;
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

fn join_from_paths(inputs: &[NamedInput]) -> Result<JoinReport, Box<dyn Error>> {
    let mut names = BTreeSet::new();
    let mut parsed = Vec::new();
    for input in inputs {
        if !names.insert(input.name.clone()) {
            return Err(format!("duplicate input name `{}`", input.name).into());
        }
        let report = serde_json::from_str::<TextureAuditReport>(&fs::read_to_string(&input.path)?)?;
        parsed.push((input.clone(), report));
    }
    Ok(join_reports(&parsed))
}

fn join_reports(inputs: &[(NamedInput, TextureAuditReport)]) -> JoinReport {
    let mut models = BTreeMap::<String, BTreeMap<String, Vec<ResidualRow>>>::new();
    for (input, report) in inputs {
        for group in &report.top_residuals_by_shading_model {
            models
                .entry(group.key.clone())
                .or_default()
                .insert(input.name.clone(), group.rows.clone());
        }
    }
    let models = models
        .into_iter()
        .map(|(model, backend_rows)| model_join(model, backend_rows))
        .collect::<Vec<_>>();
    JoinReport {
        inputs: inputs
            .iter()
            .map(|(input, _)| InputSummary {
                name: input.name.clone(),
                path: display_path(&input.path),
            })
            .collect(),
        models,
    }
}

fn model_join(model: String, backend_rows: BTreeMap<String, Vec<ResidualRow>>) -> ModelJoin {
    let mut pixel_rows = BTreeMap::<PixelKey, Vec<BackendResidual>>::new();
    let mut backends = Vec::new();
    for (backend, rows) in backend_rows {
        backends.push(backend_summary(&backend, &rows));
        for row in rows {
            pixel_rows
                .entry(PixelKey { x: row.x, y: row.y })
                .or_default()
                .push(backend_residual(&backend, row));
        }
    }
    let shared_pixel_count = pixel_rows
        .values()
        .filter(|rows| rows.len() > 1)
        .count()
        .try_into()
        .unwrap_or(u64::MAX);
    let mut pixels = pixel_rows
        .into_iter()
        .map(|(pixel, mut rows)| {
            rows.sort_by(|left, right| left.backend.cmp(&right.backend));
            PixelJoin {
                x: pixel.x,
                y: pixel.y,
                material_names: unique_strings(rows.iter().filter_map(|row| row.material_name.as_deref())),
                draw_keys: unique_strings(rows.iter().filter_map(|row| row.draw_key.as_deref())),
                rows,
            }
        })
        .collect::<Vec<_>>();
    pixels.sort_by(|left, right| {
        let left_max = max_expected_actual(&left.rows);
        let right_max = max_expected_actual(&right.rows);
        right_max
            .total_cmp(&left_max)
            .then_with(|| left.y.cmp(&right.y))
            .then_with(|| left.x.cmp(&right.x))
    });
    ModelJoin {
        model,
        backends,
        shared_pixel_count,
        pixels,
    }
}

fn backend_summary(backend: &str, rows: &[ResidualRow]) -> BackendModelSummary {
    let mut materials = BTreeMap::<String, u64>::new();
    let mut draw_keys = BTreeMap::<String, u64>::new();
    let mut distance_sum = 0.0;
    let mut delta_sum = [0.0; 3];
    for row in rows {
        distance_sum += row.expected_actual_rgb_distance;
        add_delta(&mut delta_sum, row.expected_minus_actual_rgb_delta);
        if let Some(material) = material_name(row) {
            *materials.entry(material).or_default() += 1;
        }
        if let Some(draw_key) = &row.selection_draw_key {
            *draw_keys.entry(draw_key.clone()).or_default() += 1;
        }
    }
    BackendModelSummary {
        backend: backend.to_owned(),
        row_count: rows.len().try_into().unwrap_or(u64::MAX),
        selected_count: rows
            .iter()
            .filter(|row| row.selected)
            .count()
            .try_into()
            .unwrap_or(u64::MAX),
        mean_expected_actual_rgb_distance: mean(distance_sum, rows.len()),
        mean_expected_minus_actual_rgb_delta: mean_delta(delta_sum, rows.len()),
        materials: count_summaries(materials),
        draw_keys: count_summaries(draw_keys),
    }
}

fn backend_residual(backend: &str, row: ResidualRow) -> BackendResidual {
    let material_name = material_name(&row);
    let surface = row
        .selection_surface
        .as_ref()
        .or(row.frontmost.as_ref())
        .map(surface_label);
    BackendResidual {
        backend: backend.to_owned(),
        selected: row.selected,
        frontmost_shading_model: row.frontmost_shading_model,
        material_name,
        surface,
        draw_key: row.selection_draw_key,
        actual: row.actual,
        expected: row.expected,
        expected_actual_rgb_distance: row.expected_actual_rgb_distance,
        expected_minus_actual_rgb_delta: row.expected_minus_actual_rgb_delta,
        selection_rgba: row.selection_rgba,
        selection_actual_rgb_distance: row.selection_actual_rgb_distance,
        selection_expected_rgb_distance: row.selection_expected_rgb_distance,
        actual_minus_selection_rgb_delta: row.actual_minus_selection_rgb_delta,
        expected_minus_selection_rgb_delta: row.expected_minus_selection_rgb_delta,
    }
}

fn markdown(report: &JoinReport) -> String {
    let mut output = String::new();
    output.push_str("# Shading Model Residual Join\n\n");
    output.push_str("## Inputs\n\n");
    output.push_str("| Backend | Audit JSON |\n");
    output.push_str("| --- | --- |\n");
    for input in &report.inputs {
        output.push_str(&format!("| {} | `{}` |\n", input.name, input.path));
    }
    output.push('\n');
    for model in &report.models {
        output.push_str(&format!("## `{}`\n\n", model.model));
        output.push_str(&format!(
            "- Shared top-residual pixels across multiple backends: `{}`\n\n",
            model.shared_pixel_count
        ));
        output.push_str("| Backend | Rows | Selected | Mean E-A | Mean E-A delta | Materials | Draw keys |\n");
        output.push_str("| --- | ---: | ---: | ---: | ---: | --- | --- |\n");
        for backend in &model.backends {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                backend.backend,
                backend.row_count,
                backend.selected_count,
                fmt_opt(backend.mean_expected_actual_rgb_distance),
                fmt_opt_delta(backend.mean_expected_minus_actual_rgb_delta),
                fmt_counts(&backend.materials),
                fmt_counts(&backend.draw_keys),
            ));
        }
        output.push('\n');
        output.push_str("| Pixel | Materials | Draw keys | Backend | Model | Surface | Actual | Expected | E-A | E-A delta | Selected RGBA | Sel A/E | A-S / E-S |\n");
        output.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | ---: | ---: | --- | ---: | ---: |\n");
        for pixel in model.pixels.iter().take(24) {
            for (index, row) in pixel.rows.iter().enumerate() {
                output.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} / {} | {} / {} |\n",
                    if index == 0 {
                        format!("{},{}", pixel.x, pixel.y)
                    } else {
                        String::new()
                    },
                    if index == 0 {
                        pixel.material_names.join(", ")
                    } else {
                        String::new()
                    },
                    if index == 0 {
                        pixel.draw_keys.join(", ")
                    } else {
                        String::new()
                    },
                    row.backend,
                    row.frontmost_shading_model.as_deref().unwrap_or("n/a"),
                    row.surface.as_deref().unwrap_or("n/a"),
                    fmt_rgba(row.actual),
                    fmt_rgba(row.expected),
                    fmt_opt(Some(row.expected_actual_rgb_distance)),
                    fmt_opt_delta(Some(row.expected_minus_actual_rgb_delta)),
                    fmt_opt_rgba(row.selection_rgba),
                    fmt_opt(row.selection_actual_rgb_distance),
                    fmt_opt(row.selection_expected_rgb_distance),
                    fmt_opt_delta(row.actual_minus_selection_rgb_delta),
                    fmt_opt_delta(row.expected_minus_selection_rgb_delta),
                ));
            }
        }
        output.push('\n');
    }
    output
}

fn material_name(row: &ResidualRow) -> Option<String> {
    row.selection_surface
        .as_ref()
        .or(row.frontmost.as_ref())
        .map(|surface| surface.material_name.clone())
}

fn surface_label(surface: &SurfaceLabel) -> String {
    format!("{}:tri{}", surface.material_name, surface.triangle)
}

fn max_expected_actual(rows: &[BackendResidual]) -> f64 {
    rows.iter()
        .map(|row| row.expected_actual_rgb_distance)
        .fold(0.0, f64::max)
}

fn count_summaries(counts: BTreeMap<String, u64>) -> Vec<CountSummary> {
    let mut values = counts
        .into_iter()
        .map(|(key, count)| CountSummary { key, count })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.key.cmp(&right.key))
    });
    values
}

fn unique_strings<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    values
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn mean(sum: f64, count: usize) -> Option<f64> {
    (count > 0).then_some(sum / count as f64)
}

fn add_delta(sum: &mut [f64; 3], delta: [f64; 3]) {
    for (sum, value) in sum.iter_mut().zip(delta) {
        *sum += value;
    }
}

fn mean_delta(sum: [f64; 3], count: usize) -> Option<[f64; 3]> {
    (count > 0).then(|| {
        [
            sum[0] / count as f64,
            sum[1] / count as f64,
            sum[2] / count as f64,
        ]
    })
}

fn fmt_counts(counts: &[CountSummary]) -> String {
    counts
        .iter()
        .take(4)
        .map(|count| format!("{}:{}", count.key, count.count))
        .collect::<Vec<_>>()
        .join(", ")
}

fn fmt_rgba(rgba: [u8; 4]) -> String {
    format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3])
}

fn fmt_opt_rgba(rgba: Option<[u8; 4]>) -> String {
    rgba.map(fmt_rgba).unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_opt(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_opt_delta(delta: Option<[f64; 3]>) -> String {
    delta
        .map(|delta| format!("{:.2},{:.2},{:.2}", delta[0], delta[1], delta[2]))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn write_file(path: &Path, content: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn self_test() -> Result<(), Box<dyn Error>> {
    let wgpu = TextureAuditReport {
        top_residuals_by_shading_model: vec![ResidualGroup {
            key: "gltf_pbr".to_owned(),
            rows: vec![
                residual_row(1, 2, "backpack_nm", "node/mesh/prim/base", 50.0),
                residual_row(3, 4, "backpack_nm", "node/mesh/prim/base", 25.0),
            ],
        }],
    };
    let ash = TextureAuditReport {
        top_residuals_by_shading_model: vec![
            ResidualGroup {
                key: "gltf_pbr".to_owned(),
                rows: vec![residual_row(1, 2, "backpack_nm", "node/mesh/prim/base", 40.0)],
            },
            ResidualGroup {
                key: "mtoon".to_owned(),
                rows: vec![residual_row(5, 6, "body_nm", "node/mesh/prim/body", 60.0)],
            },
        ],
    };
    let inputs = vec![
        (
            NamedInput {
                name: "wgpu".to_owned(),
                path: PathBuf::from("wgpu.json"),
            },
            wgpu,
        ),
        (
            NamedInput {
                name: "ash".to_owned(),
                path: PathBuf::from("ash.json"),
            },
            ash,
        ),
    ];
    let report = join_reports(&inputs);
    assert_eq!(report.models.len(), 2);
    let gltf = report
        .models
        .iter()
        .find(|model| model.model == "gltf_pbr")
        .expect("gltf_pbr model");
    assert_eq!(gltf.shared_pixel_count, 1);
    assert_eq!(gltf.backends.len(), 2);
    assert_eq!(gltf.pixels[0].x, 1);
    assert_eq!(gltf.pixels[0].rows.len(), 2);
    let markdown = markdown(&report);
    assert!(markdown.contains("Shading Model Residual Join"));
    assert!(markdown.contains("`gltf_pbr`"));
    assert!(markdown.contains("backpack_nm"));
    assert!(markdown.contains("wgpu"));
    Ok(())
}

fn residual_row(x: u64, y: u64, material: &str, draw_key: &str, distance: f64) -> ResidualRow {
    ResidualRow {
        x,
        y,
        selected: true,
        actual: [10, 20, 30, 255],
        expected: [40, 50, 60, 255],
        expected_actual_rgb_distance: distance,
        expected_minus_actual_rgb_delta: [30.0, 30.0, 30.0],
        selection_surface: Some(SurfaceLabel {
            material_name: material.to_owned(),
            triangle: 7,
        }),
        selection_draw_key: Some(draw_key.to_owned()),
        selection_rgba: Some([11, 21, 31, 255]),
        selection_actual_rgb_distance: Some(1.7),
        selection_expected_rgb_distance: Some(49.0),
        actual_minus_selection_rgb_delta: Some([-1.0, -1.0, -1.0]),
        expected_minus_selection_rgb_delta: Some([29.0, 29.0, 29.0]),
        frontmost_shading_model: Some("gltf_pbr".to_owned()),
        frontmost: Some(SurfaceLabel {
            material_name: material.to_owned(),
            triangle: 7,
        }),
    }
}
