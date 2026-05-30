#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
png = "0.18.1"
serde_json = "1.0.150"
---

use clap::{Parser, ValueEnum};
use png::{BitDepth, ColorType, Encoder};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-transparent-fixture",
    about = "Generate a small source-like VRM1 glTF fixture for transparent MToon render parity"
)]
struct Options {
    #[arg(
        long,
        default_value = ".external-fixtures/generated/transparent-blend.vrm.gltf"
    )]
    out: PathBuf,

    #[arg(long, value_enum, default_value_t = TransparentPalette::Subtle)]
    palette: TransparentPalette,

    #[arg(long = "case", value_enum, default_value_t = TransparentCase::Overlap)]
    fixture_case: TransparentCase,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TransparentPalette {
    Subtle,
    HighContrast,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TransparentCase {
    Overlap,
    Broad,
    TextureTransform,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse();
    if let Some(parent) = options.out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &options.out,
        format!("{}\n", fixture_json(options.palette, options.fixture_case)),
    )?;
    println!("{}", options.out.display());
    Ok(())
}

fn fixture_json(palette: TransparentPalette, fixture_case: TransparentCase) -> String {
    let mesh = mesh_buffer();
    let texture = checker_texture_png(matches!(
        fixture_case,
        TransparentCase::Broad | TransparentCase::TextureTransform
    ));
    let mesh_len = mesh.len();
    let texture_len = texture.len();
    let mut buffer = mesh;
    buffer.extend(texture);
    let materials = transparent_materials(palette, fixture_case);
    let primitives = transparent_primitives(materials.len());
    let min_filter = match fixture_case {
        TransparentCase::Overlap => 9729,
        TransparentCase::Broad | TransparentCase::TextureTransform => 9985,
    };
    let extensions_used = match fixture_case {
        TransparentCase::Overlap | TransparentCase::Broad => {
            vec!["VRMC_vrm", "VRMC_materials_mtoon"]
        }
        TransparentCase::TextureTransform => {
            vec!["VRMC_vrm", "VRMC_materials_mtoon", "KHR_texture_transform"]
        }
    };
    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs transparent render parity generator"
        },
        "extensionsUsed": extensions_used,
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": nodes(),
        "buffers": [{
            "uri": format!("data:application/octet-stream;base64,{}", base64(&buffer)),
            "byteLength": buffer.len()
        }],
        "samplers": [{
            "magFilter": 9729,
            "minFilter": min_filter,
            "wrapS": 10497,
            "wrapT": 10497
        }],
        "images": [{
            "mimeType": "image/png",
            "bufferView": 5
        }],
        "textures": [{
            "sampler": 0,
            "source": 0
        }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 48, "target": 34962 },
            { "buffer": 0, "byteOffset": 48, "byteLength": 48, "target": 34962 },
            { "buffer": 0, "byteOffset": 96, "byteLength": 32, "target": 34962 },
            { "buffer": 0, "byteOffset": 128, "byteLength": 64, "target": 34962 },
            { "buffer": 0, "byteOffset": 192, "byteLength": 12, "target": 34963 },
            { "buffer": 0, "byteOffset": mesh_len, "byteLength": texture_len }
        ],
        "accessors": [
            {
                "bufferView": 0,
                "componentType": 5126,
                "count": 4,
                "type": "VEC3",
                "min": [-1.0, 0.2, 0.0],
                "max": [1.0, 1.8, 0.0]
            },
            {
                "bufferView": 1,
                "componentType": 5126,
                "count": 4,
                "type": "VEC3",
                "min": [0.0, 0.0, 1.0],
                "max": [0.0, 0.0, 1.0]
            },
            {
                "bufferView": 2,
                "componentType": 5126,
                "count": 4,
                "type": "VEC2",
                "min": [0.0, 0.0],
                "max": [1.0, 1.0]
            },
            {
                "bufferView": 3,
                "componentType": 5126,
                "count": 4,
                "type": "VEC4",
                "min": [0.65, 0.75, 0.8, 1.0],
                "max": [1.0, 1.0, 1.0, 1.0]
            },
            {
                "bufferView": 4,
                "componentType": 5123,
                "count": 6,
                "type": "SCALAR"
            }
        ],
        "materials": materials,
        "meshes": [{
            "name": "transparent-overlap-quads",
            "primitives": primitives
        }],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Transparent Blend Fixture",
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

fn transparent_colors(palette: TransparentPalette) -> ([f32; 4], [f32; 4]) {
    match palette {
        TransparentPalette::Subtle => ([0.0, 0.85, 1.0, 0.45], [0.05, 0.75, 1.0, 0.45]),
        TransparentPalette::HighContrast => ([0.0, 0.85, 1.0, 0.45], [1.0, 0.0, 0.85, 0.45]),
    }
}

fn transparent_materials(palette: TransparentPalette, fixture_case: TransparentCase) -> Vec<Value> {
    let (front_color, zwrite_color) = transparent_colors(palette);
    match fixture_case {
        TransparentCase::Overlap => vec![
            mtoon_material(
                "transparent-textured-front",
                front_color,
                0,
                false,
                true,
                None,
            ),
            mtoon_material("transparent-zwrite", zwrite_color, 1, true, false, None),
        ],
        TransparentCase::Broad => vec![
            mtoon_material(
                "transparent-texture-alpha-early",
                [front_color[0], front_color[1], front_color[2], 0.72],
                -2,
                false,
                true,
                None,
            ),
            mtoon_material(
                "transparent-middle-queue",
                [1.0, 0.92, 0.0, 0.36],
                0,
                false,
                false,
                None,
            ),
            mtoon_material(
                "transparent-zwrite-late",
                [zwrite_color[0], zwrite_color[1], zwrite_color[2], 0.48],
                2,
                true,
                false,
                None,
            ),
            mtoon_material(
                "transparent-final-low-alpha",
                [1.0, 0.12, 0.02, 0.24],
                3,
                false,
                false,
                None,
            ),
        ],
        TransparentCase::TextureTransform => vec![
            mtoon_material(
                "transparent-transform-scale-offset",
                [0.0, 0.95, 1.0, 0.64],
                -2,
                false,
                true,
                Some(json!({
                    "offset": [0.25, 0.0],
                    "scale": [0.5, 1.0]
                })),
            ),
            mtoon_material(
                "transparent-transform-repeat-zwrite",
                [1.0, 0.0, 0.82, 0.42],
                0,
                true,
                true,
                Some(json!({
                    "offset": [0.0, 0.25],
                    "scale": [1.0, 0.5]
                })),
            ),
            mtoon_material(
                "transparent-transform-shifted",
                [1.0, 0.92, 0.0, 0.34],
                1,
                false,
                true,
                Some(json!({
                    "offset": [0.5, 0.5],
                    "scale": [0.5, 0.5]
                })),
            ),
            mtoon_material(
                "transparent-transform-solid-tail",
                [0.02, 0.25, 1.0, 0.28],
                3,
                false,
                false,
                None,
            ),
        ],
    }
}

fn transparent_primitives(material_count: usize) -> Vec<Value> {
    (0..material_count)
        .map(|material| {
            json!({
                "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2, "COLOR_0": 3 },
                "indices": 4,
                "material": material
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

fn mtoon_material(
    name: &str,
    base_color: [f32; 4],
    offset: i32,
    zwrite: bool,
    textured: bool,
    texture_transform: Option<Value>,
) -> Value {
    let mut pbr = json!({
        "baseColorFactor": base_color,
        "metallicFactor": 0.0,
        "roughnessFactor": 1.0
    });
    if textured {
        let mut texture_info = json!({ "index": 0 });
        if let Some(transform) = texture_transform {
            texture_info["extensions"] = json!({
                "KHR_texture_transform": transform
            });
        }
        pbr["baseColorTexture"] = texture_info;
    }

    json!({
        "name": name,
        "alphaMode": "BLEND",
        "doubleSided": true,
        "pbrMetallicRoughness": pbr,
        "extensions": {
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "transparentWithZWrite": zwrite,
                "renderQueueOffsetNumber": offset,
                "shadeColorFactor": [
                    base_color[0] * 0.75,
                    base_color[1] * 0.75,
                    base_color[2] * 0.75
                ],
                "shadingToonyFactor": 0.95,
                "giEqualizationFactor": 0.9,
                "outlineWidthMode": "none"
            }
        }
    })
}

fn mesh_buffer() -> Vec<u8> {
    let positions = [
        -1.0f32, 0.2, 0.0, 1.0, 0.2, 0.0, 1.0, 1.8, 0.0, -1.0, 1.8, 0.0,
    ];
    let normals = [
        0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
    ];
    let uvs = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let colors = [
        1.0f32, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.65, 0.75, 0.8, 1.0, 0.65, 0.75, 0.8, 1.0,
    ];
    let indices = [0u16, 1, 2, 0, 2, 3];

    let mut bytes = Vec::new();
    bytes.extend(positions.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(normals.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(uvs.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(colors.into_iter().flat_map(f32::to_le_bytes));
    bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
    bytes
}

fn checker_texture_png(alpha_pattern: bool) -> Vec<u8> {
    let mut rgba = [
        255, 255, 255, 255, 248, 252, 255, 255, 255, 248, 252, 255, 252, 255, 248, 255, 248, 252,
        255, 255, 255, 255, 255, 255, 252, 255, 248, 255, 255, 248, 252, 255, 255, 248, 252, 255,
        252, 255, 248, 255, 255, 255, 255, 255, 248, 252, 255, 255, 252, 255, 248, 255, 255, 248,
        252, 255, 255, 255, 255, 255, 248, 252, 255, 255,
    ];
    if alpha_pattern {
        for (pixel, alpha) in [255, 192, 128, 64].into_iter().cycle().enumerate().take(16) {
            rgba[pixel * 4 + 3] = alpha;
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
