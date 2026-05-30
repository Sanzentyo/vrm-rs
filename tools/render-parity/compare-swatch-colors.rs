#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

use clap::Parser;
use serde::Deserialize;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "compare-swatch-colors",
    about = "Compare connected opaque swatches in two render-parity RGBA JSON artifacts"
)]
struct Options {
    #[arg(long)]
    expected: PathBuf,
    #[arg(long)]
    actual: PathBuf,
    #[arg(long, value_delimiter = ',')]
    names: Vec<String>,
    #[arg(long, default_value_t = 50.0)]
    fail_under: f64,
    #[arg(long, default_value_t = 2)]
    max_channel_delta: u8,
    #[arg(long, default_value_t = 100)]
    min_pixels: usize,
    #[arg(long)]
    json_out: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
struct RgbaJson {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct SwatchReport {
    name: String,
    pixels: usize,
    interior_pixels: usize,
    bbox: [usize; 4],
    mean_delta: [f64; 3],
    max_channel_delta: u8,
    psnr: String,
    pass: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
struct Report {
    expected: String,
    actual: String,
    swatches: Vec<SwatchReport>,
    pass: bool,
    fail_under: f64,
    max_channel_delta: u8,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse();
    let expected = read_rgba_json(&options.expected)?;
    let actual = read_rgba_json(&options.actual)?;
    if expected.width != actual.width || expected.height != actual.height {
        return Err(format!(
            "image dimensions differ: expected {}x{}, actual {}x{}",
            expected.width, expected.height, actual.width, actual.height
        )
        .into());
    }
    if expected.rgba.len() != actual.rgba.len() {
        return Err(format!(
            "image buffer lengths differ: expected {}, actual {}",
            expected.rgba.len(),
            actual.rgba.len()
        )
        .into());
    }

    let components = opaque_components(&expected, options.min_pixels);
    if components.is_empty() {
        return Err(format!(
            "no opaque swatches with at least {} pixels were found",
            options.min_pixels
        )
        .into());
    }
    let swatches = components
        .iter()
        .enumerate()
        .map(|(index, component)| {
            let fallback = format!("swatch-{index}");
            let name = options.names.get(index).unwrap_or(&fallback).clone();
            compare_swatch(&name, component, &expected, &actual, &options)
        })
        .collect::<Vec<_>>();
    let pass = swatches.iter().all(|swatch| swatch.pass);
    let report = Report {
        expected: options.expected.display().to_string(),
        actual: options.actual.display().to_string(),
        swatches,
        pass,
        fail_under: options.fail_under,
        max_channel_delta: options.max_channel_delta,
    };

    let json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = &options.json_out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
    } else {
        print!("{json}");
    }

    if pass {
        Ok(())
    } else {
        Err("one or more swatches failed color thresholds".into())
    }
}

fn read_rgba_json(path: &Path) -> Result<RgbaJson, Box<dyn std::error::Error>> {
    let image: RgbaJson = serde_json::from_str(&fs::read_to_string(path)?)?;
    let expected_len = image.width * image.height * 4;
    if image.rgba.len() != expected_len {
        return Err(format!(
            "{}: rgba length {} does not match {}",
            path.display(),
            image.rgba.len(),
            expected_len
        )
        .into());
    }
    Ok(image)
}

fn opaque_components(image: &RgbaJson, min_pixels: usize) -> Vec<Vec<usize>> {
    let mut seen = vec![false; image.width * image.height];
    let mut components = Vec::new();
    for y in 1..image.height.saturating_sub(1) {
        for x in 1..image.width.saturating_sub(1) {
            let pixel = y * image.width + x;
            if seen[pixel] || alpha(image, pixel) != 255 {
                continue;
            }
            let component = flood_fill_opaque(image, pixel, &mut seen);
            if component.len() >= min_pixels {
                components.push(component);
            }
        }
    }
    components.sort_by(|left, right| {
        let [left_x0, left_y0, ..] = bbox(left, image.width);
        let [right_x0, right_y0, ..] = bbox(right, image.width);
        left_y0.cmp(&right_y0).then(left_x0.cmp(&right_x0))
    });
    components
}

fn flood_fill_opaque(image: &RgbaJson, start: usize, seen: &mut [bool]) -> Vec<usize> {
    let mut queue = VecDeque::from([start]);
    let mut component = Vec::new();
    seen[start] = true;
    while let Some(pixel) = queue.pop_front() {
        component.push(pixel);
        let x = pixel % image.width;
        let y = pixel / image.width;
        for (nx, ny) in [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ] {
            if nx == 0 || ny == 0 || nx >= image.width - 1 || ny >= image.height - 1 {
                continue;
            }
            let next = ny * image.width + nx;
            if seen[next] || alpha(image, next) != 255 {
                continue;
            }
            seen[next] = true;
            queue.push_back(next);
        }
    }
    component
}

fn compare_swatch(
    name: &str,
    component: &[usize],
    expected: &RgbaJson,
    actual: &RgbaJson,
    options: &Options,
) -> SwatchReport {
    let mut squared_error = 0.0;
    let mut channels = 0usize;
    let mut mean_delta = [0.0; 3];
    let mut max_channel_delta = 0u8;
    let mut interior_pixels = 0usize;
    for &pixel in component {
        if !is_interior_opaque(pixel, expected, actual) {
            continue;
        }
        interior_pixels += 1;
        let offset = pixel * 4;
        for channel in 0..3 {
            let delta =
                actual.rgba[offset + channel] as i16 - expected.rgba[offset + channel] as i16;
            mean_delta[channel] += f64::from(delta);
            squared_error += f64::from(delta * delta);
            max_channel_delta = max_channel_delta.max(delta.unsigned_abs() as u8);
            channels += 1;
        }
    }
    let psnr = if channels == 0 {
        None
    } else {
        let mse = squared_error / channels as f64;
        Some(if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * ((255.0 * 255.0) / mse).log10()
        })
    };
    if interior_pixels > 0 {
        for channel in &mut mean_delta {
            *channel /= interior_pixels as f64;
        }
    }
    SwatchReport {
        name: name.to_owned(),
        pixels: component.len(),
        interior_pixels,
        bbox: bbox(component, expected.width),
        mean_delta,
        max_channel_delta,
        psnr: psnr
            .map(|value| {
                if value.is_finite() {
                    format!("{value:.4}")
                } else {
                    "Infinity".to_owned()
                }
            })
            .unwrap_or_else(|| "NaN".to_owned()),
        pass: psnr.is_some_and(|value| value >= options.fail_under)
            && max_channel_delta <= options.max_channel_delta,
    }
}

fn is_interior_opaque(pixel: usize, expected: &RgbaJson, actual: &RgbaJson) -> bool {
    let x = pixel % expected.width;
    let y = pixel / expected.width;
    if x == 0 || y == 0 || x >= expected.width - 1 || y >= expected.height - 1 {
        return false;
    }
    (-1..=1).all(|dy| {
        (-1..=1).all(|dx| {
            let neighbor = ((y as isize + dy) as usize) * expected.width
                + (x as isize + dx) as usize;
            alpha(expected, neighbor) == 255 && alpha(actual, neighbor) == 255
        })
    })
}

fn alpha(image: &RgbaJson, pixel: usize) -> u8 {
    image.rgba[pixel * 4 + 3]
}

fn bbox(component: &[usize], width: usize) -> [usize; 4] {
    let mut min_x = usize::MAX;
    let mut min_y = usize::MAX;
    let mut max_x = 0;
    let mut max_y = 0;
    for &pixel in component {
        let x = pixel % width;
        let y = pixel / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    [min_x, min_y, max_x, max_y]
}
