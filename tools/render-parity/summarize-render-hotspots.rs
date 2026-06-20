#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Summarize `map-render-hotspots.rs` reports into compact review artifacts.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "summarize-render-hotspots",
    about = "Summarize render hotspot projection reports"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    input: Option<PathBuf>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
    #[arg(long, default_value_t = 12)]
    top: usize,
    #[arg(long)]
    min_hotspot_count: Option<u64>,
    #[arg(long)]
    max_hotspot_count: Option<u64>,
    #[arg(long)]
    min_frontmost_visible_count: Option<u64>,
    #[arg(long)]
    min_nearest_sample_visible_frontmost_count: Option<u64>,
    #[arg(long)]
    min_missing_center_recovered_by_nearest_visible_count: Option<u64>,
    #[arg(long)]
    max_frontmost_base_texture_local_rgb_gradient_gte_32: Option<u64>,
    #[arg(long)]
    max_frontmost_max_base_texture_local_rgb_gradient: Option<f64>,
    #[arg(long)]
    min_texture_distance_actual_closer: Option<u64>,
    #[arg(long)]
    min_texture_distance_expected_closer: Option<u64>,
    #[arg(long)]
    max_actual_expected_different_pass_count: Option<u64>,
    #[arg(long)]
    max_actual_expected_different_material_count: Option<u64>,
    #[arg(long)]
    max_actual_expected_different_triangle_count: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
struct ReviewReport {
    input: String,
    fixture: Option<String>,
    deltas: Option<String>,
    size: Option<[u64; 2]>,
    sample_center: Option<[f64; 2]>,
    hotspot_count: u64,
    frontmost_visible_count: Option<u64>,
    nearest_sample_visible_frontmost_count: Option<u64>,
    missing_center_recovered_by_nearest_visible_count: Option<u64>,
    frontmost_edge_lte_025px: Option<u64>,
    frontmost_edge_lte_050px: Option<u64>,
    frontmost_edge_lte_100px: Option<u64>,
    actual_frontmost_material_matches: Option<u64>,
    expected_frontmost_material_matches: Option<u64>,
    actual_frontmost_triangle_matches: Option<u64>,
    expected_frontmost_triangle_matches: Option<u64>,
    actual_frontmost_edge_neighbor_matches: Option<u64>,
    expected_frontmost_edge_neighbor_matches: Option<u64>,
    actual_expected_same_pass_matches: Option<u64>,
    actual_expected_same_material_matches: Option<u64>,
    actual_expected_same_triangle_matches: Option<u64>,
    actual_frontmost_pass_matches: Option<u64>,
    expected_frontmost_pass_matches: Option<u64>,
    actual_frontmost_mean_base_texture_rgb_distance: Option<f64>,
    expected_frontmost_mean_base_texture_rgb_distance: Option<f64>,
    actual_frontmost_max_base_texture_rgb_distance: Option<f64>,
    expected_frontmost_max_base_texture_rgb_distance: Option<f64>,
    actual_nearest_sample_visible_mean_base_texture_rgb_distance: Option<f64>,
    expected_nearest_sample_visible_mean_base_texture_rgb_distance: Option<f64>,
    actual_nearest_sample_visible_max_base_texture_rgb_distance: Option<f64>,
    expected_nearest_sample_visible_max_base_texture_rgb_distance: Option<f64>,
    actual_missing_center_nearest_visible_mean_base_texture_rgb_distance: Option<f64>,
    expected_missing_center_nearest_visible_mean_base_texture_rgb_distance: Option<f64>,
    actual_missing_center_nearest_visible_max_base_texture_rgb_distance: Option<f64>,
    expected_missing_center_nearest_visible_max_base_texture_rgb_distance: Option<f64>,
    actual_frontmost_mean_uv_distance: Option<f64>,
    expected_frontmost_mean_uv_distance: Option<f64>,
    actual_frontmost_max_uv_distance: Option<f64>,
    expected_frontmost_max_uv_distance: Option<f64>,
    frontmost_mean_base_texture_local_rgb_gradient: Option<f64>,
    frontmost_max_base_texture_local_rgb_gradient: Option<f64>,
    frontmost_base_texture_local_rgb_gradient_gte_32: Option<u64>,
    frontmost_base_texture_local_rgb_gradient_gte_64: Option<u64>,
    frontmost_base_texture_local_rgb_gradient_gte_96: Option<u64>,
    texture_distance_advantage: TextureDistanceAdvantage,
    top_actual_surface_transitions: Vec<Value>,
    top_expected_surface_transitions: Vec<Value>,
    top_actual_expected_surface_transitions: Vec<Value>,
    top_frontmost_edges: Vec<Value>,
    top_nearest_sample_offsets: Vec<Value>,
    top_missing_center_nearest_offsets: Vec<Value>,
    top_hotspots: Vec<HotspotLine>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct TextureDistanceAdvantage {
    actual_closer: u64,
    expected_closer: u64,
    tied: u64,
    compared: u64,
    mean_expected_minus_actual: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
struct HotspotLine {
    x: Option<u64>,
    y: Option<u64>,
    max_channel_delta: Option<u64>,
    rgb_distance: Option<f64>,
    actual: Option<[u64; 4]>,
    expected: Option<[u64; 4]>,
    frontmost: Option<SurfaceLine>,
    actual_frontmost_base_texture_rgb_distance: Option<f64>,
    expected_frontmost_base_texture_rgb_distance: Option<f64>,
    frontmost_base_texture_local_rgb_gradient: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
struct SurfaceLine {
    pass: Option<String>,
    material_name: Option<String>,
    node: Option<u64>,
    mesh: Option<u64>,
    primitive: Option<u64>,
    triangle: Option<u64>,
    edge_distance_pixels: Option<f64>,
    nearest_edge: Option<u64>,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    if options.self_test {
        self_test()?;
        return Ok(());
    }
    let input = options.input.as_ref().ok_or("missing --input")?;
    let value = serde_json::from_str::<Value>(&fs::read_to_string(input)?)?;
    let report = summarize_report(input, &value, options.top)?;
    let json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = &options.json_out {
        write_file(path, &json)?;
    } else {
        print!("{json}");
    }
    if let Some(path) = &options.markdown_out {
        write_file(path, &markdown_report(&report))?;
    }
    validate_thresholds(&report, &options)?;
    Ok(())
}

fn validate_thresholds(
    report: &ReviewReport,
    options: &Options,
) -> Result<(), Box<dyn std::error::Error>> {
    check_min_u64(
        "hotspot_count",
        Some(report.hotspot_count),
        options.min_hotspot_count,
    )?;
    check_max_u64(
        "hotspot_count",
        Some(report.hotspot_count),
        options.max_hotspot_count,
    )?;
    check_min_u64(
        "frontmost_visible_count",
        report.frontmost_visible_count,
        options.min_frontmost_visible_count,
    )?;
    check_min_u64(
        "nearest_sample_visible_frontmost_count",
        report.nearest_sample_visible_frontmost_count,
        options.min_nearest_sample_visible_frontmost_count,
    )?;
    check_min_u64(
        "missing_center_recovered_by_nearest_visible_count",
        report.missing_center_recovered_by_nearest_visible_count,
        options.min_missing_center_recovered_by_nearest_visible_count,
    )?;
    check_max_u64(
        "frontmost_base_texture_local_rgb_gradient_gte_32",
        report.frontmost_base_texture_local_rgb_gradient_gte_32,
        options.max_frontmost_base_texture_local_rgb_gradient_gte_32,
    )?;
    check_max_f64(
        "frontmost_max_base_texture_local_rgb_gradient",
        report.frontmost_max_base_texture_local_rgb_gradient,
        options.max_frontmost_max_base_texture_local_rgb_gradient,
    )?;
    check_min_u64(
        "texture_distance_actual_closer",
        Some(report.texture_distance_advantage.actual_closer),
        options.min_texture_distance_actual_closer,
    )?;
    check_min_u64(
        "texture_distance_expected_closer",
        Some(report.texture_distance_advantage.expected_closer),
        options.min_texture_distance_expected_closer,
    )?;
    check_max_difference(
        "actual_expected_different_pass_count",
        report.hotspot_count,
        report.actual_expected_same_pass_matches,
        options.max_actual_expected_different_pass_count,
    )?;
    check_max_difference(
        "actual_expected_different_material_count",
        report.hotspot_count,
        report.actual_expected_same_material_matches,
        options.max_actual_expected_different_material_count,
    )?;
    check_max_difference(
        "actual_expected_different_triangle_count",
        report.hotspot_count,
        report.actual_expected_same_triangle_matches,
        options.max_actual_expected_different_triangle_count,
    )?;
    Ok(())
}

fn check_max_difference(
    metric: &'static str,
    total: u64,
    same: Option<u64>,
    max: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(max) = max else {
        return Ok(());
    };
    let same = same.ok_or_else(|| format!("{metric} is missing; cannot apply max {max}"))?;
    let actual = total.saturating_sub(same);
    if actual > max {
        return Err(format!("{metric} {actual} exceeds max {max}").into());
    }
    Ok(())
}

fn check_min_u64(
    metric: &'static str,
    actual: Option<u64>,
    min: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(min) = min else {
        return Ok(());
    };
    let actual = actual.ok_or_else(|| format!("{metric} is missing; cannot apply min {min}"))?;
    if actual < min {
        return Err(format!("{metric} {actual} is below min {min}").into());
    }
    Ok(())
}

fn check_max_u64(
    metric: &'static str,
    actual: Option<u64>,
    max: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(max) = max else {
        return Ok(());
    };
    let actual = actual.ok_or_else(|| format!("{metric} is missing; cannot apply max {max}"))?;
    if actual > max {
        return Err(format!("{metric} {actual} exceeds max {max}").into());
    }
    Ok(())
}

fn check_max_f64(
    metric: &'static str,
    actual: Option<f64>,
    max: Option<f64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(max) = max else {
        return Ok(());
    };
    let actual = actual.ok_or_else(|| format!("{metric} is missing; cannot apply max {max}"))?;
    if actual > max {
        return Err(format!("{metric} {actual:.6} exceeds max {max:.6}").into());
    }
    Ok(())
}

fn summarize_report(
    input: &Path,
    value: &Value,
    top: usize,
) -> Result<ReviewReport, Box<dyn std::error::Error>> {
    let hotspots = value
        .get("hotspots")
        .and_then(Value::as_array)
        .ok_or("hotspots must be an array")?;
    let summary = value.get("summary").unwrap_or(&Value::Null);
    let size = value
        .get("width")
        .and_then(Value::as_u64)
        .zip(value.get("height").and_then(Value::as_u64))
        .map(|(width, height)| [width, height]);
    Ok(ReviewReport {
        input: display_path(input),
        fixture: string_field(value, "fixture"),
        deltas: string_field(value, "deltas"),
        size,
        sample_center: number_pair(value.get("sample_center")),
        hotspot_count: hotspots.len() as u64,
        frontmost_visible_count: u64_field(summary, "frontmost_visible_count"),
        nearest_sample_visible_frontmost_count: u64_field(
            summary,
            "nearest_sample_visible_frontmost_count",
        ),
        missing_center_recovered_by_nearest_visible_count: u64_field(
            summary,
            "missing_center_recovered_by_nearest_visible_count",
        ),
        frontmost_edge_lte_025px: u64_field(summary, "frontmost_edge_distance_lte_025px"),
        frontmost_edge_lte_050px: u64_field(summary, "frontmost_edge_distance_lte_050px"),
        frontmost_edge_lte_100px: u64_field(summary, "frontmost_edge_distance_lte_100px"),
        actual_frontmost_material_matches: u64_field(
            summary,
            "actual_frontmost_material_matches",
        ),
        expected_frontmost_material_matches: u64_field(
            summary,
            "expected_frontmost_material_matches",
        ),
        actual_frontmost_triangle_matches: u64_field(
            summary,
            "actual_frontmost_triangle_matches",
        ),
        expected_frontmost_triangle_matches: u64_field(
            summary,
            "expected_frontmost_triangle_matches",
        ),
        actual_frontmost_edge_neighbor_matches: u64_field(
            summary,
            "actual_frontmost_edge_neighbor_matches",
        ),
        expected_frontmost_edge_neighbor_matches: u64_field(
            summary,
            "expected_frontmost_edge_neighbor_matches",
        ),
        actual_expected_same_pass_matches: u64_field(
            summary,
            "actual_expected_same_pass_matches",
        ),
        actual_expected_same_material_matches: u64_field(
            summary,
            "actual_expected_same_material_matches",
        ),
        actual_expected_same_triangle_matches: u64_field(
            summary,
            "actual_expected_same_triangle_matches",
        ),
        actual_frontmost_pass_matches: u64_field(summary, "actual_frontmost_pass_matches"),
        expected_frontmost_pass_matches: u64_field(summary, "expected_frontmost_pass_matches"),
        actual_frontmost_mean_base_texture_rgb_distance: f64_field(
            summary,
            "actual_frontmost_mean_base_texture_rgb_distance",
        ),
        expected_frontmost_mean_base_texture_rgb_distance: f64_field(
            summary,
            "expected_frontmost_mean_base_texture_rgb_distance",
        ),
        actual_frontmost_max_base_texture_rgb_distance: f64_field(
            summary,
            "actual_frontmost_max_base_texture_rgb_distance",
        ),
        expected_frontmost_max_base_texture_rgb_distance: f64_field(
            summary,
            "expected_frontmost_max_base_texture_rgb_distance",
        ),
        actual_nearest_sample_visible_mean_base_texture_rgb_distance: f64_field(
            summary,
            "actual_nearest_sample_visible_mean_base_texture_rgb_distance",
        ),
        expected_nearest_sample_visible_mean_base_texture_rgb_distance: f64_field(
            summary,
            "expected_nearest_sample_visible_mean_base_texture_rgb_distance",
        ),
        actual_nearest_sample_visible_max_base_texture_rgb_distance: f64_field(
            summary,
            "actual_nearest_sample_visible_max_base_texture_rgb_distance",
        ),
        expected_nearest_sample_visible_max_base_texture_rgb_distance: f64_field(
            summary,
            "expected_nearest_sample_visible_max_base_texture_rgb_distance",
        ),
        actual_missing_center_nearest_visible_mean_base_texture_rgb_distance: f64_field(
            summary,
            "actual_missing_center_nearest_visible_mean_base_texture_rgb_distance",
        ),
        expected_missing_center_nearest_visible_mean_base_texture_rgb_distance: f64_field(
            summary,
            "expected_missing_center_nearest_visible_mean_base_texture_rgb_distance",
        ),
        actual_missing_center_nearest_visible_max_base_texture_rgb_distance: f64_field(
            summary,
            "actual_missing_center_nearest_visible_max_base_texture_rgb_distance",
        ),
        expected_missing_center_nearest_visible_max_base_texture_rgb_distance: f64_field(
            summary,
            "expected_missing_center_nearest_visible_max_base_texture_rgb_distance",
        ),
        actual_frontmost_mean_uv_distance: f64_field(summary, "actual_frontmost_mean_uv_distance"),
        expected_frontmost_mean_uv_distance: f64_field(
            summary,
            "expected_frontmost_mean_uv_distance",
        ),
        actual_frontmost_max_uv_distance: f64_field(summary, "actual_frontmost_max_uv_distance"),
        expected_frontmost_max_uv_distance: f64_field(
            summary,
            "expected_frontmost_max_uv_distance",
        ),
        frontmost_mean_base_texture_local_rgb_gradient: f64_field(
            summary,
            "frontmost_mean_base_texture_local_rgb_gradient",
        ),
        frontmost_max_base_texture_local_rgb_gradient: f64_field(
            summary,
            "frontmost_max_base_texture_local_rgb_gradient",
        ),
        frontmost_base_texture_local_rgb_gradient_gte_32: u64_field(
            summary,
            "frontmost_base_texture_local_rgb_gradient_gte_32",
        ),
        frontmost_base_texture_local_rgb_gradient_gte_64: u64_field(
            summary,
            "frontmost_base_texture_local_rgb_gradient_gte_64",
        ),
        frontmost_base_texture_local_rgb_gradient_gte_96: u64_field(
            summary,
            "frontmost_base_texture_local_rgb_gradient_gte_96",
        ),
        texture_distance_advantage: texture_distance_advantage(hotspots),
        top_actual_surface_transitions: top_array(
            summary,
            "actual_frontmost_surface_transitions",
            top,
        ),
        top_expected_surface_transitions: top_array(
            summary,
            "expected_frontmost_surface_transitions",
            top,
        ),
        top_actual_expected_surface_transitions: top_array(
            summary,
            "actual_expected_surface_transitions",
            top,
        ),
        top_frontmost_edges: top_array(summary, "frontmost_nearest_edge_counts", top),
        top_nearest_sample_offsets: top_array(summary, "nearest_sample_visible_offsets", top),
        top_missing_center_nearest_offsets: top_array(
            summary,
            "missing_center_nearest_visible_offsets",
            top,
        ),
        top_hotspots: top_hotspots(hotspots, top),
    })
}

fn texture_distance_advantage(hotspots: &[Value]) -> TextureDistanceAdvantage {
    let mut report = TextureDistanceAdvantage::default();
    let mut delta_sum = 0.0_f64;
    for hotspot in hotspots {
        let actual = f64_field(hotspot, "frontmost_base_texture_actual_rgb_distance");
        let expected = f64_field(hotspot, "frontmost_base_texture_expected_rgb_distance");
        let Some((actual, expected)) = actual.zip(expected) else {
            continue;
        };
        report.compared += 1;
        delta_sum += expected - actual;
        match actual.partial_cmp(&expected).unwrap_or(Ordering::Equal) {
            Ordering::Less => report.actual_closer += 1,
            Ordering::Greater => report.expected_closer += 1,
            Ordering::Equal => report.tied += 1,
        }
    }
    if report.compared > 0 {
        report.mean_expected_minus_actual = Some(delta_sum / report.compared as f64);
    }
    report
}

fn top_hotspots(hotspots: &[Value], top: usize) -> Vec<HotspotLine> {
    hotspots
        .iter()
        .take(top)
        .map(|hotspot| HotspotLine {
            x: u64_field(hotspot, "x"),
            y: u64_field(hotspot, "y"),
            max_channel_delta: u64_field(hotspot, "max_channel_delta"),
            rgb_distance: f64_field(hotspot, "rgb_distance"),
            actual: rgba_field(hotspot, "actual"),
            expected: rgba_field(hotspot, "expected"),
            frontmost: hotspot
                .get("frontmost_visible")
                .or_else(|| hotspot.get("frontmost_alpha_visible"))
                .and_then(surface_line),
            actual_frontmost_base_texture_rgb_distance: f64_field(
                hotspot,
                "frontmost_base_texture_actual_rgb_distance",
            ),
            expected_frontmost_base_texture_rgb_distance: f64_field(
                hotspot,
                "frontmost_base_texture_expected_rgb_distance",
            ),
            frontmost_base_texture_local_rgb_gradient: hotspot
                .get("frontmost_visible")
                .or_else(|| hotspot.get("frontmost_alpha_visible"))
                .and_then(|frontmost| {
                    f64_field(frontmost, "base_texture_local_rgb_gradient")
                }),
        })
        .collect()
}

fn surface_line(value: &Value) -> Option<SurfaceLine> {
    Some(SurfaceLine {
        pass: string_field(value, "pass"),
        material_name: string_field(value, "material_name"),
        node: u64_field(value, "node"),
        mesh: u64_field(value, "mesh"),
        primitive: u64_field(value, "primitive"),
        triangle: u64_field(value, "triangle"),
        edge_distance_pixels: f64_field(value, "edge_distance_pixels"),
        nearest_edge: u64_field(value, "nearest_edge"),
    })
}

fn markdown_report(report: &ReviewReport) -> String {
    let mut markdown = String::new();
    markdown.push_str("# Render Hotspot Summary\n\n");
    markdown.push_str(&format!("- Input: `{}`\n", report.input));
    if let Some(fixture) = &report.fixture {
        markdown.push_str(&format!("- Fixture: `{fixture}`\n"));
    }
    if let Some(size) = report.size {
        markdown.push_str(&format!("- Size: `{}x{}`\n", size[0], size[1]));
    }
    markdown.push_str(&format!("- Hotspots: `{}`\n", report.hotspot_count));
    markdown.push_str(&format!(
        "- Frontmost visible: `{}`\n",
        fmt_opt_u64(report.frontmost_visible_count)
    ));
    markdown.push_str(&format!(
        "- Nearest-sample visible frontmost: `{}`; missing-center recovered: `{}`\n",
        fmt_opt_u64(report.nearest_sample_visible_frontmost_count),
        fmt_opt_u64(report.missing_center_recovered_by_nearest_visible_count)
    ));
    markdown.push_str(&format!(
        "- Edge distance <= 0.25 / 0.50 / 1.00 px: `{}` / `{}` / `{}`\n",
        fmt_opt_u64(report.frontmost_edge_lte_025px),
        fmt_opt_u64(report.frontmost_edge_lte_050px),
        fmt_opt_u64(report.frontmost_edge_lte_100px)
    ));
    markdown.push_str(&format!(
        "- Material matches actual/expected vs frontmost: `{}` / `{}`\n",
        fmt_opt_u64(report.actual_frontmost_material_matches),
        fmt_opt_u64(report.expected_frontmost_material_matches)
    ));
    markdown.push_str(&format!(
        "- Triangle matches actual/expected vs frontmost: `{}` / `{}`\n",
        fmt_opt_u64(report.actual_frontmost_triangle_matches),
        fmt_opt_u64(report.expected_frontmost_triangle_matches)
    ));
    markdown.push_str(&format!(
        "- Edge-neighbor matches actual/expected vs frontmost: `{}` / `{}`\n",
        fmt_opt_u64(report.actual_frontmost_edge_neighbor_matches),
        fmt_opt_u64(report.expected_frontmost_edge_neighbor_matches)
    ));
    markdown.push_str(&format!(
        "- Actual vs expected same pass/material/triangle: `{}` / `{}` / `{}`\n",
        fmt_opt_u64(report.actual_expected_same_pass_matches),
        fmt_opt_u64(report.actual_expected_same_material_matches),
        fmt_opt_u64(report.actual_expected_same_triangle_matches)
    ));
    markdown.push_str(&format!(
        "- Base-texture mean RGB distance actual/expected: `{}` / `{}`\n",
        fmt_opt_f64(report.actual_frontmost_mean_base_texture_rgb_distance),
        fmt_opt_f64(report.expected_frontmost_mean_base_texture_rgb_distance)
    ));
    markdown.push_str(&format!(
        "- Nearest-sample base-texture mean RGB distance actual/expected: `{}` / `{}`\n",
        fmt_opt_f64(report.actual_nearest_sample_visible_mean_base_texture_rgb_distance),
        fmt_opt_f64(report.expected_nearest_sample_visible_mean_base_texture_rgb_distance)
    ));
    markdown.push_str(&format!(
        "- Missing-center nearest-sample mean RGB distance actual/expected: `{}` / `{}`\n",
        fmt_opt_f64(report.actual_missing_center_nearest_visible_mean_base_texture_rgb_distance),
        fmt_opt_f64(report.expected_missing_center_nearest_visible_mean_base_texture_rgb_distance)
    ));
    markdown.push_str(&format!(
        "- Base UV mean/max distance actual: `{}` / `{}`; expected: `{}` / `{}`\n",
        fmt_opt_f64(report.actual_frontmost_mean_uv_distance),
        fmt_opt_f64(report.actual_frontmost_max_uv_distance),
        fmt_opt_f64(report.expected_frontmost_mean_uv_distance),
        fmt_opt_f64(report.expected_frontmost_max_uv_distance)
    ));
    markdown.push_str(&format!(
        "- Base-texture local RGB gradient mean/max: `{}` / `{}`; >=32/64/96: `{}` / `{}` / `{}`\n",
        fmt_opt_f64(report.frontmost_mean_base_texture_local_rgb_gradient),
        fmt_opt_f64(report.frontmost_max_base_texture_local_rgb_gradient),
        fmt_opt_u64(report.frontmost_base_texture_local_rgb_gradient_gte_32),
        fmt_opt_u64(report.frontmost_base_texture_local_rgb_gradient_gte_64),
        fmt_opt_u64(report.frontmost_base_texture_local_rgb_gradient_gte_96)
    ));
    markdown.push_str(&format!(
        "- Base-texture closer actual/expected/tie: `{}` / `{}` / `{}` of `{}`\n\n",
        report.texture_distance_advantage.actual_closer,
        report.texture_distance_advantage.expected_closer,
        report.texture_distance_advantage.tied,
        report.texture_distance_advantage.compared
    ));
    markdown.push_str("## Top Hotspots\n\n");
    markdown.push_str("| Pixel | Max Delta | Actual | Expected | Frontmost | Base Texture Distance A/E | Local Gradient |\n");
    markdown.push_str("| --- | ---: | --- | --- | --- | ---: | ---: |\n");
    for hotspot in &report.top_hotspots {
        markdown.push_str(&format!(
            "| {},{} | {} | {} | {} | {} | {} / {} | {} |\n",
            fmt_opt_u64(hotspot.x),
            fmt_opt_u64(hotspot.y),
            fmt_opt_u64(hotspot.max_channel_delta),
            fmt_rgba(hotspot.actual),
            fmt_rgba(hotspot.expected),
            fmt_surface(hotspot.frontmost.as_ref()),
            fmt_opt_f64(hotspot.actual_frontmost_base_texture_rgb_distance),
            fmt_opt_f64(hotspot.expected_frontmost_base_texture_rgb_distance),
            fmt_opt_f64(hotspot.frontmost_base_texture_local_rgb_gradient)
        ));
    }
    markdown.push('\n');
    markdown.push_str("## Surface Transitions\n\n");
    markdown.push_str("Actual vs frontmost:\n\n");
    markdown.push_str(&value_table(&report.top_actual_surface_transitions));
    markdown.push_str("\nExpected vs frontmost:\n\n");
    markdown.push_str(&value_table(&report.top_expected_surface_transitions));
    markdown.push_str("\nActual vs expected:\n\n");
    markdown.push_str(&value_table(&report.top_actual_expected_surface_transitions));
    markdown.push_str("\n## Frontmost Edges\n\n");
    markdown.push_str(&value_table(&report.top_frontmost_edges));
    markdown.push_str("\nNearest-sample offsets:\n\n");
    markdown.push_str(&value_table(&report.top_nearest_sample_offsets));
    markdown.push_str("\nMissing-center nearest offsets:\n\n");
    markdown.push_str(&value_table(&report.top_missing_center_nearest_offsets));
    markdown
}

fn value_table(values: &[Value]) -> String {
    if values.is_empty() {
        return "_None_\n".to_owned();
    }
    values
        .iter()
        .map(|value| format!("- `{}`\n", compact_json(value)))
        .collect()
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

fn top_array(value: &Value, key: &str, top: usize) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|values| values.iter().take(top).cloned().collect())
        .unwrap_or_default()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn f64_field(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn rgba_field(value: &Value, key: &str) -> Option<[u64; 4]> {
    let values = value.get(key)?.as_array()?;
    Some([
        values.first()?.as_u64()?,
        values.get(1)?.as_u64()?,
        values.get(2)?.as_u64()?,
        values.get(3)?.as_u64()?,
    ])
}

fn number_pair(value: Option<&Value>) -> Option<[f64; 2]> {
    let values = value?.as_array()?;
    Some([values.first()?.as_f64()?, values.get(1)?.as_f64()?])
}

fn fmt_surface(surface: Option<&SurfaceLine>) -> String {
    let Some(surface) = surface else {
        return "none".to_owned();
    };
    format!(
        "{}/m{}/p{}/tri{}/edge{}px",
        surface.material_name.as_deref().unwrap_or("unknown"),
        fmt_opt_u64(surface.mesh),
        fmt_opt_u64(surface.primitive),
        fmt_opt_u64(surface.triangle),
        fmt_opt_f64(surface.edge_distance_pixels)
    )
}

fn fmt_rgba(value: Option<[u64; 4]>) -> String {
    value.map_or_else(
        || "none".to_owned(),
        |rgba| format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3]),
    )
}

fn fmt_opt_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |value| value.to_string())
}

fn fmt_opt_f64(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |value| format!("{value:.4}"))
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn self_test() -> Result<(), Box<dyn std::error::Error>> {
    let value = serde_json::from_str::<Value>(
        r#"{
            "fixture": "fixture.vrm",
            "deltas": "deltas.json",
            "width": 2,
            "height": 2,
            "sample_center": [0.5, 0.5],
            "summary": {
                "frontmost_visible_count": 1,
                "nearest_sample_visible_frontmost_count": 1,
                "missing_center_recovered_by_nearest_visible_count": 0,
                "frontmost_edge_distance_lte_025px": 1,
                "frontmost_edge_distance_lte_050px": 1,
                "frontmost_edge_distance_lte_100px": 1,
                "actual_frontmost_material_matches": 1,
                "expected_frontmost_material_matches": 0,
                "actual_frontmost_triangle_matches": 1,
                "expected_frontmost_triangle_matches": 0,
                "actual_frontmost_edge_neighbor_matches": 1,
                "expected_frontmost_edge_neighbor_matches": 1,
                "actual_expected_same_pass_matches": 1,
                "actual_expected_same_material_matches": 1,
                "actual_expected_same_triangle_matches": 0,
                "actual_frontmost_pass_matches": 1,
                "expected_frontmost_pass_matches": 1,
                "actual_frontmost_mean_base_texture_rgb_distance": 1.0,
                "expected_frontmost_mean_base_texture_rgb_distance": 2.0,
                "actual_nearest_sample_visible_mean_base_texture_rgb_distance": 1.5,
                "expected_nearest_sample_visible_mean_base_texture_rgb_distance": 2.5,
                "actual_nearest_sample_visible_max_base_texture_rgb_distance": 3.0,
                "expected_nearest_sample_visible_max_base_texture_rgb_distance": 4.0,
                "actual_missing_center_nearest_visible_mean_base_texture_rgb_distance": null,
                "expected_missing_center_nearest_visible_mean_base_texture_rgb_distance": null,
                "actual_missing_center_nearest_visible_max_base_texture_rgb_distance": null,
                "expected_missing_center_nearest_visible_max_base_texture_rgb_distance": null,
                "actual_frontmost_mean_uv_distance": 0.1,
                "expected_frontmost_mean_uv_distance": 0.2,
                "actual_frontmost_max_uv_distance": 0.3,
                "expected_frontmost_max_uv_distance": 0.4,
                "frontmost_mean_base_texture_local_rgb_gradient": 10.0,
                "frontmost_max_base_texture_local_rgb_gradient": 12.0,
                "frontmost_base_texture_local_rgb_gradient_gte_32": 0,
                "frontmost_base_texture_local_rgb_gradient_gte_64": 0,
                "frontmost_base_texture_local_rgb_gradient_gte_96": 0,
                "actual_frontmost_surface_transitions": [{"count": 1}],
                "expected_frontmost_surface_transitions": [{"count": 1}],
                "actual_expected_surface_transitions": [{"count": 1}],
                "frontmost_nearest_edge_counts": [{"count": 1}],
                "nearest_sample_visible_offsets": [{"sample_offset": [0, 0], "count": 1}],
                "missing_center_nearest_visible_offsets": []
            },
            "hotspots": [{
                "x": 1,
                "y": 1,
                "max_channel_delta": 7,
                "rgb_distance": 7.0,
                "actual": [1, 2, 3, 255],
                "expected": [4, 5, 6, 255],
                "frontmost_base_texture_actual_rgb_distance": 1.0,
                "frontmost_base_texture_expected_rgb_distance": 2.0,
                "frontmost_visible": {
                    "pass": "base",
                    "material_name": "mat",
                    "node": 0,
                    "mesh": 0,
                    "primitive": 0,
                    "triangle": 0,
                    "edge_distance_pixels": 0.1,
                    "nearest_edge": 2,
                    "base_texture_local_rgb_gradient": 10.0
                }
            }]
        }"#,
    )?;
    let report = summarize_report(Path::new("self-test.json"), &value, 4)?;
    assert_eq!(report.hotspot_count, 1);
    assert_eq!(report.texture_distance_advantage.actual_closer, 1);
    assert_eq!(report.nearest_sample_visible_frontmost_count, Some(1));
    assert_eq!(report.actual_expected_same_material_matches, Some(1));
    assert_eq!(report.actual_expected_same_triangle_matches, Some(0));
    assert_eq!(
        report.actual_nearest_sample_visible_mean_base_texture_rgb_distance,
        Some(1.5)
    );
    assert!(markdown_report(&report).contains("Render Hotspot Summary"));
    let mut options = Options {
        self_test: false,
        input: Some(PathBuf::from("self-test.json")),
        json_out: None,
        markdown_out: None,
        top: 4,
        min_hotspot_count: Some(1),
        max_hotspot_count: Some(1),
        min_frontmost_visible_count: Some(1),
        min_nearest_sample_visible_frontmost_count: Some(1),
        min_missing_center_recovered_by_nearest_visible_count: Some(0),
        max_frontmost_base_texture_local_rgb_gradient_gte_32: Some(0),
        max_frontmost_max_base_texture_local_rgb_gradient: Some(12.0),
        min_texture_distance_actual_closer: Some(1),
        min_texture_distance_expected_closer: Some(0),
        max_actual_expected_different_pass_count: Some(0),
        max_actual_expected_different_material_count: Some(0),
        max_actual_expected_different_triangle_count: Some(1),
    };
    validate_thresholds(&report, &options)?;
    options.max_frontmost_max_base_texture_local_rgb_gradient = Some(11.0);
    let error = validate_thresholds(&report, &options)
        .expect_err("gradient threshold should reject excessive local texture gradients");
    assert!(error.to_string().contains(
        "frontmost_max_base_texture_local_rgb_gradient 12.000000 exceeds max 11.000000"
    ));
    Ok(())
}
