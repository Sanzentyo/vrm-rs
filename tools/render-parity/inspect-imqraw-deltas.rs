#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
imq = { git = "https://github.com/Sanzentyo/imq.git", rev = "0fdc5263c0c21bd6d7bc55c194e98b593bf83bff", default-features = false }
serde_json = "1.0.150"
---

//! Inspect worst per-pixel deltas between two direct renderer `imqraw` RGBA8 artifacts.

use clap::Parser;
use imq::{PixelFormat, RawImageRecord, decode_imqraw_bundle};
use serde_json::{Value, json};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "inspect-imqraw-deltas",
    about = "Report worst per-pixel deltas between two single-frame imqraw artifacts"
)]
struct Options {
    #[arg(long)]
    expected: PathBuf,
    #[arg(long)]
    actual: PathBuf,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long, default_value_t = 32)]
    top: usize,
    #[arg(long, default_value_t = 1)]
    min_channel_delta: u8,
    #[arg(long, default_value_t = 0)]
    expected_index: usize,
    #[arg(long, default_value_t = 0)]
    actual_index: usize,
}

#[derive(Clone, Debug)]
struct RgbaImage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
struct PixelDelta {
    pixel: usize,
    max_channel_delta: u8,
    max_rgb_delta: u8,
    alpha_delta: u8,
    rgb_distance: f64,
    rgba_distance: f64,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    let expected = read_imqraw_rgba8(&options.expected, options.expected_index)?;
    let actual = read_imqraw_rgba8(&options.actual, options.actual_index)?;
    if expected.width != actual.width || expected.height != actual.height {
        return Err(format!(
            "image dimensions differ: expected {}x{}, actual {}x{}",
            expected.width, expected.height, actual.width, actual.height
        )
        .into());
    }

    let mut deltas = pixel_deltas(&expected, &actual, options.min_channel_delta);
    deltas.sort_by(|left, right| {
        right
            .max_channel_delta
            .cmp(&left.max_channel_delta)
            .then_with(|| {
                right
                    .rgba_distance
                    .partial_cmp(&left.rgba_distance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.pixel.cmp(&right.pixel))
    });

    let report = json!({
        "expected": display_path(&options.expected),
        "actual": display_path(&options.actual),
        "width": expected.width,
        "height": expected.height,
        "minChannelDelta": options.min_channel_delta,
        "changedPixels": deltas.len(),
        "summary": delta_summary(&expected, &actual, &deltas),
        "top": deltas
            .iter()
            .take(options.top)
            .map(|delta| pixel_delta_json(&expected, &actual, delta))
            .collect::<Vec<_>>(),
    });

    let formatted = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = options.out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, formatted)?;
    } else {
        print!("{formatted}");
    }
    Ok(())
}

fn pixel_deltas(expected: &RgbaImage, actual: &RgbaImage, min_channel_delta: u8) -> Vec<PixelDelta> {
    (0..expected.rgba.len())
        .step_by(4)
        .filter_map(|pixel| {
            let channel_deltas = channel_deltas(&expected.rgba, &actual.rgba, pixel);
            let max_channel_delta = channel_deltas.into_iter().max().unwrap_or(0);
            if max_channel_delta < min_channel_delta {
                return None;
            }
            let max_rgb_delta = channel_deltas[0..3].iter().copied().max().unwrap_or(0);
            let rgb_distance = channel_deltas[0..3]
                .iter()
                .map(|delta| f64::from(*delta).powi(2))
                .sum::<f64>()
                .sqrt();
            let rgba_distance = channel_deltas
                .iter()
                .map(|delta| f64::from(*delta).powi(2))
                .sum::<f64>()
                .sqrt();
            Some(PixelDelta {
                pixel,
                max_channel_delta,
                max_rgb_delta,
                alpha_delta: channel_deltas[3],
                rgb_distance,
                rgba_distance,
            })
        })
        .collect()
}

fn channel_deltas(expected: &[u8], actual: &[u8], pixel: usize) -> [u8; 4] {
    std::array::from_fn(|channel| {
        (i16::from(actual[pixel + channel]) - i16::from(expected[pixel + channel])).unsigned_abs()
            as u8
    })
}

fn delta_summary(expected: &RgbaImage, actual: &RgbaImage, deltas: &[PixelDelta]) -> Value {
    let mut visible = 0usize;
    let mut interior_visible = 0usize;
    let mut nonblack = 0usize;
    let mut interior_nonblack = 0usize;
    let mut alpha_changed = 0usize;
    let mut rgb_changed = 0usize;
    let mut max_channel_delta = 0u8;
    let mut max_rgb_delta = 0u8;
    let mut max_alpha_delta = 0u8;
    let mut bounds = Bounds::default();

    for delta in deltas {
        bounds.include(delta.pixel / 4, expected.width);
        max_channel_delta = max_channel_delta.max(delta.max_channel_delta);
        max_rgb_delta = max_rgb_delta.max(delta.max_rgb_delta);
        max_alpha_delta = max_alpha_delta.max(delta.alpha_delta);
        if delta.max_rgb_delta > 0 {
            rgb_changed += 1;
        }
        if delta.alpha_delta > 0 {
            alpha_changed += 1;
        }
        if is_visible(expected, actual, delta.pixel) {
            visible += 1;
        }
        if is_interior_visible(expected, actual, delta.pixel) {
            interior_visible += 1;
        }
        if is_nonblack(expected, actual, delta.pixel) {
            nonblack += 1;
        }
        if is_interior_nonblack(expected, actual, delta.pixel) {
            interior_nonblack += 1;
        }
    }

    json!({
        "maxChannelDelta": max_channel_delta,
        "maxRgbDelta": max_rgb_delta,
        "maxAlphaDelta": max_alpha_delta,
        "rgbChangedPixels": rgb_changed,
        "alphaChangedPixels": alpha_changed,
        "visibleChangedPixels": visible,
        "interiorVisibleChangedPixels": interior_visible,
        "nonblackChangedPixels": nonblack,
        "interiorNonblackChangedPixels": interior_nonblack,
        "bounds": bounds.to_json(),
    })
}

fn pixel_delta_json(expected: &RgbaImage, actual: &RgbaImage, delta: &PixelDelta) -> Value {
    let pixel_index = delta.pixel / 4;
    let x = pixel_index % expected.width;
    let y = pixel_index / expected.width;
    json!({
        "x": x,
        "y": y,
        "pixel": pixel_index,
        "expected": rgba_at(expected, delta.pixel),
        "actual": rgba_at(actual, delta.pixel),
        "delta": channel_deltas(&expected.rgba, &actual.rgba, delta.pixel),
        "maxChannelDelta": delta.max_channel_delta,
        "maxRgbDelta": delta.max_rgb_delta,
        "alphaDelta": delta.alpha_delta,
        "rgbDistance": delta.rgb_distance,
        "rgbaDistance": delta.rgba_distance,
        "visible": is_visible(expected, actual, delta.pixel),
        "interiorVisible": is_interior_visible(expected, actual, delta.pixel),
        "nonblack": is_nonblack(expected, actual, delta.pixel),
        "interiorNonblack": is_interior_nonblack(expected, actual, delta.pixel),
    })
}

fn rgba_at(image: &RgbaImage, pixel: usize) -> [u8; 4] {
    std::array::from_fn(|channel| image.rgba[pixel + channel])
}

#[derive(Clone, Copy, Debug, Default)]
struct Bounds {
    min_x: Option<usize>,
    min_y: Option<usize>,
    max_x: Option<usize>,
    max_y: Option<usize>,
}

impl Bounds {
    fn include(&mut self, pixel_index: usize, width: usize) {
        let x = pixel_index % width;
        let y = pixel_index / width;
        self.min_x = Some(self.min_x.map_or(x, |value| value.min(x)));
        self.min_y = Some(self.min_y.map_or(y, |value| value.min(y)));
        self.max_x = Some(self.max_x.map_or(x, |value| value.max(x)));
        self.max_y = Some(self.max_y.map_or(y, |value| value.max(y)));
    }

    fn to_json(self) -> Value {
        match (self.min_x, self.min_y, self.max_x, self.max_y) {
            (Some(min_x), Some(min_y), Some(max_x), Some(max_y)) => json!({
                "minX": min_x,
                "minY": min_y,
                "maxX": max_x,
                "maxY": max_y,
            }),
            _ => Value::Null,
        }
    }
}

fn is_visible(expected: &RgbaImage, actual: &RgbaImage, pixel: usize) -> bool {
    expected.rgba[pixel + 3] != 0 || actual.rgba[pixel + 3] != 0
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

fn is_interior(image: &RgbaImage, pixel: usize, include_neighbor: impl Fn(usize) -> bool) -> bool {
    let pixel_index = pixel / 4;
    let x = pixel_index % image.width;
    let y = pixel_index / image.width;
    if x == 0 || y == 0 || x == image.width - 1 || y == image.height - 1 {
        return false;
    }
    for dy in [usize::MAX, 0, 1] {
        for dx in [usize::MAX, 0, 1] {
            let neighbor_x = x.wrapping_add(dx);
            let neighbor_y = y.wrapping_add(dy);
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

fn pixel_rgb_nonzero(rgba: &[u8], pixel: usize) -> bool {
    rgba[pixel] != 0 || rgba[pixel + 1] != 0 || rgba[pixel + 2] != 0
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

fn display_path(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
