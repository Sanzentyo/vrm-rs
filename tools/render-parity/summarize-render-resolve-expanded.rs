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
    out
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
    let options = Options {
        self_test: false,
        reports_dir: Some(root.clone()),
        fixture_stem: "Seed-san".to_string(),
        suffix: "render-resolve-expanded.gradient".to_string(),
        renderer: vec!["wgpu".to_string()],
        metric_key: "rgbSharedNonblackGradientInterior1px".to_string(),
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
    assert!(render_markdown(&summary).contains("| wgpu | 42.5000 |"));
    fs::remove_dir_all(root)?;
    Ok(())
}
