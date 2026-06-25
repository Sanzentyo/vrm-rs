#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
---

//! Compile source-controlled ash GLSL utility shaders to local SPIR-V artifacts.

use clap::Parser;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "compile-ash-glsl-shaders",
    about = "Compile source ash GLSL utility shaders to local SPIR-V files"
)]
struct Options {
    #[arg(long, default_value = "crates/vrm-adapter-ash/shaders/windowed_simple.vert.glsl")]
    vertex: PathBuf,
    #[arg(long, default_value = "crates/vrm-adapter-ash/shaders/windowed_simple.frag.glsl")]
    fragment: PathBuf,
    #[arg(long, default_value = "target/ash-windowed-simple-shaders")]
    out_dir: PathBuf,
    #[arg(long, default_value = "glslangValidator")]
    glslang: PathBuf,
}

#[derive(Clone, Debug)]
struct CompiledShaders {
    vertex_spv: PathBuf,
    fragment_spv: PathBuf,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    validate_source(&options.vertex)?;
    validate_source(&options.fragment)?;
    fs::create_dir_all(&options.out_dir)?;
    let shaders = CompiledShaders {
        vertex_spv: options.out_dir.join("windowed_simple.vert.spv"),
        fragment_spv: options.out_dir.join("windowed_simple.frag.spv"),
    };
    compile_shader(
        &options.glslang,
        "vert",
        &options.vertex,
        &shaders.vertex_spv,
    )?;
    compile_shader(
        &options.glslang,
        "frag",
        &options.fragment,
        &shaders.fragment_spv,
    )?;
    validate_spirv(&shaders.vertex_spv)?;
    validate_spirv(&shaders.fragment_spv)?;
    println!(
        "compiled ash GLSL utility shaders: {} {}",
        display_path(&shaders.vertex_spv),
        display_path(&shaders.fragment_spv)
    );
    Ok(())
}

fn compile_shader(
    glslang: &Path,
    stage: &str,
    source: &Path,
    output: &Path,
) -> Result<(), Box<dyn Error>> {
    let status = Command::new(glslang)
        .arg("-V")
        .arg("-S")
        .arg(stage)
        .arg(source)
        .arg("-o")
        .arg(output)
        .status()
        .map_err(|error| format!("failed to spawn {}: {error}", display_path(glslang)))?;
    if !status.success() {
        return Err(format!(
            "glslangValidator failed for {} with status {status}",
            display_path(source)
        )
        .into());
    }
    Ok(())
}

fn validate_source(path: &Path) -> Result<(), Box<dyn Error>> {
    if !path.is_file() {
        return Err(format!("shader source does not exist: {}", display_path(path)).into());
    }
    Ok(())
}

fn validate_spirv(path: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    if bytes.len() % std::mem::size_of::<u32>() != 0 {
        return Err(format!(
            "{} is not valid SPIR-V: byte length is not a multiple of 4",
            display_path(path)
        )
        .into());
    }
    let magic = bytes
        .get(0..4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
    if magic != Some(0x0723_0203) {
        return Err(format!("{} is not valid SPIR-V", display_path(path)).into());
    }
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
