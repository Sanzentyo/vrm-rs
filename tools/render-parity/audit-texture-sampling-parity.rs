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
use std::collections::{BTreeMap, HashMap, HashSet};
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
    selection_source_buckets: Vec<SelectionSourceBucket>,
    recommended_probes: Vec<ProbeRecommendation>,
    top_residuals: Vec<ResidualRow>,
    top_residuals_by_shading_model: Vec<ResidualGroup>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct BucketStats {
    count: u64,
    mean_expected_actual_rgb_distance: Option<f64>,
    expected_actual_within_4: u64,
    expected_actual_within_8: u64,
    expected_actual_within_16: u64,
    mean_expected_minus_actual_rgb_delta: Option<[f64; 3]>,
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
    mean_nearest_expected_cpu_actual_rgb_distance: Option<f64>,
    mean_nearest_expected_cpu_expected_rgb_distance: Option<f64>,
    nearest_expected_cpu_actual_closer: u64,
    nearest_expected_cpu_expected_closer: u64,
    nearest_expected_cpu_tied: u64,
    nearest_expected_beats_frontmost_for_expected: u64,
    mean_frontmost_base_texture_actual_rgb_distance: Option<f64>,
    mean_frontmost_base_texture_expected_rgb_distance: Option<f64>,
    mean_actual_minus_texture_rgb_delta: Option<[f64; 3]>,
    mean_expected_minus_texture_rgb_delta: Option<[f64; 3]>,
    frontmost_base_texture_actual_closer: u64,
    frontmost_base_texture_expected_closer: u64,
    frontmost_base_texture_tied: u64,
    frontmost_base_texture_beats_cpu_for_expected: u64,
    mean_frontmost_texture_as_linear_srgb_actual_rgb_distance: Option<f64>,
    mean_frontmost_texture_as_linear_srgb_expected_rgb_distance: Option<f64>,
    mean_actual_minus_texture_as_linear_srgb_rgb_delta: Option<[f64; 3]>,
    mean_expected_minus_texture_as_linear_srgb_rgb_delta: Option<[f64; 3]>,
    frontmost_texture_as_linear_srgb_actual_closer: u64,
    frontmost_texture_as_linear_srgb_expected_closer: u64,
    frontmost_texture_as_linear_srgb_tied: u64,
    mean_manifest_sample_actual_rgb_distance: Option<f64>,
    mean_manifest_sample_expected_rgb_distance: Option<f64>,
    mean_actual_minus_manifest_sample_rgb_delta: Option<[f64; 3]>,
    mean_expected_minus_manifest_sample_rgb_delta: Option<[f64; 3]>,
    mean_actual_over_manifest_sample_rgb_ratio: Option<[f64; 3]>,
    mean_expected_over_manifest_sample_rgb_ratio: Option<[f64; 3]>,
    least_squares_actual_over_manifest_sample_rgb_gain: Option<[f64; 3]>,
    least_squares_expected_over_manifest_sample_rgb_gain: Option<[f64; 3]>,
    mean_expected_minus_actual_over_manifest_sample_rgb_ratio: Option<[f64; 3]>,
    least_squares_expected_minus_actual_over_manifest_sample_rgb_gain: Option<[f64; 3]>,
    manifest_sample_actual_closer: u64,
    manifest_sample_expected_closer: u64,
    manifest_sample_tied: u64,
    manifest_sample_actual_within_1_5: u64,
    manifest_sample_expected_within_1_5: u64,
    manifest_sample_actual_within_8: u64,
    manifest_sample_expected_within_8: u64,
    manifest_sample_actual_near_expected_far: u64,
    manifest_sample_actual_far_expected_near: u64,
    manifest_sample_both_far: u64,
    mean_best_sampling_actual_rgb_distance: Option<f64>,
    mean_best_sampling_expected_rgb_distance: Option<f64>,
    best_sampling_actual_within_4: u64,
    best_sampling_actual_within_8: u64,
    best_sampling_actual_within_16: u64,
    best_sampling_expected_within_4: u64,
    best_sampling_expected_within_8: u64,
    best_sampling_expected_within_16: u64,
    material_counts: Vec<MaterialCount>,
    shading_model_counts: Vec<ShadingModelCount>,
    shading_model_buckets: Vec<ShadingModelBucket>,
    material_buckets: Vec<MaterialBucket>,
    selection_material_buckets: Vec<MaterialBucket>,
    selection_material_draw_buckets: Vec<SelectionMaterialDrawBucket>,
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
struct ShadingModelCount {
    model: String,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct ShadingModelBucket {
    model: String,
    stats: MaterialBucket,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialBucket {
    material_name: String,
    count: u64,
    mean_expected_actual_rgb_distance: Option<f64>,
    expected_actual_within_4: u64,
    expected_actual_within_8: u64,
    expected_actual_within_16: u64,
    mean_expected_minus_actual_rgb_delta: Option<[f64; 3]>,
    actual_cpu_closer: u64,
    expected_cpu_closer: u64,
    cpu_tied: u64,
    mean_cpu_actual_rgb_distance: Option<f64>,
    mean_cpu_expected_rgb_distance: Option<f64>,
    edge_distance_lte_050px: u64,
    same_material_as_expected: u64,
    same_triangle_as_expected: u64,
    mean_nearest_expected_cpu_actual_rgb_distance: Option<f64>,
    mean_nearest_expected_cpu_expected_rgb_distance: Option<f64>,
    nearest_expected_cpu_actual_closer: u64,
    nearest_expected_cpu_expected_closer: u64,
    nearest_expected_cpu_tied: u64,
    nearest_expected_beats_frontmost_for_expected: u64,
    mean_frontmost_base_texture_actual_rgb_distance: Option<f64>,
    mean_frontmost_base_texture_expected_rgb_distance: Option<f64>,
    mean_actual_minus_texture_rgb_delta: Option<[f64; 3]>,
    mean_expected_minus_texture_rgb_delta: Option<[f64; 3]>,
    frontmost_base_texture_actual_closer: u64,
    frontmost_base_texture_expected_closer: u64,
    frontmost_base_texture_tied: u64,
    frontmost_base_texture_beats_cpu_for_expected: u64,
    mean_frontmost_texture_as_linear_srgb_actual_rgb_distance: Option<f64>,
    mean_frontmost_texture_as_linear_srgb_expected_rgb_distance: Option<f64>,
    mean_actual_minus_texture_as_linear_srgb_rgb_delta: Option<[f64; 3]>,
    mean_expected_minus_texture_as_linear_srgb_rgb_delta: Option<[f64; 3]>,
    frontmost_texture_as_linear_srgb_actual_closer: u64,
    frontmost_texture_as_linear_srgb_expected_closer: u64,
    frontmost_texture_as_linear_srgb_tied: u64,
    mean_manifest_sample_actual_rgb_distance: Option<f64>,
    mean_manifest_sample_expected_rgb_distance: Option<f64>,
    mean_actual_minus_manifest_sample_rgb_delta: Option<[f64; 3]>,
    mean_expected_minus_manifest_sample_rgb_delta: Option<[f64; 3]>,
    mean_actual_over_manifest_sample_rgb_ratio: Option<[f64; 3]>,
    mean_expected_over_manifest_sample_rgb_ratio: Option<[f64; 3]>,
    least_squares_actual_over_manifest_sample_rgb_gain: Option<[f64; 3]>,
    least_squares_expected_over_manifest_sample_rgb_gain: Option<[f64; 3]>,
    mean_expected_minus_actual_over_manifest_sample_rgb_ratio: Option<[f64; 3]>,
    least_squares_expected_minus_actual_over_manifest_sample_rgb_gain: Option<[f64; 3]>,
    manifest_sample_actual_closer: u64,
    manifest_sample_expected_closer: u64,
    manifest_sample_tied: u64,
    manifest_sample_actual_within_1_5: u64,
    manifest_sample_expected_within_1_5: u64,
    manifest_sample_actual_within_8: u64,
    manifest_sample_expected_within_8: u64,
    manifest_sample_actual_near_expected_far: u64,
    manifest_sample_actual_far_expected_near: u64,
    manifest_sample_both_far: u64,
    mean_best_sampling_actual_rgb_distance: Option<f64>,
    mean_best_sampling_expected_rgb_distance: Option<f64>,
    best_sampling_actual_within_4: u64,
    best_sampling_actual_within_8: u64,
    best_sampling_actual_within_16: u64,
    best_sampling_expected_within_4: u64,
    best_sampling_expected_within_8: u64,
    best_sampling_expected_within_16: u64,
    shading_model_counts: Vec<ShadingModelCount>,
    best_sampling_modes_for_actual: Vec<ModeCount>,
    best_sampling_modes_for_expected: Vec<ModeCount>,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionMaterialDrawBucket {
    material_name: String,
    draw_key: String,
    stats: MaterialBucket,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionSourceBucket {
    selection_source: String,
    stats: BucketStats,
}

#[derive(Clone, Debug, Serialize)]
struct ProbeRecommendation {
    material_name: String,
    draw_key: String,
    count: u64,
    classification: &'static str,
    action: &'static str,
    mean_expected_actual_rgb_distance: Option<f64>,
    mean_manifest_actual_rgb_distance: Option<f64>,
    mean_manifest_expected_rgb_distance: Option<f64>,
    mean_actual_minus_manifest_rgb_delta: Option<[f64; 3]>,
    mean_expected_minus_manifest_rgb_delta: Option<[f64; 3]>,
    least_squares_actual_over_manifest_rgb_gain: Option<[f64; 3]>,
    least_squares_expected_over_manifest_rgb_gain: Option<[f64; 3]>,
    least_squares_expected_minus_actual_over_manifest_rgb_gain: Option<[f64; 3]>,
    manifest_sample_actual_within_1_5: u64,
    manifest_sample_expected_within_1_5: u64,
    manifest_sample_actual_near_expected_far: u64,
    manifest_sample_actual_far_expected_near: u64,
    manifest_sample_both_far: u64,
}

#[derive(Clone, Debug, Serialize)]
struct ResidualRow {
    x: u64,
    y: u64,
    selected: bool,
    expected: [u8; 4],
    actual: [u8; 4],
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
    frontmost_material_shading: Option<MaterialShadingSnapshot>,
    frontmost: Option<SurfaceLabel>,
    actual_match: Option<SurfaceLabel>,
    expected_match: Option<SurfaceLabel>,
    frontmost_cpu_base_color_rgba: Option<[u8; 4]>,
    nearest_expected_cpu_base_color_rgba: Option<[u8; 4]>,
    cpu_actual_rgb_distance: Option<f64>,
    cpu_expected_rgb_distance: Option<f64>,
    nearest_expected_cpu_actual_rgb_distance: Option<f64>,
    nearest_expected_cpu_expected_rgb_distance: Option<f64>,
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
struct ResidualGroup {
    key: String,
    rows: Vec<ResidualRow>,
}

#[derive(Clone, Debug, Serialize)]
struct SurfaceLabel {
    material_name: String,
    triangle: u64,
}

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Default)]
struct Accumulator {
    count: u64,
    expected_actual_distance_sum: f64,
    expected_actual_distance_count: u64,
    expected_actual_within_4: u64,
    expected_actual_within_8: u64,
    expected_actual_within_16: u64,
    expected_minus_actual_delta_sum: [f64; 3],
    expected_minus_actual_delta_count: u64,
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
    nearest_expected_actual_distance_sum: f64,
    nearest_expected_actual_distance_count: u64,
    nearest_expected_expected_distance_sum: f64,
    nearest_expected_expected_distance_count: u64,
    nearest_expected_cpu_actual_closer: u64,
    nearest_expected_cpu_expected_closer: u64,
    nearest_expected_cpu_tied: u64,
    nearest_expected_beats_frontmost_for_expected: u64,
    frontmost_base_texture_actual_distance_sum: f64,
    frontmost_base_texture_actual_distance_count: u64,
    frontmost_base_texture_expected_distance_sum: f64,
    frontmost_base_texture_expected_distance_count: u64,
    actual_minus_texture_delta_sum: [f64; 3],
    actual_minus_texture_delta_count: u64,
    expected_minus_texture_delta_sum: [f64; 3],
    expected_minus_texture_delta_count: u64,
    frontmost_base_texture_actual_closer: u64,
    frontmost_base_texture_expected_closer: u64,
    frontmost_base_texture_tied: u64,
    frontmost_base_texture_beats_cpu_for_expected: u64,
    texture_as_linear_srgb_actual_distance_sum: f64,
    texture_as_linear_srgb_actual_distance_count: u64,
    texture_as_linear_srgb_expected_distance_sum: f64,
    texture_as_linear_srgb_expected_distance_count: u64,
    actual_minus_texture_as_linear_srgb_delta_sum: [f64; 3],
    actual_minus_texture_as_linear_srgb_delta_count: u64,
    expected_minus_texture_as_linear_srgb_delta_sum: [f64; 3],
    expected_minus_texture_as_linear_srgb_delta_count: u64,
    frontmost_texture_as_linear_srgb_actual_closer: u64,
    frontmost_texture_as_linear_srgb_expected_closer: u64,
    frontmost_texture_as_linear_srgb_tied: u64,
    manifest_sample_actual_distance_sum: f64,
    manifest_sample_actual_distance_count: u64,
    manifest_sample_expected_distance_sum: f64,
    manifest_sample_expected_distance_count: u64,
    actual_minus_manifest_sample_delta_sum: [f64; 3],
    actual_minus_manifest_sample_delta_count: u64,
    expected_minus_manifest_sample_delta_sum: [f64; 3],
    expected_minus_manifest_sample_delta_count: u64,
    actual_over_manifest_sample_response: ResponseAccumulator,
    expected_over_manifest_sample_response: ResponseAccumulator,
    expected_minus_actual_over_manifest_sample_response: ResponseAccumulator,
    manifest_sample_actual_closer: u64,
    manifest_sample_expected_closer: u64,
    manifest_sample_tied: u64,
    manifest_sample_actual_within_1_5: u64,
    manifest_sample_expected_within_1_5: u64,
    manifest_sample_actual_within_8: u64,
    manifest_sample_expected_within_8: u64,
    manifest_sample_actual_near_expected_far: u64,
    manifest_sample_actual_far_expected_near: u64,
    manifest_sample_both_far: u64,
    best_sampling_actual_distance_sum: f64,
    best_sampling_actual_distance_count: u64,
    best_sampling_expected_distance_sum: f64,
    best_sampling_expected_distance_count: u64,
    best_sampling_actual_within_4: u64,
    best_sampling_actual_within_8: u64,
    best_sampling_actual_within_16: u64,
    best_sampling_expected_within_4: u64,
    best_sampling_expected_within_8: u64,
    best_sampling_expected_within_16: u64,
    materials: BTreeMap<String, u64>,
    shading_models: BTreeMap<String, u64>,
    shading_model_buckets: BTreeMap<String, MaterialAccumulator>,
    material_buckets: BTreeMap<String, MaterialAccumulator>,
    selection_material_buckets: BTreeMap<String, MaterialAccumulator>,
    selection_material_draw_buckets: BTreeMap<(String, String), MaterialAccumulator>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ModeAccumulator {
    count: u64,
    distance_sum: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ResponseAccumulator {
    ratio_sum: [f64; 3],
    ratio_count: u64,
    gain_numerator: [f64; 3],
    gain_denominator: [f64; 3],
}

impl ResponseAccumulator {
    fn add(&mut self, manifest: [u8; 4], target: [u8; 4]) {
        self.add_target_rgb(
            manifest,
            [
                f64::from(target[0]),
                f64::from(target[1]),
                f64::from(target[2]),
            ],
        );
    }

    fn add_signed_delta(&mut self, manifest: [u8; 4], left: [u8; 4], right: [u8; 4]) {
        self.add_target_rgb(
            manifest,
            [
                f64::from(left[0]) - f64::from(right[0]),
                f64::from(left[1]) - f64::from(right[1]),
                f64::from(left[2]) - f64::from(right[2]),
            ],
        );
    }

    fn add_target_rgb(&mut self, manifest: [u8; 4], target: [f64; 3]) {
        if manifest[..3].iter().all(|channel| *channel > 0) {
            for channel in 0..3 {
                self.ratio_sum[channel] += target[channel] / f64::from(manifest[channel]);
            }
            self.ratio_count += 1;
        }
        for channel in 0..3 {
            let manifest = f64::from(manifest[channel]);
            if manifest > 0.0 {
                self.gain_numerator[channel] += manifest * target[channel];
                self.gain_denominator[channel] += manifest * manifest;
            }
        }
    }

    fn mean_ratio(self) -> Option<[f64; 3]> {
        mean_rgb_delta(self.ratio_sum, self.ratio_count)
    }

    fn least_squares_gain(self) -> Option<[f64; 3]> {
        self.gain_denominator
            .iter()
            .all(|denominator| *denominator > 0.0)
            .then(|| {
                [
                    self.gain_numerator[0] / self.gain_denominator[0],
                    self.gain_numerator[1] / self.gain_denominator[1],
                    self.gain_numerator[2] / self.gain_denominator[2],
                ]
            })
    }
}

#[derive(Clone, Debug, Default)]
struct MaterialAccumulator {
    count: u64,
    expected_actual_distance_sum: f64,
    expected_actual_distance_count: u64,
    expected_actual_within_4: u64,
    expected_actual_within_8: u64,
    expected_actual_within_16: u64,
    expected_minus_actual_delta_sum: [f64; 3],
    expected_minus_actual_delta_count: u64,
    actual_cpu_closer: u64,
    expected_cpu_closer: u64,
    cpu_tied: u64,
    cpu_actual_distance_sum: f64,
    cpu_actual_distance_count: u64,
    cpu_expected_distance_sum: f64,
    cpu_expected_distance_count: u64,
    edge_distance_lte_050px: u64,
    same_material_as_expected: u64,
    same_triangle_as_expected: u64,
    nearest_expected_actual_distance_sum: f64,
    nearest_expected_actual_distance_count: u64,
    nearest_expected_expected_distance_sum: f64,
    nearest_expected_expected_distance_count: u64,
    nearest_expected_cpu_actual_closer: u64,
    nearest_expected_cpu_expected_closer: u64,
    nearest_expected_cpu_tied: u64,
    nearest_expected_beats_frontmost_for_expected: u64,
    frontmost_base_texture_actual_distance_sum: f64,
    frontmost_base_texture_actual_distance_count: u64,
    frontmost_base_texture_expected_distance_sum: f64,
    frontmost_base_texture_expected_distance_count: u64,
    actual_minus_texture_delta_sum: [f64; 3],
    actual_minus_texture_delta_count: u64,
    expected_minus_texture_delta_sum: [f64; 3],
    expected_minus_texture_delta_count: u64,
    frontmost_base_texture_actual_closer: u64,
    frontmost_base_texture_expected_closer: u64,
    frontmost_base_texture_tied: u64,
    frontmost_base_texture_beats_cpu_for_expected: u64,
    texture_as_linear_srgb_actual_distance_sum: f64,
    texture_as_linear_srgb_actual_distance_count: u64,
    texture_as_linear_srgb_expected_distance_sum: f64,
    texture_as_linear_srgb_expected_distance_count: u64,
    actual_minus_texture_as_linear_srgb_delta_sum: [f64; 3],
    actual_minus_texture_as_linear_srgb_delta_count: u64,
    expected_minus_texture_as_linear_srgb_delta_sum: [f64; 3],
    expected_minus_texture_as_linear_srgb_delta_count: u64,
    frontmost_texture_as_linear_srgb_actual_closer: u64,
    frontmost_texture_as_linear_srgb_expected_closer: u64,
    frontmost_texture_as_linear_srgb_tied: u64,
    manifest_sample_actual_distance_sum: f64,
    manifest_sample_actual_distance_count: u64,
    manifest_sample_expected_distance_sum: f64,
    manifest_sample_expected_distance_count: u64,
    actual_minus_manifest_sample_delta_sum: [f64; 3],
    actual_minus_manifest_sample_delta_count: u64,
    expected_minus_manifest_sample_delta_sum: [f64; 3],
    expected_minus_manifest_sample_delta_count: u64,
    actual_over_manifest_sample_response: ResponseAccumulator,
    expected_over_manifest_sample_response: ResponseAccumulator,
    expected_minus_actual_over_manifest_sample_response: ResponseAccumulator,
    manifest_sample_actual_closer: u64,
    manifest_sample_expected_closer: u64,
    manifest_sample_tied: u64,
    manifest_sample_actual_within_1_5: u64,
    manifest_sample_expected_within_1_5: u64,
    manifest_sample_actual_within_8: u64,
    manifest_sample_expected_within_8: u64,
    manifest_sample_actual_near_expected_far: u64,
    manifest_sample_actual_far_expected_near: u64,
    manifest_sample_both_far: u64,
    best_sampling_actual_distance_sum: f64,
    best_sampling_actual_distance_count: u64,
    best_sampling_expected_distance_sum: f64,
    best_sampling_expected_distance_count: u64,
    best_sampling_actual_within_4: u64,
    best_sampling_actual_within_8: u64,
    best_sampling_actual_within_16: u64,
    best_sampling_expected_within_4: u64,
    best_sampling_expected_within_8: u64,
    best_sampling_expected_within_16: u64,
    shading_models: BTreeMap<String, u64>,
    actual_modes: BTreeMap<String, ModeAccumulator>,
    expected_modes: BTreeMap<String, ModeAccumulator>,
}

impl Accumulator {
    fn add(
        &mut self,
        hotspot: &Value,
        selection_surface: Option<&SurfaceLabel>,
        selection_draw_key: Option<&str>,
        selection_rgba: Option<[u8; 4]>,
    ) {
        self.count += 1;
        self.add_expected_actual(rgba_at(hotspot, "/actual"), rgba_at(hotspot, "/expected"));

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
        let nearest_expected_actual_cpu =
            candidate_rgb_distance(hotspot, "/nearest_visible_expected", "/actual");
        let nearest_expected_expected_cpu =
            candidate_rgb_distance(hotspot, "/nearest_visible_expected", "/expected");
        self.add_nearest_expected_distances(
            nearest_expected_actual_cpu,
            nearest_expected_expected_cpu,
            expected_cpu,
        );
        let frontmost_base_texture_actual =
            f64_at(hotspot, "/frontmost_base_texture_actual_rgb_distance");
        let frontmost_base_texture_expected =
            f64_at(hotspot, "/frontmost_base_texture_expected_rgb_distance");
        self.add_frontmost_base_texture_distances(
            frontmost_base_texture_actual,
            frontmost_base_texture_expected,
            expected_cpu,
        );
        let actual_minus_texture =
            signed_rgb_delta_at(hotspot, "/actual", "/frontmost_base_texture_rgba");
        let expected_minus_texture =
            signed_rgb_delta_at(hotspot, "/expected", "/frontmost_base_texture_rgba");
        self.add_frontmost_base_texture_deltas(actual_minus_texture, expected_minus_texture);
        self.add_frontmost_texture_as_linear_srgb(
            rgba_at(hotspot, "/frontmost_base_texture_rgba"),
            rgba_at(hotspot, "/actual"),
            rgba_at(hotspot, "/expected"),
        );
        self.add_manifest_sample(
            selection_rgba,
            rgba_at(hotspot, "/actual"),
            rgba_at(hotspot, "/expected"),
        );

        let edge = f64_at(hotspot, "/frontmost_visible/edge_distance_pixels");
        if let Some(edge) = edge {
            self.edge_distance_sum += edge;
            self.edge_distance_count += 1;
            self.edge_distance_lte_025px += u64::from(edge <= 0.25);
            self.edge_distance_lte_050px += u64::from(edge <= 0.50);
            self.edge_distance_lte_100px += u64::from(edge <= 1.00);
        }

        let frontmost = surface_at(hotspot, "/frontmost_visible");
        let actual = surface_at(hotspot, "/nearest_visible_actual");
        let expected = surface_at(hotspot, "/nearest_visible_expected");
        let shading_model = str_at(hotspot, "/frontmost_visible/material_shading/model");
        if let Some(model) = shading_model {
            *self.shading_models.entry(model.to_owned()).or_default() += 1;
        }
        let frontmost_same_material_as_expected =
            same_material(frontmost.as_ref(), expected.as_ref());
        let frontmost_same_triangle_as_expected =
            frontmost.as_ref().zip(expected.as_ref()).is_some_and(|(left, right)| {
                left.material_name == right.material_name && left.triangle == right.triangle
            });
        if let Some(model) = shading_model {
            self.shading_model_buckets
                .entry(model.to_owned())
                .or_default()
                .add(
                    hotspot,
                    actual_cpu,
                    expected_cpu,
                    nearest_expected_actual_cpu,
                    nearest_expected_expected_cpu,
                    frontmost_base_texture_actual,
                    frontmost_base_texture_expected,
                    actual_minus_texture,
                    expected_minus_texture,
                    shading_model,
                    selection_rgba,
                    edge,
                    frontmost_same_material_as_expected,
                    frontmost_same_triangle_as_expected,
                );
        }
        if same_material(frontmost.as_ref(), actual.as_ref()) {
            self.same_material_as_actual += 1;
        }
        if frontmost_same_material_as_expected {
            self.same_material_as_expected += 1;
        }
        if frontmost.as_ref().zip(actual.as_ref()).is_some_and(|(left, right)| {
            left.material_name == right.material_name && left.triangle == right.triangle
        }) {
            self.same_triangle_as_actual += 1;
        }
        if frontmost_same_triangle_as_expected {
            self.same_triangle_as_expected += 1;
        }
        if let Some(surface) = frontmost {
            *self.materials.entry(surface.material_name.clone()).or_default() += 1;
            self.material_buckets
                .entry(surface.material_name)
                .or_default()
                .add(
                    hotspot,
                    actual_cpu,
                    expected_cpu,
                    nearest_expected_actual_cpu,
                    nearest_expected_expected_cpu,
                    frontmost_base_texture_actual,
                    frontmost_base_texture_expected,
                    actual_minus_texture,
                    expected_minus_texture,
                    shading_model,
                    selection_rgba,
                    edge,
                    frontmost_same_material_as_expected,
                    frontmost_same_triangle_as_expected,
                );
        }
        if let Some(surface) = selection_surface {
            let same_material_as_expected = same_material(Some(surface), expected.as_ref());
            let same_triangle_as_expected =
                expected.as_ref().is_some_and(|expected| {
                    surface.material_name == expected.material_name
                        && surface.triangle == expected.triangle
                });
            self.selection_material_buckets
                .entry(surface.material_name.clone())
                .or_default()
                .add(
                    hotspot,
                    actual_cpu,
                    expected_cpu,
                    nearest_expected_actual_cpu,
                    nearest_expected_expected_cpu,
                    frontmost_base_texture_actual,
                    frontmost_base_texture_expected,
                    actual_minus_texture,
                    expected_minus_texture,
                    shading_model,
                    selection_rgba,
                    edge,
                    same_material_as_expected,
                    same_triangle_as_expected,
                );
        }
        if let Some((surface, draw_key)) = selection_surface.zip(selection_draw_key) {
            let same_material_as_expected = same_material(Some(surface), expected.as_ref());
            let same_triangle_as_expected =
                expected.as_ref().is_some_and(|expected| {
                    surface.material_name == expected.material_name
                        && surface.triangle == expected.triangle
                });
            self.selection_material_draw_buckets
                .entry((surface.material_name.clone(), draw_key.to_owned()))
                .or_default()
                .add(
                    hotspot,
                    actual_cpu,
                    expected_cpu,
                    nearest_expected_actual_cpu,
                    nearest_expected_expected_cpu,
                    frontmost_base_texture_actual,
                    frontmost_base_texture_expected,
                    actual_minus_texture,
                    expected_minus_texture,
                    shading_model,
                    selection_rgba,
                    edge,
                    same_material_as_expected,
                    same_triangle_as_expected,
                );
        }

        let best_actual_sampling = best_sampling_mode(
            hotspot,
            "/frontmost_texture_sampling_variants",
            "actual_rgb_distance",
        );
        let best_expected_sampling = best_sampling_mode(
            hotspot,
            "/frontmost_texture_sampling_variants",
            "expected_rgb_distance",
        );
        self.add_best_sampling_distances(
            best_actual_sampling.as_ref().map(|best| best.distance),
            best_expected_sampling.as_ref().map(|best| best.distance),
        );

        if let Some(best) = best_actual_sampling {
            push_mode_count(&mut self.actual_modes, best);
        }
        if let Some(best) = best_expected_sampling {
            push_mode_count(&mut self.expected_modes, best);
        }
    }

    fn finish(self) -> BucketStats {
        BucketStats {
            count: self.count,
            mean_expected_actual_rgb_distance: mean(
                self.expected_actual_distance_sum,
                self.expected_actual_distance_count,
            ),
            expected_actual_within_4: self.expected_actual_within_4,
            expected_actual_within_8: self.expected_actual_within_8,
            expected_actual_within_16: self.expected_actual_within_16,
            mean_expected_minus_actual_rgb_delta: mean_rgb_delta(
                self.expected_minus_actual_delta_sum,
                self.expected_minus_actual_delta_count,
            ),
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
            mean_nearest_expected_cpu_actual_rgb_distance: mean(
                self.nearest_expected_actual_distance_sum,
                self.nearest_expected_actual_distance_count,
            ),
            mean_nearest_expected_cpu_expected_rgb_distance: mean(
                self.nearest_expected_expected_distance_sum,
                self.nearest_expected_expected_distance_count,
            ),
            nearest_expected_cpu_actual_closer: self.nearest_expected_cpu_actual_closer,
            nearest_expected_cpu_expected_closer: self.nearest_expected_cpu_expected_closer,
            nearest_expected_cpu_tied: self.nearest_expected_cpu_tied,
            nearest_expected_beats_frontmost_for_expected: self
                .nearest_expected_beats_frontmost_for_expected,
            mean_frontmost_base_texture_actual_rgb_distance: mean(
                self.frontmost_base_texture_actual_distance_sum,
                self.frontmost_base_texture_actual_distance_count,
            ),
            mean_frontmost_base_texture_expected_rgb_distance: mean(
                self.frontmost_base_texture_expected_distance_sum,
                self.frontmost_base_texture_expected_distance_count,
            ),
            mean_actual_minus_texture_rgb_delta: mean_rgb_delta(
                self.actual_minus_texture_delta_sum,
                self.actual_minus_texture_delta_count,
            ),
            mean_expected_minus_texture_rgb_delta: mean_rgb_delta(
                self.expected_minus_texture_delta_sum,
                self.expected_minus_texture_delta_count,
            ),
            frontmost_base_texture_actual_closer: self.frontmost_base_texture_actual_closer,
            frontmost_base_texture_expected_closer: self.frontmost_base_texture_expected_closer,
            frontmost_base_texture_tied: self.frontmost_base_texture_tied,
            frontmost_base_texture_beats_cpu_for_expected: self
                .frontmost_base_texture_beats_cpu_for_expected,
            mean_frontmost_texture_as_linear_srgb_actual_rgb_distance: mean(
                self.texture_as_linear_srgb_actual_distance_sum,
                self.texture_as_linear_srgb_actual_distance_count,
            ),
            mean_frontmost_texture_as_linear_srgb_expected_rgb_distance: mean(
                self.texture_as_linear_srgb_expected_distance_sum,
                self.texture_as_linear_srgb_expected_distance_count,
            ),
            mean_actual_minus_texture_as_linear_srgb_rgb_delta: mean_rgb_delta(
                self.actual_minus_texture_as_linear_srgb_delta_sum,
                self.actual_minus_texture_as_linear_srgb_delta_count,
            ),
            mean_expected_minus_texture_as_linear_srgb_rgb_delta: mean_rgb_delta(
                self.expected_minus_texture_as_linear_srgb_delta_sum,
                self.expected_minus_texture_as_linear_srgb_delta_count,
            ),
            frontmost_texture_as_linear_srgb_actual_closer: self
                .frontmost_texture_as_linear_srgb_actual_closer,
            frontmost_texture_as_linear_srgb_expected_closer: self
                .frontmost_texture_as_linear_srgb_expected_closer,
            frontmost_texture_as_linear_srgb_tied: self.frontmost_texture_as_linear_srgb_tied,
            mean_manifest_sample_actual_rgb_distance: mean(
                self.manifest_sample_actual_distance_sum,
                self.manifest_sample_actual_distance_count,
            ),
            mean_manifest_sample_expected_rgb_distance: mean(
                self.manifest_sample_expected_distance_sum,
                self.manifest_sample_expected_distance_count,
            ),
            mean_actual_minus_manifest_sample_rgb_delta: mean_rgb_delta(
                self.actual_minus_manifest_sample_delta_sum,
                self.actual_minus_manifest_sample_delta_count,
            ),
            mean_expected_minus_manifest_sample_rgb_delta: mean_rgb_delta(
                self.expected_minus_manifest_sample_delta_sum,
                self.expected_minus_manifest_sample_delta_count,
            ),
            mean_actual_over_manifest_sample_rgb_ratio: self
                .actual_over_manifest_sample_response
                .mean_ratio(),
            mean_expected_over_manifest_sample_rgb_ratio: self
                .expected_over_manifest_sample_response
                .mean_ratio(),
            least_squares_actual_over_manifest_sample_rgb_gain: self
                .actual_over_manifest_sample_response
                .least_squares_gain(),
            least_squares_expected_over_manifest_sample_rgb_gain: self
                .expected_over_manifest_sample_response
                .least_squares_gain(),
            mean_expected_minus_actual_over_manifest_sample_rgb_ratio: self
                .expected_minus_actual_over_manifest_sample_response
                .mean_ratio(),
            least_squares_expected_minus_actual_over_manifest_sample_rgb_gain: self
                .expected_minus_actual_over_manifest_sample_response
                .least_squares_gain(),
            manifest_sample_actual_closer: self.manifest_sample_actual_closer,
            manifest_sample_expected_closer: self.manifest_sample_expected_closer,
            manifest_sample_tied: self.manifest_sample_tied,
            manifest_sample_actual_within_1_5: self.manifest_sample_actual_within_1_5,
            manifest_sample_expected_within_1_5: self.manifest_sample_expected_within_1_5,
            manifest_sample_actual_within_8: self.manifest_sample_actual_within_8,
            manifest_sample_expected_within_8: self.manifest_sample_expected_within_8,
            manifest_sample_actual_near_expected_far: self
                .manifest_sample_actual_near_expected_far,
            manifest_sample_actual_far_expected_near: self
                .manifest_sample_actual_far_expected_near,
            manifest_sample_both_far: self.manifest_sample_both_far,
            mean_best_sampling_actual_rgb_distance: mean(
                self.best_sampling_actual_distance_sum,
                self.best_sampling_actual_distance_count,
            ),
            mean_best_sampling_expected_rgb_distance: mean(
                self.best_sampling_expected_distance_sum,
                self.best_sampling_expected_distance_count,
            ),
            best_sampling_actual_within_4: self.best_sampling_actual_within_4,
            best_sampling_actual_within_8: self.best_sampling_actual_within_8,
            best_sampling_actual_within_16: self.best_sampling_actual_within_16,
            best_sampling_expected_within_4: self.best_sampling_expected_within_4,
            best_sampling_expected_within_8: self.best_sampling_expected_within_8,
            best_sampling_expected_within_16: self.best_sampling_expected_within_16,
            material_counts: material_counts(self.materials),
            shading_model_counts: shading_model_counts(self.shading_models),
            shading_model_buckets: shading_model_buckets(self.shading_model_buckets),
            material_buckets: material_buckets(self.material_buckets),
            selection_material_buckets: material_buckets(self.selection_material_buckets),
            selection_material_draw_buckets: material_draw_buckets(
                self.selection_material_draw_buckets,
            ),
        }
    }

    fn add_frontmost_base_texture_deltas(
        &mut self,
        actual_minus_texture: Option<[f64; 3]>,
        expected_minus_texture: Option<[f64; 3]>,
    ) {
        if let Some(delta) = actual_minus_texture {
            add_rgb_delta(&mut self.actual_minus_texture_delta_sum, delta);
            self.actual_minus_texture_delta_count += 1;
        }
        if let Some(delta) = expected_minus_texture {
            add_rgb_delta(&mut self.expected_minus_texture_delta_sum, delta);
            self.expected_minus_texture_delta_count += 1;
        }
    }

    fn add_frontmost_texture_as_linear_srgb(
        &mut self,
        texture: Option<[u8; 4]>,
        actual: Option<[u8; 4]>,
        expected: Option<[u8; 4]>,
    ) {
        let Some(texture) = texture.map(texture_as_linear_srgb_rgba) else {
            return;
        };
        let actual_distance = actual.map(|actual| rgb_distance(texture, actual));
        let expected_distance = expected.map(|expected| rgb_distance(texture, expected));
        if let Some(distance) = actual_distance {
            self.texture_as_linear_srgb_actual_distance_sum += distance;
            self.texture_as_linear_srgb_actual_distance_count += 1;
        }
        if let Some(distance) = expected_distance {
            self.texture_as_linear_srgb_expected_distance_sum += distance;
            self.texture_as_linear_srgb_expected_distance_count += 1;
        }
        if let Some(actual) = actual {
            add_rgb_delta(
                &mut self.actual_minus_texture_as_linear_srgb_delta_sum,
                signed_rgb_delta(actual, texture),
            );
            self.actual_minus_texture_as_linear_srgb_delta_count += 1;
        }
        if let Some(expected) = expected {
            add_rgb_delta(
                &mut self.expected_minus_texture_as_linear_srgb_delta_sum,
                signed_rgb_delta(expected, texture),
            );
            self.expected_minus_texture_as_linear_srgb_delta_count += 1;
        }
        match actual_distance
            .zip(expected_distance)
            .and_then(compare_f64)
        {
            Some(std::cmp::Ordering::Less) => self.frontmost_texture_as_linear_srgb_actual_closer += 1,
            Some(std::cmp::Ordering::Greater) => {
                self.frontmost_texture_as_linear_srgb_expected_closer += 1
            }
            Some(std::cmp::Ordering::Equal) => self.frontmost_texture_as_linear_srgb_tied += 1,
            None => {}
        }
    }

    fn add_manifest_sample(
        &mut self,
        manifest_rgba: Option<[u8; 4]>,
        actual: Option<[u8; 4]>,
        expected: Option<[u8; 4]>,
    ) {
        let Some(manifest_rgba) = manifest_rgba else {
            return;
        };
        let actual_distance = actual.map(|actual| rgb_distance(manifest_rgba, actual));
        let expected_distance = expected.map(|expected| rgb_distance(manifest_rgba, expected));
        if let Some(distance) = actual_distance {
            self.manifest_sample_actual_distance_sum += distance;
            self.manifest_sample_actual_distance_count += 1;
            self.manifest_sample_actual_within_1_5 += u64::from(distance <= 1.5);
            self.manifest_sample_actual_within_8 += u64::from(distance <= 8.0);
        }
        if let Some(distance) = expected_distance {
            self.manifest_sample_expected_distance_sum += distance;
            self.manifest_sample_expected_distance_count += 1;
            self.manifest_sample_expected_within_1_5 += u64::from(distance <= 1.5);
            self.manifest_sample_expected_within_8 += u64::from(distance <= 8.0);
        }
        if let Some((actual_distance, expected_distance)) = actual_distance.zip(expected_distance)
        {
            self.manifest_sample_actual_near_expected_far +=
                u64::from(actual_distance <= 1.5 && expected_distance > 32.0);
            self.manifest_sample_actual_far_expected_near +=
                u64::from(actual_distance > 32.0 && expected_distance <= 1.5);
            self.manifest_sample_both_far +=
                u64::from(actual_distance > 32.0 && expected_distance > 32.0);
        }
        if let Some(actual) = actual {
            add_rgb_delta(
                &mut self.actual_minus_manifest_sample_delta_sum,
                signed_rgb_delta(actual, manifest_rgba),
            );
            self.actual_minus_manifest_sample_delta_count += 1;
            self.actual_over_manifest_sample_response
                .add(manifest_rgba, actual);
        }
        if let Some(expected) = expected {
            add_rgb_delta(
                &mut self.expected_minus_manifest_sample_delta_sum,
                signed_rgb_delta(expected, manifest_rgba),
            );
            self.expected_minus_manifest_sample_delta_count += 1;
            self.expected_over_manifest_sample_response
                .add(manifest_rgba, expected);
        }
        if let Some((expected, actual)) = expected.zip(actual) {
            self.expected_minus_actual_over_manifest_sample_response.add_signed_delta(
                manifest_rgba,
                expected,
                actual,
            );
        }
        match actual_distance
            .zip(expected_distance)
            .and_then(compare_f64)
        {
            Some(std::cmp::Ordering::Less) => self.manifest_sample_actual_closer += 1,
            Some(std::cmp::Ordering::Greater) => self.manifest_sample_expected_closer += 1,
            Some(std::cmp::Ordering::Equal) => self.manifest_sample_tied += 1,
            None => {}
        }
    }

    fn add_expected_actual(&mut self, actual: Option<[u8; 4]>, expected: Option<[u8; 4]>) {
        let Some((actual, expected)) = actual.zip(expected) else {
            return;
        };
        let distance = rgb_distance(actual, expected);
        self.expected_actual_distance_sum += distance;
        self.expected_actual_distance_count += 1;
        self.expected_actual_within_4 += u64::from(distance <= 4.0);
        self.expected_actual_within_8 += u64::from(distance <= 8.0);
        self.expected_actual_within_16 += u64::from(distance <= 16.0);
        add_rgb_delta(
            &mut self.expected_minus_actual_delta_sum,
            signed_rgb_delta(expected, actual),
        );
        self.expected_minus_actual_delta_count += 1;
    }

    fn add_nearest_expected_distances(
        &mut self,
        actual_distance: Option<f64>,
        expected_distance: Option<f64>,
        frontmost_expected_distance: Option<f64>,
    ) {
        if let Some(distance) = actual_distance {
            self.nearest_expected_actual_distance_sum += distance;
            self.nearest_expected_actual_distance_count += 1;
        }
        if let Some(distance) = expected_distance {
            self.nearest_expected_expected_distance_sum += distance;
            self.nearest_expected_expected_distance_count += 1;
        }
        match actual_distance
            .zip(expected_distance)
            .and_then(compare_f64)
        {
            Some(std::cmp::Ordering::Less) => self.nearest_expected_cpu_actual_closer += 1,
            Some(std::cmp::Ordering::Greater) => self.nearest_expected_cpu_expected_closer += 1,
            Some(std::cmp::Ordering::Equal) => self.nearest_expected_cpu_tied += 1,
            None => {}
        }
        if expected_distance
            .zip(frontmost_expected_distance)
            .is_some_and(|(nearest, frontmost)| nearest < frontmost)
        {
            self.nearest_expected_beats_frontmost_for_expected += 1;
        }
    }

    fn add_frontmost_base_texture_distances(
        &mut self,
        actual_distance: Option<f64>,
        expected_distance: Option<f64>,
        cpu_expected_distance: Option<f64>,
    ) {
        if let Some(distance) = actual_distance {
            self.frontmost_base_texture_actual_distance_sum += distance;
            self.frontmost_base_texture_actual_distance_count += 1;
        }
        if let Some(distance) = expected_distance {
            self.frontmost_base_texture_expected_distance_sum += distance;
            self.frontmost_base_texture_expected_distance_count += 1;
        }
        match actual_distance
            .zip(expected_distance)
            .and_then(compare_f64)
        {
            Some(std::cmp::Ordering::Less) => self.frontmost_base_texture_actual_closer += 1,
            Some(std::cmp::Ordering::Greater) => self.frontmost_base_texture_expected_closer += 1,
            Some(std::cmp::Ordering::Equal) => self.frontmost_base_texture_tied += 1,
            None => {}
        }
        if expected_distance
            .zip(cpu_expected_distance)
            .is_some_and(|(texture, cpu)| texture < cpu)
        {
            self.frontmost_base_texture_beats_cpu_for_expected += 1;
        }
    }

    fn add_best_sampling_distances(
        &mut self,
        actual_distance: Option<f64>,
        expected_distance: Option<f64>,
    ) {
        if let Some(distance) = actual_distance {
            self.best_sampling_actual_distance_sum += distance;
            self.best_sampling_actual_distance_count += 1;
            self.best_sampling_actual_within_4 += u64::from(distance <= 4.0);
            self.best_sampling_actual_within_8 += u64::from(distance <= 8.0);
            self.best_sampling_actual_within_16 += u64::from(distance <= 16.0);
        }
        if let Some(distance) = expected_distance {
            self.best_sampling_expected_distance_sum += distance;
            self.best_sampling_expected_distance_count += 1;
            self.best_sampling_expected_within_4 += u64::from(distance <= 4.0);
            self.best_sampling_expected_within_8 += u64::from(distance <= 8.0);
            self.best_sampling_expected_within_16 += u64::from(distance <= 16.0);
        }
    }
}

impl MaterialAccumulator {
    fn add(
        &mut self,
        hotspot: &Value,
        actual_cpu: Option<f64>,
        expected_cpu: Option<f64>,
        nearest_expected_actual_cpu: Option<f64>,
        nearest_expected_expected_cpu: Option<f64>,
        frontmost_base_texture_actual: Option<f64>,
        frontmost_base_texture_expected: Option<f64>,
        actual_minus_texture: Option<[f64; 3]>,
        expected_minus_texture: Option<[f64; 3]>,
        shading_model: Option<&str>,
        selection_rgba: Option<[u8; 4]>,
        edge: Option<f64>,
        same_material_as_expected: bool,
        same_triangle_as_expected: bool,
    ) {
        self.count += 1;
        self.add_expected_actual(rgba_at(hotspot, "/actual"), rgba_at(hotspot, "/expected"));
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
        self.edge_distance_lte_050px += u64::from(edge.is_some_and(|edge| edge <= 0.50));
        self.same_material_as_expected += u64::from(same_material_as_expected);
        self.same_triangle_as_expected += u64::from(same_triangle_as_expected);
        self.add_nearest_expected_distances(
            nearest_expected_actual_cpu,
            nearest_expected_expected_cpu,
            expected_cpu,
        );
        self.add_frontmost_base_texture_distances(
            frontmost_base_texture_actual,
            frontmost_base_texture_expected,
            expected_cpu,
        );
        self.add_frontmost_base_texture_deltas(actual_minus_texture, expected_minus_texture);
        self.add_frontmost_texture_as_linear_srgb(
            rgba_at(hotspot, "/frontmost_base_texture_rgba"),
            rgba_at(hotspot, "/actual"),
            rgba_at(hotspot, "/expected"),
        );
        if let Some(model) = shading_model {
            *self.shading_models.entry(model.to_owned()).or_default() += 1;
        }
        self.add_manifest_sample(selection_rgba, rgba_at(hotspot, "/actual"), rgba_at(hotspot, "/expected"));
        let best_actual_sampling = best_sampling_mode(
            hotspot,
            "/frontmost_texture_sampling_variants",
            "actual_rgb_distance",
        );
        let best_expected_sampling = best_sampling_mode(
            hotspot,
            "/frontmost_texture_sampling_variants",
            "expected_rgb_distance",
        );
        self.add_best_sampling_distances(
            best_actual_sampling.as_ref().map(|best| best.distance),
            best_expected_sampling.as_ref().map(|best| best.distance),
        );

        if let Some(best) = best_actual_sampling {
            push_mode_count(&mut self.actual_modes, best);
        }
        if let Some(best) = best_expected_sampling {
            push_mode_count(&mut self.expected_modes, best);
        }
    }

    fn finish(self, material_name: String) -> MaterialBucket {
        MaterialBucket {
            material_name,
            count: self.count,
            mean_expected_actual_rgb_distance: mean(
                self.expected_actual_distance_sum,
                self.expected_actual_distance_count,
            ),
            expected_actual_within_4: self.expected_actual_within_4,
            expected_actual_within_8: self.expected_actual_within_8,
            expected_actual_within_16: self.expected_actual_within_16,
            mean_expected_minus_actual_rgb_delta: mean_rgb_delta(
                self.expected_minus_actual_delta_sum,
                self.expected_minus_actual_delta_count,
            ),
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
            edge_distance_lte_050px: self.edge_distance_lte_050px,
            same_material_as_expected: self.same_material_as_expected,
            same_triangle_as_expected: self.same_triangle_as_expected,
            mean_nearest_expected_cpu_actual_rgb_distance: mean(
                self.nearest_expected_actual_distance_sum,
                self.nearest_expected_actual_distance_count,
            ),
            mean_nearest_expected_cpu_expected_rgb_distance: mean(
                self.nearest_expected_expected_distance_sum,
                self.nearest_expected_expected_distance_count,
            ),
            nearest_expected_cpu_actual_closer: self.nearest_expected_cpu_actual_closer,
            nearest_expected_cpu_expected_closer: self.nearest_expected_cpu_expected_closer,
            nearest_expected_cpu_tied: self.nearest_expected_cpu_tied,
            nearest_expected_beats_frontmost_for_expected: self
                .nearest_expected_beats_frontmost_for_expected,
            mean_frontmost_base_texture_actual_rgb_distance: mean(
                self.frontmost_base_texture_actual_distance_sum,
                self.frontmost_base_texture_actual_distance_count,
            ),
            mean_frontmost_base_texture_expected_rgb_distance: mean(
                self.frontmost_base_texture_expected_distance_sum,
                self.frontmost_base_texture_expected_distance_count,
            ),
            mean_actual_minus_texture_rgb_delta: mean_rgb_delta(
                self.actual_minus_texture_delta_sum,
                self.actual_minus_texture_delta_count,
            ),
            mean_expected_minus_texture_rgb_delta: mean_rgb_delta(
                self.expected_minus_texture_delta_sum,
                self.expected_minus_texture_delta_count,
            ),
            frontmost_base_texture_actual_closer: self.frontmost_base_texture_actual_closer,
            frontmost_base_texture_expected_closer: self.frontmost_base_texture_expected_closer,
            frontmost_base_texture_tied: self.frontmost_base_texture_tied,
            frontmost_base_texture_beats_cpu_for_expected: self
                .frontmost_base_texture_beats_cpu_for_expected,
            mean_frontmost_texture_as_linear_srgb_actual_rgb_distance: mean(
                self.texture_as_linear_srgb_actual_distance_sum,
                self.texture_as_linear_srgb_actual_distance_count,
            ),
            mean_frontmost_texture_as_linear_srgb_expected_rgb_distance: mean(
                self.texture_as_linear_srgb_expected_distance_sum,
                self.texture_as_linear_srgb_expected_distance_count,
            ),
            mean_actual_minus_texture_as_linear_srgb_rgb_delta: mean_rgb_delta(
                self.actual_minus_texture_as_linear_srgb_delta_sum,
                self.actual_minus_texture_as_linear_srgb_delta_count,
            ),
            mean_expected_minus_texture_as_linear_srgb_rgb_delta: mean_rgb_delta(
                self.expected_minus_texture_as_linear_srgb_delta_sum,
                self.expected_minus_texture_as_linear_srgb_delta_count,
            ),
            frontmost_texture_as_linear_srgb_actual_closer: self
                .frontmost_texture_as_linear_srgb_actual_closer,
            frontmost_texture_as_linear_srgb_expected_closer: self
                .frontmost_texture_as_linear_srgb_expected_closer,
            frontmost_texture_as_linear_srgb_tied: self.frontmost_texture_as_linear_srgb_tied,
            mean_manifest_sample_actual_rgb_distance: mean(
                self.manifest_sample_actual_distance_sum,
                self.manifest_sample_actual_distance_count,
            ),
            mean_manifest_sample_expected_rgb_distance: mean(
                self.manifest_sample_expected_distance_sum,
                self.manifest_sample_expected_distance_count,
            ),
            mean_actual_minus_manifest_sample_rgb_delta: mean_rgb_delta(
                self.actual_minus_manifest_sample_delta_sum,
                self.actual_minus_manifest_sample_delta_count,
            ),
            mean_expected_minus_manifest_sample_rgb_delta: mean_rgb_delta(
                self.expected_minus_manifest_sample_delta_sum,
                self.expected_minus_manifest_sample_delta_count,
            ),
            mean_actual_over_manifest_sample_rgb_ratio: self
                .actual_over_manifest_sample_response
                .mean_ratio(),
            mean_expected_over_manifest_sample_rgb_ratio: self
                .expected_over_manifest_sample_response
                .mean_ratio(),
            least_squares_actual_over_manifest_sample_rgb_gain: self
                .actual_over_manifest_sample_response
                .least_squares_gain(),
            least_squares_expected_over_manifest_sample_rgb_gain: self
                .expected_over_manifest_sample_response
                .least_squares_gain(),
            mean_expected_minus_actual_over_manifest_sample_rgb_ratio: self
                .expected_minus_actual_over_manifest_sample_response
                .mean_ratio(),
            least_squares_expected_minus_actual_over_manifest_sample_rgb_gain: self
                .expected_minus_actual_over_manifest_sample_response
                .least_squares_gain(),
            manifest_sample_actual_closer: self.manifest_sample_actual_closer,
            manifest_sample_expected_closer: self.manifest_sample_expected_closer,
            manifest_sample_tied: self.manifest_sample_tied,
            manifest_sample_actual_within_1_5: self.manifest_sample_actual_within_1_5,
            manifest_sample_expected_within_1_5: self.manifest_sample_expected_within_1_5,
            manifest_sample_actual_within_8: self.manifest_sample_actual_within_8,
            manifest_sample_expected_within_8: self.manifest_sample_expected_within_8,
            manifest_sample_actual_near_expected_far: self
                .manifest_sample_actual_near_expected_far,
            manifest_sample_actual_far_expected_near: self
                .manifest_sample_actual_far_expected_near,
            manifest_sample_both_far: self.manifest_sample_both_far,
            mean_best_sampling_actual_rgb_distance: mean(
                self.best_sampling_actual_distance_sum,
                self.best_sampling_actual_distance_count,
            ),
            mean_best_sampling_expected_rgb_distance: mean(
                self.best_sampling_expected_distance_sum,
                self.best_sampling_expected_distance_count,
            ),
            best_sampling_actual_within_4: self.best_sampling_actual_within_4,
            best_sampling_actual_within_8: self.best_sampling_actual_within_8,
            best_sampling_actual_within_16: self.best_sampling_actual_within_16,
            best_sampling_expected_within_4: self.best_sampling_expected_within_4,
            best_sampling_expected_within_8: self.best_sampling_expected_within_8,
            best_sampling_expected_within_16: self.best_sampling_expected_within_16,
            shading_model_counts: shading_model_counts(self.shading_models),
            best_sampling_modes_for_actual: mode_counts(self.actual_modes),
            best_sampling_modes_for_expected: mode_counts(self.expected_modes),
        }
    }

    fn add_frontmost_base_texture_deltas(
        &mut self,
        actual_minus_texture: Option<[f64; 3]>,
        expected_minus_texture: Option<[f64; 3]>,
    ) {
        if let Some(delta) = actual_minus_texture {
            add_rgb_delta(&mut self.actual_minus_texture_delta_sum, delta);
            self.actual_minus_texture_delta_count += 1;
        }
        if let Some(delta) = expected_minus_texture {
            add_rgb_delta(&mut self.expected_minus_texture_delta_sum, delta);
            self.expected_minus_texture_delta_count += 1;
        }
    }

    fn add_frontmost_texture_as_linear_srgb(
        &mut self,
        texture: Option<[u8; 4]>,
        actual: Option<[u8; 4]>,
        expected: Option<[u8; 4]>,
    ) {
        let Some(texture) = texture.map(texture_as_linear_srgb_rgba) else {
            return;
        };
        let actual_distance = actual.map(|actual| rgb_distance(texture, actual));
        let expected_distance = expected.map(|expected| rgb_distance(texture, expected));
        if let Some(distance) = actual_distance {
            self.texture_as_linear_srgb_actual_distance_sum += distance;
            self.texture_as_linear_srgb_actual_distance_count += 1;
        }
        if let Some(distance) = expected_distance {
            self.texture_as_linear_srgb_expected_distance_sum += distance;
            self.texture_as_linear_srgb_expected_distance_count += 1;
        }
        if let Some(actual) = actual {
            add_rgb_delta(
                &mut self.actual_minus_texture_as_linear_srgb_delta_sum,
                signed_rgb_delta(actual, texture),
            );
            self.actual_minus_texture_as_linear_srgb_delta_count += 1;
        }
        if let Some(expected) = expected {
            add_rgb_delta(
                &mut self.expected_minus_texture_as_linear_srgb_delta_sum,
                signed_rgb_delta(expected, texture),
            );
            self.expected_minus_texture_as_linear_srgb_delta_count += 1;
        }
        match actual_distance
            .zip(expected_distance)
            .and_then(compare_f64)
        {
            Some(std::cmp::Ordering::Less) => self.frontmost_texture_as_linear_srgb_actual_closer += 1,
            Some(std::cmp::Ordering::Greater) => {
                self.frontmost_texture_as_linear_srgb_expected_closer += 1
            }
            Some(std::cmp::Ordering::Equal) => self.frontmost_texture_as_linear_srgb_tied += 1,
            None => {}
        }
    }

    fn add_manifest_sample(
        &mut self,
        manifest_rgba: Option<[u8; 4]>,
        actual: Option<[u8; 4]>,
        expected: Option<[u8; 4]>,
    ) {
        let Some(manifest_rgba) = manifest_rgba else {
            return;
        };
        let actual_distance = actual.map(|actual| rgb_distance(manifest_rgba, actual));
        let expected_distance = expected.map(|expected| rgb_distance(manifest_rgba, expected));
        if let Some(distance) = actual_distance {
            self.manifest_sample_actual_distance_sum += distance;
            self.manifest_sample_actual_distance_count += 1;
            self.manifest_sample_actual_within_1_5 += u64::from(distance <= 1.5);
            self.manifest_sample_actual_within_8 += u64::from(distance <= 8.0);
        }
        if let Some(distance) = expected_distance {
            self.manifest_sample_expected_distance_sum += distance;
            self.manifest_sample_expected_distance_count += 1;
            self.manifest_sample_expected_within_1_5 += u64::from(distance <= 1.5);
            self.manifest_sample_expected_within_8 += u64::from(distance <= 8.0);
        }
        if let Some((actual_distance, expected_distance)) = actual_distance.zip(expected_distance)
        {
            self.manifest_sample_actual_near_expected_far +=
                u64::from(actual_distance <= 1.5 && expected_distance > 32.0);
            self.manifest_sample_actual_far_expected_near +=
                u64::from(actual_distance > 32.0 && expected_distance <= 1.5);
            self.manifest_sample_both_far +=
                u64::from(actual_distance > 32.0 && expected_distance > 32.0);
        }
        if let Some(actual) = actual {
            add_rgb_delta(
                &mut self.actual_minus_manifest_sample_delta_sum,
                signed_rgb_delta(actual, manifest_rgba),
            );
            self.actual_minus_manifest_sample_delta_count += 1;
            self.actual_over_manifest_sample_response
                .add(manifest_rgba, actual);
        }
        if let Some(expected) = expected {
            add_rgb_delta(
                &mut self.expected_minus_manifest_sample_delta_sum,
                signed_rgb_delta(expected, manifest_rgba),
            );
            self.expected_minus_manifest_sample_delta_count += 1;
            self.expected_over_manifest_sample_response
                .add(manifest_rgba, expected);
        }
        if let Some((expected, actual)) = expected.zip(actual) {
            self.expected_minus_actual_over_manifest_sample_response.add_signed_delta(
                manifest_rgba,
                expected,
                actual,
            );
        }
        match actual_distance
            .zip(expected_distance)
            .and_then(compare_f64)
        {
            Some(std::cmp::Ordering::Less) => self.manifest_sample_actual_closer += 1,
            Some(std::cmp::Ordering::Greater) => self.manifest_sample_expected_closer += 1,
            Some(std::cmp::Ordering::Equal) => self.manifest_sample_tied += 1,
            None => {}
        }
    }

    fn add_expected_actual(&mut self, actual: Option<[u8; 4]>, expected: Option<[u8; 4]>) {
        let Some((actual, expected)) = actual.zip(expected) else {
            return;
        };
        let distance = rgb_distance(actual, expected);
        self.expected_actual_distance_sum += distance;
        self.expected_actual_distance_count += 1;
        self.expected_actual_within_4 += u64::from(distance <= 4.0);
        self.expected_actual_within_8 += u64::from(distance <= 8.0);
        self.expected_actual_within_16 += u64::from(distance <= 16.0);
        add_rgb_delta(
            &mut self.expected_minus_actual_delta_sum,
            signed_rgb_delta(expected, actual),
        );
        self.expected_minus_actual_delta_count += 1;
    }

    fn add_nearest_expected_distances(
        &mut self,
        actual_distance: Option<f64>,
        expected_distance: Option<f64>,
        frontmost_expected_distance: Option<f64>,
    ) {
        if let Some(distance) = actual_distance {
            self.nearest_expected_actual_distance_sum += distance;
            self.nearest_expected_actual_distance_count += 1;
        }
        if let Some(distance) = expected_distance {
            self.nearest_expected_expected_distance_sum += distance;
            self.nearest_expected_expected_distance_count += 1;
        }
        match actual_distance
            .zip(expected_distance)
            .and_then(compare_f64)
        {
            Some(std::cmp::Ordering::Less) => self.nearest_expected_cpu_actual_closer += 1,
            Some(std::cmp::Ordering::Greater) => self.nearest_expected_cpu_expected_closer += 1,
            Some(std::cmp::Ordering::Equal) => self.nearest_expected_cpu_tied += 1,
            None => {}
        }
        if expected_distance
            .zip(frontmost_expected_distance)
            .is_some_and(|(nearest, frontmost)| nearest < frontmost)
        {
            self.nearest_expected_beats_frontmost_for_expected += 1;
        }
    }

    fn add_frontmost_base_texture_distances(
        &mut self,
        actual_distance: Option<f64>,
        expected_distance: Option<f64>,
        cpu_expected_distance: Option<f64>,
    ) {
        if let Some(distance) = actual_distance {
            self.frontmost_base_texture_actual_distance_sum += distance;
            self.frontmost_base_texture_actual_distance_count += 1;
        }
        if let Some(distance) = expected_distance {
            self.frontmost_base_texture_expected_distance_sum += distance;
            self.frontmost_base_texture_expected_distance_count += 1;
        }
        match actual_distance
            .zip(expected_distance)
            .and_then(compare_f64)
        {
            Some(std::cmp::Ordering::Less) => self.frontmost_base_texture_actual_closer += 1,
            Some(std::cmp::Ordering::Greater) => self.frontmost_base_texture_expected_closer += 1,
            Some(std::cmp::Ordering::Equal) => self.frontmost_base_texture_tied += 1,
            None => {}
        }
        if expected_distance
            .zip(cpu_expected_distance)
            .is_some_and(|(texture, cpu)| texture < cpu)
        {
            self.frontmost_base_texture_beats_cpu_for_expected += 1;
        }
    }

    fn add_best_sampling_distances(
        &mut self,
        actual_distance: Option<f64>,
        expected_distance: Option<f64>,
    ) {
        if let Some(distance) = actual_distance {
            self.best_sampling_actual_distance_sum += distance;
            self.best_sampling_actual_distance_count += 1;
            self.best_sampling_actual_within_4 += u64::from(distance <= 4.0);
            self.best_sampling_actual_within_8 += u64::from(distance <= 8.0);
            self.best_sampling_actual_within_16 += u64::from(distance <= 16.0);
        }
        if let Some(distance) = expected_distance {
            self.best_sampling_expected_distance_sum += distance;
            self.best_sampling_expected_distance_count += 1;
            self.best_sampling_expected_within_4 += u64::from(distance <= 4.0);
            self.best_sampling_expected_within_8 += u64::from(distance <= 8.0);
            self.best_sampling_expected_within_16 += u64::from(distance <= 16.0);
        }
    }
}

fn push_mode_count(modes: &mut BTreeMap<String, ModeAccumulator>, best: BestSamplingMode) {
    modes
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
    let selected_surfaces = manifest
        .map(manifest_surfaces)
        .transpose()?
        .unwrap_or_default();
    let selected_rgba = manifest.map(manifest_rgba).transpose()?.unwrap_or_default();
    let selected_draw_keys = manifest
        .map(manifest_draw_keys)
        .transpose()?
        .unwrap_or_default();
    let selected_sources = manifest
        .map(manifest_selection_sources)
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
    let mut source_accs = BTreeMap::<String, Accumulator>::new();
    let mut residuals = Vec::new();

    for hotspot in hotspot_items {
        let Some(pixel) = pixel_key(hotspot) else {
            continue;
        };
        let is_selected = selected.is_empty() || selected.contains(&pixel);
        let is_carried = !baseline_selected.is_empty() && baseline_selected.contains(&pixel);
        let is_new = !baseline_selected.is_empty() && is_selected && !is_carried;
        let selection_surface = selected_surfaces.get(&pixel);
        let selection_rgba = selected_rgba.get(&pixel).copied();
        let selection_draw_key = selected_draw_keys.get(&pixel).map(String::as_str);
        let selection_source = selected_sources.get(&pixel).map(String::as_str);
        all.add(hotspot, selection_surface, selection_draw_key, selection_rgba);
        if is_selected {
            selected_acc.add(hotspot, selection_surface, selection_draw_key, selection_rgba);
            if let Some(selection_source) = selection_source {
                source_accs.entry(selection_source.to_owned()).or_default().add(
                    hotspot,
                    selection_surface,
                    selection_draw_key,
                    selection_rgba,
                );
            }
            if is_carried {
                carried_acc.add(hotspot, selection_surface, selection_draw_key, selection_rgba);
            } else if is_new {
                new_acc.add(hotspot, selection_surface, selection_draw_key, selection_rgba);
            }
        } else {
            missing_acc.add(hotspot, None, None, None);
        }
        if let Some(row) = residual_row(
            hotspot,
            is_selected,
            selection_surface.cloned(),
            selection_draw_key.map(ToOwned::to_owned),
            selection_rgba,
        ) {
            residuals.push(row);
        }
    }
    sort_residuals(&mut residuals);
    let top_residuals_by_shading_model = residual_groups_by_shading_model(&residuals, top);
    residuals.truncate(top);

    let manifest_count = u64::try_from(selected.len()).unwrap_or(u64::MAX);
    let baseline_manifest_count = u64::try_from(baseline_selected.len()).unwrap_or(u64::MAX);
    let selected_count = if selected.is_empty() {
        u64::try_from(hotspot_items.len()).unwrap_or(u64::MAX)
    } else {
        selected_acc.count
    };
    let missing_selection_count = if selected.is_empty() {
        0
    } else {
        missing_acc.count
    };
    let carried_selection_count = carried_acc.count;
    let new_selection_count = new_acc.count;
    let all_stats = all.finish();
    let selected_stats = selected_acc.finish();
    let missing_selection_stats = missing_acc.finish();
    let carried_selection_stats = carried_acc.finish();
    let new_selection_stats = new_acc.finish();
    let recommended_probes =
        recommended_probes(&selected_stats.selection_material_draw_buckets, top);
    let selection_source_buckets = selection_source_buckets(source_accs);
    Ok(AuditReport {
        hotspots: display_path(hotspots_path),
        manifest: manifest_path.map(display_path),
        baseline_manifest: baseline_manifest_path.map(display_path),
        hotspot_count: u64::try_from(hotspot_items.len()).unwrap_or(u64::MAX),
        manifest_count,
        baseline_manifest_count,
        selected_count,
        missing_selection_count,
        carried_selection_count,
        new_selection_count,
        all: all_stats,
        selected: selected_stats,
        missing_selection: missing_selection_stats,
        carried_selection: carried_selection_stats,
        new_selection: new_selection_stats,
        selection_source_buckets,
        recommended_probes,
        top_residuals: residuals,
        top_residuals_by_shading_model,
    })
}

fn sort_residuals(residuals: &mut [ResidualRow]) {
    residuals.sort_by(|left, right| {
        let left_distance = left.cpu_expected_rgb_distance.unwrap_or(f64::INFINITY);
        let right_distance = right.cpu_expected_rgb_distance.unwrap_or(f64::INFINITY);
        right_distance
            .partial_cmp(&left_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.y.cmp(&right.y))
            .then_with(|| left.x.cmp(&right.x))
    });
}

fn residual_groups_by_shading_model(residuals: &[ResidualRow], top: usize) -> Vec<ResidualGroup> {
    let mut groups = BTreeMap::<String, Vec<ResidualRow>>::new();
    for row in residuals {
        let key = row
            .frontmost_shading_model
            .as_deref()
            .unwrap_or("unknown")
            .to_owned();
        let rows = groups.entry(key).or_default();
        if rows.len() < top {
            rows.push(row.clone());
        }
    }
    let mut groups = groups
        .into_iter()
        .map(|(key, rows)| ResidualGroup { key, rows })
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        right
            .rows
            .len()
            .cmp(&left.rows.len())
            .then_with(|| left.key.cmp(&right.key))
    });
    groups
}

fn manifest_pixels(manifest: &Value) -> Result<HashSet<(u64, u64)>, Box<dyn Error>> {
    let corrections = manifest
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or("manifest corrections must be an array")?;
    Ok(corrections.iter().filter_map(pixel_key).collect())
}

fn manifest_surfaces(manifest: &Value) -> Result<HashMap<(u64, u64), SurfaceLabel>, Box<dyn Error>> {
    let corrections = manifest
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or("manifest corrections must be an array")?;
    Ok(corrections
        .iter()
        .filter_map(|correction| Some((pixel_key(correction)?, surface_at(correction, "/surface")?)))
        .collect())
}

fn manifest_rgba(manifest: &Value) -> Result<HashMap<(u64, u64), [u8; 4]>, Box<dyn Error>> {
    let corrections = manifest
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or("manifest corrections must be an array")?;
    Ok(corrections
        .iter()
        .filter_map(|correction| Some((pixel_key(correction)?, rgba_at(correction, "/rgba")?)))
        .collect())
}

fn manifest_draw_keys(manifest: &Value) -> Result<HashMap<(u64, u64), String>, Box<dyn Error>> {
    let corrections = manifest
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or("manifest corrections must be an array")?;
    Ok(corrections
        .iter()
        .filter_map(|correction| {
            Some((
                pixel_key(correction)?,
                draw_key_at(correction, "/sample_geometry")?,
            ))
        })
        .collect())
}

fn manifest_selection_sources(
    manifest: &Value,
) -> Result<HashMap<(u64, u64), String>, Box<dyn Error>> {
    let corrections = manifest
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or("manifest corrections must be an array")?;
    Ok(corrections
        .iter()
        .filter_map(|correction| {
            let source = correction
                .get("selection_source")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some((pixel_key(correction)?, source.to_owned()))
        })
        .collect())
}

fn residual_row(
    hotspot: &Value,
    selected: bool,
    selection_surface: Option<SurfaceLabel>,
    selection_draw_key: Option<String>,
    selection_rgba: Option<[u8; 4]>,
) -> Option<ResidualRow> {
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
        expected_actual_rgb_distance: rgb_distance(
            rgba_at(hotspot, "/expected")?,
            rgba_at(hotspot, "/actual")?,
        ),
        expected_minus_actual_rgb_delta: signed_rgb_delta(
            rgba_at(hotspot, "/expected")?,
            rgba_at(hotspot, "/actual")?,
        ),
        selection_surface,
        selection_draw_key,
        selection_rgba,
        selection_actual_rgb_distance: selection_rgba
            .zip(rgba_at(hotspot, "/actual"))
            .map(|(selection, actual)| rgb_distance(selection, actual)),
        selection_expected_rgb_distance: selection_rgba
            .zip(rgba_at(hotspot, "/expected"))
            .map(|(selection, expected)| rgb_distance(selection, expected)),
        actual_minus_selection_rgb_delta: selection_rgba
            .zip(rgba_at(hotspot, "/actual"))
            .map(|(selection, actual)| signed_rgb_delta(actual, selection)),
        expected_minus_selection_rgb_delta: selection_rgba
            .zip(rgba_at(hotspot, "/expected"))
            .map(|(selection, expected)| signed_rgb_delta(expected, selection)),
        frontmost_shading_model: str_at(hotspot, "/frontmost_visible/material_shading/model")
            .map(ToOwned::to_owned),
        frontmost_material_shading: material_shading_at(
            hotspot,
            "/frontmost_visible/material_shading",
        ),
        frontmost: surface_at(hotspot, "/frontmost_visible"),
        actual_match: surface_at(hotspot, "/nearest_visible_actual"),
        expected_match: surface_at(hotspot, "/nearest_visible_expected"),
        frontmost_cpu_base_color_rgba: rgba_at(hotspot, "/frontmost_cpu_base_color_rgba"),
        nearest_expected_cpu_base_color_rgba: rgba_at(
            hotspot,
            "/nearest_visible_expected/cpu_base_color_rgba",
        ),
        cpu_actual_rgb_distance: f64_at(
            hotspot,
            "/frontmost_cpu_base_color_actual_rgb_distance",
        ),
        cpu_expected_rgb_distance: f64_at(
            hotspot,
            "/frontmost_cpu_base_color_expected_rgb_distance",
        ),
        nearest_expected_cpu_actual_rgb_distance: candidate_rgb_distance(
            hotspot,
            "/nearest_visible_expected",
            "/actual",
        ),
        nearest_expected_cpu_expected_rgb_distance: candidate_rgb_distance(
            hotspot,
            "/nearest_visible_expected",
            "/expected",
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

fn draw_key_at(value: &Value, pointer: &str) -> Option<String> {
    let geometry = value.pointer(pointer)?;
    let node = geometry_u64_label(geometry, "node");
    let mesh = geometry_u64_label(geometry, "mesh");
    let primitive = geometry_u64_label(geometry, "primitive");
    let pass = geometry
        .get("pass")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    Some(format!("node{node}/mesh{mesh}/prim{primitive}/{pass}"))
}

fn geometry_u64_label(geometry: &Value, key: &str) -> String {
    geometry
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_owned())
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

fn material_shading_at(value: &Value, pointer: &str) -> Option<MaterialShadingSnapshot> {
    let value = value.pointer(pointer)?;
    Some(MaterialShadingSnapshot {
        model: value.get("model")?.as_str()?.to_owned(),
        base_color: f64_array4(value, "base_color"),
        shade_color: f64_array4(value, "shade_color"),
        emissive: f64_array3(value, "emissive"),
        metallic: value.get("metallic").and_then(Value::as_f64),
        roughness: value.get("roughness").and_then(Value::as_f64),
        occlusion_strength: value.get("occlusion_strength").and_then(Value::as_f64),
        normal_scale: value.get("normal_scale").and_then(Value::as_f64),
        unlit: value.get("unlit").and_then(Value::as_bool),
        v0_compat_shade: value.get("v0_compat_shade").and_then(Value::as_bool),
    })
}

fn f64_array3(value: &Value, key: &str) -> Option<[f64; 3]> {
    let array = value.get(key)?.as_array()?;
    Some([
        array.first()?.as_f64()?,
        array.get(1)?.as_f64()?,
        array.get(2)?.as_f64()?,
    ])
}

fn f64_array4(value: &Value, key: &str) -> Option<[f64; 4]> {
    let array = value.get(key)?.as_array()?;
    Some([
        array.first()?.as_f64()?,
        array.get(1)?.as_f64()?,
        array.get(2)?.as_f64()?,
        array.get(3)?.as_f64()?,
    ])
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

fn candidate_rgb_distance(hotspot: &Value, candidate_pointer: &str, target_pointer: &str) -> Option<f64> {
    let candidate = rgba_at(
        hotspot,
        &format!("{candidate_pointer}/cpu_base_color_rgba"),
    )?;
    let target = rgba_at(hotspot, target_pointer)?;
    Some(rgb_distance(candidate, target))
}

fn rgb_distance(left: [u8; 4], right: [u8; 4]) -> f64 {
    left.iter()
        .zip(right)
        .take(3)
        .map(|(left, right)| {
            let delta = f64::from(*left) - f64::from(right);
            delta * delta
        })
        .sum::<f64>()
        .sqrt()
}

fn texture_as_linear_srgb_rgba(rgba: [u8; 4]) -> [u8; 4] {
    [
        linear_to_srgb_u8(rgba[0]),
        linear_to_srgb_u8(rgba[1]),
        linear_to_srgb_u8(rgba[2]),
        rgba[3],
    ]
}

fn linear_to_srgb_u8(channel: u8) -> u8 {
    let linear = f64::from(channel) / 255.0;
    let srgb = if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (srgb.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn signed_rgb_delta_at(
    hotspot: &Value,
    target_pointer: &str,
    source_pointer: &str,
) -> Option<[f64; 3]> {
    Some(signed_rgb_delta(
        rgba_at(hotspot, target_pointer)?,
        rgba_at(hotspot, source_pointer)?,
    ))
}

fn signed_rgb_delta(target: [u8; 4], source: [u8; 4]) -> [f64; 3] {
    [
        f64::from(target[0]) - f64::from(source[0]),
        f64::from(target[1]) - f64::from(source[1]),
        f64::from(target[2]) - f64::from(source[2]),
    ]
}

fn add_rgb_delta(sum: &mut [f64; 3], delta: [f64; 3]) {
    for (sum, delta) in sum.iter_mut().zip(delta) {
        *sum += delta;
    }
}

fn f64_at(value: &Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer)?.as_f64()
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer)?.as_str()
}

fn compare_f64((left, right): (f64, f64)) -> Option<std::cmp::Ordering> {
    left.partial_cmp(&right)
}

fn mean(sum: f64, count: u64) -> Option<f64> {
    (count > 0).then_some(sum / count as f64)
}

fn mean_rgb_delta(sum: [f64; 3], count: u64) -> Option<[f64; 3]> {
    (count > 0).then_some(sum.map(|value| value / count as f64))
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

fn shading_model_counts(models: BTreeMap<String, u64>) -> Vec<ShadingModelCount> {
    let mut values = models
        .into_iter()
        .map(|(model, count)| ShadingModelCount { model, count })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.model.cmp(&right.model))
    });
    values
}

fn material_buckets(materials: BTreeMap<String, MaterialAccumulator>) -> Vec<MaterialBucket> {
    let mut values = materials
        .into_iter()
        .map(|(material_name, accumulator)| accumulator.finish(material_name))
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| right.expected_cpu_closer.cmp(&left.expected_cpu_closer))
            .then_with(|| left.material_name.cmp(&right.material_name))
    });
    values
}

fn shading_model_buckets(
    models: BTreeMap<String, MaterialAccumulator>,
) -> Vec<ShadingModelBucket> {
    let mut values = models
        .into_iter()
        .map(|(model, accumulator)| ShadingModelBucket {
            stats: accumulator.finish(model.clone()),
            model,
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .stats
            .count
            .cmp(&left.stats.count)
            .then_with(|| left.model.cmp(&right.model))
    });
    values
}

fn material_draw_buckets(
    materials: BTreeMap<(String, String), MaterialAccumulator>,
) -> Vec<SelectionMaterialDrawBucket> {
    let mut values = materials
        .into_iter()
        .map(|((material_name, draw_key), accumulator)| SelectionMaterialDrawBucket {
            stats: accumulator.finish(material_name.clone()),
            material_name,
            draw_key,
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .stats
            .count
            .cmp(&left.stats.count)
            .then_with(|| {
                right
                    .stats
                    .manifest_sample_expected_closer
                    .cmp(&left.stats.manifest_sample_expected_closer)
            })
            .then_with(|| left.material_name.cmp(&right.material_name))
            .then_with(|| left.draw_key.cmp(&right.draw_key))
    });
    values
}

fn selection_source_buckets(sources: BTreeMap<String, Accumulator>) -> Vec<SelectionSourceBucket> {
    let mut values = sources
        .into_iter()
        .map(|(selection_source, accumulator)| SelectionSourceBucket {
            selection_source,
            stats: accumulator.finish(),
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .stats
            .count
            .cmp(&left.stats.count)
            .then_with(|| {
                right
                    .stats
                    .manifest_sample_expected_closer
                    .cmp(&left.stats.manifest_sample_expected_closer)
            })
            .then_with(|| left.selection_source.cmp(&right.selection_source))
    });
    values
}

fn recommended_probes(
    buckets: &[SelectionMaterialDrawBucket],
    top: usize,
) -> Vec<ProbeRecommendation> {
    let mut probes = buckets
        .iter()
        .filter(|bucket| bucket.stats.count > 0)
        .map(|bucket| {
            let classification = probe_classification(&bucket.stats);
            ProbeRecommendation {
                material_name: bucket.material_name.clone(),
                draw_key: bucket.draw_key.clone(),
                count: bucket.stats.count,
                classification,
                action: probe_action(classification),
                mean_expected_actual_rgb_distance: bucket.stats.mean_expected_actual_rgb_distance,
                mean_manifest_actual_rgb_distance: bucket
                    .stats
                    .mean_manifest_sample_actual_rgb_distance,
                mean_manifest_expected_rgb_distance: bucket
                    .stats
                    .mean_manifest_sample_expected_rgb_distance,
                mean_actual_minus_manifest_rgb_delta: bucket
                    .stats
                    .mean_actual_minus_manifest_sample_rgb_delta,
                mean_expected_minus_manifest_rgb_delta: bucket
                    .stats
                    .mean_expected_minus_manifest_sample_rgb_delta,
                least_squares_actual_over_manifest_rgb_gain: bucket
                    .stats
                    .least_squares_actual_over_manifest_sample_rgb_gain,
                least_squares_expected_over_manifest_rgb_gain: bucket
                    .stats
                    .least_squares_expected_over_manifest_sample_rgb_gain,
                least_squares_expected_minus_actual_over_manifest_rgb_gain: bucket
                    .stats
                    .least_squares_expected_minus_actual_over_manifest_sample_rgb_gain,
                manifest_sample_actual_within_1_5: bucket
                    .stats
                    .manifest_sample_actual_within_1_5,
                manifest_sample_expected_within_1_5: bucket
                    .stats
                    .manifest_sample_expected_within_1_5,
                manifest_sample_actual_near_expected_far: bucket
                    .stats
                    .manifest_sample_actual_near_expected_far,
                manifest_sample_actual_far_expected_near: bucket
                    .stats
                    .manifest_sample_actual_far_expected_near,
                manifest_sample_both_far: bucket.stats.manifest_sample_both_far,
            }
        })
        .collect::<Vec<_>>();
    probes.sort_by(|left, right| {
        probe_priority(right)
            .cmp(&probe_priority(left))
            .then_with(|| right.count.cmp(&left.count))
            .then_with(|| {
                expected_manifest_gap(right).total_cmp(&expected_manifest_gap(left))
            })
            .then_with(|| left.material_name.cmp(&right.material_name))
            .then_with(|| left.draw_key.cmp(&right.draw_key))
    });
    probes.truncate(top);
    probes
}

fn probe_classification(stats: &MaterialBucket) -> &'static str {
    let majority = stats.count / 2 + 1;
    if stats.manifest_sample_actual_near_expected_far >= majority {
        "renderer_matches_selected_sample_expected_far"
    } else if stats.manifest_sample_both_far >= majority {
        "selected_sample_and_renderer_both_far"
    } else if stats.manifest_sample_expected_within_1_5 >= majority
        || stats.manifest_sample_expected_closer > stats.manifest_sample_actual_closer
    {
        "selected_sample_matches_three_vrm"
    } else if stats.manifest_sample_actual_within_1_5 >= majority
        || stats.manifest_sample_actual_closer > stats.manifest_sample_expected_closer
    {
        "renderer_closer_to_selected_sample"
    } else {
        "mixed"
    }
}

fn probe_action(classification: &str) -> &'static str {
    match classification {
        "renderer_matches_selected_sample_expected_far" => {
            "audit material/color evaluation at this selected surface"
        }
        "selected_sample_and_renderer_both_far" => {
            "audit resolve draw binding or selected-surface material inputs"
        }
        "selected_sample_matches_three_vrm" => {
            "audit whether the backend adopts this selected surface before shading"
        }
        "renderer_closer_to_selected_sample" => {
            "compare backend output formula against three-vrm for this material"
        }
        _ => "split rows by pixel and inspect owner/fill plus color inputs",
    }
}

fn probe_priority(probe: &ProbeRecommendation) -> u8 {
    match probe.classification {
        "renderer_matches_selected_sample_expected_far" => 5,
        "selected_sample_and_renderer_both_far" => 4,
        "renderer_closer_to_selected_sample" => 3,
        "selected_sample_matches_three_vrm" => 2,
        _ => 1,
    }
}

fn expected_manifest_gap(probe: &ProbeRecommendation) -> f64 {
    probe
        .mean_manifest_expected_rgb_distance
        .unwrap_or(f64::NEG_INFINITY)
        - probe
            .mean_manifest_actual_rgb_distance
            .unwrap_or(f64::NEG_INFINITY)
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
    push_probe_recommendations_markdown(&mut output, &report.recommended_probes);
    push_selection_source_buckets_markdown(&mut output, &report.selection_source_buckets);
    push_bucket_markdown(&mut output, "All", &report.all);
    push_bucket_markdown(&mut output, "Selected", &report.selected);
    push_bucket_markdown(&mut output, "Missing Selection", &report.missing_selection);
    push_bucket_markdown(&mut output, "Carried Selection", &report.carried_selection);
    push_bucket_markdown(&mut output, "New Selection", &report.new_selection);
    push_residual_rows_markdown(&mut output, "Top Residuals", &report.top_residuals);
    if !report.top_residuals_by_shading_model.is_empty() {
        output.push_str("## Top Residuals By Shading Model\n\n");
        for group in &report.top_residuals_by_shading_model {
            push_residual_rows_markdown(
                &mut output,
                &format!("Top Residuals: {}", group.key),
                &group.rows,
            );
        }
    }
    output
}

fn push_selection_source_buckets_markdown(
    output: &mut String,
    buckets: &[SelectionSourceBucket],
) {
    if buckets.is_empty() {
        return;
    }
    output.push_str("## Selection Source Buckets\n\n");
    output.push_str("| Source | Count | Mean E-A | Manifest A/E/T | Manifest <=1.5 A/E | Manifest near/far A/E/both | Mean Manifest A/E | Mean A-M / E-M | Materials | Draw keys |\n");
    output.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |\n");
    for bucket in buckets {
        let stats = &bucket.stats;
        output.push_str(&format!(
            "| {} | {} | {} | {}/{}/{} | {} / {} | {}/{}/{} | {} / {} | {} / {} | {} | {} |\n",
            bucket.selection_source,
            stats.count,
            fmt_opt(stats.mean_expected_actual_rgb_distance),
            stats.manifest_sample_actual_closer,
            stats.manifest_sample_expected_closer,
            stats.manifest_sample_tied,
            stats.manifest_sample_actual_within_1_5,
            stats.manifest_sample_expected_within_1_5,
            stats.manifest_sample_actual_near_expected_far,
            stats.manifest_sample_actual_far_expected_near,
            stats.manifest_sample_both_far,
            fmt_opt(stats.mean_manifest_sample_actual_rgb_distance),
            fmt_opt(stats.mean_manifest_sample_expected_rgb_distance),
            fmt_opt_rgb_delta(stats.mean_actual_minus_manifest_sample_rgb_delta),
            fmt_opt_rgb_delta(stats.mean_expected_minus_manifest_sample_rgb_delta),
            fmt_materials(&stats.material_counts),
            fmt_material_draws(&stats.selection_material_draw_buckets),
        ));
    }
    output.push('\n');
}

fn push_probe_recommendations_markdown(output: &mut String, probes: &[ProbeRecommendation]) {
    if probes.is_empty() {
        return;
    }
    output.push_str("## Recommended Material Probes\n\n");
    output.push_str("| Material | Draw key | Count | Class | Action | Mean E-A | Mean manifest A/E | Mean A-M / E-M | LS gain A/M / E/M | Missing LS gain (E-A)/M | <=1.5 A/E | near/far A/E/both |\n");
    output.push_str("| --- | --- | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for probe in probes.iter().take(12) {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} / {} | {} / {} | {} / {} | {} | {}/{} | {}/{}/{} |\n",
            probe.material_name,
            probe.draw_key,
            probe.count,
            probe.classification,
            probe.action,
            fmt_opt(probe.mean_expected_actual_rgb_distance),
            fmt_opt(probe.mean_manifest_actual_rgb_distance),
            fmt_opt(probe.mean_manifest_expected_rgb_distance),
            fmt_opt_rgb_delta(probe.mean_actual_minus_manifest_rgb_delta),
            fmt_opt_rgb_delta(probe.mean_expected_minus_manifest_rgb_delta),
            fmt_opt_rgb_delta(probe.least_squares_actual_over_manifest_rgb_gain),
            fmt_opt_rgb_delta(probe.least_squares_expected_over_manifest_rgb_gain),
            fmt_opt_rgb_delta(probe.least_squares_expected_minus_actual_over_manifest_rgb_gain),
            probe.manifest_sample_actual_within_1_5,
            probe.manifest_sample_expected_within_1_5,
            probe.manifest_sample_actual_near_expected_far,
            probe.manifest_sample_actual_far_expected_near,
            probe.manifest_sample_both_far,
        ));
    }
    output.push('\n');
}

fn push_residual_rows_markdown(output: &mut String, title: &str, rows: &[ResidualRow]) {
    output.push_str(&format!("## {title}\n\n"));
    output.push_str("| Pixel | Sel | Model | Actual | Expected | E-A dist | E-A delta | Selected surface | Selected draw | Selected RGBA | Sel sample A/E | A-S / E-S | Frontmost | Front CPU A/E | NearestExp CPU A/E | NearestExp RGBA | Best Sampling A/E | Edge | Gradient |\n");
    output.push_str("| --- | --- | --- | --- | --- | ---: | ---: | --- | --- | --- | ---: | ---: | --- | ---: | ---: | --- | --- | ---: | ---: |\n");
    for row in rows {
        output.push_str(&format!(
            "| {},{} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} / {} | {} / {} | {} | {} / {} | {} / {} | {} | {} / {} | {} | {} |\n",
            row.x,
            row.y,
            if row.selected { "yes" } else { "no" },
            row.frontmost_shading_model.as_deref().unwrap_or("n/a"),
            fmt_rgba(row.actual),
            fmt_rgba(row.expected),
            fmt_opt(Some(row.expected_actual_rgb_distance)),
            fmt_opt_rgb_delta(Some(row.expected_minus_actual_rgb_delta)),
            fmt_surface(row.selection_surface.as_ref()),
            row.selection_draw_key.as_deref().unwrap_or("n/a"),
            fmt_opt_rgba(row.selection_rgba),
            fmt_opt(row.selection_actual_rgb_distance),
            fmt_opt(row.selection_expected_rgb_distance),
            fmt_opt_rgb_delta(row.actual_minus_selection_rgb_delta),
            fmt_opt_rgb_delta(row.expected_minus_selection_rgb_delta),
            fmt_surface(row.frontmost.as_ref()),
            fmt_opt(row.cpu_actual_rgb_distance),
            fmt_opt(row.cpu_expected_rgb_distance),
            fmt_opt(row.nearest_expected_cpu_actual_rgb_distance),
            fmt_opt(row.nearest_expected_cpu_expected_rgb_distance),
            fmt_opt_rgba(row.nearest_expected_cpu_base_color_rgba),
            row.best_actual_sampling_mode.as_deref().unwrap_or("n/a"),
            row.best_expected_sampling_mode.as_deref().unwrap_or("n/a"),
            fmt_opt(row.edge_distance_pixels),
            fmt_opt(row.base_texture_local_rgb_gradient),
        ));
    }
    output.push('\n');
}

fn push_bucket_markdown(output: &mut String, title: &str, bucket: &BucketStats) {
    output.push_str(&format!("## {title}\n\n"));
    output.push_str(&format!("- Count: `{}`\n", bucket.count));
    output.push_str(&format!(
        "- Expected-vs-actual RGB distance mean `{}`; within4/8/16 `{}` / `{}` / `{}`; mean E-A `{}`\n",
        fmt_opt(bucket.mean_expected_actual_rgb_distance),
        bucket.expected_actual_within_4,
        bucket.expected_actual_within_8,
        bucket.expected_actual_within_16,
        fmt_opt_rgb_delta(bucket.mean_expected_minus_actual_rgb_delta)
    ));
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
        "- Nearest expected CPU color closer actual/expected/tie: `{}` / `{}` / `{}`; mean A/E `{}` / `{}`; beats frontmost expected `{}`\n",
        bucket.nearest_expected_cpu_actual_closer,
        bucket.nearest_expected_cpu_expected_closer,
        bucket.nearest_expected_cpu_tied,
        fmt_opt(bucket.mean_nearest_expected_cpu_actual_rgb_distance),
        fmt_opt(bucket.mean_nearest_expected_cpu_expected_rgb_distance),
        bucket.nearest_expected_beats_frontmost_for_expected
    ));
    output.push_str(&format!(
        "- Frontmost base texture closer actual/expected/tie: `{}` / `{}` / `{}`; mean A/E `{}` / `{}`; beats CPU expected `{}`\n",
        bucket.frontmost_base_texture_actual_closer,
        bucket.frontmost_base_texture_expected_closer,
        bucket.frontmost_base_texture_tied,
        fmt_opt(bucket.mean_frontmost_base_texture_actual_rgb_distance),
        fmt_opt(bucket.mean_frontmost_base_texture_expected_rgb_distance),
        bucket.frontmost_base_texture_beats_cpu_for_expected
    ));
    output.push_str(&format!(
        "- Mean RGB delta actual-texture / expected-texture: `{}` / `{}`\n",
        fmt_opt_rgb_delta(bucket.mean_actual_minus_texture_rgb_delta),
        fmt_opt_rgb_delta(bucket.mean_expected_minus_texture_rgb_delta)
    ));
    output.push_str(&format!(
        "- Texture-as-linear-sRGB closer actual/expected/tie: `{}` / `{}` / `{}`; mean A/E `{}` / `{}`; mean A-L / E-L `{}` / `{}`\n",
        bucket.frontmost_texture_as_linear_srgb_actual_closer,
        bucket.frontmost_texture_as_linear_srgb_expected_closer,
        bucket.frontmost_texture_as_linear_srgb_tied,
        fmt_opt(bucket.mean_frontmost_texture_as_linear_srgb_actual_rgb_distance),
        fmt_opt(bucket.mean_frontmost_texture_as_linear_srgb_expected_rgb_distance),
        fmt_opt_rgb_delta(bucket.mean_actual_minus_texture_as_linear_srgb_rgb_delta),
        fmt_opt_rgb_delta(bucket.mean_expected_minus_texture_as_linear_srgb_rgb_delta)
    ));
    output.push_str(&format!(
        "- Manifest sample closer actual/expected/tie: `{}` / `{}` / `{}`; within1.5 A/E `{}` / `{}`; within8 A/E `{}` / `{}`; near/far A/E `{}` / `{}`; both far `{}`; mean A/E `{}` / `{}`; mean A-M / E-M `{}` / `{}`\n",
        bucket.manifest_sample_actual_closer,
        bucket.manifest_sample_expected_closer,
        bucket.manifest_sample_tied,
        bucket.manifest_sample_actual_within_1_5,
        bucket.manifest_sample_expected_within_1_5,
        bucket.manifest_sample_actual_within_8,
        bucket.manifest_sample_expected_within_8,
        bucket.manifest_sample_actual_near_expected_far,
        bucket.manifest_sample_actual_far_expected_near,
        bucket.manifest_sample_both_far,
        fmt_opt(bucket.mean_manifest_sample_actual_rgb_distance),
        fmt_opt(bucket.mean_manifest_sample_expected_rgb_distance),
        fmt_opt_rgb_delta(bucket.mean_actual_minus_manifest_sample_rgb_delta),
        fmt_opt_rgb_delta(bucket.mean_expected_minus_manifest_sample_rgb_delta)
    ));
    output.push_str(&format!(
        "- Best texture sampling mean A/E: `{}` / `{}`; within4 A/E `{}` / `{}`; within8 A/E `{}` / `{}`; within16 A/E `{}` / `{}`\n",
        fmt_opt(bucket.mean_best_sampling_actual_rgb_distance),
        fmt_opt(bucket.mean_best_sampling_expected_rgb_distance),
        bucket.best_sampling_actual_within_4,
        bucket.best_sampling_expected_within_4,
        bucket.best_sampling_actual_within_8,
        bucket.best_sampling_expected_within_8,
        bucket.best_sampling_actual_within_16,
        bucket.best_sampling_expected_within_16
    ));
    output.push_str(&format!(
        "- Top frontmost materials: `{}`\n\n",
        fmt_materials(&bucket.material_counts)
    ));
    output.push_str(&format!(
        "- Frontmost shading models: `{}`\n\n",
        fmt_shading_models(&bucket.shading_model_counts)
    ));
    push_shading_model_bucket_markdown(
        output,
        "Frontmost shading-model buckets",
        &bucket.shading_model_buckets,
    );
    push_material_bucket_markdown(output, "Frontmost material buckets", &bucket.material_buckets);
    if !bucket.selection_material_buckets.is_empty() {
        push_material_bucket_markdown(
            output,
            "Manifest-selected material buckets",
            &bucket.selection_material_buckets,
        );
    }
    if !bucket.selection_material_draw_buckets.is_empty() {
        push_material_draw_bucket_markdown(
            output,
            "Manifest-selected material+draw buckets",
            &bucket.selection_material_draw_buckets,
        );
    }
}

fn push_shading_model_bucket_markdown(
    output: &mut String,
    title: &str,
    models: &[ShadingModelBucket],
) {
    output.push_str(&format!("### {title}\n\n"));
    output.push_str("| Model | Count | Mean E-A | E-A <=4/8/16 | Mean E-A delta | CPU A/E/T | Mean CPU A/E | Texture A/E/T | Mean Texture A/E | Manifest A/E/T | Manifest <=1.5 A/E | Mean Manifest A/E | Best sample <=8 A/E | Edge <=0.50px | Same expected mat/tri | Best modes A/E |\n");
    output.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");
    for model in models.iter().take(8) {
        let stats = &model.stats;
        output.push_str(&format!(
            "| {} | {} | {} | {}/{}/{} | {} | {}/{}/{} | {} / {} | {}/{}/{} | {} / {} | {}/{}/{} | {} / {} | {} / {} | {} / {} | {} | {}/{} | {} / {} |\n",
            model.model,
            stats.count,
            fmt_opt(stats.mean_expected_actual_rgb_distance),
            stats.expected_actual_within_4,
            stats.expected_actual_within_8,
            stats.expected_actual_within_16,
            fmt_opt_rgb_delta(stats.mean_expected_minus_actual_rgb_delta),
            stats.actual_cpu_closer,
            stats.expected_cpu_closer,
            stats.cpu_tied,
            fmt_opt(stats.mean_cpu_actual_rgb_distance),
            fmt_opt(stats.mean_cpu_expected_rgb_distance),
            stats.frontmost_base_texture_actual_closer,
            stats.frontmost_base_texture_expected_closer,
            stats.frontmost_base_texture_tied,
            fmt_opt(stats.mean_frontmost_base_texture_actual_rgb_distance),
            fmt_opt(stats.mean_frontmost_base_texture_expected_rgb_distance),
            stats.manifest_sample_actual_closer,
            stats.manifest_sample_expected_closer,
            stats.manifest_sample_tied,
            stats.manifest_sample_actual_within_1_5,
            stats.manifest_sample_expected_within_1_5,
            fmt_opt(stats.mean_manifest_sample_actual_rgb_distance),
            fmt_opt(stats.mean_manifest_sample_expected_rgb_distance),
            stats.best_sampling_actual_within_8,
            stats.best_sampling_expected_within_8,
            stats.edge_distance_lte_050px,
            stats.same_material_as_expected,
            stats.same_triangle_as_expected,
            fmt_modes(&stats.best_sampling_modes_for_actual),
            fmt_modes(&stats.best_sampling_modes_for_expected),
        ));
    }
    output.push('\n');
}

fn push_material_bucket_markdown(output: &mut String, title: &str, materials: &[MaterialBucket]) {
    output.push_str(&format!("### {title}\n\n"));
    output.push_str("| Material | Count | Mean E-A | E-A <=4/8/16 | Mean E-A delta | Models | CPU A/E/T | Mean CPU A/E | NExp CPU A/E/T | Mean NExp A/E | NExp beats front | Texture A/E/T | Mean Texture A/E | Mean A-T / E-T | Texture-as-linear A/E/T | Mean Linear A/E | Mean A-L / E-L | Manifest A/E/T | Manifest <=1.5 A/E | Manifest near/far A/E/both | Mean Manifest A/E | Mean A-M / E-M | Texture beats CPU | Best sample mean A/E | Best sample <=8 A/E | Edge <=0.50px | Same expected mat/tri | Best modes A/E |\n");
    output.push_str("| --- | ---: | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");
    for material in materials.iter().take(8) {
        output.push_str(&format!(
            "| {} | {} | {} | {}/{}/{} | {} | {} | {}/{}/{} | {} / {} | {}/{}/{} | {} / {} | {} | {}/{}/{} | {} / {} | {} / {} | {}/{}/{} | {} / {} | {} / {} | {}/{}/{} | {} / {} | {}/{}/{} | {} / {} | {} / {} | {} | {} / {} | {} / {} | {} | {}/{} | {} / {} |\n",
            material.material_name,
            material.count,
            fmt_opt(material.mean_expected_actual_rgb_distance),
            material.expected_actual_within_4,
            material.expected_actual_within_8,
            material.expected_actual_within_16,
            fmt_opt_rgb_delta(material.mean_expected_minus_actual_rgb_delta),
            fmt_shading_models(&material.shading_model_counts),
            material.actual_cpu_closer,
            material.expected_cpu_closer,
            material.cpu_tied,
            fmt_opt(material.mean_cpu_actual_rgb_distance),
            fmt_opt(material.mean_cpu_expected_rgb_distance),
            material.nearest_expected_cpu_actual_closer,
            material.nearest_expected_cpu_expected_closer,
            material.nearest_expected_cpu_tied,
            fmt_opt(material.mean_nearest_expected_cpu_actual_rgb_distance),
            fmt_opt(material.mean_nearest_expected_cpu_expected_rgb_distance),
            material.nearest_expected_beats_frontmost_for_expected,
            material.frontmost_base_texture_actual_closer,
            material.frontmost_base_texture_expected_closer,
            material.frontmost_base_texture_tied,
            fmt_opt(material.mean_frontmost_base_texture_actual_rgb_distance),
            fmt_opt(material.mean_frontmost_base_texture_expected_rgb_distance),
            fmt_opt_rgb_delta(material.mean_actual_minus_texture_rgb_delta),
            fmt_opt_rgb_delta(material.mean_expected_minus_texture_rgb_delta),
            material.frontmost_texture_as_linear_srgb_actual_closer,
            material.frontmost_texture_as_linear_srgb_expected_closer,
            material.frontmost_texture_as_linear_srgb_tied,
            fmt_opt(material.mean_frontmost_texture_as_linear_srgb_actual_rgb_distance),
            fmt_opt(material.mean_frontmost_texture_as_linear_srgb_expected_rgb_distance),
            fmt_opt_rgb_delta(material.mean_actual_minus_texture_as_linear_srgb_rgb_delta),
            fmt_opt_rgb_delta(material.mean_expected_minus_texture_as_linear_srgb_rgb_delta),
            material.manifest_sample_actual_closer,
            material.manifest_sample_expected_closer,
            material.manifest_sample_tied,
            material.manifest_sample_actual_within_1_5,
            material.manifest_sample_expected_within_1_5,
            material.manifest_sample_actual_near_expected_far,
            material.manifest_sample_actual_far_expected_near,
            material.manifest_sample_both_far,
            fmt_opt(material.mean_manifest_sample_actual_rgb_distance),
            fmt_opt(material.mean_manifest_sample_expected_rgb_distance),
            fmt_opt_rgb_delta(material.mean_actual_minus_manifest_sample_rgb_delta),
            fmt_opt_rgb_delta(material.mean_expected_minus_manifest_sample_rgb_delta),
            material.frontmost_base_texture_beats_cpu_for_expected,
            fmt_opt(material.mean_best_sampling_actual_rgb_distance),
            fmt_opt(material.mean_best_sampling_expected_rgb_distance),
            material.best_sampling_actual_within_8,
            material.best_sampling_expected_within_8,
            material.edge_distance_lte_050px,
            material.same_material_as_expected,
            material.same_triangle_as_expected,
            fmt_modes(&material.best_sampling_modes_for_actual),
            fmt_modes(&material.best_sampling_modes_for_expected),
        ));
    }
    output.push('\n');
}

fn push_material_draw_bucket_markdown(
    output: &mut String,
    title: &str,
    materials: &[SelectionMaterialDrawBucket],
) {
    output.push_str(&format!("### {title}\n\n"));
    output.push_str("| Material | Draw key | Count | Mean E-A | E-A <=4/8/16 | Mean E-A delta | Models | Manifest A/E/T | Manifest <=1.5 A/E | Manifest near/far A/E/both | Mean Manifest A/E | Mean A-M / E-M | LS gain A/M / E/M | Missing LS gain (E-A)/M | CPU A/E/T | Texture A/E/T | Edge <=0.50px | Best sample <=8 A/E | Best modes A/E |\n");
    output.push_str("| --- | --- | ---: | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");
    for material in materials.iter().take(12) {
        let stats = &material.stats;
        output.push_str(&format!(
            "| {} | {} | {} | {} | {}/{}/{} | {} | {} | {}/{}/{} | {} / {} | {}/{}/{} | {} / {} | {} / {} | {} / {} | {} | {}/{}/{} | {}/{}/{} | {} | {} / {} | {} / {} |\n",
            material.material_name,
            material.draw_key,
            stats.count,
            fmt_opt(stats.mean_expected_actual_rgb_distance),
            stats.expected_actual_within_4,
            stats.expected_actual_within_8,
            stats.expected_actual_within_16,
            fmt_opt_rgb_delta(stats.mean_expected_minus_actual_rgb_delta),
            fmt_shading_models(&stats.shading_model_counts),
            stats.manifest_sample_actual_closer,
            stats.manifest_sample_expected_closer,
            stats.manifest_sample_tied,
            stats.manifest_sample_actual_within_1_5,
            stats.manifest_sample_expected_within_1_5,
            stats.manifest_sample_actual_near_expected_far,
            stats.manifest_sample_actual_far_expected_near,
            stats.manifest_sample_both_far,
            fmt_opt(stats.mean_manifest_sample_actual_rgb_distance),
            fmt_opt(stats.mean_manifest_sample_expected_rgb_distance),
            fmt_opt_rgb_delta(stats.mean_actual_minus_manifest_sample_rgb_delta),
            fmt_opt_rgb_delta(stats.mean_expected_minus_manifest_sample_rgb_delta),
            fmt_opt_rgb_delta(stats.least_squares_actual_over_manifest_sample_rgb_gain),
            fmt_opt_rgb_delta(stats.least_squares_expected_over_manifest_sample_rgb_gain),
            fmt_opt_rgb_delta(
                stats.least_squares_expected_minus_actual_over_manifest_sample_rgb_gain
            ),
            stats.actual_cpu_closer,
            stats.expected_cpu_closer,
            stats.cpu_tied,
            stats.frontmost_base_texture_actual_closer,
            stats.frontmost_base_texture_expected_closer,
            stats.frontmost_base_texture_tied,
            stats.edge_distance_lte_050px,
            stats.best_sampling_actual_within_8,
            stats.best_sampling_expected_within_8,
            fmt_modes(&stats.best_sampling_modes_for_actual),
            fmt_modes(&stats.best_sampling_modes_for_expected),
        ));
    }
    output.push('\n');
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

fn fmt_material_draws(draws: &[SelectionMaterialDrawBucket]) -> String {
    draws
        .iter()
        .take(4)
        .map(|draw| format!("{} {}:{}", draw.material_name, draw.draw_key, draw.stats.count))
        .collect::<Vec<_>>()
        .join(", ")
}

fn fmt_shading_models(models: &[ShadingModelCount]) -> String {
    models
        .iter()
        .take(4)
        .map(|model| format!("{}:{}", model.model, model.count))
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

fn fmt_opt_rgba(rgba: Option<[u8; 4]>) -> String {
    rgba.map(fmt_rgba).unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_opt_rgb_delta(delta: Option<[f64; 3]>) -> String {
    delta
        .map(|delta| format!("{:.2},{:.2},{:.2}", delta[0], delta[1], delta[2]))
        .unwrap_or_else(|| "n/a".to_owned())
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

fn assert_close(actual: Option<f64>, expected: f64) {
    let Some(actual) = actual else {
        panic!("expected Some({expected}), got None");
    };
    assert!(
        (actual - expected).abs() < 0.0001,
        "expected {expected}, got {actual}"
    );
}

fn assert_close_vec3(actual: Option<[f64; 3]>, expected: [f64; 3]) {
    let Some(actual) = actual else {
        panic!("expected Some({expected:?}), got None");
    };
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {expected}, got {actual}"
        );
    }
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
                "frontmost_base_texture_rgba": [12, 10, 10, 255],
                "frontmost_cpu_base_color_actual_rgb_distance": 154.0,
                "frontmost_cpu_base_color_expected_rgb_distance": 2.0,
                "frontmost_visible": {
                    "material_name": "body",
                    "triangle": 7,
                    "material_shading": {"model": "mtoon"},
                    "edge_distance_pixels": 0.2,
                    "base_texture_local_rgb_gradient": 3.0
                },
                "nearest_visible_actual": {"material_name": "body", "triangle": 8},
                "nearest_visible_expected": {
                    "material_name": "body",
                    "triangle": 7,
                    "cpu_base_color_rgba": [10, 10, 10, 255]
                },
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
                    "material_shading": {"model": "gltf_pbr"},
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
            {
                "x": 1,
                "y": 2,
                "rgba": [12, 10, 10, 255],
                "surface": {"materialName": "selected_body", "triangle": 70},
                "selection_source": "webgl-coverage",
                "sample_geometry": {
                    "node": 145,
                    "mesh": 4,
                    "primitive": 1,
                    "pass": "base"
                }
            }
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
    assert_eq!(report.top_residuals_by_shading_model.len(), 2);
    assert_eq!(report.top_residuals_by_shading_model[0].key, "gltf_pbr");
    assert_eq!(report.top_residuals_by_shading_model[0].rows.len(), 1);
    assert_eq!(report.top_residuals_by_shading_model[1].key, "mtoon");
    assert_eq!(report.top_residuals_by_shading_model[1].rows.len(), 1);
    assert_eq!(report.all.material_buckets.len(), 2);
    assert_eq!(report.all.shading_model_counts[0].model, "gltf_pbr");
    assert_eq!(report.all.shading_model_counts[0].count, 1);
    assert_eq!(report.all.shading_model_counts[1].model, "mtoon");
    assert_eq!(report.all.shading_model_counts[1].count, 1);
    assert_eq!(report.all.shading_model_buckets.len(), 2);
    assert_eq!(report.all.shading_model_buckets[0].model, "gltf_pbr");
    assert_eq!(report.all.shading_model_buckets[0].stats.count, 1);
    assert_eq!(
        report.all.shading_model_buckets[0].stats.actual_cpu_closer,
        1
    );
    assert_eq!(report.all.shading_model_buckets[1].model, "mtoon");
    assert_eq!(report.all.shading_model_buckets[1].stats.count, 1);
    assert_eq!(
        report.all.shading_model_buckets[1].stats.expected_cpu_closer,
        1
    );
    assert_eq!(report.all.material_buckets[0].material_name, "body");
    assert_eq!(report.all.material_buckets[0].expected_cpu_closer, 1);
    assert_eq!(report.all.material_buckets[0].shading_model_counts[0].model, "mtoon");
    assert_eq!(report.selected.selection_material_buckets.len(), 1);
    assert_eq!(
        report.selected.selection_material_buckets[0].material_name,
        "selected_body"
    );
    assert_eq!(report.selected.selection_material_buckets[0].count, 1);
    assert_eq!(report.selected.selection_material_draw_buckets.len(), 1);
    assert_eq!(
        report.selected.selection_material_draw_buckets[0].material_name,
        "selected_body"
    );
    assert_eq!(
        report.selected.selection_material_draw_buckets[0].draw_key,
        "node145/mesh4/prim1/base"
    );
    assert_eq!(
        report.selected.selection_material_draw_buckets[0].stats.count,
        1
    );
    assert_eq!(report.selection_source_buckets.len(), 1);
    assert_eq!(
        report.selection_source_buckets[0].selection_source,
        "webgl-coverage"
    );
    assert_eq!(report.selection_source_buckets[0].stats.count, 1);
    assert_eq!(
        report.selection_source_buckets[0]
            .stats
            .selection_material_draw_buckets[0]
            .draw_key,
        "node145/mesh4/prim1/base"
    );
    assert_eq!(report.recommended_probes.len(), 1);
    assert_eq!(report.recommended_probes[0].material_name, "selected_body");
    assert_eq!(
        report.recommended_probes[0].classification,
        "selected_sample_matches_three_vrm"
    );
    assert_eq!(report.selected.nearest_expected_cpu_expected_closer, 1);
    assert_eq!(report.selected.nearest_expected_beats_frontmost_for_expected, 1);
    assert_eq!(report.selected.frontmost_base_texture_actual_closer, 0);
    assert_eq!(report.selected.frontmost_base_texture_expected_closer, 0);
    assert_eq!(report.selected.manifest_sample_expected_closer, 1);
    assert_eq!(report.selected.manifest_sample_actual_within_1_5, 0);
    assert_eq!(report.selected.manifest_sample_expected_within_1_5, 0);
    assert_eq!(report.selected.manifest_sample_actual_within_8, 0);
    assert_eq!(report.selected.manifest_sample_expected_within_8, 1);
    assert_eq!(report.selected.manifest_sample_actual_near_expected_far, 0);
    assert_eq!(report.selected.manifest_sample_actual_far_expected_near, 0);
    assert_eq!(report.selected.manifest_sample_both_far, 0);
    assert_close(
        report.selected.mean_expected_actual_rgb_distance,
        rgb_distance([100, 100, 100, 255], [10, 10, 10, 255]),
    );
    assert_eq!(report.selected.expected_actual_within_4, 0);
    assert_eq!(report.selected.expected_actual_within_8, 0);
    assert_eq!(report.selected.expected_actual_within_16, 0);
    assert_eq!(
        report.selected.mean_expected_minus_actual_rgb_delta,
        Some([-90.0, -90.0, -90.0])
    );
    assert_close(
        report.selected.mean_manifest_sample_actual_rgb_distance,
        rgb_distance([12, 10, 10, 255], [100, 100, 100, 255]),
    );
    assert_close(
        report.selected.mean_manifest_sample_expected_rgb_distance,
        rgb_distance([12, 10, 10, 255], [10, 10, 10, 255]),
    );
    assert_eq!(
        report.selected.mean_actual_minus_texture_rgb_delta,
        Some([88.0, 90.0, 90.0])
    );
    assert_eq!(
        report.selected.mean_expected_minus_texture_rgb_delta,
        Some([-2.0, 0.0, 0.0])
    );
    assert_close_vec3(
        report.selected.mean_actual_over_manifest_sample_rgb_ratio,
        [100.0 / 12.0, 10.0, 10.0],
    );
    assert_close_vec3(
        report
            .selected
            .least_squares_actual_over_manifest_sample_rgb_gain,
        [100.0 / 12.0, 10.0, 10.0],
    );
    assert_close_vec3(
        report.selected.mean_expected_over_manifest_sample_rgb_ratio,
        [10.0 / 12.0, 1.0, 1.0],
    );
    assert_close_vec3(
        report
            .selected
            .least_squares_expected_over_manifest_sample_rgb_gain,
        [10.0 / 12.0, 1.0, 1.0],
    );
    assert_close_vec3(
        report
            .selected
            .mean_expected_minus_actual_over_manifest_sample_rgb_ratio,
        [-90.0 / 12.0, -9.0, -9.0],
    );
    assert_close_vec3(
        report
            .selected
            .least_squares_expected_minus_actual_over_manifest_sample_rgb_gain,
        [-90.0 / 12.0, -9.0, -9.0],
    );
    assert_eq!(
        report.selected.selection_material_buckets[0].mean_actual_minus_texture_rgb_delta,
        Some([88.0, 90.0, 90.0])
    );
    assert_eq!(report.selected.best_sampling_actual_within_4, 1);
    assert_eq!(report.selected.best_sampling_expected_within_4, 1);
    assert_eq!(
        report.top_residuals[0].selection_draw_key.as_deref(),
        Some("node145/mesh4/prim1/base")
    );
    assert_eq!(report.top_residuals[0].selection_rgba, Some([12, 10, 10, 255]));
    assert_close(
        Some(report.top_residuals[0].expected_actual_rgb_distance),
        rgb_distance([100, 100, 100, 255], [10, 10, 10, 255]),
    );
    assert_eq!(
        report.top_residuals[0].expected_minus_actual_rgb_delta,
        [-90.0, -90.0, -90.0]
    );
    assert_close(
        report.top_residuals[0].selection_expected_rgb_distance,
        rgb_distance([12, 10, 10, 255], [10, 10, 10, 255]),
    );
    let markdown = markdown(&report);
    assert!(markdown.contains("Texture Sampling Parity Audit"));
    assert!(markdown.contains("Frontmost shading-model buckets"));
    assert!(markdown.contains("Top Residuals By Shading Model"));
    assert!(markdown.contains("Top Residuals: mtoon"));
    assert!(markdown.contains("Recommended Material Probes"));
    assert!(markdown.contains("selected_sample_matches_three_vrm"));
    assert!(markdown.contains("Selection Source Buckets"));
    assert!(markdown.contains("| webgl-coverage | 1 |"));
    assert!(markdown.contains("Manifest-selected material+draw buckets"));
    assert!(markdown.contains("Selected draw"));
    assert!(markdown.contains("LS gain A/M / E/M"));
    assert!(markdown.contains("Missing LS gain (E-A)/M"));
    assert!(markdown.contains("node145/mesh4/prim1/base"));
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
