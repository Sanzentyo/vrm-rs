#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
imq = { git = "https://github.com/Sanzentyo/imq.git", rev = "0fdc5263c0c21bd6d7bc55c194e98b593bf83bff", default-features = false }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
vrm-adapter = { path = "../../crates/vrm-adapter" }
---

//! Apply browser owner/sample colors to a renderer raw image as a parity upper-bound experiment.

use clap::Parser;
use imq::{
    FrameOwned, PixelFormat, RawImageBundle, RawImageRecord, decode_imqraw_bundle,
    encode_imqraw_bundle,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use vrm_adapter::{
    RenderOwnerSampleCorrectionOutcome, RenderOwnerSampleCorrectionPolicy, RenderOwnerSurfaceKey,
    RenderOwnerSampleKey, RenderSamplePoint, evaluate_render_owner_sample_correction,
};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "apply-owner-sample-correction",
    about = "Apply browser best owner/sample CPU colors to an imqraw render artifact"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    expected: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    actual: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    owner_hotspots: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    rust_hotspots: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    corrected_imqraw_out: Option<PathBuf>,
    #[arg(long, default_value_t = 0)]
    expected_index: usize,
    #[arg(long, default_value_t = 0)]
    actual_index: usize,
    #[arg(long)]
    only_expected_closer: bool,
}

#[derive(Clone, Debug)]
struct RgbaImage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
struct CorrectionReport {
    expected: String,
    actual: String,
    owner_hotspots: String,
    rust_hotspots: String,
    image_width: usize,
    image_height: usize,
    owner_hotspot_count: u64,
    joined_count: u64,
    candidate_color_count: u64,
    applied_count: u64,
    skipped_not_expected_closer: u64,
    before_all_rgb_psnr: Option<f64>,
    after_all_rgb_psnr: Option<f64>,
    before_corrected_pixel_mean_rgb_distance: Option<f64>,
    after_corrected_pixel_mean_rgb_distance: Option<f64>,
    corrected_pixel_improved: u64,
    corrected_pixel_worsened: u64,
    corrected_pixel_tied: u64,
    corrected_relation_to_expected: std::collections::BTreeMap<String, u64>,
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
    let expected_path = options.expected.as_ref().ok_or("missing --expected")?;
    let actual_path = options.actual.as_ref().ok_or("missing --actual")?;
    let owner_path = options
        .owner_hotspots
        .as_ref()
        .ok_or("missing --owner-hotspots")?;
    let rust_path = options
        .rust_hotspots
        .as_ref()
        .ok_or("missing --rust-hotspots")?;

    let expected = read_imqraw_rgba8(expected_path, options.expected_index)?;
    let actual = read_imqraw_rgba8(actual_path, options.actual_index)?;
    let owner = serde_json::from_str::<Value>(&fs::read_to_string(owner_path)?)?;
    let rust = serde_json::from_str::<Value>(&fs::read_to_string(rust_path)?)?;
    let (report, corrected) = correction_report(
        expected_path,
        actual_path,
        owner_path,
        rust_path,
        &expected,
        &actual,
        &owner,
        &rust,
        options.only_expected_closer,
    )?;
    let json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = &options.out {
        write_file(path, &json)?;
    } else {
        print!("{json}");
    }
    if let Some(path) = &options.corrected_imqraw_out {
        write_imqraw_rgba8(path, corrected.width, corrected.height, &corrected.rgba)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn correction_report(
    expected_path: &Path,
    actual_path: &Path,
    owner_path: &Path,
    rust_path: &Path,
    expected: &RgbaImage,
    actual: &RgbaImage,
    owner: &Value,
    rust: &Value,
    only_expected_closer: bool,
) -> Result<(CorrectionReport, RgbaImage), Box<dyn Error>> {
    if expected.width != actual.width || expected.height != actual.height {
        return Err(format!(
            "image dimensions differ: expected {}x{}, actual {}x{}",
            expected.width, expected.height, actual.width, actual.height
        )
        .into());
    }

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

    let mut corrected = actual.clone();
    let mut joined_count = 0;
    let mut candidate_color_count = 0;
    let mut applied_count = 0;
    let mut skipped_not_expected_closer = 0;
    let mut before_distance_sum = 0.0;
    let mut after_distance_sum = 0.0;
    let mut improved = 0;
    let mut worsened = 0;
    let mut tied = 0;
    let mut relation_counts = std::collections::BTreeMap::new();

    for owner_hotspot in owner_hotspots {
        let Some((x, y)) = pixel_key(owner_hotspot) else {
            continue;
        };
        let Some(rust_hotspot) = rust_by_pixel.get(&(x, y)).copied() else {
            continue;
        };
        joined_count += 1;
        let Some(browser_best) =
            surface_at(owner_hotspot, "/renderedOwnerRecovery/bestSubpixel/candidate")
        else {
            continue;
        };
        let Some(sample) = number_pair(owner_hotspot.pointer(
            "/renderedOwnerRecovery/bestSubpixel/sampleCenter",
        )) else {
            continue;
        };
        let sample_key = RenderOwnerSampleKey::from_pair(browser_best.clone(), sample);
        let Some(color) = rust_subpixel_color_for_owner_sample(rust_hotspot, &sample_key) else {
            continue;
        };
        candidate_color_count += 1;
        let expected_rgba = pixel_rgba(expected, x, y)?;
        let actual_rgba = pixel_rgba(actual, x, y)?;
        let policy = if only_expected_closer {
            RenderOwnerSampleCorrectionPolicy::improving_only()
        } else {
            RenderOwnerSampleCorrectionPolicy::allow_any()
        };
        let Some(correction) =
            evaluate_render_owner_sample_correction(expected_rgba, actual_rgba, color, policy)
        else {
            skipped_not_expected_closer += 1;
            continue;
        };
        before_distance_sum += correction.before_rgb_distance;
        after_distance_sum += correction.after_rgb_distance;
        match correction.outcome {
            RenderOwnerSampleCorrectionOutcome::Improved => improved += 1,
            RenderOwnerSampleCorrectionOutcome::Worsened => worsened += 1,
            RenderOwnerSampleCorrectionOutcome::Tied => tied += 1,
        };
        let rust_expected = surface_at(rust_hotspot, "/best_subpixel_visible_expected/candidate");
        *relation_counts
            .entry(browser_best.relation_to(rust_expected.as_ref()).as_str().to_owned())
            .or_default() += 1;
        set_pixel_rgba(&mut corrected, x, y, color)?;
        applied_count += 1;
    }

    let report = CorrectionReport {
        expected: display_path(expected_path),
        actual: display_path(actual_path),
        owner_hotspots: display_path(owner_path),
        rust_hotspots: display_path(rust_path),
        image_width: expected.width,
        image_height: expected.height,
        owner_hotspot_count: owner_hotspots.len() as u64,
        joined_count,
        candidate_color_count,
        applied_count,
        skipped_not_expected_closer,
        before_all_rgb_psnr: rgb_psnr(expected, actual),
        after_all_rgb_psnr: rgb_psnr(expected, &corrected),
        before_corrected_pixel_mean_rgb_distance: mean(applied_count, before_distance_sum),
        after_corrected_pixel_mean_rgb_distance: mean(applied_count, after_distance_sum),
        corrected_pixel_improved: improved,
        corrected_pixel_worsened: worsened,
        corrected_pixel_tied: tied,
        corrected_relation_to_expected: relation_counts,
    };
    Ok((report, corrected))
}

fn read_imqraw_rgba8(path: &Path, index: usize) -> Result<RgbaImage, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let bundle = decode_imqraw_bundle(&bytes)?;
    let record = bundle.select_index(index)?;
    record_to_tight_rgba8(record, path)
}

fn record_to_tight_rgba8(
    record: &RawImageRecord,
    source: &Path,
) -> Result<RgbaImage, Box<dyn Error>> {
    if record.frame.format().pixel_format != PixelFormat::Rgba8 {
        return Err(format!(
            "{}: expected Rgba8 imqraw frame, found {:?}",
            display_path(source),
            record.frame.format().pixel_format
        )
        .into());
    }
    let dimensions = record.frame.dimensions();
    let width = usize::try_from(dimensions.width)?;
    let height = usize::try_from(dimensions.height)?;
    let expected_row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| format!("{}: image row byte count overflows", display_path(source)))?;
    let plane = record
        .frame
        .owned_planes()
        .first()
        .ok_or_else(|| format!("{}: Rgba8 imqraw record has no plane", display_path(source)))?;
    let stride = plane.stride;
    if stride < expected_row_bytes {
        return Err(format!(
            "{}: imqraw stride {stride} is smaller than tight RGBA row {expected_row_bytes}",
            display_path(source)
        )
        .into());
    }
    let required_len = stride
        .checked_mul(height)
        .ok_or_else(|| format!("{}: image byte count overflows", display_path(source)))?;
    if plane.data.len() < required_len {
        return Err(format!(
            "{}: imqraw plane has {} bytes, expected at least {required_len}",
            display_path(source),
            plane.data.len()
        )
        .into());
    }

    let mut rgba = Vec::with_capacity(expected_row_bytes * height);
    for row in 0..height {
        let start = row * stride;
        rgba.extend_from_slice(&plane.data[start..start + expected_row_bytes]);
    }
    Ok(RgbaImage {
        width,
        height,
        rgba,
    })
}

fn write_imqraw_rgba8(
    path: &Path,
    width: usize,
    height: usize,
    rgba: &[u8],
) -> Result<(), Box<dyn Error>> {
    let frame = FrameOwned::packed_tight(
        rgba.to_vec(),
        u32::try_from(width)?,
        u32::try_from(height)?,
        PixelFormat::Rgba8,
    )?;
    let record = RawImageRecord::new(Some("owner-sample-corrected".to_owned()), Vec::new(), frame);
    let bytes = encode_imqraw_bundle(&RawImageBundle::new(vec![record]))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn rust_subpixel_color_for_owner_sample(
    rust_hotspot: &Value,
    sample_key: &RenderOwnerSampleKey,
) -> Option<[u8; 4]> {
    rust_hotspot
        .get("subpixel_visible_candidates")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|candidate| {
            let candidate_surface = surface_at(candidate, "/candidate")?;
            let candidate_sample = number_pair(candidate.get("sample"))?;
            sample_key
                .matches(
                    &candidate_surface,
                    RenderSamplePoint::from_pair(candidate_sample),
                )
                .then(|| candidate.pointer("/candidate/cpu_base_color_rgba"))
                .flatten()
                .and_then(rgba_array)
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

fn rgba_array(value: &Value) -> Option<[u8; 4]> {
    let values = value.as_array()?;
    Some([
        u8::try_from(values.first()?.as_u64()?).ok()?,
        u8::try_from(values.get(1)?.as_u64()?).ok()?,
        u8::try_from(values.get(2)?.as_u64()?).ok()?,
        u8::try_from(values.get(3)?.as_u64()?).ok()?,
    ])
}

fn pixel_rgba(image: &RgbaImage, x: u64, y: u64) -> Result<[u8; 4], Box<dyn Error>> {
    let x = usize::try_from(x)?;
    let y = usize::try_from(y)?;
    if x >= image.width || y >= image.height {
        return Err(format!("pixel {x},{y} is outside {}x{}", image.width, image.height).into());
    }
    let index = (y * image.width + x) * 4;
    Ok([
        image.rgba[index],
        image.rgba[index + 1],
        image.rgba[index + 2],
        image.rgba[index + 3],
    ])
}

fn set_pixel_rgba(
    image: &mut RgbaImage,
    x: u64,
    y: u64,
    color: [u8; 4],
) -> Result<(), Box<dyn Error>> {
    let x = usize::try_from(x)?;
    let y = usize::try_from(y)?;
    if x >= image.width || y >= image.height {
        return Err(format!("pixel {x},{y} is outside {}x{}", image.width, image.height).into());
    }
    let index = (y * image.width + x) * 4;
    image.rgba[index..index + 4].copy_from_slice(&color);
    Ok(())
}

fn rgb_psnr(expected: &RgbaImage, actual: &RgbaImage) -> Option<f64> {
    let channel_count = expected.width.checked_mul(expected.height)?.checked_mul(3)?;
    if channel_count == 0 {
        return None;
    }
    let squared_error = expected
        .rgba
        .chunks_exact(4)
        .zip(actual.rgba.chunks_exact(4))
        .map(|(expected, actual)| {
            expected
                .iter()
                .zip(actual.iter())
                .take(3)
                .map(|(expected, actual)| {
                    let delta = f64::from(*expected) - f64::from(*actual);
                    delta * delta
                })
                .sum::<f64>()
        })
        .sum::<f64>();
    if squared_error == 0.0 {
        Some(f64::INFINITY)
    } else {
        let mse = squared_error / channel_count as f64;
        Some(20.0 * (255.0 / mse.sqrt()).log10())
    }
}

fn mean(count: u64, sum: f64) -> Option<f64> {
    (count > 0).then(|| sum / count as f64)
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
    let expected = RgbaImage {
        width: 2,
        height: 1,
        rgba: vec![100, 100, 100, 255, 20, 20, 20, 255],
    };
    let actual = RgbaImage {
        width: 2,
        height: 1,
        rgba: vec![10, 10, 10, 255, 20, 20, 20, 255],
    };
    let owner = serde_json::from_str::<Value>(
        r#"{
            "reference": {"renderer": {"diagnosticHotspots": {"top": [{
                "x": 0,
                "y": 0,
                "renderedOwnerRecovery": {
                    "bestSubpixel": {
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
                "best_subpixel_visible_expected": {
                    "candidate": {"material_name": "body", "triangle": 7}
                },
                "subpixel_visible_candidates": [{
                    "sample": [0.7, 0.5],
                    "candidate": {
                        "material_name": "body",
                        "triangle": 7,
                        "cpu_base_color_rgba": [100, 100, 100, 255]
                    }
                }]
            }]
        }"#,
    )?;
    let (report, corrected) = correction_report(
        Path::new("expected.imqraw"),
        Path::new("actual.imqraw"),
        Path::new("owner.json"),
        Path::new("rust.json"),
        &expected,
        &actual,
        &owner,
        &rust,
        false,
    )?;
    assert_eq!(report.joined_count, 1);
    assert_eq!(report.candidate_color_count, 1);
    assert_eq!(report.applied_count, 1);
    assert_eq!(report.corrected_pixel_improved, 1);
    assert_eq!(
        report.corrected_relation_to_expected.get("same-surface"),
        Some(&1)
    );
    assert_eq!(&corrected.rgba[0..4], &[100, 100, 100, 255]);
    assert!(
        report.after_all_rgb_psnr.unwrap_or_default()
            > report.before_all_rgb_psnr.unwrap_or_default()
    );
    Ok(())
}
