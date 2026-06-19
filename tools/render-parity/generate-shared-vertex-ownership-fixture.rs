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
    name = "generate-shared-vertex-ownership-fixture",
    about = "Generate a source-like VRM1 glTF fixture for indexed shared-vertex local ownership parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/shared-vertex-ownership.vrm.gltf"
    )]
    out: PathBuf,
}

#[derive(Clone, Debug, Default)]
struct MeshData {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u16>,
}

#[derive(Clone, Debug)]
struct FixtureData {
    bytes: Vec<u8>,
    buffer_views: Vec<Value>,
    accessors: Vec<Value>,
    mesh: Value,
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
            "generator": "vrm-rs shared vertex ownership render parity generator"
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
        "meshes": [fixture.mesh],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Shared Vertex Ownership Fixture",
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
    nodes[0]["name"] = json!("shared_vertex_root");
    nodes[0]["children"] = json!([1, 2, 4, 7, 10, 13, 16]);
    nodes[1]["name"] = json!("wear_shared_vertex");
    nodes[1]["mesh"] = json!(0);
    nodes
}

fn human_bones() -> Map<String, Value> {
    [
        ("hips", 0),
        ("spine", 2),
        ("head", 3),
        ("leftUpperLeg", 4),
        ("leftLowerLeg", 5),
        ("leftFoot", 6),
        ("rightUpperLeg", 7),
        ("rightLowerLeg", 8),
        ("rightFoot", 9),
        ("leftUpperArm", 10),
        ("leftLowerArm", 11),
        ("leftHand", 12),
        ("rightUpperArm", 13),
        ("rightLowerArm", 14),
        ("rightHand", 15),
    ]
    .into_iter()
    .map(|(name, node)| (name.to_owned(), json!({ "node": node })))
    .collect()
}

fn fixture_data() -> FixtureData {
    let mesh_data = mesh_data();
    let mut bytes = Vec::new();
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let normals = vec![[0.0, 0.0, 1.0]; mesh_data.positions.len()];
    let position_accessor = push_vec3_accessor(
        &mut bytes,
        &mut buffer_views,
        &mut accessors,
        &mesh_data.positions,
    );
    let normal_accessor =
        push_vec3_accessor(&mut bytes, &mut buffer_views, &mut accessors, &normals);
    let uv_accessor = push_vec2_accessor(
        &mut bytes,
        &mut buffer_views,
        &mut accessors,
        &mesh_data.uvs,
    );
    let index_accessor = push_u16_accessor(
        &mut bytes,
        &mut buffer_views,
        &mut accessors,
        &mesh_data.indices,
    );

    let texture = shared_vertex_texture_png();
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
        mesh: json!({
            "name": "wear",
            "primitives": [{
                "attributes": {
                    "POSITION": position_accessor,
                    "NORMAL": normal_accessor,
                    "TEXCOORD_0": uv_accessor
                },
                "indices": index_accessor,
                "material": 0
            }]
        }),
        image_view,
    }
}

fn mesh_data() -> MeshData {
    let mut mesh = MeshData::default();
    for (center_x, center_y, uv_origin) in [
        (-0.42, 0.76, [0.03, 0.05]),
        (-0.14, 0.94, [0.57, 0.07]),
        (0.15, 0.76, [0.10, 0.59]),
        (0.43, 0.96, [0.61, 0.55]),
    ] {
        push_fan(&mut mesh, center_x, center_y, uv_origin);
    }
    push_shared_strip(&mut mesh);
    mesh
}

fn push_fan(mesh: &mut MeshData, center_x: f32, center_y: f32, uv_origin: [f32; 2]) {
    let center = push_vertex(mesh, [center_x, center_y, 0.0], uv_origin);
    let radius_x = 0.205;
    let radius_y = 0.245;
    let ring = (0..8)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / 8.0;
            let x = center_x + angle.cos() * radius_x;
            let y = center_y + angle.sin() * radius_y;
            let checker = if index % 2 == 0 { 0.39 } else { 0.09 };
            push_vertex(
                mesh,
                [x, y, 0.0],
                [
                    (uv_origin[0] + checker + index as f32 * 0.031).fract(),
                    (uv_origin[1] + 0.22 + index as f32 * 0.071).fract(),
                ],
            )
        })
        .collect::<Vec<_>>();
    for index in 0..ring.len() {
        mesh.indices
            .extend([center, ring[index], ring[(index + 1) % ring.len()]]);
    }
}

fn push_shared_strip(mesh: &mut MeshData) {
    let base = u16::try_from(mesh.positions.len()).expect("fixture vertex count fits u16");
    let columns = 9u16;
    for column in 0..columns {
        let x = -0.58 + column as f32 * 0.145;
        let y_low = 1.10 + (column % 3) as f32 * 0.008;
        let y_high = 1.42 - (column % 2) as f32 * 0.010;
        let u = (0.08 + column as f32 * 0.111).fract();
        push_vertex(mesh, [x, y_low, 0.0], [u, 0.12]);
        push_vertex(mesh, [x + 0.018, y_high, 0.0], [(u + 0.31).fract(), 0.93]);
    }
    for column in 0..columns - 1 {
        let left_low = base + column * 2;
        let left_high = left_low + 1;
        let right_low = left_low + 2;
        let right_high = left_low + 3;
        if column % 2 == 0 {
            mesh.indices.extend([
                left_low, right_low, right_high, left_low, right_high, left_high,
            ]);
        } else {
            mesh.indices.extend([
                left_low, right_low, left_high, right_low, right_high, left_high,
            ]);
        }
    }
}

fn push_vertex(mesh: &mut MeshData, position: [f32; 3], uv: [f32; 2]) -> u16 {
    let index = u16::try_from(mesh.positions.len()).expect("fixture vertex count fits u16");
    mesh.positions.push(position);
    mesh.uvs.push(uv);
    index
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
    push_f32_accessor(
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
    push_f32_accessor(
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

fn push_f32_accessor(
    bytes: &mut Vec<u8>,
    buffer_views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
    values: &[f32],
    count: usize,
    accessor_type: &str,
    min: Value,
    max: Value,
) -> usize {
    align_bytes(bytes, 4);
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

fn push_u16_accessor(
    bytes: &mut Vec<u8>,
    buffer_views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
    values: &[u16],
) -> usize {
    align_bytes(bytes, 2);
    let byte_offset = bytes.len();
    bytes.extend(values.iter().copied().flat_map(u16::to_le_bytes));
    let byte_length = bytes.len() - byte_offset;
    let buffer_view = buffer_views.len();
    buffer_views.push(json!({
        "buffer": 0,
        "byteOffset": byte_offset,
        "byteLength": byte_length,
        "target": 34963
    }));
    let accessor = accessors.len();
    accessors.push(json!({
        "bufferView": buffer_view,
        "componentType": 5123,
        "count": values.len(),
        "type": "SCALAR",
        "min": [values.iter().copied().min().unwrap_or(0)],
        "max": [values.iter().copied().max().unwrap_or(0)]
    }));
    accessor
}

fn align_bytes(bytes: &mut Vec<u8>, align: usize) {
    let padding = (align - (bytes.len() % align)) % align;
    bytes.extend(std::iter::repeat_n(0, padding));
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

fn shared_vertex_texture_png() -> Vec<u8> {
    let mut rgba = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32u8 {
        for x in 0..32u8 {
            let stripe = ((x / 3) + (y / 5)) % 2;
            let diagonal = x.wrapping_mul(11).wrapping_add(y.wrapping_mul(17));
            let color = if stripe == 0 {
                [
                    244u8.saturating_sub(y.wrapping_mul(4)),
                    26u8.saturating_add(diagonal / 4),
                    52u8.saturating_add(x.wrapping_mul(5)),
                    255,
                ]
            } else {
                [
                    20u8.saturating_add(y.wrapping_mul(6)),
                    230u8.saturating_sub(x.wrapping_mul(5)),
                    238u8.saturating_sub(diagonal / 3),
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
