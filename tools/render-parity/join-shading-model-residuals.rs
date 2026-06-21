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
    #[serde(default)]
    frontmost_material_shading: Option<MaterialShadingSnapshot>,
    frontmost: Option<SurfaceLabel>,
}

#[derive(Clone, Debug, Deserialize)]
struct SurfaceLabel {
    material_name: String,
    triangle: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MaterialShadingSnapshot {
    model: String,
    base_color: Option<[f64; 4]>,
    shade_color: Option<[f64; 4]>,
    emissive: Option<[f64; 3]>,
    metallic: Option<f64>,
    roughness: Option<f64>,
    occlusion_strength: Option<f64>,
    normal_scale: Option<f64>,
    unlit: Option<bool>,
    v0_compat_shade: Option<bool>,
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
    shared_backend_summaries: Vec<SharedBackendSummary>,
    backend_pairs: Vec<BackendPairSummary>,
    shared_direction_buckets: Vec<DirectionBucketSummary>,
    pixels: Vec<PixelJoin>,
}

#[derive(Clone, Debug, Serialize)]
struct BackendModelSummary {
    backend: String,
    row_count: u64,
    selected_count: u64,
    mean_expected_actual_rgb_distance: Option<f64>,
    mean_expected_minus_actual_rgb_delta: Option<[f64; 3]>,
    color_fit: ColorFitSummary,
    material_draw_color_fits: Vec<MaterialDrawColorFitSummary>,
    material_draw_shading_inputs: Vec<MaterialDrawShadingInputSummary>,
    materials: Vec<CountSummary>,
    draw_keys: Vec<CountSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct ColorFitSummary {
    mean_expected_over_actual_rgb_ratio: Option<[f64; 3]>,
    least_squares_gain_rgb: Option<[f64; 3]>,
    gain_fit_mean_rgb_distance: Option<f64>,
    additive_rgb_delta: Option<[f64; 3]>,
    additive_fit_mean_rgb_distance: Option<f64>,
    preferred_fit: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialDrawColorFitSummary {
    material_name: String,
    draw_key: String,
    row_count: u64,
    mean_expected_actual_rgb_distance: Option<f64>,
    mean_expected_minus_actual_rgb_delta: Option<[f64; 3]>,
    color_fit: ColorFitSummary,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialDrawShadingInputSummary {
    material_name: String,
    draw_key: String,
    row_count: u64,
    models: Vec<CountSummary>,
    mean_base_color: Option<[f64; 4]>,
    mean_shade_color: Option<[f64; 4]>,
    mean_emissive: Option<[f64; 3]>,
    mean_metallic: Option<f64>,
    mean_roughness: Option<f64>,
    mean_occlusion_strength: Option<f64>,
    mean_normal_scale: Option<f64>,
    unlit_count: u64,
    v0_compat_shade_count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct SharedBackendSummary {
    backend: String,
    shared_rows: u64,
    sample_exact_rows: u64,
    sample_exact_ratio: Option<f64>,
    mean_actual_selection_rgb_distance: Option<f64>,
    mean_expected_selection_rgb_distance: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
struct BackendPairSummary {
    left: String,
    right: String,
    shared_pixels: u64,
    mean_actual_rgb_distance: Option<f64>,
    mean_expected_actual_gap_delta: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
struct DirectionBucketSummary {
    signature: String,
    count: u64,
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

#[derive(Clone, Copy, Debug, Default)]
struct Vec4Accumulator {
    sum: [f64; 4],
    count: usize,
}

impl Vec4Accumulator {
    fn add(&mut self, value: Option<[f64; 4]>) {
        let Some(value) = value else {
            return;
        };
        for (sum, value) in self.sum.iter_mut().zip(value) {
            *sum += value;
        }
        self.count += 1;
    }

    fn mean(self) -> Option<[f64; 4]> {
        (self.count > 0).then(|| self.sum.map(|value| value / self.count as f64))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Vec3Accumulator {
    sum: [f64; 3],
    count: usize,
}

impl Vec3Accumulator {
    fn add(&mut self, value: Option<[f64; 3]>) {
        let Some(value) = value else {
            return;
        };
        add_delta(&mut self.sum, value);
        self.count += 1;
    }

    fn mean(self) -> Option<[f64; 3]> {
        mean_delta(self.sum, self.count)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ScalarAccumulator {
    sum: f64,
    count: usize,
}

impl ScalarAccumulator {
    fn add(&mut self, value: Option<f64>) {
        let Some(value) = value else {
            return;
        };
        self.sum += value;
        self.count += 1;
    }

    fn mean(self) -> Option<f64> {
        mean(self.sum, self.count)
    }
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
        write_file(
            path,
            &format!("{}\n", serde_json::to_string_pretty(&report)?),
        )?;
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
                material_names: unique_strings(
                    rows.iter().filter_map(|row| row.material_name.as_deref()),
                ),
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
        shared_backend_summaries: shared_backend_summaries(&pixels),
        backend_pairs: backend_pair_summaries(&pixels),
        shared_direction_buckets: shared_direction_buckets(&pixels),
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
        color_fit: color_fit(rows),
        material_draw_color_fits: material_draw_color_fits(rows),
        material_draw_shading_inputs: material_draw_shading_inputs(rows),
        materials: count_summaries(materials),
        draw_keys: count_summaries(draw_keys),
    }
}

fn material_draw_shading_inputs(rows: &[ResidualRow]) -> Vec<MaterialDrawShadingInputSummary> {
    let mut groups = BTreeMap::<(String, String), Vec<&MaterialShadingSnapshot>>::new();
    for row in rows {
        let Some(material) = material_name(row) else {
            continue;
        };
        let Some(draw_key) = &row.selection_draw_key else {
            continue;
        };
        let Some(shading) = &row.frontmost_material_shading else {
            continue;
        };
        groups
            .entry((material, draw_key.clone()))
            .or_default()
            .push(shading);
    }
    let mut summaries = groups
        .into_iter()
        .map(|((material_name, draw_key), rows)| {
            let mut models = BTreeMap::<String, u64>::new();
            let mut base = Vec4Accumulator::default();
            let mut shade = Vec4Accumulator::default();
            let mut emissive = Vec3Accumulator::default();
            let mut metallic = ScalarAccumulator::default();
            let mut roughness = ScalarAccumulator::default();
            let mut occlusion = ScalarAccumulator::default();
            let mut normal = ScalarAccumulator::default();
            let mut unlit_count = 0;
            let mut v0_compat_shade_count = 0;
            for row in &rows {
                *models.entry(row.model.clone()).or_default() += 1;
                base.add(row.base_color);
                shade.add(row.shade_color);
                emissive.add(row.emissive);
                metallic.add(row.metallic);
                roughness.add(row.roughness);
                occlusion.add(row.occlusion_strength);
                normal.add(row.normal_scale);
                unlit_count += u64::from(row.unlit.unwrap_or(false));
                v0_compat_shade_count += u64::from(row.v0_compat_shade.unwrap_or(false));
            }
            MaterialDrawShadingInputSummary {
                material_name,
                draw_key,
                row_count: rows.len().try_into().unwrap_or(u64::MAX),
                models: count_summaries(models),
                mean_base_color: base.mean(),
                mean_shade_color: shade.mean(),
                mean_emissive: emissive.mean(),
                mean_metallic: metallic.mean(),
                mean_roughness: roughness.mean(),
                mean_occlusion_strength: occlusion.mean(),
                mean_normal_scale: normal.mean(),
                unlit_count,
                v0_compat_shade_count,
            }
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        right
            .row_count
            .cmp(&left.row_count)
            .then_with(|| left.material_name.cmp(&right.material_name))
            .then_with(|| left.draw_key.cmp(&right.draw_key))
    });
    summaries
}

fn material_draw_color_fits(rows: &[ResidualRow]) -> Vec<MaterialDrawColorFitSummary> {
    let mut groups = BTreeMap::<(String, String), Vec<ResidualRow>>::new();
    for row in rows {
        let Some(material) = material_name(row) else {
            continue;
        };
        let Some(draw_key) = &row.selection_draw_key else {
            continue;
        };
        groups
            .entry((material, draw_key.clone()))
            .or_default()
            .push(row.clone());
    }
    let mut summaries = groups
        .into_iter()
        .map(|((material_name, draw_key), rows)| {
            let mut distance_sum = 0.0;
            let mut delta_sum = [0.0; 3];
            for row in &rows {
                distance_sum += row.expected_actual_rgb_distance;
                add_delta(&mut delta_sum, row.expected_minus_actual_rgb_delta);
            }
            MaterialDrawColorFitSummary {
                material_name,
                draw_key,
                row_count: rows.len().try_into().unwrap_or(u64::MAX),
                mean_expected_actual_rgb_distance: mean(distance_sum, rows.len()),
                mean_expected_minus_actual_rgb_delta: mean_delta(delta_sum, rows.len()),
                color_fit: color_fit(&rows),
            }
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        right
            .row_count
            .cmp(&left.row_count)
            .then_with(|| {
                option_f64_desc(
                    right.mean_expected_actual_rgb_distance,
                    left.mean_expected_actual_rgb_distance,
                )
            })
            .then_with(|| left.material_name.cmp(&right.material_name))
            .then_with(|| left.draw_key.cmp(&right.draw_key))
    });
    summaries
}

fn color_fit(rows: &[ResidualRow]) -> ColorFitSummary {
    let additive_rgb_delta = mean_delta(
        rows.iter().fold([0.0; 3], |mut sum, row| {
            add_delta(&mut sum, row.expected_minus_actual_rgb_delta);
            sum
        }),
        rows.len(),
    );
    let ratio = mean_ratio(rows);
    let gain = least_squares_gain(rows);
    let additive_error = additive_rgb_delta.map(|delta| {
        rows.iter()
            .map(|row| fitted_distance(row, |actual, channel| actual + delta[channel]))
            .sum::<f64>()
            / rows.len() as f64
    });
    let gain_error = gain.map(|gain| {
        rows.iter()
            .map(|row| fitted_distance(row, |actual, channel| actual * gain[channel]))
            .sum::<f64>()
            / rows.len() as f64
    });
    ColorFitSummary {
        mean_expected_over_actual_rgb_ratio: ratio,
        least_squares_gain_rgb: gain,
        gain_fit_mean_rgb_distance: gain_error,
        additive_rgb_delta,
        additive_fit_mean_rgb_distance: additive_error,
        preferred_fit: preferred_fit(additive_error, gain_error),
    }
}

fn mean_ratio(rows: &[ResidualRow]) -> Option<[f64; 3]> {
    if rows.is_empty() {
        return None;
    }
    let mut sums = [0.0; 3];
    let mut counts = [0usize; 3];
    for row in rows {
        for channel in 0..3 {
            let actual = f64::from(row.actual[channel]);
            if actual > 0.5 {
                sums[channel] += f64::from(row.expected[channel]) / actual;
                counts[channel] += 1;
            }
        }
    }
    counts.iter().all(|count| *count > 0).then(|| {
        [
            sums[0] / counts[0] as f64,
            sums[1] / counts[1] as f64,
            sums[2] / counts[2] as f64,
        ]
    })
}

fn least_squares_gain(rows: &[ResidualRow]) -> Option<[f64; 3]> {
    if rows.is_empty() {
        return None;
    }
    let mut numerator = [0.0; 3];
    let mut denominator = [0.0; 3];
    for row in rows {
        for channel in 0..3 {
            let actual = f64::from(row.actual[channel]);
            let expected = f64::from(row.expected[channel]);
            numerator[channel] += actual * expected;
            denominator[channel] += actual * actual;
        }
    }
    denominator.iter().all(|value| *value > 0.0).then(|| {
        [
            numerator[0] / denominator[0],
            numerator[1] / denominator[1],
            numerator[2] / denominator[2],
        ]
    })
}

fn fitted_distance(row: &ResidualRow, fit_channel: impl Fn(f64, usize) -> f64) -> f64 {
    (0..3)
        .map(|channel| {
            let expected = f64::from(row.expected[channel]);
            let actual = f64::from(row.actual[channel]);
            let delta = expected - fit_channel(actual, channel);
            delta * delta
        })
        .sum::<f64>()
        .sqrt()
}

fn preferred_fit(additive_error: Option<f64>, gain_error: Option<f64>) -> &'static str {
    match (additive_error, gain_error) {
        (Some(additive), Some(gain)) if additive + 0.5 < gain => "additive",
        (Some(additive), Some(gain)) if gain + 0.5 < additive => "gain",
        (Some(_), Some(_)) => "similar",
        (Some(_), None) => "additive",
        (None, Some(_)) => "gain",
        (None, None) => "n/a",
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

fn shared_backend_summaries(pixels: &[PixelJoin]) -> Vec<SharedBackendSummary> {
    #[derive(Default)]
    struct Accumulator {
        rows: u64,
        sample_exact_rows: u64,
        actual_selection_sum: f64,
        actual_selection_count: usize,
        expected_selection_sum: f64,
        expected_selection_count: usize,
    }

    let mut by_backend = BTreeMap::<String, Accumulator>::new();
    for pixel in pixels.iter().filter(|pixel| pixel.rows.len() > 1) {
        for row in &pixel.rows {
            let accumulator = by_backend.entry(row.backend.clone()).or_default();
            accumulator.rows += 1;
            if let Some(distance) = row.selection_actual_rgb_distance {
                accumulator.actual_selection_sum += distance;
                accumulator.actual_selection_count += 1;
                if distance <= 1.5 {
                    accumulator.sample_exact_rows += 1;
                }
            }
            if let Some(distance) = row.selection_expected_rgb_distance {
                accumulator.expected_selection_sum += distance;
                accumulator.expected_selection_count += 1;
            }
        }
    }
    by_backend
        .into_iter()
        .map(|(backend, accumulator)| SharedBackendSummary {
            backend,
            shared_rows: accumulator.rows,
            sample_exact_rows: accumulator.sample_exact_rows,
            sample_exact_ratio: (accumulator.rows > 0)
                .then_some(accumulator.sample_exact_rows as f64 / accumulator.rows as f64),
            mean_actual_selection_rgb_distance: mean(
                accumulator.actual_selection_sum,
                accumulator.actual_selection_count,
            ),
            mean_expected_selection_rgb_distance: mean(
                accumulator.expected_selection_sum,
                accumulator.expected_selection_count,
            ),
        })
        .collect()
}

fn backend_pair_summaries(pixels: &[PixelJoin]) -> Vec<BackendPairSummary> {
    #[derive(Default)]
    struct Accumulator {
        count: usize,
        actual_distance_sum: f64,
        gap_delta_sum: f64,
    }

    let mut by_pair = BTreeMap::<(String, String), Accumulator>::new();
    for pixel in pixels.iter().filter(|pixel| pixel.rows.len() > 1) {
        for (left_index, left) in pixel.rows.iter().enumerate() {
            for right in pixel.rows.iter().skip(left_index + 1) {
                let key = ordered_pair(&left.backend, &right.backend);
                let accumulator = by_pair.entry(key).or_default();
                accumulator.count += 1;
                accumulator.actual_distance_sum += rgb_distance(left.actual, right.actual);
                accumulator.gap_delta_sum +=
                    (left.expected_actual_rgb_distance - right.expected_actual_rgb_distance).abs();
            }
        }
    }
    by_pair
        .into_iter()
        .map(|((left, right), accumulator)| BackendPairSummary {
            left,
            right,
            shared_pixels: accumulator.count.try_into().unwrap_or(u64::MAX),
            mean_actual_rgb_distance: mean(accumulator.actual_distance_sum, accumulator.count),
            mean_expected_actual_gap_delta: mean(accumulator.gap_delta_sum, accumulator.count),
        })
        .collect()
}

fn shared_direction_buckets(pixels: &[PixelJoin]) -> Vec<DirectionBucketSummary> {
    struct Accumulator {
        count: u64,
        materials: BTreeMap<String, u64>,
        draw_keys: BTreeMap<String, u64>,
    }

    let mut by_signature = BTreeMap::<String, Accumulator>::new();
    for pixel in pixels.iter().filter(|pixel| pixel.rows.len() > 1) {
        let signature = pixel
            .rows
            .iter()
            .map(|row| {
                format!(
                    "{}:{}",
                    row.backend,
                    delta_direction(row.expected_minus_actual_rgb_delta)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let accumulator = by_signature
            .entry(signature)
            .or_insert_with(|| Accumulator {
                count: 0,
                materials: BTreeMap::new(),
                draw_keys: BTreeMap::new(),
            });
        accumulator.count += 1;
        for material in &pixel.material_names {
            *accumulator.materials.entry(material.clone()).or_default() += 1;
        }
        for draw_key in &pixel.draw_keys {
            *accumulator.draw_keys.entry(draw_key.clone()).or_default() += 1;
        }
    }
    let mut buckets = by_signature
        .into_iter()
        .map(|(signature, accumulator)| DirectionBucketSummary {
            signature,
            count: accumulator.count,
            materials: count_summaries(accumulator.materials),
            draw_keys: count_summaries(accumulator.draw_keys),
        })
        .collect::<Vec<_>>();
    buckets.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.signature.cmp(&right.signature))
    });
    buckets
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
        output.push_str(
            "| Backend | Rows | Selected | Mean E-A | Mean E-A delta | Materials | Draw keys |\n",
        );
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
        output.push_str("### Backend Color Fit\n\n");
        output.push_str("| Backend | Preferred | Additive RGB | Additive error | Gain RGB | Gain error | Mean E/A ratio |\n");
        output.push_str("| --- | --- | ---: | ---: | ---: | ---: | ---: |\n");
        for backend in &model.backends {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                backend.backend,
                backend.color_fit.preferred_fit,
                fmt_opt_delta(backend.color_fit.additive_rgb_delta),
                fmt_opt(backend.color_fit.additive_fit_mean_rgb_distance),
                fmt_opt_delta(backend.color_fit.least_squares_gain_rgb),
                fmt_opt(backend.color_fit.gain_fit_mean_rgb_distance),
                fmt_opt_delta(backend.color_fit.mean_expected_over_actual_rgb_ratio),
            ));
        }
        output.push('\n');
        if model
            .backends
            .iter()
            .any(|backend| !backend.material_draw_color_fits.is_empty())
        {
            output.push_str("### Material / Draw Color Fit\n\n");
            output.push_str("| Backend | Material | Draw key | Rows | Mean E-A | Mean E-A RGB | Preferred | Additive RGB | Additive error | Gain RGB | Gain error |\n");
            output.push_str("| --- | --- | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: |\n");
            for backend in &model.backends {
                for fit in backend.material_draw_color_fits.iter().take(8) {
                    output.push_str(&format!(
                        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                        backend.backend,
                        fit.material_name,
                        fit.draw_key,
                        fit.row_count,
                        fmt_opt(fit.mean_expected_actual_rgb_distance),
                        fmt_opt_delta(fit.mean_expected_minus_actual_rgb_delta),
                        fit.color_fit.preferred_fit,
                        fmt_opt_delta(fit.color_fit.additive_rgb_delta),
                        fmt_opt(fit.color_fit.additive_fit_mean_rgb_distance),
                        fmt_opt_delta(fit.color_fit.least_squares_gain_rgb),
                        fmt_opt(fit.color_fit.gain_fit_mean_rgb_distance),
                    ));
                }
            }
            output.push('\n');
        }
        if model
            .backends
            .iter()
            .any(|backend| !backend.material_draw_shading_inputs.is_empty())
        {
            output.push_str("### Material / Draw Shading Inputs\n\n");
            output.push_str("| Backend | Material | Draw key | Rows | Models | Base | Shade | Emissive | M/R/O/N | Unlit | V0 shade |\n");
            output.push_str("| --- | --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
            for backend in &model.backends {
                for input in backend.material_draw_shading_inputs.iter().take(8) {
                    output.push_str(&format!(
                        "| {} | {} | {} | {} | {} | {} | {} | {} | {} / {} / {} / {} | {} | {} |\n",
                        backend.backend,
                        input.material_name,
                        input.draw_key,
                        input.row_count,
                        fmt_counts(&input.models),
                        fmt_opt_vec4(input.mean_base_color),
                        fmt_opt_vec4(input.mean_shade_color),
                        fmt_opt_delta(input.mean_emissive),
                        fmt_opt(input.mean_metallic),
                        fmt_opt(input.mean_roughness),
                        fmt_opt(input.mean_occlusion_strength),
                        fmt_opt(input.mean_normal_scale),
                        input.unlit_count,
                        input.v0_compat_shade_count,
                    ));
                }
            }
            output.push('\n');
        }
        output.push_str("### Shared Backend Sample Following\n\n");
        output.push_str("| Backend | Shared rows | Sample exact rows | Sample exact ratio | Mean A-S | Mean E-S |\n");
        output.push_str("| --- | ---: | ---: | ---: | ---: | ---: |\n");
        for backend in &model.shared_backend_summaries {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                backend.backend,
                backend.shared_rows,
                backend.sample_exact_rows,
                fmt_opt(backend.sample_exact_ratio),
                fmt_opt(backend.mean_actual_selection_rgb_distance),
                fmt_opt(backend.mean_expected_selection_rgb_distance),
            ));
        }
        output.push('\n');
        output.push_str("### Backend Pair Agreement\n\n");
        output
            .push_str("| Pair | Shared pixels | Mean actual RGB distance | Mean E-A gap delta |\n");
        output.push_str("| --- | ---: | ---: | ---: |\n");
        for pair in &model.backend_pairs {
            output.push_str(&format!(
                "| {} / {} | {} | {} | {} |\n",
                pair.left,
                pair.right,
                pair.shared_pixels,
                fmt_opt(pair.mean_actual_rgb_distance),
                fmt_opt(pair.mean_expected_actual_gap_delta),
            ));
        }
        output.push('\n');
        output.push_str("### Shared Direction Buckets\n\n");
        output.push_str("| Signature | Count | Materials | Draw keys |\n");
        output.push_str("| --- | ---: | --- | --- |\n");
        for bucket in model.shared_direction_buckets.iter().take(12) {
            output.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                bucket.signature,
                bucket.count,
                fmt_counts(&bucket.materials),
                fmt_counts(&bucket.draw_keys),
            ));
        }
        output.push('\n');
        output.push_str("| Pixel | Materials | Draw keys | Backend | Model | Surface | Actual | Expected | E-A | E-A delta | Selected RGBA | Sel A/E | A-S / E-S |\n");
        output.push_str(
            "| --- | --- | --- | --- | --- | --- | --- | --- | ---: | ---: | --- | ---: | ---: |\n",
        );
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

fn ordered_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_owned(), right.to_owned())
    } else {
        (right.to_owned(), left.to_owned())
    }
}

fn rgb_distance(left: [u8; 4], right: [u8; 4]) -> f64 {
    left[..3]
        .iter()
        .zip(&right[..3])
        .map(|(&left, &right)| {
            let delta = f64::from(left) - f64::from(right);
            delta * delta
        })
        .sum::<f64>()
        .sqrt()
}

fn delta_direction(delta: [f64; 3]) -> &'static str {
    const EPSILON: f64 = 0.5;
    if delta.iter().all(|value| *value > EPSILON) {
        "expected_brighter"
    } else if delta.iter().all(|value| *value < -EPSILON) {
        "expected_darker"
    } else if delta.iter().all(|value| value.abs() <= EPSILON) {
        "matched"
    } else {
        "mixed"
    }
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

fn fmt_opt_vec4(value: Option<[f64; 4]>) -> String {
    value
        .map(|value| {
            format!(
                "{:.2},{:.2},{:.2},{:.2}",
                value[0], value[1], value[2], value[3]
            )
        })
        .unwrap_or_else(|| "n/a".to_owned())
}

fn option_f64_desc(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.total_cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
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
                rows: vec![residual_row(
                    1,
                    2,
                    "backpack_nm",
                    "node/mesh/prim/base",
                    40.0,
                )],
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
    assert_eq!(gltf.shared_backend_summaries.len(), 2);
    assert_eq!(gltf.backend_pairs.len(), 1);
    assert_eq!(gltf.shared_direction_buckets.len(), 1);
    assert!(matches!(
        gltf.backends[0].color_fit.preferred_fit,
        "additive" | "similar" | "gain"
    ));
    assert!(gltf.backends[0]
        .color_fit
        .additive_fit_mean_rgb_distance
        .is_some());
    assert!(gltf.backends[0]
        .color_fit
        .gain_fit_mean_rgb_distance
        .is_some());
    assert_eq!(
        gltf.backends[0].material_draw_color_fits[0].material_name,
        "backpack_nm"
    );
    assert_eq!(
        gltf.backends[0].material_draw_color_fits[0].draw_key,
        "node/mesh/prim/base"
    );
    assert_eq!(
        gltf.backends[0].material_draw_shading_inputs[0].material_name,
        "backpack_nm"
    );
    assert_eq!(
        gltf.backends[0].material_draw_shading_inputs[0].mean_base_color,
        Some([1.0, 0.5, 0.25, 1.0])
    );
    assert_eq!(gltf.pixels[0].x, 1);
    assert_eq!(gltf.pixels[0].rows.len(), 2);
    let markdown = markdown(&report);
    assert!(markdown.contains("Shading Model Residual Join"));
    assert!(markdown.contains("Backend Color Fit"));
    assert!(markdown.contains("Material / Draw Color Fit"));
    assert!(markdown.contains("Material / Draw Shading Inputs"));
    assert!(markdown.contains("Shared Backend Sample Following"));
    assert!(markdown.contains("Backend Pair Agreement"));
    assert!(markdown.contains("Shared Direction Buckets"));
    assert!(markdown.contains("expected_brighter"));
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
        frontmost_material_shading: Some(MaterialShadingSnapshot {
            model: "gltf_pbr".to_owned(),
            base_color: Some([1.0, 0.5, 0.25, 1.0]),
            shade_color: Some([1.0, 1.0, 1.0, 1.0]),
            emissive: Some([0.0, 0.0, 0.0]),
            metallic: Some(0.0),
            roughness: Some(0.65),
            occlusion_strength: Some(1.0),
            normal_scale: Some(1.0),
            unlit: Some(false),
            v0_compat_shade: Some(false),
        }),
        frontmost: Some(SurfaceLabel {
            material_name: material.to_owned(),
            triangle: 7,
        }),
    }
}
