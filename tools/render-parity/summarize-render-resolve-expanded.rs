#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Summarize current render-resolve parity reports without selecting samples.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "summarize-render-resolve-expanded",
    about = "Summarize render-resolve imqraw and texture-audit reports"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    reports_dir: Option<PathBuf>,
    #[arg(long, default_value = "Seed-san")]
    fixture_stem: String,
    #[arg(long, default_value = "render-resolve-expanded.gradient")]
    suffix: String,
    #[arg(long, default_values_t = ["wgpu".to_string(), "bevy".to_string(), "ash".to_string()])]
    renderer: Vec<String>,
    #[arg(long, default_value = "rgbSharedNonblackGradientInterior1px")]
    metric_key: String,
    #[arg(long)]
    shading_model_join: Option<PathBuf>,
    #[arg(long)]
    material_track_inputs: Option<PathBuf>,
    #[arg(long, value_name = "RENDERER=PATH")]
    texture_audit: Vec<String>,
    #[arg(long, value_name = "RENDERER=PATH")]
    focused_material_pixels: Vec<String>,
    #[arg(long, value_name = "RENDERER=PATH")]
    base_color_owner_join: Vec<String>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
struct ExpandedSummary {
    reports_dir: String,
    fixture_stem: String,
    suffix: String,
    metric_key: String,
    renderers: Vec<RendererSummary>,
    shading_model_join: Option<ShadingModelJoinSummary>,
    material_track_inputs: Option<MaterialTrackInputSummary>,
    texture_audits: Vec<TextureAuditProbeSummary>,
    focused_material_pixels: Vec<FocusedMaterialPixelSummary>,
    base_color_owner_joins: Vec<BaseColorOwnerJoinSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct RendererSummary {
    renderer: String,
    psnr: f64,
    changed_rgb_pixels: u64,
    max_channel_delta: u64,
    alpha_mismatches: u64,
    hotspot_count: u64,
    manifest_count: u64,
    selected_count: u64,
    missing_selection_count: u64,
    all_mean_expected_actual_distance: f64,
    selected_mean_expected_actual_distance: f64,
    selected_mtoon_count: u64,
    selected_gltf_pbr_count: u64,
    top_selected_materials: Vec<MaterialSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialSummary {
    material: String,
    count: u64,
    mean_expected_actual_distance: f64,
    mean_expected_minus_actual_rgb_delta: [f64; 3],
    manifest_actual_closer: u64,
    manifest_expected_closer: u64,
    manifest_tied: u64,
    manifest_actual_within_1_5: u64,
    manifest_expected_within_1_5: u64,
    best_sampling_actual_within_8: u64,
    best_sampling_expected_within_8: u64,
    same_expected_material: u64,
    same_expected_triangle: u64,
}

#[derive(Clone, Debug, Serialize)]
struct ShadingModelJoinSummary {
    path: String,
    models: Vec<ShadingModelSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct ShadingModelSummary {
    model: String,
    shared_pixel_count: u64,
    backends: Vec<ShadingModelBackendSummary>,
    sample_following: Vec<ShadingModelSampleFollowingSummary>,
    backend_pairs: Vec<ShadingModelBackendPairSummary>,
    top_direction_signature: Option<String>,
    top_direction_count: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
struct ShadingModelBackendSummary {
    backend: String,
    row_count: u64,
    selected_count: u64,
    mean_expected_actual_distance: f64,
    mean_expected_minus_actual_rgb_delta: [f64; 3],
    color_fit: Option<ShadingModelColorFitSummary>,
    material_draw_color_fits: Vec<ShadingModelMaterialDrawColorFitSummary>,
    material_draw_shading_inputs: Vec<ShadingModelMaterialDrawShadingInputSummary>,
    materials: String,
    draw_keys: String,
}

#[derive(Clone, Debug, Serialize)]
struct ShadingModelColorFitSummary {
    preferred_fit: String,
    additive_rgb_delta: Option<[f64; 3]>,
    #[serde(rename = "additive_fit_mean_rgb_distance")]
    additive_fit_mean_distance: Option<f64>,
    least_squares_gain_rgb: Option<[f64; 3]>,
    #[serde(rename = "gain_fit_mean_rgb_distance")]
    gain_fit_mean_distance: Option<f64>,
    mean_expected_over_actual_rgb_ratio: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Serialize)]
struct ShadingModelMaterialDrawColorFitSummary {
    material_name: String,
    draw_key: String,
    row_count: u64,
    mean_expected_actual_distance: Option<f64>,
    mean_expected_minus_actual_rgb_delta: Option<[f64; 3]>,
    color_fit: Option<ShadingModelColorFitSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct ShadingModelMaterialDrawShadingInputSummary {
    material_name: String,
    draw_key: String,
    row_count: u64,
    models: String,
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
struct ShadingModelSampleFollowingSummary {
    backend: String,
    shared_rows: u64,
    sample_exact_rows: u64,
    sample_exact_ratio: f64,
    mean_actual_selection_distance: f64,
    mean_expected_selection_distance: f64,
}

#[derive(Clone, Debug, Serialize)]
struct ShadingModelBackendPairSummary {
    left: String,
    right: String,
    shared_pixels: u64,
    mean_actual_distance: f64,
    mean_expected_actual_gap_delta: f64,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialTrackInputSummary {
    path: String,
    fixture: String,
    selected_count: u64,
    materials: Vec<MaterialTrackMaterialSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialTrackMaterialSummary {
    index: u64,
    name: String,
    branch: String,
    primitive_count: u64,
    base_color_factor: [f64; 4],
    metallic_factor: f64,
    roughness_factor: f64,
    alpha_mode: String,
    double_sided: bool,
    unlit: bool,
    emissive_factor: [f64; 3],
    emissive_strength: f64,
    normal_scale: Option<f64>,
    base_texture: String,
    shade_texture: String,
    normal_texture: String,
    mtoon_summary: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct TextureAuditProbeSummary {
    renderer: String,
    path: String,
    selection_source_buckets: Vec<TextureSelectionSourceSummary>,
    recommended_probes: Vec<TextureProbeSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct TextureSelectionSourceSummary {
    selection_source: String,
    count: u64,
    mean_expected_actual_distance: Option<f64>,
    manifest_actual_closer: u64,
    manifest_expected_closer: u64,
    manifest_tied: u64,
    manifest_actual_within_1_5: u64,
    manifest_expected_within_1_5: u64,
    mean_manifest_actual_distance: Option<f64>,
    mean_manifest_expected_distance: Option<f64>,
    mean_actual_minus_manifest_rgb_delta: Option<[f64; 3]>,
    mean_expected_minus_manifest_rgb_delta: Option<[f64; 3]>,
    materials: String,
    draw_keys: String,
}

#[derive(Clone, Debug, Serialize)]
struct TextureProbeSummary {
    material_name: String,
    draw_key: String,
    count: u64,
    classification: String,
    action: String,
    mean_expected_actual_distance: Option<f64>,
    mean_manifest_actual_distance: Option<f64>,
    mean_manifest_expected_distance: Option<f64>,
    mean_actual_minus_manifest_rgb_delta: Option<[f64; 3]>,
    mean_expected_minus_manifest_rgb_delta: Option<[f64; 3]>,
    least_squares_actual_over_manifest_rgb_gain: Option<[f64; 3]>,
    least_squares_expected_over_manifest_rgb_gain: Option<[f64; 3]>,
    manifest_sample_actual_within_1_5: u64,
    manifest_sample_expected_within_1_5: u64,
    manifest_sample_both_far: u64,
}

#[derive(Clone, Debug, Serialize)]
struct FocusedMaterialPixelSummary {
    renderer: String,
    path: String,
    actual_source: String,
    rows: Vec<FocusedMaterialPixelRowSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct FocusedMaterialPixelRowSummary {
    pixel: String,
    interpretation: String,
    selected_material: String,
    selected_triangle: Option<u64>,
    selection_source: String,
    selected_rgba: Option<[u64; 4]>,
    actual_rgba: Option<[u64; 4]>,
    expected_rgba: Option<[u64; 4]>,
    actual_expected_distance: Option<f64>,
    selected_actual_distance: Option<f64>,
    selected_expected_distance: Option<f64>,
    browser_material: String,
    renderer_material_draw: String,
    frontmost_material: String,
    nearest_expected_material: String,
}

#[derive(Clone, Debug, Serialize)]
struct BaseColorOwnerJoinSummary {
    renderer: String,
    path: String,
    joined_count: u64,
    missing_base_color_count: u64,
    rendered_owner_count: u64,
    owner_matches_base_frontmost_material: u64,
    owner_matches_base_frontmost_surface: u64,
    mean_owner_surface_base_color_rendered_distance: Option<f64>,
    mean_owner_surface_texture_as_linear_rendered_distance: Option<f64>,
    mean_owner_surface_browser_base_color_rendered_distance: Option<f64>,
    owner_to_base_frontmost_materials: String,
    frontmost_to_nearest_rendered_base_color_materials: String,
    frontmost_to_nearest_rendered_base_color_draw_order: String,
    owner_material_buckets: Vec<BaseColorOwnerMaterialBucketSummary>,
    top_owner_surface_color_deltas: Vec<BaseColorOwnerDeltaSummary>,
}

#[derive(Clone, Debug, Serialize)]
struct BaseColorOwnerMaterialBucketSummary {
    material_name: String,
    count: u64,
    mean_base_color_rendered_distance: Option<f64>,
    mean_texture_as_linear_rendered_distance: Option<f64>,
    mean_browser_base_color_rendered_distance: Option<f64>,
    frontmost_material_matches: u64,
    frontmost_surface_matches: u64,
}

#[derive(Clone, Debug, Serialize)]
struct BaseColorOwnerDeltaSummary {
    x: u64,
    y: u64,
    owner_material: Option<String>,
    base_frontmost_material: Option<String>,
    nearest_rendered_base_color_material: Option<String>,
    draw_delta: Option<i64>,
    projected_base_color: Option<[u64; 4]>,
    projected_texture_as_linear_color: Option<[u64; 4]>,
    projected_browser_base_color: Option<[u64; 4]>,
    base_color_rendered_distance: Option<f64>,
    texture_as_linear_rendered_distance: Option<f64>,
    browser_base_color_rendered_distance: Option<f64>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse();
    if options.self_test {
        return self_test();
    }
    let reports_dir = options
        .reports_dir
        .as_deref()
        .ok_or("--reports-dir is required")?;
    let summary = summarize(&options, reports_dir)?;
    let markdown = render_markdown(&summary);
    if let Some(path) = &options.json_out {
        write_parented(path, &serde_json::to_string_pretty(&summary)?)?;
    }
    if let Some(path) = &options.markdown_out {
        write_parented(path, &markdown)?;
    }
    if options.json_out.is_none() && options.markdown_out.is_none() {
        print!("{markdown}");
    }
    Ok(())
}

fn summarize(options: &Options, reports_dir: &Path) -> Result<ExpandedSummary, Box<dyn Error>> {
    let mut renderers = Vec::new();
    for renderer in &options.renderer {
        let imqraw = read_json(&reports_dir.join(format!(
            "{}.{}-vs-three-vrm.{}.imqraw-rust.json",
            options.fixture_stem, renderer, options.suffix
        )))?;
        let audit = read_json(&reports_dir.join(format!(
            "{}.{}-texture-sampling-audit.{}.json",
            options.fixture_stem, renderer, options.suffix
        )))?;
        renderers.push(RendererSummary {
            renderer: renderer.clone(),
            psnr: get_f64_path(&imqraw, &[&options.metric_key, "psnr"])?,
            changed_rgb_pixels: get_u64_path(&imqraw, &["changedPixels", "rgb"])?,
            max_channel_delta: get_u64_path(&imqraw, &["maxChannelDelta"])?,
            alpha_mismatches: get_u64_path(&imqraw, &["alpha", "mismatches"])?,
            hotspot_count: get_u64_path(&audit, &["hotspot_count"])?,
            manifest_count: get_u64_path(&audit, &["manifest_count"])?,
            selected_count: get_u64_path(&audit, &["selected_count"])?,
            missing_selection_count: get_u64_path(&audit, &["missing_selection_count"])?,
            all_mean_expected_actual_distance: get_f64_path(
                &audit,
                &["all", "mean_expected_actual_rgb_distance"],
            )?,
            selected_mean_expected_actual_distance: get_f64_path(
                &audit,
                &["selected", "mean_expected_actual_rgb_distance"],
            )?,
            selected_mtoon_count: get_named_count_path(
                &audit,
                &["selected", "shading_model_counts"],
                "model",
                "mtoon",
            )?,
            selected_gltf_pbr_count: get_named_count_path(
                &audit,
                &["selected", "shading_model_counts"],
                "model",
                "gltf_pbr",
            )?,
            top_selected_materials: material_summaries(&audit, 6)?,
        });
    }
    Ok(ExpandedSummary {
        reports_dir: reports_dir.display().to_string(),
        fixture_stem: options.fixture_stem.clone(),
        suffix: options.suffix.clone(),
        metric_key: options.metric_key.clone(),
        renderers,
        shading_model_join: options
            .shading_model_join
            .as_deref()
            .map(shading_model_join_summary)
            .transpose()?,
        material_track_inputs: options
            .material_track_inputs
            .as_deref()
            .map(material_track_input_summary)
            .transpose()?,
        texture_audits: texture_audit_probe_summaries(&options.texture_audit)?,
        focused_material_pixels: focused_material_pixel_summaries(&options.focused_material_pixels)?,
        base_color_owner_joins: base_color_owner_join_summaries(&options.base_color_owner_join)?,
    })
}

fn material_summaries(audit: &Value, limit: usize) -> Result<Vec<MaterialSummary>, Box<dyn Error>> {
    let buckets = get_path(audit, &["selected", "material_buckets"])?
        .as_array()
        .ok_or("selected.material_buckets is not an array")?;
    buckets
        .iter()
        .take(limit)
        .map(|bucket| {
            Ok(MaterialSummary {
                material: get_str_path(bucket, &["material_name"])?,
                count: get_u64_path(bucket, &["count"])?,
                mean_expected_actual_distance: get_f64_path(
                    bucket,
                    &["mean_expected_actual_rgb_distance"],
                )?,
                mean_expected_minus_actual_rgb_delta: get_vec3_path(
                    bucket,
                    &["mean_expected_minus_actual_rgb_delta"],
                )?,
                manifest_actual_closer: get_u64_path(bucket, &["manifest_sample_actual_closer"])?,
                manifest_expected_closer: get_u64_path(
                    bucket,
                    &["manifest_sample_expected_closer"],
                )?,
                manifest_tied: get_u64_path(bucket, &["manifest_sample_tied"])?,
                manifest_actual_within_1_5: get_u64_path(
                    bucket,
                    &["manifest_sample_actual_within_1_5"],
                )?,
                manifest_expected_within_1_5: get_u64_path(
                    bucket,
                    &["manifest_sample_expected_within_1_5"],
                )?,
                best_sampling_actual_within_8: get_u64_path(
                    bucket,
                    &["best_sampling_actual_within_8"],
                )?,
                best_sampling_expected_within_8: get_u64_path(
                    bucket,
                    &["best_sampling_expected_within_8"],
                )?,
                same_expected_material: get_u64_path(bucket, &["same_material_as_expected"])?,
                same_expected_triangle: get_u64_path(bucket, &["same_triangle_as_expected"])?,
            })
        })
        .collect()
}

fn render_markdown(summary: &ExpandedSummary) -> String {
    let mut out = String::new();
    out.push_str("# Render-Resolve Expanded Summary\n\n");
    out.push_str(&format!(
        "- Reports: `{}`\n- Fixture: `{}`\n- Suffix: `{}`\n- Metric: `{}`\n\n",
        summary.reports_dir, summary.fixture_stem, summary.suffix, summary.metric_key
    ));
    out.push_str("## Backend Metrics\n\n");
    out.push_str("| Renderer | PSNR | Changed RGB | Max delta | Alpha | Hotspots | Selected | Missing | All mean E-A | Selected mean E-A | Selected MToon/PBR |\n");
    out.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");
    for row in &summary.renderers {
        out.push_str(&format!(
            "| {} | {:.4} | {} | {} | {} | {} | {} | {} | {:.4} | {:.4} | {}/{} |\n",
            row.renderer,
            row.psnr,
            row.changed_rgb_pixels,
            row.max_channel_delta,
            row.alpha_mismatches,
            row.hotspot_count,
            row.selected_count,
            row.missing_selection_count,
            row.all_mean_expected_actual_distance,
            row.selected_mean_expected_actual_distance,
            row.selected_mtoon_count,
            row.selected_gltf_pbr_count
        ));
    }
    out.push_str("\n## Top Selected Materials\n\n");
    for row in &summary.renderers {
        out.push_str(&format!("### {}\n\n", row.renderer));
        out.push_str("| Material | Count | Mean E-A | Mean E-A RGB | Manifest A/E/T | Manifest <=1.5 A/E | Best sample <=8 A/E | Same expected mat/tri |\n");
        out.push_str("| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |\n");
        for material in &row.top_selected_materials {
            out.push_str(&format!(
                "| {} | {} | {:.4} | {:.2},{:.2},{:.2} | {}/{}/{} | {}/{} | {}/{} | {}/{} |\n",
                material.material,
                material.count,
                material.mean_expected_actual_distance,
                material.mean_expected_minus_actual_rgb_delta[0],
                material.mean_expected_minus_actual_rgb_delta[1],
                material.mean_expected_minus_actual_rgb_delta[2],
                material.manifest_actual_closer,
                material.manifest_expected_closer,
                material.manifest_tied,
                material.manifest_actual_within_1_5,
                material.manifest_expected_within_1_5,
                material.best_sampling_actual_within_8,
                material.best_sampling_expected_within_8,
                material.same_expected_material,
                material.same_expected_triangle
            ));
        }
        out.push('\n');
    }
    if let Some(join) = &summary.shading_model_join {
        out.push_str("## Shading Model Backend Agreement\n\n");
        out.push_str(&format!("- Join: `{}`\n\n", join.path));
        for model in &join.models {
            out.push_str(&format!("### `{}`\n\n", model.model));
            out.push_str(&format!(
                "- Shared top-residual pixels: `{}`\n",
                model.shared_pixel_count
            ));
            if let (Some(signature), Some(count)) =
                (&model.top_direction_signature, model.top_direction_count)
            {
                out.push_str(&format!(
                    "- Top direction bucket: `{}` (`{}` pixels)\n",
                    signature, count
                ));
            }
            out.push('\n');
            out.push_str(
                "| Backend | Rows | Selected | Mean E-A | Mean E-A RGB | Materials | Draw keys |\n",
            );
            out.push_str("| --- | ---: | ---: | ---: | --- | --- | --- |\n");
            for backend in &model.backends {
                out.push_str(&format!(
                    "| {} | {} | {} | {:.4} | {:.2},{:.2},{:.2} | {} | {} |\n",
                    backend.backend,
                    backend.row_count,
                    backend.selected_count,
                    backend.mean_expected_actual_distance,
                    backend.mean_expected_minus_actual_rgb_delta[0],
                    backend.mean_expected_minus_actual_rgb_delta[1],
                    backend.mean_expected_minus_actual_rgb_delta[2],
                    backend.materials,
                    backend.draw_keys
                ));
            }
            out.push('\n');
            if model
                .backends
                .iter()
                .any(|backend| backend.color_fit.is_some())
            {
                out.push_str("#### Backend Color Fit\n\n");
                out.push_str("| Backend | Preferred | Additive RGB | Additive error | Gain RGB | Gain error | Mean E/A ratio |\n");
                out.push_str("| --- | --- | ---: | ---: | ---: | ---: | ---: |\n");
                for backend in &model.backends {
                    if let Some(fit) = &backend.color_fit {
                        out.push_str(&format!(
                            "| {} | {} | {} | {} | {} | {} | {} |\n",
                            backend.backend,
                            fit.preferred_fit,
                            fmt_optional_vec3(fit.additive_rgb_delta),
                            fmt_optional_f64(fit.additive_fit_mean_distance),
                            fmt_optional_vec3(fit.least_squares_gain_rgb),
                            fmt_optional_f64(fit.gain_fit_mean_distance),
                            fmt_optional_vec3(fit.mean_expected_over_actual_rgb_ratio),
                        ));
                    }
                }
                out.push('\n');
            }
            if model
                .backends
                .iter()
                .any(|backend| !backend.material_draw_color_fits.is_empty())
            {
                out.push_str("#### Material / Draw Color Fit\n\n");
                out.push_str("| Backend | Material | Draw key | Rows | Mean E-A | Mean E-A RGB | Preferred | Additive RGB | Additive error | Gain RGB | Gain error |\n");
                out.push_str("| --- | --- | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: |\n");
                for backend in &model.backends {
                    for fit in backend.material_draw_color_fits.iter().take(8) {
                        let color_fit = fit.color_fit.as_ref();
                        out.push_str(&format!(
                            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                            backend.backend,
                            fit.material_name,
                            fit.draw_key,
                            fit.row_count,
                            fmt_optional_f64(fit.mean_expected_actual_distance),
                            fmt_optional_vec3(fit.mean_expected_minus_actual_rgb_delta),
                            color_fit
                                .map(|fit| fit.preferred_fit.as_str())
                                .unwrap_or("n/a"),
                            fmt_optional_vec3(color_fit.and_then(|fit| fit.additive_rgb_delta)),
                            fmt_optional_f64(
                                color_fit.and_then(|fit| fit.additive_fit_mean_distance)
                            ),
                            fmt_optional_vec3(
                                color_fit.and_then(|fit| fit.least_squares_gain_rgb)
                            ),
                            fmt_optional_f64(
                                color_fit.and_then(|fit| fit.gain_fit_mean_distance)
                            ),
                        ));
                    }
                }
                out.push('\n');
            }
            if model
                .backends
                .iter()
                .any(|backend| !backend.material_draw_shading_inputs.is_empty())
            {
                out.push_str("#### Material / Draw Shading Inputs\n\n");
                out.push_str("| Backend | Material | Draw key | Rows | Models | Base | Shade | Emissive | M/R/O/N | Unlit | V0 shade |\n");
                out.push_str("| --- | --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
                for backend in &model.backends {
                    for input in backend.material_draw_shading_inputs.iter().take(8) {
                        out.push_str(&format!(
                            "| {} | {} | {} | {} | {} | {} | {} | {} | {} / {} / {} / {} | {} | {} |\n",
                            backend.backend,
                            input.material_name,
                            input.draw_key,
                            input.row_count,
                            input.models,
                            fmt_optional_vec4(input.mean_base_color),
                            fmt_optional_vec4(input.mean_shade_color),
                            fmt_optional_vec3(input.mean_emissive),
                            fmt_optional_f64(input.mean_metallic),
                            fmt_optional_f64(input.mean_roughness),
                            fmt_optional_f64(input.mean_occlusion_strength),
                            fmt_optional_f64(input.mean_normal_scale),
                            input.unlit_count,
                            input.v0_compat_shade_count,
                        ));
                    }
                }
                out.push('\n');
            }
            out.push_str(
                "| Backend | Shared rows | Sample exact | Exact ratio | Mean A-S | Mean E-S |\n",
            );
            out.push_str("| --- | ---: | ---: | ---: | ---: | ---: |\n");
            for sample in &model.sample_following {
                out.push_str(&format!(
                    "| {} | {} | {} | {:.4} | {:.4} | {:.4} |\n",
                    sample.backend,
                    sample.shared_rows,
                    sample.sample_exact_rows,
                    sample.sample_exact_ratio,
                    sample.mean_actual_selection_distance,
                    sample.mean_expected_selection_distance
                ));
            }
            out.push('\n');
            out.push_str(
                "| Pair | Shared pixels | Mean actual RGB distance | Mean E-A gap delta |\n",
            );
            out.push_str("| --- | ---: | ---: | ---: |\n");
            for pair in &model.backend_pairs {
                out.push_str(&format!(
                    "| {} / {} | {} | {:.4} | {:.4} |\n",
                    pair.left,
                    pair.right,
                    pair.shared_pixels,
                    pair.mean_actual_distance,
                    pair.mean_expected_actual_gap_delta
                ));
            }
            out.push('\n');
        }
    }
    if let Some(inputs) = &summary.material_track_inputs {
        out.push_str("## Material Track Inputs\n\n");
        out.push_str(&format!(
            "- Input: `{}`\n- Fixture: `{}`\n- Selected materials: `{}`\n\n",
            inputs.path, inputs.fixture, inputs.selected_count
        ));
        out.push_str("| Material | Branch | Prims | Base | M/R | Alpha | Double-sided | Unlit | Emissive | Base tex | Shade tex | Normal tex | MToon |\n");
        out.push_str("| --- | --- | ---: | ---: | ---: | --- | --- | --- | ---: | --- | --- | --- | --- |\n");
        for material in &inputs.materials {
            out.push_str(&format!(
                "| {}#{} | {} | {} | {} | {:.3}/{:.3} | {} | {} | {} | {} x{:.3} | {} | {} | {} | {} |\n",
                material.name,
                material.index,
                material.branch,
                material.primitive_count,
                fmt_vec4(material.base_color_factor),
                material.metallic_factor,
                material.roughness_factor,
                material.alpha_mode,
                material.double_sided,
                material.unlit,
                fmt_vec3(material.emissive_factor),
                material.emissive_strength,
                material.base_texture,
                material.shade_texture,
                material.normal_texture,
                material.mtoon_summary.as_deref().unwrap_or("n/a"),
            ));
        }
        out.push('\n');
    }
    if !summary.texture_audits.is_empty() {
        out.push_str("## Recommended Material Probes\n\n");
        for audit in &summary.texture_audits {
            out.push_str(&format!(
                "### {}\n\n- Input: `{}`\n\n",
                audit.renderer, audit.path
            ));
            if !audit.selection_source_buckets.is_empty() {
                out.push_str("#### Selection Source Buckets\n\n");
                out.push_str("| Source | Count | Mean E-A | Manifest A/E/T | Manifest <=1.5 A/E | Mean Manifest A/E | Mean A-M / E-M | Materials | Draw keys |\n");
                out.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |\n");
                for bucket in &audit.selection_source_buckets {
                    out.push_str(&format!(
                        "| {} | {} | {} | {}/{}/{} | {} / {} | {} / {} | {} / {} | {} | {} |\n",
                        bucket.selection_source,
                        bucket.count,
                        fmt_optional_f64(bucket.mean_expected_actual_distance),
                        bucket.manifest_actual_closer,
                        bucket.manifest_expected_closer,
                        bucket.manifest_tied,
                        bucket.manifest_actual_within_1_5,
                        bucket.manifest_expected_within_1_5,
                        fmt_optional_f64(bucket.mean_manifest_actual_distance),
                        fmt_optional_f64(bucket.mean_manifest_expected_distance),
                        fmt_optional_vec3(bucket.mean_actual_minus_manifest_rgb_delta),
                        fmt_optional_vec3(bucket.mean_expected_minus_manifest_rgb_delta),
                        bucket.materials,
                        bucket.draw_keys,
                    ));
                }
                out.push('\n');
            }
            out.push_str("| Material | Draw key | Count | Classification | Action | Mean E-A | Manifest A/E | A-M RGB | E-M RGB | LS gain A/M | LS gain E/M | Near sample A/E/Both-far |\n");
            out.push_str("| --- | --- | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
            for probe in &audit.recommended_probes {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} / {} | {} | {} | {} | {} | {}/{}/{} |\n",
                    probe.material_name,
                    probe.draw_key,
                    probe.count,
                    probe.classification,
                    probe.action,
                    fmt_optional_f64(probe.mean_expected_actual_distance),
                    fmt_optional_f64(probe.mean_manifest_actual_distance),
                    fmt_optional_f64(probe.mean_manifest_expected_distance),
                    fmt_optional_vec3(probe.mean_actual_minus_manifest_rgb_delta),
                    fmt_optional_vec3(probe.mean_expected_minus_manifest_rgb_delta),
                    fmt_optional_vec3(probe.least_squares_actual_over_manifest_rgb_gain),
                    fmt_optional_vec3(probe.least_squares_expected_over_manifest_rgb_gain),
                    probe.manifest_sample_actual_within_1_5,
                    probe.manifest_sample_expected_within_1_5,
                    probe.manifest_sample_both_far,
                ));
            }
            out.push('\n');
        }
    }
    push_focused_material_state_matrix(summary, &mut out);
    if !summary.focused_material_pixels.is_empty() {
        out.push_str("## Focused Material Pixels\n\n");
        for focus in &summary.focused_material_pixels {
            out.push_str(&format!(
                "### {}\n\n- Input: `{}`\n- Actual: `{}`\n\n",
                focus.renderer, focus.path, focus.actual_source
            ));
            out.push_str("| Pixel | Interpretation | Selected | Source | Browser material | Renderer material | RGBA A/E/S | Dist A-E / S-A / S-E | Frontmost | Nearest expected |\n");
            out.push_str("| --- | --- | --- | --- | --- | --- | --- | ---: | --- | --- |\n");
            for row in &focus.rows {
                out.push_str(&format!(
                    "| {} | {} | {}{} | {} | {} | {} | {} / {} / {} | {} / {} / {} | {} | {} |\n",
                    row.pixel,
                    row.interpretation,
                    row.selected_material,
                    row.selected_triangle
                        .map(|triangle| format!("#{triangle}"))
                        .unwrap_or_default(),
                    row.selection_source,
                    row.browser_material,
                    row.renderer_material_draw,
                    fmt_optional_rgba(row.actual_rgba),
                    fmt_optional_rgba(row.expected_rgba),
                    fmt_optional_rgba(row.selected_rgba),
                    fmt_optional_f64(row.actual_expected_distance),
                    fmt_optional_f64(row.selected_actual_distance),
                    fmt_optional_f64(row.selected_expected_distance),
                    row.frontmost_material,
                    row.nearest_expected_material,
                ));
            }
            out.push('\n');
        }
    }
    if !summary.base_color_owner_joins.is_empty() {
        out.push_str("## Browser Projected Base-Color Joins\n\n");
        for join in &summary.base_color_owner_joins {
            out.push_str(&format!("### `{}`\n\n", join.renderer));
            out.push_str(&format!("- Join: `{}`\n", join.path));
            out.push_str(&format!(
                "- Joined/missing/rendered-owner: `{}` / `{}` / `{}`\n",
                join.joined_count, join.missing_base_color_count, join.rendered_owner_count
            ));
            out.push_str(&format!(
                "- Owner matches base frontmost material/surface: `{}` / `{}`\n",
                join.owner_matches_base_frontmost_material,
                join.owner_matches_base_frontmost_surface
            ));
            out.push_str(&format!(
                "- Mean owner-surface base/browser-compatible distance: `{}` / `{}`\n",
                fmt_optional_f64(join.mean_owner_surface_base_color_rendered_distance),
                fmt_optional_f64(join.mean_owner_surface_browser_base_color_rendered_distance)
            ));
            out.push_str(&format!(
                "- Owner to base frontmost materials: `{}`\n",
                join.owner_to_base_frontmost_materials
            ));
            out.push_str(&format!(
                "- Frontmost to nearest rendered base-color materials: `{}`\n",
                join.frontmost_to_nearest_rendered_base_color_materials
            ));
            out.push_str(&format!(
                "- Frontmost to nearest rendered base-color draw order: `{}`\n\n",
                join.frontmost_to_nearest_rendered_base_color_draw_order
            ));
            out.push_str("| Material | Count | Front material/surface matches | Mean base / browser-compatible distance |\n");
            out.push_str("| --- | ---: | ---: | ---: |\n");
            for bucket in join.owner_material_buckets.iter().take(8) {
                out.push_str(&format!(
                    "| {} | {} | {} / {} | {} / {} |\n",
                    bucket.material_name,
                    bucket.count,
                    bucket.frontmost_material_matches,
                    bucket.frontmost_surface_matches,
                    fmt_optional_f64(bucket.mean_base_color_rendered_distance),
                    fmt_optional_f64(bucket.mean_browser_base_color_rendered_distance)
                ));
            }
            out.push('\n');
            out.push_str("| Pixel | Owner | Base frontmost | Nearest rendered base | Draw delta | Projected base / browser-compatible | Distance base / browser-compatible |\n");
            out.push_str("| --- | --- | --- | --- | ---: | ---: | ---: |\n");
            for delta in join.top_owner_surface_color_deltas.iter().take(8) {
                out.push_str(&format!(
                    "| {},{} | {} | {} | {} | {} | {} / {} | {} / {} |\n",
                    delta.x,
                    delta.y,
                    delta.owner_material.as_deref().unwrap_or("n/a"),
                    delta.base_frontmost_material.as_deref().unwrap_or("n/a"),
                    delta
                        .nearest_rendered_base_color_material
                        .as_deref()
                        .unwrap_or("n/a"),
                    delta
                        .draw_delta
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "n/a".to_owned()),
                    fmt_optional_rgba(delta.projected_base_color),
                    fmt_optional_rgba(delta.projected_browser_base_color),
                    fmt_optional_f64(delta.base_color_rendered_distance),
                    fmt_optional_f64(delta.browser_base_color_rendered_distance)
                ));
            }
            out.push('\n');
        }
    }
    out
}

fn push_focused_material_state_matrix(summary: &ExpandedSummary, out: &mut String) {
    let rows = summary
        .focused_material_pixels
        .iter()
        .flat_map(|focus| {
            focus
                .rows
                .iter()
                .map(move |row| (focus.renderer.as_str(), row))
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return;
    }

    let material_matches = rows
        .iter()
        .filter(|(_, row)| focused_material_names_match(row))
        .count();
    let resolve_rows = rows
        .iter()
        .filter(|(_, row)| row.renderer_material_draw.contains("@owner-sample-resolve"))
        .count();
    let expected_rows = rows
        .iter()
        .filter(|(_, row)| row.expected_rgba.is_some())
        .count();

    out.push_str("## Focused Material State Matrix\n\n");
    out.push_str(&format!(
        "- Rows/material matches/resolve rows/expected-color rows: `{}` / `{}` / `{}` / `{}`\n\n",
        rows.len(),
        material_matches,
        resolve_rows,
        expected_rows
    ));
    out.push_str("| Renderer | Pixel | Selected | Source | Browser material | Rust material | Match | Role | RGBA A/E/S | Dist A-E / S-A / S-E |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- | ---: |\n");
    for (renderer, row) in rows {
        out.push_str(&format!(
            "| {} | {} | {}{} | {} | {} | {} | {} | {} | {} / {} / {} | {} / {} / {} |\n",
            renderer,
            row.pixel,
            row.selected_material,
            row.selected_triangle
                .map(|triangle| format!("#{triangle}"))
                .unwrap_or_default(),
            row.selection_source,
            focused_browser_material_name(row),
            focused_renderer_material_name(row),
            if focused_material_names_match(row) {
                "yes"
            } else {
                "no"
            },
            focused_renderer_role(row),
            fmt_optional_rgba(row.actual_rgba),
            fmt_optional_rgba(row.expected_rgba),
            fmt_optional_rgba(row.selected_rgba),
            fmt_optional_f64(row.actual_expected_distance),
            fmt_optional_f64(row.selected_actual_distance),
            fmt_optional_f64(row.selected_expected_distance),
        ));
    }
    out.push('\n');
}

fn focused_material_names_match(row: &FocusedMaterialPixelRowSummary) -> bool {
    let browser = focused_browser_material_name(row);
    let renderer = focused_renderer_material_name(row);
    browser != "n/a" && renderer != "n/a" && browser == renderer
}

fn focused_browser_material_name(row: &FocusedMaterialPixelRowSummary) -> &str {
    row.browser_material
        .split_whitespace()
        .next()
        .unwrap_or("n/a")
}

fn focused_renderer_material_name(row: &FocusedMaterialPixelRowSummary) -> &str {
    row.renderer_material_draw
        .split_once('@')
        .map(|(material, _)| material)
        .unwrap_or("n/a")
}

fn focused_renderer_role(row: &FocusedMaterialPixelRowSummary) -> &str {
    row.renderer_material_draw
        .split_once('@')
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .unwrap_or("n/a")
}

fn material_track_input_summary(
    path: &Path,
) -> Result<MaterialTrackInputSummary, Box<dyn Error>> {
    let value = read_json(path)?;
    let materials = get_path(&value, &["selected_materials"])?
        .as_array()
        .ok_or("selected_materials is not an array")?
        .iter()
        .map(material_track_material_summary)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MaterialTrackInputSummary {
        path: path.display().to_string(),
        fixture: get_str_path(&value, &["fixture"])?,
        selected_count: get_u64_path(&value, &["selected_count"])?,
        materials,
    })
}

fn material_track_material_summary(
    value: &Value,
) -> Result<MaterialTrackMaterialSummary, Box<dyn Error>> {
    Ok(MaterialTrackMaterialSummary {
        index: get_u64_path(value, &["index"])?,
        name: optional_string_path(value, &["name"])?.unwrap_or_else(|| "unnamed".to_owned()),
        branch: get_str_path(value, &["branch"])?,
        primitive_count: get_u64_path(value, &["primitive_count"])?,
        base_color_factor: get_vec4_path(value, &["base_color_factor"])?,
        metallic_factor: get_f64_path(value, &["metallic_factor"])?,
        roughness_factor: get_f64_path(value, &["roughness_factor"])?,
        alpha_mode: get_str_path(value, &["alpha_mode"])?,
        double_sided: get_bool_path(value, &["double_sided"])?,
        unlit: get_bool_path(value, &["unlit"])?,
        emissive_factor: get_vec3_path(value, &["emissive_factor"])?,
        emissive_strength: get_f64_path(value, &["emissive_strength"])?,
        normal_scale: optional_f64_path(value, &["normal_scale"])?,
        base_texture: texture_slot_summary(value, &["textures"], "baseColorTexture")?,
        shade_texture: texture_slot_summary(value, &["mtoon", "textures"], "shadeMultiplyTexture")?,
        normal_texture: texture_slot_summary(value, &["textures"], "normalTexture")?,
        mtoon_summary: optional_mtoon_summary(value)?,
    })
}

fn optional_mtoon_summary(value: &Value) -> Result<Option<String>, Box<dyn Error>> {
    let Some(mtoon) = optional_path(value, &["mtoon"]) else {
        return Ok(None);
    };
    if mtoon.is_null() {
        return Ok(None);
    }
    let shade = get_vec3_path(mtoon, &["shade_color_factor"])?;
    let rim = get_vec3_path(mtoon, &["parametric_rim_color_factor"])?;
    Ok(Some(format!(
        "shade={} shift/toony/gi={:.3}/{:.3}/{:.3} rim={} mix={:.3} outline={}:{:.4}",
        fmt_vec3(shade),
        get_f64_path(mtoon, &["shading_shift_factor"])?,
        get_f64_path(mtoon, &["shading_toony_factor"])?,
        get_f64_path(mtoon, &["gi_equalization_factor"])?,
        fmt_vec3(rim),
        get_f64_path(mtoon, &["rim_lighting_mix_factor"])?,
        get_str_path(mtoon, &["outline_width_mode"])?,
        get_f64_path(mtoon, &["outline_width_factor"])?,
    )))
}

fn texture_slot_summary(
    value: &Value,
    slots_path: &[&str],
    slot_name: &str,
) -> Result<String, Box<dyn Error>> {
    let Some(slots) = optional_path(value, slots_path) else {
        return Ok("n/a".to_owned());
    };
    let slots = slots
        .as_array()
        .ok_or_else(|| format!("JSON path {} is not an array", slots_path.join(".")))?;
    let Some(slot) = slots
        .iter()
        .find(|slot| slot.get("slot").and_then(Value::as_str) == Some(slot_name))
    else {
        return Ok("n/a".to_owned());
    };
    if slot.get("texture").is_none_or(Value::is_null) {
        return Ok("none".to_owned());
    }
    let texture = get_u64_path(slot, &["texture"])?;
    let image = optional_string_path(slot, &["image", "name"])?
        .or_else(|| optional_u64_path(slot, &["image", "index"]).ok().flatten().map(|index| {
            format!("image#{index}")
        }))
        .unwrap_or_else(|| "image?".to_owned());
    let sampler = optional_u64_path(slot, &["sampler", "min_filter"])?
        .map(|min| format!(" min={min}"))
        .unwrap_or_default();
    Ok(format!("{slot_name}:tex#{texture}:{image}{sampler}"))
}

fn texture_audit_probe_summaries(
    audits: &[String],
) -> Result<Vec<TextureAuditProbeSummary>, Box<dyn Error>> {
    renderer_path_inputs(audits, "--texture-audit")?
        .into_iter()
        .map(|(renderer, path)| texture_audit_probe_summary(&renderer, Path::new(&path)))
        .collect()
}

fn texture_audit_probe_summary(
    renderer: &str,
    path: &Path,
) -> Result<TextureAuditProbeSummary, Box<dyn Error>> {
    let value = read_json(path)?;
    let probes = optional_path(&value, &["recommended_probes"])
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .take(8)
        .map(texture_probe_summary)
        .collect::<Result<Vec<_>, _>>()?;
    let selection_source_buckets = optional_path(&value, &["selection_source_buckets"])
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .take(8)
        .map(texture_selection_source_summary)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TextureAuditProbeSummary {
        renderer: renderer.to_owned(),
        path: path.display().to_string(),
        selection_source_buckets,
        recommended_probes: probes,
    })
}

fn texture_selection_source_summary(
    value: &Value,
) -> Result<TextureSelectionSourceSummary, Box<dyn Error>> {
    let stats = get_path(value, &["stats"])?;
    Ok(TextureSelectionSourceSummary {
        selection_source: get_str_path(value, &["selection_source"])?,
        count: get_u64_path(stats, &["count"])?,
        mean_expected_actual_distance: optional_f64_path(
            stats,
            &["mean_expected_actual_rgb_distance"],
        )?,
        manifest_actual_closer: get_u64_path(stats, &["manifest_sample_actual_closer"])?,
        manifest_expected_closer: get_u64_path(stats, &["manifest_sample_expected_closer"])?,
        manifest_tied: get_u64_path(stats, &["manifest_sample_tied"])?,
        manifest_actual_within_1_5: get_u64_path(
            stats,
            &["manifest_sample_actual_within_1_5"],
        )?,
        manifest_expected_within_1_5: get_u64_path(
            stats,
            &["manifest_sample_expected_within_1_5"],
        )?,
        mean_manifest_actual_distance: optional_f64_path(
            stats,
            &["mean_manifest_sample_actual_rgb_distance"],
        )?,
        mean_manifest_expected_distance: optional_f64_path(
            stats,
            &["mean_manifest_sample_expected_rgb_distance"],
        )?,
        mean_actual_minus_manifest_rgb_delta: optional_vec3_path(
            stats,
            &["mean_actual_minus_manifest_sample_rgb_delta"],
        )?,
        mean_expected_minus_manifest_rgb_delta: optional_vec3_path(
            stats,
            &["mean_expected_minus_manifest_sample_rgb_delta"],
        )?,
        materials: material_count_list(stats, &["material_counts"])?,
        draw_keys: material_draw_bucket_list(stats, &["selection_material_draw_buckets"])?,
    })
}

fn texture_probe_summary(value: &Value) -> Result<TextureProbeSummary, Box<dyn Error>> {
    Ok(TextureProbeSummary {
        material_name: get_str_path(value, &["material_name"])?,
        draw_key: get_str_path(value, &["draw_key"])?,
        count: get_u64_path(value, &["count"])?,
        classification: get_str_path(value, &["classification"])?,
        action: get_str_path(value, &["action"])?,
        mean_expected_actual_distance: optional_f64_path(
            value,
            &["mean_expected_actual_rgb_distance"],
        )?,
        mean_manifest_actual_distance: optional_f64_path(
            value,
            &["mean_manifest_actual_rgb_distance"],
        )?,
        mean_manifest_expected_distance: optional_f64_path(
            value,
            &["mean_manifest_expected_rgb_distance"],
        )?,
        mean_actual_minus_manifest_rgb_delta: optional_vec3_path(
            value,
            &["mean_actual_minus_manifest_rgb_delta"],
        )?,
        mean_expected_minus_manifest_rgb_delta: optional_vec3_path(
            value,
            &["mean_expected_minus_manifest_rgb_delta"],
        )?,
        least_squares_actual_over_manifest_rgb_gain: optional_vec3_path(
            value,
            &["least_squares_actual_over_manifest_rgb_gain"],
        )?,
        least_squares_expected_over_manifest_rgb_gain: optional_vec3_path(
            value,
            &["least_squares_expected_over_manifest_rgb_gain"],
        )?,
        manifest_sample_actual_within_1_5: get_u64_path(
            value,
            &["manifest_sample_actual_within_1_5"],
        )?,
        manifest_sample_expected_within_1_5: get_u64_path(
            value,
            &["manifest_sample_expected_within_1_5"],
        )?,
        manifest_sample_both_far: get_u64_path(value, &["manifest_sample_both_far"])?,
    })
}

fn focused_material_pixel_summaries(
    inputs: &[String],
) -> Result<Vec<FocusedMaterialPixelSummary>, Box<dyn Error>> {
    renderer_path_inputs(inputs, "--focused-material-pixels")?
        .into_iter()
        .map(|(renderer, path)| focused_material_pixel_summary(&renderer, Path::new(&path)))
        .collect()
}

fn focused_material_pixel_summary(
    renderer: &str,
    path: &Path,
) -> Result<FocusedMaterialPixelSummary, Box<dyn Error>> {
    let value = read_json(path)?;
    let rows = get_path(&value, &["rows"])?
        .as_array()
        .ok_or("focused material pixels rows is not an array")?
        .iter()
        .take(12)
        .map(focused_material_pixel_row_summary)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FocusedMaterialPixelSummary {
        renderer: renderer.to_owned(),
        path: path.display().to_string(),
        actual_source: get_str_path(&value, &["actual_source"])?,
        rows,
    })
}

fn focused_material_pixel_row_summary(
    value: &Value,
) -> Result<FocusedMaterialPixelRowSummary, Box<dyn Error>> {
    Ok(FocusedMaterialPixelRowSummary {
        pixel: format!(
            "{},{}",
            get_u64_path(value, &["x"])?,
            get_u64_path(value, &["y"])?
        ),
        interpretation: get_str_path(value, &["interpretation"])?,
        selected_material: optional_string_path(value, &["selected_surface", "material_name"])?
            .unwrap_or_else(|| "n/a".to_owned()),
        selected_triangle: optional_u64_path(value, &["selected_surface", "triangle"])?,
        selection_source: optional_string_path(value, &["selection_source"])?
            .unwrap_or_else(|| "n/a".to_owned()),
        selected_rgba: optional_rgba_path(value, &["selected_rgba"])?,
        actual_rgba: optional_rgba_path(value, &["actual"])?,
        expected_rgba: optional_rgba_path(value, &["expected"])?,
        actual_expected_distance: optional_f64_path(value, &["actual_expected_rgb_distance"])?,
        selected_actual_distance: optional_f64_path(value, &["selected_actual_rgb_distance"])?,
        selected_expected_distance: optional_f64_path(value, &["selected_expected_rgb_distance"])?,
        browser_material: focused_browser_material_summary(value)?,
        renderer_material_draw: focused_renderer_material_draw_summary(value)?,
        frontmost_material: optional_string_path(
            value,
            &["frontmost", "surface", "material_name"],
        )?
        .unwrap_or_else(|| "n/a".to_owned()),
        nearest_expected_material: optional_string_path(
            value,
            &["nearest_expected", "surface", "material_name"],
        )?
        .unwrap_or_else(|| "n/a".to_owned()),
    })
}

fn focused_browser_material_summary(value: &Value) -> Result<String, Box<dyn Error>> {
    let Some(material) = optional_path(value, &["browser_material"]) else {
        return Ok("n/a".to_owned());
    };
    if material.is_null() {
        return Ok("n/a".to_owned());
    }
    let name =
        optional_string_path(material, &["material_name"])?.unwrap_or_else(|| "n/a".to_owned());
    let material_type =
        optional_string_path(material, &["material_type"])?.unwrap_or_else(|| "n/a".to_owned());
    let mesh = optional_string_path(material, &["mesh_name"])?.unwrap_or_else(|| "n/a".to_owned());
    let pass = optional_string_path(material, &["pass"])?.unwrap_or_else(|| "n/a".to_owned());
    let map = optional_string_path(material, &["map_name"])?.unwrap_or_else(|| "n/a".to_owned());
    let color_space = optional_string_path(material, &["map_color_space"])?
        .unwrap_or_else(|| "n/a".to_owned());
    let flip_y = optional_bool_path(material, &["map_flip_y"])?
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned());
    let min_filter = optional_u64_path(material, &["map_min_filter"])?
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned());
    let mag_filter = optional_u64_path(material, &["map_mag_filter"])?
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned());
    Ok(format!(
        "{} {} mesh={} pass={} map={} cs={} flipY={} filter={}/{} color={}",
        name,
        material_type,
        mesh,
        pass,
        map,
        color_space,
        flip_y,
        min_filter,
        mag_filter,
        fmt_optional_vec3(optional_vec3_path(material, &["color"])?),
    ))
}

fn focused_renderer_material_draw_summary(value: &Value) -> Result<String, Box<dyn Error>> {
    let Some(draw) = optional_path(value, &["renderer_material_draw"]) else {
        return Ok("n/a".to_owned());
    };
    if draw.is_null() {
        return Ok("n/a".to_owned());
    }
    let material =
        optional_string_path(draw, &["material_name"])?.unwrap_or_else(|| "n/a".to_owned());
    let role = optional_string_path(draw, &["draw_role"])?.unwrap_or_else(|| "unknown".to_owned());
    let branch = focused_material_shader_branch(draw)?;
    let base = optional_u64_path(draw, &["base_texture"])?
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned());
    let shade = optional_u64_path(draw, &["shade_texture"])?
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned());
    let normal = optional_u64_path(draw, &["normal_texture"])?
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned());
    Ok(format!(
        "{}@{} branch:{} m/r/o/d={}/{}/{}/{} tex(b/s/n)={}/{}/{} base={} shade={} shift/toony/gi={}/{}/{} policy={}/{}/dw:{}/blend:{}",
        material,
        role,
        branch,
        fmt_optional_f64(optional_f64_path(draw, &["metallic"])?),
        fmt_optional_f64(optional_f64_path(draw, &["roughness"])?),
        fmt_optional_f64(optional_f64_path(draw, &["occlusion_strength"])?),
        fmt_optional_f64(optional_f64_path(draw, &["direct_light_scale"])?),
        base,
        shade,
        normal,
        fmt_optional_vec4(optional_vec4_path(draw, &["base_color"])?),
        fmt_optional_vec4(optional_vec4_path(draw, &["shade_color"])?),
        fmt_optional_f64(optional_f64_path(draw, &["shading_shift"])?),
        fmt_optional_f64(optional_f64_path(draw, &["shading_toony"])?),
        fmt_optional_f64(optional_f64_path(draw, &["gi_equalization"])?),
        optional_string_path(draw, &["alpha_mode"])?.unwrap_or_else(|| "n/a".to_owned()),
        optional_string_path(draw, &["cull_mode"])?.unwrap_or_else(|| "n/a".to_owned()),
        optional_bool_path(draw, &["depth_write"])?
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        optional_bool_path(draw, &["blend"])?
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
    ))
}

fn focused_material_shader_branch(draw: &Value) -> Result<String, Box<dyn Error>> {
    if let Some(branch) = optional_string_path(draw, &["shader_branch"])? {
        return Ok(branch);
    }
    if optional_bool_path(draw, &["unlit"])? == Some(true) {
        return Ok("unlit".to_owned());
    }
    Ok(optional_bool_path(draw, &["gltf_pbr"])?
        .or(optional_bool_path(draw, &["pbr_fallback"])?)
        .and_then(|is_pbr| is_pbr.then(|| "gltf_pbr".to_owned()))
        .unwrap_or_else(|| "n/a".to_owned()))
}

fn base_color_owner_join_summaries(
    joins: &[String],
) -> Result<Vec<BaseColorOwnerJoinSummary>, Box<dyn Error>> {
    renderer_path_inputs(joins, "--base-color-owner-join")?
        .into_iter()
        .map(|(renderer, path)| base_color_owner_join_summary(&renderer, Path::new(&path)))
        .collect()
}

fn renderer_path_inputs(
    inputs: &[String],
    option_name: &str,
) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    inputs
        .iter()
        .map(|input| {
            let (renderer, path) = input
                .split_once('=')
                .ok_or_else(|| format!("{option_name} must be RENDERER=PATH: {input}"))?;
            Ok((renderer.to_owned(), path.to_owned()))
        })
        .collect()
}

fn base_color_owner_join_summary(
    renderer: &str,
    path: &Path,
) -> Result<BaseColorOwnerJoinSummary, Box<dyn Error>> {
    let value = read_json(path)?;
    Ok(BaseColorOwnerJoinSummary {
        renderer: renderer.to_owned(),
        path: path.display().to_string(),
        joined_count: get_u64_path(&value, &["joined_count"])?,
        missing_base_color_count: get_u64_path(&value, &["missing_base_color_count"])?,
        rendered_owner_count: get_u64_path(&value, &["rendered_owner_count"])?,
        owner_matches_base_frontmost_material: get_u64_path(
            &value,
            &["owner_matches_base_frontmost_material"],
        )?,
        owner_matches_base_frontmost_surface: get_u64_path(
            &value,
            &["owner_matches_base_frontmost_surface"],
        )?,
        mean_owner_surface_base_color_rendered_distance: optional_f64_path(
            &value,
            &["mean_owner_surface_base_color_rendered_rgb_distance"],
        )?,
        mean_owner_surface_texture_as_linear_rendered_distance: optional_f64_path(
            &value,
            &["mean_owner_surface_texture_as_linear_rendered_rgb_distance"],
        )?,
        mean_owner_surface_browser_base_color_rendered_distance: optional_f64_path(
            &value,
            &["mean_owner_surface_browser_base_color_rendered_rgb_distance"],
        )?
        .or(optional_f64_path(
            &value,
            &["mean_owner_surface_texture_as_linear_rendered_rgb_distance"],
        )?),
        owner_to_base_frontmost_materials: count_map_summary(
            &value,
            &["owner_to_base_frontmost_materials"],
        )?,
        frontmost_to_nearest_rendered_base_color_materials: count_map_summary(
            &value,
            &["frontmost_to_nearest_rendered_base_color_materials"],
        )?,
        frontmost_to_nearest_rendered_base_color_draw_order: count_map_summary(
            &value,
            &["frontmost_to_nearest_rendered_base_color_draw_order"],
        )?,
        owner_material_buckets: base_color_owner_material_buckets(&value, 8)?,
        top_owner_surface_color_deltas: base_color_owner_deltas(&value, 8)?,
    })
}

fn base_color_owner_material_buckets(
    value: &Value,
    limit: usize,
) -> Result<Vec<BaseColorOwnerMaterialBucketSummary>, Box<dyn Error>> {
    let buckets = get_path(value, &["owner_material_buckets"])?
        .as_array()
        .ok_or("owner_material_buckets is not an array")?;
    buckets
        .iter()
        .take(limit)
        .map(|bucket| {
            Ok(BaseColorOwnerMaterialBucketSummary {
                material_name: get_str_path(bucket, &["material_name"])?,
                count: get_u64_path(bucket, &["count"])?,
                mean_base_color_rendered_distance: optional_f64_path(
                    bucket,
                    &["mean_base_color_rendered_rgb_distance"],
                )?,
                mean_texture_as_linear_rendered_distance: optional_f64_path(
                    bucket,
                    &["mean_texture_as_linear_rendered_rgb_distance"],
                )?,
                mean_browser_base_color_rendered_distance: optional_f64_path(
                    bucket,
                    &["mean_browser_base_color_rendered_rgb_distance"],
                )?
                .or(optional_f64_path(
                    bucket,
                    &["mean_texture_as_linear_rendered_rgb_distance"],
                )?),
                frontmost_material_matches: get_u64_path(
                    bucket,
                    &["frontmost_material_matches"],
                )?,
                frontmost_surface_matches: get_u64_path(bucket, &["frontmost_surface_matches"])?,
            })
        })
        .collect()
}

fn base_color_owner_deltas(
    value: &Value,
    limit: usize,
) -> Result<Vec<BaseColorOwnerDeltaSummary>, Box<dyn Error>> {
    let deltas = get_path(value, &["top_owner_surface_color_deltas"])?
        .as_array()
        .ok_or("top_owner_surface_color_deltas is not an array")?;
    deltas
        .iter()
        .take(limit)
        .map(|delta| {
            Ok(BaseColorOwnerDeltaSummary {
                x: get_u64_path(delta, &["x"])?,
                y: get_u64_path(delta, &["y"])?,
                owner_material: optional_surface_material(delta, &["owner_surface"])?,
                base_frontmost_material: optional_surface_material(delta, &["base_frontmost"])?,
                nearest_rendered_base_color_material: optional_surface_material(
                    delta,
                    &["nearest_rendered_base_color"],
                )?,
                draw_delta: optional_i64_path(
                    delta,
                    &["frontmost_to_nearest_rendered_base_color_draw_delta"],
                )?,
                projected_base_color: optional_rgba_path(
                    delta,
                    &["base_frontmost_projected_color"],
                )?,
                projected_texture_as_linear_color: optional_rgba_path(
                    delta,
                    &["base_frontmost_texture_as_linear_color"],
                )?,
                projected_browser_base_color: optional_rgba_path(
                    delta,
                    &["base_frontmost_browser_base_color"],
                )?
                .or(optional_rgba_path(
                    delta,
                    &["base_frontmost_texture_as_linear_color"],
                )?),
                base_color_rendered_distance: optional_f64_path(
                    delta,
                    &["base_color_rendered_rgb_distance"],
                )?,
                texture_as_linear_rendered_distance: optional_f64_path(
                    delta,
                    &["texture_as_linear_rendered_rgb_distance"],
                )?,
                browser_base_color_rendered_distance: optional_f64_path(
                    delta,
                    &["browser_base_color_rendered_rgb_distance"],
                )?
                .or(optional_f64_path(
                    delta,
                    &["texture_as_linear_rendered_rgb_distance"],
                )?),
            })
        })
        .collect()
}

fn shading_model_join_summary(path: &Path) -> Result<ShadingModelJoinSummary, Box<dyn Error>> {
    let value = read_json(path)?;
    let models = get_path(&value, &["models"])?
        .as_array()
        .ok_or("models is not an array")?
        .iter()
        .map(shading_model_summary)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ShadingModelJoinSummary {
        path: path.display().to_string(),
        models,
    })
}

fn shading_model_summary(value: &Value) -> Result<ShadingModelSummary, Box<dyn Error>> {
    let top_direction = get_path(value, &["shared_direction_buckets"])
        .ok()
        .and_then(Value::as_array)
        .and_then(|array| array.first());
    Ok(ShadingModelSummary {
        model: get_str_path(value, &["model"])?,
        shared_pixel_count: get_u64_path(value, &["shared_pixel_count"])?,
        backends: get_path(value, &["backends"])?
            .as_array()
            .ok_or("backends is not an array")?
            .iter()
            .map(shading_model_backend_summary)
            .collect::<Result<Vec<_>, _>>()?,
        sample_following: get_path(value, &["shared_backend_summaries"])?
            .as_array()
            .ok_or("shared_backend_summaries is not an array")?
            .iter()
            .map(shading_model_sample_following_summary)
            .collect::<Result<Vec<_>, _>>()?,
        backend_pairs: get_path(value, &["backend_pairs"])?
            .as_array()
            .ok_or("backend_pairs is not an array")?
            .iter()
            .map(shading_model_backend_pair_summary)
            .collect::<Result<Vec<_>, _>>()?,
        top_direction_signature: top_direction
            .and_then(|bucket| bucket.get("signature"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        top_direction_count: top_direction
            .and_then(|bucket| bucket.get("count"))
            .and_then(Value::as_u64),
    })
}

fn shading_model_backend_summary(
    value: &Value,
) -> Result<ShadingModelBackendSummary, Box<dyn Error>> {
    Ok(ShadingModelBackendSummary {
        backend: get_str_path(value, &["backend"])?,
        row_count: get_u64_path(value, &["row_count"])?,
        selected_count: get_u64_path(value, &["selected_count"])?,
        mean_expected_actual_distance: get_f64_path(value, &["mean_expected_actual_rgb_distance"])?,
        mean_expected_minus_actual_rgb_delta: get_vec3_path(
            value,
            &["mean_expected_minus_actual_rgb_delta"],
        )?,
        color_fit: optional_color_fit(value)?,
        material_draw_color_fits: optional_material_draw_color_fits(value)?,
        material_draw_shading_inputs: optional_material_draw_shading_inputs(value)?,
        materials: key_count_list(value, &["materials"])?,
        draw_keys: key_count_list(value, &["draw_keys"])?,
    })
}

fn optional_material_draw_shading_inputs(
    backend: &Value,
) -> Result<Vec<ShadingModelMaterialDrawShadingInputSummary>, Box<dyn Error>> {
    let Some(entries) = optional_path(backend, &["material_draw_shading_inputs"]) else {
        return Ok(Vec::new());
    };
    let entries = entries
        .as_array()
        .ok_or("material_draw_shading_inputs is not an array")?;
    entries
        .iter()
        .map(|entry| {
            Ok(ShadingModelMaterialDrawShadingInputSummary {
                material_name: get_str_path(entry, &["material_name"])?,
                draw_key: get_str_path(entry, &["draw_key"])?,
                row_count: get_u64_path(entry, &["row_count"])?,
                models: key_count_list(entry, &["models"])?,
                mean_base_color: optional_vec4_path(entry, &["mean_base_color"])?,
                mean_shade_color: optional_vec4_path(entry, &["mean_shade_color"])?,
                mean_emissive: optional_vec3_path(entry, &["mean_emissive"])?,
                mean_metallic: optional_f64_path(entry, &["mean_metallic"])?,
                mean_roughness: optional_f64_path(entry, &["mean_roughness"])?,
                mean_occlusion_strength: optional_f64_path(entry, &["mean_occlusion_strength"])?,
                mean_normal_scale: optional_f64_path(entry, &["mean_normal_scale"])?,
                unlit_count: get_u64_path(entry, &["unlit_count"])?,
                v0_compat_shade_count: get_u64_path(entry, &["v0_compat_shade_count"])?,
            })
        })
        .collect()
}

fn optional_material_draw_color_fits(
    backend: &Value,
) -> Result<Vec<ShadingModelMaterialDrawColorFitSummary>, Box<dyn Error>> {
    let Some(entries) = optional_path(backend, &["material_draw_color_fits"]) else {
        return Ok(Vec::new());
    };
    let entries = entries
        .as_array()
        .ok_or("material_draw_color_fits is not an array")?;
    entries
        .iter()
        .map(|entry| {
            Ok(ShadingModelMaterialDrawColorFitSummary {
                material_name: get_str_path(entry, &["material_name"])?,
                draw_key: get_str_path(entry, &["draw_key"])?,
                row_count: get_u64_path(entry, &["row_count"])?,
                mean_expected_actual_distance: optional_f64_path(
                    entry,
                    &["mean_expected_actual_rgb_distance"],
                )?,
                mean_expected_minus_actual_rgb_delta: optional_vec3_path(
                    entry,
                    &["mean_expected_minus_actual_rgb_delta"],
                )?,
                color_fit: optional_color_fit(entry)?,
            })
        })
        .collect()
}

fn optional_color_fit(
    backend: &Value,
) -> Result<Option<ShadingModelColorFitSummary>, Box<dyn Error>> {
    let Some(value) = optional_color_fit_value(backend) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(ShadingModelColorFitSummary {
        preferred_fit: get_str_path(value, &["preferred_fit"])?,
        additive_rgb_delta: optional_vec3_path(value, &["additive_rgb_delta"])?,
        additive_fit_mean_distance: optional_f64_path_alias(
            value,
            &["additive_fit_mean_rgb_distance"],
            &["additive_fit_mean_distance"],
        )?,
        least_squares_gain_rgb: optional_vec3_path(value, &["least_squares_gain_rgb"])?,
        gain_fit_mean_distance: optional_f64_path_alias(
            value,
            &["gain_fit_mean_rgb_distance"],
            &["gain_fit_mean_distance"],
        )?,
        mean_expected_over_actual_rgb_ratio: optional_vec3_path(
            value,
            &["mean_expected_over_actual_rgb_ratio"],
        )?,
    }))
}

fn optional_color_fit_value(value: &Value) -> Option<&Value> {
    ["color_fit", "color_fit_summary", "colorFit", "colorFitSummary"]
        .into_iter()
        .filter_map(|key| value.get(key))
        .find(|candidate| !candidate.is_null())
}

fn shading_model_sample_following_summary(
    value: &Value,
) -> Result<ShadingModelSampleFollowingSummary, Box<dyn Error>> {
    Ok(ShadingModelSampleFollowingSummary {
        backend: get_str_path(value, &["backend"])?,
        shared_rows: get_u64_path(value, &["shared_rows"])?,
        sample_exact_rows: get_u64_path(value, &["sample_exact_rows"])?,
        sample_exact_ratio: get_f64_path(value, &["sample_exact_ratio"])?,
        mean_actual_selection_distance: get_f64_path(
            value,
            &["mean_actual_selection_rgb_distance"],
        )?,
        mean_expected_selection_distance: get_f64_path(
            value,
            &["mean_expected_selection_rgb_distance"],
        )?,
    })
}

fn shading_model_backend_pair_summary(
    value: &Value,
) -> Result<ShadingModelBackendPairSummary, Box<dyn Error>> {
    Ok(ShadingModelBackendPairSummary {
        left: get_str_path(value, &["left"])?,
        right: get_str_path(value, &["right"])?,
        shared_pixels: get_u64_path(value, &["shared_pixels"])?,
        mean_actual_distance: get_f64_path(value, &["mean_actual_rgb_distance"])?,
        mean_expected_actual_gap_delta: get_f64_path(value, &["mean_expected_actual_gap_delta"])?,
    })
}

fn key_count_list(value: &Value, path: &[&str]) -> Result<String, Box<dyn Error>> {
    let entries = get_path(value, path)?
        .as_array()
        .ok_or_else(|| format!("JSON path {} is not an array", path.join(".")))?;
    let labels = entries
        .iter()
        .map(|entry| {
            let key = get_str_path(entry, &["key"])?;
            let count = get_u64_path(entry, &["count"])?;
            Ok(format!("{key}:{count}"))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    Ok(labels.join(", "))
}

fn material_count_list(value: &Value, path: &[&str]) -> Result<String, Box<dyn Error>> {
    let Some(entries) = optional_path(value, path) else {
        return Ok(String::new());
    };
    let entries = entries
        .as_array()
        .ok_or_else(|| format!("JSON path {} is not an array", path.join(".")))?;
    let labels = entries
        .iter()
        .take(6)
        .map(|entry| {
            let material = get_str_path(entry, &["material_name"])?;
            let count = get_u64_path(entry, &["count"])?;
            Ok(format!("{material}:{count}"))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    Ok(labels.join(", "))
}

fn material_draw_bucket_list(value: &Value, path: &[&str]) -> Result<String, Box<dyn Error>> {
    let Some(entries) = optional_path(value, path) else {
        return Ok(String::new());
    };
    let entries = entries
        .as_array()
        .ok_or_else(|| format!("JSON path {} is not an array", path.join(".")))?;
    let labels = entries
        .iter()
        .take(4)
        .map(|entry| {
            let material = get_str_path(entry, &["material_name"])?;
            let draw_key = get_str_path(entry, &["draw_key"])?;
            let count = get_u64_path(entry, &["stats", "count"])?;
            Ok(format!("{material} {draw_key}:{count}"))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    Ok(labels.join(", "))
}

fn count_map_summary(value: &Value, path: &[&str]) -> Result<String, Box<dyn Error>> {
    let Some(object) = optional_path(value, path) else {
        return Ok(String::new());
    };
    let object = object
        .as_object()
        .ok_or_else(|| format!("JSON path {} is not an object", path.join(".")))?;
    let mut entries = object
        .iter()
        .map(|(key, count)| {
            Ok((
                key.clone(),
                count
                    .as_u64()
                    .ok_or_else(|| format!("JSON path {}.{key} is not an unsigned integer", path.join(".")))?,
            ))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    Ok(entries
        .into_iter()
        .take(6)
        .map(|(key, count)| format!("{key}:{count}"))
        .collect::<Vec<_>>()
        .join(", "))
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn write_parented(path: &Path, text: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    Ok(())
}

fn get_path<'a>(value: &'a Value, path: &[&str]) -> Result<&'a Value, Box<dyn Error>> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| format!("missing JSON path {}", path.join(".")))?;
    }
    Ok(current)
}

fn get_f64_path(value: &Value, path: &[&str]) -> Result<f64, Box<dyn Error>> {
    get_path(value, path)?
        .as_f64()
        .ok_or_else(|| format!("JSON path {} is not a number", path.join(".")).into())
}

fn get_u64_path(value: &Value, path: &[&str]) -> Result<u64, Box<dyn Error>> {
    get_path(value, path)?
        .as_u64()
        .ok_or_else(|| format!("JSON path {} is not an unsigned integer", path.join(".")).into())
}

fn get_str_path(value: &Value, path: &[&str]) -> Result<String, Box<dyn Error>> {
    get_path(value, path)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("JSON path {} is not a string", path.join(".")).into())
}

fn get_vec3_path(value: &Value, path: &[&str]) -> Result<[f64; 3], Box<dyn Error>> {
    let array = get_path(value, path)?
        .as_array()
        .ok_or_else(|| format!("JSON path {} is not an array", path.join(".")))?;
    if array.len() != 3 {
        return Err(format!("JSON path {} does not have length 3", path.join(".")).into());
    }
    Ok([
        array[0]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[0] is not a number", path.join(".")))?,
        array[1]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[1] is not a number", path.join(".")))?,
        array[2]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[2] is not a number", path.join(".")))?,
    ])
}

fn get_vec4_path(value: &Value, path: &[&str]) -> Result<[f64; 4], Box<dyn Error>> {
    let array = get_path(value, path)?
        .as_array()
        .ok_or_else(|| format!("JSON path {} is not an array", path.join(".")))?;
    if array.len() != 4 {
        return Err(format!("JSON path {} does not have length 4", path.join(".")).into());
    }
    Ok([
        array[0]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[0] is not a number", path.join(".")))?,
        array[1]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[1] is not a number", path.join(".")))?,
        array[2]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[2] is not a number", path.join(".")))?,
        array[3]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[3] is not a number", path.join(".")))?,
    ])
}

fn get_bool_path(value: &Value, path: &[&str]) -> Result<bool, Box<dyn Error>> {
    get_path(value, path)?
        .as_bool()
        .ok_or_else(|| format!("JSON path {} is not a boolean", path.join(".")).into())
}

fn optional_string_path(value: &Value, path: &[&str]) -> Result<Option<String>, Box<dyn Error>> {
    let Some(value) = optional_path(value, path) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| format!("JSON path {} is not a string", path.join(".")).into())
}

fn optional_u64_path(value: &Value, path: &[&str]) -> Result<Option<u64>, Box<dyn Error>> {
    let Some(value) = optional_path(value, path) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| format!("JSON path {} is not an unsigned integer", path.join(".")).into())
}

fn optional_bool_path(value: &Value, path: &[&str]) -> Result<Option<bool>, Box<dyn Error>> {
    let Some(value) = optional_path(value, path) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| format!("JSON path {} is not a boolean", path.join(".")).into())
}

fn optional_f64_path(value: &Value, path: &[&str]) -> Result<Option<f64>, Box<dyn Error>> {
    let Some(value) = optional_path(value, path) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_f64()
        .map(Some)
        .ok_or_else(|| format!("JSON path {} is not a number", path.join(".")).into())
}

fn optional_i64_path(value: &Value, path: &[&str]) -> Result<Option<i64>, Box<dyn Error>> {
    let Some(value) = optional_path(value, path) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_i64()
        .map(Some)
        .ok_or_else(|| format!("JSON path {} is not an integer", path.join(".")).into())
}

fn optional_f64_path_alias(
    value: &Value,
    primary_path: &[&str],
    alias_path: &[&str],
) -> Result<Option<f64>, Box<dyn Error>> {
    optional_f64_path(value, primary_path).and_then(|primary| match primary {
        Some(value) => Ok(Some(value)),
        None => optional_f64_path(value, alias_path),
    })
}

fn optional_vec3_path(value: &Value, path: &[&str]) -> Result<Option<[f64; 3]>, Box<dyn Error>> {
    let Some(value) = optional_path(value, path) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let array = value
        .as_array()
        .ok_or_else(|| format!("JSON path {} is not an array", path.join(".")))?;
    if array.len() != 3 {
        return Err(format!("JSON path {} does not have length 3", path.join(".")).into());
    }
    Ok(Some([
        array[0]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[0] is not a number", path.join(".")))?,
        array[1]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[1] is not a number", path.join(".")))?,
        array[2]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[2] is not a number", path.join(".")))?,
    ]))
}

fn optional_rgba_path(value: &Value, path: &[&str]) -> Result<Option<[u64; 4]>, Box<dyn Error>> {
    let Some(value) = optional_path(value, path) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let array = value
        .as_array()
        .ok_or_else(|| format!("JSON path {} is not an array", path.join(".")))?;
    if array.len() != 4 {
        return Err(format!("JSON path {} does not have length 4", path.join(".")).into());
    }
    Ok(Some([
        array[0]
            .as_u64()
            .ok_or_else(|| format!("JSON path {}[0] is not an unsigned integer", path.join(".")))?,
        array[1]
            .as_u64()
            .ok_or_else(|| format!("JSON path {}[1] is not an unsigned integer", path.join(".")))?,
        array[2]
            .as_u64()
            .ok_or_else(|| format!("JSON path {}[2] is not an unsigned integer", path.join(".")))?,
        array[3]
            .as_u64()
            .ok_or_else(|| format!("JSON path {}[3] is not an unsigned integer", path.join(".")))?,
    ]))
}

fn optional_surface_material(value: &Value, path: &[&str]) -> Result<Option<String>, Box<dyn Error>> {
    let Some(value) = optional_path(value, path) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(get_str_path(value, &["material_name"])?))
}

fn optional_vec4_path(value: &Value, path: &[&str]) -> Result<Option<[f64; 4]>, Box<dyn Error>> {
    let Some(value) = optional_path(value, path) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let array = value
        .as_array()
        .ok_or_else(|| format!("JSON path {} is not an array", path.join(".")))?;
    if array.len() != 4 {
        return Err(format!("JSON path {} does not have length 4", path.join(".")).into());
    }
    Ok(Some([
        array[0]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[0] is not a number", path.join(".")))?,
        array[1]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[1] is not a number", path.join(".")))?,
        array[2]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[2] is not a number", path.join(".")))?,
        array[3]
            .as_f64()
            .ok_or_else(|| format!("JSON path {}[3] is not a number", path.join(".")))?,
    ]))
}

fn optional_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn get_named_count_path(
    value: &Value,
    path: &[&str],
    name_key: &str,
    name: &str,
) -> Result<u64, Box<dyn Error>> {
    let Some(array) = get_path(value, path)?.as_array() else {
        return Ok(0);
    };
    Ok(array
        .iter()
        .find(|entry| entry.get(name_key).and_then(Value::as_str) == Some(name))
        .and_then(|entry| entry.get("count").and_then(Value::as_u64))
        .unwrap_or(0))
}

fn fmt_optional_f64(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_optional_vec3(value: Option<[f64; 3]>) -> String {
    value
        .map(fmt_vec3)
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_optional_vec4(value: Option<[f64; 4]>) -> String {
    value
        .map(fmt_vec4)
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_vec3(value: [f64; 3]) -> String {
    format!("{:.2},{:.2},{:.2}", value[0], value[1], value[2])
}

fn fmt_vec4(value: [f64; 4]) -> String {
    format!(
        "{:.2},{:.2},{:.2},{:.2}",
        value[0], value[1], value[2], value[3]
    )
}

fn fmt_optional_rgba(value: Option<[u64; 4]>) -> String {
    value
        .map(|value| format!("{},{},{},{}", value[0], value[1], value[2], value[3]))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn self_test() -> Result<(), Box<dyn Error>> {
    let root = std::env::temp_dir().join(format!(
        "vrm-rs-expanded-summary-self-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(
        root.join("Seed-san.wgpu-vs-three-vrm.render-resolve-expanded.gradient.imqraw-rust.json"),
        r#"{
            "rgbSharedNonblackGradientInterior1px": {"psnr": 42.5},
            "changedPixels": {"rgb": 7},
            "maxChannelDelta": 3,
            "alpha": {"mismatches": 0}
        }"#,
    )?;
    fs::write(
        root.join("Seed-san.wgpu-texture-sampling-audit.render-resolve-expanded.gradient.json"),
        r#"{
            "hotspot_count": 2,
            "manifest_count": 3,
            "selected_count": 1,
            "missing_selection_count": 1,
            "all": {"mean_expected_actual_rgb_distance": 9.0},
            "selected": {
                "mean_expected_actual_rgb_distance": 4.0,
                "shading_model_counts": [{"model": "mtoon", "count": 1}],
                "material_buckets": [{
                    "material_name": "mat",
                    "count": 1,
                    "mean_expected_actual_rgb_distance": 4.0,
                    "mean_expected_minus_actual_rgb_delta": [1.0, 2.0, 3.0],
                    "manifest_sample_actual_closer": 0,
                    "manifest_sample_expected_closer": 1,
                    "manifest_sample_tied": 0,
                    "manifest_sample_actual_within_1_5": 0,
                    "manifest_sample_expected_within_1_5": 1,
                    "best_sampling_actual_within_8": 0,
                    "best_sampling_expected_within_8": 1,
                    "same_material_as_expected": 1,
                    "same_triangle_as_expected": 1
                }]
            }
        }"#,
    )?;
    let join_path = root.join("Seed-san.shading-model-residual-join.json");
    fs::write(
        &join_path,
        r#"{
            "models": [{
                "model": "gltf_pbr",
                "shared_pixel_count": 2,
                "backends": [{
                    "backend": "wgpu",
                    "row_count": 2,
                    "selected_count": 1,
                    "mean_expected_actual_rgb_distance": 4.5,
                    "mean_expected_minus_actual_rgb_delta": [1.0, 2.0, 3.0],
                    "color_fit": null,
                    "color_fit_summary": {
                        "mean_expected_over_actual_rgb_ratio": [1.1, 1.2, 1.3],
                        "least_squares_gain_rgb": [1.05, 1.10, 1.15],
                        "gain_fit_mean_rgb_distance": 2.0,
                        "additive_rgb_delta": [1.0, 2.0, 3.0],
                        "additive_fit_mean_rgb_distance": 1.0,
                        "preferred_fit": "additive"
                    },
                    "material_draw_color_fits": [{
                        "material_name": "backpack_nm",
                        "draw_key": "node145/mesh4/prim9/base",
                        "row_count": 2,
                        "mean_expected_actual_rgb_distance": 4.5,
                        "mean_expected_minus_actual_rgb_delta": [1.0, 2.0, 3.0],
                        "color_fit": null,
                        "colorFit": {
                            "mean_expected_over_actual_rgb_ratio": [1.1, 1.2, 1.3],
                            "least_squares_gain_rgb": [1.05, 1.10, 1.15],
                            "gain_fit_mean_distance": 2.0,
                            "additive_rgb_delta": [1.0, 2.0, 3.0],
                            "additive_fit_mean_distance": 1.0,
                            "preferred_fit": "additive"
                        }
                    }],
                    "material_draw_shading_inputs": [{
                        "material_name": "backpack_nm",
                        "draw_key": "node145/mesh4/prim9/base",
                        "row_count": 2,
                        "models": [{"key": "gltf_pbr", "count": 2}],
                        "mean_base_color": [1.0, 0.5, 0.25, 1.0],
                        "mean_shade_color": [1.0, 1.0, 1.0, 1.0],
                        "mean_emissive": [0.0, 0.0, 0.0],
                        "mean_metallic": 0.0,
                        "mean_roughness": 0.65,
                        "mean_occlusion_strength": 1.0,
                        "mean_normal_scale": 1.0,
                        "unlit_count": 0,
                        "v0_compat_shade_count": 0
                    }],
                    "materials": [{"key": "backpack_nm", "count": 2}],
                    "draw_keys": [{"key": "node145/mesh4/prim9/base", "count": 2}]
                }],
                "shared_backend_summaries": [{
                    "backend": "wgpu",
                    "shared_rows": 2,
                    "sample_exact_rows": 0,
                    "sample_exact_ratio": 0.0,
                    "mean_actual_selection_rgb_distance": 8.0,
                    "mean_expected_selection_rgb_distance": 12.0
                }],
                "backend_pairs": [{
                    "left": "ash",
                    "right": "wgpu",
                    "shared_pixels": 2,
                    "mean_actual_rgb_distance": 0.5,
                    "mean_expected_actual_gap_delta": 0.25
                }],
                "shared_direction_buckets": [{
                    "signature": "ash:expected_brighter, wgpu:expected_brighter",
                    "count": 2
                }]
            }]
        }"#,
    )?;
    let base_color_owner_join_path = root.join("Seed-san.owner-base-color-hotspots.json");
    fs::write(
        &base_color_owner_join_path,
        r#"{
            "joined_count": 2,
            "missing_base_color_count": 0,
            "rendered_owner_count": 2,
            "owner_matches_base_frontmost_material": 1,
            "owner_matches_base_frontmost_surface": 1,
            "mean_owner_surface_base_color_rendered_rgb_distance": 12.5,
            "mean_owner_surface_texture_as_linear_rendered_rgb_distance": 4.5,
            "owner_to_base_frontmost_materials": {"backpack_nm -> backpack_nm": 1, "eye -> face": 1},
            "frontmost_to_nearest_rendered_base_color_materials": {"backpack_nm -> arm_plastic": 1},
            "frontmost_to_nearest_rendered_base_color_draw_order": {"nearest-after": 1, "same": 1},
            "owner_material_buckets": [{
                "material_name": "backpack_nm",
                "count": 1,
                "mean_base_color_rendered_rgb_distance": 12.5,
                "mean_texture_as_linear_rendered_rgb_distance": 4.5,
                "frontmost_material_matches": 1,
                "frontmost_surface_matches": 1
            }],
            "top_owner_surface_color_deltas": [{
                "x": 106,
                "y": 131,
                "owner_surface": {"material_name": "backpack_nm", "triangle": 4},
                "base_frontmost": {"material_name": "backpack_nm", "triangle": 4},
                "nearest_rendered_base_color": {"material_name": "arm_plastic", "triangle": 9},
                "frontmost_to_nearest_rendered_base_color_draw_delta": 2,
                "base_frontmost_projected_color": [112, 115, 119, 255],
                "base_frontmost_texture_as_linear_color": [90, 92, 95, 255],
                "base_color_rendered_rgb_distance": 12.5,
                "texture_as_linear_rendered_rgb_distance": 4.5
            }]
        }"#,
    )?;
    let material_track_inputs_path = root.join("Seed-san.material-track-inputs.json");
    fs::write(
        &material_track_inputs_path,
        r#"{
            "fixture": ".external-fixtures/official/Seed-san.vrm",
            "material_name_filters": ["backpack_nm", "eye", "arm_mat"],
            "material_count": 17,
            "selected_count": 2,
            "selected_materials": [{
                "index": 14,
                "name": "backpack_nm",
                "branch": "gltf_pbr",
                "primitive_count": 1,
                "base_color_factor": [1.0, 1.0, 1.0, 1.0],
                "metallic_factor": 0.0,
                "roughness_factor": 0.657,
                "alpha_mode": "Opaque",
                "double_sided": false,
                "unlit": false,
                "emissive_factor": [0.0, 0.0, 0.0],
                "emissive_strength": 1.0,
                "normal_scale": 1.0,
                "textures": [{
                    "slot": "baseColorTexture",
                    "texture": 12,
                    "image": {"index": 12, "name": "backpack"},
                    "sampler": {"min_filter": 9985}
                }, {
                    "slot": "normalTexture",
                    "texture": 13,
                    "image": {"index": 13, "name": "nm_backpack_normals"},
                    "sampler": {"min_filter": 9985}
                }]
            }, {
                "index": 3,
                "name": "eye",
                "branch": "mtoon",
                "primitive_count": 1,
                "base_color_factor": [1.0, 1.0, 1.0, 1.0],
                "metallic_factor": 1.0,
                "roughness_factor": 1.0,
                "alpha_mode": "Opaque",
                "double_sided": false,
                "unlit": true,
                "emissive_factor": [0.0, 0.0, 0.0],
                "emissive_strength": 1.0,
                "normal_scale": null,
                "textures": [{
                    "slot": "baseColorTexture",
                    "texture": 7,
                    "image": {"index": 7, "name": "faceparts"},
                    "sampler": {"min_filter": 9985}
                }],
                "mtoon": {
                    "shade_color_factor": [0.435, 0.397, 0.501],
                    "shading_shift_factor": -0.2,
                    "shading_toony_factor": 0.8,
                    "gi_equalization_factor": 0.9,
                    "parametric_rim_color_factor": [0.0, 0.0, 0.0],
                    "rim_lighting_mix_factor": 1.0,
                    "outline_width_mode": "none",
                    "outline_width_factor": 0.5,
                    "textures": [{
                        "slot": "shadeMultiplyTexture",
                        "texture": 7,
                        "image": {"index": 7, "name": "faceparts"},
                        "sampler": {"min_filter": 9985}
                    }]
                }
            }]
        }"#,
    )?;
    let texture_audit_path = root.join("Seed-san.wgpu-texture-sampling-audit.json");
    fs::write(
        &texture_audit_path,
        r#"{
            "selection_source_buckets": [{
                "selection_source": "webgl-coverage",
                "stats": {
                    "count": 6,
                    "mean_expected_actual_rgb_distance": 51.75,
                    "manifest_sample_actual_closer": 2,
                    "manifest_sample_expected_closer": 4,
                    "manifest_sample_tied": 0,
                    "manifest_sample_actual_within_1_5": 0,
                    "manifest_sample_expected_within_1_5": 1,
                    "mean_manifest_sample_actual_rgb_distance": 107.9,
                    "mean_manifest_sample_expected_rgb_distance": 75.6,
                    "mean_actual_minus_manifest_sample_rgb_delta": [-19.0, -20.0, -21.0],
                    "mean_expected_minus_manifest_sample_rgb_delta": [-5.0, -3.0, -2.0],
                    "material_counts": [
                        {"material_name": "arm_plastic", "count": 2},
                        {"material_name": "backpack_nm", "count": 2}
                    ],
                    "selection_material_draw_buckets": [{
                        "material_name": "arm_plastic",
                        "draw_key": "node144/mesh3/prim1/base",
                        "stats": {"count": 2}
                    }]
                }
            }],
            "recommended_probes": [{
                "material_name": "backpack_nm",
                "draw_key": "node145/mesh4/prim9/base",
                "count": 15,
                "classification": "selected_sample_and_renderer_both_far",
                "action": "audit resolve draw binding or selected-surface material inputs",
                "mean_expected_actual_rgb_distance": 35.8,
                "mean_manifest_actual_rgb_distance": 70.6,
                "mean_manifest_expected_rgb_distance": 106.2,
                "mean_actual_minus_manifest_rgb_delta": [37.5, 41.1, 43.1],
                "mean_expected_minus_manifest_rgb_delta": [56.0, 62.1, 65.4],
                "least_squares_actual_over_manifest_rgb_gain": [1.74, 1.90, 1.94],
                "least_squares_expected_over_manifest_rgb_gain": [2.11, 2.36, 2.43],
                "manifest_sample_actual_within_1_5": 0,
                "manifest_sample_expected_within_1_5": 0,
                "manifest_sample_both_far": 12
            }]
        }"#,
    )?;
    let focused_pixels_path = root.join("Seed-san.wgpu-focused-material-pixels.json");
    fs::write(
        &focused_pixels_path,
        r#"{
            "actual_source": "expanded-readback",
            "rows": [{
                "x": 141,
                "y": 90,
                "interpretation": "selected sample is closer to three-vrm expected",
                "selected_surface": {"material_name": "backpack_nm", "triangle": 42},
                "selection_source": "center",
                "selected_rgba": [208, 211, 213, 255],
                "actual": [77, 74, 76, 255],
                "expected": [208, 211, 213, 255],
                "actual_expected_rgb_distance": 224.0,
                "selected_actual_rgb_distance": 224.0,
                "selected_expected_rgb_distance": 0.0,
                "browser_material": {
                    "material_name": "backpack_nm",
                    "material_type": "MeshStandardMaterial",
                    "mesh_name": "wear_10",
                    "pass": "base",
                    "color": [1.0, 1.0, 1.0],
                    "map_name": "backpack",
                    "map_color_space": "srgb",
                    "map_flip_y": false,
                    "map_min_filter": 1007,
                    "map_mag_filter": 1006
                },
                "renderer_material_draw": {
                    "draw_role": "owner-sample-resolve",
                    "material_name": "backpack_nm",
                    "material_index": 14,
                    "alpha_mode": "opaque",
                    "cull_mode": "off",
                    "depth_write": false,
                    "blend": false,
                    "shader_branch": "gltf_pbr",
                    "pbr_fallback": false,
                    "metallic": 0.0,
                    "roughness": 0.657,
                    "occlusion_strength": 1.0,
                    "base_texture": 12,
                    "normal_texture": 13,
                    "base_color": [1.0, 1.0, 1.0, 1.0],
                    "shade_color": [1.0, 1.0, 1.0, 1.0],
                    "shading_shift": 0.0,
                    "shading_toony": 0.0,
                    "gi_equalization": 0.0
                },
                "frontmost": {"surface": {"material_name": "backpack_nm", "triangle": 42}},
                "nearest_expected": {"surface": {"material_name": "arm_plastic", "triangle": 7}}
            }]
        }"#,
    )?;
    let options = Options {
        self_test: false,
        reports_dir: Some(root.clone()),
        fixture_stem: "Seed-san".to_string(),
        suffix: "render-resolve-expanded.gradient".to_string(),
        renderer: vec!["wgpu".to_string()],
        metric_key: "rgbSharedNonblackGradientInterior1px".to_string(),
        shading_model_join: Some(join_path),
        material_track_inputs: Some(material_track_inputs_path),
        texture_audit: vec![format!("wgpu={}", texture_audit_path.display())],
        focused_material_pixels: vec![format!("wgpu={}", focused_pixels_path.display())],
        base_color_owner_join: vec![format!(
            "wgpu={}",
            base_color_owner_join_path.display()
        )],
        json_out: None,
        markdown_out: None,
    };
    let summary = summarize(&options, &root)?;
    assert_eq!(summary.renderers.len(), 1);
    assert_eq!(summary.renderers[0].psnr, 42.5);
    assert_eq!(
        summary.renderers[0].top_selected_materials[0].material,
        "mat"
    );
    let color_fit = summary
        .shading_model_join
        .as_ref()
        .and_then(|join| join.models.first())
        .and_then(|model| model.backends.first())
        .and_then(|backend| backend.color_fit.as_ref())
        .ok_or("self-test shading model color_fit was not parsed")?;
    assert_eq!(color_fit.preferred_fit, "additive");
    assert_eq!(color_fit.additive_fit_mean_distance, Some(1.0));
    assert_eq!(color_fit.gain_fit_mean_distance, Some(2.0));
    let material_color_fit = summary
        .shading_model_join
        .as_ref()
        .and_then(|join| join.models.first())
        .and_then(|model| model.backends.first())
        .and_then(|backend| backend.material_draw_color_fits.first())
        .and_then(|fit| fit.color_fit.as_ref())
        .ok_or("self-test material/draw color_fit was not parsed")?;
    assert_eq!(material_color_fit.additive_fit_mean_distance, Some(1.0));
    assert_eq!(material_color_fit.gain_fit_mean_distance, Some(2.0));
    let shading_input = summary
        .shading_model_join
        .as_ref()
        .and_then(|join| join.models.first())
        .and_then(|model| model.backends.first())
        .and_then(|backend| backend.material_draw_shading_inputs.first())
        .ok_or("self-test shading input was not parsed")?;
    assert_eq!(shading_input.mean_base_color, Some([1.0, 0.5, 0.25, 1.0]));
    let summary_json = serde_json::to_string(&summary)?;
    assert!(summary_json.contains(r#""preferred_fit":"additive""#));
    assert!(summary_json.contains(r#""additive_fit_mean_rgb_distance":1.0"#));
    assert!(summary_json.contains(r#""gain_fit_mean_rgb_distance":2.0"#));
    assert!(!summary_json.contains(r#""additive_fit_mean_distance""#));
    assert!(!summary_json.contains(r#""gain_fit_mean_distance""#));
    assert!(summary_json.contains(r#""material_draw_shading_inputs""#));
    assert!(summary_json.contains(r#""material_track_inputs""#));
    assert!(summary_json.contains(r#""texture_audits""#));
    assert!(summary_json.contains(r#""selection_source_buckets""#));
    assert!(summary_json.contains(r#""selection_source":"webgl-coverage""#));
    assert!(summary_json.contains(r#""recommended_probes""#));
    assert!(summary_json.contains(r#""least_squares_actual_over_manifest_rgb_gain":[1.74,1.9,1.94]"#));
    assert!(summary_json.contains(r#""least_squares_expected_over_manifest_rgb_gain":[2.11,2.36,2.43]"#));
    assert!(summary_json.contains(r#""focused_material_pixels""#));
    assert!(summary_json.contains(r#""browser_material":"backpack_nm MeshStandardMaterial"#));
    assert!(summary_json.contains(r#""base_texture":"baseColorTexture:tex#12:backpack min=9985""#));
    assert!(summary_json.contains(r#""base_color_owner_joins""#));
    assert!(summary_json.contains(r#""projected_base_color":[112,115,119,255]"#));
    assert!(!summary_json.contains(r#""color_fit":null"#));
    let markdown = render_markdown(&summary);
    assert!(markdown.contains("| wgpu | 42.5000 |"));
    assert!(markdown.contains("## Shading Model Backend Agreement"));
    assert!(markdown.contains("#### Backend Color Fit"));
    assert!(markdown.contains("#### Material / Draw Color Fit"));
    assert!(markdown.contains(
        "| wgpu | additive | 1.00,2.00,3.00 | 1.0000 | 1.05,1.10,1.15 | 2.0000 | 1.10,1.20,1.30 |"
    ));
    assert!(markdown.contains("| wgpu | backpack_nm | node145/mesh4/prim9/base | 2 | 4.5000 | 1.00,2.00,3.00 | additive | 1.00,2.00,3.00 | 1.0000 | 1.05,1.10,1.15 | 2.0000 |"));
    assert!(markdown.contains("#### Material / Draw Shading Inputs"));
    assert!(markdown.contains("| wgpu | backpack_nm | node145/mesh4/prim9/base | 2 | gltf_pbr:2 | 1.00,0.50,0.25,1.00 |"));
    assert!(markdown.contains("## Material Track Inputs"));
    assert!(markdown.contains("backpack_nm#14"));
    assert!(markdown.contains("baseColorTexture:tex#12:backpack min=9985"));
    assert!(markdown.contains("shade=0.43,0.40,0.50 shift/toony/gi=-0.200/0.800/0.900"));
    assert!(markdown.contains("## Recommended Material Probes"));
    assert!(markdown.contains("#### Selection Source Buckets"));
    assert!(markdown.contains("| webgl-coverage | 6 | 51.7500 | 2/4/0 | 0 / 1 | 107.9000 / 75.6000 | -19.00,-20.00,-21.00 / -5.00,-3.00,-2.00 |"));
    assert!(markdown.contains("LS gain A/M"));
    assert!(markdown.contains("| backpack_nm | node145/mesh4/prim9/base | 15 | selected_sample_and_renderer_both_far | audit resolve draw binding or selected-surface material inputs | 35.8000 | 70.6000 / 106.2000 | 37.50,41.10,43.10 | 56.00,62.10,65.40 | 1.74,1.90,1.94 | 2.11,2.36,2.43 | 0/0/12 |"));
    assert!(markdown.contains("selected_sample_and_renderer_both_far"));
    assert!(markdown.contains("## Focused Material State Matrix"));
    assert!(markdown.contains(
        "| wgpu | 141,90 | backpack_nm#42 | center | backpack_nm | backpack_nm | yes | owner-sample-resolve |"
    ));
    assert!(markdown.contains("## Focused Material Pixels"));
    assert!(markdown.contains("backpack_nm MeshStandardMaterial mesh=wear_10 pass=base map=backpack cs=srgb"));
    assert!(markdown.contains("| 141,90 | selected sample is closer to three-vrm expected | backpack_nm#42 | center | backpack_nm MeshStandardMaterial"));
    assert!(markdown.contains(
        "backpack_nm@owner-sample-resolve branch:gltf_pbr m/r/o/d=0.0000/0.6570/1.0000/n/a"
    ));
    assert_eq!(
        focused_material_shader_branch(&serde_json::json!({"pbr_fallback": false}))?,
        "n/a"
    );
    assert!(markdown.contains("## Browser Projected Base-Color Joins"));
    assert!(markdown.contains("| 106,131 | backpack_nm | backpack_nm | arm_plastic | 2 | 112,115,119,255 / 90,92,95,255 | 12.5000 / 4.5000 |"));
    assert!(markdown.contains("| ash / wgpu | 2 | 0.5000 | 0.2500 |"));
    fs::remove_dir_all(root)?;
    Ok(())
}
