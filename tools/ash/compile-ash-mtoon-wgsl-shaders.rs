#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
naga = { version = "29.0.3", features = ["wgsl-in", "spv-out"] }
vrm-adapter-ash = { path = "../../crates/vrm-adapter-ash" }
---

//! Compile the source-controlled Ash MToon WGSL shader to Vulkan SPIR-V.

use clap::Parser;
use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use vrm_adapter_ash::AshMtoonWgslShaderAbi;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "compile-ash-mtoon-wgsl-shaders",
    about = "Compile the Ash MToon WGSL shader ABI to vertex and fragment SPIR-V"
)]
struct Options {
    /// Output directory for the SPIR-V artifacts named by AshMtoonWgslShaderAbi.
    #[arg(long, default_value = "target/ash-mtoon-wgsl-base-shaders")]
    out_dir: PathBuf,
    /// Print parsed entry point names and global resource bindings.
    #[arg(long)]
    print_reflection: bool,
}

#[derive(Clone, Copy, Debug)]
enum CompileStage {
    Vertex,
    Fragment,
}

impl CompileStage {
    fn naga_stage(self) -> naga::ShaderStage {
        match self {
            Self::Vertex => naga::ShaderStage::Vertex,
            Self::Fragment => naga::ShaderStage::Fragment,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Vertex => "vertex",
            Self::Fragment => "fragment",
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
    let abi = AshMtoonWgslShaderAbi::default();
    let source = read_shader_source(abi)?;
    let module = naga::front::wgsl::parse_str(&source)
        .map_err(|error| format!("failed to parse WGSL {}: {error}", abi.source_path))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|error| format!("failed to validate WGSL {}: {error}", abi.source_path))?;
    if options.print_reflection {
        print_reflection(&module);
    }

    compile_stage(
        abi,
        &module,
        &info,
        CompileStage::Vertex,
        abi.vertex_entry,
        &abi.vertex_spirv_path(&options.out_dir),
    )?;
    compile_stage(
        abi,
        &module,
        &info,
        CompileStage::Fragment,
        abi.fragment_entry,
        &abi.fragment_spirv_path(&options.out_dir),
    )?;
    Ok(())
}

fn read_shader_source(abi: AshMtoonWgslShaderAbi) -> Result<String, Box<dyn Error>> {
    let mut source = String::new();
    source.push_str(&fs::read_to_string(abi.prelude_path)?);
    source.push_str("\n\n");
    source.push_str(&fs::read_to_string(abi.source_path)?);
    Ok(source)
}

fn compile_stage(
    abi: AshMtoonWgslShaderAbi,
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: CompileStage,
    entry: &str,
    out: &Path,
) -> Result<(), Box<dyn Error>> {
    let mut options = spv::Options::default();
    if !abi.adjust_coordinate_space {
        options
            .flags
            .remove(spv::WriterFlags::ADJUST_COORDINATE_SPACE);
    }
    let words = spv::write_vec(
        module,
        info,
        &options,
        Some(&spv::PipelineOptions {
            shader_stage: stage.naga_stage(),
            entry_point: entry.to_owned(),
        }),
    )
    .map_err(|error| {
        format!(
            "failed to write Ash MToon {} SPIR-V for entry {entry}: {error}",
            stage.label()
        )
    })?;
    validate_spirv_words(&words)?;
    write_spirv(out, &words)?;
    println!(
        "compiled Ash MToon WGSL {} SPIR-V: {} -> {} (entry={}, {} words)",
        stage.label(),
        display_path(Path::new(abi.source_path)),
        display_path(out),
        entry,
        words.len()
    );
    Ok(())
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
