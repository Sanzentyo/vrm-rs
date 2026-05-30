#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

use clap::Parser;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-transparent-alpha-modes-fixture",
    about = "Generate a source-like VRM1 glTF fixture for MToon alpha mode and cutoff parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/transparent-alpha-modes.vrm.gltf"
    )]
    out: PathBuf,
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
    let materials = materials();
    let primitive_count = materials.len();
    let buffer = mesh_buffer(primitive_count);

    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs transparent alpha mode render parity generator"
        },
        "extensionsUsed": [
            "VRMC_vrm",
            "VRMC_materials_mtoon"
        ],
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": nodes(),
        "buffers": [{
            "uri": format!("data:application/octet-stream;base64,{}", base64(&buffer)),
            "byteLength": buffer.len()
        }],
        "bufferViews": buffer_views(primitive_count),
        "accessors": accessors(primitive_count),
        "materials": materials,
        "meshes": [{
            "name": "transparent-alpha-mode-swatches",
            "primitives": (0..primitive_count)
                .map(|index| json!({
                    "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
                    "indices": 3 + index,
                    "material": index
                }))
                .collect::<Vec<_>>()
        }],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Transparent Alpha Modes Fixture",
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

fn buffer_views(primitive_count: usize) -> Vec<Value> {
    let vertex_count = primitive_count * 4;
    let position_len = vertex_count * 3 * 4;
    let normal_len = vertex_count * 3 * 4;
    let uv_len = vertex_count * 2 * 4;
    let index_len = 6 * 2;
    let mut offset = 0;
    let mut views = Vec::new();
    for (byte_length, target) in [(position_len, 34962), (normal_len, 34962), (uv_len, 34962)] {
        views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": byte_length,
            "target": target
        }));
        offset += byte_length;
    }
    views.extend((0..primitive_count).map(|_| {
        let view = json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": index_len,
            "target": 34963
        });
        offset += index_len;
        view
    }));
    views
}

fn accessors(primitive_count: usize) -> Vec<Value> {
    let vertex_count = primitive_count * 4;
    let mut accessors = vec![
        json!({
            "bufferView": 0,
            "componentType": 5126,
            "count": vertex_count,
            "type": "VEC3",
            "min": [-0.88, 0.45, 0.0],
            "max": [0.88, 1.55, 0.0]
        }),
        json!({
            "bufferView": 1,
            "componentType": 5126,
            "count": vertex_count,
            "type": "VEC3",
            "min": [0.0, 0.0, 1.0],
            "max": [0.0, 0.0, 1.0]
        }),
        json!({
            "bufferView": 2,
            "componentType": 5126,
            "count": vertex_count,
            "type": "VEC2",
            "min": [0.0, 0.0],
            "max": [1.0, 1.0]
        }),
    ];
    accessors.extend((0..primitive_count).map(|index| {
        json!({
            "bufferView": 3 + index,
            "componentType": 5123,
            "count": 6,
            "type": "SCALAR"
        })
    }));
    accessors
}

fn materials() -> Vec<Value> {
    vec![
        mtoon_material("opaque-alpha-forced", "OPAQUE", None, [0.04, 0.65, 1.0, 0.25]),
        mtoon_material(
            "mask-custom-cutoff-pass",
            "MASK",
            Some(0.25),
            [0.05, 0.95, 0.24, 0.30],
        ),
        mtoon_material(
            "mask-custom-cutoff-fail",
            "MASK",
            Some(0.70),
            [1.0, 0.18, 0.02, 0.60],
        ),
        mtoon_material(
            "blend-cutoff-ignored",
            "BLEND",
            Some(0.90),
            [0.95, 0.04, 0.85, 0.40],
        ),
    ]
}

fn mtoon_material(name: &str, alpha_mode: &str, alpha_cutoff: Option<f32>, base_color: [f32; 4]) -> Value {
    let mut material = json!({
        "name": name,
        "alphaMode": alpha_mode,
        "doubleSided": true,
        "pbrMetallicRoughness": {
            "baseColorFactor": base_color,
            "metallicFactor": 0.0,
            "roughnessFactor": 1.0
        },
        "extensions": {
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "shadeColorFactor": [base_color[0] * 0.72, base_color[1] * 0.72, base_color[2] * 0.72],
                "shadingShiftFactor": 1.4,
                "shadingToonyFactor": 0.95,
                "giEqualizationFactor": 0.9,
                "outlineWidthMode": "none"
            }
        }
    });
    if let Some(cutoff) = alpha_cutoff {
        material["alphaCutoff"] = json!(cutoff);
    }
    material
}

fn mesh_buffer(primitive_count: usize) -> Vec<u8> {
    let quads = [
        (-0.88f32, -0.50f32, 0.45f32, 1.55f32),
        (-0.42, -0.04, 0.45, 1.55),
        (0.04, 0.42, 0.45, 1.55),
        (0.50, 0.88, 0.45, 1.55),
    ];
    let positions = quads
        .iter()
        .take(primitive_count)
        .flat_map(|(left, right, bottom, top)| {
            [
                *left, *bottom, 0.0, *right, *bottom, 0.0, *right, *top, 0.0, *left, *top, 0.0,
            ]
        })
        .collect::<Vec<_>>();
    let normals = (0..primitive_count)
        .flat_map(|_| [0.0f32, 0.0, 1.0].repeat(4))
        .collect::<Vec<_>>();
    let uvs = (0..primitive_count)
        .flat_map(|_| [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])
        .collect::<Vec<_>>();

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    for primitive in 0..primitive_count as u16 {
        let base = primitive * 4;
        let indices = [base, base + 1, base + 2, base, base + 2, base + 3];
        bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
    }
    bytes
}

fn nodes() -> Vec<Value> {
    let mut nodes = (0..15)
        .map(|index| json!({ "name": format!("node_{index}") }))
        .collect::<Vec<_>>();
    nodes[0]["mesh"] = json!(0);
    nodes[0]["children"] = json!([1, 3, 6, 9, 12]);
    nodes
}

fn human_bones() -> Map<String, Value> {
    [
        ("hips", 0),
        ("spine", 1),
        ("head", 2),
        ("leftUpperLeg", 3),
        ("leftLowerLeg", 4),
        ("leftFoot", 5),
        ("rightUpperLeg", 6),
        ("rightLowerLeg", 7),
        ("rightFoot", 8),
        ("leftUpperArm", 9),
        ("leftLowerArm", 10),
        ("leftHand", 11),
        ("rightUpperArm", 12),
        ("rightLowerArm", 13),
        ("rightHand", 14),
    ]
    .into_iter()
    .map(|(name, node)| (name.to_owned(), json!({ "node": node })))
    .collect()
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
