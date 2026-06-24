#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
naga = { version = "29.0.3", features = ["wgsl-in", "spv-out"] }
---

//! Compile WGSL to SPIR-V with naga for ash/Vulkan shader experiments.

use clap::{Parser, ValueEnum};
use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "compile-wgsl-to-spirv",
    about = "Compile a WGSL entry point to SPIR-V using naga"
)]
struct Options {
    /// WGSL shader source.
    #[arg(long)]
    source: PathBuf,
    /// Optional WGSL prelude prepended before `source`.
    #[arg(long)]
    prelude: Option<PathBuf>,
    /// Entry point name.
    #[arg(long, default_value = "main")]
    entry: String,
    /// Entry point stage.
    #[arg(long, value_enum)]
    stage: ShaderStageArg,
    /// Output SPIR-V path.
    #[arg(long)]
    out: PathBuf,
    /// Disable naga's SPIR-V coordinate-space adjustment.
    #[arg(long)]
    no_adjust_coordinate_space: bool,
    /// Print parsed entry point names and global resource bindings.
    #[arg(long)]
    print_reflection: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ShaderStageArg {
    Vertex,
    Fragment,
}

impl From<ShaderStageArg> for naga::ShaderStage {
    fn from(value: ShaderStageArg) -> Self {
        match value {
            ShaderStageArg::Vertex => Self::Vertex,
            ShaderStageArg::Fragment => Self::Fragment,
        }
    }
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    let source = read_shader_source(&options)?;
    let module = naga::front::wgsl::parse_str(&source)
        .map_err(|error| format!("failed to parse WGSL {}: {error}", options.source.display()))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|error| format!("failed to validate WGSL {}: {error}", options.source.display()))?;
    if options.print_reflection {
        print_reflection(&module);
    }
    let mut spv_options = spv::Options::default();
    if options.no_adjust_coordinate_space {
        spv_options
            .flags
            .remove(spv::WriterFlags::ADJUST_COORDINATE_SPACE);
    }
    let words = spv::write_vec(
        &module,
        &info,
        &spv_options,
        Some(&spv::PipelineOptions {
            shader_stage: options.stage.into(),
            entry_point: options.entry.clone(),
        }),
    )
    .map_err(|error| {
        format!(
            "failed to write SPIR-V for {}::{:?}: {error}",
            options.entry, options.stage
        )
    })?;
    validate_spirv_words(&words)?;
    write_spirv(&options.out, &words)?;
    println!(
        "compiled WGSL to SPIR-V: {} -> {} ({} words)",
        display_path(&options.source),
        display_path(&options.out),
        words.len()
    );
    Ok(())
}

fn read_shader_source(options: &Options) -> Result<String, Box<dyn Error>> {
    let mut source = String::new();
    if let Some(prelude) = &options.prelude {
        source.push_str(&fs::read_to_string(prelude)?);
        source.push_str("\n\n");
    }
    source.push_str(&fs::read_to_string(&options.source)?);
    Ok(source)
}

fn write_spirv(path: &Path, words: &[u32]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<_>>();
    fs::write(path, bytes)?;
    Ok(())
}

fn validate_spirv_words(words: &[u32]) -> Result<(), Box<dyn Error>> {
    if words.first().copied() != Some(0x0723_0203) {
        return Err("naga produced invalid SPIR-V: missing magic word".into());
    }
    if words.len() < 5 {
        return Err("naga produced invalid SPIR-V: module header is truncated".into());
    }
    Ok(())
}

fn print_reflection(module: &naga::Module) {
    println!("entry points:");
    for entry in &module.entry_points {
        println!("  {:?} {}", entry.stage, entry.name);
    }
    println!("global resource bindings:");
    for (_, global) in module.global_variables.iter() {
        if let Some(binding) = &global.binding {
            println!(
                "  {:?} group={} binding={}",
                global.space, binding.group, binding.binding
            );
        }
    }
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
