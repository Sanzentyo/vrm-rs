#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
glam = "0.32.1"
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
vrm-adapter = { path = "../../crates/vrm-adapter" }
vrm-core = { path = "../../crates/vrm-core" }
vrm-io = { path = "../../crates/vrm-io" }
vrm-rs = { path = "../.." }
---

//! Map direct imqraw hotspot pixels back to CPU-projected glTF primitives.

use clap::Parser;
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use vrm_adapter::{
    renderer_material_pipeline_plan, GltfMaterialAlphaMode, GltfMaterialPipelineOverride,
    MtoonMaterializationOptions, RendererMaterialAlphaMode, RendererMaterialCullMode,
    RendererMaterialPipelinePlan,
};
use vrm_core::MaterialRef;
use vrm_io::{
    transform_tex_coord_0, CpuRgba8Image, GltfAlphaMode, GltfExpressionRenderEffects,
    GltfMaterialShadingOptions, GltfOutlineScale, GltfOutlineVertexSettings,
    GltfTransformedVertex, LoadedVrm, Rgba8SamplingOrigin,
};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "map-render-hotspots",
    about = "Map imqraw delta hotspot pixels to CPU-projected VRM primitive/material candidates"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    fixture: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    deltas: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    width: Option<usize>,
    #[arg(long)]
    height: Option<usize>,
    #[arg(long, default_value_t = 1.0)]
    camera_y: f32,
    #[arg(long, default_value_t = 3.0)]
    camera_z: f32,
    #[arg(long, default_value_t = 1.0)]
    target_y: f32,
    #[arg(long, default_value_t = 0.0)]
    mtoon_time: f32,
    #[arg(long, default_value_t = 1.0)]
    outline_width_scale: f32,
    #[arg(long)]
    disable_outlines: bool,
    #[arg(long)]
    expand_outlines: bool,
    #[arg(long = "expression")]
    expressions: Vec<String>,
    #[arg(long, default_value_t = 32)]
    top_pixels: usize,
    #[arg(long, default_value_t = 64)]
    candidate_limit: usize,
    #[arg(long, default_value_t = 1)]
    hit_radius: i32,
    #[arg(long, default_value_t = 0.5)]
    sample_center_x: f32,
    #[arg(long, default_value_t = 0.5)]
    sample_center_y: f32,
    #[arg(long, default_value_t = 5)]
    subpixel_steps: usize,
    #[arg(long, default_value_t = 0.00001)]
    source_order_depth_epsilon: f32,
}

#[derive(Clone, Debug, Deserialize)]
struct DeltaReport {
    width: usize,
    height: usize,
    top: Vec<DeltaPixel>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DeltaPixel {
    x: usize,
    y: usize,
    pixel: usize,
    expected: [u8; 4],
    actual: [u8; 4],
    delta: [u8; 4],
    #[serde(rename = "maxChannelDelta")]
    max_channel_delta: u8,
    #[serde(rename = "rgbDistance")]
    rgb_distance: f64,
}

#[derive(Clone, Debug)]
struct Surface {
    draw_index: usize,
    node: usize,
    mesh: usize,
    primitive: usize,
    pass: &'static str,
    material: Option<usize>,
    material_name: Option<String>,
    policy: MaterialPolicyReport,
    base_uv_transform: Option<vrm_rs::core::TextureTransform2d>,
    base_texture: Option<CpuRgba8Image>,
    base_color: [f32; 4],
    base_color_alpha: f32,
    pbr_fallback: bool,
    indices: Vec<u32>,
    edge_adjacency: BTreeMap<[u32; 2], Vec<usize>>,
    vertices: Vec<GltfTransformedVertex>,
}

#[derive(Clone, Debug, Serialize)]
struct HotspotReport {
    fixture: String,
    deltas: String,
    width: usize,
    height: usize,
    camera: CameraReport,
    sample_center: [f32; 2],
    summary: HotspotSummary,
    hotspots: Vec<Hotspot>,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct CameraReport {
    y: f32,
    z: f32,
    target_y: f32,
}

#[derive(Clone, Debug, Serialize)]
struct HotspotSummary {
    hotspot_count: usize,
    frontmost_any_count: usize,
    frontmost_alpha_visible_count: usize,
    frontmost_visible_count: usize,
    frontmost_any_cull_rejected_count: usize,
    nearest_sample_visible_frontmost_count: usize,
    missing_center_recovered_by_nearest_visible_count: usize,
    strict_frontmost_visible_count: usize,
    strict_frontmost_differs_from_loose_count: usize,
    cull_policy_rejected_candidate_count: usize,
    alpha_policy_rejected_candidate_count: usize,
    actual_frontmost_any_triangle_matches: usize,
    expected_frontmost_any_triangle_matches: usize,
    actual_frontmost_alpha_visible_triangle_matches: usize,
    expected_frontmost_alpha_visible_triangle_matches: usize,
    actual_frontmost_triangle_matches: usize,
    expected_frontmost_triangle_matches: usize,
    actual_frontmost_material_matches: usize,
    expected_frontmost_material_matches: usize,
    actual_frontmost_pass_matches: usize,
    expected_frontmost_pass_matches: usize,
    frontmost_pass_counts: Vec<PassCount>,
    nearest_visible_actual_pass_counts: Vec<PassCount>,
    nearest_visible_expected_pass_counts: Vec<PassCount>,
    actual_frontmost_surface_transitions: Vec<SurfaceTransitionCount>,
    expected_frontmost_surface_transitions: Vec<SurfaceTransitionCount>,
    actual_frontmost_mean_uv_distance: Option<f32>,
    expected_frontmost_mean_uv_distance: Option<f32>,
    actual_frontmost_max_uv_distance: Option<f32>,
    expected_frontmost_max_uv_distance: Option<f32>,
    actual_frontmost_mean_rgb_distance: Option<f32>,
    expected_frontmost_mean_rgb_distance: Option<f32>,
    actual_frontmost_max_rgb_distance: Option<f32>,
    expected_frontmost_max_rgb_distance: Option<f32>,
    actual_frontmost_mean_base_texture_rgb_distance: Option<f32>,
    expected_frontmost_mean_base_texture_rgb_distance: Option<f32>,
    actual_frontmost_max_base_texture_rgb_distance: Option<f32>,
    expected_frontmost_max_base_texture_rgb_distance: Option<f32>,
    actual_nearest_sample_visible_mean_base_texture_rgb_distance: Option<f32>,
    expected_nearest_sample_visible_mean_base_texture_rgb_distance: Option<f32>,
    actual_nearest_sample_visible_max_base_texture_rgb_distance: Option<f32>,
    expected_nearest_sample_visible_max_base_texture_rgb_distance: Option<f32>,
    actual_missing_center_nearest_visible_mean_base_texture_rgb_distance: Option<f32>,
    expected_missing_center_nearest_visible_mean_base_texture_rgb_distance: Option<f32>,
    actual_missing_center_nearest_visible_max_base_texture_rgb_distance: Option<f32>,
    expected_missing_center_nearest_visible_max_base_texture_rgb_distance: Option<f32>,
    actual_frontmost_mean_cpu_base_color_rgb_distance: Option<f32>,
    expected_frontmost_mean_cpu_base_color_rgb_distance: Option<f32>,
    actual_frontmost_max_cpu_base_color_rgb_distance: Option<f32>,
    expected_frontmost_max_cpu_base_color_rgb_distance: Option<f32>,
    actual_nearest_sample_visible_mean_cpu_base_color_rgb_distance: Option<f32>,
    expected_nearest_sample_visible_mean_cpu_base_color_rgb_distance: Option<f32>,
    actual_nearest_sample_visible_max_cpu_base_color_rgb_distance: Option<f32>,
    expected_nearest_sample_visible_max_cpu_base_color_rgb_distance: Option<f32>,
    actual_strict_frontmost_mean_cpu_base_color_rgb_distance: Option<f32>,
    expected_strict_frontmost_mean_cpu_base_color_rgb_distance: Option<f32>,
    actual_strict_frontmost_max_cpu_base_color_rgb_distance: Option<f32>,
    expected_strict_frontmost_max_cpu_base_color_rgb_distance: Option<f32>,
    actual_strict_frontmost_improved_count: usize,
    expected_strict_frontmost_improved_count: usize,
    strict_frontmost_same_material_count: usize,
    strict_frontmost_same_triangle_count: usize,
    strict_frontmost_surface_transitions: Vec<SurfaceTransitionCount>,
    frontmost_texture_sampling_variants: Vec<TextureSamplingVariantSummary>,
    nearest_sample_visible_texture_sampling_variants: Vec<TextureSamplingVariantSummary>,
    frontmost_mean_base_texture_local_rgb_gradient: Option<f32>,
    frontmost_max_base_texture_local_rgb_gradient: Option<f32>,
    frontmost_base_texture_local_rgb_gradient_gte_32: usize,
    frontmost_base_texture_local_rgb_gradient_gte_64: usize,
    frontmost_base_texture_local_rgb_gradient_gte_96: usize,
    frontmost_mean_edge_distance_pixels: Option<f32>,
    frontmost_edge_distance_lte_025px: usize,
    frontmost_edge_distance_lte_050px: usize,
    frontmost_edge_distance_lte_100px: usize,
    actual_frontmost_edge_neighbor_matches: usize,
    expected_frontmost_edge_neighbor_matches: usize,
    actual_expected_same_pass_matches: usize,
    actual_expected_same_material_matches: usize,
    actual_expected_same_triangle_matches: usize,
    frontmost_nearest_edge_counts: Vec<EdgeBucketCount>,
    nearest_sample_visible_offsets: Vec<OffsetCount>,
    missing_center_nearest_visible_offsets: Vec<OffsetCount>,
    actual_visible_sample_offsets: Vec<OffsetCount>,
    expected_visible_sample_offsets: Vec<OffsetCount>,
    actual_expected_surface_transitions: Vec<SurfaceTransitionCount>,
    actual_best_subpixel_visible_count: usize,
    expected_best_subpixel_visible_count: usize,
    actual_best_subpixel_improved_count: usize,
    expected_best_subpixel_improved_count: usize,
    actual_best_subpixel_mean_cpu_base_color_rgb_distance: Option<f32>,
    expected_best_subpixel_mean_cpu_base_color_rgb_distance: Option<f32>,
    actual_best_subpixel_mean_cpu_base_color_improvement: Option<f32>,
    expected_best_subpixel_mean_cpu_base_color_improvement: Option<f32>,
    actual_best_subpixel_same_triangle_matches: usize,
    expected_best_subpixel_same_triangle_matches: usize,
    actual_best_subpixel_improved_same_triangle_count: usize,
    expected_best_subpixel_improved_same_triangle_count: usize,
    actual_best_subpixel_improved_different_triangle_count: usize,
    expected_best_subpixel_improved_different_triangle_count: usize,
    actual_best_subpixel_mean_sample_distance_from_center: Option<f32>,
    expected_best_subpixel_mean_sample_distance_from_center: Option<f32>,
    actual_best_subpixel_surface_transitions: Vec<SurfaceTransitionCount>,
    expected_best_subpixel_surface_transitions: Vec<SurfaceTransitionCount>,
    actual_subpixel_sample_summaries: Vec<SubpixelSampleSummary>,
    expected_subpixel_sample_summaries: Vec<SubpixelSampleSummary>,
    source_order_depth_epsilon: f32,
    depth_near_later_visible_count: usize,
    actual_depth_near_later_improved_count: usize,
    expected_depth_near_later_improved_count: usize,
    actual_depth_near_later_mean_cpu_base_color_rgb_distance: Option<f32>,
    expected_depth_near_later_mean_cpu_base_color_rgb_distance: Option<f32>,
    actual_depth_near_later_mean_cpu_base_color_improvement: Option<f32>,
    expected_depth_near_later_mean_cpu_base_color_improvement: Option<f32>,
    depth_near_later_same_material_count: usize,
    depth_near_later_same_triangle_count: usize,
    depth_near_later_surface_transitions: Vec<SurfaceTransitionCount>,
}

#[derive(Clone, Debug, Serialize)]
struct EdgeBucketCount {
    node: usize,
    mesh: usize,
    primitive: usize,
    pass: &'static str,
    material: Option<usize>,
    material_name: Option<String>,
    triangle: usize,
    edge: usize,
    edge_indices: [u32; 2],
    count: usize,
    mean_edge_distance_pixels: f32,
}

#[derive(Clone, Debug, Serialize)]
struct PassCount {
    pass: &'static str,
    count: usize,
}

#[derive(Clone, Debug, Serialize)]
struct SurfaceTransitionCount {
    from: SurfaceKeyReport,
    to: SurfaceKeyReport,
    count: usize,
}

#[derive(Clone, Debug, Serialize)]
struct SurfaceKeyReport {
    pass: &'static str,
    material: Option<usize>,
    material_name: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SurfaceKey {
    pass: &'static str,
    material: Option<usize>,
    material_name: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EdgeBucketKey {
    node: usize,
    mesh: usize,
    primitive: usize,
    pass: &'static str,
    material: Option<usize>,
    material_name: Option<String>,
    triangle: usize,
    edge: usize,
    edge_indices: [u32; 2],
}

#[derive(Clone, Debug, Serialize)]
struct OffsetCount {
    offset: [i32; 2],
    count: usize,
}

#[derive(Clone, Debug, Serialize)]
struct SubpixelSampleSummary {
    sample: [f32; 2],
    visible_count: usize,
    same_triangle_count: usize,
    improved_count: usize,
    improved_same_triangle_count: usize,
    improved_different_triangle_count: usize,
    mean_cpu_base_color_rgb_distance: f32,
    mean_cpu_base_color_improvement: Option<f32>,
    mean_sample_distance_from_center: f32,
}

#[derive(Clone, Debug, Serialize)]
struct Hotspot {
    x: usize,
    y: usize,
    pixel: usize,
    expected: [u8; 4],
    actual: [u8; 4],
    delta: [u8; 4],
    expected_linear_uv: [f32; 2],
    actual_linear_uv: [f32; 2],
    max_channel_delta: u8,
    rgb_distance: f64,
    nearest_expected: Option<CandidateMatch>,
    nearest_actual: Option<CandidateMatch>,
    nearest_visible_expected: Option<CandidateMatch>,
    nearest_visible_actual: Option<CandidateMatch>,
    frontmost_any: Option<CandidateMatch>,
    frontmost_alpha_visible: Option<CandidateMatch>,
    frontmost_visible: Option<CandidateMatch>,
    strict_frontmost_visible: Option<CandidateMatch>,
    nearest_sample_visible_frontmost: Option<CandidateMatch>,
    nearest_sample_any_frontmost: Option<CandidateMatch>,
    depth_near_later_visible: Option<CandidateMatch>,
    frontmost_base_uv_srgb: Option<[u8; 4]>,
    frontmost_base_texture_rgba: Option<[u8; 4]>,
    frontmost_expected_rgb_distance: Option<f32>,
    frontmost_actual_rgb_distance: Option<f32>,
    frontmost_base_texture_expected_rgb_distance: Option<f32>,
    frontmost_base_texture_actual_rgb_distance: Option<f32>,
    nearest_sample_visible_base_texture_rgba: Option<[u8; 4]>,
    nearest_sample_visible_base_texture_expected_rgb_distance: Option<f32>,
    nearest_sample_visible_base_texture_actual_rgb_distance: Option<f32>,
    frontmost_cpu_base_color_rgba: Option<[u8; 4]>,
    frontmost_cpu_base_color_expected_rgb_distance: Option<f32>,
    frontmost_cpu_base_color_actual_rgb_distance: Option<f32>,
    strict_frontmost_cpu_base_color_rgba: Option<[u8; 4]>,
    strict_frontmost_cpu_base_color_expected_rgb_distance: Option<f32>,
    strict_frontmost_cpu_base_color_actual_rgb_distance: Option<f32>,
    nearest_sample_visible_cpu_base_color_rgba: Option<[u8; 4]>,
    nearest_sample_visible_cpu_base_color_expected_rgb_distance: Option<f32>,
    nearest_sample_visible_cpu_base_color_actual_rgb_distance: Option<f32>,
    depth_near_later_cpu_base_color_rgba: Option<[u8; 4]>,
    depth_near_later_cpu_base_color_expected_rgb_distance: Option<f32>,
    depth_near_later_cpu_base_color_actual_rgb_distance: Option<f32>,
    frontmost_texture_sampling_variants: Vec<TextureSamplingDistance>,
    nearest_sample_visible_texture_sampling_variants: Vec<TextureSamplingDistance>,
    subpixel_visible_candidates: Vec<SubpixelCandidate>,
    best_subpixel_visible_actual: Option<SubpixelMatch>,
    best_subpixel_visible_expected: Option<SubpixelMatch>,
    candidates: Vec<HitCandidate>,
}

#[derive(Clone, Debug, Serialize)]
struct SubpixelMatch {
    sample: [f32; 2],
    rgb_distance: f32,
    center_rgb_distance: Option<f32>,
    improvement: Option<f32>,
    candidate: CandidateMatch,
}

#[derive(Clone, Debug, Serialize)]
struct TextureSamplingVariantSummary {
    mode: &'static str,
    count: usize,
    actual_mean_rgb_distance: f32,
    expected_mean_rgb_distance: f32,
    actual_max_rgb_distance: f32,
    expected_max_rgb_distance: f32,
    actual_closer: usize,
    expected_closer: usize,
    tied: usize,
    mean_expected_minus_actual: f32,
}

#[derive(Clone, Debug, Serialize)]
struct TextureSamplingDistance {
    mode: &'static str,
    rgba: [u8; 4],
    actual_rgb_distance: f32,
    expected_rgb_distance: f32,
}

#[derive(Clone, Debug, Serialize)]
struct TextureSamplingColor {
    mode: &'static str,
    rgba: [u8; 4],
}

#[derive(Clone, Debug, Serialize)]
struct CandidateMatch {
    candidate_index: usize,
    draw_index: usize,
    base_uv_distance: f32,
    pass: &'static str,
    material: Option<usize>,
    material_name: Option<String>,
    policy: MaterialPolicyReport,
    sample_offset: [i32; 2],
    sample_distance: f32,
    node: usize,
    mesh: usize,
    primitive: usize,
    triangle: usize,
    indices: [u32; 3],
    depth: f32,
    min_barycentric: f32,
    edge_distance_pixels: f32,
    nearest_edge: usize,
    nearest_edge_indices: [u32; 2],
    nearest_edge_neighbor_triangles: Vec<usize>,
    front_facing: bool,
    alpha: f32,
    visible_by_cull_policy: bool,
    visible_by_alpha_policy: bool,
    visible_by_policy: bool,
    base_uv: [f32; 2],
    base_texture_rgba: Option<[u8; 4]>,
    cpu_base_color_rgba: [u8; 4],
    base_texture_sampling_rgba: Vec<TextureSamplingColor>,
    base_texture_local_rgb_gradient: Option<f32>,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct MaterialPolicyReport {
    render_order: i32,
    phase_order: i32,
    cull_mode: &'static str,
    alpha_mode: &'static str,
    depth_write: bool,
    blend: bool,
    alpha_cutoff: f32,
}

#[derive(Clone, Debug, Serialize)]
struct HitCandidate {
    draw_index: usize,
    node: usize,
    mesh: usize,
    primitive: usize,
    pass: &'static str,
    triangle: usize,
    indices: [u32; 3],
    material: Option<usize>,
    material_name: Option<String>,
    policy: MaterialPolicyReport,
    sample_offset: [i32; 2],
    sample_distance: f32,
    depth: f32,
    barycentric: [f32; 3],
    min_barycentric: f32,
    edge_distance_pixels: f32,
    nearest_edge: usize,
    nearest_edge_indices: [u32; 2],
    nearest_edge_neighbor_triangles: Vec<usize>,
    raw_uv: [f32; 2],
    base_uv: [f32; 2],
    base_texture_rgba: Option<[u8; 4]>,
    cpu_base_color_rgba: [u8; 4],
    base_texture_sampling_rgba: Vec<TextureSamplingColor>,
    base_texture_local_rgb_gradient: Option<f32>,
    screen: [[f32; 2]; 3],
    front_facing: bool,
    alpha: f32,
    visible_by_cull_policy: bool,
    visible_by_alpha_policy: bool,
    visible_by_policy: bool,
}

#[derive(Clone, Copy, Debug)]
struct ProjectedVertex {
    screen: [f32; 2],
    depth: f32,
    uv: [f32; 2],
    reciprocal_w: f32,
}

#[derive(Clone, Copy, Debug)]
struct TriangleEdgeDistance {
    edge: usize,
    distance_pixels: f32,
}

fn main() {
    if let Err(error) = run(Options::parse_from(script_args())) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    if options.self_test {
        run_self_test()?;
        return Ok(());
    }
    let fixture = options
        .fixture
        .as_deref()
        .ok_or("--fixture is required unless --self-test is used")?;
    let deltas = options
        .deltas
        .as_deref()
        .ok_or("--deltas is required unless --self-test is used")?;
    let delta_report = read_delta_report(deltas)?;
    let width = options.width.unwrap_or(delta_report.width);
    let height = options.height.unwrap_or(delta_report.height);
    let loaded = vrm_io::load_vrm_from_path(fixture)?;
    let expression_effects =
        loaded.expression_render_effects(parse_expression_args(&options.expressions)?)?;
    let surfaces = build_surfaces(&loaded, &expression_effects, &options)?;
    let view_projection = view_projection(width, height, &options);

    let hotspots: Vec<Hotspot> = delta_report
        .top
        .iter()
        .take(options.top_pixels)
        .map(|delta| {
            let expected_linear_uv = diagnostic_linear_uv(delta.expected);
            let actual_linear_uv = diagnostic_linear_uv(delta.actual);
            let candidates = candidates_for_pixel(
                delta.x,
                delta.y,
                &surfaces,
                view_projection,
                width,
                height,
                options.candidate_limit,
                options.hit_radius,
                [options.sample_center_x, options.sample_center_y],
            );
            let frontmost_any = frontmost_any_candidate_match(&candidates);
            let frontmost_alpha_visible = frontmost_alpha_visible_candidate_match(&candidates);
            let frontmost_visible = frontmost_visible_candidate_match(&candidates);
            let strict_frontmost_visible = strict_frontmost_visible_candidate_match(&candidates);
            let depth_near_later_visible = depth_near_later_visible_candidate_match(
                &candidates,
                frontmost_visible.as_ref(),
                options.source_order_depth_epsilon,
            );
            let nearest_sample_visible_frontmost =
                nearest_sample_visible_frontmost_candidate_match(&candidates);
            let nearest_sample_any_frontmost =
                nearest_sample_any_frontmost_candidate_match(&candidates);
            let frontmost_base_uv_srgb =
                frontmost_visible.as_ref().map(|frontmost| {
                    diagnostic_linear_uv_to_srgb_color(frontmost.base_uv, delta.actual[3])
                });
            let frontmost_base_texture_rgba = frontmost_visible
                .as_ref()
                .and_then(|frontmost| frontmost.base_texture_rgba);
            let nearest_sample_visible_base_texture_rgba = nearest_sample_visible_frontmost
                .as_ref()
                .and_then(|frontmost| frontmost.base_texture_rgba);
            let frontmost_cpu_base_color_rgba = frontmost_visible
                .as_ref()
                .map(|frontmost| frontmost.cpu_base_color_rgba);
            let strict_frontmost_cpu_base_color_rgba = strict_frontmost_visible
                .as_ref()
                .map(|frontmost| frontmost.cpu_base_color_rgba);
            let nearest_sample_visible_cpu_base_color_rgba = nearest_sample_visible_frontmost
                .as_ref()
                .map(|frontmost| frontmost.cpu_base_color_rgba);
            let depth_near_later_cpu_base_color_rgba = depth_near_later_visible
                .as_ref()
                .map(|candidate| candidate.cpu_base_color_rgba);
            let frontmost_texture_sampling_variants =
                texture_sampling_distances(frontmost_visible.as_ref(), delta.actual, delta.expected);
            let nearest_sample_visible_texture_sampling_variants = texture_sampling_distances(
                nearest_sample_visible_frontmost.as_ref(),
                delta.actual,
                delta.expected,
            );
            let subpixel_candidates = subpixel_frontmost_visible_candidates(
                delta.x,
                delta.y,
                &surfaces,
                view_projection,
                width,
                height,
                options.subpixel_steps,
            );
            let best_subpixel_visible_actual = best_subpixel_match(
                &subpixel_candidates,
                delta.actual,
                frontmost_cpu_base_color_rgba
                    .map(|color| rgb_distance(color, delta.actual)),
            );
            let best_subpixel_visible_expected = best_subpixel_match(
                &subpixel_candidates,
                delta.expected,
                frontmost_cpu_base_color_rgba
                    .map(|color| rgb_distance(color, delta.expected)),
            );
            Hotspot {
                x: delta.x,
                y: delta.y,
                pixel: delta.pixel,
                expected: delta.expected,
                actual: delta.actual,
                delta: delta.delta,
                expected_linear_uv,
                actual_linear_uv,
                max_channel_delta: delta.max_channel_delta,
                rgb_distance: delta.rgb_distance,
                nearest_expected: nearest_candidate_match(&candidates, expected_linear_uv),
                nearest_actual: nearest_candidate_match(&candidates, actual_linear_uv),
                nearest_visible_expected: nearest_visible_candidate_match(
                    &candidates,
                    expected_linear_uv,
                ),
                nearest_visible_actual: nearest_visible_candidate_match(
                    &candidates,
                    actual_linear_uv,
                ),
                frontmost_any,
                frontmost_alpha_visible,
                frontmost_visible,
                strict_frontmost_visible,
                nearest_sample_visible_frontmost,
                nearest_sample_any_frontmost,
                depth_near_later_visible,
                frontmost_base_uv_srgb,
                frontmost_base_texture_rgba,
                frontmost_expected_rgb_distance: frontmost_base_uv_srgb
                    .map(|color| rgb_distance(color, delta.expected)),
                frontmost_actual_rgb_distance: frontmost_base_uv_srgb
                    .map(|color| rgb_distance(color, delta.actual)),
                frontmost_base_texture_expected_rgb_distance: frontmost_base_texture_rgba
                    .map(|color| rgb_distance(color, delta.expected)),
                frontmost_base_texture_actual_rgb_distance: frontmost_base_texture_rgba
                    .map(|color| rgb_distance(color, delta.actual)),
                nearest_sample_visible_base_texture_rgba,
                nearest_sample_visible_base_texture_expected_rgb_distance:
                    nearest_sample_visible_base_texture_rgba
                        .map(|color| rgb_distance(color, delta.expected)),
                nearest_sample_visible_base_texture_actual_rgb_distance:
                    nearest_sample_visible_base_texture_rgba
                        .map(|color| rgb_distance(color, delta.actual)),
                frontmost_cpu_base_color_rgba,
                frontmost_cpu_base_color_expected_rgb_distance: frontmost_cpu_base_color_rgba
                    .map(|color| rgb_distance(color, delta.expected)),
                frontmost_cpu_base_color_actual_rgb_distance: frontmost_cpu_base_color_rgba
                    .map(|color| rgb_distance(color, delta.actual)),
                strict_frontmost_cpu_base_color_rgba,
                strict_frontmost_cpu_base_color_expected_rgb_distance:
                    strict_frontmost_cpu_base_color_rgba
                        .map(|color| rgb_distance(color, delta.expected)),
                strict_frontmost_cpu_base_color_actual_rgb_distance:
                    strict_frontmost_cpu_base_color_rgba
                        .map(|color| rgb_distance(color, delta.actual)),
                nearest_sample_visible_cpu_base_color_rgba,
                nearest_sample_visible_cpu_base_color_expected_rgb_distance:
                    nearest_sample_visible_cpu_base_color_rgba
                        .map(|color| rgb_distance(color, delta.expected)),
                nearest_sample_visible_cpu_base_color_actual_rgb_distance:
                    nearest_sample_visible_cpu_base_color_rgba
                        .map(|color| rgb_distance(color, delta.actual)),
                depth_near_later_cpu_base_color_rgba,
                depth_near_later_cpu_base_color_expected_rgb_distance:
                    depth_near_later_cpu_base_color_rgba
                        .map(|color| rgb_distance(color, delta.expected)),
                depth_near_later_cpu_base_color_actual_rgb_distance:
                    depth_near_later_cpu_base_color_rgba.map(|color| rgb_distance(color, delta.actual)),
                frontmost_texture_sampling_variants,
                nearest_sample_visible_texture_sampling_variants,
                subpixel_visible_candidates: subpixel_candidates,
                best_subpixel_visible_actual,
                best_subpixel_visible_expected,
                candidates,
            }
        })
        .collect();
    let summary = summarize_hotspots(&hotspots, options.source_order_depth_epsilon);

    let report = HotspotReport {
        fixture: display_path(fixture),
        deltas: display_path(deltas),
        width,
        height,
        camera: CameraReport {
            y: options.camera_y,
            z: options.camera_z,
            target_y: options.target_y,
        },
        sample_center: [options.sample_center_x, options.sample_center_y],
        summary,
        hotspots,
    };
    let formatted = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = options.out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, formatted)?;
    } else {
        print!("{formatted}");
    }
    Ok(())
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    assert_close(
        triangle_edge_distance_pixels([0.25, 0.25], [0.0, 0.0], [1.0, 0.0], [0.0, 1.0]),
        0.25,
        "interior point nearest axis edge",
    )?;
    assert_close(
        triangle_edge_distance_pixels([0.5, 0.5], [0.0, 0.0], [1.0, 0.0], [0.0, 1.0]),
        0.0,
        "point on hypotenuse edge",
    )?;
    assert_close(
        point_to_segment_distance([2.0, 0.0], [0.0, 0.0], [1.0, 0.0]),
        1.0,
        "point past segment endpoint",
    )?;
    let nearest = nearest_triangle_edge([0.5, 0.05], [0.0, 0.0], [1.0, 0.0], [0.0, 1.0]);
    if nearest.edge != 0 {
        return Err(format!("expected edge 0, got {}", nearest.edge).into());
    }
    if triangle_edge_indices([10, 11, 12], 2) != [12, 10] {
        return Err("triangle edge 2 indices should wrap from c to a".into());
    }
    let adjacency = edge_adjacency(&[0, 1, 2, 2, 1, 3]);
    if edge_neighbors(&adjacency, [1, 2], 0) != [1] {
        return Err("shared edge should report the adjacent triangle".into());
    }
    if edge_neighbors(&adjacency, [0, 1], 0) != Vec::<usize>::new() {
        return Err("boundary edge should not report neighbors".into());
    }
    let encoded = diagnostic_linear_uv_to_srgb_color([0.25, 0.5], 255);
    if diagnostic_linear_uv(encoded)
        .into_iter()
        .zip([0.25, 0.5])
        .any(|(actual, expected)| (actual - expected).abs() > 0.003)
    {
        return Err("base-uv sRGB encode/decode should round-trip within one byte".into());
    }
    Ok(())
}

fn read_delta_report(path: &Path) -> Result<DeltaReport, Box<dyn Error>> {
    let value = serde_json::from_str::<Value>(&fs::read_to_string(path)?)?;
    Ok(serde_json::from_value(value)?)
}

fn summarize_hotspots(hotspots: &[Hotspot], source_order_depth_epsilon: f32) -> HotspotSummary {
    HotspotSummary {
        hotspot_count: hotspots.len(),
        frontmost_any_count: hotspots
            .iter()
            .filter(|hotspot| hotspot.frontmost_any.is_some())
            .count(),
        frontmost_alpha_visible_count: hotspots
            .iter()
            .filter(|hotspot| hotspot.frontmost_alpha_visible.is_some())
            .count(),
        frontmost_visible_count: hotspots
            .iter()
            .filter(|hotspot| hotspot.frontmost_visible.is_some())
            .count(),
        frontmost_any_cull_rejected_count: hotspots
            .iter()
            .filter_map(|hotspot| hotspot.frontmost_any.as_ref())
            .filter(|frontmost| !frontmost.visible_by_cull_policy)
            .count(),
        nearest_sample_visible_frontmost_count: hotspots
            .iter()
            .filter(|hotspot| hotspot.nearest_sample_visible_frontmost.is_some())
            .count(),
        missing_center_recovered_by_nearest_visible_count: hotspots
            .iter()
            .filter(|hotspot| {
                hotspot.frontmost_visible.is_none()
                    && hotspot.nearest_sample_visible_frontmost.is_some()
            })
            .count(),
        strict_frontmost_visible_count: hotspots
            .iter()
            .filter(|hotspot| hotspot.strict_frontmost_visible.is_some())
            .count(),
        strict_frontmost_differs_from_loose_count: hotspots
            .iter()
            .filter(|hotspot| {
                matches!(
                    (
                        hotspot.frontmost_visible.as_ref(),
                        hotspot.strict_frontmost_visible.as_ref()
                    ),
                    (Some(loose), Some(strict)) if !same_surface_triangle(Some(loose), Some(strict))
                )
            })
            .count(),
        cull_policy_rejected_candidate_count: hotspots
            .iter()
            .flat_map(|hotspot| hotspot.candidates.iter())
            .filter(|candidate| !candidate.visible_by_cull_policy)
            .count(),
        alpha_policy_rejected_candidate_count: hotspots
            .iter()
            .flat_map(|hotspot| hotspot.candidates.iter())
            .filter(|candidate| candidate.visible_by_cull_policy && !candidate.visible_by_alpha_policy)
            .count(),
        actual_frontmost_any_triangle_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_any.as_ref(),
                    hotspot.nearest_actual.as_ref(),
                )
            })
            .count(),
        expected_frontmost_any_triangle_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_any.as_ref(),
                    hotspot.nearest_expected.as_ref(),
                )
            })
            .count(),
        actual_frontmost_alpha_visible_triangle_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_alpha_visible.as_ref(),
                    hotspot.nearest_actual.as_ref(),
                )
            })
            .count(),
        expected_frontmost_alpha_visible_triangle_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_alpha_visible.as_ref(),
                    hotspot.nearest_expected.as_ref(),
                )
            })
            .count(),
        actual_frontmost_triangle_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.nearest_visible_actual.as_ref(),
                )
            })
            .count(),
        expected_frontmost_triangle_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.nearest_visible_expected.as_ref(),
                )
            })
            .count(),
        actual_frontmost_material_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_material(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.nearest_visible_actual.as_ref(),
                )
            })
            .count(),
        expected_frontmost_material_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_material(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.nearest_visible_expected.as_ref(),
                )
            })
            .count(),
        actual_frontmost_pass_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_pass(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.nearest_visible_actual.as_ref(),
                )
            })
            .count(),
        expected_frontmost_pass_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_pass(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.nearest_visible_expected.as_ref(),
                )
            })
            .count(),
        frontmost_pass_counts: pass_counts(
            hotspots
                .iter()
                .filter_map(|hotspot| hotspot.frontmost_visible.as_ref()),
        ),
        nearest_visible_actual_pass_counts: pass_counts(
            hotspots
                .iter()
                .filter_map(|hotspot| hotspot.nearest_visible_actual.as_ref()),
        ),
        nearest_visible_expected_pass_counts: pass_counts(
            hotspots
                .iter()
                .filter_map(|hotspot| hotspot.nearest_visible_expected.as_ref()),
        ),
        actual_frontmost_surface_transitions: surface_transition_counts(hotspots, |hotspot| {
            hotspot.nearest_visible_actual.as_ref()
        }),
        expected_frontmost_surface_transitions: surface_transition_counts(hotspots, |hotspot| {
            hotspot.nearest_visible_expected.as_ref()
        }),
        actual_frontmost_mean_uv_distance: mean_frontmost_uv_distance(hotspots, |hotspot| {
            hotspot.actual_linear_uv
        }),
        expected_frontmost_mean_uv_distance: mean_frontmost_uv_distance(hotspots, |hotspot| {
            hotspot.expected_linear_uv
        }),
        actual_frontmost_max_uv_distance: max_frontmost_uv_distance(hotspots, |hotspot| {
            hotspot.actual_linear_uv
        }),
        expected_frontmost_max_uv_distance: max_frontmost_uv_distance(hotspots, |hotspot| {
            hotspot.expected_linear_uv
        }),
        actual_frontmost_mean_rgb_distance: mean_frontmost_rgb_distance(hotspots, |hotspot| {
            hotspot.frontmost_actual_rgb_distance
        }),
        expected_frontmost_mean_rgb_distance: mean_frontmost_rgb_distance(hotspots, |hotspot| {
            hotspot.frontmost_expected_rgb_distance
        }),
        actual_frontmost_max_rgb_distance: max_frontmost_rgb_distance(hotspots, |hotspot| {
            hotspot.frontmost_actual_rgb_distance
        }),
        expected_frontmost_max_rgb_distance: max_frontmost_rgb_distance(hotspots, |hotspot| {
            hotspot.frontmost_expected_rgb_distance
        }),
        actual_frontmost_mean_base_texture_rgb_distance: mean_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.frontmost_base_texture_actual_rgb_distance,
        ),
        expected_frontmost_mean_base_texture_rgb_distance: mean_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.frontmost_base_texture_expected_rgb_distance,
        ),
        actual_frontmost_max_base_texture_rgb_distance: max_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.frontmost_base_texture_actual_rgb_distance,
        ),
        expected_frontmost_max_base_texture_rgb_distance: max_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.frontmost_base_texture_expected_rgb_distance,
        ),
        actual_nearest_sample_visible_mean_base_texture_rgb_distance:
            mean_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_base_texture_actual_rgb_distance
            }),
        expected_nearest_sample_visible_mean_base_texture_rgb_distance:
            mean_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_base_texture_expected_rgb_distance
            }),
        actual_nearest_sample_visible_max_base_texture_rgb_distance: max_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.nearest_sample_visible_base_texture_actual_rgb_distance,
        ),
        expected_nearest_sample_visible_max_base_texture_rgb_distance:
            max_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_base_texture_expected_rgb_distance
            }),
        actual_missing_center_nearest_visible_mean_base_texture_rgb_distance:
            mean_missing_center_nearest_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_base_texture_actual_rgb_distance
            }),
        expected_missing_center_nearest_visible_mean_base_texture_rgb_distance:
            mean_missing_center_nearest_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_base_texture_expected_rgb_distance
            }),
        actual_missing_center_nearest_visible_max_base_texture_rgb_distance:
            max_missing_center_nearest_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_base_texture_actual_rgb_distance
            }),
        expected_missing_center_nearest_visible_max_base_texture_rgb_distance:
            max_missing_center_nearest_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_base_texture_expected_rgb_distance
            }),
        actual_frontmost_mean_cpu_base_color_rgb_distance: mean_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.frontmost_cpu_base_color_actual_rgb_distance,
        ),
        expected_frontmost_mean_cpu_base_color_rgb_distance: mean_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.frontmost_cpu_base_color_expected_rgb_distance,
        ),
        actual_frontmost_max_cpu_base_color_rgb_distance: max_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.frontmost_cpu_base_color_actual_rgb_distance,
        ),
        expected_frontmost_max_cpu_base_color_rgb_distance: max_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.frontmost_cpu_base_color_expected_rgb_distance,
        ),
        actual_nearest_sample_visible_mean_cpu_base_color_rgb_distance:
            mean_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_cpu_base_color_actual_rgb_distance
            }),
        expected_nearest_sample_visible_mean_cpu_base_color_rgb_distance:
            mean_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_cpu_base_color_expected_rgb_distance
            }),
        actual_nearest_sample_visible_max_cpu_base_color_rgb_distance:
            max_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_cpu_base_color_actual_rgb_distance
            }),
        expected_nearest_sample_visible_max_cpu_base_color_rgb_distance:
            max_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.nearest_sample_visible_cpu_base_color_expected_rgb_distance
            }),
        actual_strict_frontmost_mean_cpu_base_color_rgb_distance:
            mean_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.strict_frontmost_cpu_base_color_actual_rgb_distance
            }),
        expected_strict_frontmost_mean_cpu_base_color_rgb_distance:
            mean_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.strict_frontmost_cpu_base_color_expected_rgb_distance
            }),
        actual_strict_frontmost_max_cpu_base_color_rgb_distance:
            max_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.strict_frontmost_cpu_base_color_actual_rgb_distance
            }),
        expected_strict_frontmost_max_cpu_base_color_rgb_distance:
            max_frontmost_rgb_distance(hotspots, |hotspot| {
                hotspot.strict_frontmost_cpu_base_color_expected_rgb_distance
            }),
        actual_strict_frontmost_improved_count: strict_frontmost_improved_count(
            hotspots,
            |hotspot| hotspot.strict_frontmost_cpu_base_color_actual_rgb_distance,
            |hotspot| hotspot.frontmost_cpu_base_color_actual_rgb_distance,
        ),
        expected_strict_frontmost_improved_count: strict_frontmost_improved_count(
            hotspots,
            |hotspot| hotspot.strict_frontmost_cpu_base_color_expected_rgb_distance,
            |hotspot| hotspot.frontmost_cpu_base_color_expected_rgb_distance,
        ),
        strict_frontmost_same_material_count: hotspots
            .iter()
            .filter(|hotspot| {
                same_material(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.strict_frontmost_visible.as_ref(),
                )
            })
            .count(),
        strict_frontmost_same_triangle_count: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.strict_frontmost_visible.as_ref(),
                )
            })
            .count(),
        strict_frontmost_surface_transitions: surface_pair_transition_counts(
            hotspots,
            |hotspot| hotspot.frontmost_visible.as_ref(),
            |hotspot| hotspot.strict_frontmost_visible.as_ref(),
        ),
        frontmost_texture_sampling_variants: texture_sampling_variant_summary(
            hotspots.iter().flat_map(|hotspot| {
                hotspot.frontmost_texture_sampling_variants.iter()
            }),
        ),
        nearest_sample_visible_texture_sampling_variants: texture_sampling_variant_summary(
            hotspots.iter().flat_map(|hotspot| {
                hotspot.nearest_sample_visible_texture_sampling_variants.iter()
            }),
        ),
        frontmost_mean_base_texture_local_rgb_gradient: mean_frontmost_texture_gradient(hotspots),
        frontmost_max_base_texture_local_rgb_gradient: max_frontmost_texture_gradient(hotspots),
        frontmost_base_texture_local_rgb_gradient_gte_32: frontmost_texture_gradient_gte(
            hotspots, 32.0,
        ),
        frontmost_base_texture_local_rgb_gradient_gte_64: frontmost_texture_gradient_gte(
            hotspots, 64.0,
        ),
        frontmost_base_texture_local_rgb_gradient_gte_96: frontmost_texture_gradient_gte(
            hotspots, 96.0,
        ),
        frontmost_mean_edge_distance_pixels: mean_frontmost_edge_distance(hotspots),
        frontmost_edge_distance_lte_025px: frontmost_edge_distance_lte(hotspots, 0.25),
        frontmost_edge_distance_lte_050px: frontmost_edge_distance_lte(hotspots, 0.50),
        frontmost_edge_distance_lte_100px: frontmost_edge_distance_lte(hotspots, 1.00),
        actual_frontmost_edge_neighbor_matches: hotspots
            .iter()
            .filter(|hotspot| {
                frontmost_edge_neighbor_matches(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.nearest_visible_actual.as_ref(),
                )
            })
            .count(),
        expected_frontmost_edge_neighbor_matches: hotspots
            .iter()
            .filter(|hotspot| {
                frontmost_edge_neighbor_matches(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.nearest_visible_expected.as_ref(),
                )
            })
            .count(),
        actual_expected_same_pass_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_pass(
                    hotspot.nearest_visible_actual.as_ref(),
                    hotspot.nearest_visible_expected.as_ref(),
                )
            })
            .count(),
        actual_expected_same_material_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_material(
                    hotspot.nearest_visible_actual.as_ref(),
                    hotspot.nearest_visible_expected.as_ref(),
                )
            })
            .count(),
        actual_expected_same_triangle_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.nearest_visible_actual.as_ref(),
                    hotspot.nearest_visible_expected.as_ref(),
                )
            })
            .count(),
        frontmost_nearest_edge_counts: frontmost_nearest_edge_counts(hotspots),
        nearest_sample_visible_offsets: offset_counts(
            hotspots
                .iter()
                .filter_map(|hotspot| hotspot.nearest_sample_visible_frontmost.as_ref()),
        ),
        missing_center_nearest_visible_offsets: offset_counts(hotspots.iter().filter_map(
            |hotspot| {
                hotspot
                    .frontmost_visible
                    .is_none()
                    .then_some(hotspot.nearest_sample_visible_frontmost.as_ref())
                    .flatten()
            },
        )),
        actual_visible_sample_offsets: offset_counts(
            hotspots
                .iter()
                .filter_map(|hotspot| hotspot.nearest_visible_actual.as_ref()),
        ),
        expected_visible_sample_offsets: offset_counts(
            hotspots
                .iter()
                .filter_map(|hotspot| hotspot.nearest_visible_expected.as_ref()),
        ),
        actual_expected_surface_transitions: surface_pair_transition_counts(
            hotspots,
            |hotspot| hotspot.nearest_visible_actual.as_ref(),
            |hotspot| hotspot.nearest_visible_expected.as_ref(),
        ),
        actual_best_subpixel_visible_count: hotspots
            .iter()
            .filter(|hotspot| hotspot.best_subpixel_visible_actual.is_some())
            .count(),
        expected_best_subpixel_visible_count: hotspots
            .iter()
            .filter(|hotspot| hotspot.best_subpixel_visible_expected.is_some())
            .count(),
        actual_best_subpixel_improved_count: subpixel_improved_count(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_actual.as_ref(),
        ),
        expected_best_subpixel_improved_count: subpixel_improved_count(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_expected.as_ref(),
        ),
        actual_best_subpixel_mean_cpu_base_color_rgb_distance: mean_subpixel_distance(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_actual.as_ref(),
        ),
        expected_best_subpixel_mean_cpu_base_color_rgb_distance: mean_subpixel_distance(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_expected.as_ref(),
        ),
        actual_best_subpixel_mean_cpu_base_color_improvement: mean_subpixel_improvement(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_actual.as_ref(),
        ),
        expected_best_subpixel_mean_cpu_base_color_improvement: mean_subpixel_improvement(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_expected.as_ref(),
        ),
        actual_best_subpixel_same_triangle_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot
                        .best_subpixel_visible_actual
                        .as_ref()
                        .map(|matched| &matched.candidate),
                )
            })
            .count(),
        expected_best_subpixel_same_triangle_matches: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot
                        .best_subpixel_visible_expected
                        .as_ref()
                        .map(|matched| &matched.candidate),
                )
            })
            .count(),
        actual_best_subpixel_improved_same_triangle_count: subpixel_improved_same_triangle_count(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_actual.as_ref(),
        ),
        expected_best_subpixel_improved_same_triangle_count: subpixel_improved_same_triangle_count(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_expected.as_ref(),
        ),
        actual_best_subpixel_improved_different_triangle_count:
            subpixel_improved_different_triangle_count(
                hotspots,
                |hotspot| hotspot.best_subpixel_visible_actual.as_ref(),
            ),
        expected_best_subpixel_improved_different_triangle_count:
            subpixel_improved_different_triangle_count(
                hotspots,
                |hotspot| hotspot.best_subpixel_visible_expected.as_ref(),
            ),
        actual_best_subpixel_mean_sample_distance_from_center: mean_subpixel_sample_distance(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_actual.as_ref(),
        ),
        expected_best_subpixel_mean_sample_distance_from_center: mean_subpixel_sample_distance(
            hotspots,
            |hotspot| hotspot.best_subpixel_visible_expected.as_ref(),
        ),
        actual_best_subpixel_surface_transitions: surface_pair_transition_counts(
            hotspots,
            |hotspot| hotspot.frontmost_visible.as_ref(),
            |hotspot| {
                hotspot
                    .best_subpixel_visible_actual
                    .as_ref()
                    .map(|matched| &matched.candidate)
            },
        ),
        expected_best_subpixel_surface_transitions: surface_pair_transition_counts(
            hotspots,
            |hotspot| hotspot.frontmost_visible.as_ref(),
            |hotspot| {
                hotspot
                    .best_subpixel_visible_expected
                    .as_ref()
                    .map(|matched| &matched.candidate)
            },
        ),
        actual_subpixel_sample_summaries: subpixel_sample_summaries(
            hotspots,
            |hotspot| hotspot.actual,
            |hotspot| hotspot.frontmost_cpu_base_color_actual_rgb_distance,
        ),
        expected_subpixel_sample_summaries: subpixel_sample_summaries(
            hotspots,
            |hotspot| hotspot.expected,
            |hotspot| hotspot.frontmost_cpu_base_color_expected_rgb_distance,
        ),
        source_order_depth_epsilon,
        depth_near_later_visible_count: hotspots
            .iter()
            .filter(|hotspot| hotspot.depth_near_later_visible.is_some())
            .count(),
        actual_depth_near_later_improved_count: depth_near_later_improved_count(
            hotspots,
            |hotspot| hotspot.depth_near_later_cpu_base_color_actual_rgb_distance,
            |hotspot| hotspot.frontmost_cpu_base_color_actual_rgb_distance,
        ),
        expected_depth_near_later_improved_count: depth_near_later_improved_count(
            hotspots,
            |hotspot| hotspot.depth_near_later_cpu_base_color_expected_rgb_distance,
            |hotspot| hotspot.frontmost_cpu_base_color_expected_rgb_distance,
        ),
        actual_depth_near_later_mean_cpu_base_color_rgb_distance: mean_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.depth_near_later_cpu_base_color_actual_rgb_distance,
        ),
        expected_depth_near_later_mean_cpu_base_color_rgb_distance: mean_frontmost_rgb_distance(
            hotspots,
            |hotspot| hotspot.depth_near_later_cpu_base_color_expected_rgb_distance,
        ),
        actual_depth_near_later_mean_cpu_base_color_improvement:
            depth_near_later_mean_improvement(
                hotspots,
                |hotspot| hotspot.depth_near_later_cpu_base_color_actual_rgb_distance,
                |hotspot| hotspot.frontmost_cpu_base_color_actual_rgb_distance,
            ),
        expected_depth_near_later_mean_cpu_base_color_improvement:
            depth_near_later_mean_improvement(
                hotspots,
                |hotspot| hotspot.depth_near_later_cpu_base_color_expected_rgb_distance,
                |hotspot| hotspot.frontmost_cpu_base_color_expected_rgb_distance,
            ),
        depth_near_later_same_material_count: hotspots
            .iter()
            .filter(|hotspot| {
                same_material(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.depth_near_later_visible.as_ref(),
                )
            })
            .count(),
        depth_near_later_same_triangle_count: hotspots
            .iter()
            .filter(|hotspot| {
                same_surface_triangle(
                    hotspot.frontmost_visible.as_ref(),
                    hotspot.depth_near_later_visible.as_ref(),
                )
            })
            .count(),
        depth_near_later_surface_transitions: surface_pair_transition_counts(
            hotspots,
            |hotspot| hotspot.frontmost_visible.as_ref(),
            |hotspot| hotspot.depth_near_later_visible.as_ref(),
        ),
    }
}

fn same_surface_triangle(left: Option<&CandidateMatch>, right: Option<&CandidateMatch>) -> bool {
    matches!(
        (left, right),
        (Some(left), Some(right))
            if left.draw_index == right.draw_index
                && left.pass == right.pass
                && left.material == right.material
                && left.triangle == right.triangle
    )
}

fn same_material(left: Option<&CandidateMatch>, right: Option<&CandidateMatch>) -> bool {
    matches!(
        (left, right),
        (Some(left), Some(right))
            if left.pass == right.pass && left.material == right.material
    )
}

fn same_pass(left: Option<&CandidateMatch>, right: Option<&CandidateMatch>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left.pass == right.pass)
}

fn pass_counts<'a>(candidates: impl Iterator<Item = &'a CandidateMatch>) -> Vec<PassCount> {
    let mut counts = BTreeMap::<&'static str, usize>::new();
    for candidate in candidates {
        *counts.entry(candidate.pass).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(pass, count)| PassCount { pass, count })
        .collect()
}

fn surface_transition_counts<'a>(
    hotspots: &'a [Hotspot],
    to: impl Fn(&'a Hotspot) -> Option<&'a CandidateMatch>,
) -> Vec<SurfaceTransitionCount> {
    surface_pair_transition_counts(hotspots, |hotspot| hotspot.frontmost_visible.as_ref(), to)
}

fn surface_pair_transition_counts<'a>(
    hotspots: &'a [Hotspot],
    from: impl Fn(&'a Hotspot) -> Option<&'a CandidateMatch>,
    to: impl Fn(&'a Hotspot) -> Option<&'a CandidateMatch>,
) -> Vec<SurfaceTransitionCount> {
    let mut counts = BTreeMap::<(SurfaceKey, SurfaceKey), usize>::new();
    for hotspot in hotspots {
        let Some(from) = from(hotspot) else {
            continue;
        };
        let Some(to) = to(hotspot) else {
            continue;
        };
        *counts
            .entry((SurfaceKey::from(from), SurfaceKey::from(to)))
            .or_default() += 1;
    }
    let mut transitions = counts
        .into_iter()
        .map(|((from, to), count)| SurfaceTransitionCount {
            from: SurfaceKeyReport::from(from),
            to: SurfaceKeyReport::from(to),
            count,
        })
        .collect::<Vec<_>>();
    transitions.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.from.pass.cmp(right.from.pass))
            .then_with(|| left.to.pass.cmp(right.to.pass))
            .then_with(|| left.from.material.cmp(&right.from.material))
            .then_with(|| left.to.material.cmp(&right.to.material))
    });
    transitions
}

impl From<&CandidateMatch> for SurfaceKey {
    fn from(value: &CandidateMatch) -> Self {
        Self {
            pass: value.pass,
            material: value.material,
            material_name: value.material_name.clone(),
        }
    }
}

impl From<SurfaceKey> for SurfaceKeyReport {
    fn from(value: SurfaceKey) -> Self {
        Self {
            pass: value.pass,
            material: value.material,
            material_name: value.material_name,
        }
    }
}

fn frontmost_edge_neighbor_matches(
    frontmost: Option<&CandidateMatch>,
    other: Option<&CandidateMatch>,
) -> bool {
    matches!(
        (frontmost, other),
        (Some(frontmost), Some(other))
            if frontmost.draw_index == other.draw_index
                && frontmost.pass == other.pass
                && frontmost.material == other.material
                && frontmost.nearest_edge_neighbor_triangles.contains(&other.triangle)
    )
}

fn mean_frontmost_uv_distance(
    hotspots: &[Hotspot],
    uv: impl Fn(&Hotspot) -> [f32; 2],
) -> Option<f32> {
    let (sum, count) = hotspots
        .iter()
        .filter_map(|hotspot| {
            hotspot
                .frontmost_visible
                .as_ref()
                .map(|frontmost| uv_distance(frontmost.base_uv, uv(hotspot)))
        })
        .fold((0.0, 0usize), |(sum, count), distance| {
            (sum + distance, count + 1)
        });
    (count > 0).then_some(sum / count as f32)
}

fn max_frontmost_uv_distance(
    hotspots: &[Hotspot],
    uv: impl Fn(&Hotspot) -> [f32; 2],
) -> Option<f32> {
    hotspots
        .iter()
        .filter_map(|hotspot| {
            hotspot
                .frontmost_visible
                .as_ref()
                .map(|frontmost| uv_distance(frontmost.base_uv, uv(hotspot)))
        })
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

fn mean_frontmost_rgb_distance(
    hotspots: &[Hotspot],
    distance: impl Fn(&Hotspot) -> Option<f32>,
) -> Option<f32> {
    let (sum, count) = hotspots
        .iter()
        .filter_map(distance)
        .fold((0.0, 0usize), |(sum, count), distance| {
            (sum + distance, count + 1)
        });
    (count > 0).then_some(sum / count as f32)
}

fn max_frontmost_rgb_distance(
    hotspots: &[Hotspot],
    distance: impl Fn(&Hotspot) -> Option<f32>,
) -> Option<f32> {
    hotspots
        .iter()
        .filter_map(distance)
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

fn mean_missing_center_nearest_rgb_distance(
    hotspots: &[Hotspot],
    distance: impl Fn(&Hotspot) -> Option<f32>,
) -> Option<f32> {
    let (sum, count) = hotspots
        .iter()
        .filter(|hotspot| hotspot.frontmost_visible.is_none())
        .filter_map(distance)
        .fold((0.0, 0usize), |(sum, count), distance| {
            (sum + distance, count + 1)
        });
    (count > 0).then_some(sum / count as f32)
}

fn max_missing_center_nearest_rgb_distance(
    hotspots: &[Hotspot],
    distance: impl Fn(&Hotspot) -> Option<f32>,
) -> Option<f32> {
    hotspots
        .iter()
        .filter(|hotspot| hotspot.frontmost_visible.is_none())
        .filter_map(distance)
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

fn texture_sampling_distances(
    candidate: Option<&CandidateMatch>,
    actual: [u8; 4],
    expected: [u8; 4],
) -> Vec<TextureSamplingDistance> {
    candidate
        .into_iter()
        .flat_map(|candidate| candidate.base_texture_sampling_rgba.iter())
        .map(|sample| TextureSamplingDistance {
            mode: sample.mode,
            rgba: sample.rgba,
            actual_rgb_distance: rgb_distance(sample.rgba, actual),
            expected_rgb_distance: rgb_distance(sample.rgba, expected),
        })
        .collect()
}

fn texture_sampling_variant_summary<'a>(
    samples: impl Iterator<Item = &'a TextureSamplingDistance>,
) -> Vec<TextureSamplingVariantSummary> {
    #[derive(Default)]
    struct Accum {
        count: usize,
        actual_sum: f32,
        expected_sum: f32,
        actual_max: f32,
        expected_max: f32,
        actual_closer: usize,
        expected_closer: usize,
        tied: usize,
        expected_minus_actual_sum: f32,
    }

    let mut by_mode = BTreeMap::<&'static str, Accum>::new();
    for sample in samples {
        let entry = by_mode.entry(sample.mode).or_default();
        entry.count += 1;
        entry.actual_sum += sample.actual_rgb_distance;
        entry.expected_sum += sample.expected_rgb_distance;
        entry.actual_max = entry.actual_max.max(sample.actual_rgb_distance);
        entry.expected_max = entry.expected_max.max(sample.expected_rgb_distance);
        entry.expected_minus_actual_sum +=
            sample.expected_rgb_distance - sample.actual_rgb_distance;
        match sample
            .actual_rgb_distance
            .partial_cmp(&sample.expected_rgb_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
        {
            std::cmp::Ordering::Less => entry.actual_closer += 1,
            std::cmp::Ordering::Greater => entry.expected_closer += 1,
            std::cmp::Ordering::Equal => entry.tied += 1,
        }
    }
    by_mode
        .into_iter()
        .filter(|(_, entry)| entry.count > 0)
        .map(|(mode, entry)| TextureSamplingVariantSummary {
            mode,
            count: entry.count,
            actual_mean_rgb_distance: entry.actual_sum / entry.count as f32,
            expected_mean_rgb_distance: entry.expected_sum / entry.count as f32,
            actual_max_rgb_distance: entry.actual_max,
            expected_max_rgb_distance: entry.expected_max,
            actual_closer: entry.actual_closer,
            expected_closer: entry.expected_closer,
            tied: entry.tied,
            mean_expected_minus_actual: entry.expected_minus_actual_sum / entry.count as f32,
        })
        .collect()
}

fn subpixel_improved_count(
    hotspots: &[Hotspot],
    subpixel: impl Fn(&Hotspot) -> Option<&SubpixelMatch>,
) -> usize {
    hotspots
        .iter()
        .filter_map(subpixel)
        .filter(|matched| matched.improvement.is_some_and(|improvement| improvement > 0.0))
        .count()
}

fn subpixel_improved_same_triangle_count(
    hotspots: &[Hotspot],
    subpixel: impl Fn(&Hotspot) -> Option<&SubpixelMatch>,
) -> usize {
    hotspots
        .iter()
        .filter(|hotspot| {
            let Some(matched) = subpixel(hotspot) else {
                return false;
            };
            matched.improvement.is_some_and(|improvement| improvement > 0.0)
                && same_surface_triangle(hotspot.frontmost_visible.as_ref(), Some(&matched.candidate))
        })
        .count()
}

fn subpixel_improved_different_triangle_count(
    hotspots: &[Hotspot],
    subpixel: impl Fn(&Hotspot) -> Option<&SubpixelMatch>,
) -> usize {
    hotspots
        .iter()
        .filter(|hotspot| {
            let Some(matched) = subpixel(hotspot) else {
                return false;
            };
            matched.improvement.is_some_and(|improvement| improvement > 0.0)
                && !same_surface_triangle(
                    hotspot.frontmost_visible.as_ref(),
                    Some(&matched.candidate),
                )
        })
        .count()
}

fn mean_subpixel_distance(
    hotspots: &[Hotspot],
    subpixel: impl Fn(&Hotspot) -> Option<&SubpixelMatch>,
) -> Option<f32> {
    let (sum, count) = hotspots
        .iter()
        .filter_map(subpixel)
        .fold((0.0, 0usize), |(sum, count), matched| {
            (sum + matched.rgb_distance, count + 1)
        });
    (count > 0).then_some(sum / count as f32)
}

fn mean_subpixel_improvement(
    hotspots: &[Hotspot],
    subpixel: impl Fn(&Hotspot) -> Option<&SubpixelMatch>,
) -> Option<f32> {
    let (sum, count) = hotspots
        .iter()
        .filter_map(subpixel)
        .filter_map(|matched| matched.improvement)
        .fold((0.0, 0usize), |(sum, count), improvement| {
            (sum + improvement, count + 1)
        });
    (count > 0).then_some(sum / count as f32)
}

fn mean_subpixel_sample_distance(
    hotspots: &[Hotspot],
    subpixel: impl Fn(&Hotspot) -> Option<&SubpixelMatch>,
) -> Option<f32> {
    let (sum, count) = hotspots
        .iter()
        .filter_map(subpixel)
        .fold((0.0, 0usize), |(sum, count), matched| {
            let dx = matched.sample[0] - 0.5;
            let dy = matched.sample[1] - 0.5;
            (sum + (dx * dx + dy * dy).sqrt(), count + 1)
        });
    (count > 0).then_some(sum / count as f32)
}

fn depth_near_later_improved_count(
    hotspots: &[Hotspot],
    depth_near_distance: impl Fn(&Hotspot) -> Option<f32>,
    frontmost_distance: impl Fn(&Hotspot) -> Option<f32>,
) -> usize {
    hotspots
        .iter()
        .filter(|hotspot| {
            matches!(
                (depth_near_distance(hotspot), frontmost_distance(hotspot)),
                (Some(depth_near), Some(frontmost)) if depth_near < frontmost
            )
        })
        .count()
}

fn depth_near_later_mean_improvement(
    hotspots: &[Hotspot],
    depth_near_distance: impl Fn(&Hotspot) -> Option<f32>,
    frontmost_distance: impl Fn(&Hotspot) -> Option<f32>,
) -> Option<f32> {
    let (sum, count) = hotspots.iter().fold((0.0, 0usize), |(sum, count), hotspot| {
        match (depth_near_distance(hotspot), frontmost_distance(hotspot)) {
            (Some(depth_near), Some(frontmost)) => (sum + (frontmost - depth_near), count + 1),
            _ => (sum, count),
        }
    });
    (count > 0).then_some(sum / count as f32)
}

fn strict_frontmost_improved_count(
    hotspots: &[Hotspot],
    strict_distance: impl Fn(&Hotspot) -> Option<f32>,
    loose_distance: impl Fn(&Hotspot) -> Option<f32>,
) -> usize {
    hotspots
        .iter()
        .filter(|hotspot| {
            matches!(
                (strict_distance(hotspot), loose_distance(hotspot)),
                (Some(strict), Some(loose)) if strict < loose
            )
        })
        .count()
}

fn subpixel_sample_summaries(
    hotspots: &[Hotspot],
    target: impl Fn(&Hotspot) -> [u8; 4],
    center_distance: impl Fn(&Hotspot) -> Option<f32>,
) -> Vec<SubpixelSampleSummary> {
    #[derive(Clone, Debug, Default)]
    struct Aggregate {
        sample: [f32; 2],
        visible_count: usize,
        same_triangle_count: usize,
        improved_count: usize,
        improved_same_triangle_count: usize,
        improved_different_triangle_count: usize,
        distance_sum: f32,
        improvement_sum: f32,
        improvement_count: usize,
        sample_distance_sum: f32,
    }

    let mut aggregates = BTreeMap::<[i32; 2], Aggregate>::new();
    for hotspot in hotspots {
        let target = target(hotspot);
        let center_distance = center_distance(hotspot);
        for candidate in &hotspot.subpixel_visible_candidates {
            let key = subpixel_sample_key(candidate.sample);
            let aggregate = aggregates.entry(key).or_insert_with(|| Aggregate {
                sample: candidate.sample,
                ..Aggregate::default()
            });
            let rgb_distance = rgb_distance(candidate.candidate.cpu_base_color_rgba, target);
            let same_triangle = same_surface_triangle(
                hotspot.frontmost_visible.as_ref(),
                Some(&candidate.candidate),
            );
            aggregate.visible_count += 1;
            aggregate.distance_sum += rgb_distance;
            aggregate.sample_distance_sum += sample_distance_from_center(candidate.sample);
            if same_triangle {
                aggregate.same_triangle_count += 1;
            }
            if let Some(improvement) = center_distance.map(|center| center - rgb_distance) {
                aggregate.improvement_sum += improvement;
                aggregate.improvement_count += 1;
                if improvement > 0.0 {
                    aggregate.improved_count += 1;
                    if same_triangle {
                        aggregate.improved_same_triangle_count += 1;
                    } else {
                        aggregate.improved_different_triangle_count += 1;
                    }
                }
            }
        }
    }

    let mut summaries = aggregates
        .into_values()
        .filter(|aggregate| aggregate.visible_count > 0)
        .map(|aggregate| SubpixelSampleSummary {
            sample: aggregate.sample,
            visible_count: aggregate.visible_count,
            same_triangle_count: aggregate.same_triangle_count,
            improved_count: aggregate.improved_count,
            improved_same_triangle_count: aggregate.improved_same_triangle_count,
            improved_different_triangle_count: aggregate.improved_different_triangle_count,
            mean_cpu_base_color_rgb_distance: aggregate.distance_sum
                / aggregate.visible_count as f32,
            mean_cpu_base_color_improvement: (aggregate.improvement_count > 0)
                .then_some(aggregate.improvement_sum / aggregate.improvement_count as f32),
            mean_sample_distance_from_center: aggregate.sample_distance_sum
                / aggregate.visible_count as f32,
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        left.mean_cpu_base_color_rgb_distance
            .partial_cmp(&right.mean_cpu_base_color_rgb_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.improved_count.cmp(&left.improved_count))
            .then_with(|| {
                left.mean_sample_distance_from_center
                    .partial_cmp(&right.mean_sample_distance_from_center)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| sample_order(left.sample, right.sample))
    });
    summaries
}

fn subpixel_sample_key(sample: [f32; 2]) -> [i32; 2] {
    [
        (sample[0] * 1_000_000.0).round() as i32,
        (sample[1] * 1_000_000.0).round() as i32,
    ]
}

fn sample_distance_from_center(sample: [f32; 2]) -> f32 {
    let dx = sample[0] - 0.5;
    let dy = sample[1] - 0.5;
    (dx * dx + dy * dy).sqrt()
}

fn sample_order(left: [f32; 2], right: [f32; 2]) -> std::cmp::Ordering {
    left[1]
        .partial_cmp(&right[1])
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            left[0]
                .partial_cmp(&right[0])
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn mean_frontmost_texture_gradient(hotspots: &[Hotspot]) -> Option<f32> {
    let (sum, count) = hotspots
        .iter()
        .filter_map(|hotspot| {
            hotspot
                .frontmost_visible
                .as_ref()
                .and_then(|frontmost| frontmost.base_texture_local_rgb_gradient)
        })
        .fold((0.0, 0usize), |(sum, count), gradient| {
            (sum + gradient, count + 1)
        });
    (count > 0).then_some(sum / count as f32)
}

fn max_frontmost_texture_gradient(hotspots: &[Hotspot]) -> Option<f32> {
    hotspots
        .iter()
        .filter_map(|hotspot| {
            hotspot
                .frontmost_visible
                .as_ref()
                .and_then(|frontmost| frontmost.base_texture_local_rgb_gradient)
        })
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

fn frontmost_texture_gradient_gte(hotspots: &[Hotspot], threshold: f32) -> usize {
    hotspots
        .iter()
        .filter_map(|hotspot| {
            hotspot
                .frontmost_visible
                .as_ref()
                .and_then(|frontmost| frontmost.base_texture_local_rgb_gradient)
        })
        .filter(|gradient| *gradient >= threshold)
        .count()
}

fn mean_frontmost_edge_distance(hotspots: &[Hotspot]) -> Option<f32> {
    let (sum, count) = hotspots
        .iter()
        .filter_map(|hotspot| {
            hotspot
                .frontmost_visible
                .as_ref()
                .map(|frontmost| frontmost.edge_distance_pixels)
        })
        .fold((0.0, 0usize), |(sum, count), distance| {
            (sum + distance, count + 1)
        });
    (count > 0).then_some(sum / count as f32)
}

fn frontmost_edge_distance_lte(hotspots: &[Hotspot], threshold: f32) -> usize {
    hotspots
        .iter()
        .filter_map(|hotspot| hotspot.frontmost_visible.as_ref())
        .filter(|frontmost| frontmost.edge_distance_pixels <= threshold)
        .count()
}

fn frontmost_nearest_edge_counts(hotspots: &[Hotspot]) -> Vec<EdgeBucketCount> {
    let mut counts = BTreeMap::<EdgeBucketKey, (usize, f32)>::new();
    for frontmost in hotspots
        .iter()
        .filter_map(|hotspot| hotspot.frontmost_visible.as_ref())
    {
        let key = EdgeBucketKey {
            node: frontmost.node,
            mesh: frontmost.mesh,
            primitive: frontmost.primitive,
            pass: frontmost.pass,
            material: frontmost.material,
            material_name: frontmost.material_name.clone(),
            triangle: frontmost.triangle,
            edge: frontmost.nearest_edge,
            edge_indices: frontmost.nearest_edge_indices,
        };
        let (count, distance_sum) = counts.entry(key).or_default();
        *count += 1;
        *distance_sum += frontmost.edge_distance_pixels;
    }
    let mut buckets = counts
        .into_iter()
        .map(|(key, (count, distance_sum))| EdgeBucketCount {
            node: key.node,
            mesh: key.mesh,
            primitive: key.primitive,
            pass: key.pass,
            material: key.material,
            material_name: key.material_name,
            triangle: key.triangle,
            edge: key.edge,
            edge_indices: key.edge_indices,
            count,
            mean_edge_distance_pixels: distance_sum / count as f32,
        })
        .collect::<Vec<_>>();
    buckets.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.node.cmp(&right.node))
            .then_with(|| left.mesh.cmp(&right.mesh))
            .then_with(|| left.primitive.cmp(&right.primitive))
            .then_with(|| left.triangle.cmp(&right.triangle))
            .then_with(|| left.edge.cmp(&right.edge))
    });
    buckets
}

fn offset_counts<'a>(candidates: impl Iterator<Item = &'a CandidateMatch>) -> Vec<OffsetCount> {
    let mut counts = BTreeMap::<[i32; 2], usize>::new();
    for candidate in candidates {
        *counts.entry(candidate.sample_offset).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(offset, count)| OffsetCount { offset, count })
        .collect()
}

fn build_surfaces(
    loaded: &LoadedVrm,
    expression_effects: &GltfExpressionRenderEffects,
    options: &Options,
) -> Result<Vec<Surface>, Box<dyn Error>> {
    let world_matrices = vrm_rs::evaluated_world_matrices(loaded)?;
    let orientation = Mat4::from_rotation_y(std::f32::consts::PI);
    let mut surfaces = Vec::new();

    for (node_index, node) in loaded.scene.nodes.iter().enumerate() {
        let Some(mesh_index) = node.mesh else {
            continue;
        };
        let Some(mesh) = loaded.meshes.get(mesh_index) else {
            continue;
        };
        let node_world = world_matrices
            .get(node_index)
            .copied()
            .unwrap_or(node.world_matrix);
        let world = orientation * node_world;
        let skin_matrices = node
            .skin
            .and_then(|skin| loaded.skins.get(skin))
            .map(|skin| skin.joint_matrices(&loaded.scene, &world_matrices, orientation));
        let morph_weights = expression_effects.active_morph_weights(node_index, node, mesh);

        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            let Some(vertices) =
                primitive.transformed_vertices(&morph_weights, world, skin_matrices.as_deref())
            else {
                continue;
            };
            let material_name = loaded
                .material_display_name(primitive.material)
                .map(str::to_owned);
            let uv_transforms = loaded.expression_material_uv_transforms(
                primitive.material,
                options.mtoon_time,
                expression_effects,
            );
            let base_uv_transform = uv_transforms.base;
            let base_texture = loaded.material_base_texture_rgba8_image(primitive.material);
            let shading = loaded.expression_material_shading_plan(
                primitive.material,
                GltfMaterialShadingOptions::default(),
                expression_effects,
            );
            let base_policy = capture_material_policy(loaded, primitive.material);
            let indices = primitive_indices(primitive.indices.as_slice(), vertices.len());
            surfaces.push(Surface {
                draw_index: 0,
                node: node_index,
                mesh: mesh_index,
                primitive: primitive_index,
                pass: "base",
                material: primitive.material,
                material_name: material_name.clone(),
                policy: base_policy,
                base_uv_transform,
                base_texture: base_texture.clone(),
                base_color: shading.base_color,
                base_color_alpha: shading.base_color[3],
                pbr_fallback: shading.pbr_fallback,
                edge_adjacency: edge_adjacency(&indices),
                indices,
                vertices: vertices.clone(),
            });
            if options.disable_outlines {
                continue;
            }
            let Some(outline) =
                loaded.expression_mtoon_outline_plan(primitive.material, expression_effects)
            else {
                continue;
            };
            let width_texture = loaded.material_outline_width_rgba8_image(primitive.material);
            let outline_scale = GltfOutlineScale::new(
                outline.width_mode,
                camera_view(options),
                projection_y_scale(),
            );
            let outline_vertices = if options.expand_outlines {
                let Some(outline_vertices) = primitive.outline_vertices(
                    &morph_weights,
                    GltfOutlineVertexSettings {
                        base_width: outline.width_factor * options.outline_width_scale,
                        scale: outline_scale,
                        width_texture: width_texture.as_ref(),
                        width_transform: uv_transforms.outline_width,
                        width_texture_origin: Rgba8SamplingOrigin::TopLeft,
                    },
                    world,
                    skin_matrices.as_deref(),
                ) else {
                    continue;
                };
                outline_vertices
            } else {
                vertices.clone()
            };
            let indices = primitive_indices(primitive.indices.as_slice(), outline_vertices.len());
            surfaces.push(Surface {
                draw_index: 0,
                node: node_index,
                mesh: mesh_index,
                primitive: primitive_index,
                pass: "outline",
                material: primitive.material,
                material_name,
                policy: outline_material_policy(base_policy),
                base_uv_transform,
                base_texture,
                base_color: [0.0, 0.0, 0.0, 1.0],
                base_color_alpha: 1.0,
                pbr_fallback: false,
                edge_adjacency: edge_adjacency(&indices),
                indices,
                vertices: outline_vertices,
            });
        }
    }

    if surfaces.is_empty() {
        return Err("no drawable surfaces were found".into());
    }
    surfaces.sort_by_key(|surface| surface.policy.render_order);
    for (draw_index, surface) in surfaces.iter_mut().enumerate() {
        surface.draw_index = draw_index;
    }
    Ok(surfaces)
}

fn candidates_for_pixel(
    x: usize,
    y: usize,
    surfaces: &[Surface],
    view_projection: Mat4,
    width: usize,
    height: usize,
    candidate_limit: usize,
    hit_radius: i32,
    sample_center: [f32; 2],
) -> Vec<HitCandidate> {
    let mut candidates = surfaces
        .iter()
        .flat_map(|surface| {
            (-hit_radius..=hit_radius).flat_map(move |dy| {
                (-hit_radius..=hit_radius).flat_map(move |dx| {
                    let point = [
                        x as f32 + sample_center[0] + dx as f32,
                        y as f32 + sample_center[1] + dy as f32,
                    ];
                    surface_candidates(surface, view_projection, point, [dx, dy], width, height)
                })
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.sample_distance
            .partial_cmp(&right.sample_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.depth
                    .partial_cmp(&right.depth)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.material.cmp(&right.material))
            .then_with(|| left.node.cmp(&right.node))
            .then_with(|| left.mesh.cmp(&right.mesh))
            .then_with(|| left.primitive.cmp(&right.primitive))
            .then_with(|| left.triangle.cmp(&right.triangle))
    });
    candidates.dedup_by(|left, right| {
        left.node == right.node
            && left.mesh == right.mesh
            && left.primitive == right.primitive
            && left.triangle == right.triangle
            && left.sample_offset == right.sample_offset
    });
    candidates.truncate(candidate_limit);
    candidates
}

fn nearest_candidate_match(
    candidates: &[HitCandidate],
    linear_uv: [f32; 2],
) -> Option<CandidateMatch> {
    nearest_candidate_match_by(candidates, linear_uv, |_| true)
}

fn nearest_visible_candidate_match(
    candidates: &[HitCandidate],
    linear_uv: [f32; 2],
) -> Option<CandidateMatch> {
    nearest_candidate_match_by(candidates, linear_uv, |candidate| {
        candidate.visible_by_policy
    })
}

fn nearest_candidate_match_by(
    candidates: &[HitCandidate],
    linear_uv: [f32; 2],
    filter: impl Fn(&HitCandidate) -> bool,
) -> Option<CandidateMatch> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| filter(candidate))
        .map(|(candidate_index, candidate)| {
            candidate_match(
                candidate_index,
                candidate,
                uv_distance(candidate.base_uv, linear_uv),
            )
        })
        .min_by(|left, right| {
            left.base_uv_distance
                .partial_cmp(&right.base_uv_distance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.candidate_index.cmp(&right.candidate_index))
        })
}

fn frontmost_any_candidate_match(candidates: &[HitCandidate]) -> Option<CandidateMatch> {
    frontmost_candidate_match_by(candidates, |_| true)
}

fn frontmost_alpha_visible_candidate_match(candidates: &[HitCandidate]) -> Option<CandidateMatch> {
    frontmost_candidate_match_by(candidates, |candidate| candidate.visible_by_alpha_policy)
}

fn frontmost_visible_candidate_match(candidates: &[HitCandidate]) -> Option<CandidateMatch> {
    frontmost_candidate_match_by(candidates, |candidate| candidate.visible_by_policy)
}

fn strict_frontmost_visible_candidate_match(candidates: &[HitCandidate]) -> Option<CandidateMatch> {
    frontmost_candidate_match_by(candidates, |candidate| {
        candidate.visible_by_policy && candidate.min_barycentric >= 0.0
    })
}

fn depth_near_later_visible_candidate_match(
    candidates: &[HitCandidate],
    frontmost: Option<&CandidateMatch>,
    depth_epsilon: f32,
) -> Option<CandidateMatch> {
    let frontmost = frontmost?;
    let min_depth = frontmost.depth - depth_epsilon;
    let max_depth = frontmost.depth + depth_epsilon;
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.sample_offset == [0, 0]
                && candidate.depth >= -1.0
                && candidate.visible_by_policy
                && candidate.draw_index > frontmost.draw_index
                && (min_depth..=max_depth).contains(&candidate.depth)
        })
        .map(|(candidate_index, candidate)| {
            candidate_match(
                candidate_index,
                candidate,
                uv_distance(candidate.base_uv, frontmost.base_uv),
            )
        })
        .max_by(|left, right| {
            left.draw_index
                .cmp(&right.draw_index)
                .then_with(|| {
                    (right.depth - frontmost.depth)
                        .abs()
                        .partial_cmp(&(left.depth - frontmost.depth).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
}

fn nearest_sample_any_frontmost_candidate_match(
    candidates: &[HitCandidate],
) -> Option<CandidateMatch> {
    nearest_sample_frontmost_candidate_match_by(candidates, |_| true)
}

fn nearest_sample_visible_frontmost_candidate_match(
    candidates: &[HitCandidate],
) -> Option<CandidateMatch> {
    nearest_sample_frontmost_candidate_match_by(candidates, |candidate| candidate.visible_by_policy)
}

fn frontmost_candidate_match_by(
    candidates: &[HitCandidate],
    filter: impl Fn(&HitCandidate) -> bool,
) -> Option<CandidateMatch> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.sample_offset == [0, 0] && candidate.depth >= -1.0 && filter(candidate)
        })
        .map(|(candidate_index, candidate)| candidate_match(candidate_index, candidate, 0.0))
        .min_by(|left, right| {
            left.depth
                .partial_cmp(&right.depth)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.draw_index.cmp(&left.draw_index))
        })
}

fn nearest_sample_frontmost_candidate_match_by(
    candidates: &[HitCandidate],
    filter: impl Fn(&HitCandidate) -> bool,
) -> Option<CandidateMatch> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.depth >= -1.0 && filter(candidate))
        .map(|(candidate_index, candidate)| candidate_match(candidate_index, candidate, 0.0))
        .min_by(|left, right| {
            left.sample_distance
                .partial_cmp(&right.sample_distance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    left.depth
                        .partial_cmp(&right.depth)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| right.draw_index.cmp(&left.draw_index))
        })
}

fn subpixel_frontmost_visible_candidates(
    x: usize,
    y: usize,
    surfaces: &[Surface],
    view_projection: Mat4,
    width: usize,
    height: usize,
    steps: usize,
) -> Vec<SubpixelCandidate> {
    let steps = steps.max(1);
    (0..steps)
        .flat_map(|row| {
            (0..steps).filter_map(move |column| {
                let sample = [
                    (column as f32 + 0.5) / steps as f32,
                    (row as f32 + 0.5) / steps as f32,
                ];
                let point = [x as f32 + sample[0], y as f32 + sample[1]];
                let candidates = surfaces
                    .iter()
                    .flat_map(|surface| {
                        surface_candidates(
                            surface,
                            view_projection,
                            point,
                            [0, 0],
                            width,
                            height,
                        )
                    })
                    .collect::<Vec<_>>();
                frontmost_visible_candidate_match(&candidates)
                    .map(|candidate| SubpixelCandidate { sample, candidate })
            })
        })
        .collect()
}

#[derive(Clone, Debug, Serialize)]
struct SubpixelCandidate {
    sample: [f32; 2],
    candidate: CandidateMatch,
}

fn best_subpixel_match(
    candidates: &[SubpixelCandidate],
    target: [u8; 4],
    center_distance: Option<f32>,
) -> Option<SubpixelMatch> {
    candidates
        .iter()
        .map(|candidate| {
            let rgb_distance = rgb_distance(candidate.candidate.cpu_base_color_rgba, target);
            SubpixelMatch {
                sample: candidate.sample,
                rgb_distance,
                center_rgb_distance: center_distance,
                improvement: center_distance.map(|center| center - rgb_distance),
                candidate: candidate.candidate.clone(),
            }
        })
        .min_by(|left, right| {
            left.rgb_distance
                .partial_cmp(&right.rgb_distance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    left.sample[1]
                        .partial_cmp(&right.sample[1])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    left.sample[0]
                        .partial_cmp(&right.sample[0])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
}

fn candidate_match(
    candidate_index: usize,
    candidate: &HitCandidate,
    base_uv_distance: f32,
) -> CandidateMatch {
    CandidateMatch {
        candidate_index,
        draw_index: candidate.draw_index,
        base_uv_distance,
        pass: candidate.pass,
        material: candidate.material,
        material_name: candidate.material_name.clone(),
        policy: candidate.policy,
        sample_offset: candidate.sample_offset,
        sample_distance: candidate.sample_distance,
        node: candidate.node,
        mesh: candidate.mesh,
        primitive: candidate.primitive,
        triangle: candidate.triangle,
        indices: candidate.indices,
        depth: candidate.depth,
        min_barycentric: candidate.min_barycentric,
        edge_distance_pixels: candidate.edge_distance_pixels,
        nearest_edge: candidate.nearest_edge,
        nearest_edge_indices: candidate.nearest_edge_indices,
        nearest_edge_neighbor_triangles: candidate.nearest_edge_neighbor_triangles.clone(),
        front_facing: candidate.front_facing,
        alpha: candidate.alpha,
        visible_by_cull_policy: candidate.visible_by_cull_policy,
        visible_by_alpha_policy: candidate.visible_by_alpha_policy,
        visible_by_policy: candidate.visible_by_policy,
        base_uv: candidate.base_uv,
        base_texture_rgba: candidate.base_texture_rgba,
        cpu_base_color_rgba: candidate.cpu_base_color_rgba,
        base_texture_sampling_rgba: candidate.base_texture_sampling_rgba.clone(),
        base_texture_local_rgb_gradient: candidate.base_texture_local_rgb_gradient,
    }
}

fn surface_candidates(
    surface: &Surface,
    view_projection: Mat4,
    point: [f32; 2],
    sample_offset: [i32; 2],
    width: usize,
    height: usize,
) -> Vec<HitCandidate> {
    surface
        .indices
        .chunks_exact(3)
        .enumerate()
        .filter_map(|(triangle, indices)| {
            let [ia, ib, ic] = [indices[0], indices[1], indices[2]];
            let a = project(
                surface.vertices.get(ia as usize)?,
                view_projection,
                width,
                height,
            )?;
            let b = project(
                surface.vertices.get(ib as usize)?,
                view_projection,
                width,
                height,
            )?;
            let c = project(
                surface.vertices.get(ic as usize)?,
                view_projection,
                width,
                height,
            )?;
            let barycentric = barycentric(point, a.screen, b.screen, c.screen)?;
            let raw_uv = interpolate_perspective_correct_uv(barycentric, a, b, c);
            let base_uv = transform_tex_coord_0(raw_uv, surface.base_uv_transform);
            let base_texture_rgba_linear = surface
                .base_texture
                .as_ref()
                .map(|texture| {
                    texture.sample_rgba_repeat_linear(base_uv, Rgba8SamplingOrigin::TopLeft)
                });
            let base_texture_rgba = base_texture_rgba_linear.map(|rgba| rgba.map(quantize_unorm8));
            let texture_color = base_texture_rgba_linear.unwrap_or([1.0, 1.0, 1.0, 1.0]);
            let base_texture_sampling_rgba = surface
                .base_texture
                .as_ref()
                .map(|texture| texture_sampling_colors(texture, base_uv))
                .unwrap_or_default();
            let base_texture_local_rgb_gradient = surface
                .base_texture
                .as_ref()
                .map(|texture| base_texture_local_rgb_gradient(texture, base_uv));
            let vertex_color = if surface.pbr_fallback {
                interpolate_vertex_color(
                    barycentric,
                    surface.vertices.get(ia as usize)?,
                    surface.vertices.get(ib as usize)?,
                    surface.vertices.get(ic as usize)?,
                )
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            let cpu_base_color_rgba =
                multiply_rgba(multiply_rgba(surface.base_color, vertex_color), texture_color)
                    .map(quantize_unorm8);
            let vertex_alpha = if surface.pbr_fallback {
                vertex_color[3]
            } else {
                1.0
            };
            let texture_alpha = texture_color[3];
            let alpha = surface.base_color_alpha * vertex_alpha * texture_alpha;
            let signed_area = signed_area(a.screen, b.screen, c.screen);
            let front_facing = signed_area < 0.0;
            let visible_by_cull_policy =
                visible_by_cull_policy(surface.policy.cull_mode, front_facing);
            let visible_by_alpha_policy = visible_by_alpha_policy(surface.policy, alpha);
            let nearest_edge = nearest_triangle_edge(point, a.screen, b.screen, c.screen);
            let edge_indices = triangle_edge_indices([ia, ib, ic], nearest_edge.edge);
            let nearest_edge_neighbor_triangles =
                edge_neighbors(&surface.edge_adjacency, edge_indices, triangle);
            Some(HitCandidate {
                draw_index: surface.draw_index,
                node: surface.node,
                mesh: surface.mesh,
                primitive: surface.primitive,
                pass: surface.pass,
                triangle,
                indices: [ia, ib, ic],
                material: surface.material,
                material_name: surface.material_name.clone(),
                policy: surface.policy,
                sample_offset,
                sample_distance: ((sample_offset[0] * sample_offset[0]
                    + sample_offset[1] * sample_offset[1])
                    as f32)
                    .sqrt(),
                depth: interpolate_scalar(barycentric, a.depth, b.depth, c.depth),
                barycentric,
                min_barycentric: barycentric
                    .into_iter()
                    .fold(f32::INFINITY, |left, right| left.min(right)),
                edge_distance_pixels: nearest_edge.distance_pixels,
                nearest_edge: nearest_edge.edge,
                nearest_edge_indices: edge_indices,
                nearest_edge_neighbor_triangles,
                raw_uv,
                base_uv,
                base_texture_rgba,
                cpu_base_color_rgba,
                base_texture_sampling_rgba,
                base_texture_local_rgb_gradient,
                screen: [a.screen, b.screen, c.screen],
                front_facing,
                alpha,
                visible_by_cull_policy,
                visible_by_alpha_policy,
                visible_by_policy: visible_by_cull_policy && visible_by_alpha_policy,
            })
        })
        .collect()
}

fn capture_material_policy(loaded: &LoadedVrm, material: Option<usize>) -> MaterialPolicyReport {
    material_policy_report(capture_material_plan(loaded, material))
}

fn capture_material_plan(
    loaded: &LoadedVrm,
    material: Option<usize>,
) -> RendererMaterialPipelinePlan {
    let material_ref = material.map(MaterialRef);
    let gltf_override = material
        .and_then(|index| loaded.gltf_materials.get(index))
        .map(|gltf| GltfMaterialPipelineOverride {
            alpha_mode: gltf_alpha_mode(gltf.alpha_mode),
            alpha_cutoff: gltf.alpha_cutoff,
            double_sided: gltf.double_sided,
        });
    renderer_material_pipeline_plan(
        loaded.model().document(),
        material_ref,
        MtoonMaterializationOptions::default(),
        gltf_override,
    )
}

fn gltf_alpha_mode(mode: GltfAlphaMode) -> GltfMaterialAlphaMode {
    match mode {
        GltfAlphaMode::Opaque => GltfMaterialAlphaMode::Opaque,
        GltfAlphaMode::Mask => GltfMaterialAlphaMode::Mask,
        GltfAlphaMode::Blend => GltfMaterialAlphaMode::Blend,
    }
}

fn material_policy_report(plan: RendererMaterialPipelinePlan) -> MaterialPolicyReport {
    MaterialPolicyReport {
        render_order: plan.render_order,
        phase_order: plan.phase_order,
        cull_mode: cull_mode_name(plan.cull_mode),
        alpha_mode: alpha_mode_name(plan.alpha_mode),
        depth_write: plan.depth_write,
        blend: plan.blend,
        alpha_cutoff: plan.alpha_cutoff,
    }
}

fn outline_material_policy(base_policy: MaterialPolicyReport) -> MaterialPolicyReport {
    MaterialPolicyReport {
        render_order: base_policy.render_order + 1,
        phase_order: base_policy.phase_order + 1,
        cull_mode: "front",
        alpha_mode: "opaque",
        depth_write: true,
        blend: false,
        alpha_cutoff: 0.5,
    }
}

fn cull_mode_name(mode: RendererMaterialCullMode) -> &'static str {
    match mode {
        RendererMaterialCullMode::Off => "off",
        RendererMaterialCullMode::Front => "front",
        RendererMaterialCullMode::Back => "back",
    }
}

fn alpha_mode_name(mode: RendererMaterialAlphaMode) -> &'static str {
    match mode {
        RendererMaterialAlphaMode::Opaque => "opaque",
        RendererMaterialAlphaMode::Mask => "mask",
        RendererMaterialAlphaMode::Blend => "blend",
    }
}

fn visible_by_cull_policy(cull_mode: &'static str, front_facing: bool) -> bool {
    match cull_mode {
        "off" => true,
        "front" => !front_facing,
        "back" => front_facing,
        _ => true,
    }
}

fn visible_by_alpha_policy(policy: MaterialPolicyReport, alpha: f32) -> bool {
    policy.alpha_mode != "mask" || alpha >= policy.alpha_cutoff
}

fn project(
    vertex: &GltfTransformedVertex,
    view_projection: Mat4,
    width: usize,
    height: usize,
) -> Option<ProjectedVertex> {
    let clip = view_projection * vertex.position.extend(1.0);
    if clip.w.abs() <= f32::EPSILON {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(ProjectedVertex {
        screen: [
            (ndc.x * 0.5 + 0.5) * width as f32,
            (0.5 - ndc.y * 0.5) * height as f32,
        ],
        depth: ndc.z,
        uv: vertex.tex_coord_0,
        reciprocal_w: 1.0 / clip.w,
    })
}

fn barycentric(point: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> Option<[f32; 3]> {
    let denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if denominator.abs() <= 1.0e-5 {
        return None;
    }
    let w0 = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / denominator;
    let w1 = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / denominator;
    let w2 = 1.0 - w0 - w1;
    let epsilon = -1.0e-4;
    (w0 >= epsilon && w1 >= epsilon && w2 >= epsilon).then_some([w0, w1, w2])
}

fn view_projection(width: usize, height: usize, options: &Options) -> Mat4 {
    let view = camera_view(options);
    let projection = Mat4::perspective_rh(
        30.0_f32.to_radians(),
        width as f32 / height as f32,
        0.1,
        20.0,
    );
    projection * view
}

fn camera_view(options: &Options) -> Mat4 {
    let camera_eye = Vec3::new(0.0, options.camera_y, -options.camera_z);
    Mat4::look_at_rh(camera_eye, Vec3::new(0.0, options.target_y, 0.0), Vec3::Y)
}

fn projection_y_scale() -> f32 {
    1.0 / (0.5 * 30.0_f32.to_radians()).tan()
}

fn primitive_indices(indices: &[u32], vertex_count: usize) -> Vec<u32> {
    if indices.is_empty() {
        (0..u32::try_from(vertex_count).unwrap_or(0)).collect()
    } else {
        indices.to_vec()
    }
}

fn edge_adjacency(indices: &[u32]) -> BTreeMap<[u32; 2], Vec<usize>> {
    let mut adjacency = BTreeMap::<[u32; 2], Vec<usize>>::new();
    for (triangle, indices) in indices.chunks_exact(3).enumerate() {
        for edge in 0..3 {
            adjacency
                .entry(normalized_edge(triangle_edge_indices(
                    [indices[0], indices[1], indices[2]],
                    edge,
                )))
                .or_default()
                .push(triangle);
        }
    }
    adjacency
}

fn edge_neighbors(
    adjacency: &BTreeMap<[u32; 2], Vec<usize>>,
    edge_indices: [u32; 2],
    triangle: usize,
) -> Vec<usize> {
    adjacency
        .get(&normalized_edge(edge_indices))
        .map(|triangles| {
            triangles
                .iter()
                .copied()
                .filter(|neighbor| *neighbor != triangle)
                .collect()
        })
        .unwrap_or_default()
}

fn normalized_edge(edge_indices: [u32; 2]) -> [u32; 2] {
    if edge_indices[0] <= edge_indices[1] {
        edge_indices
    } else {
        [edge_indices[1], edge_indices[0]]
    }
}

fn parse_expression_args(args: &[String]) -> Result<Vec<(String, f32)>, Box<dyn Error>> {
    args.iter()
        .map(|arg| {
            let Some((name, value)) = arg.split_once('=') else {
                return Err(format!("invalid expression '{arg}', expected name=weight").into());
            };
            let weight = value
                .parse::<f32>()
                .map_err(|err| format!("invalid expression weight in '{arg}': {err}"))?;
            Ok((name.to_owned(), weight))
        })
        .collect()
}

fn interpolate_uv(barycentric: [f32; 3], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> [f32; 2] {
    [
        barycentric[0] * a[0] + barycentric[1] * b[0] + barycentric[2] * c[0],
        barycentric[0] * a[1] + barycentric[1] * b[1] + barycentric[2] * c[1],
    ]
}

fn interpolate_perspective_correct_uv(
    barycentric: [f32; 3],
    a: ProjectedVertex,
    b: ProjectedVertex,
    c: ProjectedVertex,
) -> [f32; 2] {
    let weights = [
        barycentric[0] * a.reciprocal_w,
        barycentric[1] * b.reciprocal_w,
        barycentric[2] * c.reciprocal_w,
    ];
    let denominator = weights[0] + weights[1] + weights[2];
    if denominator.abs() <= f32::EPSILON {
        return interpolate_uv(barycentric, a.uv, b.uv, c.uv);
    }
    [
        (weights[0] * a.uv[0] + weights[1] * b.uv[0] + weights[2] * c.uv[0]) / denominator,
        (weights[0] * a.uv[1] + weights[1] * b.uv[1] + weights[2] * c.uv[1]) / denominator,
    ]
}

fn uv_distance(left: [f32; 2], right: [f32; 2]) -> f32 {
    ((left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2)).sqrt()
}

fn base_texture_local_rgb_gradient(texture: &CpuRgba8Image, uv: [f32; 2]) -> f32 {
    let center = texture.sample_rgba8_repeat_linear(uv, Rgba8SamplingOrigin::TopLeft);
    let du = 1.0 / texture.width.max(1) as f32;
    let dv = 1.0 / texture.height.max(1) as f32;
    [[du, 0.0], [-du, 0.0], [0.0, dv], [0.0, -dv]]
        .into_iter()
        .map(|offset| {
            texture.sample_rgba8_repeat_linear(
                [uv[0] + offset[0], uv[1] + offset[1]],
                Rgba8SamplingOrigin::TopLeft,
            )
        })
        .map(|sample| rgb_distance(center, sample))
        .fold(0.0, f32::max)
}

fn texture_sampling_colors(texture: &CpuRgba8Image, uv: [f32; 2]) -> Vec<TextureSamplingColor> {
    [
        (
            "linear_top_left_half_texel",
            texture
                .sample_rgba_repeat_linear(uv, Rgba8SamplingOrigin::TopLeft)
                .map(quantize_unorm8),
        ),
        (
            "linear_bottom_left_half_texel",
            texture
                .sample_rgba_repeat_linear(uv, Rgba8SamplingOrigin::BottomLeft)
                .map(quantize_unorm8),
        ),
        (
            "nearest_top_left",
            sample_rgba8_repeat_nearest(texture, uv, Rgba8SamplingOrigin::TopLeft),
        ),
        (
            "nearest_bottom_left",
            sample_rgba8_repeat_nearest(texture, uv, Rgba8SamplingOrigin::BottomLeft),
        ),
        (
            "linear_top_left_no_half_texel",
            sample_rgba8_repeat_linear_no_half(texture, uv, Rgba8SamplingOrigin::TopLeft),
        ),
        (
            "linear_bottom_left_no_half_texel",
            sample_rgba8_repeat_linear_no_half(texture, uv, Rgba8SamplingOrigin::BottomLeft),
        ),
    ]
    .into_iter()
    .map(|(mode, rgba)| TextureSamplingColor { mode, rgba })
    .collect()
}

fn sample_rgba8_repeat_nearest(
    texture: &CpuRgba8Image,
    uv: [f32; 2],
    origin: Rgba8SamplingOrigin,
) -> [u8; 4] {
    let x = (uv[0].rem_euclid(1.0) * texture.width as f32).floor() as i32;
    let y = match origin {
        Rgba8SamplingOrigin::TopLeft => (uv[1].rem_euclid(1.0) * texture.height as f32).floor(),
        Rgba8SamplingOrigin::BottomLeft => {
            ((1.0 - uv[1]).rem_euclid(1.0) * texture.height as f32).floor()
        }
    } as i32;
    rgba8_at_repeat(texture, x, y)
}

fn sample_rgba8_repeat_linear_no_half(
    texture: &CpuRgba8Image,
    uv: [f32; 2],
    origin: Rgba8SamplingOrigin,
) -> [u8; 4] {
    let u = uv[0].rem_euclid(1.0);
    let v = uv[1].rem_euclid(1.0);
    let x = u * texture.width as f32;
    let y = match origin {
        Rgba8SamplingOrigin::TopLeft => v * texture.height as f32,
        Rgba8SamplingOrigin::BottomLeft => (1.0 - v) * texture.height as f32,
    };
    let x0 = x.floor();
    let y0 = y.floor();
    let tx = x - x0;
    let ty = y - y0;
    let x0 = x0 as i32;
    let y0 = y0 as i32;
    std::array::from_fn(|channel| {
        let top = lerp_u8(
            channel_at_repeat(texture, x0, y0, channel),
            channel_at_repeat(texture, x0 + 1, y0, channel),
            tx,
        );
        let bottom = lerp_u8(
            channel_at_repeat(texture, x0, y0 + 1, channel),
            channel_at_repeat(texture, x0 + 1, y0 + 1, channel),
            tx,
        );
        quantize_unorm8(lerp_f32(top, bottom, ty))
    })
}

fn rgba8_at_repeat(texture: &CpuRgba8Image, x: i32, y: i32) -> [u8; 4] {
    std::array::from_fn(|channel| channel_at_repeat(texture, x, y, channel))
}

fn channel_at_repeat(texture: &CpuRgba8Image, x: i32, y: i32, channel: usize) -> u8 {
    let width = texture.width as i32;
    let height = texture.height as i32;
    let x = x.rem_euclid(width) as u32;
    let y = y.rem_euclid(height) as u32;
    let index = ((y * texture.width + x) * 4) as usize + channel;
    texture.rgba.get(index).copied().unwrap_or(0)
}

fn lerp_u8(left: u8, right: u8, amount: f32) -> f32 {
    lerp_f32(f32::from(left) / 255.0, f32::from(right) / 255.0, amount)
}

fn lerp_f32(left: f32, right: f32, amount: f32) -> f32 {
    left + (right - left) * amount
}

fn interpolate_scalar(barycentric: [f32; 3], a: f32, b: f32, c: f32) -> f32 {
    barycentric[0] * a + barycentric[1] * b + barycentric[2] * c
}

fn interpolate_vertex_color(
    barycentric: [f32; 3],
    a: &GltfTransformedVertex,
    b: &GltfTransformedVertex,
    c: &GltfTransformedVertex,
) -> [f32; 4] {
    [
        interpolate_scalar(barycentric, a.color_0[0], b.color_0[0], c.color_0[0]),
        interpolate_scalar(barycentric, a.color_0[1], b.color_0[1], c.color_0[1]),
        interpolate_scalar(barycentric, a.color_0[2], b.color_0[2], c.color_0[2]),
        interpolate_scalar(barycentric, a.color_0[3], b.color_0[3], c.color_0[3]),
    ]
}

fn multiply_rgba(left: [f32; 4], right: [f32; 4]) -> [f32; 4] {
    [
        left[0] * right[0],
        left[1] * right[1],
        left[2] * right[2],
        left[3] * right[3],
    ]
}

fn signed_area(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn triangle_edge_distance_pixels(
    point: [f32; 2],
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
) -> f32 {
    nearest_triangle_edge(point, a, b, c).distance_pixels
}

fn nearest_triangle_edge(
    point: [f32; 2],
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
) -> TriangleEdgeDistance {
    [(0, a, b), (1, b, c), (2, c, a)]
        .into_iter()
        .map(|(edge, start, end)| TriangleEdgeDistance {
            edge,
            distance_pixels: point_to_segment_distance(point, start, end),
        })
        .min_by(|left, right| {
            left.distance_pixels
                .partial_cmp(&right.distance_pixels)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.edge.cmp(&right.edge))
        })
        .unwrap_or(TriangleEdgeDistance {
            edge: 0,
            distance_pixels: 0.0,
        })
}

fn triangle_edge_indices(indices: [u32; 3], edge: usize) -> [u32; 2] {
    match edge {
        0 => [indices[0], indices[1]],
        1 => [indices[1], indices[2]],
        2 => [indices[2], indices[0]],
        _ => [indices[0], indices[1]],
    }
}

fn point_to_segment_distance(point: [f32; 2], start: [f32; 2], end: [f32; 2]) -> f32 {
    let segment = [end[0] - start[0], end[1] - start[1]];
    let length_squared = segment[0] * segment[0] + segment[1] * segment[1];
    if length_squared <= f32::EPSILON {
        return ((point[0] - start[0]).powi(2) + (point[1] - start[1]).powi(2)).sqrt();
    }
    let relative = [point[0] - start[0], point[1] - start[1]];
    let t = ((relative[0] * segment[0] + relative[1] * segment[1]) / length_squared)
        .clamp(0.0, 1.0);
    let closest = [start[0] + segment[0] * t, start[1] + segment[1] * t];
    ((point[0] - closest[0]).powi(2) + (point[1] - closest[1]).powi(2)).sqrt()
}

fn assert_close(actual: f32, expected: f32, label: &str) -> Result<(), Box<dyn Error>> {
    if (actual - expected).abs() > 1.0e-6 {
        return Err(format!("{label}: expected {expected}, got {actual}").into());
    }
    Ok(())
}

fn diagnostic_linear_uv(color: [u8; 4]) -> [f32; 2] {
    [
        srgb_to_linear_channel(f32::from(color[0]) / 255.0),
        srgb_to_linear_channel(f32::from(color[1]) / 255.0),
    ]
}

fn diagnostic_linear_uv_to_srgb_color(linear_uv: [f32; 2], alpha: u8) -> [u8; 4] {
    [
        quantize_unorm8(linear_to_srgb_channel(linear_uv[0])),
        quantize_unorm8(linear_to_srgb_channel(linear_uv[1])),
        0,
        alpha,
    ]
}

fn srgb_to_linear_channel(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb_channel(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn quantize_unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn rgb_distance(left: [u8; 4], right: [u8; 4]) -> f32 {
    let dr = f32::from(left[0]) - f32::from(right[0]);
    let dg = f32::from(left[1]) - f32::from(right[1]);
    let db = f32::from(left[2]) - f32::from(right[2]);
    (dr * dr + dg * dg + db * db).sqrt()
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn script_args() -> Vec<std::ffi::OsString> {
    std::env::args_os().filter(|arg| arg != "--").collect()
}
