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

const PRIMITIVE_COUNT: usize = 4;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-texture-selection-fixture",
    about = "Generate a source-like VRM1 glTF fixture for per-material base texture selection parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/texture-selection.vrm.gltf"
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
    let textures = texture_pngs();
    let mesh_len = mesh.len();
    let mut buffer = mesh;
    let image_views = textures
        .iter()
        .scan(mesh_len, |offset, texture| {
            let view = json!({
                "buffer": 0,
                "byteOffset": *offset,
                "byteLength": texture.len()
            });
            *offset += texture.len();
            Some(view)
        })
        .collect::<Vec<_>>();
    buffer.extend(textures.iter().flatten().copied());

    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs texture selection render parity generator"
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
        "images": (0..PRIMITIVE_COUNT)
            .map(|index| json!({
                "mimeType": "image/png",
                "bufferView": 3 + PRIMITIVE_COUNT + index
            }))
            .collect::<Vec<_>>(),
        "textures": (0..PRIMITIVE_COUNT)
            .map(|index| json!({ "sampler": 0, "source": index }))
            .collect::<Vec<_>>(),
        "bufferViews": buffer_views(mesh_len, image_views),
        "accessors": accessors(),
        "materials": (0..PRIMITIVE_COUNT)
            .map(mtoon_material)
            .collect::<Vec<_>>(),
        "meshes": [{
            "name": "texture-selection-quads",
            "primitives": (0..PRIMITIVE_COUNT)
                .map(|index| json!({
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
                    "name": "vrm-rs Texture Selection Fixture",
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

fn buffer_views(mesh_len: usize, image_views: Vec<Value>) -> Vec<Value> {
    debug_assert_eq!(mesh_len, 560);
    let mut views = vec![
        { json!({ "buffer": 0, "byteOffset": 0, "byteLength": 192, "target": 34962 }) },
        { json!({ "buffer": 0, "byteOffset": 192, "byteLength": 192, "target": 34962 }) },
        { json!({ "buffer": 0, "byteOffset": 384, "byteLength": 128, "target": 34962 }) },
    ];
    views.extend((0..PRIMITIVE_COUNT).map(|index| {
        json!({
            "buffer": 0,
            "byteOffset": 512 + index * 12,
            "byteLength": 12,
            "target": 34963
        })
    }));
    views.extend(image_views);
    views
}

fn accessors() -> Vec<Value> {
    let mut accessors = vec![
        json!({
            "bufferView": 0,
            "componentType": 5126,
            "count": 16,
            "type": "VEC3",
            "min": [-0.86, 0.36, 0.0],
            "max": [0.86, 1.64, 0.0]
        }),
        json!({
            "bufferView": 1,
            "componentType": 5126,
            "count": 16,
            "type": "VEC3",
            "min": [0.0, 0.0, 1.0],
            "max": [0.0, 0.0, 1.0]
        }),
        json!({
            "bufferView": 2,
            "componentType": 5126,
            "count": 16,
            "type": "VEC2",
            "min": [0.0, 0.0],
            "max": [1.0, 1.0]
        }),
    ];
    accessors.extend((0..PRIMITIVE_COUNT).map(|index| {
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

fn mtoon_material(texture_index: usize) -> Value {
    json!({
        "name": format!("texture-selection-{texture_index}"),
        "alphaMode": "OPAQUE",
        "doubleSided": false,
        "pbrMetallicRoughness": {
            "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
            "baseColorTexture": { "index": texture_index },
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
    let quads = [
        (-0.86f32, -0.08f32, 0.36f32, 0.94f32),
        (0.08, 0.86, 0.36, 0.94),
        (-0.86, -0.08, 1.06, 1.64),
        (0.08, 0.86, 1.06, 1.64),
    ];
    let positions = quads
        .iter()
        .flat_map(|(left, right, bottom, top)| {
            [
                *left, *bottom, 0.0, *right, *bottom, 0.0, *right, *top, 0.0, *left, *top, 0.0,
            ]
        })
        .collect::<Vec<_>>();
    let normals = (0..PRIMITIVE_COUNT * 4)
        .flat_map(|_| [0.0f32, 0.0, 1.0])
        .collect::<Vec<_>>();
    let uvs = (0..PRIMITIVE_COUNT)
        .flat_map(|_| [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])
        .collect::<Vec<_>>();

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    for primitive in 0..PRIMITIVE_COUNT as u16 {
        let base = primitive * 4;
        bytes.extend(
            [base, base + 1, base + 2, base, base + 2, base + 3]
                .into_iter()
                .flat_map(u16::to_le_bytes),
        );
    }
    bytes
}

fn texture_pngs() -> Vec<Vec<u8>> {
    [
        ([255, 48, 40], [255, 180, 80]),
        ([50, 210, 95], [190, 255, 120]),
        ([48, 120, 255], [120, 230, 255]),
        ([255, 70, 220], [255, 230, 80]),
    ]
    .into_iter()
    .map(|(low, high)| gradient_png(low, high))
    .collect()
}

fn gradient_png(low: [u8; 3], high: [u8; 3]) -> Vec<u8> {
    let rgba = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| {
                let amount = (x + y) as f32 / 6.0;
                (0..3)
                    .map(move |channel| {
                        let low = low[channel] as f32;
                        let high = high[channel] as f32;
                        (low + (high - low) * amount).round() as u8
                    })
                    .chain([255])
            })
        })
        .collect::<Vec<_>>();
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
