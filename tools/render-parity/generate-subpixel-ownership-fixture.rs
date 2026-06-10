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
    name = "generate-subpixel-ownership-fixture",
    about = "Generate a source-like VRM1 glTF fixture for subpixel same-material ownership parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/subpixel-ownership.vrm.gltf"
    )]
    out: PathBuf,
}

#[derive(Clone, Copy, Debug)]
struct Triangle {
    positions: [[f32; 3]; 3],
    uvs: [[f32; 2]; 3],
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
            "generator": "vrm-rs subpixel ownership render parity generator"
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
            { "buffer": 0, "byteOffset": 0, "byteLength": 288, "target": 34962 },
            { "buffer": 0, "byteOffset": 288, "byteLength": 288, "target": 34962 },
            { "buffer": 0, "byteOffset": 576, "byteLength": 192, "target": 34962 },
            { "buffer": 0, "byteOffset": 768, "byteLength": 48, "target": 34963 },
            { "buffer": 0, "byteOffset": mesh_len, "byteLength": texture_len }
        ],
        "accessors": [
            {
                "bufferView": 0,
                "componentType": 5126,
                "count": 24,
                "type": "VEC3",
                "min": [-0.64, 0.58, 0.0],
                "max": [0.64, 1.42, 0.0]
            },
            {
                "bufferView": 1,
                "componentType": 5126,
                "count": 24,
                "type": "VEC3",
                "min": [0.0, 0.0, 1.0],
                "max": [0.0, 0.0, 1.0]
            },
            {
                "bufferView": 2,
                "componentType": 5126,
                "count": 24,
                "type": "VEC2",
                "min": [0.0, 0.0],
                "max": [1.0, 1.0]
            },
            {
                "bufferView": 3,
                "componentType": 5123,
                "count": 24,
                "type": "SCALAR"
            }
        ],
        "materials": [mtoon_material()],
        "meshes": [{
            "name": "huku_bake_subpixel_panels",
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
                    "name": "vrm-rs Subpixel Ownership Fixture",
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
    nodes[0]["name"] = json!("subpixel_root");
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

fn mesh_buffer() -> Vec<u8> {
    let triangles = triangles();
    let positions = triangles
        .iter()
        .flat_map(|triangle| triangle.positions)
        .collect::<Vec<_>>();
    let normals = vec![[0.0f32, 0.0, 1.0]; positions.len()];
    let uvs = triangles
        .iter()
        .flat_map(|triangle| triangle.uvs)
        .collect::<Vec<_>>();
    let indices = (0..u16::try_from(positions.len()).expect("fixture vertex count fits u16"))
        .collect::<Vec<_>>();

    let mut bytes = Vec::new();
    bytes.extend(
        positions
            .iter()
            .flat_map(|position| position.iter().copied())
            .flat_map(f32::to_le_bytes),
    );
    bytes.extend(
        normals
            .iter()
            .flat_map(|normal| normal.iter().copied())
            .flat_map(f32::to_le_bytes),
    );
    bytes.extend(
        uvs.iter()
            .flat_map(|uv| uv.iter().copied())
            .flat_map(f32::to_le_bytes),
    );
    bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
    bytes
}

fn triangles() -> Vec<Triangle> {
    let panels = [
        (-0.48, 0.620),
        (-0.16, 0.623),
        (0.16, 0.626),
        (0.48, 0.629),
    ];
    panels
        .into_iter()
        .flat_map(|(center_x, seam_y)| panel_triangles(center_x, seam_y))
        .collect()
}

fn panel_triangles(center_x: f32, seam_y: f32) -> [Triangle; 2] {
    let half_width = 0.14;
    let x0 = center_x - half_width;
    let x1 = center_x + half_width;
    let y0 = 0.58;
    let y1 = 1.42;
    let seam_low = seam_y;
    let seam_high = seam_y + 0.006;
    [
        Triangle {
            positions: [[x0, y0, 0.0], [x1, y0, 0.0], [x1, y1, 0.0]],
            uvs: [[0.02, 0.02], [0.46, 0.02], [0.46, 0.98]],
        },
        Triangle {
            positions: [[x0, seam_low, 0.0], [x1, seam_high, 0.0], [x0, y1, 0.0]],
            uvs: [[0.54, 0.02], [0.98, 0.04], [0.54, 0.98]],
        },
    ]
}

fn texture_png() -> Vec<u8> {
    let mut rgba = Vec::with_capacity(8 * 8 * 4);
    for y in 0..8 {
        for x in 0..8 {
            let color = if x < 4 {
                [
                    224u8.saturating_sub(y * 12),
                    24u8.saturating_add(x * 18),
                    32u8.saturating_add(y * 9),
                    255,
                ]
            } else {
                [
                    20u8.saturating_add(y * 7),
                    208u8.saturating_sub((x - 4) * 16),
                    244u8.saturating_sub(y * 8),
                    255,
                ]
            };
            rgba.extend(color);
        }
    }

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
