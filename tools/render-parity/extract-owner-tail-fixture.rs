#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
glam = "0.32.1"
png = "0.18.1"
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
vrm-io = { path = "../../crates/vrm-io" }
vrm-rs = { path = "../.." }
---

use clap::{Parser, ValueEnum};
use glam::Mat4;
use png::{BitDepth, ColorType, Encoder};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use vrm_io::{
    image_data_to_rgba8, load_vrm_from_path, GltfAlphaMode, GltfPrimitiveData, LoadedVrm,
};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "extract-owner-tail-fixture",
    about = "Extract a reduced source-like VRM1 glTF fixture from owner-tail diagnostics"
)]
struct Options {
    #[arg(long)]
    fixture: PathBuf,
    #[arg(long)]
    report: PathBuf,
    #[arg(
        long,
        default_value = ".external-fixtures/generated/owner-tail-extract.vrm.gltf"
    )]
    out: PathBuf,
    #[arg(long, default_value_t = 12)]
    limit: usize,
    #[arg(long, value_enum, default_value_t = LabelSelection::Both)]
    labels: LabelSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum LabelSelection {
    Actual,
    Expected,
    Both,
}

#[derive(Clone, Debug, Deserialize)]
struct OwnerReport {
    #[serde(default)]
    top_unexplained_expected_to_actual_details: Vec<OwnerDetail>,
}

#[derive(Clone, Debug, Deserialize)]
struct OwnerDetail {
    expected: OwnerLabel,
    actual: OwnerLabel,
    count: u64,
    #[serde(default)]
    sample_pixels: Vec<OwnerPixel>,
}

#[derive(Clone, Debug, Deserialize)]
struct OwnerPixel {
    x: u32,
    y: u32,
}

#[derive(Clone, Debug, Deserialize)]
struct OwnerLabel {
    #[serde(default)]
    node_index: Option<usize>,
    #[serde(default)]
    mesh_index: Option<usize>,
    #[serde(default)]
    primitive_index: Option<usize>,
    #[serde(default)]
    mesh_name: Option<String>,
    #[serde(default)]
    material_name: Option<String>,
    pass: String,
    triangle: usize,
    indices: [u32; 3],
}

#[derive(Clone, Debug)]
struct ExtractedTriangle {
    label_side: LabelSide,
    detail_index: usize,
    count: u64,
    sample_pixels: Vec<OwnerPixel>,
    source: ResolvedTriangle,
    vertices: [[ExtractedVertex; 3]; 1],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabelSide {
    Expected,
    Actual,
}

impl LabelSide {
    fn as_str(self) -> &'static str {
        match self {
            Self::Expected => "expected",
            Self::Actual => "actual",
        }
    }
}

#[derive(Clone, Debug)]
struct ResolvedTriangle {
    node_index: usize,
    mesh_index: usize,
    primitive_index: usize,
    material: Option<usize>,
    triangle: usize,
    indices: [u32; 3],
}

#[derive(Clone, Copy, Debug, Default)]
struct ExtractedVertex {
    position: [f32; 3],
    normal: [f32; 3],
    tex_coord: [f32; 2],
    color: [f32; 4],
}

#[derive(Clone, Debug, Default)]
struct FixtureBuilder {
    bytes: Vec<u8>,
    buffer_views: Vec<Value>,
    accessors: Vec<Value>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse();
    let loaded = load_vrm_from_path(&options.fixture)?;
    let report = serde_json::from_slice::<OwnerReport>(&fs::read(&options.report)?)?;
    let extracted = extract_triangles(&loaded, &report, options.labels, options.limit)?;
    if extracted.is_empty() {
        return Err("no owner-tail triangles could be resolved from the report".into());
    }
    if let Some(parent) = options.out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &options.out,
        format!(
            "{}\n",
            fixture_json(&loaded, &options.fixture, &options.report, &extracted)?
        ),
    )?;
    println!("{} ({} triangles)", options.out.display(), extracted.len());
    Ok(())
}

fn extract_triangles(
    loaded: &LoadedVrm,
    report: &OwnerReport,
    labels: LabelSelection,
    limit: usize,
) -> Result<Vec<ExtractedTriangle>, Box<dyn std::error::Error>> {
    let world_matrices = vrm_rs::evaluated_world_matrices(loaded)?;
    let orientation = Mat4::from_rotation_y(std::f32::consts::PI);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (detail_index, detail) in report
        .top_unexplained_expected_to_actual_details
        .iter()
        .take(limit)
        .enumerate()
    {
        for (side, label) in selected_labels(labels, detail) {
            if label.pass != "base" {
                continue;
            }
            let Some(source) = resolve_label(loaded, label) else {
                continue;
            };
            let key = (
                side.as_str(),
                source.node_index,
                source.mesh_index,
                source.primitive_index,
                source.triangle,
            );
            if !seen.insert(key) {
                continue;
            }
            let vertices = bake_triangle(loaded, &world_matrices, orientation, &source)?;
            out.push(ExtractedTriangle {
                label_side: side,
                detail_index,
                count: detail.count,
                sample_pixels: detail.sample_pixels.clone(),
                source,
                vertices: [vertices],
            });
        }
    }
    Ok(out)
}

fn selected_labels(labels: LabelSelection, detail: &OwnerDetail) -> Vec<(LabelSide, &OwnerLabel)> {
    match labels {
        LabelSelection::Actual => vec![(LabelSide::Actual, &detail.actual)],
        LabelSelection::Expected => vec![(LabelSide::Expected, &detail.expected)],
        LabelSelection::Both => vec![
            (LabelSide::Expected, &detail.expected),
            (LabelSide::Actual, &detail.actual),
        ],
    }
}

fn resolve_label(loaded: &LoadedVrm, label: &OwnerLabel) -> Option<ResolvedTriangle> {
    if let (Some(node_index), Some(mesh_index), Some(primitive_index)) =
        (label.node_index, label.mesh_index, label.primitive_index)
    {
        let primitive = loaded
            .meshes
            .get(mesh_index)?
            .primitives
            .get(primitive_index)?;
        return Some(ResolvedTriangle {
            node_index,
            mesh_index,
            primitive_index,
            material: primitive.material,
            triangle: label.triangle,
            indices: label.indices,
        });
    }

    let target_mesh = label.mesh_name.as_deref().map(normalized_name)?;
    let target_material = label.material_name.as_deref().map(base_material_name);
    loaded
        .scene
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(node_index, node)| node.mesh.map(|mesh_index| (node_index, mesh_index)))
        .find_map(|(node_index, mesh_index)| {
            let mesh = loaded.meshes.get(mesh_index)?;
            (normalized_name(mesh.name.as_deref().unwrap_or("")) == target_mesh).then_some(())?;
            mesh.primitives
                .iter()
                .enumerate()
                .find_map(|(primitive_index, primitive)| {
                    let material_name = loaded
                        .material_display_name(primitive.material)
                        .map(base_material_name);
                    if material_name != target_material {
                        return None;
                    }
                    triangle_matches(primitive, label.triangle, label.indices).map(|triangle| {
                        ResolvedTriangle {
                            node_index,
                            mesh_index,
                            primitive_index,
                            material: primitive.material,
                            triangle,
                            indices: label.indices,
                        }
                    })
                })
        })
}

fn triangle_matches(
    primitive: &GltfPrimitiveData,
    triangle: usize,
    indices: [u32; 3],
) -> Option<usize> {
    primitive
        .indices
        .chunks_exact(3)
        .nth(triangle)
        .and_then(|actual| (actual == indices).then_some(triangle))
        .or_else(|| {
            primitive
                .indices
                .chunks_exact(3)
                .position(|actual| actual == indices)
        })
}

fn bake_triangle(
    loaded: &LoadedVrm,
    world_matrices: &[Mat4],
    orientation: Mat4,
    source: &ResolvedTriangle,
) -> Result<[ExtractedVertex; 3], Box<dyn std::error::Error>> {
    let node = loaded
        .scene
        .nodes
        .get(source.node_index)
        .ok_or("resolved node index is out of bounds")?;
    let mesh = loaded
        .meshes
        .get(source.mesh_index)
        .ok_or("resolved mesh index is out of bounds")?;
    let primitive = mesh
        .primitives
        .get(source.primitive_index)
        .ok_or("resolved primitive index is out of bounds")?;
    let node_world = world_matrices
        .get(source.node_index)
        .copied()
        .unwrap_or(node.world_matrix);
    let world = orientation * node_world;
    let skin_matrices = node
        .skin
        .and_then(|skin| loaded.skins.get(skin))
        .map(|skin| skin.joint_matrices(&loaded.scene, world_matrices, orientation));
    let morph_weights = mesh.weights.as_slice();
    let transformed = primitive
        .transformed_vertices(morph_weights, world, skin_matrices.as_deref())
        .ok_or("failed to transform primitive vertices")?;
    let mut vertices = [ExtractedVertex::default(); 3];
    for (out, index) in vertices.iter_mut().zip(source.indices) {
        let vertex = transformed
            .get(index as usize)
            .ok_or("resolved triangle index is out of bounds")?;
        *out = ExtractedVertex {
            position: vertex.position.to_array(),
            normal: vertex.normal.to_array(),
            tex_coord: vertex.tex_coord_0,
            color: vertex.color_0,
        };
    }
    Ok(vertices)
}

fn fixture_json(
    loaded: &LoadedVrm,
    fixture: &PathBuf,
    report: &PathBuf,
    extracted: &[ExtractedTriangle],
) -> Result<String, Box<dyn std::error::Error>> {
    let mut builder = FixtureBuilder::default();
    let mut texture_map = BTreeMap::new();
    let materials = material_map(extracted);
    let material_values = materials
        .keys()
        .map(|material| material_json(loaded, *material, &mut builder, &mut texture_map))
        .collect::<Result<Vec<_>, _>>()?;
    let normalized_positions = normalized_triangle_positions(extracted);
    let meshes = extracted
        .iter()
        .enumerate()
        .map(|(index, triangle)| {
            triangle_mesh_json(
                index,
                triangle,
                normalized_positions[index],
                materials
                    .get(&triangle.source.material)
                    .copied()
                    .expect("material map should contain every extracted material"),
                &mut builder,
            )
        })
        .collect::<Vec<_>>();
    let image_sources = texture_map.keys().copied().collect::<Vec<_>>();
    let textures = image_sources
        .iter()
        .enumerate()
        .map(|(index, _image)| json!({ "sampler": 0, "source": index }))
        .collect::<Vec<_>>();
    let images = image_sources
        .iter()
        .map(|image| {
            json!({
                "mimeType": "image/png",
                "bufferView": texture_map
                    .get(image)
                    .copied()
                    .expect("image source must have a buffer view")
            })
        })
        .collect::<Vec<_>>();
    let nodes = extracted
        .iter()
        .enumerate()
        .map(|(index, triangle)| {
            json!({
                "name": format!(
                    "owner_tail_{}_{}_{}",
                    index,
                    triangle.label_side.as_str(),
                    source_name(loaded, &triangle.source)
                ),
                "mesh": index
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_string_pretty(&json!({
        "asset": {
            "version": "2.0",
            "generator": "vrm-rs owner-tail extraction generator"
        },
        "extensionsUsed": [
            "VRMC_vrm",
            "VRMC_materials_mtoon"
        ],
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
        "buffers": [{
            "uri": format!("data:application/octet-stream;base64,{}", base64(&builder.bytes)),
            "byteLength": builder.bytes.len()
        }],
        "samplers": [{
            "magFilter": 9729,
            "minFilter": 9729,
            "wrapS": 10497,
            "wrapT": 10497
        }],
        "images": images,
        "textures": textures,
        "bufferViews": builder.buffer_views,
        "accessors": builder.accessors,
        "materials": material_values,
        "meshes": meshes,
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {
                    "name": "vrm-rs Owner Tail Extract",
                    "authors": ["vrm-rs"],
                    "licenseUrl": "https://vrm.dev/licenses/1.0/",
                    "otherLicenseUrl": "https://github.com/Sanzentyo/vrm-rs"
                },
                "humanoid": { "humanBones": human_bones() }
            }
        },
        "extras": {
            "vrmRsOwnerTailExtract": {
                "sourceFixture": fixture.to_string_lossy(),
                "sourceReport": report.to_string_lossy(),
                "triangleCount": extracted.len(),
                "triangles": extracted.iter().map(|triangle| triangle_extra(loaded, triangle)).collect::<Vec<_>>()
            }
        }
    }))
    .map_err(Into::into)
}

fn triangle_mesh_json(
    index: usize,
    triangle: &ExtractedTriangle,
    positions: [[f32; 3]; 3],
    material: usize,
    builder: &mut FixtureBuilder,
) -> Value {
    let vertices = triangle.vertices[0];
    let normals = vertices.map(|vertex| vertex.normal);
    let uvs = vertices.map(|vertex| vertex.tex_coord);
    let colors = vertices.map(|vertex| vertex.color);
    let position = builder.push_vec3_accessor(&positions);
    let normal = builder.push_vec3_accessor(&normals);
    let uv = builder.push_vec2_accessor(&uvs);
    let color = builder.push_vec4_accessor(&colors);
    let indices = builder.push_u32_accessor(&[0, 1, 2]);
    json!({
        "name": format!("owner-tail-triangle-{index}"),
        "primitives": [{
            "attributes": {
                "POSITION": position,
                "NORMAL": normal,
                "TEXCOORD_0": uv,
                "COLOR_0": color
            },
            "indices": indices,
            "material": material
        }]
    })
}

fn material_map(extracted: &[ExtractedTriangle]) -> BTreeMap<Option<usize>, usize> {
    extracted
        .iter()
        .map(|triangle| triangle.source.material)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(next, source)| (source, next))
        .collect()
}

fn material_json(
    loaded: &LoadedVrm,
    material: Option<usize>,
    builder: &mut FixtureBuilder,
    texture_map: &mut BTreeMap<usize, usize>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let gltf = material.and_then(|index| loaded.gltf_materials.get(index));
    let name = material
        .and_then(|index| loaded.material_display_name(Some(index)))
        .unwrap_or("owner-tail-material");
    let base_color = gltf
        .map(|material| material.base_color_factor)
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let texture = if let Some(texture) = gltf.and_then(|material| material.base_color_texture) {
        texture_json(loaded, texture, builder, texture_map)?
    } else {
        None
    };
    let mut pbr = json!({
        "baseColorFactor": base_color,
        "metallicFactor": gltf.map_or(0.0, |material| material.metallic_factor),
        "roughnessFactor": gltf.map_or(1.0, |material| material.roughness_factor)
    });
    if let Some(texture) = texture {
        pbr["baseColorTexture"] = texture;
    }
    Ok(json!({
        "name": name,
        "alphaMode": gltf.map_or("OPAQUE", |material| alpha_mode_str(material.alpha_mode)),
        "alphaCutoff": gltf.and_then(|material| material.alpha_cutoff).unwrap_or(0.5),
        "doubleSided": true,
        "pbrMetallicRoughness": pbr,
        "extensions": {
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "shadeColorFactor": [base_color[0], base_color[1], base_color[2]],
                "shadingShiftFactor": 0.0,
                "shadingToonyFactor": 1.0,
                "giEqualizationFactor": 0.0,
                "outlineWidthMode": "none",
                "outlineWidthFactor": 0.0,
                "outlineColorFactor": [0.0, 0.0, 0.0],
                "outlineLightingMixFactor": 0.0
            }
        }
    }))
}

fn normalized_triangle_positions(extracted: &[ExtractedTriangle]) -> Vec<[[f32; 3]; 3]> {
    let points = extracted
        .iter()
        .flat_map(|triangle| triangle.vertices[0].map(|vertex| vertex.position))
        .collect::<Vec<_>>();
    if points.is_empty() {
        return Vec::new();
    }
    let (min, max) = bounds3(&points);
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let extent_x = (max[0] - min[0]).abs().max(0.001);
    let extent_y = (max[1] - min[1]).abs().max(0.001);
    let scale = (1.70 / extent_x).min(1.25 / extent_y).min(32.0);
    extracted
        .iter()
        .map(|triangle| {
            triangle.vertices[0].map(|vertex| {
                [
                    (vertex.position[0] - center[0]) * scale,
                    1.0 + (vertex.position[1] - center[1]) * scale,
                    (vertex.position[2] - center[2]) * scale * 0.25,
                ]
            })
        })
        .collect()
}

fn bounds3(points: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let min = (0..3)
        .map(|component| {
            points
                .iter()
                .map(|point| point[component])
                .fold(f32::INFINITY, f32::min)
        })
        .collect::<Vec<_>>();
    let max = (0..3)
        .map(|component| {
            points
                .iter()
                .map(|point| point[component])
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect::<Vec<_>>();
    ([min[0], min[1], min[2]], [max[0], max[1], max[2]])
}

fn texture_json(
    loaded: &LoadedVrm,
    texture: usize,
    builder: &mut FixtureBuilder,
    texture_map: &mut BTreeMap<usize, usize>,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let Some(texture_data) = loaded.textures.get(texture) else {
        return Ok(None);
    };
    if !texture_map.contains_key(&texture_data.image) {
        let Some(image) = loaded.images.get(texture_data.image) else {
            return Ok(None);
        };
        let rgba = image_data_to_rgba8(image)?;
        let png = encode_png(image.width, image.height, &rgba)?;
        let view = builder.push_bytes(&png, None);
        texture_map.insert(texture_data.image, view);
    }
    let texture_index = texture_map
        .keys()
        .position(|image| *image == texture_data.image)
        .unwrap_or(0);
    Ok(Some(json!({ "index": texture_index })))
}

fn alpha_mode_str(mode: GltfAlphaMode) -> &'static str {
    match mode {
        GltfAlphaMode::Opaque => "OPAQUE",
        GltfAlphaMode::Mask => "MASK",
        GltfAlphaMode::Blend => "BLEND",
    }
}

fn triangle_extra(loaded: &LoadedVrm, triangle: &ExtractedTriangle) -> Value {
    json!({
        "labelSide": triangle.label_side.as_str(),
        "detailIndex": triangle.detail_index,
        "count": triangle.count,
        "samplePixels": triangle.sample_pixels.iter().map(|pixel| json!({ "x": pixel.x, "y": pixel.y })).collect::<Vec<_>>(),
        "nodeIndex": triangle.source.node_index,
        "nodeName": loaded.scene.nodes.get(triangle.source.node_index).and_then(|node| node.name.as_deref()),
        "meshIndex": triangle.source.mesh_index,
        "meshName": loaded.meshes.get(triangle.source.mesh_index).and_then(|mesh| mesh.name.as_deref()),
        "primitiveIndex": triangle.source.primitive_index,
        "materialIndex": triangle.source.material,
        "materialName": loaded.material_display_name(triangle.source.material),
        "triangle": triangle.source.triangle,
        "indices": triangle.source.indices
    })
}

impl FixtureBuilder {
    fn push_vec2_accessor(&mut self, values: &[[f32; 2]]) -> usize {
        let flat = values
            .iter()
            .flat_map(|value| value.iter().copied())
            .collect::<Vec<_>>();
        let (min, max) = min_max_vec2(values);
        self.push_f32_accessor(&flat, values.len(), "VEC2", min, max)
    }

    fn push_vec3_accessor(&mut self, values: &[[f32; 3]]) -> usize {
        let flat = values
            .iter()
            .flat_map(|value| value.iter().copied())
            .collect::<Vec<_>>();
        let (min, max) = min_max_vec3(values);
        self.push_f32_accessor(&flat, values.len(), "VEC3", min, max)
    }

    fn push_vec4_accessor(&mut self, values: &[[f32; 4]]) -> usize {
        let flat = values
            .iter()
            .flat_map(|value| value.iter().copied())
            .collect::<Vec<_>>();
        let (min, max) = min_max_vec4(values);
        self.push_f32_accessor(&flat, values.len(), "VEC4", min, max)
    }

    fn push_f32_accessor(
        &mut self,
        values: &[f32],
        count: usize,
        accessor_type: &str,
        min: Value,
        max: Value,
    ) -> usize {
        let bytes = values
            .iter()
            .copied()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let buffer_view = self.push_bytes(&bytes, Some(34962));
        let accessor = self.accessors.len();
        self.accessors.push(json!({
            "bufferView": buffer_view,
            "componentType": 5126,
            "count": count,
            "type": accessor_type,
            "min": min,
            "max": max
        }));
        accessor
    }

    fn push_u32_accessor(&mut self, values: &[u32]) -> usize {
        let bytes = values
            .iter()
            .copied()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let buffer_view = self.push_bytes(&bytes, Some(34963));
        let accessor = self.accessors.len();
        self.accessors.push(json!({
            "bufferView": buffer_view,
            "componentType": 5125,
            "count": values.len(),
            "type": "SCALAR",
            "min": [values.iter().copied().min().unwrap_or(0)],
            "max": [values.iter().copied().max().unwrap_or(0)]
        }));
        accessor
    }

    fn push_bytes(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.bytes.len() % 4 != 0 {
            self.bytes.push(0);
        }
        let byte_offset = self.bytes.len();
        self.bytes.extend(bytes);
        let buffer_view = self.buffer_views.len();
        let mut view = json!({
            "buffer": 0,
            "byteOffset": byte_offset,
            "byteLength": bytes.len()
        });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.buffer_views.push(view);
        buffer_view
    }
}

fn min_max_vec2(values: &[[f32; 2]]) -> (Value, Value) {
    min_max_components::<2>(values)
}

fn min_max_vec3(values: &[[f32; 3]]) -> (Value, Value) {
    min_max_components::<3>(values)
}

fn min_max_vec4(values: &[[f32; 4]]) -> (Value, Value) {
    min_max_components::<4>(values)
}

fn min_max_components<const N: usize>(values: &[[f32; N]]) -> (Value, Value) {
    let min = (0..N)
        .map(|component| {
            values
                .iter()
                .map(|value| value[component])
                .fold(f32::INFINITY, f32::min)
        })
        .collect::<Vec<_>>();
    let max = (0..N)
        .map(|component| {
            values
                .iter()
                .map(|value| value[component])
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect::<Vec<_>>();
    (json!(min), json!(max))
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, png::EncodingError> {
    let mut bytes = Vec::new();
    {
        let mut cursor = Cursor::new(&mut bytes);
        let mut encoder = Encoder::new(&mut cursor, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(bytes)
}

fn source_name(loaded: &LoadedVrm, source: &ResolvedTriangle) -> String {
    loaded
        .meshes
        .get(source.mesh_index)
        .and_then(|mesh| mesh.name.as_deref())
        .unwrap_or("mesh")
        .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
}

fn normalized_name(name: &str) -> String {
    let Some((base, suffix)) = name.rsplit_once('_') else {
        return name.to_owned();
    };
    if suffix.chars().all(|ch| ch.is_ascii_digit()) {
        base.to_owned()
    } else {
        name.to_owned()
    }
}

fn base_material_name(name: &str) -> String {
    name.strip_suffix(" (Outline)").unwrap_or(name).to_owned()
}

fn human_bones() -> Map<String, Value> {
    [
        ("hips", 0),
        ("spine", 0),
        ("head", 0),
        ("leftUpperLeg", 0),
        ("leftLowerLeg", 0),
        ("leftFoot", 0),
        ("rightUpperLeg", 0),
        ("rightLowerLeg", 0),
        ("rightFoot", 0),
        ("leftUpperArm", 0),
        ("leftLowerArm", 0),
        ("leftHand", 0),
        ("rightUpperArm", 0),
        ("rightLowerArm", 0),
        ("rightHand", 0),
    ]
    .into_iter()
    .map(|(name, node)| (name.to_owned(), json!({ "node": node })))
    .collect()
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
