//! Offscreen wgpu render capture for render-parity experiments.
//!
//! This example intentionally keeps renderer policy small: it loads real glTF
//! primitive buffers from `vrm-io`, draws them with a fixed camera/light setup,
//! and writes the same RGBA JSON artifact consumed by
//! `tools/render-parity/compare-psnr.mjs`.

#[path = "common/render_capture_scene.rs"]
mod render_capture_scene;

use bytemuck::{Pod, Zeroable};
use clap::{Parser, ValueEnum};
use glam::{Mat4, Vec2, Vec3, Vec4};
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use vrm_core::{MtoonAlphaMode, MtoonCullMode, OutlineWidthMode, TextureTransform2d, VrmKind};
use vrm_io::{
    GltfAlphaMode, GltfPrimitiveData, GltfSkinData, ImageData, ImageFormat, LoadedVrm,
    load_vrm_from_path,
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 4],
    tex_coord: [f32; 2],
    color: [f32; 4],
    shade_color: [f32; 4],
    shading: [f32; 4],
    emissive: [f32; 4],
    matcap_factor: [f32; 4],
    rim_color: [f32; 4],
    rim_params: [f32; 4],
    outline_color: [f32; 4],
    alpha_mode: f32,
    normal_scale: f32,
    double_sided: f32,
    _padding: f32,
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 15] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4, 12 => Float32, 13 => Float32, 14 => Float32];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    view_projection: [[f32; 4]; 4],
    light_dir: [f32; 4],
    camera_pos: [f32; 4],
    mtoon_lighting: [f32; 4],
}

#[derive(Clone, Debug, Parser)]
struct CaptureOptions {
    #[arg(long)]
    fixture: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    png_out: Option<PathBuf>,
    #[arg(long, default_value_t = 512)]
    width: u32,
    #[arg(long, default_value_t = 512)]
    height: u32,
    #[arg(long, default_value_t = 1.0)]
    camera_y: f32,
    #[arg(long, default_value_t = 5.0)]
    camera_z: f32,
    #[arg(long, default_value_t = 1.0)]
    target_y: f32,
    #[arg(long, default_value_t = 0.78)]
    mtoon_exposure: f32,
    #[arg(long, default_value_t = 0.12)]
    mtoon_ambient_base: f32,
    #[arg(long, default_value_t = 0.20)]
    mtoon_ambient_gi_scale: f32,
    #[arg(long, default_value_t = 0.03183099)]
    pbr_ambient: f32,
    #[arg(long, default_value_t = 1.0)]
    direct_light_scale: f32,
    #[arg(long, value_enum, default_value_t = MtoonLightAccumulation::Tuned)]
    mtoon_light_accumulation: MtoonLightAccumulation,
    #[arg(long, default_value_t = 0.0)]
    mtoon_time: f32,
    #[arg(long, value_enum, default_value_t = CaptureBackground::OpaqueBlack)]
    background: CaptureBackground,
}

#[derive(Clone, Debug)]
struct MeshDrawData {
    primitives: Vec<DrawPrimitive>,
}

#[derive(Clone, Debug)]
struct DrawPrimitive {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    images: MaterialImages,
    uv_transforms: MaterialUvTransforms,
    material_extra: MaterialExtraUniform,
    policy: MaterialPolicy,
}

struct GpuPrimitive {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    texture_bind_group_index: usize,
    pipeline_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MaterialPolicy {
    render_order: i32,
    cull_mode: CaptureCullMode,
    alpha_mode: CaptureAlphaMode,
    depth_write: bool,
    blend: bool,
    alpha_cutoff: f32,
}

impl Default for MaterialPolicy {
    fn default() -> Self {
        Self {
            render_order: 2000,
            cull_mode: CaptureCullMode::Back,
            alpha_mode: CaptureAlphaMode::Opaque,
            depth_write: true,
            blend: false,
            alpha_cutoff: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum MtoonLightAccumulation {
    Tuned,
    ThreeVrm,
}

impl MtoonLightAccumulation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tuned => "tuned",
            Self::ThreeVrm => "three-vrm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PipelineKey {
    cull_mode: CaptureCullMode,
    depth_write: bool,
    blend: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CaptureCullMode {
    Off,
    Front,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureAlphaMode {
    Opaque,
    Mask,
    Blend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum CaptureBackground {
    OpaqueBlack,
    Transparent,
}

impl CaptureBackground {
    fn clear_color(self) -> wgpu::Color {
        match self {
            Self::OpaqueBlack => wgpu::Color::BLACK,
            Self::Transparent => wgpu::Color::TRANSPARENT,
        }
    }
}

impl From<MaterialUvTransforms> for MaterialUvUniform {
    fn from(transforms: MaterialUvTransforms) -> Self {
        Self {
            base_transform: uv_transform_uniform(transforms.base),
            shade_transform: uv_transform_uniform(transforms.shade),
            shading_shift_transform: uv_transform_uniform(transforms.shading_shift),
            normal_transform: uv_transform_uniform(transforms.normal),
            matcap_transform: uv_transform_uniform(transforms.matcap),
            rim_transform: uv_transform_uniform(transforms.rim),
            emissive_transform: uv_transform_uniform(transforms.emissive),
            occlusion_transform: uv_transform_uniform(transforms.occlusion),
            uv_animation_mask_transform: uv_transform_uniform(transforms.uv_animation_mask),
            rotation_a: [
                uv_rotation_uniform(transforms.base),
                uv_rotation_uniform(transforms.shade),
                uv_rotation_uniform(transforms.shading_shift),
                uv_rotation_uniform(transforms.normal),
            ],
            rotation_b: [
                uv_rotation_uniform(transforms.rim),
                uv_rotation_uniform(transforms.emissive),
                uv_rotation_uniform(transforms.uv_animation_mask),
                uv_rotation_uniform(transforms.matcap),
            ],
            uv_animation: [
                transforms.uv_animation_scroll[0],
                transforms.uv_animation_scroll[1],
                transforms.uv_animation_rotation,
                uv_rotation_uniform(transforms.occlusion),
            ],
        }
    }
}

fn uv_transform_uniform(transform: Option<TextureTransform2d>) -> [f32; 4] {
    let Some(transform) =
        transform.filter(|transform| transform.tex_coord.is_none_or(|tex_coord| tex_coord == 0))
    else {
        return [0.0, 0.0, 1.0, 1.0];
    };
    [
        transform.offset[0],
        transform.offset[1],
        transform.scale[0],
        transform.scale[1],
    ]
}

fn uv_rotation_uniform(transform: Option<TextureTransform2d>) -> f32 {
    transform
        .filter(|transform| transform.tex_coord.is_none_or(|tex_coord| tex_coord == 0))
        .map_or(0.0, |transform| transform.rotation)
}

struct TextureBindGroup {
    bind_group: wgpu::BindGroup,
    _uv_uniform_buffer: wgpu::Buffer,
    _material_extra_buffer: wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MaterialUvUniform {
    base_transform: [f32; 4],
    shade_transform: [f32; 4],
    shading_shift_transform: [f32; 4],
    normal_transform: [f32; 4],
    matcap_transform: [f32; 4],
    rim_transform: [f32; 4],
    emissive_transform: [f32; 4],
    occlusion_transform: [f32; 4],
    uv_animation_mask_transform: [f32; 4],
    rotation_a: [f32; 4],
    rotation_b: [f32; 4],
    uv_animation: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MaterialExtraUniform {
    flags: [f32; 4],
    pbr_params: [f32; 4],
}

struct TextureResource {
    image: Option<usize>,
    view: wgpu::TextureView,
}

struct TextureResourceTables<'a> {
    color: &'a [TextureResource],
    normal: &'a [TextureResource],
    indices: &'a HashMap<usize, usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct MaterialImages {
    base: Option<usize>,
    shade: Option<usize>,
    shading_shift: Option<usize>,
    normal: Option<usize>,
    matcap: Option<usize>,
    rim: Option<usize>,
    emissive: Option<usize>,
    occlusion: Option<usize>,
    uv_animation_mask: Option<usize>,
}

struct TextureUpload<'a> {
    image: Option<usize>,
    width: u32,
    height: u32,
    rgba: &'a [u8],
}

struct CpuRgbaImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default)]
struct MaterialUvTransforms {
    base: Option<TextureTransform2d>,
    shade: Option<TextureTransform2d>,
    shading_shift: Option<TextureTransform2d>,
    normal: Option<TextureTransform2d>,
    matcap: Option<TextureTransform2d>,
    rim: Option<TextureTransform2d>,
    outline_width: Option<TextureTransform2d>,
    emissive: Option<TextureTransform2d>,
    occlusion: Option<TextureTransform2d>,
    uv_animation_mask: Option<TextureTransform2d>,
    uv_animation_scroll: [f32; 2],
    uv_animation_rotation: f32,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = CaptureOptions::parse();
    let loaded = load_vrm_from_path(&options.fixture)?;
    let mesh = mesh_draw_data(&loaded, &options)?;
    let rgba = pollster::block_on(render_capture(&loaded, &mesh, &options))?;

    write_rgba_json(&options, &rgba)?;
    if let Some(path) = &options.png_out {
        write_png(path, options.width, options.height, &rgba)?;
    }
    Ok(())
}

fn mesh_draw_data(
    loaded: &LoadedVrm,
    options: &CaptureOptions,
) -> Result<MeshDrawData, Box<dyn Error>> {
    let mut primitives = Vec::new();
    let world_matrices = render_capture_scene::runtime_world_matrices(loaded)?;

    for (node_index, node) in loaded.scene.nodes.iter().enumerate() {
        let Some(mesh_index) = node.mesh else {
            continue;
        };
        let Some(mesh) = loaded.meshes.get(mesh_index) else {
            continue;
        };
        let orientation = Mat4::from_rotation_y(std::f32::consts::PI);
        let node_world = world_matrices
            .get(node_index)
            .copied()
            .unwrap_or(node.world_matrix);
        let world = orientation * node_world;
        let skin_matrices = node
            .skin
            .and_then(|skin| loaded.skins.get(skin))
            .map(|skin| skin_matrices(loaded, skin, &world_matrices, orientation));
        for primitive in &mesh.primitives {
            let surface =
                draw_primitive(loaded, primitive, world, skin_matrices.as_deref(), options)?;
            primitives.push(surface.clone());
            if let Some(outline) = outline_primitive(loaded, primitive, &surface, options) {
                primitives.push(outline);
            }
        }
    }

    if primitives.is_empty() {
        return Err("no drawable mesh primitives were found".into());
    }
    primitives.sort_by_key(|primitive| primitive.policy.render_order);
    Ok(MeshDrawData { primitives })
}

fn outline_primitive(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    surface: &DrawPrimitive,
    options: &CaptureOptions,
) -> Option<DrawPrimitive> {
    let material = primitive
        .material
        .and_then(|index| loaded.model().document().materials.get(index))?;
    let mtoon = material.mtoon.as_ref()?;
    if !mtoon.outline_enabled() {
        return None;
    }
    let outline_color = [
        mtoon.outline_color_factor[0],
        mtoon.outline_color_factor[1],
        mtoon.outline_color_factor[2],
        mtoon.outline_lighting_mix_factor,
    ];
    let width_texture = mtoon
        .textures
        .outline_width_multiply_texture
        .and_then(|texture| sampled_image_for_texture(loaded, texture.0));
    let uv_transforms = surface.uv_transforms;
    let width = mtoon.outline_width_factor;
    let outline_scale = OutlineScale::new(mtoon.outline_width_mode, options);
    let vertices = surface
        .vertices
        .iter()
        .map(|vertex| {
            let normal = Vec3::from_array(vertex.normal).normalize_or_zero();
            let outline_coord = transform_uv(vertex.tex_coord, uv_transforms.outline_width);
            let width = width
                * width_texture
                    .as_ref()
                    .map(|image| image.sample_green(outline_coord))
                    .unwrap_or(1.0);
            let mut vertex = *vertex;
            let position = Vec3::from_array(vertex.position);
            vertex.position = (position + normal * width * outline_scale.at(position)).to_array();
            vertex.outline_color = outline_color;
            vertex.alpha_mode = alpha_mode_code(CaptureAlphaMode::Opaque);
            vertex.double_sided = 0.0;
            vertex
        })
        .collect();
    Some(DrawPrimitive {
        vertices,
        indices: surface.indices.clone(),
        images: surface.images,
        uv_transforms: surface.uv_transforms,
        material_extra: surface.material_extra,
        policy: MaterialPolicy {
            render_order: surface.policy.render_order.saturating_add(1),
            cull_mode: CaptureCullMode::Front,
            alpha_mode: CaptureAlphaMode::Opaque,
            depth_write: true,
            blend: false,
            alpha_cutoff: 0.5,
        },
    })
}

#[derive(Clone, Copy, Debug)]
struct OutlineScale {
    mode: OutlineWidthMode,
    view: Mat4,
    projection_y: f32,
}

impl OutlineScale {
    fn new(mode: OutlineWidthMode, options: &CaptureOptions) -> Self {
        Self {
            mode,
            view: camera_view(options),
            projection_y: projection_y_scale(),
        }
    }

    fn at(self, world_position: Vec3) -> f32 {
        match self.mode {
            OutlineWidthMode::ScreenCoordinates => {
                let view_z = self.view.transform_point3(world_position).z;
                (-view_z / self.projection_y).max(0.0)
            }
            OutlineWidthMode::None
            | OutlineWidthMode::WorldCoordinates
            | OutlineWidthMode::Unknown => 1.0,
        }
    }
}

fn draw_primitive(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    options: &CaptureOptions,
) -> Result<DrawPrimitive, Box<dyn Error>> {
    let shading = material_shading(loaded, primitive.material);
    let uv_transforms = material_uv_transforms(loaded, primitive.material, options.mtoon_time);
    let policy = material_policy(loaded, primitive.material);
    let mut vertices = primitive
        .positions
        .iter()
        .enumerate()
        .map(|(index, position)| {
            let normal = primitive
                .normals
                .get(index)
                .copied()
                .unwrap_or([0.0, 0.0, 1.0]);
            let tex_coord = primitive
                .tex_coords_0
                .get(index)
                .copied()
                .unwrap_or([0.0, 0.0]);
            let vertex_color = primitive
                .colors_0
                .get(index)
                .copied()
                .unwrap_or([1.0, 1.0, 1.0, 1.0]);
            let tangent = primitive
                .tangents
                .get(index)
                .copied()
                .unwrap_or([1.0, 0.0, 0.0, 1.0]);
            let normal_scale = if primitive.tangents.get(index).is_some() {
                shading.normal_scale
            } else {
                0.0
            };
            let (position, normal) = transform_vertex(
                Vec3::from_array(*position),
                Vec3::from_array(normal),
                world,
                skin_matrices,
                primitive.joints_0.get(index).copied(),
                primitive.weights_0.get(index).copied(),
            );
            let tangent = transform_direction(
                Vec3::new(tangent[0], tangent[1], tangent[2]),
                world,
                skin_matrices,
                primitive.joints_0.get(index).copied(),
                primitive.weights_0.get(index).copied(),
            )
            .extend(tangent[3]);
            Vertex {
                position: position.to_array(),
                normal: normal.to_array(),
                tangent: tangent.to_array(),
                tex_coord,
                color: if shading.pbr_fallback {
                    multiply_rgba(shading.base_color, vertex_color)
                } else {
                    shading.base_color
                },
                shade_color: shading.shade_color,
                shading: [
                    shading.shading_shift,
                    shading.shading_toony,
                    shading.gi_equalization,
                    shading.shading_shift_texture_scale,
                ],
                emissive: [
                    shading.emissive[0],
                    shading.emissive[1],
                    shading.emissive[2],
                    0.0,
                ],
                matcap_factor: [
                    shading.matcap_factor[0],
                    shading.matcap_factor[1],
                    shading.matcap_factor[2],
                    0.0,
                ],
                rim_color: [
                    shading.parametric_rim_color[0],
                    shading.parametric_rim_color[1],
                    shading.parametric_rim_color[2],
                    0.0,
                ],
                rim_params: [
                    shading.rim_lighting_mix,
                    shading.parametric_rim_fresnel_power,
                    shading.parametric_rim_lift,
                    policy.alpha_cutoff,
                ],
                outline_color: [1.0, 1.0, 1.0, -1.0],
                alpha_mode: alpha_mode_code(policy.alpha_mode),
                normal_scale,
                double_sided: if policy.cull_mode == CaptureCullMode::Off {
                    1.0
                } else {
                    0.0
                },
                _padding: 0.0,
            }
        })
        .collect::<Vec<_>>();
    if shading.normal_scale > 0.0 && primitive.tangents.is_empty() {
        generate_missing_tangents(&mut vertices, &primitive.indices, shading.normal_scale);
    }
    Ok(DrawPrimitive {
        vertices,
        indices: primitive.indices.clone(),
        images: material_images(loaded, primitive.material),
        uv_transforms,
        material_extra: material_extra_uniform(shading),
        policy,
    })
}

fn material_extra_uniform(shading: MaterialShading) -> MaterialExtraUniform {
    MaterialExtraUniform {
        flags: [
            if shading.v0_compat_shade { 1.0 } else { 0.0 },
            if shading.pbr_fallback { 1.0 } else { 0.0 },
            0.0,
            0.0,
        ],
        pbr_params: [
            shading.metallic,
            shading.roughness,
            shading.occlusion_strength,
            0.0,
        ],
    }
}

fn material_policy(loaded: &LoadedVrm, material: Option<usize>) -> MaterialPolicy {
    let mtoon = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref());
    let mut policy = mtoon
        .map(|mtoon| {
            let hints = mtoon.pipeline_hints();
            MaterialPolicy {
                render_order: hints.render_order,
                cull_mode: capture_cull_mode(hints.cull_mode),
                alpha_mode: capture_alpha_mode(hints.alpha_mode),
                depth_write: hints.depth_write,
                blend: hints.blend,
                alpha_cutoff: mtoon.cutoff_factor,
            }
        })
        .unwrap_or_default();
    if let Some(gltf) = material.and_then(|index| loaded.gltf_materials.get(index)) {
        match gltf.alpha_mode {
            GltfAlphaMode::Opaque => {}
            GltfAlphaMode::Mask => {
                policy.alpha_mode = CaptureAlphaMode::Mask;
                policy.depth_write = true;
                policy.blend = false;
                policy.alpha_cutoff = gltf.alpha_cutoff.unwrap_or(0.5);
            }
            GltfAlphaMode::Blend => {
                policy.alpha_mode = CaptureAlphaMode::Blend;
                policy.depth_write = mtoon.is_some_and(|mtoon| mtoon.transparent_with_z_write);
                policy.blend = true;
                policy.render_order = mtoon.map_or(policy.render_order.max(3000), |mtoon| {
                    3000 + mtoon_transparent_order_offset(mtoon)
                });
            }
        }
        if gltf.double_sided {
            policy.cull_mode = CaptureCullMode::Off;
        }
    }
    policy
}

fn mtoon_transparent_order_offset(mtoon: &vrm_core::MtoonMaterial) -> i32 {
    let queue_offset = if mtoon.transparent_with_z_write {
        0
    } else {
        19
    };
    queue_offset + mtoon.render_queue_offset_number
}

fn capture_cull_mode(mode: MtoonCullMode) -> CaptureCullMode {
    match mode {
        MtoonCullMode::Off => CaptureCullMode::Off,
        MtoonCullMode::Front => CaptureCullMode::Front,
        MtoonCullMode::Back => CaptureCullMode::Back,
    }
}

fn capture_alpha_mode(mode: MtoonAlphaMode) -> CaptureAlphaMode {
    match mode {
        MtoonAlphaMode::Opaque => CaptureAlphaMode::Opaque,
        MtoonAlphaMode::Mask => CaptureAlphaMode::Mask,
        MtoonAlphaMode::Blend => CaptureAlphaMode::Blend,
    }
}

fn alpha_mode_code(mode: CaptureAlphaMode) -> f32 {
    match mode {
        CaptureAlphaMode::Opaque => 0.0,
        CaptureAlphaMode::Mask => 1.0,
        CaptureAlphaMode::Blend => 2.0,
    }
}

fn skin_matrices(
    loaded: &LoadedVrm,
    skin: &GltfSkinData,
    world_matrices: &[Mat4],
    orientation: Mat4,
) -> Vec<Mat4> {
    skin.joints
        .iter()
        .enumerate()
        .map(|(index, joint)| {
            let joint_world = world_matrices
                .get(*joint)
                .copied()
                .or_else(|| loaded.scene.node(*joint).map(|node| node.world_matrix))
                .unwrap_or(Mat4::IDENTITY);
            let inverse_bind = skin
                .inverse_bind_matrices
                .get(index)
                .copied()
                .unwrap_or(Mat4::IDENTITY);
            orientation * joint_world * inverse_bind
        })
        .collect()
}

fn transform_vertex(
    position: Vec3,
    normal: Vec3,
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    joints: Option<[u16; 4]>,
    weights: Option<[f32; 4]>,
) -> (Vec3, Vec3) {
    let (Some(skin_matrices), Some(joints), Some(weights)) = (skin_matrices, joints, weights)
    else {
        return (
            world.transform_point3(position),
            world.transform_vector3(normal).normalize_or_zero(),
        );
    };

    let mut skinned_position = Vec3::ZERO;
    let mut skinned_normal = Vec3::ZERO;
    let mut total_weight = 0.0;
    for (joint, weight) in joints.into_iter().zip(weights) {
        if weight <= 0.0 {
            continue;
        }
        let Some(matrix) = skin_matrices.get(usize::from(joint)) else {
            continue;
        };
        skinned_position += matrix.transform_point3(position) * weight;
        skinned_normal += matrix.transform_vector3(normal) * weight;
        total_weight += weight;
    }
    if total_weight > 0.0 {
        (skinned_position, skinned_normal.normalize_or_zero())
    } else {
        (
            world.transform_point3(position),
            world.transform_vector3(normal).normalize_or_zero(),
        )
    }
}

fn multiply_rgba(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

fn generate_missing_tangents(vertices: &mut [Vertex], indices: &[u32], normal_scale: f32) {
    let mut tangents = vec![Vec3::ZERO; vertices.len()];
    let mut bitangents = vec![Vec3::ZERO; vertices.len()];

    for triangle in indices.chunks_exact(3) {
        let [Some(i0), Some(i1), Some(i2)] = [
            usize::try_from(triangle[0])
                .ok()
                .filter(|index| *index < vertices.len()),
            usize::try_from(triangle[1])
                .ok()
                .filter(|index| *index < vertices.len()),
            usize::try_from(triangle[2])
                .ok()
                .filter(|index| *index < vertices.len()),
        ] else {
            continue;
        };

        let p0 = Vec3::from_array(vertices[i0].position);
        let p1 = Vec3::from_array(vertices[i1].position);
        let p2 = Vec3::from_array(vertices[i2].position);
        let uv0 = Vec2::from_array(vertices[i0].tex_coord);
        let uv1 = Vec2::from_array(vertices[i1].tex_coord);
        let uv2 = Vec2::from_array(vertices[i2].tex_coord);
        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let delta_uv1 = uv1 - uv0;
        let delta_uv2 = uv2 - uv0;
        let determinant = delta_uv1.x * delta_uv2.y - delta_uv1.y * delta_uv2.x;
        if determinant.abs() < 0.000001 {
            continue;
        }
        let scale = determinant.recip();
        let tangent = (edge1 * delta_uv2.y - edge2 * delta_uv1.y) * scale;
        let bitangent = (edge2 * delta_uv1.x - edge1 * delta_uv2.x) * scale;
        for index in [i0, i1, i2] {
            tangents[index] += tangent;
            bitangents[index] += bitangent;
        }
    }

    for (index, vertex) in vertices.iter_mut().enumerate() {
        let normal = Vec3::from_array(vertex.normal).normalize_or_zero();
        let tangent = tangents[index] - normal * normal.dot(tangents[index]);
        if tangent.length_squared() < 0.000001 || bitangents[index].length_squared() < 0.000001 {
            vertex.normal_scale = 0.0;
            continue;
        }
        let tangent = tangent.normalize();
        let handedness = if normal.cross(tangent).dot(bitangents[index]) < 0.0 {
            -1.0
        } else {
            1.0
        };
        vertex.tangent = tangent.extend(handedness).to_array();
        vertex.normal_scale = normal_scale;
    }
}

fn transform_direction(
    direction: Vec3,
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    joints: Option<[u16; 4]>,
    weights: Option<[f32; 4]>,
) -> Vec3 {
    let Some(skin_matrices) = skin_matrices else {
        return world.transform_vector3(direction).normalize_or_zero();
    };
    let (Some(joints), Some(weights)) = (joints, weights) else {
        return world.transform_vector3(direction).normalize_or_zero();
    };

    let mut transformed = Vec3::ZERO;
    let mut total_weight = 0.0;
    for (joint, weight) in joints.into_iter().zip(weights) {
        if weight <= 0.0 {
            continue;
        }
        let Some(matrix) = skin_matrices.get(joint as usize) else {
            continue;
        };
        transformed += matrix.transform_vector3(direction) * weight;
        total_weight += weight;
    }
    if total_weight > 0.0 {
        transformed.normalize_or_zero()
    } else {
        world.transform_vector3(direction).normalize_or_zero()
    }
}

#[derive(Clone, Copy, Debug)]
struct MaterialShading {
    base_color: [f32; 4],
    shade_color: [f32; 4],
    shading_shift: f32,
    shading_toony: f32,
    shading_shift_texture_scale: f32,
    gi_equalization: f32,
    emissive: [f32; 3],
    matcap_factor: [f32; 3],
    parametric_rim_color: [f32; 3],
    rim_lighting_mix: f32,
    parametric_rim_fresnel_power: f32,
    parametric_rim_lift: f32,
    normal_scale: f32,
    metallic: f32,
    roughness: f32,
    occlusion_strength: f32,
    pbr_fallback: bool,
    v0_compat_shade: bool,
}

fn material_shading(loaded: &LoadedVrm, material: Option<usize>) -> MaterialShading {
    if let Some(shading) = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|core_material| {
            let mtoon = core_material.mtoon.as_ref()?;
            let (emissive_strength, _) = core_material.effective_emissive_strength();
            let v0_compat_shade = loaded.model().document().kind == VrmKind::Vrm0Compat;
            Some(MaterialShading {
                base_color: mtoon.base_color_factor,
                shade_color: [
                    mtoon.shade_color_factor[0],
                    mtoon.shade_color_factor[1],
                    mtoon.shade_color_factor[2],
                    1.0,
                ],
                shading_shift: mtoon.shading_shift_factor,
                shading_toony: mtoon.shading_toony_factor,
                shading_shift_texture_scale: mtoon.shading_shift_texture_scale,
                gi_equalization: mtoon.gi_equalization_factor,
                emissive: [
                    mtoon.emissive_factor[0] * emissive_strength.0,
                    mtoon.emissive_factor[1] * emissive_strength.0,
                    mtoon.emissive_factor[2] * emissive_strength.0,
                ],
                matcap_factor: mtoon.matcap_factor,
                parametric_rim_color: mtoon.parametric_rim_color_factor,
                rim_lighting_mix: mtoon.rim_lighting_mix_factor,
                parametric_rim_fresnel_power: mtoon.parametric_rim_fresnel_power_factor,
                parametric_rim_lift: mtoon.parametric_rim_lift_factor,
                normal_scale: material_normal_texture(loaded, material).map_or(0.0, |_| {
                    material
                        .and_then(|index| loaded.gltf_materials.get(index))
                        .map_or(1.0, |gltf_material| gltf_material.normal_scale)
                }),
                metallic: 0.0,
                roughness: 1.0,
                occlusion_strength: 0.0,
                pbr_fallback: false,
                v0_compat_shade,
            })
        })
    {
        return shading;
    }
    let gltf = material.and_then(|index| loaded.gltf_materials.get(index));
    let base_color = gltf
        .map(|material| material.base_color_factor)
        .unwrap_or([0.78, 0.78, 0.78, 1.0]);
    let emissive = gltf
        .map(|material| {
            material
                .emissive_factor
                .map(|channel| channel * material.emissive_strength)
        })
        .unwrap_or([0.0, 0.0, 0.0]);
    MaterialShading {
        base_color,
        shade_color: base_color,
        shading_shift: 0.0,
        shading_toony: 0.0,
        shading_shift_texture_scale: 1.0,
        gi_equalization: 0.0,
        emissive,
        matcap_factor: [0.0, 0.0, 0.0],
        parametric_rim_color: [0.0, 0.0, 0.0],
        rim_lighting_mix: 1.0,
        parametric_rim_fresnel_power: 5.0,
        parametric_rim_lift: 0.0,
        normal_scale: material_normal_texture(loaded, material)
            .map_or(0.0, |_| gltf.map_or(1.0, |material| material.normal_scale)),
        metallic: gltf.map_or(0.0, |material| material.metallic_factor),
        roughness: gltf.map_or(1.0, |material| material.roughness_factor),
        occlusion_strength: gltf.map_or(1.0, |material| material.occlusion_strength),
        pbr_fallback: true,
        v0_compat_shade: false,
    }
}

fn material_normal_texture(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let mtoon_texture = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.normal_texture)
        .map(|texture| texture.0);
    mtoon_texture.or_else(|| {
        material
            .and_then(|index| loaded.gltf_materials.get(index))
            .and_then(|material| material.normal_texture)
    })
}

fn material_uv_transforms(
    loaded: &LoadedVrm,
    material: Option<usize>,
    mtoon_time: f32,
) -> MaterialUvTransforms {
    let mtoon = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref());
    let gltf = material.and_then(|index| loaded.gltf_materials.get(index));
    let base = mtoon
        .and_then(|mtoon| mtoon.texture_transforms.main_texture)
        .or_else(|| gltf.and_then(|material| material.base_color_texture_transform));
    let shade = mtoon
        .and_then(|mtoon| mtoon.texture_transforms.shade_multiply_texture)
        .or(base);
    MaterialUvTransforms {
        base,
        shade,
        shading_shift: mtoon.and_then(|mtoon| mtoon.texture_transforms.shading_shift_texture),
        normal: mtoon
            .and_then(|mtoon| mtoon.texture_transforms.normal_texture)
            .or_else(|| gltf.and_then(|material| material.normal_texture_transform)),
        matcap: mtoon.and_then(|mtoon| mtoon.texture_transforms.matcap_texture),
        rim: mtoon.and_then(|mtoon| mtoon.texture_transforms.rim_multiply_texture),
        outline_width: mtoon
            .and_then(|mtoon| mtoon.texture_transforms.outline_width_multiply_texture),
        emissive: gltf.and_then(|material| material.emissive_texture_transform),
        occlusion: gltf.and_then(|material| material.occlusion_texture_transform),
        uv_animation_mask: mtoon
            .and_then(|mtoon| mtoon.texture_transforms.uv_animation_mask_texture),
        uv_animation_scroll: mtoon.map_or([0.0, 0.0], |mtoon| {
            [
                mtoon.uv_animation.scroll_x_speed * mtoon_time,
                mtoon.uv_animation.scroll_y_speed * mtoon_time,
            ]
        }),
        uv_animation_rotation: mtoon
            .map_or(0.0, |mtoon| mtoon.uv_animation.rotation_speed * mtoon_time),
    }
}

fn transform_uv(uv: [f32; 2], transform: Option<TextureTransform2d>) -> [f32; 2] {
    let Some(transform) = transform else {
        return uv;
    };
    if transform.tex_coord.is_some_and(|tex_coord| tex_coord != 0) {
        return uv;
    }
    let (sin, cos) = transform.rotation.sin_cos();
    let scaled = [uv[0] * transform.scale[0], uv[1] * transform.scale[1]];
    [
        cos * scaled[0] - sin * scaled[1] + transform.offset[0],
        sin * scaled[0] + cos * scaled[1] + transform.offset[1],
    ]
}

fn material_main_image(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let mtoon_texture = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.main_texture);
    let texture = mtoon_texture.map(|texture| texture.0).or_else(|| {
        material
            .and_then(|index| loaded.gltf_materials.get(index))
            .and_then(|material| material.base_color_texture)
    })?;
    loaded.textures.get(texture).map(|texture| texture.image)
}

fn material_images(loaded: &LoadedVrm, material: Option<usize>) -> MaterialImages {
    let mtoon = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref());
    let base = material_main_image(loaded, material);
    let shade = mtoon
        .and_then(|mtoon| mtoon.textures.shade_multiply_texture)
        .and_then(|texture| loaded.textures.get(texture.0))
        .map(|texture| texture.image);
    let shading_shift = mtoon
        .and_then(|mtoon| mtoon.textures.shading_shift_texture)
        .and_then(|texture| loaded.textures.get(texture.0))
        .map(|texture| texture.image);
    let normal = material_normal_texture(loaded, material)
        .and_then(|texture| loaded.textures.get(texture))
        .map(|texture| texture.image);
    let matcap = mtoon
        .and_then(|mtoon| mtoon.textures.matcap_texture)
        .and_then(|texture| loaded.textures.get(texture.0))
        .map(|texture| texture.image);
    let rim = mtoon
        .and_then(|mtoon| mtoon.textures.rim_multiply_texture)
        .and_then(|texture| loaded.textures.get(texture.0))
        .map(|texture| texture.image);
    let emissive = material
        .and_then(|index| loaded.gltf_materials.get(index))
        .and_then(|material| material.emissive_texture)
        .and_then(|texture| loaded.textures.get(texture))
        .map(|texture| texture.image);
    let occlusion = material
        .and_then(|index| loaded.gltf_materials.get(index))
        .and_then(|material| material.occlusion_texture)
        .and_then(|texture| loaded.textures.get(texture))
        .map(|texture| texture.image);
    let uv_animation_mask = mtoon
        .and_then(|mtoon| mtoon.textures.uv_animation_mask_texture)
        .and_then(|texture| loaded.textures.get(texture.0))
        .map(|texture| texture.image);
    MaterialImages {
        base,
        shade,
        shading_shift,
        normal,
        matcap,
        rim,
        emissive,
        occlusion,
        uv_animation_mask,
    }
}

fn sampled_image_for_texture(loaded: &LoadedVrm, texture: usize) -> Option<CpuRgbaImage> {
    let image = loaded.textures.get(texture)?.image;
    let image = loaded.images.get(image)?;
    Some(CpuRgbaImage {
        width: image.width,
        height: image.height,
        rgba: image_rgba8(image).ok()?,
    })
}

impl CpuRgbaImage {
    fn sample_green(&self, tex_coord: [f32; 2]) -> f32 {
        let u = tex_coord[0].rem_euclid(1.0);
        let v = tex_coord[1].rem_euclid(1.0);
        let x = u * self.width as f32 - 0.5;
        let y = v * self.height as f32 - 0.5;
        let x0 = x.floor();
        let y0 = y.floor();
        let tx = x - x0;
        let ty = y - y0;
        let x0 = x0 as i32;
        let y0 = y0 as i32;
        let top = lerp(self.green_at(x0, y0), self.green_at(x0 + 1, y0), tx);
        let bottom = lerp(self.green_at(x0, y0 + 1), self.green_at(x0 + 1, y0 + 1), tx);
        lerp(top, bottom, ty)
    }

    fn green_at(&self, x: i32, y: i32) -> f32 {
        let width = self.width as i32;
        let height = self.height as i32;
        let x = x.rem_euclid(width) as u32;
        let y = y.rem_euclid(height) as u32;
        let index = ((y * self.width + x) * 4 + 1) as usize;
        self.rgba.get(index).copied().unwrap_or(255) as f32 / 255.0
    }
}

fn lerp(left: f32, right: f32, t: f32) -> f32 {
    left + (right - left) * t
}

fn texture_resources(
    loaded: &LoadedVrm,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
) -> Result<Vec<TextureResource>, Box<dyn Error>> {
    let mut resources = vec![
        texture_resource(
            device,
            queue,
            format,
            TextureUpload {
                image: None,
                width: 1,
                height: 1,
                rgba: &[255, 255, 255, 255],
            },
        ),
        texture_resource(
            device,
            queue,
            format,
            TextureUpload {
                image: None,
                width: 1,
                height: 1,
                rgba: &[0, 0, 0, 255],
            },
        ),
        texture_resource(
            device,
            queue,
            format,
            TextureUpload {
                image: None,
                width: 1,
                height: 1,
                rgba: &[128, 128, 255, 255],
            },
        ),
    ];
    for (index, image) in loaded.images.iter().enumerate() {
        let rgba = image_rgba8(image)?;
        resources.push(texture_resource(
            device,
            queue,
            format,
            TextureUpload {
                image: Some(index),
                width: image.width,
                height: image.height,
                rgba: &rgba,
            },
        ));
    }
    Ok(resources)
}

fn texture_resource(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    upload: TextureUpload<'_>,
) -> TextureResource {
    let mip_levels = mip_chain(upload.width, upload.height, upload.rgba);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render parity material texture"),
        size: wgpu::Extent3d {
            width: upload.width,
            height: upload.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: u32::try_from(mip_levels.len()).unwrap_or(1),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip_level, level) in mip_levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: u32::try_from(mip_level).unwrap_or(0),
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &level.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(level.width * 4),
                rows_per_image: Some(level.height),
            },
            wgpu::Extent3d {
                width: level.width,
                height: level.height,
                depth_or_array_layers: 1,
            },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    TextureResource {
        image: upload.image,
        view,
    }
}

struct TextureMipLevel {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn mip_chain(width: u32, height: u32, rgba: &[u8]) -> Vec<TextureMipLevel> {
    let mut levels = vec![TextureMipLevel {
        width,
        height,
        rgba: rgba.to_vec(),
    }];
    let mut current_width = width;
    let mut current_height = height;
    let mut current_rgba = rgba.to_vec();
    while current_width > 1 || current_height > 1 {
        let next_width = (current_width / 2).max(1);
        let next_height = (current_height / 2).max(1);
        let Some(image) = image::RgbaImage::from_raw(current_width, current_height, current_rgba)
        else {
            break;
        };
        let next = image::imageops::resize(
            &image,
            next_width,
            next_height,
            image::imageops::FilterType::Triangle,
        );
        current_width = next_width;
        current_height = next_height;
        current_rgba = next.into_raw();
        levels.push(TextureMipLevel {
            width: current_width,
            height: current_height,
            rgba: current_rgba.clone(),
        });
    }
    levels
}

fn material_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    resources: TextureResourceTables<'_>,
    images: MaterialImages,
    uv_transforms: MaterialUvTransforms,
    material_extra: MaterialExtraUniform,
) -> TextureBindGroup {
    let base = texture_view(resources.color, resources.indices, images.base, 0);
    let shade = texture_view(resources.color, resources.indices, images.shade, 0);
    let shading_shift = texture_view(resources.color, resources.indices, images.shading_shift, 1);
    let matcap = texture_view(resources.color, resources.indices, images.matcap, 1);
    let rim = texture_view(resources.color, resources.indices, images.rim, 0);
    let emissive = texture_view(resources.color, resources.indices, images.emissive, 0);
    let occlusion = texture_view(resources.normal, resources.indices, images.occlusion, 0);
    let uv_animation_mask = texture_view(
        resources.color,
        resources.indices,
        images.uv_animation_mask,
        0,
    );
    let normal = texture_view(resources.normal, resources.indices, images.normal, 2);
    let uv_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("render parity material uv transform uniform"),
        contents: bytemuck::bytes_of(&MaterialUvUniform::from(uv_transforms)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let material_extra_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("render parity material extra uniform"),
        contents: bytemuck::bytes_of(&material_extra),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("render parity texture bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(base),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(shade),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(shading_shift),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(matcap),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(rim),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(normal),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(emissive),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: uv_uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(uv_animation_mask),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: material_extra_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: wgpu::BindingResource::TextureView(occlusion),
            },
        ],
    });
    TextureBindGroup {
        bind_group,
        _uv_uniform_buffer: uv_uniform_buffer,
        _material_extra_buffer: material_extra_buffer,
    }
}

fn texture_view<'a>(
    resources: &'a [TextureResource],
    indices: &HashMap<usize, usize>,
    image: Option<usize>,
    fallback_index: usize,
) -> &'a wgpu::TextureView {
    image
        .and_then(|image| indices.get(&image).copied())
        .and_then(|index| resources.get(index))
        .or_else(|| resources.get(fallback_index))
        .map(|resource| &resource.view)
        .expect("texture resource table must contain a white fallback")
}

fn texture_resource_indices(resources: &[TextureResource]) -> HashMap<usize, usize> {
    resources
        .iter()
        .enumerate()
        .filter_map(|(index, resource)| resource.image.map(|image| (image, index)))
        .collect()
}

fn image_rgba8(image: &ImageData) -> Result<Vec<u8>, Box<dyn Error>> {
    match image.format {
        ImageFormat::R8 => Ok(image
            .bytes
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect()),
        ImageFormat::R8G8 => Ok(image
            .bytes
            .chunks_exact(2)
            .flat_map(|chunk| [chunk[0], chunk[0], chunk[0], chunk[1]])
            .collect()),
        ImageFormat::R8G8B8 => Ok(image
            .bytes
            .chunks_exact(3)
            .flat_map(|chunk| [chunk[0], chunk[1], chunk[2], 255])
            .collect()),
        ImageFormat::R8G8B8A8 => Ok(image.bytes.clone()),
        ImageFormat::R16
        | ImageFormat::R16G16
        | ImageFormat::R16G16B16
        | ImageFormat::R16G16B16A16
        | ImageFormat::R32G32B32Float
        | ImageFormat::R32G32B32A32Float => Err(format!(
            "unsupported render capture image format: {:?}",
            image.format
        )
        .into()),
    }
}

fn pipeline_keys(mesh: &MeshDrawData) -> Vec<PipelineKey> {
    let mut keys = mesh
        .primitives
        .iter()
        .map(|primitive| pipeline_key(primitive.policy))
        .collect::<Vec<_>>();
    keys.sort_by_key(|key| (key.cull_mode as u8, key.depth_write, key.blend));
    keys.dedup();
    keys
}

fn pipeline_indices(keys: &[PipelineKey]) -> HashMap<PipelineKey, usize> {
    keys.iter()
        .copied()
        .enumerate()
        .map(|(index, key)| (key, index))
        .collect()
}

fn pipeline_key(policy: MaterialPolicy) -> PipelineKey {
    PipelineKey {
        cull_mode: policy.cull_mode,
        depth_write: policy.depth_write,
        blend: policy.blend,
    }
}

fn render_pipeline(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    texture_layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    key: PipelineKey,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render parity pipeline layout"),
        bind_group_layouts: &[Some(uniform_layout), Some(texture_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render parity pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Vertex::layout()],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: cull_face(key.cull_mode),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(key.depth_write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: key.blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn cull_face(mode: CaptureCullMode) -> Option<wgpu::Face> {
    match mode {
        CaptureCullMode::Off => None,
        CaptureCullMode::Front => Some(wgpu::Face::Front),
        CaptureCullMode::Back => Some(wgpu::Face::Back),
    }
}

async fn render_capture(
    loaded: &LoadedVrm,
    mesh: &MeshDrawData,
    options: &CaptureOptions,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("vrm-rs render parity device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        })
        .await?;

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let color_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render parity color"),
        size: extent(options),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render parity depth"),
        size: extent(options),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth24Plus,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let uniforms = uniforms(options);
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("render parity uniforms"),
        contents: bytemuck::bytes_of(&uniforms),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let uniform_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render parity uniform bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
    let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("render parity uniform bind group"),
        layout: &uniform_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buffer.as_entire_binding(),
        }],
    });
    let texture_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render parity texture bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("render parity sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let color_texture_resources =
        texture_resources(loaded, &device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb)?;
    let normal_texture_resources =
        texture_resources(loaded, &device, &queue, wgpu::TextureFormat::Rgba8Unorm)?;
    let texture_resource_indices = texture_resource_indices(&color_texture_resources);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("render parity shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline_keys = pipeline_keys(mesh);
    let pipelines = pipeline_keys
        .iter()
        .map(|key| {
            render_pipeline(
                &device,
                &uniform_bind_group_layout,
                &texture_bind_group_layout,
                &shader,
                format,
                *key,
            )
        })
        .collect::<Vec<_>>();
    let pipeline_indices = pipeline_indices(&pipeline_keys);
    let primitive_texture_bind_groups = mesh
        .primitives
        .iter()
        .map(|primitive| {
            material_texture_bind_group(
                &device,
                &texture_bind_group_layout,
                &sampler,
                TextureResourceTables {
                    color: &color_texture_resources,
                    normal: &normal_texture_resources,
                    indices: &texture_resource_indices,
                },
                primitive.images,
                primitive.uv_transforms,
                primitive.material_extra,
            )
        })
        .collect::<Vec<_>>();
    let gpu_primitives = mesh
        .primitives
        .iter()
        .enumerate()
        .map(|(primitive_index, primitive)| {
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("render parity vertices"),
                contents: bytemuck::cast_slice(&primitive.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("render parity indices"),
                contents: bytemuck::cast_slice(&primitive.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            Ok::<_, Box<dyn Error>>(GpuPrimitive {
                vertex_buffer,
                index_buffer,
                index_count: u32::try_from(primitive.indices.len())?,
                texture_bind_group_index: primitive_index,
                pipeline_index: pipeline_indices[&pipeline_key(primitive.policy)],
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let bytes_per_pixel = 4;
    let unpadded_bytes_per_row = options.width * bytes_per_pixel;
    let padded_bytes_per_row = align_to(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("render parity readback"),
        size: u64::from(padded_bytes_per_row * options.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("render parity encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("render parity pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(options.background.clear_color()),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &uniform_bind_group, &[]);
        for primitive in &gpu_primitives {
            pass.set_pipeline(&pipelines[primitive.pipeline_index]);
            pass.set_bind_group(
                1,
                &primitive_texture_bind_groups[primitive.texture_bind_group_index].bind_group,
                &[],
            );
            pass.set_vertex_buffer(0, primitive.vertex_buffer.slice(..));
            pass.set_index_buffer(primitive.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..primitive.index_count, 0, 0..1);
        }
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &output_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(options.height),
            },
        },
        extent(options),
    );
    queue.submit(Some(encoder.finish()));

    let slice = output_buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    rx.recv()??;
    let mapped = slice.get_mapped_range();
    let mut rgba = vec![0; (options.width * options.height * bytes_per_pixel) as usize];
    for row in 0..options.height as usize {
        let source_start = row * padded_bytes_per_row as usize;
        let source_end = source_start + unpadded_bytes_per_row as usize;
        let destination_start = row * unpadded_bytes_per_row as usize;
        rgba[destination_start..destination_start + unpadded_bytes_per_row as usize]
            .copy_from_slice(&mapped[source_start..source_end]);
    }
    drop(mapped);
    output_buffer.unmap();
    Ok(rgba)
}

fn uniforms(options: &CaptureOptions) -> Uniforms {
    let eye = camera_eye(options);
    let view = camera_view(options);
    let projection = Mat4::perspective_rh(
        30.0_f32.to_radians(),
        options.width as f32 / options.height as f32,
        0.1,
        20.0,
    );
    let light_dir = Vec3::new(-1.0, 1.0, -1.0).normalize();
    Uniforms {
        view_projection: (projection * view).to_cols_array_2d(),
        light_dir: Vec4::new(
            light_dir.x,
            light_dir.y,
            light_dir.z,
            options.direct_light_scale,
        )
        .to_array(),
        camera_pos: Vec4::new(eye.x, eye.y, eye.z, 1.0).to_array(),
        mtoon_lighting: mtoon_lighting_uniform(options),
    }
}

fn mtoon_lighting_uniform(options: &CaptureOptions) -> [f32; 4] {
    match options.mtoon_light_accumulation {
        MtoonLightAccumulation::Tuned => [
            options.mtoon_exposure,
            options.mtoon_ambient_base,
            options.mtoon_ambient_gi_scale,
            options.pbr_ambient,
        ],
        MtoonLightAccumulation::ThreeVrm => [1.0, options.pbr_ambient, 0.0, options.pbr_ambient],
    }
}

fn camera_eye(options: &CaptureOptions) -> Vec3 {
    Vec3::new(0.0, options.camera_y, -options.camera_z)
}

fn camera_view(options: &CaptureOptions) -> Mat4 {
    Mat4::look_at_rh(
        camera_eye(options),
        Vec3::new(0.0, options.target_y, 0.0),
        Vec3::Y,
    )
}

fn projection_y_scale() -> f32 {
    1.0 / (0.5 * 30.0_f32.to_radians()).tan()
}

fn extent(options: &CaptureOptions) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: options.width,
        height: options.height,
        depth_or_array_layers: 1,
    }
}

fn align_to(value: u32, alignment: u32) -> u32 {
    value.div_ceil(alignment) * alignment
}

fn write_rgba_json(options: &CaptureOptions, rgba: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = options.out.parent() {
        fs::create_dir_all(parent)?;
    }
    let effective_lighting = mtoon_lighting_uniform(options);
    let artifact = json!({
        "generator": "vrm-rs examples/wgpu_render_capture.rs",
        "fixture": options.fixture.to_string_lossy(),
        "width": options.width,
        "height": options.height,
        "camera": { "y": options.camera_y, "z": options.camera_z, "targetY": options.target_y },
        "mtoonLighting": {
            "exposure": options.mtoon_exposure,
            "ambientBase": options.mtoon_ambient_base,
            "ambientGiScale": options.mtoon_ambient_gi_scale,
            "pbrAmbient": options.pbr_ambient,
            "directLightScale": options.direct_light_scale,
            "lightAccumulation": options.mtoon_light_accumulation.as_str(),
            "effective": {
                "exposure": effective_lighting[0],
                "ambientBase": effective_lighting[1],
                "ambientGiScale": effective_lighting[2],
                "pbrAmbient": effective_lighting[3]
            },
            "time": options.mtoon_time
        },
        "format": "rgba8",
        "rgba": rgba,
    });
    fs::write(
        &options.out,
        format!("{}\n", serde_json::to_string_pretty(&artifact)?),
    )?;
    Ok(())
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    image::save_buffer(path, rgba, width, height, image::ColorType::Rgba8)?;
    Ok(())
}

const SHADER: &str = r#"
struct Uniforms {
    view_projection: mat4x4<f32>,
    light_dir: vec4<f32>,
    camera_pos: vec4<f32>,
    mtoon_lighting: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(1) @binding(0)
var base_texture: texture_2d<f32>;

@group(1) @binding(1)
var shade_texture: texture_2d<f32>;

@group(1) @binding(2)
var shading_shift_texture: texture_2d<f32>;

@group(1) @binding(3)
var matcap_texture: texture_2d<f32>;

@group(1) @binding(4)
var rim_texture: texture_2d<f32>;

@group(1) @binding(5)
var normal_texture: texture_2d<f32>;

@group(1) @binding(6)
var base_sampler: sampler;

@group(1) @binding(7)
var emissive_texture: texture_2d<f32>;

struct MaterialUvUniform {
    base_transform: vec4<f32>,
    shade_transform: vec4<f32>,
    shading_shift_transform: vec4<f32>,
    normal_transform: vec4<f32>,
    matcap_transform: vec4<f32>,
    rim_transform: vec4<f32>,
    emissive_transform: vec4<f32>,
    occlusion_transform: vec4<f32>,
    uv_animation_mask_transform: vec4<f32>,
    rotation_a: vec4<f32>,
    rotation_b: vec4<f32>,
    uv_animation: vec4<f32>,
};

@group(1) @binding(8)
var<uniform> material_uv: MaterialUvUniform;

@group(1) @binding(9)
var uv_animation_mask_texture: texture_2d<f32>;

struct MaterialExtraUniform {
    flags: vec4<f32>,
    pbr_params: vec4<f32>,
};

@group(1) @binding(10)
var<uniform> material_extra: MaterialExtraUniform;

@group(1) @binding(11)
var occlusion_texture: texture_2d<f32>;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) tex_coord: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) shade_color: vec4<f32>,
    @location(6) shading: vec4<f32>,
    @location(7) emissive: vec4<f32>,
    @location(8) matcap_factor: vec4<f32>,
    @location(9) rim_color: vec4<f32>,
    @location(10) rim_params: vec4<f32>,
    @location(11) outline_color: vec4<f32>,
    @location(12) alpha_mode: f32,
    @location(13) normal_scale: f32,
    @location(14) double_sided: f32,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) tangent: vec4<f32>,
    @location(2) tex_coord: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) shade_color: vec4<f32>,
    @location(5) shading: vec4<f32>,
    @location(6) emissive: vec4<f32>,
    @location(7) matcap_factor: vec4<f32>,
    @location(8) world_position: vec3<f32>,
    @location(9) rim_color: vec4<f32>,
    @location(10) rim_params: vec4<f32>,
    @location(11) outline_color: vec4<f32>,
    @location(12) alpha_mode: f32,
    @location(13) normal_scale: f32,
    @location(14) double_sided: f32,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
    out.normal = normalize(input.normal);
    out.tangent = vec4<f32>(normalize(input.tangent.xyz), input.tangent.w);
    out.world_position = input.position;
    out.tex_coord = input.tex_coord;
    out.color = input.color;
    out.shade_color = input.shade_color;
    out.shading = input.shading;
    out.emissive = input.emissive;
    out.matcap_factor = input.matcap_factor;
    out.rim_color = input.rim_color;
    out.rim_params = input.rim_params;
    out.outline_color = input.outline_color;
    out.alpha_mode = input.alpha_mode;
    out.normal_scale = input.normal_scale;
    out.double_sided = input.double_sided;
    return out;
}

fn linearstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    return clamp((value - edge0) / max(edge1 - edge0, 0.00001), 0.0, 1.0);
}

fn linear_to_srgb_channel(value: f32) -> f32 {
    let x = clamp(value, 0.0, 1.0);
    return select(1.055 * pow(x, 1.0 / 2.4) - 0.055, 12.92 * x, x <= 0.0031308);
}

fn output_color(color: vec3<f32>, alpha: f32) -> vec4<f32> {
    return vec4<f32>(
        linear_to_srgb_channel(color.r),
        linear_to_srgb_channel(color.g),
        linear_to_srgb_channel(color.b),
        alpha,
    );
}

fn pbr_direct(
    diffuse: vec3<f32>,
    normal: vec3<f32>,
    view_dir: vec3<f32>,
    light_dir: vec3<f32>,
    metallic: f32,
    roughness: f32,
) -> vec3<f32> {
    let pi = 3.141592653589793;
    let n_dot_l = max(dot(normal, light_dir), 0.0);
    let n_dot_v = max(dot(normal, view_dir), 0.0001);
    let half_dir = normalize(light_dir + view_dir);
    let n_dot_h = max(dot(normal, half_dir), 0.0001);
    let v_dot_h = max(dot(view_dir, half_dir), 0.0);
    let rough = clamp(roughness, 0.04, 1.0);
    let alpha = rough * rough;
    let alpha2 = alpha * alpha;
    let denom = n_dot_h * n_dot_h * (alpha2 - 1.0) + 1.0;
    let distribution = alpha2 / max(pi * denom * denom, 0.0001);
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let geometry_l = n_dot_l / (n_dot_l * (1.0 - k) + k);
    let geometry_v = n_dot_v / (n_dot_v * (1.0 - k) + k);
    let geometry = geometry_l * geometry_v;
    let f0 = mix(vec3<f32>(0.04), diffuse, metallic);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - v_dot_h, 5.0);
    let specular = distribution * geometry * fresnel / max(4.0 * n_dot_l * n_dot_v, 0.0001);
    let diffuse_lobe = diffuse * (1.0 - metallic) / pi;
    return (diffuse_lobe + specular) * pi * n_dot_l;
}

fn transform_uv(uv: vec2<f32>, offset_scale: vec4<f32>, rotation: f32) -> vec2<f32> {
    let scaled = uv * offset_scale.zw;
    let c = cos(rotation);
    let s = sin(rotation);
    return vec2<f32>(
        c * scaled.x - s * scaled.y + offset_scale.x,
        s * scaled.x + c * scaled.y + offset_scale.y,
    );
}

fn animate_uv(uv: vec2<f32>) -> vec2<f32> {
    let mask_uv = transform_uv(
        uv,
        material_uv.uv_animation_mask_transform,
        material_uv.rotation_b.z,
    );
    let mask = textureSample(uv_animation_mask_texture, base_sampler, mask_uv).b;
    let phase = material_uv.uv_animation.z * mask;
    let c = cos(phase);
    let s = sin(phase);
    let centered = uv - vec2<f32>(0.5, 0.5);
    let rotated = vec2<f32>(
        c * centered.x + s * centered.y,
        -s * centered.x + c * centered.y,
    ) + vec2<f32>(0.5, 0.5);
    return rotated + material_uv.uv_animation.xy * mask;
}

fn surface_normal(input: VertexOut, front_facing: bool, normal_uv: vec2<f32>) -> vec3<f32> {
    let face_sign = select(-1.0, 1.0, front_facing || input.double_sided < 0.5);
    let geometric_normal = normalize(input.normal) * face_sign;
    if input.normal_scale <= 0.0 {
        return geometric_normal;
    }
    let tangent = normalize(input.tangent.xyz) * face_sign;
    let bitangent = normalize(cross(geometric_normal, tangent) * input.tangent.w) * face_sign;
    let sampled = textureSample(normal_texture, base_sampler, normal_uv).xyz;
    let tangent_normal = vec3<f32>(
        (sampled.x * 2.0 - 1.0) * input.normal_scale,
        (1.0 - sampled.y * 2.0) * input.normal_scale,
        sampled.z * 2.0 - 1.0,
    );
    return normalize(
        tangent * tangent_normal.x +
        bitangent * tangent_normal.y +
        geometric_normal * tangent_normal.z,
    );
}

@fragment
fn fs_main(input: VertexOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let animated_uv = animate_uv(input.tex_coord);
    let base_uv = transform_uv(animated_uv, material_uv.base_transform, material_uv.rotation_a.x);
    let shade_uv = transform_uv(animated_uv, material_uv.shade_transform, material_uv.rotation_a.y);
    let shading_shift_uv = transform_uv(animated_uv, material_uv.shading_shift_transform, material_uv.rotation_a.z);
    let normal_uv = transform_uv(animated_uv, material_uv.normal_transform, material_uv.rotation_a.w);
    let rim_uv = transform_uv(animated_uv, material_uv.rim_transform, material_uv.rotation_b.x);
    let emissive_uv = transform_uv(animated_uv, material_uv.emissive_transform, material_uv.rotation_b.y);
    let occlusion_uv = transform_uv(animated_uv, material_uv.occlusion_transform, material_uv.uv_animation.w);
    let normal = surface_normal(input, front_facing, normal_uv);
    let ndotl = clamp(dot(normal, normalize(uniforms.light_dir.xyz)), -1.0, 1.0);
    let texel = textureSample(base_texture, base_sampler, base_uv);
    let emissive_texel = textureSample(emissive_texture, base_sampler, emissive_uv).rgb;
    let alpha = input.color.a * texel.a;
    if input.alpha_mode > 0.5 && input.alpha_mode < 1.5 && alpha < input.rim_params.w {
        discard;
    }
    let opaque_alpha = select(alpha, 1.0, input.alpha_mode < 1.5);
    let diffuse = input.color.rgb * texel.rgb;
    let view_dir = normalize(uniforms.camera_pos.xyz - input.world_position);
    if material_extra.flags.y > 0.5 {
        let direct = pbr_direct(
            diffuse,
            normal,
            view_dir,
            normalize(uniforms.light_dir.xyz),
            material_extra.pbr_params.x,
            material_extra.pbr_params.y,
        ) * uniforms.light_dir.w;
        let occlusion = (textureSample(occlusion_texture, base_sampler, occlusion_uv).r - 1.0) * material_extra.pbr_params.z + 1.0;
        let ambient = diffuse * (1.0 - material_extra.pbr_params.x) * uniforms.mtoon_lighting.w * occlusion;
        var pbr_color = direct + ambient + input.emissive.rgb * emissive_texel;
        if input.outline_color.a >= 0.0 {
            pbr_color = input.outline_color.rgb * mix(vec3<f32>(1.0), pbr_color, input.outline_color.a);
        }
        return output_color(pbr_color, opaque_alpha);
    }
    let shade_texel = textureSample(shade_texture, base_sampler, shade_uv);
    let shade = input.shade_color.rgb * shade_texel.rgb;
    let shift_texel = textureSample(shading_shift_texture, base_sampler, shading_shift_uv).r;
    let shift = input.shading.x + shift_texel * input.shading.w;
    let toony = input.shading.y;
    let gi = input.shading.z;
    let toon = linearstep(-1.0 + toony, 1.0 - toony, ndotl + shift);
    var direct = mix(shade, diffuse, toon) * uniforms.light_dir.w;
    if material_extra.flags.x > 0.5 {
        direct = min(direct, diffuse);
    }
    let occlusion = (textureSample(occlusion_texture, base_sampler, occlusion_uv).r - 1.0) * material_extra.pbr_params.z + 1.0;
    let ambient = diffuse * (uniforms.mtoon_lighting.y + uniforms.mtoon_lighting.z * gi) * occlusion;
    let matcap_x = normalize(vec3<f32>(view_dir.z, 0.0, -view_dir.x));
    let matcap_y = cross(view_dir, matcap_x);
    let raw_matcap_uv = vec2<f32>(
        0.5 + 0.5 * dot(matcap_x, normal),
        0.5 - 0.5 * dot(matcap_y, normal),
    );
    let matcap_uv = transform_uv(raw_matcap_uv, material_uv.matcap_transform, material_uv.rotation_b.w);
    let matcap = textureSample(matcap_texture, base_sampler, matcap_uv).rgb * input.matcap_factor.rgb;
    let rim_base = input.rim_color.rgb * pow(
        clamp(1.0 - dot(view_dir, normal) + input.rim_params.z, 0.0, 1.0),
        input.rim_params.y,
    );
    let rim_texel = textureSample(rim_texture, base_sampler, rim_uv).rgb;
    let rim_light = vec3<f32>(uniforms.light_dir.w + uniforms.mtoon_lighting.w);
    let rim_mix = mix(vec3<f32>(1.0), rim_light, input.rim_params.x);
    let rim = (rim_base + matcap) * rim_texel * rim_mix;
    var color = (direct + ambient + rim + input.emissive.rgb * emissive_texel) * uniforms.mtoon_lighting.x;
    if input.outline_color.a >= 0.0 {
        color = input.outline_color.rgb * mix(vec3<f32>(1.0), color, input.outline_color.a);
    }
    return output_color(color, opaque_alpha);
}
"#;
