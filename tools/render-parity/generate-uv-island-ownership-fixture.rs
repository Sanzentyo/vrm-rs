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
    name = "generate-uv-island-ownership-fixture",
    about = "Generate a source-like VRM1 glTF fixture for same-material UV-island ownership parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/uv-island-ownership.vrm.gltf"
    )]
    out: PathBuf,
}

#[derive(Clone, Debug)]
struct MeshDef {
    name: &'static str,
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
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
            "generator": "vrm-rs uv island ownership render parity generator"
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
                    "name": "vrm-rs UV Island Ownership Fixture",
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
    let mut nodes = (0..18)
        .map(|index| json!({ "name": format!("node_{index}") }))
        .collect::<Vec<_>>();
    nodes[0]["name"] = json!("uv_island_root");
    nodes[0]["children"] = json!([1, 2, 3, 4, 6, 9, 12, 15]);
    nodes[1]["name"] = json!("huku_bake_island_left");
    nodes[1]["mesh"] = json!(0);
    nodes[2]["name"] = json!("huku_bake_island_center");
    nodes[2]["mesh"] = json!(1);
    nodes[3]["name"] = json!("huku_bake_island_right");
    nodes[3]["mesh"] = json!(2);
    nodes
}

fn human_bones() -> Map<String, Value> {
    [
        ("hips", 0),
        ("spine", 4),
        ("head", 5),
        ("leftUpperLeg", 6),
        ("leftLowerLeg", 7),
        ("leftFoot", 8),
        ("rightUpperLeg", 9),
        ("rightLowerLeg", 10),
        ("rightFoot", 11),
        ("leftUpperArm", 12),
        ("leftLowerArm", 13),
        ("leftHand", 14),
        ("rightUpperArm", 15),
        ("rightLowerArm", 16),
        ("rightHand", 17),
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
            let position_accessor = push_vec3_accessor(
                &mut bytes,
                &mut buffer_views,
                &mut accessors,
                &mesh_def.positions,
            );
            let normals = vec![[0.0, 0.0, 1.0]; mesh_def.positions.len()];
            let normal_accessor =
                push_vec3_accessor(&mut bytes, &mut buffer_views, &mut accessors, &normals);
            let uv_accessor =
                push_vec2_accessor(&mut bytes, &mut buffer_views, &mut accessors, &mesh_def.uvs);
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

    let texture = uv_island_texture_png();
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
        island_quad(
            "huku_bake_island_left",
            [-0.68, 0.58],
            [0.08, 1.42],
            [[0.02, 0.05], [0.42, 0.05], [0.42, 0.95], [0.02, 0.95]],
        ),
        island_quad(
            "huku_bake_island_center",
            [-0.10, 0.60],
            [0.52, 1.39],
            [[0.54, 0.06], [0.94, 0.08], [0.92, 0.94], [0.52, 0.92]],
        ),
        island_quad(
            "huku_bake_island_right",
            [0.18, 0.64],
            [0.72, 1.33],
            [[0.10, 0.08], [0.34, 0.12], [0.34, 0.88], [0.08, 0.90]],
        ),
    ]
}

fn island_quad(
    name: &'static str,
    min: [f32; 2],
    max: [f32; 2],
    uv: [[f32; 2]; 4],
) -> MeshDef {
    MeshDef {
        name,
        positions: vec![
            [min[0], min[1], 0.0],
            [max[0], min[1], 0.0],
            [max[0], max[1], 0.0],
            [min[0], min[1], 0.0],
            [max[0], max[1], 0.0],
            [min[0], max[1], 0.0],
        ],
        uvs: vec![uv[0], uv[1], uv[2], uv[0], uv[2], uv[3]],
    }
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
    push_accessor(
        bytes,
        buffer_views,
        accessors,
        &flat,
        values.len(),
        "VEC2",
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
    )
}

fn push_accessor(
    bytes: &mut Vec<u8>,
    buffer_views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
    values: &[f32],
    count: usize,
    accessor_type: &str,
    min: [f32; 3],
    max: [f32; 3],
) -> usize {
    let byte_offset = bytes.len();
    bytes.extend(values.iter().copied().flat_map(f32::to_le_bytes));
    let byte_length = bytes.len() - byte_offset;
    let view = buffer_views.len();
    buffer_views.push(json!({
        "buffer": 0,
        "byteOffset": byte_offset,
        "byteLength": byte_length,
        "target": 34962
    }));
    let accessor = accessors.len();
    let (min, max) = if accessor_type == "VEC2" {
        (json!([min[0], min[1]]), json!([max[0], max[1]]))
    } else {
        (json!(min), json!(max))
    };
    accessors.push(json!({
        "bufferView": view,
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

fn uv_island_texture_png() -> Vec<u8> {
    let mut rgba = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u8 {
        for x in 0..16u8 {
            let island = x / 4;
            let local = x % 4;
            let color = match island {
                0 => [220 - y * 5, 24 + local * 42, 36 + y * 7, 255],
                1 => [32 + y * 8, 210 - local * 28, 250 - y * 6, 255],
                2 => [250 - local * 18, 80 + y * 7, 48 + local * 44, 255],
                _ => [64 + y * 5, 48 + local * 36, 224 - y * 7, 255],
            };
            rgba.extend(color);
        }
    }

    let mut bytes = Vec::new();
    {
        let mut cursor = Cursor::new(&mut bytes);
        let mut encoder = Encoder::new(&mut cursor, 16, 16);
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
