#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Join three-vrm owner-id and base-color hotspot sidecars for color/root-cause audits.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "join-base-color-owner-hotspots",
    about = "Join three-vrm owner-id and base-color hotspot projections"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    owner_hotspots: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    base_color_hotspots: Option<PathBuf>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
    #[arg(long, default_value_t = 16)]
    top: usize,
}

#[derive(Clone, Debug, Serialize)]
struct JoinReport {
    owner_hotspots: String,
    base_color_hotspots: String,
    owner_hotspot_count: u64,
    base_color_hotspot_count: u64,
    joined_count: u64,
    missing_base_color_count: u64,
    rendered_owner_count: u64,
    owner_matches_base_frontmost_material: u64,
    owner_matches_base_frontmost_surface: u64,
    mean_owner_surface_base_color_rendered_rgb_distance: Option<f64>,
    mean_owner_surface_texture_as_linear_rendered_rgb_distance: Option<f64>,
    mean_owner_surface_browser_base_color_rendered_rgb_distance: Option<f64>,
    owner_to_base_frontmost_materials: BTreeMap<String, u64>,
    owner_to_nearest_rendered_base_color_materials: BTreeMap<String, u64>,
    owner_to_nearest_rendered_texture_as_linear_materials: BTreeMap<String, u64>,
    frontmost_to_nearest_rendered_base_color_materials: BTreeMap<String, u64>,
    frontmost_to_nearest_rendered_texture_as_linear_materials: BTreeMap<String, u64>,
    frontmost_to_nearest_rendered_base_color_draw_order: BTreeMap<String, u64>,
    owner_material_buckets: Vec<MaterialBucket>,
    top_owner_surface_color_deltas: Vec<JoinedHotspot>,
}

#[derive(Clone, Debug, Default)]
struct MaterialAccumulator {
    count: u64,
    base_color_distance_sum: f64,
    base_color_distance_count: u64,
    texture_as_linear_distance_sum: f64,
    texture_as_linear_distance_count: u64,
    frontmost_material_matches: u64,
    frontmost_surface_matches: u64,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialBucket {
    material_name: String,
    count: u64,
    mean_base_color_rendered_rgb_distance: Option<f64>,
    mean_texture_as_linear_rendered_rgb_distance: Option<f64>,
    mean_browser_base_color_rendered_rgb_distance: Option<f64>,
    frontmost_material_matches: u64,
    frontmost_surface_matches: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Surface {
    material_name: String,
    triangle: u64,
}

#[derive(Clone, Debug, Serialize)]
struct JoinedHotspot {
    x: u64,
    y: u64,
    rendered_pixel_rgba: Option<[u64; 4]>,
    owner_surface: Option<SurfaceSummary>,
    base_frontmost: Option<SurfaceSummary>,
    nearest_rendered_base_color: Option<SurfaceSummary>,
    nearest_rendered_texture_as_linear: Option<SurfaceSummary>,
    base_frontmost_draw_index: Option<u64>,
    nearest_rendered_base_color_draw_index: Option<u64>,
    frontmost_to_nearest_rendered_base_color_draw_delta: Option<i64>,
    base_frontmost_projected_color: Option<[u64; 4]>,
    base_frontmost_texture_as_linear_color: Option<[u64; 4]>,
    base_frontmost_browser_base_color: Option<[u64; 4]>,
    base_color_rendered_rgb_distance: Option<f64>,
    texture_as_linear_rendered_rgb_distance: Option<f64>,
    browser_base_color_rendered_rgb_distance: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
struct SurfaceSummary {
    material_name: String,
    triangle: u64,
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
    let owner_path = options.owner_hotspots.as_ref().ok_or("missing --owner-hotspots")?;
    let base_path = options
        .base_color_hotspots
        .as_ref()
        .ok_or("missing --base-color-hotspots")?;
    let owner = serde_json::from_str::<Value>(&fs::read_to_string(owner_path)?)?;
    let base = serde_json::from_str::<Value>(&fs::read_to_string(base_path)?)?;
    let report = join_reports(owner_path, base_path, &owner, &base, options.top)?;
    let json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = &options.json_out {
        write_file(path, &json)?;
    } else {
        print!("{json}");
    }
    if let Some(path) = &options.markdown_out {
        write_file(path, &markdown_report(&report))?;
    }
    Ok(())
}

fn join_reports(
    owner_path: &Path,
    base_path: &Path,
    owner: &Value,
    base: &Value,
    top: usize,
) -> Result<JoinReport, Box<dyn std::error::Error>> {
    let owner_hotspots = hotspot_array(owner, "owner")?;
    let base_hotspots = hotspot_array(base, "base-color")?;
    let base_by_pixel = base_hotspots
        .iter()
        .filter_map(|hotspot| Some((pixel_key(hotspot)?, *hotspot)))
        .collect::<HashMap<_, _>>();

    let mut report = JoinReport {
        owner_hotspots: display_path(owner_path),
        base_color_hotspots: display_path(base_path),
        owner_hotspot_count: owner_hotspots.len() as u64,
        base_color_hotspot_count: base_hotspots.len() as u64,
        joined_count: 0,
        missing_base_color_count: 0,
        rendered_owner_count: 0,
        owner_matches_base_frontmost_material: 0,
        owner_matches_base_frontmost_surface: 0,
        mean_owner_surface_base_color_rendered_rgb_distance: None,
        mean_owner_surface_texture_as_linear_rendered_rgb_distance: None,
        mean_owner_surface_browser_base_color_rendered_rgb_distance: None,
        owner_to_base_frontmost_materials: BTreeMap::new(),
        owner_to_nearest_rendered_base_color_materials: BTreeMap::new(),
        owner_to_nearest_rendered_texture_as_linear_materials: BTreeMap::new(),
        frontmost_to_nearest_rendered_base_color_materials: BTreeMap::new(),
        frontmost_to_nearest_rendered_texture_as_linear_materials: BTreeMap::new(),
        frontmost_to_nearest_rendered_base_color_draw_order: BTreeMap::new(),
        owner_material_buckets: Vec::new(),
        top_owner_surface_color_deltas: Vec::new(),
    };
    let mut base_distance_sum = 0.0;
    let mut base_distance_count = 0;
    let mut linear_distance_sum = 0.0;
    let mut linear_distance_count = 0;
    let mut material_buckets = BTreeMap::<String, MaterialAccumulator>::new();
    let mut joined_lines = Vec::new();

    for owner_hotspot in owner_hotspots {
        let Some(pixel) = pixel_key(owner_hotspot) else {
            continue;
        };
        let Some(base_hotspot) = base_by_pixel.get(&pixel) else {
            report.missing_base_color_count += 1;
            continue;
        };
        report.joined_count += 1;
        let owner_surface = surface_at(owner_hotspot, "/renderedOwner/owner");
        if owner_surface.is_some() {
            report.rendered_owner_count += 1;
        }
        let frontmost = surface_at(base_hotspot, "/frontmost");
        let nearest_base = surface_at(base_hotspot, "/nearestRenderedBaseColor");
        let nearest_linear = surface_at_any(
            base_hotspot,
            &[
                "/nearestRenderedBrowserBaseColor",
                "/nearestRenderedTextureAsLinearBaseColor",
            ],
        );
        let frontmost_draw_index = u64_at(base_hotspot, "/frontmost/drawIndex");
        let nearest_base_draw_index = u64_at(base_hotspot, "/nearestRenderedBaseColor/drawIndex");

        bump_pair(
            &mut report.owner_to_base_frontmost_materials,
            owner_surface.as_ref(),
            frontmost.as_ref(),
        );
        bump_pair(
            &mut report.owner_to_nearest_rendered_base_color_materials,
            owner_surface.as_ref(),
            nearest_base.as_ref(),
        );
        bump_pair(
            &mut report.owner_to_nearest_rendered_texture_as_linear_materials,
            owner_surface.as_ref(),
            nearest_linear.as_ref(),
        );
        bump_pair(
            &mut report.frontmost_to_nearest_rendered_base_color_materials,
            frontmost.as_ref(),
            nearest_base.as_ref(),
        );
        bump_pair(
            &mut report.frontmost_to_nearest_rendered_texture_as_linear_materials,
            frontmost.as_ref(),
            nearest_linear.as_ref(),
        );
        bump_draw_relation(
            &mut report.frontmost_to_nearest_rendered_base_color_draw_order,
            frontmost_draw_index,
            nearest_base_draw_index,
        );

        let material_match = owner_surface
            .as_ref()
            .zip(frontmost.as_ref())
            .is_some_and(|(owner, frontmost)| owner.material_name == frontmost.material_name);
        let surface_match = owner_surface
            .as_ref()
            .zip(frontmost.as_ref())
            .is_some_and(|(owner, frontmost)| owner == frontmost);
        if material_match {
            report.owner_matches_base_frontmost_material += 1;
        }
        if surface_match {
            report.owner_matches_base_frontmost_surface += 1;
        }

        let base_distance = f64_at(
            base_hotspot,
            "/frontmost/projectedBaseColorRenderedPixelRgbDistance",
        );
        let linear_distance = f64_at_any(
            base_hotspot,
            &[
                "/frontmost/projectedBrowserBaseColorRenderedPixelRgbDistance",
                "/frontmost/projectedBaseColorTextureAsLinearRenderedPixelRgbDistance",
            ],
        );
        if surface_match {
            if let Some(distance) = base_distance {
                base_distance_sum += distance;
                base_distance_count += 1;
            }
            if let Some(distance) = linear_distance {
                linear_distance_sum += distance;
                linear_distance_count += 1;
            }
        }
        if let Some(owner) = &owner_surface {
            let bucket = material_buckets.entry(owner.material_name.clone()).or_default();
            bucket.count += 1;
            if material_match {
                bucket.frontmost_material_matches += 1;
            }
            if surface_match {
                bucket.frontmost_surface_matches += 1;
                if let Some(distance) = base_distance {
                    bucket.base_color_distance_sum += distance;
                    bucket.base_color_distance_count += 1;
                }
                if let Some(distance) = linear_distance {
                    bucket.texture_as_linear_distance_sum += distance;
                    bucket.texture_as_linear_distance_count += 1;
                }
            }
        }

        joined_lines.push(JoinedHotspot {
            x: pixel.0,
            y: pixel.1,
            rendered_pixel_rgba: rgba_at(base_hotspot, "/renderedPixelRgba"),
            owner_surface: owner_surface.as_ref().map(SurfaceSummary::from),
            base_frontmost: frontmost.as_ref().map(SurfaceSummary::from),
            nearest_rendered_base_color: nearest_base.as_ref().map(SurfaceSummary::from),
            nearest_rendered_texture_as_linear: nearest_linear.as_ref().map(SurfaceSummary::from),
            base_frontmost_draw_index: frontmost_draw_index,
            nearest_rendered_base_color_draw_index: nearest_base_draw_index,
            frontmost_to_nearest_rendered_base_color_draw_delta: draw_delta(
                frontmost_draw_index,
                nearest_base_draw_index,
            ),
            base_frontmost_projected_color: rgba_at(base_hotspot, "/frontmost/projectedBaseColorSrgb"),
            base_frontmost_texture_as_linear_color: rgba_at(
                base_hotspot,
                "/frontmost/projectedBaseColorTextureAsLinearSrgb",
            ),
            base_frontmost_browser_base_color: rgba_at_any(
                base_hotspot,
                &[
                    "/frontmost/projectedBrowserBaseColorSrgb",
                    "/frontmost/projectedBaseColorTextureAsLinearSrgb",
                ],
            ),
            base_color_rendered_rgb_distance: base_distance,
            texture_as_linear_rendered_rgb_distance: linear_distance,
            browser_base_color_rendered_rgb_distance: linear_distance,
        });
    }

    report.mean_owner_surface_base_color_rendered_rgb_distance =
        mean(base_distance_sum, base_distance_count);
    report.mean_owner_surface_texture_as_linear_rendered_rgb_distance =
        mean(linear_distance_sum, linear_distance_count);
    report.mean_owner_surface_browser_base_color_rendered_rgb_distance =
        report.mean_owner_surface_texture_as_linear_rendered_rgb_distance;
    report.owner_material_buckets = material_buckets
        .into_iter()
        .map(|(material_name, bucket)| MaterialBucket {
            material_name,
            count: bucket.count,
            mean_base_color_rendered_rgb_distance: mean(
                bucket.base_color_distance_sum,
                bucket.base_color_distance_count,
            ),
            mean_texture_as_linear_rendered_rgb_distance: mean(
                bucket.texture_as_linear_distance_sum,
                bucket.texture_as_linear_distance_count,
            ),
            mean_browser_base_color_rendered_rgb_distance: mean(
                bucket.texture_as_linear_distance_sum,
                bucket.texture_as_linear_distance_count,
            ),
            frontmost_material_matches: bucket.frontmost_material_matches,
            frontmost_surface_matches: bucket.frontmost_surface_matches,
        })
        .collect();
    report.owner_material_buckets.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.material_name.cmp(&right.material_name))
    });
    joined_lines.sort_by(|left, right| {
        option_f64_desc(left.base_color_rendered_rgb_distance, right.base_color_rendered_rgb_distance)
            .then_with(|| left.x.cmp(&right.x))
            .then_with(|| left.y.cmp(&right.y))
    });
    report.top_owner_surface_color_deltas = joined_lines.into_iter().take(top).collect();
    Ok(report)
}

fn hotspot_array<'a>(
    value: &'a Value,
    name: &str,
) -> Result<Vec<&'a Value>, Box<dyn std::error::Error>> {
    Ok(value
        .pointer("/reference/renderer/diagnosticHotspots/top")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{name} diagnosticHotspots.top must be an array"))?
        .iter()
        .collect())
}

fn surface_at(value: &Value, pointer: &str) -> Option<Surface> {
    let value = value.pointer(pointer)?;
    Some(Surface {
        material_name: normalize_material(value.get("materialName")?.as_str()?),
        triangle: value.get("triangle")?.as_u64()?,
    })
}

impl From<&Surface> for SurfaceSummary {
    fn from(value: &Surface) -> Self {
        Self {
            material_name: value.material_name.clone(),
            triangle: value.triangle,
        }
    }
}

fn normalize_material(name: &str) -> String {
    name.strip_suffix(":vrm-rs-owner-id-diagnostic")
        .or_else(|| name.strip_suffix(":vrm-rs-flat-diagnostic"))
        .unwrap_or(name)
        .to_owned()
}

fn pixel_key(value: &Value) -> Option<(u64, u64)> {
    Some((value.get("x")?.as_u64()?, value.get("y")?.as_u64()?))
}

fn surface_at_any(value: &Value, pointers: &[&str]) -> Option<Surface> {
    pointers
        .iter()
        .find_map(|pointer| surface_at(value, pointer))
}

fn rgba_at(value: &Value, pointer: &str) -> Option<[u64; 4]> {
    let values = value.pointer(pointer)?.as_array()?;
    Some([
        values.first()?.as_u64()?,
        values.get(1)?.as_u64()?,
        values.get(2)?.as_u64()?,
        values.get(3)?.as_u64()?,
    ])
}

fn rgba_at_any(value: &Value, pointers: &[&str]) -> Option<[u64; 4]> {
    pointers.iter().find_map(|pointer| rgba_at(value, pointer))
}

fn f64_at(value: &Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer)?.as_f64()
}

fn f64_at_any(value: &Value, pointers: &[&str]) -> Option<f64> {
    pointers.iter().find_map(|pointer| f64_at(value, pointer))
}

fn u64_at(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer)?.as_u64()
}

fn bump_pair(map: &mut BTreeMap<String, u64>, left: Option<&Surface>, right: Option<&Surface>) {
    let key = format!(
        "{} -> {}",
        left.map(|surface| surface.material_name.as_str())
            .unwrap_or("none"),
        right
            .map(|surface| surface.material_name.as_str())
            .unwrap_or("none")
    );
    *map.entry(key).or_default() += 1;
}

fn bump_draw_relation(map: &mut BTreeMap<String, u64>, left: Option<u64>, right: Option<u64>) {
    *map.entry(draw_relation(left, right)).or_default() += 1;
}

fn draw_relation(left: Option<u64>, right: Option<u64>) -> String {
    match draw_delta(left, right) {
        Some(0) => "same".to_owned(),
        Some(delta) if delta > 0 => "nearest-after".to_owned(),
        Some(_) => "nearest-before".to_owned(),
        None => "missing".to_owned(),
    }
}

fn draw_delta(left: Option<u64>, right: Option<u64>) -> Option<i64> {
    Some(i64::try_from(right?).ok()? - i64::try_from(left?).ok()?)
}

fn mean(sum: f64, count: u64) -> Option<f64> {
    (count > 0).then_some(sum / count as f64)
}

fn option_f64_desc(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    right
        .unwrap_or(f64::NEG_INFINITY)
        .partial_cmp(&left.unwrap_or(f64::NEG_INFINITY))
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn markdown_report(report: &JoinReport) -> String {
    let mut output = String::new();
    output.push_str("# Base-Color Owner Hotspot Join\n\n");
    output.push_str(&format!("- Owner hotspots: `{}`\n", report.owner_hotspots));
    output.push_str(&format!(
        "- Base-color hotspots: `{}`\n",
        report.base_color_hotspots
    ));
    output.push_str(&format!(
        "- Joined/missing/rendered-owner: `{}` / `{}` / `{}`\n",
        report.joined_count, report.missing_base_color_count, report.rendered_owner_count
    ));
    output.push_str(&format!(
        "- Owner matches base frontmost material/surface: `{}` / `{}`\n",
        report.owner_matches_base_frontmost_material, report.owner_matches_base_frontmost_surface
    ));
    output.push_str(&format!(
        "- Mean owner-surface base/browser-compatible distance: `{}` / `{}`\n",
        fmt_opt(report.mean_owner_surface_base_color_rendered_rgb_distance),
        fmt_opt(report.mean_owner_surface_browser_base_color_rendered_rgb_distance)
    ));
    write_top_counts(
        &mut output,
        "Owner To Base Frontmost Materials",
        &report.owner_to_base_frontmost_materials,
    );
    write_top_counts(
        &mut output,
        "Owner To Nearest Rendered Base-Color Materials",
        &report.owner_to_nearest_rendered_base_color_materials,
    );
    write_top_counts(
        &mut output,
        "Frontmost To Nearest Rendered Base-Color Materials",
        &report.frontmost_to_nearest_rendered_base_color_materials,
    );
    write_top_counts(
        &mut output,
        "Frontmost To Nearest Rendered Base-Color Draw Order",
        &report.frontmost_to_nearest_rendered_base_color_draw_order,
    );
    output.push_str("## Owner Material Buckets\n\n");
    output.push_str("| Material | Count | Front material/surface matches | Mean base / browser-compatible distance |\n");
    output.push_str("| --- | ---: | ---: | ---: |\n");
    for bucket in report.owner_material_buckets.iter().take(12) {
        output.push_str(&format!(
            "| {} | {} | {} / {} | {} / {} |\n",
            bucket.material_name,
            bucket.count,
            bucket.frontmost_material_matches,
            bucket.frontmost_surface_matches,
            fmt_opt(bucket.mean_base_color_rendered_rgb_distance),
            fmt_opt(bucket.mean_browser_base_color_rendered_rgb_distance)
        ));
    }
    output.push_str("\n## Top Owner-Surface Color Deltas\n\n");
    output.push_str("| Pixel | Owner | Frontmost | Nearest rendered | Draw delta frontmost | Base / browser-compatible distance | Rendered | Projected / browser-compatible |\n");
    output.push_str("| --- | --- | --- | --- | ---: | ---: | --- | --- |\n");
    for line in &report.top_owner_surface_color_deltas {
        output.push_str(&format!(
            "| {},{} | {} | {} | {} | {} | {} / {} | {} | {} / {} |\n",
            line.x,
            line.y,
            fmt_surface(line.owner_surface.as_ref()),
            fmt_surface(line.base_frontmost.as_ref()),
            fmt_surface(line.nearest_rendered_base_color.as_ref()),
            fmt_i64(line.frontmost_to_nearest_rendered_base_color_draw_delta),
            fmt_opt(line.base_color_rendered_rgb_distance),
            fmt_opt(line.browser_base_color_rendered_rgb_distance),
            fmt_rgba(line.rendered_pixel_rgba),
            fmt_rgba(line.base_frontmost_projected_color),
            fmt_rgba(line.base_frontmost_browser_base_color)
        ));
    }
    output
}

fn write_top_counts(output: &mut String, title: &str, counts: &BTreeMap<String, u64>) {
    output.push_str(&format!("## {title}\n\n"));
    let mut ordered = counts.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
    for (key, count) in ordered.into_iter().take(12) {
        output.push_str(&format!("- `{key}`: `{count}`\n"));
    }
    output.push('\n');
}

fn fmt_opt(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_surface(value: Option<&SurfaceSummary>) -> String {
    value
        .map(|surface| format!("{}:tri{}", surface.material_name, surface.triangle))
        .unwrap_or_else(|| "none".to_owned())
}

fn fmt_rgba(value: Option<[u64; 4]>) -> String {
    value
        .map(|rgba| format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3]))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn fmt_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned())
}

fn write_file(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn self_test() -> Result<(), Box<dyn std::error::Error>> {
    let owner = serde_json::json!({
        "reference": {"renderer": {"diagnosticHotspots": {"top": [{
            "x": 1,
            "y": 2,
            "renderedOwner": {"owner": {"materialName": "body:vrm-rs-owner-id-diagnostic", "triangle": 7}}
        }]}}}
    });
    let base = serde_json::json!({
        "reference": {"renderer": {"diagnosticHotspots": {"top": [{
            "x": 1,
            "y": 2,
            "renderedPixelRgba": [20, 21, 22, 255],
            "frontmost": {
                "materialName": "body:vrm-rs-flat-diagnostic",
                "triangle": 7,
                "drawIndex": 11,
                "projectedBaseColorSrgb": [10, 11, 12, 255],
                "projectedBaseColorTextureAsLinearSrgb": [18, 19, 20, 255],
                "projectedBrowserBaseColorSrgb": [19, 20, 21, 255],
                "projectedBaseColorRenderedPixelRgbDistance": 17.3205,
                "projectedBaseColorTextureAsLinearRenderedPixelRgbDistance": 3.4641,
                "projectedBrowserBaseColorRenderedPixelRgbDistance": 1.7321
            },
            "nearestRenderedBaseColor": {"materialName": "body:vrm-rs-flat-diagnostic", "triangle": 7, "drawIndex": 12},
            "nearestRenderedTextureAsLinearBaseColor": {"materialName": "legacy:vrm-rs-flat-diagnostic", "triangle": 8},
            "nearestRenderedBrowserBaseColor": {"materialName": "body:vrm-rs-flat-diagnostic", "triangle": 7}
        }]}}}
    });
    let report = join_reports(Path::new("owner.json"), Path::new("base.json"), &owner, &base, 8)?;
    assert_eq!(report.joined_count, 1);
    assert_eq!(report.owner_matches_base_frontmost_surface, 1);
    assert_eq!(
        report.owner_to_base_frontmost_materials.get("body -> body"),
        Some(&1)
    );
    assert_eq!(
        report
            .frontmost_to_nearest_rendered_base_color_draw_order
            .get("nearest-after"),
        Some(&1)
    );
    assert_eq!(report.owner_material_buckets[0].material_name, "body");
    assert_eq!(
        report.mean_owner_surface_browser_base_color_rendered_rgb_distance,
        Some(1.7321)
    );
    assert_eq!(
        report.top_owner_surface_color_deltas[0].base_frontmost_browser_base_color,
        Some([19, 20, 21, 255])
    );
    assert_eq!(
        report
            .owner_to_nearest_rendered_texture_as_linear_materials
            .get("body -> body"),
        Some(&1)
    );
    Ok(())
}
