#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
imq = { git = "https://github.com/Sanzentyo/imq.git", rev = "0fdc5263c0c21bd6d7bc55c194e98b593bf83bff", default-features = false }
serde_json = "1.0.150"
---

//! Verify that a renderer `imqraw` artifact matches its companion RGBA JSON.

use clap::Parser;
use imq::{PixelFormat, RawImageRecord, decode_imqraw_bundle};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "verify-imqraw-rgba",
    about = "Verify that a direct imqraw artifact matches a companion RGBA JSON artifact"
)]
struct Options {
    #[arg(long)]
    imqraw: PathBuf,
    #[arg(long)]
    rgba_json: PathBuf,
    #[arg(long, default_value_t = 0)]
    index: usize,
}

#[derive(Clone, Debug)]
struct RgbaImage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    let imqraw = read_imqraw_rgba8(&options.imqraw, options.index)?;
    let rgba_json = read_rgba_json(&options.rgba_json)?;
    if imqraw.width != rgba_json.width || imqraw.height != rgba_json.height {
        return Err(format!(
            "artifact dimensions differ: imqraw {}x{}, rgba JSON {}x{}",
            imqraw.width, imqraw.height, rgba_json.width, rgba_json.height
        )
        .into());
    }
    if let Some(index) = imqraw
        .rgba
        .iter()
        .zip(&rgba_json.rgba)
        .position(|(imqraw, json)| imqraw != json)
    {
        return Err(format!(
            "artifact bytes differ at rgba[{index}]: imqraw={}, rgba JSON={}",
            imqraw.rgba[index], rgba_json.rgba[index]
        )
        .into());
    }
    println!(
        "verified imqraw matches rgba JSON: {} ({}x{})",
        display_path(&options.imqraw),
        imqraw.width,
        imqraw.height
    );
    Ok(())
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

fn read_rgba_json(path: &Path) -> Result<RgbaImage, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let value = serde_json::from_str::<serde_json::Value>(&text)?;
    let width = read_usize_field(&value, "width", path)?;
    let height = read_usize_field(&value, "height", path)?;
    let rgba_values = value
        .get("rgba")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{}: rgba must be an array", display_path(path)))?;
    let expected_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("{}: rgba length overflows", display_path(path)))?;
    if rgba_values.len() != expected_len {
        return Err(format!(
            "{}: rgba length {} does not match {expected_len}",
            display_path(path),
            rgba_values.len()
        )
        .into());
    }
    let rgba = rgba_values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let value = value.as_u64().ok_or_else(|| {
                format!("{}: rgba[{index}] must be an integer", display_path(path))
            })?;
            u8::try_from(value)
                .map_err(|_| format!("{}: rgba[{index}] must be in 0..255", display_path(path)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RgbaImage {
        width,
        height,
        rgba,
    })
}

fn read_usize_field(
    value: &serde_json::Value,
    field: &str,
    source: &Path,
) -> Result<usize, Box<dyn Error>> {
    let value = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            format!(
                "{}: {field} must be a positive integer",
                display_path(source)
            )
        })?;
    let value = usize::try_from(value)?;
    if value == 0 {
        Err(format!(
            "{}: {field} must be a positive integer",
            display_path(source)
        )
        .into())
    } else {
        Ok(value)
    }
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
