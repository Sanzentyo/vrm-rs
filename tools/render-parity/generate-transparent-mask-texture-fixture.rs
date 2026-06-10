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
    name = "generate-transparent-mask-texture-fixture",
    about = "Generate a source-like VRM1 glTF fixture for texture-alpha MASK/BLEND parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/transparent-mask-texture.vrm.gltf"
    )]
    out: PathBuf,
}

#[derive(Clone, Copy, Debug)]
struct Swatch {
    name: &'static str,
    alpha_mode: &'static str,
    alpha_cutoff: Option<f32>,
    base_color: [f32; 4],
}

const SWATCHES: [Swatch; 3] = [
    Swatch {
        name: "mask-texture-alpha-cutoff",
        alpha_mode: "MASK",
        alpha_cutoff: Some(0.5),
        base_color: [0.04, 0.90, 0.18, 1.0],
    },
    Swatch {
        name: "mask-factor-times-texture-alpha",
        alpha_mode: "MASK",
        alpha_cutoff: Some(0.5),
        base_color: [0.08, 0.45, 1.0, 0.75],
    },
    Swatch {
        name: "blend-texture-alpha-control",
        alpha_mode: "BLEND",
        alpha_cutoff: None,
        base_color: [1.0, 0.14, 0.78, 0.80],
    },
];

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
    let texture = alpha_texture_png();
    let mesh_len = mesh.len();
    let texture_len = texture.len();
    let mut buffer = mesh;
    buffer.extend(texture);

    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs transparent mask texture render parity generator"
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
            "bufferView": 3 + SWATCHES.len()
        }],
        "textures": [{
            "sampler": 0,
            "source": 0
        }],
        "bufferViews": buffer_views(mesh_len, texture_len),
        "accessors": accessors(),
        "materials": SWATCHES.iter().map(material).collect::<Vec<_>>(),
        "meshes": [{
            "name": "transparent-mask-texture-swatches",
            "primitives": SWATCHES
                .iter()
                .enumerate()
                .map(|(index, _)| json!({
                    "attributes": {
                        "POSITION": 0,
                        "NORMAL": 1,
                        "TEXCOORD_0": 2
                    },
                    "indices": 3 + index,
                    "material": index
                }))
                .collect::<Vec<_>>()
        }],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Transparent Mask Texture Fixture",
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

fn material(swatch: &Swatch) -> Value {
    let mut material = json!({
        "name": swatch.name,
        "alphaMode": swatch.alpha_mode,
        "doubleSided": true,
        "pbrMetallicRoughness": {
            "baseColorFactor": swatch.base_color,
            "baseColorTexture": { "index": 0 },
            "metallicFactor": 0.0,
            "roughnessFactor": 1.0
        },
        "extensions": {
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "shadeColorFactor": [
                    swatch.base_color[0] * 0.70,
                    swatch.base_color[1] * 0.70,
                    swatch.base_color[2] * 0.70
                ],
                "shadingShiftFactor": 1.5,
                "shadingToonyFactor": 0.95,
                "giEqualizationFactor": 0.9,
                "outlineWidthMode": "none"
            }
        }
    });
    if let Some(cutoff) = swatch.alpha_cutoff {
        material["alphaCutoff"] = json!(cutoff);
    }
    material
}

fn buffer_views(mesh_len: usize, texture_len: usize) -> Vec<Value> {
    let position_len = SWATCHES.len() * 4 * 3 * 4;
    let normal_len = SWATCHES.len() * 4 * 3 * 4;
    let uv_len = SWATCHES.len() * 4 * 2 * 4;
    let index_len = 6 * 2;
    let mut offset = 0;
    let mut views = Vec::new();
    for (byte_length, target) in [
        (position_len, 34962),
        (normal_len, 34962),
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
    views.extend((0..SWATCHES.len()).map(|_| {
        let view = json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": index_len,
            "target": 34963
        });
        offset += index_len;
        view
    }));
    debug_assert_eq!(offset, mesh_len);
    views.push(json!({
        "buffer": 0,
        "byteOffset": mesh_len,
        "byteLength": texture_len
    }));
    views
}

fn accessors() -> Vec<Value> {
    let vertex_count = SWATCHES.len() * 4;
    let mut accessors = vec![
        json!({
            "bufferView": 0,
            "componentType": 5126,
            "count": vertex_count,
            "type": "VEC3",
            "min": [-0.90, 0.45, 0.0],
            "max": [0.90, 1.55, 0.0]
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
    accessors.extend((0..SWATCHES.len()).map(|index| {
        json!({
            "bufferView": 3 + index,
            "componentType": 5123,
            "count": 6,
            "type": "SCALAR"
        })
    }));
    accessors
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
        (-0.90f32, -0.32f32, 0.45f32, 1.55f32),
        (-0.28, 0.34, 0.45, 1.55),
        (0.40, 0.90, 0.45, 1.55),
    ];
    let positions = quads
        .into_iter()
        .flat_map(|(left, right, bottom, top)| {
            [
                left, bottom, 0.0, right, bottom, 0.0, right, top, 0.0, left, top, 0.0,
            ]
        })
        .collect::<Vec<_>>();
    let normals = (0..SWATCHES.len())
        .flat_map(|_| [0.0f32, 0.0, 1.0].repeat(4))
        .collect::<Vec<_>>();
    let uvs = (0..SWATCHES.len())
        .flat_map(|_| [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])
        .collect::<Vec<_>>();
    let indices = (0..SWATCHES.len() as u16)
        .flat_map(|primitive| {
            let base = primitive * 4;
            [base, base + 1, base + 2, base, base + 2, base + 3]
        })
        .collect::<Vec<_>>();

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
    bytes
}

fn alpha_texture_png() -> Vec<u8> {
    let mut rgba = Vec::with_capacity(4 * 4 * 4);
    for y in 0..4 {
        for x in 0..4 {
            let alpha = [255, 192, 96, 32][x];
            let tint = if (x + y) % 2 == 0 { 255 } else { 236 };
            rgba.extend([tint, tint, tint, alpha]);
        }
    }
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
