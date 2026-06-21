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
//! the selected geometry is driven only by the browser owner-id pass. The
//! default mode follows the WebGL raster owner: if the browser owner-id pass
//! shaded a pixel from a triangle that only covers part of that pixel, the
//! selection uses that triangle's pixel coverage point instead of rejecting it
//! or choosing a sample by RGB distance.

use clap::{Parser, ValueEnum};
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
    #[arg(long, value_enum, default_value_t = SelectionMode::WebglRasterOwner)]
    selection_mode: SelectionMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SelectionMode {
    /// Match the actual rendered owner using WebGL-style pixel coverage.
    WebglRasterOwner,
    /// Match the actual rendered owner at the pixel center.
    CenterOwner,
    /// Diagnostic mode: recover the rendered owner from coverage/subpixel samples.
    RecoveredOwner,
}

impl SelectionMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::WebglRasterOwner => "webgl-raster-owner",
            Self::CenterOwner => "center-owner",
            Self::RecoveredOwner => "recovered-owner",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct SelectionReport {
    owner_hotspots: String,
    rust_hotspots: String,
    selection_mode: &'static str,
    owner_hotspot_count: u64,
    joined_count: u64,
    rendered_owner_count: u64,
    rendered_owner_center_candidate_count: u64,
    rendered_owner_coverage_recovered_count: u64,
    rendered_owner_subpixel_recovered_count: u64,
    rendered_owner_recovered_count: u64,
    rendered_owner_center_shading_geometry_count: u64,
    selection_count: u64,
    missing_rust_count: u64,
    missing_rendered_owner_count: u64,
    missing_rendered_owner_center_candidate_count: u64,
    missing_recovery_count: u64,
    missing_rust_sample_count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionManifest {
    generator: &'static str,
    owner_hotspots: String,
    rust_hotspots: String,
    selection_mode: &'static str,
    corrections: Vec<SelectionManifestEntry>,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionManifestEntry {
    x: u64,
    y: u64,
    rgba: [u8; 4],
    surface: SelectionManifestSurface,
    sample: [f64; 2],
    selection_source: &'static str,
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
    Center,
    WebglCoverage,
    DiagnosticCoverage,
    Subpixel,
}

impl RecoverySource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::WebglCoverage => "webgl-coverage",
            Self::DiagnosticCoverage => "diagnostic-coverage",
            Self::Subpixel => "subpixel",
        }
    }
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
    let (report, manifest) =
        build_selection(owner_path, rust_path, &owner, &rust, options.selection_mode)?;
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
    selection_mode: SelectionMode,
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
        selection_mode: selection_mode.as_str(),
        owner_hotspot_count: owner_hotspots.len() as u64,
        joined_count: 0,
        rendered_owner_count: 0,
        rendered_owner_center_candidate_count: 0,
        rendered_owner_coverage_recovered_count: 0,
        rendered_owner_subpixel_recovered_count: 0,
        rendered_owner_recovered_count: 0,
        rendered_owner_center_shading_geometry_count: 0,
        selection_count: 0,
        missing_rust_count: 0,
        missing_rendered_owner_count: 0,
        missing_rendered_owner_center_candidate_count: 0,
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

        if owner_hotspot.pointer("/renderedOwnerCandidate").is_none() {
            report.missing_rendered_owner_center_candidate_count += 1;
        }

        let Some(recovery) = owner_sample_for_mode(owner_hotspot, selection_mode) else {
            report.missing_recovery_count += 1;
            continue;
        };
        match recovery.source {
            RecoverySource::Center => report.rendered_owner_center_candidate_count += 1,
            RecoverySource::WebglCoverage | RecoverySource::DiagnosticCoverage => {
                report.rendered_owner_coverage_recovered_count += 1
            }
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
            selection_source: recovery.source.as_str(),
            sample_geometry: resolved_sample.geometry,
        });
        report.selection_count += 1;
    }

    let manifest = SelectionManifest {
        generator: "vrm-rs tools/render-parity/build-owner-sample-selection.rs",
        owner_hotspots: display_path(owner_path),
        rust_hotspots: display_path(rust_path),
        selection_mode: selection_mode.as_str(),
        corrections,
    };
    Ok((report, manifest))
}

fn owner_sample_for_mode(
    owner_hotspot: &Value,
    selection_mode: SelectionMode,
) -> Option<OwnerRecoverySample> {
    match selection_mode {
        SelectionMode::WebglRasterOwner => owner_webgl_raster_sample(owner_hotspot),
        SelectionMode::CenterOwner => owner_center_sample(owner_hotspot),
        SelectionMode::RecoveredOwner => owner_recovery_sample(owner_hotspot),
    }
}

fn owner_webgl_raster_sample(owner_hotspot: &Value) -> Option<OwnerRecoverySample> {
    owner_center_sample(owner_hotspot).or_else(|| {
        recovery_sample_at(
            owner_hotspot,
            RecoverySource::WebglCoverage,
            "/renderedOwnerRecovery/bestCoverage",
        )
    })
}

fn owner_center_sample(owner_hotspot: &Value) -> Option<OwnerRecoverySample> {
    Some(OwnerRecoverySample {
        source: RecoverySource::Center,
        surface: surface_at(owner_hotspot, "/renderedOwnerCandidate")?,
        sample: [0.5, 0.5],
    })
}

fn owner_recovery_sample(owner_hotspot: &Value) -> Option<OwnerRecoverySample> {
    recovery_sample_at(
        owner_hotspot,
        RecoverySource::DiagnosticCoverage,
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
        RecoverySource::Center => {
            return rust_center_candidate_for_surface(
                rust_hotspot,
                sample_key.surface(),
                CenterCandidatePolicy::StrictInside,
            )
            .map(|sample| (sample, GeometrySource::PixelCenter));
        }
        RecoverySource::WebglCoverage | RecoverySource::DiagnosticCoverage => {
            &["coverage_visible_candidates", "subpixel_visible_candidates"]
        }
        RecoverySource::Subpixel => &["subpixel_visible_candidates"],
    };
    let recovery = candidate_arrays
        .iter()
        .find_map(|array_name| {
            rust_candidate_in_array(rust_hotspot, array_name, sample_key)
        })?;
    if recovery.1 == GeometrySource::PixelCenter {
        return Some(recovery);
    }
    if let Some(center) = rust_center_candidate_for_surface(
        rust_hotspot,
        sample_key.surface(),
        CenterCandidatePolicy::LooseInside,
    ) {
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
            let center_candidate = candidate.get("center_candidate");
            let geometry_source = center_candidate
                .map(|_| GeometrySource::PixelCenter)
                .unwrap_or(GeometrySource::RecoveryPoint);
            let candidate = center_candidate.or_else(|| candidate.get("candidate"))?;
            Some((
                ResolvedSample {
                    rgba: rgba_array(candidate.get("cpu_base_color_rgba")?).unwrap_or([0, 0, 0, 0]),
                    geometry: sample_geometry_from_direct_candidate(candidate)?,
                },
                geometry_source,
            ))
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CenterCandidatePolicy {
    StrictInside,
    LooseInside,
}

fn rust_center_candidate_for_surface(
    rust_hotspot: &Value,
    surface: &RenderOwnerSurfaceKey,
    policy: CenterCandidatePolicy,
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
                && center_candidate_matches_policy(candidate, policy)
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

fn center_candidate_matches_policy(candidate: &Value, policy: CenterCandidatePolicy) -> bool {
    let Some(min_barycentric) = f64_optional_field(candidate, "min_barycentric") else {
        return false;
    };
    match policy {
        CenterCandidatePolicy::StrictInside => min_barycentric >= 0.0,
        CenterCandidatePolicy::LooseInside => min_barycentric >= -0.00001,
    }
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

fn f64_optional_field(value: &Value, field: &str) -> Option<f64> {
    value.get(field).and_then(Value::as_f64)
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
                "renderedOwnerCandidate": {
                    "materialName": "body",
                    "triangle": 7
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
                "candidates": [{
                    "node": 0,
                    "mesh": 1,
                    "primitive": 2,
                    "pass": "base",
                    "material_name": "body",
                    "triangle": 7,
                    "indices": [3, 4, 5],
                    "barycentric": [0.4, 0.4, 0.2],
                    "min_barycentric": 0.2,
                    "raw_uv": [0.45, 0.55],
                    "base_uv": [0.5, 0.6],
                    "depth": 0.40,
                    "draw_index": 9,
                    "visible_by_policy": true,
                    "sample_offset": [0, 0],
                    "cpu_base_color_rgba": [110, 111, 112, 255]
                }],
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
                        "min_barycentric": 0.2,
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
                        "min_barycentric": 0.2,
                        "raw_uv": [0.45, 0.55],
                        "base_uv": [0.5, 0.6],
                        "depth": 0.40,
                        "cpu_base_color_rgba": [110, 111, 112, 255]
                    }
                }]
            }]
        }"#,
    )?;
    let (report, manifest) = build_selection(
        Path::new("owner.json"),
        Path::new("rust.json"),
        &owner,
        &rust,
        SelectionMode::CenterOwner,
    )?;
    assert_eq!(report.joined_count, 1);
    assert_eq!(report.rendered_owner_count, 1);
    assert_eq!(report.rendered_owner_center_candidate_count, 1);
    assert_eq!(report.rendered_owner_recovered_count, 1);
    assert_eq!(report.rendered_owner_coverage_recovered_count, 0);
    assert_eq!(report.rendered_owner_center_shading_geometry_count, 1);
    assert_eq!(report.selection_count, 1);
    assert_eq!(manifest.corrections.len(), 1);
    assert_eq!(manifest.corrections[0].x, 0);
    assert_eq!(manifest.corrections[0].y, 0);
    assert_eq!(manifest.corrections[0].rgba, [110, 111, 112, 255]);
    assert_eq!(manifest.corrections[0].surface.material_name, "body");
    assert_eq!(manifest.corrections[0].surface.triangle, 7);
    assert_eq!(manifest.corrections[0].sample, [0.5, 0.5]);
    assert_eq!(manifest.corrections[0].selection_source, "center");
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

    let (default_report, default_manifest) = build_selection(
        Path::new("owner.json"),
        Path::new("rust.json"),
        &owner,
        &rust,
        SelectionMode::WebglRasterOwner,
    )?;
    assert_eq!(default_report.selection_mode, "webgl-raster-owner");
    assert_eq!(default_report.rendered_owner_center_candidate_count, 1);
    assert_eq!(default_report.selection_count, 1);
    assert_eq!(default_manifest.corrections[0].sample, [0.5, 0.5]);
    assert_eq!(default_manifest.corrections[0].selection_source, "center");
    assert_eq!(
        default_manifest.corrections[0].sample_geometry.barycentric,
        [0.4, 0.4, 0.2]
    );

    let (recovered_report, recovered_manifest) = build_selection(
        Path::new("owner.json"),
        Path::new("rust.json"),
        &owner,
        &rust,
        SelectionMode::RecoveredOwner,
    )?;
    assert_eq!(recovered_report.rendered_owner_coverage_recovered_count, 1);
    assert_eq!(recovered_manifest.corrections[0].sample, [0.7, 0.5]);
    assert_eq!(
        recovered_manifest.corrections[0].selection_source,
        "diagnostic-coverage"
    );

    let coverage_owner = serde_json::from_str::<Value>(
        r#"{
            "reference": {"renderer": {"diagnosticHotspots": {"top": [{
                "x": 1,
                "y": 2,
                "renderedOwner": {
                    "owner": {"materialName": "body:vrm-rs-owner-id-diagnostic", "triangle": 9}
                },
                "renderedOwnerRecovery": {
                    "bestCoverage": {
                        "sampleCenter": [0.25, 0.75],
                        "candidate": {"materialName": "body:vrm-rs-owner-id-diagnostic", "triangle": 9}
                    }
                }
            }]}}}
        }"#,
    )?;
    let coverage_rust = serde_json::from_str::<Value>(
        r#"{
            "hotspots": [{
                "x": 1,
                "y": 2,
                "candidates": [],
                "coverage_visible_candidates": [{
                    "sample": [0.25, 0.75],
                    "candidate": {
                        "node": 0,
                        "mesh": 1,
                        "primitive": 2,
                        "pass": "base",
                        "material_name": "body",
                        "triangle": 9,
                        "indices": [3, 4, 5],
                        "barycentric": [0.1, 0.2, 0.7],
                        "min_barycentric": 0.1,
                        "raw_uv": [0.2, 0.3],
                        "base_uv": [0.4, 0.5],
                        "depth": 0.6,
                        "draw_index": 9,
                        "visible_by_policy": true,
                        "sample_offset": [0, 0],
                        "cpu_base_color_rgba": [10, 20, 30, 255]
                    },
                    "center_candidate": {
                        "node": 0,
                        "mesh": 1,
                        "primitive": 2,
                        "pass": "base",
                        "material_name": "body",
                        "triangle": 9,
                        "indices": [3, 4, 5],
                        "barycentric": [1.1, 0.2, -0.3],
                        "min_barycentric": -0.3,
                        "raw_uv": [0.8, 0.9],
                        "base_uv": [0.8, 0.9],
                        "depth": 0.7,
                        "draw_index": 9,
                        "visible_by_policy": true,
                        "sample_offset": [0, 0],
                        "cpu_base_color_rgba": [200, 210, 220, 255]
                    }
                }]
            }]
        }"#,
    )?;
    let (coverage_report, coverage_manifest) = build_selection(
        Path::new("owner.json"),
        Path::new("rust.json"),
        &coverage_owner,
        &coverage_rust,
        SelectionMode::WebglRasterOwner,
    )?;
    assert_eq!(coverage_report.selection_count, 1);
    assert_eq!(coverage_report.rendered_owner_coverage_recovered_count, 1);
    assert_eq!(coverage_report.rendered_owner_center_shading_geometry_count, 1);
    assert_eq!(coverage_manifest.corrections[0].sample, [0.25, 0.75]);
    assert_eq!(
        coverage_manifest.corrections[0].selection_source,
        "webgl-coverage"
    );
    assert_eq!(coverage_manifest.corrections[0].rgba, [200, 210, 220, 255]);
    assert_eq!(
        coverage_manifest.corrections[0].sample_geometry.barycentric,
        [1.1, 0.2, -0.3]
    );
    assert_eq!(coverage_manifest.corrections[0].sample_geometry.raw_uv, [0.8, 0.9]);
    Ok(())
}
