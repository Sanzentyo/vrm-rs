#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
imq = { git = "https://github.com/Sanzentyo/imq.git", rev = "0fdc5263c0c21bd6d7bc55c194e98b593bf83bff", default-features = false }
serde_json = "1.0.150"
---

//! Compare two direct renderer `imqraw` RGBA8 artifacts for VRM render parity.

use clap::Parser;
use imq::{PixelFormat, RawImageRecord, decode_imqraw_bundle};
use serde_json::{Value, json};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

const CHANGE_THRESHOLDS: [u8; 4] = [32, 64, 96, 128];

#[derive(Clone, Debug, Parser)]
#[command(
    name = "compare-imqraw",
    about = "Compare two single-frame imqraw render parity artifacts"
)]
struct Options {
    #[arg(long)]
    expected: PathBuf,
    #[arg(long)]
    actual: PathBuf,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long, default_value = "rgba")]
    metric: MetricName,
    #[arg(long)]
    fail_under: Option<f64>,
    #[arg(long)]
    max_selected_channel_delta: Option<u8>,
    #[arg(long)]
    max_alpha_delta: Option<u8>,
    #[arg(long, default_value_t = 0)]
    expected_index: usize,
    #[arg(long, default_value_t = 0)]
    actual_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MetricName {
    Rgba,
    RgbAll,
    RgbOpaque,
    RgbVisible,
    RgbNonblack,
    RgbInterior1px,
    RgbVisibleInterior1px,
    RgbNonblackInterior1px,
    RgbSharedNonblackInterior1px,
    RgbSharedNonblackInterior2px,
    RgbSharedNonblackInterior3px,
    RgbSharedNonblackFlat32Interior1px,
    RgbSharedNonblackGradientInterior1px,
}

impl std::str::FromStr for MetricName {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "rgba" => Ok(Self::Rgba),
            "rgb-all" => Ok(Self::RgbAll),
            "rgb-opaque" => Ok(Self::RgbOpaque),
            "rgb-visible" => Ok(Self::RgbVisible),
            "rgb-nonblack" => Ok(Self::RgbNonblack),
            "rgb-interior1px" => Ok(Self::RgbInterior1px),
            "rgb-visible-interior1px" => Ok(Self::RgbVisibleInterior1px),
            "rgb-nonblack-interior1px" => Ok(Self::RgbNonblackInterior1px),
            "rgb-shared-nonblack-interior1px" => Ok(Self::RgbSharedNonblackInterior1px),
            "rgb-shared-nonblack-interior2px" => Ok(Self::RgbSharedNonblackInterior2px),
            "rgb-shared-nonblack-interior3px" => Ok(Self::RgbSharedNonblackInterior3px),
            "rgb-shared-nonblack-flat32-interior1px" => {
                Ok(Self::RgbSharedNonblackFlat32Interior1px)
            }
            "rgb-shared-nonblack-gradient-interior1px" => {
                Ok(Self::RgbSharedNonblackGradientInterior1px)
            }
            other => Err(format!(
                "invalid metric `{other}`; expected rgba, rgb-all, rgb-opaque, rgb-visible, rgb-nonblack, rgb-interior1px, rgb-visible-interior1px, rgb-nonblack-interior1px, rgb-shared-nonblack-interior1px, rgb-shared-nonblack-interior2px, rgb-shared-nonblack-interior3px, rgb-shared-nonblack-flat32-interior1px, or rgb-shared-nonblack-gradient-interior1px"
            )),
        }
    }
}

impl MetricName {
    fn as_str(self) -> &'static str {
        match self {
            Self::Rgba => "rgba",
            Self::RgbAll => "rgb-all",
            Self::RgbOpaque => "rgb-opaque",
            Self::RgbVisible => "rgb-visible",
            Self::RgbNonblack => "rgb-nonblack",
            Self::RgbInterior1px => "rgb-interior1px",
            Self::RgbVisibleInterior1px => "rgb-visible-interior1px",
            Self::RgbNonblackInterior1px => "rgb-nonblack-interior1px",
            Self::RgbSharedNonblackInterior1px => "rgb-shared-nonblack-interior1px",
            Self::RgbSharedNonblackInterior2px => "rgb-shared-nonblack-interior2px",
            Self::RgbSharedNonblackInterior3px => "rgb-shared-nonblack-interior3px",
            Self::RgbSharedNonblackFlat32Interior1px => {
                "rgb-shared-nonblack-flat32-interior1px"
            }
            Self::RgbSharedNonblackGradientInterior1px => {
                "rgb-shared-nonblack-gradient-interior1px"
            }
        }
    }
}

#[derive(Clone, Debug)]
struct RgbaImage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
struct Metric {
    pixel_count: usize,
    channel_count: usize,
    mse: Option<f64>,
    mae: Option<f64>,
    psnr: Option<f64>,
    max_channel_delta: u8,
    max_pixel_delta: f64,
}

#[derive(Clone, Copy, Debug)]
struct AlphaStats {
    expected: AlphaCounts,
    actual: AlphaCounts,
    mismatches: usize,
    max_delta: u8,
    mismatches_beyond_one: usize,
}

#[derive(Clone, Copy, Debug)]
struct ChangedPixelStats {
    rgba_changed: usize,
    rgb_changed: usize,
    alpha_changed: usize,
    expected_only_rgb: usize,
    actual_only_rgb: usize,
    shared_nonblack_rgb_changed: usize,
    shared_nonblack_interior1px_rgb_changed: usize,
    shared_nonblack_interior2px_rgb_changed: usize,
    shared_nonblack_interior3px_rgb_changed: usize,
    shared_nonblack_flat32_interior1px_rgb_changed: usize,
    shared_nonblack_gradient_interior1px_rgb_changed: usize,
    high_delta: [HighDeltaStats; CHANGE_THRESHOLDS.len()],
}

impl Default for ChangedPixelStats {
    fn default() -> Self {
        let mut high_delta = [HighDeltaStats::default(); CHANGE_THRESHOLDS.len()];
        for (bucket, threshold) in high_delta.iter_mut().zip(CHANGE_THRESHOLDS) {
            bucket.threshold = threshold;
        }
        Self {
            rgba_changed: 0,
            rgb_changed: 0,
            alpha_changed: 0,
            expected_only_rgb: 0,
            actual_only_rgb: 0,
            shared_nonblack_rgb_changed: 0,
            shared_nonblack_interior1px_rgb_changed: 0,
            shared_nonblack_interior2px_rgb_changed: 0,
            shared_nonblack_interior3px_rgb_changed: 0,
            shared_nonblack_flat32_interior1px_rgb_changed: 0,
            shared_nonblack_gradient_interior1px_rgb_changed: 0,
            high_delta,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct HighDeltaStats {
    threshold: u8,
    rgb: usize,
    expected_only_rgb: usize,
    actual_only_rgb: usize,
    shared_nonblack_rgb: usize,
    shared_nonblack_interior1px_rgb: usize,
    shared_nonblack_interior2px_rgb: usize,
    shared_nonblack_interior3px_rgb: usize,
    shared_nonblack_flat32_interior1px_rgb: usize,
    shared_nonblack_gradient_interior1px_rgb: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct AlphaCounts {
    transparent: usize,
    opaque: usize,
    partial: usize,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    validate_thresholds(&options)?;
    let expected = read_imqraw_rgba8(&options.expected, options.expected_index)?;
    let actual = read_imqraw_rgba8(&options.actual, options.actual_index)?;
    if expected.width != actual.width || expected.height != actual.height {
        return Err(format!(
            "image dimensions differ: expected {}x{}, actual {}x{}",
            expected.width, expected.height, actual.width, actual.height
        )
        .into());
    }

    let full_image = compare_channels(&expected, &actual, |_| true, &[0, 1, 2, 3]);
    let all_rgb = compare_channels(&expected, &actual, |_| true, &[0, 1, 2]);
    let opaque_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| expected.rgba[pixel + 3] == 255 && actual.rgba[pixel + 3] == 255,
        &[0, 1, 2],
    );
    let visible_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| expected.rgba[pixel + 3] > 0 || actual.rgba[pixel + 3] > 0,
        &[0, 1, 2],
    );
    let nonblack_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| is_nonblack(&expected, &actual, pixel),
        &[0, 1, 2],
    );
    let interior_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| is_interior_opaque(&expected, &actual, pixel),
        &[0, 1, 2],
    );
    let visible_interior_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| is_interior_visible(&expected, &actual, pixel),
        &[0, 1, 2],
    );
    let nonblack_interior_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| is_interior_nonblack(&expected, &actual, pixel),
        &[0, 1, 2],
    );
    let shared_nonblack_interior_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| is_interior_shared_nonblack(&expected, &actual, pixel),
        &[0, 1, 2],
    );
    let shared_nonblack_interior_2px_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| is_interior_radius(&expected, pixel, 2, |neighbor| {
            is_shared_nonblack(&expected, &actual, neighbor)
        }),
        &[0, 1, 2],
    );
    let shared_nonblack_interior_3px_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| is_interior_radius(&expected, pixel, 3, |neighbor| {
            is_shared_nonblack(&expected, &actual, neighbor)
        }),
        &[0, 1, 2],
    );
    let shared_nonblack_flat32_interior_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| is_flat_shared_nonblack_interior(&expected, &actual, pixel, 1, 32),
        &[0, 1, 2],
    );
    let shared_nonblack_gradient_interior_rgb = compare_channels(
        &expected,
        &actual,
        |pixel| is_gradient_shared_nonblack_interior(&expected, &actual, pixel, 1, 32),
        &[0, 1, 2],
    );
    let alpha = alpha_stats(&expected, &actual);
    let changed_pixels = changed_pixel_stats(&expected, &actual);
    let selected = select_metric(
        options.metric,
        &[
            (MetricName::Rgba, full_image),
            (MetricName::RgbAll, all_rgb),
            (MetricName::RgbOpaque, opaque_rgb),
            (MetricName::RgbVisible, visible_rgb),
            (MetricName::RgbNonblack, nonblack_rgb),
            (MetricName::RgbInterior1px, interior_rgb),
            (MetricName::RgbVisibleInterior1px, visible_interior_rgb),
            (MetricName::RgbNonblackInterior1px, nonblack_interior_rgb),
            (
                MetricName::RgbSharedNonblackInterior1px,
                shared_nonblack_interior_rgb,
            ),
            (
                MetricName::RgbSharedNonblackInterior2px,
                shared_nonblack_interior_2px_rgb,
            ),
            (
                MetricName::RgbSharedNonblackInterior3px,
                shared_nonblack_interior_3px_rgb,
            ),
            (
                MetricName::RgbSharedNonblackFlat32Interior1px,
                shared_nonblack_flat32_interior_rgb,
            ),
            (
                MetricName::RgbSharedNonblackGradientInterior1px,
                shared_nonblack_gradient_interior_rgb,
            ),
        ],
    )?;
    let pass = pass_status(selected, alpha, &options);
    let report = json!({
        "expected": display_path(&options.expected),
        "actual": display_path(&options.actual),
        "width": expected.width,
        "height": expected.height,
        "channels": 4,
        "sourceFormat": "imqraw",
        "mse": full_image.mse,
        "psnr": psnr_value(full_image.psnr),
        "maxChannelDelta": full_image.max_channel_delta,
        "maxPixelDelta": full_image.max_pixel_delta,
        "alpha": alpha_report(alpha),
        "changedPixels": changed_pixel_report(changed_pixels),
        "rgbAll": metric_report(all_rgb),
        "rgbOpaque": metric_report(opaque_rgb),
        "rgbVisible": metric_report(visible_rgb),
        "rgbNonblack": metric_report(nonblack_rgb),
        "rgbInterior1px": metric_report(interior_rgb),
        "rgbVisibleInterior1px": metric_report(visible_interior_rgb),
        "rgbNonblackInterior1px": metric_report(nonblack_interior_rgb),
        "rgbSharedNonblackInterior1px": metric_report(shared_nonblack_interior_rgb),
        "rgbSharedNonblackInterior2px": metric_report(shared_nonblack_interior_2px_rgb),
        "rgbSharedNonblackInterior3px": metric_report(shared_nonblack_interior_3px_rgb),
        "rgbSharedNonblackFlat32Interior1px": metric_report(shared_nonblack_flat32_interior_rgb),
        "rgbSharedNonblackGradientInterior1px": metric_report(shared_nonblack_gradient_interior_rgb),
        "selectedMetric": selected_metric_report(options.metric, selected),
        "pass": pass,
        "thresholds": {
            "failUnder": options.fail_under,
            "maxSelectedChannelDelta": options.max_selected_channel_delta,
            "maxAlphaDelta": options.max_alpha_delta,
        },
        "failUnder": options.fail_under,
    });
    let output = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = &options.out {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, output)?;
    } else {
        print!("{output}");
    }
    if !pass {
        if let Some(threshold) = options.fail_under
            && selected.psnr.is_none_or(|psnr| psnr < threshold)
        {
            eprintln!(
                "PSNR {} dB for {} is below threshold {threshold} dB",
                selected
                    .psnr
                    .map(|psnr| format!("{psnr:.4}"))
                    .unwrap_or_else(|| "null".to_owned()),
                options.metric.as_str()
            );
        }
        if let Some(threshold) = options.max_selected_channel_delta
            && selected.max_channel_delta > threshold
        {
            eprintln!(
                "max selected channel delta {} for {} exceeds threshold {threshold}",
                selected.max_channel_delta,
                options.metric.as_str()
            );
        }
        if let Some(threshold) = options.max_alpha_delta
            && alpha.max_delta > threshold
        {
            eprintln!(
                "max alpha delta {} exceeds threshold {threshold}",
                alpha.max_delta
            );
        }
        std::process::exit(4);
    }
    Ok(())
}

fn validate_thresholds(options: &Options) -> Result<(), Box<dyn Error>> {
    if let Some(value) = options.fail_under
        && (!value.is_finite() || value < 0.0)
    {
        return Err(format!("invalid --fail-under: {value}").into());
    }
    Ok(())
}

fn read_imqraw_rgba8(path: &Path, index: usize) -> Result<RgbaImage, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let bundle = decode_imqraw_bundle(&bytes)?;
    let record = bundle.select_index(index)?;
    record_to_rgba8(path, index, record)
}

fn record_to_rgba8(
    path: &Path,
    index: usize,
    record: &RawImageRecord,
) -> Result<RgbaImage, Box<dyn Error>> {
    if record.frame.format().pixel_format != PixelFormat::Rgba8 {
        return Err(format!(
            "{}[{index}]: expected Rgba8, got {:?}",
            path.display(),
            record.frame.format().pixel_format
        )
        .into());
    }
    let dims = record.frame.dimensions();
    let width = usize::try_from(dims.width)?;
    let height = usize::try_from(dims.height)?;
    let row_bytes = width
        .checked_mul(4)
        .ok_or("RGBA row byte count overflows usize")?;
    let plane = record
        .frame
        .owned_planes()
        .first()
        .ok_or("Rgba8 imqraw record has no plane")?;
    if plane.stride < row_bytes {
        return Err(format!(
            "{}[{index}]: stride {} is smaller than row bytes {row_bytes}",
            path.display(),
            plane.stride
        )
        .into());
    }
    let required = height
        .checked_sub(1)
        .and_then(|last_row| last_row.checked_mul(plane.stride))
        .and_then(|last_row_start| last_row_start.checked_add(row_bytes))
        .ok_or("RGBA buffer size overflows usize")?;
    if plane.data.len() < required {
        return Err(format!(
            "{}[{index}]: plane has {} bytes, needs {required}",
            path.display(),
            plane.data.len()
        )
        .into());
    }
    let mut rgba = Vec::with_capacity(row_bytes * height);
    for row in 0..height {
        let offset = row * plane.stride;
        rgba.extend_from_slice(&plane.data[offset..offset + row_bytes]);
    }
    Ok(RgbaImage {
        width,
        height,
        rgba,
    })
}

fn compare_channels(
    expected: &RgbaImage,
    actual: &RgbaImage,
    include_pixel: impl Fn(usize) -> bool,
    channels: &[usize],
) -> Metric {
    let mut squared_error = 0.0;
    let mut absolute_error = 0.0;
    let mut sample_count = 0;
    let mut pixel_count = 0;
    let mut max_channel_delta = 0u8;
    let mut max_pixel_delta = 0.0f64;
    for pixel in (0..expected.rgba.len()).step_by(4) {
        if !include_pixel(pixel) {
            continue;
        }
        let mut pixel_squared = 0.0;
        for channel in channels {
            let delta =
                i32::from(actual.rgba[pixel + channel]) - i32::from(expected.rgba[pixel + channel]);
            let absolute = delta.unsigned_abs() as u8;
            max_channel_delta = max_channel_delta.max(absolute);
            let squared = f64::from(delta * delta);
            squared_error += squared;
            absolute_error += f64::from(absolute);
            pixel_squared += squared;
            sample_count += 1;
        }
        pixel_count += 1;
        max_pixel_delta = max_pixel_delta.max(pixel_squared.sqrt());
    }
    if sample_count == 0 {
        return Metric {
            pixel_count,
            channel_count: 0,
            mse: None,
            mae: None,
            psnr: None,
            max_channel_delta,
            max_pixel_delta,
        };
    }
    let mse = squared_error / sample_count as f64;
    let mae = absolute_error / sample_count as f64;
    Metric {
        pixel_count,
        channel_count: sample_count,
        mse: Some(mse),
        mae: Some(mae),
        psnr: Some(if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * ((255.0 * 255.0) / mse).log10()
        }),
        max_channel_delta,
        max_pixel_delta,
    }
}

fn select_metric(
    name: MetricName,
    metrics: &[(MetricName, Metric)],
) -> Result<Metric, Box<dyn Error>> {
    let metric = metrics
        .iter()
        .find_map(|(candidate, metric)| (*candidate == name).then_some(*metric))
        .ok_or("missing selected metric")?;
    if metric.channel_count == 0 {
        return Err(format!("selected metric {} has no comparable pixels", name.as_str()).into());
    }
    Ok(metric)
}

fn alpha_stats(expected: &RgbaImage, actual: &RgbaImage) -> AlphaStats {
    (0..expected.rgba.len())
        .step_by(4)
        .fold(AlphaStats::default(), |mut stats, pixel| {
            count_alpha(&mut stats.expected, expected.rgba[pixel + 3]);
            count_alpha(&mut stats.actual, actual.rgba[pixel + 3]);
            let delta = actual.rgba[pixel + 3].abs_diff(expected.rgba[pixel + 3]);
            stats.max_delta = stats.max_delta.max(delta);
            if delta != 0 {
                stats.mismatches += 1;
            }
            if delta > 1 {
                stats.mismatches_beyond_one += 1;
            }
            stats
        })
}

fn changed_pixel_stats(expected: &RgbaImage, actual: &RgbaImage) -> ChangedPixelStats {
    (0..expected.rgba.len())
        .step_by(4)
        .fold(ChangedPixelStats::default(), |mut stats, pixel| {
            let rgb_changed = rgb_pixel_changed(expected, actual, pixel);
            let alpha_changed = expected.rgba[pixel + 3] != actual.rgba[pixel + 3];
            if rgb_changed || alpha_changed {
                stats.rgba_changed += 1;
            }
            if alpha_changed {
                stats.alpha_changed += 1;
            }
            if !rgb_changed {
                return stats;
            }

            stats.rgb_changed += 1;
            let rgb_max_delta = rgb_pixel_max_delta(expected, actual, pixel);
            let expected_nonblack = pixel_rgb_nonzero(&expected.rgba, pixel);
            let actual_nonblack = pixel_rgb_nonzero(&actual.rgba, pixel);
            match (expected_nonblack, actual_nonblack) {
                (true, false) => {
                    stats.expected_only_rgb += 1;
                    count_high_delta(
                        &mut stats.high_delta,
                        rgb_max_delta,
                        HighDeltaDomain {
                            expected_only: true,
                            ..HighDeltaDomain::default()
                        },
                    );
                }
                (false, true) => {
                    stats.actual_only_rgb += 1;
                    count_high_delta(
                        &mut stats.high_delta,
                        rgb_max_delta,
                        HighDeltaDomain {
                            actual_only: true,
                            ..HighDeltaDomain::default()
                        },
                    );
                }
                (true, true) => {
                    stats.shared_nonblack_rgb_changed += 1;
                    let interior1 = is_interior_shared_nonblack(expected, actual, pixel);
                    let interior2 = is_interior_radius(expected, pixel, 2, |neighbor| {
                        is_shared_nonblack(expected, actual, neighbor)
                    });
                    let interior3 = is_interior_radius(expected, pixel, 3, |neighbor| {
                        is_shared_nonblack(expected, actual, neighbor)
                    });
                    let flat32 = is_flat_shared_nonblack_interior(expected, actual, pixel, 1, 32);
                    let gradient =
                        is_gradient_shared_nonblack_interior(expected, actual, pixel, 1, 32);
                    if interior1 {
                        stats.shared_nonblack_interior1px_rgb_changed += 1;
                    }
                    if interior2 {
                        stats.shared_nonblack_interior2px_rgb_changed += 1;
                    }
                    if interior3 {
                        stats.shared_nonblack_interior3px_rgb_changed += 1;
                    }
                    if flat32 {
                        stats.shared_nonblack_flat32_interior1px_rgb_changed += 1;
                    }
                    if gradient {
                        stats.shared_nonblack_gradient_interior1px_rgb_changed += 1;
                    }
                    count_high_delta(
                        &mut stats.high_delta,
                        rgb_max_delta,
                        HighDeltaDomain {
                            shared_nonblack: true,
                            interior1,
                            interior2,
                            interior3,
                            flat32,
                            gradient,
                            ..HighDeltaDomain::default()
                        },
                    );
                }
                (false, false) => count_high_delta(
                    &mut stats.high_delta,
                    rgb_max_delta,
                    HighDeltaDomain::default(),
                ),
            }
            stats
        })
}

#[derive(Clone, Copy, Debug, Default)]
struct HighDeltaDomain {
    expected_only: bool,
    actual_only: bool,
    shared_nonblack: bool,
    interior1: bool,
    interior2: bool,
    interior3: bool,
    flat32: bool,
    gradient: bool,
}

fn count_high_delta(
    high_delta: &mut [HighDeltaStats; CHANGE_THRESHOLDS.len()],
    rgb_max_delta: u8,
    domain: HighDeltaDomain,
) {
    for (index, threshold) in CHANGE_THRESHOLDS.iter().copied().enumerate() {
        let bucket = &mut high_delta[index];
        bucket.threshold = threshold;
        if rgb_max_delta < threshold {
            continue;
        }
        bucket.rgb += 1;
        if domain.expected_only {
            bucket.expected_only_rgb += 1;
        }
        if domain.actual_only {
            bucket.actual_only_rgb += 1;
        }
        if !domain.shared_nonblack {
            continue;
        }
        bucket.shared_nonblack_rgb += 1;
        if domain.interior1 {
            bucket.shared_nonblack_interior1px_rgb += 1;
        }
        if domain.interior2 {
            bucket.shared_nonblack_interior2px_rgb += 1;
        }
        if domain.interior3 {
            bucket.shared_nonblack_interior3px_rgb += 1;
        }
        if domain.flat32 {
            bucket.shared_nonblack_flat32_interior1px_rgb += 1;
        }
        if domain.gradient {
            bucket.shared_nonblack_gradient_interior1px_rgb += 1;
        }
    }
}

impl Default for AlphaStats {
    fn default() -> Self {
        Self {
            expected: AlphaCounts::default(),
            actual: AlphaCounts::default(),
            mismatches: 0,
            max_delta: 0,
            mismatches_beyond_one: 0,
        }
    }
}

fn count_alpha(counts: &mut AlphaCounts, alpha: u8) {
    match alpha {
        0 => counts.transparent += 1,
        255 => counts.opaque += 1,
        _ => counts.partial += 1,
    }
}

fn pass_status(selected: Metric, alpha: AlphaStats, options: &Options) -> bool {
    options
        .fail_under
        .is_none_or(|threshold| selected.psnr.is_some_and(|psnr| psnr >= threshold))
        && options
            .max_selected_channel_delta
            .is_none_or(|threshold| selected.max_channel_delta <= threshold)
        && options
            .max_alpha_delta
            .is_none_or(|threshold| alpha.max_delta <= threshold)
}

fn metric_report(metric: Metric) -> Value {
    json!({
        "pixels": metric.pixel_count,
        "channels": metric.channel_count,
        "mse": metric.mse,
        "mae": metric.mae,
        "psnr": psnr_value(metric.psnr),
        "maxChannelDelta": metric.max_channel_delta,
        "maxPixelDelta": metric.max_pixel_delta,
    })
}

fn selected_metric_report(name: MetricName, metric: Metric) -> Value {
    let mut value = metric_report(metric);
    value
        .as_object_mut()
        .expect("metric report is an object")
        .insert("name".to_owned(), json!(name.as_str()));
    value
}

fn alpha_report(alpha: AlphaStats) -> Value {
    json!({
        "expected": alpha_counts_report(alpha.expected),
        "actual": alpha_counts_report(alpha.actual),
        "mismatches": alpha.mismatches,
        "maxDelta": alpha.max_delta,
        "mismatchesBeyondOne": alpha.mismatches_beyond_one,
    })
}

fn changed_pixel_report(stats: ChangedPixelStats) -> Value {
    let shared_edge_1px =
        stats.shared_nonblack_rgb_changed - stats.shared_nonblack_interior1px_rgb_changed;
    let shared_edge_2px =
        stats.shared_nonblack_rgb_changed - stats.shared_nonblack_interior2px_rgb_changed;
    let shared_edge_3px =
        stats.shared_nonblack_rgb_changed - stats.shared_nonblack_interior3px_rgb_changed;
    json!({
        "scope": "changed pixels; RGB-domain fields exclude alpha-only changes",
        "rgba": stats.rgba_changed,
        "rgb": stats.rgb_changed,
        "alpha": stats.alpha_changed,
        "expectedOnlyRgb": stats.expected_only_rgb,
        "actualOnlyRgb": stats.actual_only_rgb,
        "sharedNonblackRgb": stats.shared_nonblack_rgb_changed,
        "sharedNonblackInterior1pxRgb": stats.shared_nonblack_interior1px_rgb_changed,
        "sharedNonblackInterior2pxRgb": stats.shared_nonblack_interior2px_rgb_changed,
        "sharedNonblackInterior3pxRgb": stats.shared_nonblack_interior3px_rgb_changed,
        "sharedNonblackEdgeBand1pxRgb": shared_edge_1px,
        "sharedNonblackEdgeBand2pxRgb": shared_edge_2px,
        "sharedNonblackEdgeBand3pxRgb": shared_edge_3px,
        "sharedNonblackEdgeBand1pxRatio": ratio_value(shared_edge_1px, stats.shared_nonblack_rgb_changed),
        "sharedNonblackEdgeBand2pxRatio": ratio_value(shared_edge_2px, stats.shared_nonblack_rgb_changed),
        "sharedNonblackEdgeBand3pxRatio": ratio_value(shared_edge_3px, stats.shared_nonblack_rgb_changed),
        "sharedNonblackEdgeBand1pxRatioOfSharedNonblackRgb": ratio_value(shared_edge_1px, stats.shared_nonblack_rgb_changed),
        "sharedNonblackEdgeBand2pxRatioOfSharedNonblackRgb": ratio_value(shared_edge_2px, stats.shared_nonblack_rgb_changed),
        "sharedNonblackEdgeBand3pxRatioOfSharedNonblackRgb": ratio_value(shared_edge_3px, stats.shared_nonblack_rgb_changed),
        "sharedNonblackFlat32Interior1pxRgb": stats.shared_nonblack_flat32_interior1px_rgb_changed,
        "sharedNonblackGradientInterior1pxRgb": stats.shared_nonblack_gradient_interior1px_rgb_changed,
        "highDelta": stats.high_delta.map(high_delta_report),
    })
}

fn high_delta_report(stats: HighDeltaStats) -> Value {
    let edge1 = stats.shared_nonblack_rgb - stats.shared_nonblack_interior1px_rgb;
    let edge2 = stats.shared_nonblack_rgb - stats.shared_nonblack_interior2px_rgb;
    let edge3 = stats.shared_nonblack_rgb - stats.shared_nonblack_interior3px_rgb;
    let coverage_only = stats.expected_only_rgb + stats.actual_only_rgb;
    json!({
        "scope": "changed RGB pixels whose max RGB channel delta is at least maxChannelDeltaGte",
        "maxChannelDeltaGte": stats.threshold,
        "rgb": stats.rgb,
        "expectedOnlyRgb": stats.expected_only_rgb,
        "actualOnlyRgb": stats.actual_only_rgb,
        "coverageOnlyRgb": coverage_only,
        "expectedOnlyRatioOfRgb": ratio_value(stats.expected_only_rgb, stats.rgb),
        "actualOnlyRatioOfRgb": ratio_value(stats.actual_only_rgb, stats.rgb),
        "coverageOnlyRatioOfRgb": ratio_value(coverage_only, stats.rgb),
        "sharedNonblackRgb": stats.shared_nonblack_rgb,
        "sharedNonblackRatioOfRgb": ratio_value(stats.shared_nonblack_rgb, stats.rgb),
        "sharedNonblackInterior1pxRgb": stats.shared_nonblack_interior1px_rgb,
        "sharedNonblackInterior2pxRgb": stats.shared_nonblack_interior2px_rgb,
        "sharedNonblackInterior3pxRgb": stats.shared_nonblack_interior3px_rgb,
        "sharedNonblackEdgeBand1pxRgb": edge1,
        "sharedNonblackEdgeBand2pxRgb": edge2,
        "sharedNonblackEdgeBand3pxRgb": edge3,
        "sharedNonblackEdgeBand1pxRatio": ratio_value(edge1, stats.shared_nonblack_rgb),
        "sharedNonblackEdgeBand2pxRatio": ratio_value(edge2, stats.shared_nonblack_rgb),
        "sharedNonblackEdgeBand3pxRatio": ratio_value(edge3, stats.shared_nonblack_rgb),
        "sharedNonblackEdgeBand1pxRatioOfSharedNonblackRgb": ratio_value(edge1, stats.shared_nonblack_rgb),
        "sharedNonblackEdgeBand2pxRatioOfSharedNonblackRgb": ratio_value(edge2, stats.shared_nonblack_rgb),
        "sharedNonblackEdgeBand3pxRatioOfSharedNonblackRgb": ratio_value(edge3, stats.shared_nonblack_rgb),
        "sharedNonblackFlat32Interior1pxRgb": stats.shared_nonblack_flat32_interior1px_rgb,
        "sharedNonblackGradientInterior1pxRgb": stats.shared_nonblack_gradient_interior1px_rgb,
    })
}

fn alpha_counts_report(counts: AlphaCounts) -> Value {
    json!({
        "transparent": counts.transparent,
        "opaque": counts.opaque,
        "partial": counts.partial,
    })
}

fn ratio_value(numerator: usize, denominator: usize) -> Value {
    if denominator == 0 {
        Value::Null
    } else {
        json!(numerator as f64 / denominator as f64)
    }
}

fn psnr_value(psnr: Option<f64>) -> Value {
    match psnr {
        None => Value::Null,
        Some(value) if value.is_infinite() => json!("Infinity"),
        Some(value) => json!(value),
    }
}

fn display_path(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn is_interior_opaque(expected: &RgbaImage, actual: &RgbaImage, pixel: usize) -> bool {
    is_interior(expected, pixel, |neighbor| {
        expected.rgba[neighbor + 3] == 255 && actual.rgba[neighbor + 3] == 255
    })
}

fn is_interior_visible(expected: &RgbaImage, actual: &RgbaImage, pixel: usize) -> bool {
    is_interior(expected, pixel, |neighbor| {
        expected.rgba[neighbor + 3] != 0 && actual.rgba[neighbor + 3] != 0
    })
}

fn is_interior_nonblack(expected: &RgbaImage, actual: &RgbaImage, pixel: usize) -> bool {
    is_interior(expected, pixel, |neighbor| {
        is_nonblack(expected, actual, neighbor)
    })
}

fn is_interior_shared_nonblack(expected: &RgbaImage, actual: &RgbaImage, pixel: usize) -> bool {
    is_interior(expected, pixel, |neighbor| {
        is_shared_nonblack(expected, actual, neighbor)
    })
}

fn is_interior(image: &RgbaImage, pixel: usize, include_neighbor: impl Fn(usize) -> bool) -> bool {
    is_interior_radius(image, pixel, 1, include_neighbor)
}

fn is_interior_radius(
    image: &RgbaImage,
    pixel: usize,
    radius: usize,
    include_neighbor: impl Fn(usize) -> bool,
) -> bool {
    let pixel_index = pixel / 4;
    let x = pixel_index % image.width;
    let y = pixel_index / image.width;
    if x < radius
        || y < radius
        || x + radius >= image.width
        || y + radius >= image.height
    {
        return false;
    }
    for dy in 0..=(radius * 2) {
        for dx in 0..=(radius * 2) {
            let neighbor_x = x + dx - radius;
            let neighbor_y = y + dy - radius;
            let neighbor = (neighbor_y * image.width + neighbor_x) * 4;
            if !include_neighbor(neighbor) {
                return false;
            }
        }
    }
    true
}

fn is_nonblack(expected: &RgbaImage, actual: &RgbaImage, pixel: usize) -> bool {
    pixel_rgb_nonzero(&expected.rgba, pixel) || pixel_rgb_nonzero(&actual.rgba, pixel)
}

fn is_shared_nonblack(expected: &RgbaImage, actual: &RgbaImage, pixel: usize) -> bool {
    pixel_rgb_nonzero(&expected.rgba, pixel) && pixel_rgb_nonzero(&actual.rgba, pixel)
}

fn is_flat_shared_nonblack_interior(
    expected: &RgbaImage,
    actual: &RgbaImage,
    pixel: usize,
    radius: usize,
    max_channel_delta: u8,
) -> bool {
    is_interior_radius(expected, pixel, radius, |neighbor| {
        is_shared_nonblack(expected, actual, neighbor)
            && rgb_max_delta(&expected.rgba, pixel, neighbor) <= max_channel_delta
            && rgb_max_delta(&actual.rgba, pixel, neighbor) <= max_channel_delta
    })
}

fn is_gradient_shared_nonblack_interior(
    expected: &RgbaImage,
    actual: &RgbaImage,
    pixel: usize,
    radius: usize,
    max_channel_delta: u8,
) -> bool {
    is_interior_shared_nonblack(expected, actual, pixel)
        && !is_flat_shared_nonblack_interior(expected, actual, pixel, radius, max_channel_delta)
}

fn rgb_max_delta(rgba: &[u8], left: usize, right: usize) -> u8 {
    (0..3)
        .map(|channel| rgba[left + channel].abs_diff(rgba[right + channel]))
        .max()
        .unwrap_or(0)
}

fn rgb_pixel_changed(expected: &RgbaImage, actual: &RgbaImage, pixel: usize) -> bool {
    (0..3).any(|channel| expected.rgba[pixel + channel] != actual.rgba[pixel + channel])
}

fn rgb_pixel_max_delta(expected: &RgbaImage, actual: &RgbaImage, pixel: usize) -> u8 {
    (0..3)
        .map(|channel| expected.rgba[pixel + channel].abs_diff(actual.rgba[pixel + channel]))
        .max()
        .unwrap_or(0)
}

fn pixel_rgb_nonzero(rgba: &[u8], pixel: usize) -> bool {
    rgba[pixel] != 0 || rgba[pixel + 1] != 0 || rgba[pixel + 2] != 0
}
