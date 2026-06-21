#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
image = { version = "0.25.10", default-features = false, features = ["png"] }
serde_json = "1.0.150"
---

//! Convert a render-parity `.rgba.json` artifact into a PNG for visual review.

use clap::Parser;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "rgba-json-to-png",
    about = "Convert a render-parity RGBA JSON artifact into a byte-equivalent PNG"
)]
struct Options {
    /// Source render-parity `.rgba.json` artifact.
    #[arg(long)]
    input: Option<PathBuf>,
    /// Destination PNG path.
    #[arg(long)]
    png_out: Option<PathBuf>,
    /// Write and verify a tiny sample artifact under `target/`.
    #[arg(long)]
    self_test: bool,
}

#[derive(Clone, Debug)]
struct RgbaArtifact {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    if options.self_test {
        return run_self_test();
    }
    let input = options
        .input
        .as_deref()
        .ok_or("--input is required unless --self-test is set")?;
    let png_out = options
        .png_out
        .as_deref()
        .ok_or("--png-out is required unless --self-test is set")?;
    convert_rgba_json_to_png(input, png_out)
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let dir = PathBuf::from("target/rgba-json-to-png-self-test");
    fs::create_dir_all(&dir)?;
    let input = dir.join("sample.rgba.json");
    let png = dir.join("sample.png");
    fs::write(
        &input,
        r#"{"width":2,"height":1,"rgba":[255,0,0,255,0,0,255,128]}"#,
    )?;
    convert_rgba_json_to_png(&input, &png)?;
    println!(
        "rgba-json-to-png self-test: wrote {}",
        display_path(&png)
    );
    Ok(())
}

fn convert_rgba_json_to_png(input: &Path, png_out: &Path) -> Result<(), Box<dyn Error>> {
    let artifact = read_rgba_artifact(input)?;
    if let Some(parent) = png_out.parent() {
        fs::create_dir_all(parent)?;
    }
    image::save_buffer(
        png_out,
        &artifact.rgba,
        artifact.width,
        artifact.height,
        image::ColorType::Rgba8,
    )?;
    verify_png_matches_artifact(&artifact, png_out)?;
    println!(
        "wrote PNG from RGBA JSON: {} -> {} ({}x{})",
        display_path(input),
        display_path(png_out),
        artifact.width,
        artifact.height
    );
    Ok(())
}

fn read_rgba_artifact(path: &Path) -> Result<RgbaArtifact, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let width = u32_field(&value, "width", path)?;
    let height = u32_field(&value, "height", path)?;
    let expected_len = usize::try_from(width)?
        .checked_mul(usize::try_from(height)?)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("{} dimensions overflow RGBA length", display_path(path)))?;
    let rgba_values = value
        .get("rgba")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{}: rgba must be an array", display_path(path)))?;
    if rgba_values.len() != expected_len {
        return Err(format!(
            "{}: rgba length {} does not match expected length {expected_len}",
            display_path(path),
            rgba_values.len()
        )
        .into());
    }
    let rgba = rgba_values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| format!("{}: rgba[{index}] must be a u8", display_path(path)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RgbaArtifact {
        width,
        height,
        rgba,
    })
}

fn u32_field(
    value: &serde_json::Value,
    field: &str,
    source: &Path,
) -> Result<u32, Box<dyn Error>> {
    let raw = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{}: {field} must be an integer", display_path(source)))?;
    Ok(u32::try_from(raw)?)
}

fn verify_png_matches_artifact(
    artifact: &RgbaArtifact,
    png_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let image = image::ImageReader::open(png_path)?
        .with_guessed_format()?
        .decode()?
        .to_rgba8();
    if image.width() != artifact.width || image.height() != artifact.height {
        return Err(format!(
            "{} dimensions differ from source artifact: png {}x{}, artifact {}x{}",
            display_path(png_path),
            image.width(),
            image.height(),
            artifact.width,
            artifact.height
        )
        .into());
    }
    if image.as_raw().as_slice() != artifact.rgba.as_slice() {
        return Err(format!(
            "{} bytes differ from source artifact",
            display_path(png_path)
        )
        .into());
    }
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
