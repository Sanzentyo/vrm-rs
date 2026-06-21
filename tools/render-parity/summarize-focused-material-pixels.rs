#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Summarize selected render-parity pixels without changing renderer behavior.
//!
//! This is a Sans I/O diagnostic joiner: it reads existing hotspot and
//! owner/sample manifest JSON, then emits a compact per-pixel material/sample
//! report for human review.

use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "summarize-focused-material-pixels",
    about = "Summarize selected material/sample state for render-parity pixels"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    hotspots: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    manifest: Option<PathBuf>,
    #[arg(long, value_name = "X,Y")]
    pixel: Vec<String>,
    #[arg(long)]
    actual_rgba_json: Option<PathBuf>,
    #[arg(long, default_value = "hotspot")]
    actual_label: String,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
struct FocusReport {
    hotspots: String,
    manifest: String,
    actual_source: String,
    requested_pixels: Vec<String>,
    rows: Vec<FocusRow>,
}

#[derive(Clone, Debug, Serialize)]
struct FocusRow {
    x: u64,
    y: u64,
    hotspot_found: bool,
    manifest_found: bool,
    expected: Option<[u8; 4]>,
    actual: Option<[u8; 4]>,
    actual_expected_rgb_distance: Option<f64>,
    max_channel_delta: Option<u64>,
    rgb_distance: Option<f64>,
    selection_source: Option<String>,
    selected_surface: Option<SurfaceSummary>,
    selected_rgba: Option<[u8; 4]>,
    selected_sample: Option<[f64; 2]>,
    selected_base_uv: Option<[f64; 2]>,
    selected_raw_uv: Option<[f64; 2]>,
    selected_depth: Option<f64>,
    selected_pass: Option<String>,
    selected_draw_key: Option<String>,
    selected_actual_rgb_distance: Option<f64>,
    selected_expected_rgb_distance: Option<f64>,
    renderer_material_draw: Option<RendererMaterialDrawSummary>,
    frontmost: Option<CandidateSummary>,
    nearest_expected: Option<CandidateSummary>,
    nearest_actual: Option<CandidateSummary>,
    frontmost_actual_rgb_distance: Option<f64>,
    frontmost_expected_rgb_distance: Option<f64>,
    nearest_expected_actual_rgb_distance: Option<f64>,
    nearest_expected_expected_rgb_distance: Option<f64>,
    nearest_actual_actual_rgb_distance: Option<f64>,
    nearest_actual_expected_rgb_distance: Option<f64>,
    interpretation: String,
}

#[derive(Clone, Debug, Serialize)]
struct RendererMaterialDrawSummary {
    draw_role: Option<String>,
    material_name: String,
    material_index: Option<u64>,
    cull_mode: Option<String>,
    alpha_mode: Option<String>,
    depth_write: Option<bool>,
    blend: Option<bool>,
    pbr_fallback: Option<bool>,
    metallic: Option<f64>,
    roughness: Option<f64>,
    emissive_strength: Option<f64>,
    occlusion_strength: Option<f64>,
    base_texture: Option<u64>,
    shade_texture: Option<u64>,
    normal_texture: Option<u64>,
    base_color: Option<[f64; 4]>,
    shade_color: Option<[f64; 4]>,
    shading_shift: Option<f64>,
    shading_toony: Option<f64>,
    gi_equalization: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
struct SurfaceSummary {
    material_name: String,
    triangle: u64,
}

#[derive(Clone, Debug, Serialize)]
struct CandidateSummary {
    surface: SurfaceSummary,
    draw_index: Option<u64>,
    pass: Option<String>,
    shading_model: Option<String>,
    base_texture_rgba: Option<[u8; 4]>,
    cpu_base_color_rgba: Option<[u8; 4]>,
    base_uv: Option<[f64; 2]>,
    raw_uv: Option<[f64; 2]>,
    depth: Option<f64>,
    edge_distance_pixels: Option<f64>,
    base_texture_local_rgb_gradient: Option<f64>,
    alpha_mode: Option<String>,
    cull_mode: Option<String>,
    depth_write: Option<bool>,
    blend: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Pixel {
    x: u64,
    y: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct RgbaJsonImage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
struct RgbaJsonArtifact {
    image: RgbaJsonImage,
    material_draws_by_key: HashMap<String, RendererMaterialDrawSummary>,
}

impl RgbaJsonImage {
    fn pixel(&self, pixel: Pixel) -> Option<[u8; 4]> {
        if usize::try_from(pixel.x).ok()? >= self.width || usize::try_from(pixel.y).ok()? >= self.height {
            return None;
        }
        let index = (usize::try_from(pixel.y).ok()? * self.width + usize::try_from(pixel.x).ok()?) * 4;
        Some([
            *self.rgba.get(index)?,
            *self.rgba.get(index + 1)?,
            *self.rgba.get(index + 2)?,
            *self.rgba.get(index + 3)?,
        ])
    }
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
    let hotspots_path = options.hotspots.as_ref().ok_or("missing --hotspots")?;
    let manifest_path = options.manifest.as_ref().ok_or("missing --manifest")?;
    let hotspots = serde_json::from_str::<Value>(&fs::read_to_string(hotspots_path)?)?;
    let manifest = serde_json::from_str::<Value>(&fs::read_to_string(manifest_path)?)?;
    let actual_artifact = options
        .actual_rgba_json
        .as_ref()
        .map(|path| read_rgba_json_artifact(path))
        .transpose()?;
    let pixels = parse_pixels(&options.pixel)?;
    let actual_source = options
        .actual_rgba_json
        .as_ref()
        .map(|path| format!("{} ({})", display_path(path), options.actual_label))
        .unwrap_or_else(|| "hotspot actual".to_owned());
    let report = summarize(
        hotspots_path,
        manifest_path,
        actual_source,
        actual_artifact.as_ref(),
        &hotspots,
        &manifest,
        &pixels,
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

fn summarize(
    hotspots_path: &Path,
    manifest_path: &Path,
    actual_source: String,
    actual_artifact: Option<&RgbaJsonArtifact>,
    hotspots: &Value,
    manifest: &Value,
    pixels: &[Pixel],
) -> Result<FocusReport, Box<dyn Error>> {
    let hotspot_by_pixel = values_by_pixel(hotspot_values(hotspots)?);
    let manifest_by_pixel = values_by_pixel(array_at(manifest, "/corrections")?);
    let rows = pixels
        .iter()
        .map(|pixel| {
            let hotspot = hotspot_by_pixel.get(pixel).copied();
            let selection = manifest_by_pixel.get(pixel).copied();
            focus_row(*pixel, hotspot, selection, actual_artifact)
        })
        .collect();
    Ok(FocusReport {
        hotspots: display_path(hotspots_path),
        manifest: display_path(manifest_path),
        actual_source,
        requested_pixels: pixels
            .iter()
            .map(|pixel| format!("{},{}", pixel.x, pixel.y))
            .collect(),
        rows,
    })
}

fn focus_row(
    pixel: Pixel,
    hotspot: Option<&Value>,
    selection: Option<&Value>,
    actual_artifact: Option<&RgbaJsonArtifact>,
) -> FocusRow {
    let expected = hotspot.and_then(|value| rgba_at(value, "/expected"));
    let actual = actual_artifact
        .and_then(|artifact| artifact.image.pixel(pixel))
        .or_else(|| hotspot.and_then(|value| rgba_at(value, "/actual")));
    let selected_draw_key = selection.and_then(selection_draw_key);
    let selected_rgba = selection.and_then(|value| rgba_at(value, "/rgba"));
    let frontmost = hotspot.and_then(|value| candidate_at(value, "/frontmost_visible"));
    let nearest_expected = hotspot.and_then(|value| candidate_at(value, "/nearest_visible_expected"));
    let nearest_actual = hotspot.and_then(|value| candidate_at(value, "/nearest_visible_actual"));
    let selected_actual_rgb_distance = selected_rgba
        .zip(actual)
        .map(|(selected, actual)| rgb_distance(selected, actual));
    let selected_expected_rgb_distance = selected_rgba
        .zip(expected)
        .map(|(selected, expected)| rgb_distance(selected, expected));
    let actual_expected_rgb_distance = actual
        .zip(expected)
        .map(|(actual, expected)| rgb_distance(actual, expected));
    let frontmost_rgba = frontmost.as_ref().and_then(|candidate| candidate.cpu_base_color_rgba);
    let nearest_expected_rgba = nearest_expected
        .as_ref()
        .and_then(|candidate| candidate.cpu_base_color_rgba);
    let nearest_actual_rgba = nearest_actual
        .as_ref()
        .and_then(|candidate| candidate.cpu_base_color_rgba);
    FocusRow {
        x: pixel.x,
        y: pixel.y,
        hotspot_found: hotspot.is_some(),
        manifest_found: selection.is_some(),
        expected,
        actual,
        actual_expected_rgb_distance,
        max_channel_delta: hotspot.and_then(max_channel_delta),
        rgb_distance: hotspot.and_then(rgb_distance_field),
        selection_source: selection.and_then(selection_source),
        selected_surface: selection.and_then(|value| manifest_surface(value, "/surface")),
        selected_rgba,
        selected_sample: selection.and_then(|value| vec2_at(value, "/sample")),
        selected_base_uv: selection.and_then(|value| vec2_at(value, "/sample_geometry/base_uv")),
        selected_raw_uv: selection.and_then(|value| vec2_at(value, "/sample_geometry/raw_uv")),
        selected_depth: selection.and_then(|value| f64_at(value, "/sample_geometry/depth")),
        selected_pass: selection.and_then(|value| string_at(value, "/sample_geometry/pass")),
        selected_draw_key: selected_draw_key.clone(),
        selected_actual_rgb_distance,
        selected_expected_rgb_distance,
        renderer_material_draw: selected_draw_key.as_ref().and_then(|key| {
            actual_artifact.and_then(|artifact| artifact.material_draws_by_key.get(key).cloned())
        }),
        frontmost,
        nearest_expected,
        nearest_actual,
        frontmost_actual_rgb_distance: frontmost_rgba
            .zip(actual)
            .map(|(candidate, actual)| rgb_distance(candidate, actual)),
        frontmost_expected_rgb_distance: frontmost_rgba
            .zip(expected)
            .map(|(candidate, expected)| rgb_distance(candidate, expected)),
        nearest_expected_actual_rgb_distance: nearest_expected_rgba
            .zip(actual)
            .map(|(candidate, actual)| rgb_distance(candidate, actual)),
        nearest_expected_expected_rgb_distance: nearest_expected_rgba
            .zip(expected)
            .map(|(candidate, expected)| rgb_distance(candidate, expected)),
        nearest_actual_actual_rgb_distance: nearest_actual_rgba
            .zip(actual)
            .map(|(candidate, actual)| rgb_distance(candidate, actual)),
        nearest_actual_expected_rgb_distance: nearest_actual_rgba
            .zip(expected)
            .map(|(candidate, expected)| rgb_distance(candidate, expected)),
        interpretation: interpretation(
            actual_expected_rgb_distance,
            selected_actual_rgb_distance,
            selected_expected_rgb_distance,
            frontmost_rgba.zip(actual).map(|(candidate, actual)| rgb_distance(candidate, actual)),
            frontmost_rgba
                .zip(expected)
                .map(|(candidate, expected)| rgb_distance(candidate, expected)),
        ),
    }
}

fn interpretation(
    actual_expected: Option<f64>,
    selected_actual: Option<f64>,
    selected_expected: Option<f64>,
    frontmost_actual: Option<f64>,
    frontmost_expected: Option<f64>,
) -> String {
    if actual_expected.is_some_and(|distance| distance == 0.0) {
        return "actual matches three-vrm expected".to_owned();
    }
    if actual_expected.is_some_and(|distance| distance <= 1.5) {
        return "actual is within focused sample tolerance".to_owned();
    }
    match (
        closer(selected_actual, selected_expected),
        closer(frontmost_actual, frontmost_expected),
    ) {
        (Some("actual"), _) => "selected sample is closer to Rust actual".to_owned(),
        (Some("expected"), _) => "selected sample is closer to three-vrm expected".to_owned(),
        (_, Some("actual")) => "frontmost candidate is closer to Rust actual".to_owned(),
        (_, Some("expected")) => "frontmost candidate is closer to three-vrm expected".to_owned(),
        _ => "insufficient color-distance evidence".to_owned(),
    }
}

fn closer(left: Option<f64>, right: Option<f64>) -> Option<&'static str> {
    let (left, right) = left.zip(right)?;
    match left.partial_cmp(&right)? {
        std::cmp::Ordering::Less => Some("actual"),
        std::cmp::Ordering::Greater => Some("expected"),
        std::cmp::Ordering::Equal => Some("tie"),
    }
}

fn candidate_at(value: &Value, pointer: &str) -> Option<CandidateSummary> {
    let value = value.pointer(pointer)?;
    Some(CandidateSummary {
        surface: surface(value)?,
        draw_index: u64_at(value, "/draw_index").or_else(|| u64_at(value, "/drawIndex")),
        pass: string_at(value, "/pass"),
        shading_model: string_at(value, "/material_shading/model"),
        base_texture_rgba: rgba_at(value, "/base_texture_rgba"),
        cpu_base_color_rgba: rgba_at(value, "/cpu_base_color_rgba"),
        base_uv: vec2_at(value, "/base_uv"),
        raw_uv: vec2_at(value, "/raw_uv"),
        depth: f64_at(value, "/depth"),
        edge_distance_pixels: f64_at(value, "/edge_distance_pixels"),
        base_texture_local_rgb_gradient: f64_at(value, "/base_texture_local_rgb_gradient"),
        alpha_mode: string_at(value, "/policy/alpha_mode"),
        cull_mode: string_at(value, "/policy/cull_mode"),
        depth_write: bool_at(value, "/policy/depth_write"),
        blend: bool_at(value, "/policy/blend"),
    })
}

fn surface(value: &Value) -> Option<SurfaceSummary> {
    Some(SurfaceSummary {
        material_name: value.get("material_name")?.as_str()?.to_owned(),
        triangle: value.get("triangle")?.as_u64()?,
    })
}

fn manifest_surface(value: &Value, pointer: &str) -> Option<SurfaceSummary> {
    let value = value.pointer(pointer)?;
    Some(SurfaceSummary {
        material_name: value.get("materialName")?.as_str()?.to_owned(),
        triangle: value.get("triangle")?.as_u64()?,
    })
}

fn hotspot_values(value: &Value) -> Result<&[Value], Box<dyn Error>> {
    if let Some(values) = value.get("hotspots").and_then(Value::as_array) {
        return Ok(values);
    }
    if let Some(values) = value
        .pointer("/reference/renderer/diagnosticHotspots/top")
        .and_then(Value::as_array)
    {
        return Ok(values);
    }
    Err("hotspots JSON must contain /hotspots or /reference/renderer/diagnosticHotspots/top".into())
}

fn array_at<'a>(value: &'a Value, pointer: &str) -> Result<&'a [Value], Box<dyn Error>> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{pointer} must be an array").into())
}

fn values_by_pixel(values: &[Value]) -> HashMap<Pixel, &Value> {
    values
        .iter()
        .filter_map(|value| Some((pixel(value)?, value)))
        .collect()
}

fn pixel(value: &Value) -> Option<Pixel> {
    Some(Pixel {
        x: value.get("x")?.as_u64()?,
        y: value.get("y")?.as_u64()?,
    })
}

fn parse_pixels(values: &[String]) -> Result<Vec<Pixel>, Box<dyn Error>> {
    if values.is_empty() {
        return Err("at least one --pixel X,Y is required".into());
    }
    values
        .iter()
        .map(|value| {
            let Some((x, y)) = value.split_once(',') else {
                return Err(format!("pixel must be X,Y, got {value:?}").into());
            };
            Ok(Pixel {
                x: x.trim().parse()?,
                y: y.trim().parse()?,
            })
        })
        .collect()
}

fn rgba_at(value: &Value, pointer: &str) -> Option<[u8; 4]> {
    let values = value.pointer(pointer)?.as_array()?;
    Some([
        u8::try_from(values.first()?.as_u64()?).ok()?,
        u8::try_from(values.get(1)?.as_u64()?).ok()?,
        u8::try_from(values.get(2)?.as_u64()?).ok()?,
        u8::try_from(values.get(3)?.as_u64()?).ok()?,
    ])
}

fn vec2_at(value: &Value, pointer: &str) -> Option<[f64; 2]> {
    let values = value.pointer(pointer)?.as_array()?;
    Some([values.first()?.as_f64()?, values.get(1)?.as_f64()?])
}

fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer)?.as_str().map(ToOwned::to_owned)
}

fn selection_source(value: &Value) -> Option<String> {
    value
        .get("selection_source")
        .and_then(Value::as_str)
        .or_else(|| value.get("selectionSource").and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn selection_draw_key(value: &Value) -> Option<String> {
    let geometry = value.pointer("/sample_geometry")?;
    Some(format!(
        "node{}/mesh{}/prim{}/{}",
        geometry.get("node")?.as_u64()?,
        geometry.get("mesh")?.as_u64()?,
        geometry.get("primitive")?.as_u64()?,
        geometry.get("pass")?.as_str()?
    ))
}

fn f64_at(value: &Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer)?.as_f64()
}

fn u64_at(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer)?.as_u64()
}

fn bool_at(value: &Value, pointer: &str) -> Option<bool> {
    value.pointer(pointer)?.as_bool()
}

fn max_channel_delta(value: &Value) -> Option<u64> {
    value
        .get("max_channel_delta")
        .and_then(Value::as_u64)
        .or_else(|| value.get("maxChannelDelta").and_then(Value::as_u64))
}

fn rgb_distance_field(value: &Value) -> Option<f64> {
    value
        .get("rgb_distance")
        .and_then(Value::as_f64)
        .or_else(|| value.get("rgbDistance").and_then(Value::as_f64))
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

fn markdown(report: &FocusReport) -> String {
    let mut output = String::new();
    output.push_str("# Focused Material Pixel Summary\n\n");
    output.push_str(&format!("- Hotspots: `{}`\n", report.hotspots));
    output.push_str(&format!("- Manifest: `{}`\n", report.manifest));
    output.push_str(&format!("- Actual source: `{}`\n", report.actual_source));
    output.push_str(&format!(
        "- Requested pixels: `{}`\n\n",
        report.requested_pixels.join("`, `")
    ));
    output.push_str("| Pixel | Expected | Actual | Actual-expected | Delta / RGB | Source | Selected draw | Selected surface | Renderer material | Selected RGBA | Selected A/E | Frontmost | Front A/E | Nearest expected | NExp A/E | Edge / gradient | Interpretation |\n");
    output.push_str("| --- | --- | --- | ---: | ---: | --- | --- | --- | --- | --- | ---: | --- | ---: | --- | ---: | ---: | --- |\n");
    for row in &report.rows {
        output.push_str(&format!(
            "| {},{} | {} | {} | {} | {} / {} | {} | {} | {} | {} | {} | {} / {} | {} | {} / {} | {} | {} / {} | {} | {} |\n",
            row.x,
            row.y,
            fmt_opt_rgba(row.expected),
            fmt_opt_rgba(row.actual),
            fmt_opt(row.actual_expected_rgb_distance),
            fmt_opt_u64(row.max_channel_delta),
            fmt_opt(row.rgb_distance),
            row.selection_source.as_deref().unwrap_or("n/a"),
            row.selected_draw_key.as_deref().unwrap_or("n/a"),
            fmt_surface(row.selected_surface.as_ref()),
            fmt_renderer_material_draw(row.renderer_material_draw.as_ref()),
            fmt_opt_rgba(row.selected_rgba),
            fmt_opt(row.selected_actual_rgb_distance),
            fmt_opt(row.selected_expected_rgb_distance),
            fmt_candidate(row.frontmost.as_ref()),
            fmt_opt(row.frontmost_actual_rgb_distance),
            fmt_opt(row.frontmost_expected_rgb_distance),
            fmt_candidate(row.nearest_expected.as_ref()),
            fmt_opt(row.nearest_expected_actual_rgb_distance),
            fmt_opt(row.nearest_expected_expected_rgb_distance),
            fmt_edge_gradient(row.frontmost.as_ref()),
            row.interpretation,
        ));
    }
    output
}

fn fmt_renderer_material_draw(value: Option<&RendererMaterialDrawSummary>) -> String {
    value
        .map(|draw| {
            format!(
                "{} pbr:{} m/r/e/o={}/{}/{}/{} tex(b/s/n)={}/{}/{} base={} shade={} shift/toony/gi={}/{}/{} policy={}/{}/dw:{}/blend:{}",
                format!(
                    "{}@{}",
                    draw.material_name,
                    draw.draw_role.as_deref().unwrap_or("unknown")
                ),
                draw.pbr_fallback
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned()),
                fmt_opt(draw.metallic),
                fmt_opt(draw.roughness),
                fmt_opt(draw.emissive_strength),
                fmt_opt(draw.occlusion_strength),
                fmt_opt_u64(draw.base_texture),
                fmt_opt_u64(draw.shade_texture),
                fmt_opt_u64(draw.normal_texture),
                fmt_opt_vec4(draw.base_color),
                fmt_opt_vec4(draw.shade_color),
                fmt_opt(draw.shading_shift),
                fmt_opt(draw.shading_toony),
                fmt_opt(draw.gi_equalization),
                draw.alpha_mode.as_deref().unwrap_or("n/a"),
                draw.cull_mode.as_deref().unwrap_or("n/a"),
                draw.depth_write
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned()),
                draw.blend
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned())
            )
        })
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_candidate(value: Option<&CandidateSummary>) -> String {
    value
        .map(|candidate| {
            format!(
                "{} {} rgba={} uv={} policy={}/{}/dw:{}/blend:{}",
                fmt_surface(Some(&candidate.surface)),
                candidate.shading_model.as_deref().unwrap_or("n/a"),
                fmt_opt_rgba(candidate.cpu_base_color_rgba),
                fmt_opt_vec2(candidate.base_uv),
                candidate.alpha_mode.as_deref().unwrap_or("n/a"),
                candidate.cull_mode.as_deref().unwrap_or("n/a"),
                candidate
                    .depth_write
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned()),
                candidate
                    .blend
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned())
            )
        })
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_edge_gradient(value: Option<&CandidateSummary>) -> String {
    value
        .map(|candidate| {
            format!(
                "{} / {}",
                fmt_opt(candidate.edge_distance_pixels),
                fmt_opt(candidate.base_texture_local_rgb_gradient)
            )
        })
        .unwrap_or_else(|| "n/a / n/a".to_owned())
}

fn fmt_surface(value: Option<&SurfaceSummary>) -> String {
    value
        .map(|surface| format!("{}:tri{}", surface.material_name, surface.triangle))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_opt_rgba(value: Option<[u8; 4]>) -> String {
    value
        .map(|rgba| format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3]))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_opt_vec2(value: Option<[f64; 2]>) -> String {
    value
        .map(|value| format!("{:.6},{:.6}", value[0], value[1]))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_opt_vec4(value: Option<[f64; 4]>) -> String {
    value
        .map(|value| format!("{:.3},{:.3},{:.3},{:.3}", value[0], value[1], value[2], value[3]))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_opt(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_opt_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned())
}

fn write_file(path: &Path, content: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn read_rgba_json_artifact(path: &Path) -> Result<RgbaJsonArtifact, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let value = serde_json::from_str::<Value>(&text)?;
    let image = serde_json::from_value::<RgbaJsonImage>(value.clone())?;
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
    Ok(RgbaJsonArtifact {
        image,
        material_draws_by_key: material_draws_by_key(&value),
    })
}

fn material_draws_by_key(value: &Value) -> HashMap<String, RendererMaterialDrawSummary> {
    let mut draws = HashMap::new();
    for (key, draw) in value
        .pointer("/renderer/materialDraws")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|draw| {
            let key = string_at(draw, "/draw/key")?;
            Some((key, renderer_material_draw_summary(draw)))
        }) {
        let replace = draws
            .get(&key)
            .is_none_or(|existing| draw_role_priority(&draw) >= draw_role_priority(existing));
        if replace {
            draws.insert(key, draw);
        }
    }
    draws
}

fn draw_role_priority(draw: &RendererMaterialDrawSummary) -> u8 {
    match draw.draw_role.as_deref() {
        Some("owner-sample-resolve") => 2,
        Some("source") => 1,
        _ => 0,
    }
}

fn renderer_material_draw_summary(value: &Value) -> RendererMaterialDrawSummary {
    RendererMaterialDrawSummary {
        draw_role: string_at(value, "/draw/role"),
        material_name: string_at(value, "/material/name").unwrap_or_else(|| "n/a".to_owned()),
        material_index: u64_at(value, "/material/index"),
        cull_mode: string_at(value, "/policy/cullMode"),
        alpha_mode: string_at(value, "/policy/alphaMode"),
        depth_write: bool_at(value, "/policy/depthWrite"),
        blend: bool_at(value, "/policy/blend"),
        pbr_fallback: bool_at(value, "/materialExtra/flags/pbrFallback"),
        metallic: f64_at(value, "/materialExtra/pbr/metallic"),
        roughness: f64_at(value, "/materialExtra/pbr/roughness"),
        emissive_strength: f64_at(value, "/materialExtra/pbr/emissiveStrength"),
        occlusion_strength: f64_at(value, "/materialExtra/pbr/occlusionStrength"),
        base_texture: u64_at(value, "/textureSlots/base"),
        shade_texture: u64_at(value, "/textureSlots/shade"),
        normal_texture: u64_at(value, "/textureSlots/normal"),
        base_color: vec4_at(value, "/vertexMaterial/baseColor"),
        shade_color: vec4_at(value, "/vertexMaterial/shadeColor"),
        shading_shift: f64_at(value, "/vertexMaterial/shading/shift"),
        shading_toony: f64_at(value, "/vertexMaterial/shading/toony"),
        gi_equalization: f64_at(value, "/vertexMaterial/shading/giEqualization"),
    }
}

fn vec4_at(value: &Value, pointer: &str) -> Option<[f64; 4]> {
    let values = value.pointer(pointer)?.as_array()?;
    Some([
        values.first()?.as_f64()?,
        values.get(1)?.as_f64()?,
        values.get(2)?.as_f64()?,
        values.get(3)?.as_f64()?,
    ])
}

fn display_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn self_test() -> Result<(), Box<dyn Error>> {
    let hotspots = serde_json::json!({
        "hotspots": [{
            "x": 10,
            "y": 20,
            "expected": [100, 100, 100, 255],
            "actual": [20, 20, 20, 255],
            "max_channel_delta": 80,
            "rgb_distance": 138.5641,
            "frontmost_visible": {
                "material_name": "front",
                "triangle": 7,
                "draw_index": 2,
                "pass": "base",
                "material_shading": {"model": "gltf_pbr"},
                "base_texture_rgba": [22, 20, 20, 255],
                "cpu_base_color_rgba": [22, 20, 20, 255],
                "base_uv": [0.25, 0.75],
                "raw_uv": [0.25, 0.75],
                "depth": 0.5,
                "edge_distance_pixels": 0.2,
                "base_texture_local_rgb_gradient": 1.0,
                "policy": {
                    "alpha_mode": "opaque",
                    "cull_mode": "back",
                    "depth_write": true,
                    "blend": false
                }
            },
            "nearest_visible_expected": {
                "material_name": "expected",
                "triangle": 8,
                "cpu_base_color_rgba": [100, 101, 100, 255]
            },
            "nearest_visible_actual": {
                "material_name": "actual",
                "triangle": 9,
                "cpu_base_color_rgba": [20, 20, 21, 255]
            }
        }]
    });
    let manifest = serde_json::json!({
        "corrections": [{
            "x": 10,
            "y": 20,
            "rgba": [22, 20, 20, 255],
            "selection_source": "center",
            "surface": {"materialName": "front", "triangle": 7},
            "sample": [0.5, 0.5],
            "sample_geometry": {
                "node": 1,
                "mesh": 2,
                "primitive": 3,
                "base_uv": [0.25, 0.75],
                "raw_uv": [0.25, 0.75],
                "depth": 0.5,
                "pass": "base"
            }
        }]
    });
    let pixels = parse_pixels(&["10,20".to_owned()])?;
    let report = summarize(
        Path::new("hotspots.json"),
        Path::new("manifest.json"),
        "hotspot actual".to_owned(),
        None,
        &hotspots,
        &manifest,
        &pixels,
    )?;
    assert_eq!(report.rows.len(), 1);
    assert!(report.rows[0].hotspot_found);
    assert!(report.rows[0].manifest_found);
    assert_eq!(report.rows[0].selection_source.as_deref(), Some("center"));
    assert_eq!(
        report.rows[0].selected_surface.as_ref().map(|value| value.material_name.as_str()),
        Some("front")
    );
    assert_eq!(
        report.rows[0].frontmost.as_ref().and_then(|value| value.shading_model.as_deref()),
        Some("gltf_pbr")
    );
    assert!(report.rows[0].selected_actual_rgb_distance.unwrap() < 3.0);
    assert!(markdown(&report).contains("Focused Material Pixel Summary"));
    let actual_override = RgbaJsonArtifact {
        image: RgbaJsonImage {
            width: 16,
            height: 32,
            rgba: vec![0; 16 * 32 * 4],
        },
        material_draws_by_key: [(
            "node1/mesh2/prim3/base".to_owned(),
            RendererMaterialDrawSummary {
                draw_role: Some("owner-sample-resolve".to_owned()),
                material_name: "front".to_owned(),
                material_index: Some(4),
                cull_mode: Some("back".to_owned()),
                alpha_mode: Some("opaque".to_owned()),
                depth_write: Some(true),
                blend: Some(false),
                pbr_fallback: Some(true),
                metallic: Some(0.0),
                roughness: Some(0.657),
                emissive_strength: Some(1.0),
                occlusion_strength: Some(1.0),
                base_texture: Some(12),
                shade_texture: None,
                normal_texture: Some(13),
                base_color: Some([1.0, 1.0, 1.0, 1.0]),
                shade_color: Some([1.0, 1.0, 1.0, 1.0]),
                shading_shift: Some(0.0),
                shading_toony: Some(0.0),
                gi_equalization: Some(1.0),
            },
        )]
        .into_iter()
        .collect(),
    };
    let override_report = summarize(
        Path::new("hotspots.json"),
        Path::new("manifest.json"),
        "override".to_owned(),
        Some(&actual_override),
        &hotspots,
        &manifest,
        &pixels,
    )?;
    assert_eq!(override_report.rows[0].actual, Some([0, 0, 0, 0]));
    assert_eq!(
        override_report.rows[0].selected_draw_key.as_deref(),
        Some("node1/mesh2/prim3/base")
    );
    assert_eq!(
        override_report.rows[0]
            .renderer_material_draw
            .as_ref()
            .and_then(|draw| draw.normal_texture),
        Some(13)
    );
    assert!(markdown(&override_report).contains("front@owner-sample-resolve pbr:true"));

    let draws = material_draws_by_key(&serde_json::json!({
        "renderer": {
            "materialDraws": [{
                "draw": {"key": "node1/mesh2/prim3/base", "role": "source"},
                "material": {"name": "source"},
                "policy": {"depthWrite": true}
            }, {
                "draw": {"key": "node1/mesh2/prim3/base", "role": "owner-sample-resolve"},
                "material": {"name": "resolve"},
                "policy": {"depthWrite": false}
            }]
        }
    }));
    let draw = draws
        .get("node1/mesh2/prim3/base")
        .ok_or("self-test material draw role priority missing key")?;
    assert_eq!(draw.material_name, "resolve");
    assert_eq!(draw.draw_role.as_deref(), Some("owner-sample-resolve"));
    assert_eq!(draw.depth_write, Some(false));
    Ok(())
}
