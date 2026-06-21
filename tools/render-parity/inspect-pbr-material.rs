#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
gltf = { version = "1.4.1", features = ["extensions", "import", "KHR_materials_unlit", "KHR_texture_transform"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Inspect glTF PBR material inputs for render-parity residuals.
//!
//! This intentionally does not render. It extracts source material/texture
//! conditions so a later renderer change can be tied to a specific glTF/PBR
//! input rather than tuned against a PSNR number.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "inspect-pbr-material",
    about = "Inspect glTF PBR material inputs for render parity diagnostics"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    fixture: Option<PathBuf>,
    #[arg(long = "material-name", default_value = "backpack_nm")]
    material_names: Vec<String>,
    #[arg(long)]
    all: bool,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
struct Report {
    fixture: String,
    material_name_filters: Vec<String>,
    material_count: usize,
    selected_count: usize,
    selected_materials: Vec<MaterialReport>,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialReport {
    index: usize,
    name: Option<String>,
    branch: MaterialBranch,
    primitive_count: usize,
    base_color_factor: [f32; 4],
    metallic_factor: f32,
    roughness_factor: f32,
    alpha_mode: String,
    alpha_cutoff: Option<f32>,
    double_sided: bool,
    unlit: bool,
    emissive_factor: [f32; 3],
    emissive_strength: f32,
    normal_scale: Option<f32>,
    occlusion_strength: Option<f32>,
    textures: Vec<TextureSlotReport>,
    extensions: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum MaterialBranch {
    Mtoon,
    GltfPbr,
}

#[derive(Clone, Debug, Serialize)]
struct TextureSlotReport {
    slot: String,
    texture: Option<usize>,
    tex_coord: Option<u64>,
    transform: Option<TextureTransformReport>,
    image: Option<ImageReport>,
    sampler: Option<SamplerReport>,
}

#[derive(Clone, Debug, Serialize)]
struct TextureTransformReport {
    offset: [f64; 2],
    scale: [f64; 2],
    rotation: f64,
    tex_coord: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
struct ImageReport {
    index: usize,
    name: Option<String>,
    mime_type: Option<String>,
    width: u32,
    height: u32,
    format: String,
    byte_len: usize,
}

#[derive(Clone, Debug, Serialize)]
struct SamplerReport {
    index: Option<usize>,
    mag_filter: Option<u64>,
    min_filter: Option<u64>,
    wrap_s: u64,
    wrap_t: u64,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse();
    if options.self_test {
        return self_test();
    }

    let fixture = options.fixture.as_deref().expect("required by clap");
    let report = inspect_fixture(fixture, &options.material_names, options.all)?;
    write_report(&report, &options)?;
    Ok(())
}

fn inspect_fixture(
    fixture: &Path,
    material_names: &[String],
    all: bool,
) -> Result<Report, Box<dyn Error>> {
    let (document, _buffers, images) = gltf::import(fixture)?;
    let root = serde_json::to_value(document.as_json())?;
    let materials_json = root
        .get("materials")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let primitive_counts = primitive_counts_by_material(&root);

    let selected_materials = document
        .materials()
        .filter_map(|material| {
            let index = material.index()?;
            let name = material.name().map(ToOwned::to_owned);
            let selected = all
                || name
                    .as_deref()
                    .is_some_and(|name| material_names.iter().any(|needle| name.contains(needle)));
            selected.then_some((material, index, name))
        })
        .map(|(material, index, name)| {
            let raw = materials_json.get(index);
            material_report(
                &material,
                index,
                name,
                raw,
                &root,
                &images,
                primitive_counts.get(index).copied().unwrap_or_default(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    if selected_materials.is_empty() {
        let available = document
            .materials()
            .filter_map(|material| material.name().map(ToOwned::to_owned))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "no material matched {:?}; available named materials: {}",
            material_names, available
        )
        .into());
    }

    Ok(Report {
        fixture: fixture.display().to_string(),
        material_name_filters: material_names.to_vec(),
        material_count: materials_json.len(),
        selected_count: selected_materials.len(),
        selected_materials,
    })
}

fn material_report(
    material: &gltf::Material<'_>,
    index: usize,
    name: Option<String>,
    raw: Option<&Value>,
    root: &Value,
    images: &[gltf::image::Data],
    primitive_count: usize,
) -> Result<MaterialReport, Box<dyn Error>> {
    let pbr = material.pbr_metallic_roughness();
    let extensions = extension_names(raw);
    let branch = if extensions.iter().any(|name| name == "VRMC_materials_mtoon") {
        MaterialBranch::Mtoon
    } else {
        MaterialBranch::GltfPbr
    };
    let emissive_strength = material
        .extension_value("KHR_materials_emissive_strength")
        .and_then(|value| value.get("emissiveStrength"))
        .and_then(Value::as_f64)
        .or_else(|| {
            raw
                .and_then(|raw| {
                    raw.pointer("/extensions/KHR_materials_emissive_strength/emissiveStrength")
                })
                .and_then(Value::as_f64)
        })
        .unwrap_or(1.0) as f32;
    let extensions = merge_extension_names(
        extensions,
        [
            ("KHR_materials_emissive_strength", emissive_strength != 1.0),
            ("KHR_materials_unlit", material.unlit()),
        ],
    );

    Ok(MaterialReport {
        index,
        name,
        branch,
        primitive_count,
        base_color_factor: pbr.base_color_factor(),
        metallic_factor: pbr.metallic_factor(),
        roughness_factor: pbr.roughness_factor(),
        alpha_mode: format!("{:?}", material.alpha_mode()),
        alpha_cutoff: material.alpha_cutoff(),
        double_sided: material.double_sided(),
        unlit: material.unlit(),
        emissive_factor: material.emissive_factor(),
        emissive_strength,
        normal_scale: raw
            .and_then(|raw| raw.pointer("/normalTexture/scale"))
            .and_then(Value::as_f64)
            .map(|value| value as f32),
        occlusion_strength: raw
            .and_then(|raw| raw.pointer("/occlusionTexture/strength"))
            .and_then(Value::as_f64)
            .map(|value| value as f32),
        textures: texture_slot_reports(raw, root, images),
        extensions,
    })
}

fn merge_extension_names<const N: usize>(
    mut extensions: Vec<String>,
    inferred: [(&str, bool); N],
) -> Vec<String> {
    for (extension, present) in inferred {
        if present && !extensions.iter().any(|existing| existing == extension) {
            extensions.push(extension.to_owned());
        }
    }
    extensions.sort();
    extensions
}

fn texture_slot_reports(
    raw: Option<&Value>,
    root: &Value,
    images: &[gltf::image::Data],
) -> Vec<TextureSlotReport> {
    [
        ("baseColorTexture", "/pbrMetallicRoughness/baseColorTexture"),
        (
            "metallicRoughnessTexture",
            "/pbrMetallicRoughness/metallicRoughnessTexture",
        ),
        ("normalTexture", "/normalTexture"),
        ("occlusionTexture", "/occlusionTexture"),
        ("emissiveTexture", "/emissiveTexture"),
    ]
    .into_iter()
    .map(|(slot, pointer)| {
        texture_slot_report(slot, raw.and_then(|raw| raw.pointer(pointer)), root, images)
    })
    .collect()
}

fn texture_slot_report(
    slot: &str,
    texture_info: Option<&Value>,
    root: &Value,
    images: &[gltf::image::Data],
) -> TextureSlotReport {
    let texture = texture_info
        .and_then(|raw| {
            raw.get("index")
        })
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok());
    let tex_coord = texture_info
        .and_then(|info| info.get("texCoord"))
        .and_then(Value::as_u64);
    let transform = texture_info
        .and_then(|info| info.pointer("/extensions/KHR_texture_transform"))
        .map(texture_transform_report);
    let texture_json = texture.and_then(|texture| {
        root.get("textures")
            .and_then(Value::as_array)
            .and_then(|textures| textures.get(texture))
    });
    let image = texture_json
        .and_then(|texture| texture.get("source"))
        .and_then(Value::as_u64)
        .and_then(|source| usize::try_from(source).ok())
        .and_then(|source| image_report(root, images, source));
    let sampler = texture_json
        .and_then(|texture| texture.get("sampler"))
        .and_then(Value::as_u64)
        .and_then(|sampler| usize::try_from(sampler).ok())
        .map(|sampler| sampler_report(root, Some(sampler)))
        .or_else(|| texture.map(|_| sampler_report(root, None)));

    TextureSlotReport {
        slot: slot.to_owned(),
        texture,
        tex_coord,
        transform,
        image,
        sampler,
    }
}

fn texture_transform_report(value: &Value) -> TextureTransformReport {
    TextureTransformReport {
        offset: vec2(value.get("offset"), [0.0, 0.0]),
        scale: vec2(value.get("scale"), [1.0, 1.0]),
        rotation: value.get("rotation").and_then(Value::as_f64).unwrap_or(0.0),
        tex_coord: value.get("texCoord").and_then(Value::as_u64),
    }
}

fn image_report(root: &Value, images: &[gltf::image::Data], index: usize) -> Option<ImageReport> {
    let data = images.get(index)?;
    let raw = root
        .get("images")
        .and_then(Value::as_array)
        .and_then(|images| images.get(index));
    Some(ImageReport {
        index,
        name: raw
            .and_then(|raw| raw.get("name"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        mime_type: raw
            .and_then(|raw| raw.get("mimeType"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        width: data.width,
        height: data.height,
        format: format!("{:?}", data.format),
        byte_len: data.pixels.len(),
    })
}

fn sampler_report(root: &Value, index: Option<usize>) -> SamplerReport {
    let raw = index.and_then(|index| {
        root.get("samplers")
            .and_then(Value::as_array)
            .and_then(|samplers| samplers.get(index))
    });
    SamplerReport {
        index,
        mag_filter: raw
            .and_then(|raw| raw.get("magFilter"))
            .and_then(Value::as_u64),
        min_filter: raw
            .and_then(|raw| raw.get("minFilter"))
            .and_then(Value::as_u64),
        wrap_s: raw
            .and_then(|raw| raw.get("wrapS"))
            .and_then(Value::as_u64)
            .unwrap_or(10497),
        wrap_t: raw
            .and_then(|raw| raw.get("wrapT"))
            .and_then(Value::as_u64)
            .unwrap_or(10497),
    }
}

fn primitive_counts_by_material(root: &Value) -> Vec<usize> {
    let material_count = root
        .get("materials")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mut counts = vec![0; material_count];
    for primitive in root
        .get("meshes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|mesh| mesh.get("primitives").and_then(Value::as_array))
        .flatten()
    {
        let Some(material) = primitive
            .get("material")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
        else {
            continue;
        };
        if let Some(count) = counts.get_mut(material) {
            *count += 1;
        }
    }
    counts
}

fn extension_names(raw: Option<&Value>) -> Vec<String> {
    raw.and_then(|raw| raw.get("extensions"))
        .and_then(Value::as_object)
        .map(|extensions| extensions.keys().cloned().collect())
        .unwrap_or_default()
}

fn vec2(value: Option<&Value>, default: [f64; 2]) -> [f64; 2] {
    let Some(array) = value.and_then(Value::as_array) else {
        return default;
    };
    [
        array.first().and_then(Value::as_f64).unwrap_or(default[0]),
        array.get(1).and_then(Value::as_f64).unwrap_or(default[1]),
    ]
}

fn write_report(report: &Report, options: &Options) -> Result<(), Box<dyn Error>> {
    if let Some(path) = &options.json_out {
        write_file(path, &serde_json::to_string_pretty(report)?)?;
    }
    let markdown = report_markdown(report);
    if let Some(path) = &options.markdown_out {
        write_file(path, &markdown)?;
    }
    if options.json_out.is_none() && options.markdown_out.is_none() {
        println!("{markdown}");
    }
    Ok(())
}

fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn report_markdown(report: &Report) -> String {
    let mut out = String::new();
    out.push_str("# PBR Material Inspection\n\n");
    out.push_str(&format!("- Fixture: `{}`\n", report.fixture));
    out.push_str(&format!(
        "- Material filters: `{}`\n",
        report.material_name_filters.join("`, `")
    ));
    out.push_str(&format!(
        "- Selected/material total: `{}/{}`\n\n",
        report.selected_count, report.material_count
    ));

    for material in &report.selected_materials {
        out.push_str(&format!(
            "## Material {} `{}`\n\n",
            material.index,
            material.name.as_deref().unwrap_or("<unnamed>")
        ));
        out.push_str(&format!(
            "- Branch: `{:?}`\n- Primitive count: `{}`\n- Base color factor: `{:?}`\n- Metallic/Roughness: `{:.4}` / `{:.4}`\n- Alpha: `{}` cutoff `{:?}`, double-sided `{}`\n- Emissive factor/strength: `{:?}` / `{:.4}`\n- Normal scale: `{:?}`\n- Occlusion strength: `{:?}`\n- Extensions: `{}`\n\n",
            material.branch,
            material.primitive_count,
            material.base_color_factor,
            material.metallic_factor,
            material.roughness_factor,
            material.alpha_mode,
            material.alpha_cutoff,
            material.double_sided,
            material.emissive_factor,
            material.emissive_strength,
            material.normal_scale,
            material.occlusion_strength,
            material.extensions.join("`, `")
        ));
        out.push_str("| Slot | Texture | Image | Size | Sampler | Transform |\n");
        out.push_str("| --- | ---: | --- | --- | --- | --- |\n");
        for texture in &material.textures {
            let image_name = texture
                .image
                .as_ref()
                .and_then(|image| image.name.as_deref())
                .unwrap_or("-");
            let image_size = texture
                .image
                .as_ref()
                .map(|image| format!("{}x{} {}", image.width, image.height, image.format))
                .unwrap_or_else(|| "-".to_owned());
            let sampler = texture
                .sampler
                .as_ref()
                .map(|sampler| {
                    format!(
                        "idx={:?} mag={:?} min={:?} wrap={}/{}",
                        sampler.index,
                        sampler.mag_filter,
                        sampler.min_filter,
                        sampler.wrap_s,
                        sampler.wrap_t
                    )
                })
                .unwrap_or_else(|| "-".to_owned());
            let transform = texture
                .transform
                .as_ref()
                .map(|transform| {
                    format!(
                        "offset={:?} scale={:?} rot={:.4} texCoord={:?}",
                        transform.offset, transform.scale, transform.rotation, transform.tex_coord
                    )
                })
                .unwrap_or_else(|| "-".to_owned());
            out.push_str(&format!(
                "| `{}` | {:?} | `{}` | `{}` | `{}` | `{}` |\n",
                texture.slot, texture.texture, image_name, image_size, sampler, transform
            ));
        }
        out.push('\n');
    }

    out
}

fn self_test() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from("target/inspect-pbr-material-self-test.gltf");
    let sample = serde_json::json!({
        "asset": { "version": "2.0" },
        "extensionsUsed": ["KHR_materials_emissive_strength"],
        "materials": [{
            "name": "backpack_nm_test",
            "pbrMetallicRoughness": {
                "baseColorFactor": [0.5, 0.6, 0.7, 1.0],
                "metallicFactor": 0.2,
                "roughnessFactor": 0.8
            },
            "emissiveFactor": [0.1, 0.2, 0.3],
            "extensions": {
                "KHR_materials_emissive_strength": { "emissiveStrength": 2.0 }
            }
        }],
        "meshes": []
    });
    write_file(&out, &serde_json::to_string_pretty(&sample)?)?;
    let report = inspect_fixture(&out, &[String::from("backpack_nm")], false)?;
    let material = report
        .selected_materials
        .first()
        .ok_or("self-test selected material missing")?;
    assert_eq!(material.primitive_count, 0);
    assert_eq!(material.emissive_strength, 2.0);
    assert!(matches!(material.branch, MaterialBranch::GltfPbr));
    Ok(())
}
