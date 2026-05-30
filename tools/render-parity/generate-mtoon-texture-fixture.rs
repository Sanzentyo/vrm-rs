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
    name = "generate-mtoon-texture-fixture",
    about = "Generate a source-like VRM1 glTF fixture for MToon texture slot parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/mtoon-texture-slots.vrm.gltf"
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
    let slot_texture = slot_texture_png();
    let mesh_len = mesh.len();
    let slot_texture_len = slot_texture.len();
    let mut buffer = mesh;
    buffer.extend(slot_texture);
    let primitive_count = materials().len();
    let buffer_views = buffer_views(primitive_count, mesh_len, slot_texture_len);

    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs MToon texture slot render parity generator"
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
        "images": [
            { "mimeType": "image/png", "bufferView": 4 + primitive_count }
        ],
        "textures": [
            { "sampler": 0, "source": 0 }
        ],
        "bufferViews": buffer_views,
        "accessors": accessors(primitive_count),
        "materials": materials(),
        "meshes": [{
            "name": "mtoon-texture-slot-grid",
            "primitives": (0..primitive_count)
                .map(|index| json!({
                    "attributes": {
                        "POSITION": 0,
                        "NORMAL": 1,
                        "TANGENT": 2,
                        "TEXCOORD_0": 3
                    },
                    "indices": 4 + index,
                    "material": index
                }))
                .collect::<Vec<_>>()
        }],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs MToon Texture Slot Fixture",
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

fn buffer_views(
    primitive_count: usize,
    mesh_len: usize,
    slot_texture_len: usize,
) -> Vec<Value> {
    let vertex_count = primitive_count * 4;
    let position_len = vertex_count * 3 * 4;
    let normal_len = vertex_count * 3 * 4;
    let tangent_len = vertex_count * 4 * 4;
    let uv_len = vertex_count * 2 * 4;
    let index_len = 6 * 2;
    let mut offset = 0;
    let mut views = Vec::new();
    for (byte_length, target) in [
        (position_len, 34962),
        (normal_len, 34962),
        (tangent_len, 34962),
        (uv_len, 34962),
    ] {
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
    debug_assert_eq!(offset, mesh_len);
    views.push(json!({
        "buffer": 0,
        "byteOffset": mesh_len,
        "byteLength": slot_texture_len
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
            "min": [-0.82, 0.25, 0.0],
            "max": [0.82, 1.75, 0.0]
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
            "type": "VEC4",
            "min": [1.0, 0.0, 0.0, 1.0],
            "max": [1.0, 0.0, 0.0, 1.0]
        }),
        json!({
            "bufferView": 3,
            "componentType": 5126,
            "count": vertex_count,
            "type": "VEC2",
            "min": [0.0, 0.0],
            "max": [1.0, 1.0]
        }),
    ];
    accessors.extend((0..primitive_count).map(|index| {
        json!({
            "bufferView": 4 + index,
            "componentType": 5123,
            "count": 6,
            "type": "SCALAR"
        })
    }));
    accessors
}

fn materials() -> Vec<Value> {
    vec![
        material(
            "shading-shift-texture",
            [0.95, 0.55, 0.12, 1.0],
            json!({
                "shadeColorFactor": [0.05, 0.03, 0.12],
                "shadingShiftFactor": -0.35,
                "shadingShiftTexture": { "index": 0, "scale": 0.45 },
                "shadingToonyFactor": 0.2
            }),
            None,
        ),
        material(
            "rim-texture",
            [0.10, 0.08, 0.16, 1.0],
            json!({
                "shadeColorFactor": [0.01, 0.01, 0.03],
                "shadingShiftFactor": -0.25,
                "shadingToonyFactor": 0.8,
                "parametricRimColorFactor": [0.9, 0.55, 1.0],
                "rimMultiplyTexture": { "index": 0 },
                "rimLightingMixFactor": 1.0,
                "parametricRimFresnelPowerFactor": 1.0,
                "parametricRimLiftFactor": 0.2
            }),
            None,
        ),
        material(
            "uv-animation-mask",
            [1.0, 1.0, 1.0, 1.0],
            json!({
                "shadeColorFactor": [0.24, 0.12, 0.02],
                "shadingShiftFactor": -0.2,
                "shadingToonyFactor": 0.7,
                "uvAnimationMaskTexture": { "index": 0 },
                "uvAnimationScrollXSpeedFactor": 0.35,
                "uvAnimationScrollYSpeedFactor": 0.15,
                "uvAnimationRotationSpeedFactor": 0.25
            }),
            None,
        ),
        material(
            "outline-width-texture",
            [0.64, 0.92, 0.40, 1.0],
            json!({
                "shadeColorFactor": [0.04, 0.18, 0.04],
                "shadingShiftFactor": -0.1,
                "shadingToonyFactor": 0.85,
                "outlineWidthMode": "worldCoordinates",
                "outlineWidthFactor": 0.035,
                "outlineWidthMultiplyTexture": { "index": 0 },
                "outlineColorFactor": [0.02, 0.04, 0.02],
                "outlineLightingMixFactor": 0.0
            }),
            None,
        ),
    ]
}

fn material(name: &str, base_color: [f32; 4], mut extension: Value, normal: Option<Value>) -> Value {
    extension["specVersion"] = json!("1.0");
    extension["giEqualizationFactor"] = json!(0.9);
    if extension.get("outlineWidthMode").is_none() {
        extension["outlineWidthMode"] = json!("none");
    }

    let mut material = json!({
        "name": name,
        "alphaMode": "OPAQUE",
        "doubleSided": true,
        "pbrMetallicRoughness": {
            "baseColorFactor": base_color,
            "baseColorTexture": { "index": 0 },
            "metallicFactor": 0.0,
            "roughnessFactor": 1.0
        },
        "extensions": {
            "VRMC_materials_mtoon": extension
        }
    });
    if let Some(normal) = normal {
        material["normalTexture"] = normal;
    }
    material
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

fn mesh_buffer() -> Vec<u8> {
    let quads = [
        (-0.82f32, -0.08f32, 0.25f32, 0.95f32),
        (0.08, 0.82, 0.25, 0.95),
        (-0.82, -0.08, 1.05, 1.75),
        (0.08, 0.82, 1.05, 1.75),
    ];
    let positions = quads
        .iter()
        .flat_map(|(left, right, bottom, top)| {
            [
                *left, *bottom, 0.0, *right, *bottom, 0.0, *right, *top, 0.0, *left, *top, 0.0,
            ]
        })
        .collect::<Vec<_>>();
    let normals = (0..quads.len() * 4)
        .flat_map(|_| [0.0f32, 0.0, 1.0])
        .collect::<Vec<_>>();
    let tangents = (0..quads.len() * 4)
        .flat_map(|_| [1.0f32, 0.0, 0.0, 1.0])
        .collect::<Vec<_>>();
    let uvs = (0..quads.len())
        .flat_map(|_| [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])
        .collect::<Vec<_>>();

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(tangents.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    for primitive in 0..quads.len() as u16 {
        let base = primitive * 4;
        let indices = [base, base + 1, base + 2, base, base + 2, base + 3];
        bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
    }
    bytes
}

fn slot_texture_png() -> Vec<u8> {
    let rgba = [
        255, 80, 40, 255, 220, 160, 40, 255, 80, 220, 120, 255, 40, 140, 255, 255, 255, 80, 40,
        255, 220, 160, 40, 255, 80, 220, 120, 255, 40, 140, 255, 255, 255, 80, 40, 255, 220,
        160, 40, 255, 80, 220, 120, 255, 40, 140, 255, 255, 255, 80, 40, 255, 220, 160, 40,
        255, 80, 220, 120, 255, 40, 140, 255, 255,
    ];
    png_rgba(4, 4, &rgba)
}

fn png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut cursor = Cursor::new(&mut bytes);
        let mut encoder = Encoder::new(&mut cursor, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .expect("generated PNG header should be valid");
        writer
            .write_image_data(rgba)
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
