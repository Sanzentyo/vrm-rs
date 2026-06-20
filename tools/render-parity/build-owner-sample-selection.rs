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

//! Build owner/sample resolve selections from rendered three-vrm owner IDs.
//!
//! Unlike apply-owner-sample-correction.rs, this tool never reads expected or
//! actual color images and does not choose samples by RGB distance. It emits the
//! same manifest shape because the renderers already consume that schema, but
//! the selected geometry is driven only by the browser owner-id pass.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use vrm_adapter::{RenderOwnerSampleKey, RenderOwnerSurfaceKey, RenderSamplePoint};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "build-owner-sample-selection",
    about = "Build a renderer owner/sample selection manifest from three-vrm rendered owner IDs"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    owner_hotspots: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    rust_hotspots: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    manifest_out: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionReport {
    owner_hotspots: String,
    rust_hotspots: String,
    owner_hotspot_count: u64,
    joined_count: u64,
    rendered_owner_count: u64,
    rendered_owner_coverage_recovered_count: u64,
    rendered_owner_subpixel_recovered_count: u64,
    rendered_owner_recovered_count: u64,
    rendered_owner_center_shading_geometry_count: u64,
    selection_count: u64,
    missing_rust_count: u64,
    missing_rendered_owner_count: u64,
    missing_recovery_count: u64,
    missing_rust_sample_count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionManifest {
    generator: &'static str,
    owner_hotspots: String,
    rust_hotspots: String,
    corrections: Vec<SelectionManifestEntry>,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionManifestEntry {
    x: u64,
    y: u64,
    rgba: [u8; 4],
    surface: SelectionManifestSurface,
    sample: [f64; 2],
    sample_geometry: SelectionManifestSampleGeometry,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionManifestSurface {
    #[serde(rename = "materialName")]
    material_name: String,
    triangle: u64,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionManifestSampleGeometry {
    node: u64,
    mesh: u64,
    primitive: u64,
    triangle: u64,
    indices: [u64; 3],
    barycentric: [f64; 3],
    raw_uv: [f64; 2],
    base_uv: [f64; 2],
    depth: f64,
    pass: String,
}

#[derive(Clone, Debug)]
struct ResolvedSample {
    rgba: [u8; 4],
    geometry: SelectionManifestSampleGeometry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoverySource {
    Coverage,
    Subpixel,
}

#[derive(Clone, Debug)]
struct OwnerRecoverySample {
    source: RecoverySource,
    surface: RenderOwnerSurfaceKey,
    sample: [f64; 2],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeometrySource {
    PixelCenter,
    RecoveryPoint,
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
        .as_ref()
        .ok_or("missing --owner-hotspots")?;
    let rust_path = options
        .rust_hotspots
        .as_ref()
        .ok_or("missing --rust-hotspots")?;
    let owner = serde_json::from_str::<Value>(&fs::read_to_string(owner_path)?)?;
    let rust = serde_json::from_str::<Value>(&fs::read_to_string(rust_path)?)?;
    let (report, manifest) = build_selection(owner_path, rust_path, &owner, &rust)?;
    let report_json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = &options.out {
        write_file(path, &report_json)?;
    } else {
        print!("{report_json}");
    }
    if let Some(path) = &options.manifest_out {
        write_file(path, &format!("{}\n", serde_json::to_string_pretty(&manifest)?))?;
    }
    Ok(())
}

fn build_selection(
    owner_path: &Path,
    rust_path: &Path,
    owner: &Value,
    rust: &Value,
) -> Result<(SelectionReport, SelectionManifest), Box<dyn Error>> {
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

    let mut report = SelectionReport {
        owner_hotspots: display_path(owner_path),
        rust_hotspots: display_path(rust_path),
        owner_hotspot_count: owner_hotspots.len() as u64,
        joined_count: 0,
        rendered_owner_count: 0,
        rendered_owner_coverage_recovered_count: 0,
        rendered_owner_subpixel_recovered_count: 0,
        rendered_owner_recovered_count: 0,
        rendered_owner_center_shading_geometry_count: 0,
        selection_count: 0,
        missing_rust_count: 0,
        missing_rendered_owner_count: 0,
        missing_recovery_count: 0,
        missing_rust_sample_count: 0,
    };
    let mut corrections = Vec::new();

    for owner_hotspot in owner_hotspots {
        let Some((x, y)) = pixel_key(owner_hotspot) else {
            continue;
        };
        let Some(rust_hotspot) = rust_by_pixel.get(&(x, y)).copied() else {
            report.missing_rust_count += 1;
            continue;
        };
        report.joined_count += 1;

        if owner_hotspot.pointer("/renderedOwner/owner").is_some() {
            report.rendered_owner_count += 1;
        } else {
            report.missing_rendered_owner_count += 1;
            continue;
        }

        let Some(recovery) = owner_recovery_sample(owner_hotspot) else {
            report.missing_recovery_count += 1;
            continue;
        };
        match recovery.source {
            RecoverySource::Coverage => report.rendered_owner_coverage_recovered_count += 1,
            RecoverySource::Subpixel => report.rendered_owner_subpixel_recovered_count += 1,
        }
        report.rendered_owner_recovered_count += 1;

        let sample_key = RenderOwnerSampleKey::from_pair(recovery.surface.clone(), recovery.sample);
        let Some((resolved_sample, geometry_source)) =
            rust_candidate_for_owner_sample(rust_hotspot, &sample_key, recovery.source)
        else {
            report.missing_rust_sample_count += 1;
            continue;
        };
        if geometry_source == GeometrySource::PixelCenter {
            report.rendered_owner_center_shading_geometry_count += 1;
        }

        corrections.push(SelectionManifestEntry {
            x,
            y,
            rgba: resolved_sample.rgba,
            surface: SelectionManifestSurface {
                material_name: recovery.surface.material_name().to_owned(),
                triangle: recovery.surface.triangle(),
            },
            sample: sample_key.sample().to_pair(),
            sample_geometry: resolved_sample.geometry,
        });
        report.selection_count += 1;
    }

    let manifest = SelectionManifest {
        generator: "vrm-rs tools/render-parity/build-owner-sample-selection.rs",
        owner_hotspots: display_path(owner_path),
        rust_hotspots: display_path(rust_path),
        corrections,
    };
    Ok((report, manifest))
}

fn owner_recovery_sample(owner_hotspot: &Value) -> Option<OwnerRecoverySample> {
    recovery_sample_at(
        owner_hotspot,
        RecoverySource::Coverage,
        "/renderedOwnerRecovery/bestCoverage",
    )
    .or_else(|| {
        recovery_sample_at(
            owner_hotspot,
            RecoverySource::Subpixel,
            "/renderedOwnerRecovery/bestSubpixel",
        )
    })
}

fn recovery_sample_at(
    owner_hotspot: &Value,
    source: RecoverySource,
    pointer: &str,
) -> Option<OwnerRecoverySample> {
    Some(OwnerRecoverySample {
        source,
        surface: surface_at(owner_hotspot, &format!("{pointer}/candidate"))?,
        sample: number_pair(owner_hotspot.pointer(&format!("{pointer}/sampleCenter")))?,
    })
}

fn rust_candidate_for_owner_sample(
    rust_hotspot: &Value,
    sample_key: &RenderOwnerSampleKey,
    source: RecoverySource,
) -> Option<(ResolvedSample, GeometrySource)> {
    let candidate_arrays: &[&str] = match source {
        RecoverySource::Coverage => &["coverage_visible_candidates", "subpixel_visible_candidates"],
        RecoverySource::Subpixel => &["subpixel_visible_candidates"],
    };
    let recovery = candidate_arrays
        .iter()
        .find_map(|array_name| rust_candidate_in_array(rust_hotspot, array_name, sample_key))?;
    if recovery.1 == GeometrySource::PixelCenter {
        return Some(recovery);
    }
    if let Some(center) = rust_center_candidate_for_surface(rust_hotspot, sample_key.surface()) {
        Some((center, GeometrySource::PixelCenter))
    } else {
        Some(recovery)
    }
}

fn rust_candidate_in_array(
    rust_hotspot: &Value,
    array_name: &str,
    sample_key: &RenderOwnerSampleKey,
) -> Option<(ResolvedSample, GeometrySource)> {
    rust_hotspot
        .get(array_name)
        .and_then(Value::as_array)?
        .iter()
        .find_map(|candidate| {
            let candidate_surface = surface_at(candidate, "/candidate")?;
            let candidate_sample = number_pair(candidate.get("sample"))?;
            if !sample_key.matches(
                &candidate_surface,
                RenderSamplePoint::from_pair(candidate_sample),
            ) {
                return None;
            }
            let geometry_source = candidate
                .get("center_candidate")
                .map(|_| GeometrySource::PixelCenter)
                .unwrap_or(GeometrySource::RecoveryPoint);
            let candidate = candidate
                .get("center_candidate")
                .or_else(|| candidate.get("candidate"))?;
            Some((
                ResolvedSample {
                    rgba: rgba_array(candidate.get("cpu_base_color_rgba")?).unwrap_or([0, 0, 0, 0]),
                    geometry: sample_geometry_from_direct_candidate(candidate)?,
                },
                geometry_source,
            ))
        })
}

fn rust_center_candidate_for_surface(
    rust_hotspot: &Value,
    surface: &RenderOwnerSurfaceKey,
) -> Option<ResolvedSample> {
    rust_hotspot
        .get("candidates")
        .and_then(Value::as_array)?
        .iter()
        .filter(|candidate| {
            surface_at(candidate, "")
                .as_ref()
                .is_some_and(|candidate_surface| candidate_surface == surface)
                && bool_field(candidate, "visible_by_policy").unwrap_or(false)
                && i64_pair(candidate.get("sample_offset")).is_some_and(|offset| offset == [0, 0])
        })
        .min_by(|left, right| {
            f64_field(left, "depth")
                .partial_cmp(&f64_field(right, "depth"))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| u64_field(right, "draw_index").cmp(&u64_field(left, "draw_index")))
        })
        .and_then(|candidate| {
            Some(ResolvedSample {
                rgba: rgba_array(candidate.get("cpu_base_color_rgba")?).unwrap_or([0, 0, 0, 0]),
                geometry: sample_geometry_from_direct_candidate(candidate)?,
            })
        })
}

fn sample_geometry_from_direct_candidate(
    candidate: &Value,
) -> Option<SelectionManifestSampleGeometry> {
    Some(SelectionManifestSampleGeometry {
        node: candidate.get("node")?.as_u64()?,
        mesh: candidate.get("mesh")?.as_u64()?,
        primitive: candidate.get("primitive")?.as_u64()?,
        triangle: candidate.get("triangle")?.as_u64()?,
        indices: u64_array3(candidate.get("indices")?)?,
        barycentric: f64_array3(candidate.get("barycentric")?)?,
        raw_uv: f64_array2(candidate.get("raw_uv")?)?,
        base_uv: f64_array2(candidate.get("base_uv")?)?,
        depth: candidate.get("depth")?.as_f64()?,
        pass: candidate.get("pass")?.as_str()?.to_owned(),
    })
}

fn surface_at(value: &Value, pointer: &str) -> Option<RenderOwnerSurfaceKey> {
    let value = value.pointer(pointer)?;
    RenderOwnerSurfaceKey::from_diagnostic_material_name(
        value
            .get("materialName")
            .or_else(|| value.get("material_name"))
            .and_then(Value::as_str)?,
        value.get("triangle").and_then(Value::as_u64)?,
    )
    .into()
}

fn pixel_key(value: &Value) -> Option<(u64, u64)> {
    Some((
        value.get("x").and_then(Value::as_u64)?,
        value.get("y").and_then(Value::as_u64)?,
    ))
}

fn number_pair(value: Option<&Value>) -> Option<[f64; 2]> {
    let values = value?.as_array()?;
    Some([values.first()?.as_f64()?, values.get(1)?.as_f64()?])
}

fn i64_pair(value: Option<&Value>) -> Option<[i64; 2]> {
    let values = value?.as_array()?;
    Some([values.first()?.as_i64()?, values.get(1)?.as_i64()?])
}

fn bool_field(value: &Value, field: &str) -> Option<bool> {
    value.get(field)?.as_bool()
}

fn f64_field(value: &Value, field: &str) -> f64 {
    value
        .get(field)
        .and_then(Value::as_f64)
        .unwrap_or(f64::INFINITY)
}

fn u64_field(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or(0)
}

fn rgba_array(value: &Value) -> Option<[u8; 4]> {
    let values = value.as_array()?;
    Some([
        u8::try_from(values.first()?.as_u64()?).ok()?,
        u8::try_from(values.get(1)?.as_u64()?).ok()?,
        u8::try_from(values.get(2)?.as_u64()?).ok()?,
        u8::try_from(values.get(3)?.as_u64()?).ok()?,
    ])
}

fn f64_array2(value: &Value) -> Option<[f64; 2]> {
    let values = value.as_array()?;
    Some([values.first()?.as_f64()?, values.get(1)?.as_f64()?])
}

fn f64_array3(value: &Value) -> Option<[f64; 3]> {
    let values = value.as_array()?;
    Some([
        values.first()?.as_f64()?,
        values.get(1)?.as_f64()?,
        values.get(2)?.as_f64()?,
    ])
}

fn u64_array3(value: &Value) -> Option<[u64; 3]> {
    let values = value.as_array()?;
    Some([
        values.first()?.as_u64()?,
        values.get(1)?.as_u64()?,
        values.get(2)?.as_u64()?,
    ])
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
    let owner = serde_json::from_str::<Value>(
        r#"{
            "reference": {"renderer": {"diagnosticHotspots": {"top": [{
                "x": 0,
                "y": 0,
                "renderedOwner": {
                    "owner": {"materialName": "body:vrm-rs-owner-id-diagnostic", "triangle": 7}
                },
                "renderedOwnerRecovery": {
                    "bestCoverage": {
                        "sampleCenter": [0.7, 0.5],
                        "candidate": {"materialName": "body:vrm-rs-owner-id-diagnostic", "triangle": 7}
                    }
                }
            }]}}}
        }"#,
    )?;
    let rust = serde_json::from_str::<Value>(
        r#"{
            "hotspots": [{
                "x": 0,
                "y": 0,
                "coverage_visible_candidates": [{
                    "sample": [0.7, 0.5],
                    "candidate": {
                        "node": 0,
                        "mesh": 1,
                        "primitive": 2,
                        "pass": "base",
                        "material_name": "body",
                        "triangle": 7,
                        "indices": [3, 4, 5],
                        "barycentric": [0.2, 0.3, 0.5],
                        "raw_uv": [0.25, 0.75],
                        "base_uv": [0.3, 0.8],
                        "depth": 0.42,
                        "cpu_base_color_rgba": [100, 101, 102, 255]
                    },
                    "center_candidate": {
                        "node": 0,
                        "mesh": 1,
                        "primitive": 2,
                        "pass": "base",
                        "material_name": "body",
                        "triangle": 7,
                        "indices": [3, 4, 5],
                        "barycentric": [0.4, 0.4, 0.2],
                        "raw_uv": [0.45, 0.55],
                        "base_uv": [0.5, 0.6],
                        "depth": 0.40,
                        "cpu_base_color_rgba": [110, 111, 112, 255]
                    }
                }]
            }]
        }"#,
    )?;
    let (report, manifest) =
        build_selection(Path::new("owner.json"), Path::new("rust.json"), &owner, &rust)?;
    assert_eq!(report.joined_count, 1);
    assert_eq!(report.rendered_owner_count, 1);
    assert_eq!(report.rendered_owner_recovered_count, 1);
    assert_eq!(report.rendered_owner_coverage_recovered_count, 1);
    assert_eq!(report.rendered_owner_center_shading_geometry_count, 1);
    assert_eq!(report.selection_count, 1);
    assert_eq!(manifest.corrections.len(), 1);
    assert_eq!(manifest.corrections[0].x, 0);
    assert_eq!(manifest.corrections[0].y, 0);
    assert_eq!(manifest.corrections[0].rgba, [110, 111, 112, 255]);
    assert_eq!(manifest.corrections[0].surface.material_name, "body");
    assert_eq!(manifest.corrections[0].surface.triangle, 7);
    assert_eq!(manifest.corrections[0].sample, [0.7, 0.5]);
    assert_eq!(manifest.corrections[0].sample_geometry.node, 0);
    assert_eq!(manifest.corrections[0].sample_geometry.mesh, 1);
    assert_eq!(manifest.corrections[0].sample_geometry.primitive, 2);
    assert_eq!(manifest.corrections[0].sample_geometry.triangle, 7);
    assert_eq!(manifest.corrections[0].sample_geometry.indices, [3, 4, 5]);
    assert_eq!(
        manifest.corrections[0].sample_geometry.barycentric,
        [0.4, 0.4, 0.2]
    );
    assert_eq!(manifest.corrections[0].sample_geometry.raw_uv, [0.45, 0.55]);
    assert_eq!(manifest.corrections[0].sample_geometry.base_uv, [0.5, 0.6]);
    assert_eq!(manifest.corrections[0].sample_geometry.depth, 0.40);
    assert_eq!(manifest.corrections[0].sample_geometry.pass, "base");
    Ok(())
}
