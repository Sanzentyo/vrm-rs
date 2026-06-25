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
use naga::{AddressSpace, ImageClass, ImageDimension, ScalarKind, TypeInner};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use vrm_adapter_ash::{
    AshMtoonWgslShaderAbi, AshWgslResourceKind, ash_mtoon_wgsl_resource_bindings,
};

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
    validate_shader_abi(abi, &module)?;
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReflectedResource {
    name: String,
    kind: AshWgslResourceKind,
}

fn validate_shader_abi(
    abi: AshMtoonWgslShaderAbi,
    module: &naga::Module,
) -> Result<(), Box<dyn Error>> {
    validate_entry_point(module, naga::ShaderStage::Vertex, abi.vertex_entry)?;
    validate_entry_point(module, naga::ShaderStage::Fragment, abi.fragment_entry)?;

    let reflected = reflect_resource_bindings(module)?;
    let expected = ash_mtoon_wgsl_resource_bindings();
    let expected_keys = expected
        .iter()
        .map(|binding| ((binding.group, binding.binding), binding))
        .collect::<HashMap<_, _>>();

    for binding in expected {
        let key = (binding.group, binding.binding);
        let Some(actual) = reflected.get(&key) else {
            return Err(format!(
                "WGSL ABI mismatch: missing resource {} at group {} binding {}",
                binding.name, binding.group, binding.binding
            )
            .into());
        };
        if actual.name != binding.name {
            return Err(format!(
                "WGSL ABI mismatch at group {} binding {}: expected name {}, found {}",
                binding.group, binding.binding, binding.name, actual.name
            )
            .into());
        }
        if actual.kind != binding.kind {
            return Err(format!(
                "WGSL ABI mismatch for {} at group {} binding {}: expected {:?}, found {:?}",
                binding.name, binding.group, binding.binding, binding.kind, actual.kind
            )
            .into());
        }
    }

    let unexpected = reflected
        .iter()
        .filter(|(key, _)| !expected_keys.contains_key(key))
        .map(|((group, binding), resource)| {
            format!(
                "{} at group {group} binding {binding} ({:?})",
                resource.name, resource.kind
            )
        })
        .collect::<Vec<_>>();
    if !unexpected.is_empty() {
        return Err(format!(
            "WGSL ABI mismatch: unexpected bound resources: {}",
            unexpected.join(", ")
        )
        .into());
    }

    println!(
        "validated Ash MToon WGSL ABI: {} resource bindings, entries {}/{}",
        expected.len(),
        abi.vertex_entry,
        abi.fragment_entry
    );
    Ok(())
}

fn validate_entry_point(
    module: &naga::Module,
    stage: naga::ShaderStage,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    let matches = module
        .entry_points
        .iter()
        .filter(|entry| entry.stage == stage && entry.name == name)
        .count();
    match matches {
        1 => Ok(()),
        0 => Err(format!("WGSL ABI mismatch: missing {stage:?} entry point {name}").into()),
        _ => Err(format!("WGSL ABI mismatch: duplicate {stage:?} entry point {name}").into()),
    }
}

fn reflect_resource_bindings(
    module: &naga::Module,
) -> Result<HashMap<(u32, u32), ReflectedResource>, Box<dyn Error>> {
    let mut reflected = HashMap::new();
    let mut names = HashSet::new();
    for (_, global) in module.global_variables.iter() {
        let Some(binding) = &global.binding else {
            continue;
        };
        let name = global.name.clone().ok_or_else(|| {
            format!(
                "WGSL ABI mismatch: unnamed resource at group {} binding {}",
                binding.group, binding.binding
            )
        })?;
        if !names.insert(name.clone()) {
            return Err(format!("WGSL ABI mismatch: duplicate resource name {name}").into());
        }
        let key = (binding.group, binding.binding);
        let kind = reflected_resource_kind(module, global).map_err(|error| {
            format!(
                "WGSL ABI mismatch for {name} at group {} binding {}: {error}",
                binding.group, binding.binding
            )
        })?;
        if let Some(previous) = reflected.insert(key, ReflectedResource { name, kind }) {
            return Err(format!(
                "WGSL ABI mismatch: duplicate group {} binding {} for {} and {}",
                binding.group,
                binding.binding,
                previous.name,
                reflected
                    .get(&key)
                    .map(|resource| resource.name.as_str())
                    .unwrap_or("<unknown>")
            )
            .into());
        }
    }
    Ok(reflected)
}

fn reflected_resource_kind(
    module: &naga::Module,
    global: &naga::GlobalVariable,
) -> Result<AshWgslResourceKind, String> {
    match global.space {
        AddressSpace::Uniform => Ok(AshWgslResourceKind::UniformBuffer),
        AddressSpace::Storage { .. } => Ok(AshWgslResourceKind::StorageBuffer),
        AddressSpace::Handle => match &module.types[global.ty].inner {
            TypeInner::Image {
                dim: ImageDimension::D2,
                arrayed: false,
                class:
                    ImageClass::Sampled {
                        kind: ScalarKind::Float,
                        multi: false,
                    },
            } => Ok(AshWgslResourceKind::SampledImage),
            TypeInner::Sampler { comparison: false } => Ok(AshWgslResourceKind::Sampler),
            other => Err(format!("unsupported handle type {other:?}")),
        },
        other => Err(format!("unsupported resource address space {other:?}")),
    }
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
