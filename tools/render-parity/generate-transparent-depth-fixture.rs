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
    name = "generate-transparent-depth-fixture",
    about = "Generate a source-like VRM1 glTF fixture for transparent depth-sort parity"
)]
struct Options {
    #[arg(long, default_value = ".external-fixtures/generated/transparent-depth-stack.vrm.gltf")]
    out: PathBuf,
}

#[derive(Clone, Copy, Debug)]
struct Layer {
    name: &'static str,
    color: [f32; 4],
    local_z: f32,
    textured: bool,
}

const LAYERS: [Layer; 3] = [
    Layer {
        name: "near-red-unordered",
        color: [1.0, 0.03, 0.02, 0.46],
        local_z: -0.42,
        textured: false,
    },
    Layer {
        name: "middle-green-textured",
        color: [0.0, 0.95, 0.26, 0.48],
        local_z: 0.0,
        textured: true,
    },
    Layer {
        name: "far-blue-unordered",
        color: [0.02, 0.24, 1.0, 0.52],
        local_z: 0.42,
        textured: false,
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
    let texture = alpha_ramp_png();
    let mesh_len = mesh.len();
    let texture_len = texture.len();
    let mut buffer = mesh;
    buffer.extend(texture);

    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs transparent depth-sort parity generator"
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
            "minFilter": 9985,
            "wrapS": 10497,
            "wrapT": 10497
        }],
        "images": [{
            "mimeType": "image/png",
            "bufferView": 6
        }],
        "textures": [{
            "sampler": 0,
            "source": 0
        }],
        "bufferViews": buffer_views(mesh_len, texture_len),
        "accessors": accessors(),
        "materials": materials(),
        "meshes": [{
            "name": "transparent-depth-stack",
            "primitives": primitives()
        }],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Transparent Depth Sort Fixture",
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

fn buffer_views(mesh_len: usize, texture_len: usize) -> Vec<Value> {
    let mut offset = 0;
    let mut views = Vec::new();
    for _ in LAYERS {
        views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": 48,
            "target": 34962
        }));
        offset += 48;
    }
    for (byte_length, target) in [(48, 34962), (32, 34962), (12, 34963)] {
        views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": byte_length,
            "target": target
        }));
        offset += byte_length;
    }
    debug_assert_eq!(offset, mesh_len);
    views.push(json!({
        "buffer": 0,
        "byteOffset": mesh_len,
        "byteLength": texture_len
    }));
    views
}

fn accessors() -> Vec<Value> {
    let mut accessors = LAYERS
        .iter()
        .enumerate()
        .map(|(index, layer)| {
            json!({
                "bufferView": index,
                "componentType": 5126,
                "count": 4,
                "type": "VEC3",
                "min": [-0.55, 0.55, layer.local_z],
                "max": [0.55, 1.45, layer.local_z]
            })
        })
        .collect::<Vec<_>>();
    accessors.extend([
        json!({
            "bufferView": 3,
            "componentType": 5126,
            "count": 4,
            "type": "VEC3",
            "min": [0.0, 0.0, 1.0],
            "max": [0.0, 0.0, 1.0]
        }),
        json!({
            "bufferView": 4,
            "componentType": 5126,
            "count": 4,
            "type": "VEC2",
            "min": [0.0, 0.0],
            "max": [1.0, 1.0]
        }),
        json!({
            "bufferView": 5,
            "componentType": 5123,
            "count": 6,
            "type": "SCALAR"
        }),
    ]);
    accessors
}

fn materials() -> Vec<Value> {
    LAYERS
        .iter()
        .map(|layer| mtoon_material(layer.name, layer.color, layer.textured))
        .collect()
}

fn primitives() -> Vec<Value> {
    LAYERS
        .iter()
        .enumerate()
        .map(|(index, _)| {
            json!({
                "attributes": {
                    "POSITION": index,
                    "NORMAL": 3,
                    "TEXCOORD_0": 4
                },
                "indices": 5,
                "material": index
            })
        })
        .collect()
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

fn mtoon_material(name: &str, base_color: [f32; 4], textured: bool) -> Value {
    let mut pbr = json!({
        "baseColorFactor": base_color,
        "metallicFactor": 0.0,
        "roughnessFactor": 1.0
    });
    if textured {
        pbr["baseColorTexture"] = json!({ "index": 0 });
    }

    json!({
        "name": name,
        "alphaMode": "BLEND",
        "doubleSided": true,
        "pbrMetallicRoughness": pbr,
        "extensions": {
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "renderQueueOffsetNumber": 0,
                "transparentWithZWrite": false,
                "shadeColorFactor": [
                    base_color[0] * 0.72,
                    base_color[1] * 0.72,
                    base_color[2] * 0.72
                ],
                "shadingToonyFactor": 0.95,
                "giEqualizationFactor": 0.9,
                "outlineWidthMode": "none"
            }
        }
    })
}

fn mesh_buffer() -> Vec<u8> {
    let mut bytes = Vec::new();
    for layer in LAYERS {
        let positions = [
            -0.55f32,
            0.55,
            layer.local_z,
            0.55,
            0.55,
            layer.local_z,
            0.55,
            1.45,
            layer.local_z,
            -0.55,
            1.45,
            layer.local_z,
        ];
        bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    }
    let normals = [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    let uvs = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let indices = [0u16, 1, 2, 0, 2, 3];
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
    bytes
}

fn alpha_ramp_png() -> Vec<u8> {
    let mut rgba = [
        255, 255, 255, 255, 248, 252, 255, 224, 255, 248, 252, 192, 252, 255, 248, 160, 248,
        252, 255, 224, 255, 255, 255, 255, 252, 255, 248, 160, 255, 248, 252, 192, 255, 248,
        252, 192, 252, 255, 248, 160, 255, 255, 255, 255, 248, 252, 255, 224, 252, 255, 248,
        160, 255, 248, 252, 192, 255, 255, 255, 255, 248, 252, 255, 224,
    ];
    for pixel in 0..16 {
        rgba[pixel * 4 + 3] = rgba[pixel * 4 + 3].max(96);
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
