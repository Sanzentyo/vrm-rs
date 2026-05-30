#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

use clap::Parser;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-screen-outline-fixture",
    about = "Generate a source-like VRM1 glTF fixture for MToon screen-coordinate outline parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/screen-outline.vrm.gltf"
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
    let mesh = mesh_buffer();
    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs MToon screen outline render parity generator"
        },
        "extensionsUsed": [
            "VRMC_vrm",
            "VRMC_materials_mtoon"
        ],
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": nodes(),
        "buffers": [{
            "uri": format!("data:application/octet-stream;base64,{}", base64(&mesh)),
            "byteLength": mesh.len()
        }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 576, "target": 34962 },
            { "buffer": 0, "byteOffset": 576, "byteLength": 576, "target": 34962 },
            { "buffer": 0, "byteOffset": 1152, "byteLength": 384, "target": 34962 },
            { "buffer": 0, "byteOffset": 1536, "byteLength": 72, "target": 34963 }
        ],
        "accessors": [
            {
                "bufferView": 0,
                "componentType": 5126,
                "count": 48,
                "type": "VEC3",
                "min": [-1.25, 0.35, -0.28],
                "max": [1.25, 1.65, 0.42]
            },
            {
                "bufferView": 1,
                "componentType": 5126,
                "count": 48,
                "type": "VEC3",
                "min": [-1.0, -1.0, -1.0],
                "max": [1.0, 1.0, 1.0]
            },
            {
                "bufferView": 2,
                "componentType": 5126,
                "count": 48,
                "type": "VEC2",
                "min": [0.0, 0.0],
                "max": [1.0, 1.0]
            },
            {
                "bufferView": 3,
                "componentType": 5123,
                "count": 36,
                "type": "SCALAR"
            }
        ],
        "materials": [
            mtoon_material()
        ],
        "meshes": [{
            "name": "screen-outline-prism",
            "primitives": [{
                "attributes": {
                    "POSITION": 0,
                    "NORMAL": 1,
                    "TEXCOORD_0": 2
                },
                "indices": 3,
                "material": 0
            }]
        }],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Screen Outline Fixture",
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

fn mtoon_material() -> Value {
    json!({
        "name": "screen-coordinate-outline",
        "alphaMode": "OPAQUE",
        "doubleSided": false,
        "pbrMetallicRoughness": {
            "baseColorFactor": [0.72, 0.86, 1.0, 1.0],
            "metallicFactor": 0.0,
            "roughnessFactor": 1.0
        },
        "extensions": {
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "shadeColorFactor": [0.18, 0.28, 0.45],
                "shadingShiftFactor": 0.0,
                "shadingToonyFactor": 0.9,
                "giEqualizationFactor": 0.9,
                "outlineWidthMode": "screenCoordinates",
                "outlineWidthFactor": 0.055,
                "outlineColorFactor": [0.04, 0.06, 0.12],
                "outlineLightingMixFactor": 0.0
            }
        }
    })
}

fn mesh_buffer() -> Vec<u8> {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    push_box(
        &mut positions,
        &mut normals,
        &mut uvs,
        &mut indices,
        [-0.95, 0.45, -0.28],
        [0.05, 1.55, 0.20],
    );
    push_box(
        &mut positions,
        &mut normals,
        &mut uvs,
        &mut indices,
        [0.25, 0.35, -0.08],
        [1.25, 1.65, 0.42],
    );

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
    bytes
}

fn push_box(
    positions: &mut Vec<f32>,
    normals: &mut Vec<f32>,
    uvs: &mut Vec<f32>,
    indices: &mut Vec<u16>,
    min: [f32; 3],
    max: [f32; 3],
) {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let faces = [
        (
            [0.0, 0.0, -1.0],
            [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]],
        ),
        (
            [0.0, 0.0, 1.0],
            [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        ),
        (
            [-1.0, 0.0, 0.0],
            [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        ),
        (
            [1.0, 0.0, 0.0],
            [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]],
        ),
        (
            [0.0, -1.0, 0.0],
            [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        ),
        (
            [0.0, 1.0, 0.0],
            [[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]],
        ),
    ];

    for (normal, vertices) in faces {
        let base = (positions.len() / 3) as u16;
        for vertex in vertices {
            positions.extend(vertex);
            normals.extend(normal);
        }
        uvs.extend([0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]);
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
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
