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
    name = "generate-front-face-cull-fixture",
    about = "Generate a source-like VRM1 glTF fixture for owner-id front-face/cull diagnostics"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/front-face-cull.vrm.gltf"
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
    let mut bytes = Vec::new();
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let position_accessor =
        push_vec3_accessor(&mut bytes, &mut buffer_views, &mut accessors, &positions());
    let normal_accessor =
        push_vec3_accessor(&mut bytes, &mut buffer_views, &mut accessors, &normals());
    let uv_accessor = push_vec2_accessor(&mut bytes, &mut buffer_views, &mut accessors, &uvs());

    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs front-face/cull render parity generator"
        },
        "extensionsUsed": [
            "VRMC_vrm",
            "VRMC_materials_mtoon"
        ],
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": nodes(),
        "buffers": [{
            "uri": format!("data:application/octet-stream;base64,{}", base64(&bytes)),
            "byteLength": bytes.len()
        }],
        "bufferViews": buffer_views,
        "accessors": accessors,
        "materials": [mtoon_material()],
        "meshes": [{
            "name": "front-face-cull-triangles",
            "primitives": [{
                "attributes": {
                    "POSITION": position_accessor,
                    "NORMAL": normal_accessor,
                    "TEXCOORD_0": uv_accessor
                },
                "material": 0
            }]
        }],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Front Face Cull Fixture",
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

fn positions() -> Vec<[f32; 3]> {
    vec![
        // Left triangle: one winding.
        [-0.88, 0.35, 0.0],
        [-0.18, 0.35, 0.0],
        [-0.53, 1.45, 0.0],
        // Right triangle: opposite winding, same material and cull policy.
        [0.18, 0.35, 0.0],
        [0.53, 1.45, 0.0],
        [0.88, 0.35, 0.0],
    ]
}

fn normals() -> Vec<[f32; 3]> {
    vec![[0.0, 0.0, 1.0]; positions().len()]
}

fn uvs() -> Vec<[f32; 2]> {
    vec![
        [0.0, 0.0],
        [1.0, 0.0],
        [0.5, 1.0],
        [0.0, 0.0],
        [0.5, 1.0],
        [1.0, 0.0],
    ]
}

fn nodes() -> Vec<Value> {
    let mut nodes = (0..15)
        .map(|index| json!({ "name": format!("node_{index}") }))
        .collect::<Vec<_>>();
    nodes[0]["name"] = json!("front_face_cull_root");
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
        "name": "front-face-cull-back",
        "alphaMode": "OPAQUE",
        "doubleSided": false,
        "pbrMetallicRoughness": {
            "baseColorFactor": [0.95, 0.95, 0.95, 1.0],
            "metallicFactor": 0.0,
            "roughnessFactor": 1.0
        },
        "extensions": {
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "shadeColorFactor": [0.95, 0.95, 0.95],
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
