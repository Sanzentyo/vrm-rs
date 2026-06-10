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
    name = "generate-texture-boundary-fixture",
    about = "Generate a source-like VRM1 glTF fixture for base texture UV-boundary parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/texture-boundary.vrm.gltf"
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
    let texture = texture_png();
    let mesh_len = mesh.len();
    let texture_len = texture.len();
    let mut buffer = mesh;
    buffer.extend(texture);

    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs texture boundary render parity generator"
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
        "samplers": [{
            "magFilter": 9729,
            "minFilter": 9729,
            "wrapS": 10497,
            "wrapT": 10497
        }],
        "images": [{
            "mimeType": "image/png",
            "bufferView": 4
        }],
        "textures": [{
            "sampler": 0,
            "source": 0
        }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 144, "target": 34962 },
            { "buffer": 0, "byteOffset": 144, "byteLength": 144, "target": 34962 },
            { "buffer": 0, "byteOffset": 288, "byteLength": 96, "target": 34962 },
            { "buffer": 0, "byteOffset": 384, "byteLength": 24, "target": 34963 },
            { "buffer": 0, "byteOffset": mesh_len, "byteLength": texture_len }
        ],
        "accessors": [
            {
                "bufferView": 0,
                "componentType": 5126,
                "count": 12,
                "type": "VEC3",
                "min": [-0.82, 0.58, 0.0],
                "max": [0.82, 1.42, 0.0]
            },
            {
                "bufferView": 1,
                "componentType": 5126,
                "count": 12,
                "type": "VEC3",
                "min": [0.0, 0.0, 1.0],
                "max": [0.0, 0.0, 1.0]
            },
            {
                "bufferView": 2,
                "componentType": 5126,
                "count": 12,
                "type": "VEC2",
                "min": [0.0, 0.0],
                "max": [1.0, 1.0]
            },
            {
                "bufferView": 3,
                "componentType": 5123,
                "count": 12,
                "type": "SCALAR"
            }
        ],
        "materials": [mtoon_material()],
        "meshes": [{
            "name": "texture-boundary-panels",
            "primitives": [
                {
                    "attributes": {
                        "POSITION": 0,
                        "NORMAL": 1,
                        "TEXCOORD_0": 2
                    },
                    "indices": 3,
                    "material": 0
                }
            ]
        }],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Texture Boundary Fixture",
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
        "name": "texture-boundary-mtoon",
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

fn mesh_buffer() -> Vec<u8> {
    let positions = [
        // Left panel, split along the descending diagonal.
        -0.82, 0.58, 0.0, -0.02, 0.58, 0.0, -0.02, 1.42, 0.0,
        -0.82, 0.58, 0.0, -0.02, 1.42, 0.0, -0.82, 1.42, 0.0,
        // Right panel, split along the ascending diagonal and with discontinuous UVs.
        0.02, 0.58, 0.0, 0.82, 0.58, 0.0, 0.82, 1.42, 0.0,
        0.02, 0.58, 0.0, 0.82, 1.42, 0.0, 0.02, 1.42, 0.0,
    ];
    let normals = [0.0f32, 0.0, 1.0].repeat(12);
    let uvs = [
        // Continuous left panel.
        0.00, 0.00, 0.48, 0.00, 0.48, 1.00,
        0.00, 0.00, 0.48, 1.00, 0.00, 1.00,
        // Right panel intentionally jumps in U across its internal diagonal.
        0.52, 0.00, 1.00, 0.00, 1.00, 1.00,
        0.02, 0.00, 0.50, 1.00, 0.02, 1.00,
    ];
    let indices = [0u16, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
    bytes
}

fn texture_png() -> Vec<u8> {
    let rgba = [
        255, 64, 32, 255, 255, 160, 32, 255, 64, 220, 96, 255, 32, 160, 255, 255,
        240, 72, 160, 255, 220, 220, 64, 255, 80, 240, 220, 255, 96, 120, 255, 255,
        255, 96, 64, 255, 250, 190, 96, 255, 120, 240, 120, 255, 80, 190, 255, 255,
        230, 80, 210, 255, 255, 220, 120, 255, 140, 255, 220, 255, 160, 160, 255, 255,
    ];
    let mut bytes = Vec::new();
    {
        let mut cursor = Cursor::new(&mut bytes);
        let mut encoder = Encoder::new(&mut cursor, 4, 4);
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
