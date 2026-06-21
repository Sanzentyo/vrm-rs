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

const MATERIAL_COUNT: usize = 9;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-gltf-pbr-fixture",
    about = "Generate a source-like VRM1 glTF fixture for non-MToon glTF PBR fallback parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/gltf-pbr.vrm.gltf"
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
            "generator": "vrm-rs glTF PBR render parity generator"
        },
        "extensionsUsed": [
            "VRMC_vrm",
            "KHR_materials_emissive_strength",
            "KHR_materials_unlit"
        ],
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": nodes(),
        "buffers": [{
            "uri": format!("data:application/octet-stream;base64,{}", base64(&buffer)),
            "byteLength": buffer.len()
        }],
        "samplers": [
            {
                "magFilter": 9729,
                "minFilter": 9729,
                "wrapS": 10497,
                "wrapT": 10497
            },
            {
                "magFilter": 9728,
                "minFilter": 9728,
                "wrapS": 33071,
                "wrapT": 33071
            },
            {
                "magFilter": 9729,
                "minFilter": 9985,
                "wrapS": 10497,
                "wrapT": 10497
            }
        ],
        "images": image_views
            .iter()
            .enumerate()
            .map(|(index, _)| json!({
                "mimeType": "image/png",
                "bufferView": 3 + MATERIAL_COUNT + index
            }))
            .collect::<Vec<_>>(),
        "textures": (0..image_views.len())
            .map(|index| json!({
                "sampler": match index {
                    0 => 0,
                    2 => 2,
                    3 => 2,
                    _ => 1,
                },
                "source": index
            }))
            .collect::<Vec<_>>(),
        "bufferViews": buffer_views(mesh_len, image_views),
        "accessors": accessors(),
        "materials": pbr_materials(),
        "meshes": [{
            "name": "gltf-pbr-grid",
            "primitives": (0..MATERIAL_COUNT)
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
                    "name": "vrm-rs glTF PBR Fixture",
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
    let vertex_count = MATERIAL_COUNT * 4;
    let position_len = vertex_count * 3 * 4;
    let normal_len = vertex_count * 3 * 4;
    let uv_len = vertex_count * 2 * 4;
    let index_len = 6 * 2;
    let mut offset = 0;
    let mut views = Vec::new();
    for (byte_length, target) in [(position_len, 34962), (normal_len, 34962), (uv_len, 34962)] {
        views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": byte_length,
            "target": target
        }));
        offset += byte_length;
    }
    for _ in 0..MATERIAL_COUNT {
        views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": index_len,
            "target": 34963
        }));
        offset += index_len;
    }
    debug_assert_eq!(offset, mesh_len);
    views.extend(image_views);
    views
}

fn accessors() -> Vec<Value> {
    let vertex_count = MATERIAL_COUNT * 4;
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
            "min": [-0.70710677, -0.4082483, 0.57735026],
            "max": [0.70710677, 0.4082483, 1.0]
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
    accessors.extend((0..MATERIAL_COUNT).map(|index| {
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

fn pbr_materials() -> Vec<Value> {
    [
        PbrMaterialSpec {
            name: "pbr-base-red",
            base_color: [0.90, 0.18, 0.10, 1.0],
            ..PbrMaterialSpec::default()
        },
        PbrMaterialSpec {
            name: "pbr-texture-gradient",
            base_texture: Some(0),
            ..PbrMaterialSpec::default()
        },
        PbrMaterialSpec {
            name: "pbr-rough-blue",
            base_color: [0.12, 0.36, 0.95, 1.0],
            roughness: 0.35,
            ..PbrMaterialSpec::default()
        },
        PbrMaterialSpec {
            name: "pbr-metal-gold",
            base_color: [1.0, 0.72, 0.23, 1.0],
            metallic: 1.0,
            roughness: 0.28,
            ..PbrMaterialSpec::default()
        },
        PbrMaterialSpec {
            name: "pbr-emissive-strength",
            base_color: [0.02, 0.02, 0.03, 1.0],
            emissive: [0.16, 0.32, 0.72],
            emissive_strength: Some(1.75),
            ..PbrMaterialSpec::default()
        },
        PbrMaterialSpec {
            name: "pbr-occlusion",
            base_color: [0.62, 0.62, 0.62, 1.0],
            base_texture: Some(0),
            occlusion_texture: Some(1),
            ..PbrMaterialSpec::default()
        },
        PbrMaterialSpec {
            name: "pbr-unlit",
            base_color: [0.18, 0.86, 0.52, 1.0],
            base_texture: Some(0),
            unlit: true,
            ..PbrMaterialSpec::default()
        },
        PbrMaterialSpec {
            name: "pbr-texture-factor",
            base_color: [0.42, 0.78, 1.0, 1.0],
            base_texture: Some(0),
            roughness: 0.82,
            emissive: [0.02, 0.01, 0.0],
            ..PbrMaterialSpec::default()
        },
        PbrMaterialSpec {
            name: "pbr-backpack-like",
            base_texture: Some(3),
            normal_texture: Some(2),
            roughness: 0.657,
            double_sided: false,
            ..PbrMaterialSpec::default()
        },
    ]
    .into_iter()
    .map(pbr_material)
    .collect()
}

#[derive(Clone, Copy, Debug)]
struct PbrMaterialSpec {
    name: &'static str,
    base_color: [f32; 4],
    base_texture: Option<usize>,
    normal_texture: Option<usize>,
    metallic: f32,
    roughness: f32,
    emissive: [f32; 3],
    emissive_strength: Option<f32>,
    occlusion_texture: Option<usize>,
    unlit: bool,
    double_sided: bool,
}

impl Default for PbrMaterialSpec {
    fn default() -> Self {
        Self {
            name: "pbr",
            base_color: [1.0, 1.0, 1.0, 1.0],
            base_texture: None,
            normal_texture: None,
            metallic: 0.0,
            roughness: 1.0,
            emissive: [0.0, 0.0, 0.0],
            emissive_strength: None,
            occlusion_texture: None,
            unlit: false,
            double_sided: true,
        }
    }
}

fn pbr_material(spec: PbrMaterialSpec) -> Value {
    let mut material = json!({
        "name": spec.name,
        "alphaMode": "OPAQUE",
        "doubleSided": spec.double_sided,
        "pbrMetallicRoughness": {
            "baseColorFactor": spec.base_color,
            "metallicFactor": spec.metallic,
            "roughnessFactor": spec.roughness
        },
        "emissiveFactor": spec.emissive
    });
    if let Some(texture) = spec.base_texture {
        material["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": texture });
    }
    if let Some(texture) = spec.normal_texture {
        material["normalTexture"] = json!({ "index": texture, "scale": 1.0 });
    }
    if let Some(texture) = spec.occlusion_texture {
        material["occlusionTexture"] = json!({ "index": texture, "strength": 0.65 });
    }
    if spec.emissive_strength.is_some() || spec.unlit {
        material["extensions"] = json!({});
    }
    if let Some(strength) = spec.emissive_strength {
        material["extensions"]["KHR_materials_emissive_strength"] =
            json!({ "emissiveStrength": strength });
    }
    if spec.unlit {
        material["extensions"]["KHR_materials_unlit"] = json!({});
    }
    material
}

fn mesh_buffer() -> Vec<u8> {
    let quads = [
        (-0.82f32, -0.44f32, 0.25f32, 0.64f32),
        (-0.27, 0.11, 0.25, 0.64),
        (0.28, 0.66, 0.25, 0.64),
        (-0.82, -0.44, 0.83, 1.22),
        (-0.27, 0.11, 0.83, 1.22),
        (0.28, 0.66, 0.83, 1.22),
        (-0.82, -0.44, 1.41, 1.75),
        (-0.27, 0.11, 1.41, 1.75),
        (0.28, 0.66, 1.41, 1.75),
    ];
    let normal_vectors = [
        [0.0f32, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [-0.70710677, 0.0, 0.70710677],
        [0.70710677, 0.0, 0.70710677],
        [0.0, 0.4082483, 0.9128709],
        [0.0, -0.4082483, 0.9128709],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
    ];
    let positions = quads
        .iter()
        .flat_map(|(left, right, bottom, top)| {
            [
                *left, *bottom, 0.0, *right, *bottom, 0.0, *right, *top, 0.0, *left, *top, 0.0,
            ]
        })
        .collect::<Vec<_>>();
    let normals = normal_vectors
        .iter()
        .flat_map(|normal| normal.repeat(4))
        .collect::<Vec<_>>();
    let uvs = (0..MATERIAL_COUNT)
        .flat_map(|_| [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])
        .collect::<Vec<_>>();

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    for primitive in 0..MATERIAL_COUNT as u16 {
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
    vec![gradient_png(), occlusion_png(), normal_png(), backpack_png()]
}

fn gradient_png() -> Vec<u8> {
    let rgba = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| {
                let u = x as f32 / 3.0;
                let v = y as f32 / 3.0;
                [
                    (48.0 + 160.0 * u).round() as u8,
                    (64.0 + 120.0 * v).round() as u8,
                    (220.0 - 80.0 * u + 20.0 * v).round() as u8,
                    255,
                ]
            })
        })
        .collect::<Vec<_>>();
    png_rgba(4, 4, &rgba)
}

fn occlusion_png() -> Vec<u8> {
    let rgba = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| {
                let value = if (x + y) % 2 == 0 { 96 } else { 224 };
                [value, value, value, 255]
            })
        })
        .collect::<Vec<_>>();
    png_rgba(4, 4, &rgba)
}

fn normal_png() -> Vec<u8> {
    let rgba = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| {
                let red = match x {
                    0 => 84,
                    1 => 112,
                    2 => 144,
                    _ => 172,
                };
                let green = match y {
                    0 => 172,
                    1 => 144,
                    2 => 112,
                    _ => 84,
                };
                [red, green, 255, 255]
            })
        })
        .collect::<Vec<_>>();
    png_rgba(4, 4, &rgba)
}

fn backpack_png() -> Vec<u8> {
    let rgba = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| {
                let u = x as f32 / 3.0;
                let v = y as f32 / 3.0;
                [
                    (72.0 + 92.0 * u + 18.0 * v).round() as u8,
                    (80.0 + 72.0 * v + 12.0 * u).round() as u8,
                    (92.0 + 100.0 * (1.0 - u) + 20.0 * v).round() as u8,
                    255,
                ]
            })
        })
        .collect::<Vec<_>>();
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
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | chunk[2] as u32;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push(TABLE[(n & 0x3f) as usize] as char);
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let b0 = rem[0] as u32;
        let b1 = rem.get(1).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8);
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if rem.len() == 2 {
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}
