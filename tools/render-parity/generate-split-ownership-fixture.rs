#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
png = "0.18.1"
serde_json = "1.0.150"
---

use clap::Parser;
use png::{BitDepth, ColorType, Encoder};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-split-ownership-fixture",
    about = "Generate a source-like VRM1 glTF fixture for same-material split mesh ownership parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/split-ownership.vrm.gltf"
    )]
    out: PathBuf,
}

#[derive(Clone, Debug)]
struct MeshDef {
    name: &'static str,
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
}

#[derive(Clone, Debug)]
struct FixtureData {
    bytes: Vec<u8>,
    buffer_views: Vec<Value>,
    accessors: Vec<Value>,
    meshes: Vec<Value>,
    image_view: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse();
    if let Some(parent) = options.out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&options.out, format!("{}\n", fixture_json()))?;
    println!("{}", options.out.display());
    Ok(())
}

fn fixture_json() -> String {
    let fixture = fixture_data();
    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs split ownership render parity generator"
        },
        "extensionsUsed": [
            "VRMC_vrm",
            "VRMC_materials_mtoon"
        ],
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": nodes(),
        "buffers": [{
            "uri": format!("data:application/octet-stream;base64,{}", base64(&fixture.bytes)),
            "byteLength": fixture.bytes.len()
        }],
        "samplers": [{
            "magFilter": 9729,
            "minFilter": 9729,
            "wrapS": 10497,
            "wrapT": 10497
        }],
        "images": [{
            "mimeType": "image/png",
            "bufferView": fixture.image_view
        }],
        "textures": [{
            "sampler": 0,
            "source": 0
        }],
        "bufferViews": fixture.buffer_views,
        "accessors": fixture.accessors,
        "materials": [mtoon_material()],
        "meshes": fixture.meshes,
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Split Ownership Fixture",
                    "authors": ["vrm-rs"],
                    "licenseUrl": "https://vrm.dev/licenses/1.0/",
                    "otherLicenseUrl": "https://github.com/Sanzentyo/vrm-rs"
                },
                "humanoid": { "humanBones": human_bones() }
            }
        }
    }))
    .expect("fixture JSON should serialize")
}

fn nodes() -> Vec<Value> {
    let mut nodes = (0..17)
        .map(|index| json!({ "name": format!("node_{index}") }))
        .collect::<Vec<_>>();
    nodes[0]["name"] = json!("split_root");
    nodes[0]["children"] = json!([1, 2, 3, 5, 8, 11, 14]);
    nodes[1]["name"] = json!("wear_4_node");
    nodes[1]["mesh"] = json!(0);
    nodes[2]["name"] = json!("wear_node");
    nodes[2]["mesh"] = json!(1);
    nodes
}

fn human_bones() -> Map<String, Value> {
    [
        ("hips", 0),
        ("spine", 3),
        ("head", 4),
        ("leftUpperLeg", 5),
        ("leftLowerLeg", 6),
        ("leftFoot", 7),
        ("rightUpperLeg", 8),
        ("rightLowerLeg", 9),
        ("rightFoot", 10),
        ("leftUpperArm", 11),
        ("leftLowerArm", 12),
        ("leftHand", 13),
        ("rightUpperArm", 14),
        ("rightLowerArm", 15),
        ("rightHand", 16),
    ]
    .into_iter()
    .map(|(name, node)| (name.to_owned(), json!({ "node": node })))
    .collect()
}

fn fixture_data() -> FixtureData {
    let mut bytes = Vec::new();
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let meshes = mesh_defs()
        .into_iter()
        .map(|mesh_def| {
            let position_accessor = push_vec3_accessor(
                &mut bytes,
                &mut buffer_views,
                &mut accessors,
                &mesh_def.positions,
            );
            let normals = vec![[0.0, 0.0, 1.0]; mesh_def.positions.len()];
            let normal_accessor =
                push_vec3_accessor(&mut bytes, &mut buffer_views, &mut accessors, &normals);
            let uv_accessor =
                push_vec2_accessor(&mut bytes, &mut buffer_views, &mut accessors, &mesh_def.uvs);
            json!({
                "name": mesh_def.name,
                "primitives": [{
                    "attributes": {
                        "POSITION": position_accessor,
                        "NORMAL": normal_accessor,
                        "TEXCOORD_0": uv_accessor
                    },
                    "material": 0
                }]
            })
        })
        .collect::<Vec<_>>();

    let texture = ownership_texture_png();
    let image_view = buffer_views.len();
    buffer_views.push(json!({
        "buffer": 0,
        "byteOffset": bytes.len(),
        "byteLength": texture.len()
    }));
    bytes.extend(texture);

    FixtureData {
        bytes,
        buffer_views,
        accessors,
        meshes,
        image_view,
    }
}

fn mesh_defs() -> Vec<MeshDef> {
    vec![
        MeshDef {
            name: "wear_4",
            positions: vec![
                [-0.72, 0.55, 0.0],
                [0.04, 0.55, 0.0],
                [0.04, 1.45, 0.0],
                [-0.72, 0.55, 0.0],
                [0.04, 1.45, 0.0],
                [-0.72, 1.45, 0.0],
            ],
            uvs: vec![
                [0.02, 0.02],
                [0.48, 0.02],
                [0.48, 0.98],
                [0.02, 0.02],
                [0.48, 0.98],
                [0.02, 0.98],
            ],
        },
        MeshDef {
            name: "wear",
            positions: vec![
                [-0.04, 0.55, 0.0],
                [0.72, 0.55, 0.0],
                [0.72, 1.45, 0.0],
                [-0.04, 0.55, 0.0],
                [0.72, 1.45, 0.0],
                [-0.04, 1.45, 0.0],
            ],
            uvs: vec![
                [0.52, 0.02],
                [0.98, 0.02],
                [0.98, 0.98],
                [0.52, 0.02],
                [0.98, 0.98],
                [0.52, 0.98],
            ],
        },
    ]
}

fn push_vec3_accessor(
    bytes: &mut Vec<u8>,
    buffer_views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
    values: &[[f32; 3]],
) -> usize {
    let flat = values
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect::<Vec<_>>();
    let (min, max) = min_max_vec3(values);
    push_accessor(
        bytes,
        buffer_views,
        accessors,
        &flat,
        values.len(),
        "VEC3",
        min,
        max,
    )
}

fn push_vec2_accessor(
    bytes: &mut Vec<u8>,
    buffer_views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
    values: &[[f32; 2]],
) -> usize {
    let flat = values
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect::<Vec<_>>();
    let (min, max) = min_max_vec2(values);
    push_accessor(
        bytes,
        buffer_views,
        accessors,
        &flat,
        values.len(),
        "VEC2",
        min,
        max,
    )
}

fn push_accessor(
    bytes: &mut Vec<u8>,
    buffer_views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
    values: &[f32],
    count: usize,
    accessor_type: &str,
    min: Value,
    max: Value,
) -> usize {
    let byte_offset = bytes.len();
    let byte_length = std::mem::size_of_val(values);
    bytes.extend(values.iter().copied().flat_map(f32::to_le_bytes));
    let buffer_view = buffer_views.len();
    buffer_views.push(json!({
        "buffer": 0,
        "byteOffset": byte_offset,
        "byteLength": byte_length,
        "target": 34962
    }));
    let accessor = accessors.len();
    accessors.push(json!({
        "bufferView": buffer_view,
        "componentType": 5126,
        "count": count,
        "type": accessor_type,
        "min": min,
        "max": max
    }));
    accessor
}

fn min_max_vec3(values: &[[f32; 3]]) -> (Value, Value) {
    let min = (0..3)
        .map(|component| {
            values
                .iter()
                .map(|value| value[component])
                .fold(f32::INFINITY, f32::min)
        })
        .collect::<Vec<_>>();
    let max = (0..3)
        .map(|component| {
            values
                .iter()
                .map(|value| value[component])
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect::<Vec<_>>();
    (json!(min), json!(max))
}

fn min_max_vec2(values: &[[f32; 2]]) -> (Value, Value) {
    let min = (0..2)
        .map(|component| {
            values
                .iter()
                .map(|value| value[component])
                .fold(f32::INFINITY, f32::min)
        })
        .collect::<Vec<_>>();
    let max = (0..2)
        .map(|component| {
            values
                .iter()
                .map(|value| value[component])
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect::<Vec<_>>();
    (json!(min), json!(max))
}

fn mtoon_material() -> Value {
    json!({
        "name": "huku_bake",
        "alphaMode": "OPAQUE",
        "doubleSided": false,
        "pbrMetallicRoughness": {
            "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
            "baseColorTexture": { "index": 0 },
            "metallicFactor": 0.0,
            "roughnessFactor": 1.0
        },
        "extensions": {
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "shadeColorFactor": [1.0, 1.0, 1.0],
                "shadingShiftFactor": 0.0,
                "shadingToonyFactor": 1.0,
                "giEqualizationFactor": 0.0,
                "outlineWidthMode": "none",
                "outlineWidthFactor": 0.0,
                "outlineColorFactor": [0.0, 0.0, 0.0],
                "outlineLightingMixFactor": 0.0
            }
        }
    })
}

fn ownership_texture_png() -> Vec<u8> {
    let rgba = (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                let left = x < 4;
                let checker = (x + y) % 2 == 0;
                let color = match (left, checker) {
                    (true, true) => [255, 48, 40],
                    (true, false) => [255, 220, 40],
                    (false, true) => [40, 120, 255],
                    (false, false) => [40, 240, 170],
                };
                color.into_iter().chain([255])
            })
        })
        .collect::<Vec<_>>();
    let mut bytes = Vec::new();
    {
        let mut cursor = Cursor::new(&mut bytes);
        let mut encoder = Encoder::new(&mut cursor, 8, 8);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .expect("generated PNG header should be valid");
        writer
            .write_image_data(&rgba)
            .expect("generated PNG payload should be valid");
    }
    bytes
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        let value = ((first as u32) << 16) | ((second as u32) << 8) | third as u32;
        out.push(TABLE[((value >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((value >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(value & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}
