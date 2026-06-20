#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
vrm-adapter = { path = "../../crates/vrm-adapter" }
---

//! Classify why rendered three-vrm owners are not selected by center-owner mode.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use vrm_adapter::RenderOwnerSurfaceKey;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "audit-center-owner-misses",
    about = "Classify center-owner misses between three-vrm owner hotspots and Rust hotspot candidates"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    owner_hotspots: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    rust_hotspots: Option<PathBuf>,
    #[arg(long)]
    selection_manifest: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long, default_value_t = 16)]
    top: usize,
}

#[derive(Clone, Debug, Serialize)]
struct CenterOwnerMissReport {
    owner_hotspots: String,
    rust_hotspots: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection_manifest: Option<String>,
    owner_hotspot_count: u64,
    rust_hotspot_count: u64,
    joined_count: u64,
    rendered_owner_pixel_count: u64,
    rendered_owner_candidate_count: u64,
    rendered_owner_count: u64,
    selected_center_count: u64,
    manifest_selected_count: u64,
    manifest_missing_selected_count: u64,
    missing_center_count: u64,
    categories: BTreeMap<String, u64>,
    top_misses: Vec<CenterOwnerMiss>,
}

#[derive(Clone, Debug, Serialize)]
struct CenterOwnerMiss {
    x: u64,
    y: u64,
    category: String,
    owner_surface: SurfaceReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    frontmost_visible: Option<CandidateReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    same_surface_center: Option<CandidateReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    same_material_center: Option<CandidateReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    same_surface_nearby: Option<CandidateReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rgb_distance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_channel_delta: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
struct SurfaceReport {
    material_name: String,
    triangle: u64,
}

#[derive(Clone, Debug, Serialize)]
struct CandidateReport {
    material_name: String,
    triangle: u64,
    pass: String,
    sample_offset: [i64; 2],
    visible_by_policy: bool,
    min_barycentric: Option<f64>,
    edge_distance_pixels: Option<f64>,
    depth: Option<f64>,
    draw_index: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CenterSelection {
    Selected,
    Missing,
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

    let owner_path = options
        .owner_hotspots
        .as_deref()
        .ok_or("missing --owner-hotspots")?;
    let rust_path = options
        .rust_hotspots
        .as_deref()
        .ok_or("missing --rust-hotspots")?;
    let owner = serde_json::from_str::<Value>(&fs::read_to_string(owner_path)?)?;
    let rust = serde_json::from_str::<Value>(&fs::read_to_string(rust_path)?)?;
    let manifest = options
        .selection_manifest
        .as_deref()
        .map(|path| read_manifest_pixels(path).map(|pixels| (path, pixels)))
        .transpose()?;
    let report = audit(
        owner_path,
        rust_path,
        manifest.as_ref().map(|(path, pixels)| (*path, pixels)),
        &owner,
        &rust,
        options.top,
    )?;
    let json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = options.out {
        write_file(&path, &json)?;
    } else {
        print!("{json}");
    }
    Ok(())
}

fn audit(
    owner_path: &Path,
    rust_path: &Path,
    manifest: Option<(&Path, &HashSet<(u64, u64)>)>,
    owner: &Value,
    rust: &Value,
    top: usize,
) -> Result<CenterOwnerMissReport, Box<dyn Error>> {
    let owner_hotspots = owner
        .pointer("/reference/renderer/diagnosticHotspots/top")
        .and_then(Value::as_array)
        .ok_or("owner diagnosticHotspots.top must be an array")?;
    let rust_hotspots = rust
        .get("hotspots")
        .and_then(Value::as_array)
        .ok_or("rust hotspots must be an array")?;
    let rust_by_pixel = rust_hotspots
        .iter()
        .filter_map(|hotspot| Some((pixel_key(hotspot)?, hotspot)))
        .collect::<HashMap<_, _>>();

    let mut categories = BTreeMap::<String, u64>::new();
    let mut joined_count = 0;
    let mut rendered_owner_pixel_count = 0;
    let mut rendered_owner_candidate_count = 0;
    let mut rendered_owner_count = 0;
    let mut selected_center_count = 0;
    let mut missing_center_count = 0;
    let mut manifest_selected_count = 0;
    let mut manifest_missing_selected_count = 0;
    let mut misses = Vec::new();

    for owner_hotspot in owner_hotspots {
        let Some(pixel) = pixel_key(owner_hotspot) else {
            continue;
        };
        let Some(rust_hotspot) = rust_by_pixel.get(&pixel).copied() else {
            increment(&mut categories, "missing-rust-hotspot");
            continue;
        };
        joined_count += 1;
        if owner_hotspot.pointer("/renderedOwner/owner").is_some() {
            rendered_owner_pixel_count += 1;
        }
        let owner_candidate = surface_at(owner_hotspot, "/renderedOwnerCandidate");
        if owner_candidate.is_some() {
            rendered_owner_candidate_count += 1;
        }
        let owner_surface = owner_candidate
            .clone()
            .or_else(|| surface_at(owner_hotspot, "/renderedOwner/owner"));
        let Some(owner_surface) = owner_surface else {
            increment(&mut categories, "missing-rendered-owner");
            continue;
        };
        rendered_owner_count += 1;

        let selected = if owner_candidate.is_some() {
            center_candidate_for_surface(rust_hotspot, &owner_surface)
                .filter(|candidate| is_strict_visible_center(candidate))
                .map(|_| CenterSelection::Selected)
                .unwrap_or(CenterSelection::Missing)
        } else {
            CenterSelection::Missing
        };

        if manifest
            .map(|(_, pixels)| pixels.contains(&pixel))
            .unwrap_or(false)
        {
            manifest_selected_count += 1;
        }

        if selected == CenterSelection::Selected {
            selected_center_count += 1;
            continue;
        }

        missing_center_count += 1;
        if manifest
            .map(|(_, pixels)| pixels.contains(&pixel))
            .unwrap_or(false)
        {
            manifest_missing_selected_count += 1;
        }

        let classification = if owner_candidate.is_none() {
            classify_missing_owner_candidate(rust_hotspot, &owner_surface)
        } else {
            classify_miss(rust_hotspot, &owner_surface)
        };
        increment(&mut categories, classification);
        misses.push(CenterOwnerMiss {
            x: pixel.0,
            y: pixel.1,
            category: classification.to_owned(),
            owner_surface: surface_report(&owner_surface),
            frontmost_visible: candidate_at(rust_hotspot, "/frontmost_visible"),
            same_surface_center: center_candidate_for_surface(rust_hotspot, &owner_surface)
                .and_then(candidate_report),
            same_material_center: same_material_center_candidate(rust_hotspot, &owner_surface)
                .and_then(candidate_report),
            same_surface_nearby: same_surface_nearby_candidate(rust_hotspot, &owner_surface)
                .and_then(candidate_report),
            rgb_distance: f64_field(rust_hotspot, "rgb_distance")
                .or_else(|| f64_field(rust_hotspot, "rgbDistance")),
            max_channel_delta: u64_field(rust_hotspot, "max_channel_delta")
                .or_else(|| u64_field(rust_hotspot, "maxChannelDelta")),
        });
    }

    misses.sort_by(|left, right| {
        right
            .max_channel_delta
            .cmp(&left.max_channel_delta)
            .then_with(|| {
                right
                    .rgb_distance
                    .partial_cmp(&left.rgb_distance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.y.cmp(&right.y))
            .then_with(|| left.x.cmp(&right.x))
    });
    misses.truncate(top);

    Ok(CenterOwnerMissReport {
        owner_hotspots: display_path(owner_path),
        rust_hotspots: display_path(rust_path),
        selection_manifest: manifest.map(|(path, _)| display_path(path)),
        owner_hotspot_count: owner_hotspots.len() as u64,
        rust_hotspot_count: rust_hotspots.len() as u64,
        joined_count,
        rendered_owner_pixel_count,
        rendered_owner_candidate_count,
        rendered_owner_count,
        selected_center_count,
        manifest_selected_count,
        manifest_missing_selected_count,
        missing_center_count,
        categories,
        top_misses: misses,
    })
}

fn classify_missing_owner_candidate(
    rust_hotspot: &Value,
    owner_surface: &RenderOwnerSurfaceKey,
) -> &'static str {
    if center_candidate_for_surface(rust_hotspot, owner_surface).is_some() {
        return "missing-rendered-owner-candidate-with-rust-center";
    }
    if same_material_center_candidate(rust_hotspot, owner_surface).is_some() {
        return "missing-rendered-owner-candidate-same-material-center";
    }
    if same_surface_nearby_candidate(rust_hotspot, owner_surface).is_some() {
        return "missing-rendered-owner-candidate-same-surface-nearby";
    }
    "missing-rendered-owner-candidate"
}

fn classify_miss(rust_hotspot: &Value, owner_surface: &RenderOwnerSurfaceKey) -> &'static str {
    if center_candidate_for_surface(rust_hotspot, owner_surface).is_some() {
        return "same-surface-center-outside-or-rejected";
    }
    if same_material_center_candidate(rust_hotspot, owner_surface).is_some() {
        return "same-material-different-triangle-center";
    }
    if same_surface_nearby_candidate(rust_hotspot, owner_surface).is_some() {
        return "same-surface-nearby-sample";
    }
    if candidate_at(rust_hotspot, "/frontmost_visible").is_some() {
        return "different-surface-center-frontmost";
    }
    "no-visible-center-candidate"
}

fn center_candidate_for_surface<'a>(
    rust_hotspot: &'a Value,
    surface: &RenderOwnerSurfaceKey,
) -> Option<&'a Value> {
    rust_hotspot
        .get("candidates")
        .and_then(Value::as_array)?
        .iter()
        .filter(|candidate| {
            surface_at(candidate, "")
                .as_ref()
                .is_some_and(|candidate_surface| candidate_surface == surface)
                && sample_offset(candidate).is_some_and(|offset| offset == [0, 0])
        })
        .min_by(candidate_depth_draw_order)
}

fn same_material_center_candidate<'a>(
    rust_hotspot: &'a Value,
    surface: &RenderOwnerSurfaceKey,
) -> Option<&'a Value> {
    rust_hotspot
        .get("candidates")
        .and_then(Value::as_array)?
        .iter()
        .filter(|candidate| {
            surface_at(candidate, "").as_ref().is_some_and(|candidate_surface| {
                candidate_surface.material_name() == surface.material_name()
                    && candidate_surface.triangle() != surface.triangle()
            }) && sample_offset(candidate).is_some_and(|offset| offset == [0, 0])
                && bool_field(candidate, "visible_by_policy").unwrap_or(false)
        })
        .min_by(candidate_depth_draw_order)
}

fn same_surface_nearby_candidate<'a>(
    rust_hotspot: &'a Value,
    surface: &RenderOwnerSurfaceKey,
) -> Option<&'a Value> {
    rust_hotspot
        .get("candidates")
        .and_then(Value::as_array)?
        .iter()
        .filter(|candidate| {
            surface_at(candidate, "")
                .as_ref()
                .is_some_and(|candidate_surface| candidate_surface == surface)
                && sample_offset(candidate).is_some_and(|offset| offset != [0, 0])
                && bool_field(candidate, "visible_by_policy").unwrap_or(false)
        })
        .min_by(|left, right| {
            f64_field(left, "sample_distance")
                .partial_cmp(&f64_field(right, "sample_distance"))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| candidate_depth_draw_order(left, right))
        })
}

fn candidate_depth_draw_order(left: &&Value, right: &&Value) -> std::cmp::Ordering {
    f64_field(left, "depth")
        .partial_cmp(&f64_field(right, "depth"))
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| u64_field(right, "draw_index").cmp(&u64_field(left, "draw_index")))
}

fn is_strict_visible_center(candidate: &Value) -> bool {
    bool_field(candidate, "visible_by_policy").unwrap_or(false)
        && f64_field(candidate, "min_barycentric")
            .is_some_and(|min_barycentric| min_barycentric >= 0.0)
}

fn candidate_at(value: &Value, pointer: &str) -> Option<CandidateReport> {
    candidate_report(value.pointer(pointer)?)
}

fn candidate_report(candidate: &Value) -> Option<CandidateReport> {
    Some(CandidateReport {
        material_name: material_name(candidate)?.to_owned(),
        triangle: candidate.get("triangle")?.as_u64()?,
        pass: candidate.get("pass")?.as_str()?.to_owned(),
        sample_offset: sample_offset(candidate)?,
        visible_by_policy: bool_field(candidate, "visible_by_policy").unwrap_or(false),
        min_barycentric: f64_field(candidate, "min_barycentric"),
        edge_distance_pixels: f64_field(candidate, "edge_distance_pixels"),
        depth: f64_field(candidate, "depth"),
        draw_index: u64_field(candidate, "draw_index"),
    })
}

fn surface_report(surface: &RenderOwnerSurfaceKey) -> SurfaceReport {
    SurfaceReport {
        material_name: surface.material_name().to_owned(),
        triangle: surface.triangle(),
    }
}

fn read_manifest_pixels(path: &Path) -> Result<HashSet<(u64, u64)>, Box<dyn Error>> {
    let manifest = serde_json::from_str::<Value>(&fs::read_to_string(path)?)?;
    let corrections = manifest
        .get("corrections")
        .and_then(Value::as_array)
        .ok_or("manifest corrections must be an array")?;
    Ok(corrections
        .iter()
        .filter_map(pixel_key)
        .collect::<HashSet<_>>())
}

fn surface_at(value: &Value, pointer: &str) -> Option<RenderOwnerSurfaceKey> {
    let value = value.pointer(pointer)?;
    RenderOwnerSurfaceKey::from_diagnostic_material_name(material_name(value)?, triangle(value)?)
        .into()
}

fn material_name(value: &Value) -> Option<&str> {
    value
        .get("materialName")
        .or_else(|| value.get("material_name"))
        .and_then(Value::as_str)
}

fn triangle(value: &Value) -> Option<u64> {
    value.get("triangle").and_then(Value::as_u64)
}

fn pixel_key(value: &Value) -> Option<(u64, u64)> {
    Some((value.get("x")?.as_u64()?, value.get("y")?.as_u64()?))
}

fn sample_offset(value: &Value) -> Option<[i64; 2]> {
    let values = value.get("sample_offset")?.as_array()?;
    Some([values.first()?.as_i64()?, values.get(1)?.as_i64()?])
}

fn bool_field(value: &Value, field: &str) -> Option<bool> {
    value.get(field)?.as_bool()
}

fn f64_field(value: &Value, field: &str) -> Option<f64> {
    value.get(field).and_then(Value::as_f64)
}

fn u64_field(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

fn increment(categories: &mut BTreeMap<String, u64>, category: &str) {
    *categories.entry(category.to_owned()).or_default() += 1;
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

fn self_test() -> Result<(), Box<dyn Error>> {
    let owner = serde_json::json!({
        "reference": {"renderer": {"diagnosticHotspots": {"top": [
            {"x": 0, "y": 0, "renderedOwnerCandidate": {"materialName": "body:vrm-rs-owner-id-diagnostic", "triangle": 7}},
            {"x": 1, "y": 0, "renderedOwnerCandidate": {"materialName": "body:vrm-rs-owner-id-diagnostic", "triangle": 8}}
        ]}}}
    });
    let rust = serde_json::json!({
        "hotspots": [
            {
                "x": 0,
                "y": 0,
                "rgb_distance": 10.0,
                "max_channel_delta": 5,
                "frontmost_visible": {
                    "material_name": "body",
                    "triangle": 7,
                    "pass": "base",
                    "sample_offset": [0, 0],
                    "visible_by_policy": true,
                    "min_barycentric": 0.2,
                    "edge_distance_pixels": 0.1,
                    "depth": 0.5,
                    "draw_index": 1
                },
                "candidates": [{
                    "material_name": "body",
                    "triangle": 7,
                    "pass": "base",
                    "sample_offset": [0, 0],
                    "visible_by_policy": true,
                    "min_barycentric": 0.2,
                    "edge_distance_pixels": 0.1,
                    "depth": 0.5,
                    "draw_index": 1
                }]
            },
            {
                "x": 1,
                "y": 0,
                "rgb_distance": 20.0,
                "max_channel_delta": 9,
                "frontmost_visible": {
                    "material_name": "body",
                    "triangle": 9,
                    "pass": "base",
                    "sample_offset": [0, 0],
                    "visible_by_policy": true,
                    "min_barycentric": 0.3,
                    "edge_distance_pixels": 0.2,
                    "depth": 0.4,
                    "draw_index": 2
                },
                "candidates": [{
                    "material_name": "body",
                    "triangle": 9,
                    "pass": "base",
                    "sample_offset": [0, 0],
                    "visible_by_policy": true,
                    "min_barycentric": 0.3,
                    "edge_distance_pixels": 0.2,
                    "depth": 0.4,
                    "draw_index": 2
                }]
            }
        ]
    });
    let manifest_pixels = HashSet::from([(0, 0)]);
    let report = audit(
        Path::new("owner.json"),
        Path::new("rust.json"),
        Some((Path::new("manifest.json"), &manifest_pixels)),
        &owner,
        &rust,
        8,
    )?;
    assert_eq!(report.joined_count, 2);
    assert_eq!(report.selected_center_count, 1);
    assert_eq!(report.missing_center_count, 1);
    assert_eq!(report.manifest_selected_count, 1);
    assert_eq!(
        report
            .categories
            .get("same-material-different-triangle-center"),
        Some(&1)
    );
    assert_eq!(report.top_misses.len(), 1);
    assert_eq!(
        report.top_misses[0].category,
        "same-material-different-triangle-center"
    );
    Ok(())
}
