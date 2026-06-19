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
    name = "generate-dense-ownership-fixture",
    about = "Generate a source-like VRM1 glTF fixture for dense same-material local ownership parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/dense-ownership.vrm.gltf"
    )]
    out: PathBuf,
}

#[derive(Clone, Copy, Debug)]
struct Triangle {
    positions: [[f32; 3]; 3],
    uvs: [[f32; 2]; 3],
}

#[derive(Clone, Debug)]
struct MeshDef {
    name: &'static str,
    triangles: Vec<Triangle>,
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
            "generator": "vrm-rs dense ownership render parity generator"
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
                    "name": "vrm-rs Dense Ownership Fixture",
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
    let mut nodes = (0..20)
        .map(|index| json!({ "name": format!("node_{index}") }))
        .collect::<Vec<_>>();
    nodes[0]["name"] = json!("dense_ownership_root");
    nodes[0]["children"] = json!([1, 2, 3, 4, 6, 9, 12, 15, 18]);
    for (node, name, mesh) in [
        (1, "huku_bake_dense_left", 0),
        (2, "huku_bake_dense_center", 1),
        (3, "huku_bake_dense_right", 2),
        (4, "huku_bake_dense_overlay", 3),
    ] {
        nodes[node]["name"] = json!(name);
        nodes[node]["mesh"] = json!(mesh);
    }
    nodes
}

fn human_bones() -> Map<String, Value> {
    [
        ("hips", 0),
        ("spine", 5),
        ("head", 6),
        ("leftUpperLeg", 7),
        ("leftLowerLeg", 8),
        ("leftFoot", 9),
        ("rightUpperLeg", 10),
        ("rightLowerLeg", 11),
        ("rightFoot", 12),
        ("leftUpperArm", 13),
        ("leftLowerArm", 14),
        ("leftHand", 15),
        ("rightUpperArm", 16),
        ("rightLowerArm", 17),
        ("rightHand", 18),
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
            let positions = mesh_def
                .triangles
                .iter()
                .flat_map(|triangle| triangle.positions)
                .collect::<Vec<_>>();
            let uvs = mesh_def
                .triangles
                .iter()
                .flat_map(|triangle| triangle.uvs)
                .collect::<Vec<_>>();
            let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
            let position_accessor =
                push_vec3_accessor(&mut bytes, &mut buffer_views, &mut accessors, &positions);
            let normal_accessor =
                push_vec3_accessor(&mut bytes, &mut buffer_views, &mut accessors, &normals);
            let uv_accessor =
                push_vec2_accessor(&mut bytes, &mut buffer_views, &mut accessors, &uvs);
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

    let texture = dense_texture_png();
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
        dense_strip("huku_bake_dense_left", -0.54, -0.004, 0.00),
        dense_strip("huku_bake_dense_center", -0.18, 0.003, 0.31),
        dense_strip("huku_bake_dense_right", 0.16, -0.002, 0.58),
        dense_overlay("huku_bake_dense_overlay"),
    ]
}

fn dense_strip(name: &'static str, x_start: f32, y_jitter: f32, uv_offset: f32) -> MeshDef {
    let triangles = (0..8)
        .flat_map(|index| {
            let x0 = x_start + index as f32 * 0.055;
            let x1 = x0 + 0.135;
            let y0 = 0.58 + y_jitter * index as f32;
            let y1 = 1.42 - y_jitter * (7 - index) as f32;
            let split = 0.615 + 0.004 * (index % 3) as f32;
            let u0 = (uv_offset + 0.071 * index as f32).fract();
            let u1 = (u0 + 0.19).min(0.98);
            let u2 = (u0 + 0.37).fract().max(0.02);
            [
                Triangle {
                    positions: [[x0, y0, 0.0], [x1, y0 + 0.010, 0.0], [x1, y1, 0.0]],
                    uvs: [[u0, 0.04], [u1, 0.06], [u1, 0.96]],
                },
                Triangle {
                    positions: [
                        [x0 + 0.006, split, 0.0],
                        [x1 - 0.004, split + 0.008, 0.0],
                        [x0, y1, 0.0],
                    ],
                    uvs: [[u2, 0.02], [(u2 + 0.16).min(0.99), 0.12], [u2, 0.94]],
                },
            ]
        })
        .collect();
    MeshDef { name, triangles }
}

fn dense_overlay(name: &'static str) -> MeshDef {
    let triangles = (0..10)
        .map(|index| {
            let x = -0.42 + index as f32 * 0.095;
            let y = 0.72 + (index % 4) as f32 * 0.045;
            let u = 0.08 + (index % 5) as f32 * 0.17;
            Triangle {
                positions: [
                    [x, y, 0.0],
                    [x + 0.19, y + 0.035, 0.0],
                    [x + 0.025, y + 0.46, 0.0],
                ],
                uvs: [[u, 0.08], [(u + 0.21).min(0.98), 0.22], [u, 0.92]],
            }
        })
        .collect();
    MeshDef { name, triangles }
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
        json!(min),
        json!(max),
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
        json!(min),
        json!(max),
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
    bytes.extend(values.iter().copied().flat_map(f32::to_le_bytes));
    let byte_length = bytes.len() - byte_offset;
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

fn min_max_vec3(values: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    values.iter().fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |(mut min, mut max), value| {
            for index in 0..3 {
                min[index] = min[index].min(value[index]);
                max[index] = max[index].max(value[index]);
            }
            (min, max)
        },
    )
}

fn min_max_vec2(values: &[[f32; 2]]) -> ([f32; 2], [f32; 2]) {
    values.iter().fold(
        ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
        |(mut min, mut max), value| {
            for index in 0..2 {
                min[index] = min[index].min(value[index]);
                max[index] = max[index].max(value[index]);
            }
            (min, max)
        },
    )
}

fn dense_texture_png() -> Vec<u8> {
    let mut rgba = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32u8 {
        for x in 0..32u8 {
            let diagonal = x.wrapping_mul(7).wrapping_add(y.wrapping_mul(13));
            let tile = ((x / 4) + (y / 4)) % 2;
            let color = if tile == 0 {
                [
                    28u8.saturating_add(diagonal),
                    235u8.saturating_sub(x.wrapping_mul(5)),
                    44u8.saturating_add(y.wrapping_mul(6)),
                    255,
                ]
            } else {
                [
                    245u8.saturating_sub(y.wrapping_mul(4)),
                    36u8.saturating_add(x.wrapping_mul(6)),
                    220u8.saturating_sub(diagonal / 2),
                    255,
                ]
            };
            rgba.extend(color);
        }
    }

    let mut bytes = Vec::new();
    {
        let mut cursor = Cursor::new(&mut bytes);
        let mut encoder = Encoder::new(&mut cursor, 32, 32);
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
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
