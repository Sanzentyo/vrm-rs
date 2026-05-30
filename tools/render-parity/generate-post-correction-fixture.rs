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
    name = "generate-post-correction-fixture",
    about = "Generate a source-like VRM1 glTF fixture for MToon post-correction parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/mtoon-post-correction.vrm.gltf"
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
    let buffer = mesh_buffer();
    let material_count = materials().len();
    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs MToon post-correction render parity generator"
        },
        "extensionsUsed": [
            "VRMC_vrm",
            "VRMC_materials_mtoon",
            "KHR_materials_emissive_strength"
        ],
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": nodes(),
        "buffers": [{
            "uri": format!("data:application/octet-stream;base64,{}", base64(&buffer)),
            "byteLength": buffer.len()
        }],
        "bufferViews": buffer_views(material_count),
        "accessors": accessors(material_count),
        "materials": materials(),
        "meshes": [{
            "name": "mtoon-post-correction-quads",
            "primitives": (0..material_count)
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
                    "name": "vrm-rs MToon Post Correction Fixture",
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

fn materials() -> Vec<Value> {
    vec![
        emissive_material(
            "srgb-mid-emissive",
            [0.21404114, 0.5, 0.75],
            1.0,
            "OPAQUE",
            1.0,
        ),
        emissive_material(
            "overbright-clamp-emissive",
            [1.35, 0.24, 0.02],
            1.0,
            "OPAQUE",
            1.0,
        ),
        emissive_material(
            "strength-clamp-emissive",
            [0.24, 0.34, 0.48],
            3.0,
            "OPAQUE",
            1.0,
        ),
        emissive_material(
            "transparent-post-correction",
            [0.16, 1.25, 0.32],
            1.0,
            "BLEND",
            0.5,
        ),
    ]
}

fn emissive_material(
    name: &str,
    emissive: [f32; 3],
    emissive_strength: f32,
    alpha_mode: &str,
    alpha: f32,
) -> Value {
    json!({
        "name": name,
        "alphaMode": alpha_mode,
        "doubleSided": true,
        "pbrMetallicRoughness": {
            "baseColorFactor": [0.0, 0.0, 0.0, alpha],
            "metallicFactor": 0.0,
            "roughnessFactor": 1.0
        },
        "emissiveFactor": emissive,
        "extensions": {
            "KHR_materials_emissive_strength": {
                "emissiveStrength": emissive_strength
            },
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "shadeColorFactor": [0.0, 0.0, 0.0],
                "shadingShiftFactor": -1.5,
                "shadingToonyFactor": 0.9,
                "giEqualizationFactor": 0.0,
                "outlineWidthMode": "none"
            }
        }
    })
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
    for _ in 0..primitive_count {
        views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": index_len,
            "target": 34963
        }));
        offset += index_len;
    }
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
            "min": [-0.78, 0.43, 0.0],
            "max": [0.78, 1.57, 0.0]
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

fn mesh_buffer() -> Vec<u8> {
    let quads = [
        (-0.78f32, -0.43f32, 0.43f32, 0.88f32),
        (-0.38, -0.03, 0.43, 0.88),
        (0.03, 0.38, 0.43, 0.88),
        (0.43, 0.78, 0.43, 0.88),
    ];
    let positions = quads
        .iter()
        .flat_map(|(left, right, bottom, top)| {
            [
                *left, *bottom, 0.0, *right, *bottom, 0.0, *right, *top, 0.0, *left, *top, 0.0,
            ]
        })
        .collect::<Vec<_>>();
    let normals = (0..quads.len())
        .flat_map(|_| [0.0f32, 0.0, 1.0].repeat(4))
        .collect::<Vec<_>>();
    let uvs = (0..quads.len())
        .flat_map(|_| [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])
        .collect::<Vec<_>>();

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    for primitive in 0..quads.len() as u16 {
        let base = primitive * 4;
        let indices = [base, base + 1, base + 2, base, base + 2, base + 3];
        bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
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
