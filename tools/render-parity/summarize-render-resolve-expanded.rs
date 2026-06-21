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
    materials: String,
    draw_keys: String,
}

#[derive(Clone, Debug, Serialize)]
struct ShadingModelColorFitSummary {
    preferred_fit: String,
    additive_rgb_delta: Option<[f64; 3]>,
    additive_fit_mean_distance: Option<f64>,
    least_squares_gain_rgb: Option<[f64; 3]>,
    gain_fit_mean_distance: Option<f64>,
    mean_expected_over_actual_rgb_ratio: Option<[f64; 3]>,
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
    out
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
        materials: key_count_list(value, &["materials"])?,
        draw_keys: key_count_list(value, &["draw_keys"])?,
    })
}

fn optional_color_fit(
    backend: &Value,
) -> Result<Option<ShadingModelColorFitSummary>, Box<dyn Error>> {
    let Some(value) = backend.get("color_fit") else {
        return Ok(None);
    };
    Ok(Some(ShadingModelColorFitSummary {
        preferred_fit: get_str_path(value, &["preferred_fit"])?,
        additive_rgb_delta: optional_vec3_path(value, &["additive_rgb_delta"])?,
        additive_fit_mean_distance: optional_f64_path(value, &["additive_fit_mean_rgb_distance"])?,
        least_squares_gain_rgb: optional_vec3_path(value, &["least_squares_gain_rgb"])?,
        gain_fit_mean_distance: optional_f64_path(value, &["gain_fit_mean_rgb_distance"])?,
        mean_expected_over_actual_rgb_ratio: optional_vec3_path(
            value,
            &["mean_expected_over_actual_rgb_ratio"],
        )?,
    }))
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
        .map(|value| format!("{:.2},{:.2},{:.2}", value[0], value[1], value[2]))
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
                    "color_fit": {
                        "mean_expected_over_actual_rgb_ratio": [1.1, 1.2, 1.3],
                        "least_squares_gain_rgb": [1.05, 1.10, 1.15],
                        "gain_fit_mean_rgb_distance": 2.0,
                        "additive_rgb_delta": [1.0, 2.0, 3.0],
                        "additive_fit_mean_rgb_distance": 1.0,
                        "preferred_fit": "additive"
                    },
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
    let options = Options {
        self_test: false,
        reports_dir: Some(root.clone()),
        fixture_stem: "Seed-san".to_string(),
        suffix: "render-resolve-expanded.gradient".to_string(),
        renderer: vec!["wgpu".to_string()],
        metric_key: "rgbSharedNonblackGradientInterior1px".to_string(),
        shading_model_join: Some(join_path),
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
    let summary_json = serde_json::to_string(&summary)?;
    assert!(summary_json.contains(r#""preferred_fit":"additive""#));
    assert!(!summary_json.contains(r#""color_fit":null"#));
    let markdown = render_markdown(&summary);
    assert!(markdown.contains("| wgpu | 42.5000 |"));
    assert!(markdown.contains("## Shading Model Backend Agreement"));
    assert!(markdown.contains("#### Backend Color Fit"));
    assert!(markdown.contains(
        "| wgpu | additive | 1.00,2.00,3.00 | 1.0000 | 1.05,1.10,1.15 | 2.0000 | 1.10,1.20,1.30 |"
    ));
    assert!(markdown.contains("| ash / wgpu | 2 | 0.5000 | 0.2500 |"));
    fs::remove_dir_all(root)?;
    Ok(())
}
