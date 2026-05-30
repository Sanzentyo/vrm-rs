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
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-mtoon-light-fixture",
    about = "Generate a source-like VRM1 glTF fixture for isolated MToon light/color parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/mtoon-light.vrm.gltf"
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
    let texture = matcap_texture_png();
    let mesh_len = mesh.len();
    let texture_len = texture.len();
    let mut buffer = mesh;
    buffer.extend(texture);

    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs MToon light render parity generator"
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
            "bufferView": 9
        }],
        "textures": [{
            "sampler": 0,
            "source": 0
        }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 288, "target": 34962 },
            { "buffer": 0, "byteOffset": 288, "byteLength": 288, "target": 34962 },
            { "buffer": 0, "byteOffset": 576, "byteLength": 192, "target": 34962 },
            { "buffer": 0, "byteOffset": 768, "byteLength": 12, "target": 34963 },
            { "buffer": 0, "byteOffset": 780, "byteLength": 12, "target": 34963 },
            { "buffer": 0, "byteOffset": 792, "byteLength": 12, "target": 34963 },
            { "buffer": 0, "byteOffset": 804, "byteLength": 12, "target": 34963 },
            { "buffer": 0, "byteOffset": 816, "byteLength": 12, "target": 34963 },
            { "buffer": 0, "byteOffset": 828, "byteLength": 12, "target": 34963 },
            { "buffer": 0, "byteOffset": mesh_len, "byteLength": texture_len }
        ],
        "accessors": [
            {
                "bufferView": 0,
                "componentType": 5126,
                "count": 24,
                "type": "VEC3",
                "min": [-1.85, 0.15, 0.0],
                "max": [1.85, 1.85, 0.0]
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
            index_accessor(3),
            index_accessor(4),
            index_accessor(5),
            index_accessor(6),
            index_accessor(7),
            index_accessor(8)
        ],
        "materials": [
            mtoon_material("direct-base", [0.92, 0.18, 0.10, 1.0], [0.06, 0.02, 0.02], 1.5, 0.0, 1.0, [0.0, 0.0, 0.0], None, false),
            mtoon_material("forced-shade", [0.12, 0.62, 1.0, 1.0], [0.02, 0.08, 0.28], -1.5, 0.0, 1.0, [0.0, 0.0, 0.0], None, false),
            mtoon_material("ambient-ao-ignored", [0.75, 0.75, 0.75, 1.0], [0.02, 0.02, 0.02], -1.5, 0.0, 0.0, [0.0, 0.0, 0.0], None, true),
            mtoon_material("parametric-rim", [0.02, 0.02, 0.02, 1.0], [0.02, 0.02, 0.02], -1.5, 1.0, 1.0, [1.0, 0.48, 0.12], None, false),
            mtoon_material("matcap-rim", [0.02, 0.02, 0.02, 1.0], [0.02, 0.02, 0.02], -1.5, 0.0, 1.0, [0.0, 0.0, 0.0], Some([0.65, 0.35, 1.0]), false),
            mtoon_material("mixed-rim", [0.25, 0.18, 0.50, 1.0], [0.04, 0.03, 0.10], 0.0, 0.5, 1.0, [0.4, 0.8, 1.0], Some([0.35, 0.55, 1.0]), false)
        ],
        "meshes": [{
            "name": "mtoon-light-grid",
            "primitives": (0..6)
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
                    "name": "vrm-rs MToon Light Fixture",
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

fn index_accessor(buffer_view: usize) -> Value {
    json!({
        "bufferView": buffer_view,
        "componentType": 5123,
        "count": 6,
        "type": "SCALAR"
    })
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

fn mtoon_material(
    name: &str,
    base_color: [f32; 4],
    shade_color: [f32; 3],
    shading_shift: f32,
    rim_lighting_mix: f32,
    gi_equalization: f32,
    rim_color: [f32; 3],
    matcap_factor: Option<[f32; 3]>,
    occlusion: bool,
) -> Value {
    let mut extension = json!({
        "specVersion": "1.0",
        "shadeColorFactor": shade_color,
        "shadingShiftFactor": shading_shift,
        "shadingToonyFactor": 0.9,
        "giEqualizationFactor": gi_equalization,
        "parametricRimColorFactor": rim_color,
        "rimLightingMixFactor": rim_lighting_mix,
        "parametricRimFresnelPowerFactor": 1.0,
        "parametricRimLiftFactor": 0.25,
        "outlineWidthMode": "none"
    });
    if let Some(factor) = matcap_factor {
        extension["matcapFactor"] = json!(factor);
        extension["matcapTexture"] = json!({ "index": 0 });
    }

    let mut material = json!({
        "name": name,
        "alphaMode": "OPAQUE",
        "doubleSided": true,
        "pbrMetallicRoughness": {
            "baseColorFactor": base_color,
            "metallicFactor": 0.0,
            "roughnessFactor": 1.0
        },
        "extensions": {
            "VRMC_materials_mtoon": extension
        }
    });
    if occlusion {
        material["occlusionTexture"] = json!({ "index": 0, "strength": 0.65 });
    }
    material
}

fn mesh_buffer() -> Vec<u8> {
    let quads = [
        (-1.85f32, -0.65f32, 0.15f32, 0.95f32),
        (-0.55, 0.65, 0.15, 0.95),
        (0.75, 1.85, 0.15, 0.95),
        (-1.85, -0.65, 1.05, 1.85),
        (-0.55, 0.65, 1.05, 1.85),
        (0.75, 1.85, 1.05, 1.85),
    ];
    let positions = quads
        .iter()
        .flat_map(|(left, right, bottom, top)| {
            [
                *left, *bottom, 0.0, *right, *bottom, 0.0, *right, *top, 0.0, *left, *top, 0.0,
            ]
        })
        .collect::<Vec<_>>();
    let normals = (0..24).flat_map(|_| [0.0f32, 0.0, 1.0]).collect::<Vec<_>>();
    let uvs = (0..6)
        .flat_map(|_| [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])
        .collect::<Vec<_>>();

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    for primitive in 0..6u16 {
        let base = primitive * 4;
        let indices = [base, base + 1, base + 2, base, base + 2, base + 3];
        bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
    }
    bytes
}

fn matcap_texture_png() -> Vec<u8> {
    let rgba = [
        255, 220, 255, 255, 128, 170, 255, 255, 200, 255, 220, 255, 255, 255, 255, 255,
    ];
    let mut bytes = Vec::new();
    {
        let mut cursor = Cursor::new(&mut bytes);
        let mut encoder = Encoder::new(&mut cursor, 2, 2);
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
