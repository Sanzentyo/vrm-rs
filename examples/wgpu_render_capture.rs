//! Offscreen wgpu render capture for render-parity experiments.
//!
//! This example intentionally keeps renderer policy small: it loads real glTF
//! primitive buffers from `vrm-io`, draws them with a fixed camera/light setup,
//! and writes the same RGBA JSON artifact consumed by
//! `tools/render-parity/compare-psnr.mjs`.

#[path = "common/render_capture_imqraw.rs"]
mod render_capture_imqraw;
#[path = "common/render_capture_scene.rs"]
mod render_capture_scene;

use bytemuck::{Pod, Zeroable};
use clap::{Parser, ValueEnum};
use glam::{Mat4, Vec3, Vec4};
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use vrm_adapter::{MtoonLightAccumulation as AdapterMtoonLightAccumulation, MtoonLightingConfig};
use vrm_io::{
    GltfExpressionRenderEffects, GltfMagFilter, GltfMaterialRenderExtraOptions,
    GltfMaterialShadingOptions, GltfMaterialShadingPlan, GltfMaterialTextureBinding,
    GltfMaterialTextureBindingPlan, GltfMaterialTextureColorSpace, GltfMaterialTextureFallback,
    GltfMaterialTextureSlot, GltfMaterialTextureSlots, GltfMaterialUvTransforms, GltfMinFilter,
    GltfMtoonLightAccumulation as GltfLightAccumulation, GltfNormalMapMode, GltfOutlineScale,
    GltfOutlineVertexSettings, GltfPrimitiveData, GltfSamplerData, GltfWrapMode, LoadedVrm,
    Rgba8SamplingOrigin, generate_rgba_mip_chain, generate_tangents, image_data_to_rgba8,
    load_vrm_from_path,
};
use wgpu::util::DeviceExt;

type MaterialUvTransforms = GltfMaterialUvTransforms;

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
    view: [[f32; 4]; 4],
    light_dir: [f32; 4],
    light_color: [f32; 4],
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
    #[arg(long)]
    imqraw_out: Option<PathBuf>,
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
    #[arg(long, default_value_t = 1.0)]
    directional_r: f32,
    #[arg(long, default_value_t = 1.0)]
    directional_g: f32,
    #[arg(long, default_value_t = 1.0)]
    directional_b: f32,
    #[arg(long, value_enum, default_value_t = MtoonLightAccumulation::ThreeVrm)]
    mtoon_light_accumulation: MtoonLightAccumulation,
    #[arg(long, default_value_t = 0.0)]
    mtoon_time: f32,
    #[arg(long, value_enum, default_value_t = CaptureBackground::OpaqueBlack)]
    background: CaptureBackground,
    #[arg(long)]
    disable_outlines: bool,
    #[arg(long, default_value_t = 1.0)]
    outline_width_scale: f32,
    #[arg(long)]
    disable_normal_maps: bool,
    #[arg(long, value_enum, default_value_t = NormalMapMode::GeneratedTangents)]
    normal_map_mode: NormalMapMode,
    #[arg(long)]
    mtoon_v0_compat_shade: bool,
    #[arg(long = "expression")]
    expressions: Vec<String>,
}

#[derive(Clone, Debug)]
struct MeshDrawData {
    primitives: Vec<DrawPrimitive>,
}

#[derive(Clone, Debug)]
struct DrawPrimitive {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    images: GltfMaterialTextureSlots,
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
        AdapterMtoonLightAccumulation::from(self).as_str()
    }
}

impl From<MtoonLightAccumulation> for AdapterMtoonLightAccumulation {
    fn from(value: MtoonLightAccumulation) -> Self {
        match value {
            MtoonLightAccumulation::Tuned => Self::Tuned,
            MtoonLightAccumulation::ThreeVrm => Self::ThreeVrm,
        }
    }
}

impl From<MtoonLightAccumulation> for GltfLightAccumulation {
    fn from(value: MtoonLightAccumulation) -> Self {
        match value {
            MtoonLightAccumulation::Tuned => Self::Tuned,
            MtoonLightAccumulation::ThreeVrm => Self::ThreeVrm,
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
enum NormalMapMode {
    GeneratedTangents,
    Derivative,
}

impl NormalMapMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::GeneratedTangents => "generated-tangents",
            Self::Derivative => "derivative",
        }
    }
}

impl From<NormalMapMode> for GltfNormalMapMode {
    fn from(value: NormalMapMode) -> Self {
        match value {
            NormalMapMode::GeneratedTangents => Self::GeneratedTangents,
            NormalMapMode::Derivative => Self::Derivative,
        }
    }
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
        let plan = transforms.uniform_plan();
        Self {
            base_transform: plan.base_transform,
            shade_transform: plan.shade_transform,
            shading_shift_transform: plan.shading_shift_transform,
            normal_transform: plan.normal_transform,
            matcap_transform: plan.matcap_transform,
            rim_transform: plan.rim_transform,
            emissive_transform: plan.emissive_transform,
            occlusion_transform: plan.occlusion_transform,
            uv_animation_mask_transform: plan.uv_animation_mask_transform,
            rotation_a: plan.rotation_a,
            rotation_b: plan.rotation_b,
            uv_animation: plan.uv_animation,
        }
    }
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
    flags2: [f32; 4],
}

struct TextureResource {
    texture: Option<usize>,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

#[derive(Clone, Copy)]
struct TextureResourceTables<'a> {
    color: &'a [TextureResource],
    normal: &'a [TextureResource],
    indices: &'a HashMap<usize, usize>,
}

struct TextureUpload<'a> {
    texture: Option<usize>,
    width: u32,
    height: u32,
    rgba: &'a [u8],
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
    if let Some(path) = &options.imqraw_out {
        render_capture_imqraw::write_imqraw_rgba8(
            path,
            "wgpu",
            ["wgpu", "candidate"],
            options.width,
            options.height,
            &rgba,
        )?;
    }
    Ok(())
}

fn mesh_draw_data(
    loaded: &LoadedVrm,
    options: &CaptureOptions,
) -> Result<MeshDrawData, Box<dyn Error>> {
    let mut primitives = Vec::new();
    let world_matrices = render_capture_scene::runtime_world_matrices(loaded)?;
    let expression_effects = expression_render_effects(loaded, &options.expressions)?;

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
            .map(|skin| skin.joint_matrices(&loaded.scene, &world_matrices, orientation));
        let morph_weights = expression_effects.active_morph_weights(node_index, node, mesh);
        let draw_context = PrimitiveDrawContext {
            expression_effects: &expression_effects,
            world,
            skin_matrices: skin_matrices.as_deref(),
            options,
        };
        for primitive in &mesh.primitives {
            let surface = draw_primitive(loaded, primitive, &morph_weights, &draw_context)?;
            primitives.push(surface.clone());
            if !options.disable_outlines
                && let Some(outline) =
                    outline_primitive(loaded, primitive, &morph_weights, &surface, &draw_context)
            {
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

#[derive(Clone, Copy)]
struct PrimitiveDrawContext<'a> {
    expression_effects: &'a GltfExpressionRenderEffects,
    world: Mat4,
    skin_matrices: Option<&'a [Mat4]>,
    options: &'a CaptureOptions,
}

fn outline_primitive(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    morph_weights: &[f32],
    surface: &DrawPrimitive,
    context: &PrimitiveDrawContext<'_>,
) -> Option<DrawPrimitive> {
    let outline =
        loaded.expression_mtoon_outline_plan(primitive.material, context.expression_effects)?;
    let width_texture = loaded.material_outline_width_rgba8_image(primitive.material);
    let uv_transforms = surface.uv_transforms;
    let width = outline.width_factor * context.options.outline_width_scale;
    let outline_scale = GltfOutlineScale::new(
        outline.width_mode,
        camera_view(context.options),
        projection_y_scale(),
    );
    let outline_vertices = primitive.outline_vertices(
        morph_weights,
        GltfOutlineVertexSettings {
            base_width: width,
            scale: outline_scale,
            width_texture: width_texture.as_ref(),
            width_transform: uv_transforms.outline_width,
            width_texture_origin: Rgba8SamplingOrigin::TopLeft,
        },
        context.world,
        context.skin_matrices,
    )?;
    let vertices = surface
        .vertices
        .iter()
        .zip(outline_vertices)
        .map(|(vertex, outline_vertex)| {
            let mut vertex = *vertex;
            vertex.position = outline_vertex.position.to_array();
            vertex.outline_color = outline.color;
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

fn draw_primitive(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    morph_weights: &[f32],
    context: &PrimitiveDrawContext<'_>,
) -> Result<DrawPrimitive, Box<dyn Error>> {
    let mut shading = loaded.expression_material_shading_plan(
        primitive.material,
        GltfMaterialShadingOptions {
            v0_compat_shade: context.options.mtoon_v0_compat_shade,
        },
        context.expression_effects,
    );
    if context.options.disable_normal_maps {
        shading.normal_scale = 0.0;
    }
    let uv_transforms = loaded.expression_material_uv_transforms(
        primitive.material,
        context.options.mtoon_time,
        context.expression_effects,
    );
    let policy = material_policy(loaded, primitive.material);
    let normal_plan = primitive.normal_map_plan(
        shading.normal_scale,
        GltfNormalMapMode::from(context.options.normal_map_mode),
    );
    let transformed_vertices = primitive
        .transformed_vertices(morph_weights, context.world, context.skin_matrices)
        .ok_or("failed to prepare transformed primitive vertices")?;
    let mut vertices = transformed_vertices
        .iter()
        .enumerate()
        .map(|(index, transformed)| {
            let normal_scale =
                normal_plan.vertex_normal_scale(primitive.tangents.get(index).is_some());
            Vertex {
                position: transformed.position.to_array(),
                normal: transformed.normal.to_array(),
                tangent: transformed.tangent.to_array(),
                tex_coord: transformed.tex_coord_0,
                color: if shading.pbr_fallback {
                    multiply_rgba(shading.base_color, transformed.color_0)
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
    if normal_plan.should_generate_tangents() {
        generate_missing_tangents(&mut vertices, &primitive.indices, normal_plan.normal_scale);
    }
    Ok(DrawPrimitive {
        vertices,
        indices: primitive.indices.clone(),
        images: loaded.material_texture_slots(primitive.material),
        uv_transforms,
        material_extra: material_extra_uniform(shading, context.options),
        policy,
    })
}

fn expression_render_effects(
    loaded: &LoadedVrm,
    expression_args: &[String],
) -> Result<GltfExpressionRenderEffects, Box<dyn Error>> {
    Ok(loaded.expression_render_effects(parse_expression_args(expression_args)?)?)
}

fn parse_expression_args(args: &[String]) -> Result<Vec<(String, f32)>, Box<dyn Error>> {
    args.iter()
        .map(|arg| {
            let Some((name, value)) = arg.split_once('=') else {
                return Err(format!("invalid expression '{arg}', expected name=weight").into());
            };
            let weight = value
                .parse::<f32>()
                .map_err(|err| format!("invalid expression weight in '{arg}': {err}"))?;
            Ok((name.to_owned(), weight))
        })
        .collect()
}

fn material_extra_uniform(
    shading: GltfMaterialShadingPlan,
    options: &CaptureOptions,
) -> MaterialExtraUniform {
    let plan = shading
        .render_extra_plan(GltfMaterialRenderExtraOptions {
            light_accumulation: options.mtoon_light_accumulation.into(),
            derivative_normals: false,
            direct_light_scale: options.direct_light_scale,
        })
        .uniform_plan();
    MaterialExtraUniform {
        flags: plan.flags,
        pbr_params: plan.pbr_params,
        flags2: plan.flags2,
    }
}

fn material_policy(loaded: &LoadedVrm, material: Option<usize>) -> MaterialPolicy {
    let plan = render_capture_scene::capture_material_plan(loaded, material);
    MaterialPolicy {
        render_order: plan.render_order,
        cull_mode: capture_cull_mode(plan.cull_mode),
        alpha_mode: capture_alpha_mode(plan.alpha_mode),
        depth_write: plan.depth_write,
        blend: plan.blend,
        alpha_cutoff: plan.alpha_cutoff,
    }
}

fn capture_cull_mode(mode: render_capture_scene::CaptureMaterialCullMode) -> CaptureCullMode {
    match mode {
        render_capture_scene::CaptureMaterialCullMode::Off => CaptureCullMode::Off,
        render_capture_scene::CaptureMaterialCullMode::Front => CaptureCullMode::Front,
        render_capture_scene::CaptureMaterialCullMode::Back => CaptureCullMode::Back,
    }
}

fn capture_alpha_mode(mode: render_capture_scene::CaptureMaterialAlphaMode) -> CaptureAlphaMode {
    match mode {
        render_capture_scene::CaptureMaterialAlphaMode::Opaque => CaptureAlphaMode::Opaque,
        render_capture_scene::CaptureMaterialAlphaMode::Mask => CaptureAlphaMode::Mask,
        render_capture_scene::CaptureMaterialAlphaMode::Blend => CaptureAlphaMode::Blend,
    }
}

fn alpha_mode_code(mode: CaptureAlphaMode) -> f32 {
    match mode {
        CaptureAlphaMode::Opaque => 0.0,
        CaptureAlphaMode::Mask => 1.0,
        CaptureAlphaMode::Blend => 2.0,
    }
}

fn multiply_rgba(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

fn generate_missing_tangents(vertices: &mut [Vertex], indices: &[u32], normal_scale: f32) {
    let positions = vertices
        .iter()
        .map(|vertex| vertex.position)
        .collect::<Vec<_>>();
    let normals = vertices
        .iter()
        .map(|vertex| vertex.normal)
        .collect::<Vec<_>>();
    let tex_coords = vertices
        .iter()
        .map(|vertex| vertex.tex_coord)
        .collect::<Vec<_>>();
    let Some(generated) = generate_tangents(&positions, &normals, &tex_coords, indices) else {
        return;
    };

    for (vertex, tangent) in vertices.iter_mut().zip(generated.tangents) {
        if let Some(tangent) = tangent {
            vertex.tangent = tangent;
            vertex.normal_scale = normal_scale;
        } else {
            vertex.normal_scale = 0.0;
        }
    }
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
            GltfSamplerData::default(),
            TextureUpload {
                texture: None,
                width: 1,
                height: 1,
                rgba: &[255, 255, 255, 255],
            },
        ),
        texture_resource(
            device,
            queue,
            format,
            GltfSamplerData::default(),
            TextureUpload {
                texture: None,
                width: 1,
                height: 1,
                rgba: &[0, 0, 0, 255],
            },
        ),
        texture_resource(
            device,
            queue,
            format,
            GltfSamplerData::default(),
            TextureUpload {
                texture: None,
                width: 1,
                height: 1,
                rgba: &[128, 128, 255, 255],
            },
        ),
    ];
    for (index, texture) in loaded.textures.iter().enumerate() {
        let Some(image) = loaded.images.get(texture.image) else {
            continue;
        };
        let rgba = image_data_to_rgba8(image)?;
        resources.push(texture_resource(
            device,
            queue,
            format,
            texture.sampler,
            TextureUpload {
                texture: Some(index),
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
    sampler_data: GltfSamplerData,
    upload: TextureUpload<'_>,
) -> TextureResource {
    let mip_levels = generate_rgba_mip_chain(upload.width, upload.height, upload.rgba)
        .expect("texture upload RGBA data should match its dimensions");
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
    let sampler = device.create_sampler(&sampler_descriptor(sampler_data));
    TextureResource {
        texture: upload.texture,
        view,
        sampler,
    }
}

fn sampler_descriptor(sampler: GltfSamplerData) -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        label: Some("render parity texture sampler"),
        address_mode_u: address_mode(sampler.wrap_s),
        address_mode_v: address_mode(sampler.wrap_t),
        mag_filter: mag_filter(sampler.mag_filter),
        min_filter: min_filter(sampler.min_filter),
        mipmap_filter: mipmap_filter(sampler.min_filter),
        lod_max_clamp: if sampler.min_filter.uses_mipmaps() {
            32.0
        } else {
            0.0
        },
        ..Default::default()
    }
}

fn address_mode(mode: GltfWrapMode) -> wgpu::AddressMode {
    match mode {
        GltfWrapMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        GltfWrapMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
        GltfWrapMode::Repeat => wgpu::AddressMode::Repeat,
    }
}

fn mag_filter(filter: GltfMagFilter) -> wgpu::FilterMode {
    match filter {
        GltfMagFilter::Nearest => wgpu::FilterMode::Nearest,
        GltfMagFilter::Linear => wgpu::FilterMode::Linear,
    }
}

fn min_filter(filter: GltfMinFilter) -> wgpu::FilterMode {
    match filter {
        GltfMinFilter::Nearest
        | GltfMinFilter::NearestMipmapNearest
        | GltfMinFilter::NearestMipmapLinear => wgpu::FilterMode::Nearest,
        GltfMinFilter::Linear
        | GltfMinFilter::LinearMipmapNearest
        | GltfMinFilter::LinearMipmapLinear => wgpu::FilterMode::Linear,
    }
}

fn mipmap_filter(filter: GltfMinFilter) -> wgpu::MipmapFilterMode {
    match filter {
        GltfMinFilter::Nearest
        | GltfMinFilter::Linear
        | GltfMinFilter::NearestMipmapNearest
        | GltfMinFilter::LinearMipmapNearest => wgpu::MipmapFilterMode::Nearest,
        GltfMinFilter::NearestMipmapLinear | GltfMinFilter::LinearMipmapLinear => {
            wgpu::MipmapFilterMode::Linear
        }
    }
}

fn material_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    resources: TextureResourceTables<'_>,
    images: GltfMaterialTextureSlots,
    uv_transforms: MaterialUvTransforms,
    material_extra: MaterialExtraUniform,
) -> TextureBindGroup {
    let binding_plan = images.binding_plan();
    let base = texture_binding_view(&binding_plan, GltfMaterialTextureSlot::Base, resources);
    let shade = texture_binding_view(&binding_plan, GltfMaterialTextureSlot::Shade, resources);
    let shading_shift = texture_binding_view(
        &binding_plan,
        GltfMaterialTextureSlot::ShadingShift,
        resources,
    );
    let matcap = texture_binding_view(&binding_plan, GltfMaterialTextureSlot::Matcap, resources);
    let rim = texture_binding_view(&binding_plan, GltfMaterialTextureSlot::Rim, resources);
    let emissive =
        texture_binding_view(&binding_plan, GltfMaterialTextureSlot::Emissive, resources);
    let occlusion =
        texture_binding_view(&binding_plan, GltfMaterialTextureSlot::Occlusion, resources);
    let uv_animation_mask = texture_binding_view(
        &binding_plan,
        GltfMaterialTextureSlot::UvAnimationMask,
        resources,
    );
    let normal = texture_binding_view(&binding_plan, GltfMaterialTextureSlot::Normal, resources);
    let base_sampler =
        texture_binding_sampler(&binding_plan, GltfMaterialTextureSlot::Base, resources);
    let shade_sampler =
        texture_binding_sampler(&binding_plan, GltfMaterialTextureSlot::Shade, resources);
    let shading_shift_sampler = texture_binding_sampler(
        &binding_plan,
        GltfMaterialTextureSlot::ShadingShift,
        resources,
    );
    let matcap_sampler =
        texture_binding_sampler(&binding_plan, GltfMaterialTextureSlot::Matcap, resources);
    let rim_sampler =
        texture_binding_sampler(&binding_plan, GltfMaterialTextureSlot::Rim, resources);
    let normal_sampler =
        texture_binding_sampler(&binding_plan, GltfMaterialTextureSlot::Normal, resources);
    let emissive_sampler =
        texture_binding_sampler(&binding_plan, GltfMaterialTextureSlot::Emissive, resources);
    let uv_animation_mask_sampler = texture_binding_sampler(
        &binding_plan,
        GltfMaterialTextureSlot::UvAnimationMask,
        resources,
    );
    let occlusion_sampler =
        texture_binding_sampler(&binding_plan, GltfMaterialTextureSlot::Occlusion, resources);
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
                resource: wgpu::BindingResource::Sampler(base_sampler),
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
            wgpu::BindGroupEntry {
                binding: 12,
                resource: wgpu::BindingResource::Sampler(shade_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 13,
                resource: wgpu::BindingResource::Sampler(shading_shift_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 14,
                resource: wgpu::BindingResource::Sampler(matcap_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 15,
                resource: wgpu::BindingResource::Sampler(rim_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 16,
                resource: wgpu::BindingResource::Sampler(normal_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 17,
                resource: wgpu::BindingResource::Sampler(emissive_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 18,
                resource: wgpu::BindingResource::Sampler(uv_animation_mask_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 19,
                resource: wgpu::BindingResource::Sampler(occlusion_sampler),
            },
        ],
    });
    TextureBindGroup {
        bind_group,
        _uv_uniform_buffer: uv_uniform_buffer,
        _material_extra_buffer: material_extra_buffer,
    }
}

fn texture_binding_view<'a>(
    plan: &GltfMaterialTextureBindingPlan,
    slot: GltfMaterialTextureSlot,
    resources: TextureResourceTables<'a>,
) -> &'a wgpu::TextureView {
    let binding = plan
        .binding(slot)
        .expect("MToon texture binding plan must contain every shader slot");
    let (resource_table, fallback_index) = texture_binding_resources(binding, resources);
    texture_view(
        resource_table,
        resources.indices,
        binding.texture,
        fallback_index,
    )
}

fn texture_binding_sampler<'a>(
    plan: &GltfMaterialTextureBindingPlan,
    slot: GltfMaterialTextureSlot,
    resources: TextureResourceTables<'a>,
) -> &'a wgpu::Sampler {
    let binding = plan
        .binding(slot)
        .expect("MToon texture binding plan must contain every shader slot");
    let (resource_table, fallback_index) = texture_binding_resources(binding, resources);
    texture_sampler(
        resource_table,
        resources.indices,
        binding.texture,
        fallback_index,
    )
}

fn texture_binding_resources<'a>(
    binding: GltfMaterialTextureBinding,
    resources: TextureResourceTables<'a>,
) -> (&'a [TextureResource], usize) {
    let selected = match binding.color_space {
        GltfMaterialTextureColorSpace::Srgb => resources.color,
        GltfMaterialTextureColorSpace::Linear => resources.normal,
    };
    (selected, texture_fallback_index(binding.fallback))
}

fn texture_fallback_index(fallback: GltfMaterialTextureFallback) -> usize {
    match fallback {
        GltfMaterialTextureFallback::White => 0,
        GltfMaterialTextureFallback::Black => 1,
        GltfMaterialTextureFallback::NeutralNormal => 2,
    }
}

fn texture_view<'a>(
    resources: &'a [TextureResource],
    indices: &HashMap<usize, usize>,
    texture: Option<usize>,
    fallback_index: usize,
) -> &'a wgpu::TextureView {
    texture
        .and_then(|texture| indices.get(&texture).copied())
        .and_then(|index| resources.get(index))
        .or_else(|| resources.get(fallback_index))
        .map(|resource| &resource.view)
        .expect("texture resource table must contain a white fallback")
}

fn texture_sampler<'a>(
    resources: &'a [TextureResource],
    indices: &HashMap<usize, usize>,
    texture: Option<usize>,
    fallback_index: usize,
) -> &'a wgpu::Sampler {
    texture
        .and_then(|texture| indices.get(&texture).copied())
        .and_then(|index| resources.get(index))
        .or_else(|| resources.get(fallback_index))
        .map(|resource| &resource.sampler)
        .expect("texture resource table must contain a sampler fallback")
}

fn texture_resource_indices(resources: &[TextureResource]) -> HashMap<usize, usize> {
    resources
        .iter()
        .enumerate()
        .filter_map(|(index, resource)| resource.texture.map(|texture| (texture, index)))
        .collect()
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
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 14,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 15,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 16,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 17,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 18,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 19,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
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
        view: view.to_cols_array_2d(),
        light_dir: Vec4::new(
            light_dir.x,
            light_dir.y,
            light_dir.z,
            options.direct_light_scale,
        )
        .to_array(),
        light_color: Vec4::new(
            options.directional_r,
            options.directional_g,
            options.directional_b,
            0.0,
        )
        .to_array(),
        camera_pos: Vec4::new(eye.x, eye.y, eye.z, 1.0).to_array(),
        mtoon_lighting: mtoon_lighting_uniform(options),
    }
}

fn mtoon_lighting_uniform(options: &CaptureOptions) -> [f32; 4] {
    MtoonLightingConfig {
        accumulation: options.mtoon_light_accumulation.into(),
        exposure: options.mtoon_exposure,
        ambient_base: options.mtoon_ambient_base,
        ambient_gi_scale: options.mtoon_ambient_gi_scale,
        pbr_ambient: options.pbr_ambient,
    }
    .effective_values()
    .to_array()
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
        "disableOutlines": options.disable_outlines,
        "outlineWidthScale": options.outline_width_scale,
        "disableNormalMaps": options.disable_normal_maps,
        "normalMapMode": options.normal_map_mode.as_str(),
        "expressions": options.expressions,
        "camera": { "y": options.camera_y, "z": options.camera_z, "targetY": options.target_y },
        "mtoonLighting": {
            "exposure": options.mtoon_exposure,
            "ambientBase": options.mtoon_ambient_base,
            "ambientGiScale": options.mtoon_ambient_gi_scale,
            "pbrAmbient": options.pbr_ambient,
            "directLightScale": options.direct_light_scale,
            "directionalColor": [
                options.directional_r,
                options.directional_g,
                options.directional_b
            ],
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
    view: mat4x4<f32>,
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
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
    flags2: vec4<f32>,
};

@group(1) @binding(10)
var<uniform> material_extra: MaterialExtraUniform;

@group(1) @binding(11)
var occlusion_texture: texture_2d<f32>;

@group(1) @binding(12)
var shade_sampler: sampler;

@group(1) @binding(13)
var shading_shift_sampler: sampler;

@group(1) @binding(14)
var matcap_sampler: sampler;

@group(1) @binding(15)
var rim_sampler: sampler;

@group(1) @binding(16)
var normal_sampler: sampler;

@group(1) @binding(17)
var emissive_sampler: sampler;

@group(1) @binding(18)
var uv_animation_mask_sampler: sampler;

@group(1) @binding(19)
var occlusion_sampler: sampler;

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
    if input.outline_color.a >= 0.0 {
        out.position.z += 0.000001 * out.position.w;
    }
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
    let mask = textureSample(uv_animation_mask_texture, uv_animation_mask_sampler, mask_uv).b;
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
    if input.normal_scale == 0.0 {
        return geometric_normal;
    }
    let normal_scale = abs(input.normal_scale);
    let tangent = normalize(input.tangent.xyz) * face_sign;
    let bitangent = normalize(cross(geometric_normal, tangent) * input.tangent.w) * face_sign;
    let sampled = textureSample(normal_texture, normal_sampler, normal_uv).xyz;
    let tangent_normal = vec3<f32>(
        (sampled.x * 2.0 - 1.0) * normal_scale,
        (1.0 - sampled.y * 2.0) * normal_scale,
        sampled.z * 2.0 - 1.0,
    );
    if input.normal_scale < 0.0 {
        let q0 = dpdx(input.world_position);
        let q1 = dpdy(input.world_position);
        let st0 = dpdx(normal_uv);
        let st1 = dpdy(normal_uv);
        let q1perp = cross(q1, geometric_normal);
        let q0perp = cross(geometric_normal, q0);
        var tangent = q1perp * st0.x + q0perp * st1.x;
        var bitangent = q1perp * st0.y + q0perp * st1.y;
        let det = max(dot(tangent, tangent), dot(bitangent, bitangent));
        if det <= 0.0 {
            return geometric_normal;
        }
        let scale = 1.0 / sqrt(det);
        tangent = tangent * scale * face_sign;
        bitangent = bitangent * scale * face_sign;
        return normalize(
            tangent * tangent_normal.x +
            bitangent * tangent_normal.y +
            geometric_normal * tangent_normal.z,
        );
    }
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
    let emissive_texel = textureSample(emissive_texture, emissive_sampler, emissive_uv).rgb;
    let alpha = input.color.a * texel.a;
    if input.alpha_mode > 0.5 && input.alpha_mode < 1.5 && alpha < input.rim_params.w {
        discard;
    }
    let opaque_alpha = select(alpha, 1.0, input.alpha_mode < 1.5);
    let diffuse = input.color.rgb * texel.rgb;
    let view_dir = normalize(uniforms.camera_pos.xyz - input.world_position);
    if material_extra.flags2.x > 0.5 {
        return output_color(diffuse + input.emissive.rgb * emissive_texel, opaque_alpha);
    }
    if material_extra.flags.y > 0.5 {
        let direct = pbr_direct(
            diffuse,
            normal,
            view_dir,
            normalize(uniforms.light_dir.xyz),
            material_extra.pbr_params.x,
            material_extra.pbr_params.y,
        ) * uniforms.light_color.rgb * uniforms.light_dir.w;
        let occlusion = (textureSample(occlusion_texture, occlusion_sampler, occlusion_uv).r - 1.0) * material_extra.pbr_params.z + 1.0;
        let ambient = diffuse * (1.0 - material_extra.pbr_params.x) * uniforms.mtoon_lighting.w * occlusion;
        var pbr_color = direct + ambient + input.emissive.rgb * emissive_texel;
        if input.outline_color.a >= 0.0 {
            pbr_color = input.outline_color.rgb * mix(vec3<f32>(1.0), pbr_color, input.outline_color.a);
        }
        return output_color(pbr_color, opaque_alpha);
    }
    let shade_texel = textureSample(shade_texture, shade_sampler, shade_uv);
    let shade = input.shade_color.rgb * shade_texel.rgb;
    let shift_texel = textureSample(shading_shift_texture, shading_shift_sampler, shading_shift_uv).r;
    let shift = input.shading.x + shift_texel * input.shading.w;
    let toony = input.shading.y;
    let gi = input.shading.z;
    let toon = linearstep(-1.0 + toony, 1.0 - toony, ndotl + shift);
    var direct = mix(shade, diffuse, toon) * uniforms.light_color.rgb * uniforms.light_dir.w;
    if material_extra.flags.x > 0.5 {
        direct = min(direct, diffuse);
    }
    let sampled_occlusion = (textureSample(occlusion_texture, occlusion_sampler, occlusion_uv).r - 1.0) * material_extra.pbr_params.z + 1.0;
    let occlusion = select(sampled_occlusion, 1.0, material_extra.flags.z > 0.5);
    let ambient = diffuse * (uniforms.mtoon_lighting.y + uniforms.mtoon_lighting.z * gi) * occlusion;
    let matcap_view_position = (uniforms.view * vec4<f32>(input.world_position, 1.0)).xyz;
    let matcap_view_dir = normalize(-matcap_view_position);
    let matcap_normal = normalize((uniforms.view * vec4<f32>(normal, 0.0)).xyz);
    let matcap_x = normalize(vec3<f32>(matcap_view_dir.z, 0.0, -matcap_view_dir.x));
    let matcap_y = cross(matcap_view_dir, matcap_x);
    let raw_matcap_uv = vec2<f32>(
        0.5 + 0.5 * dot(matcap_x, matcap_normal),
        0.5 - 0.5 * dot(matcap_y, matcap_normal),
    );
    let matcap_uv = transform_uv(raw_matcap_uv, material_uv.matcap_transform, material_uv.rotation_b.w);
    let matcap = textureSample(matcap_texture, matcap_sampler, matcap_uv).rgb * input.matcap_factor.rgb;
    let rim_base = input.rim_color.rgb * pow(
        clamp(1.0 - dot(view_dir, normal) + input.rim_params.z, 0.0, 1.0),
        input.rim_params.y,
    );
    let rim_texel = textureSample(rim_texture, rim_sampler, rim_uv).rgb;
    let rim_light = uniforms.light_color.rgb * uniforms.light_dir.w + vec3<f32>(uniforms.mtoon_lighting.w);
    let rim_mix = mix(vec3<f32>(1.0), rim_light, input.rim_params.x);
    let rim = (rim_base + matcap) * rim_texel * rim_mix;
    var color = (direct + ambient + rim + input.emissive.rgb * emissive_texel) * uniforms.mtoon_lighting.x;
    if input.outline_color.a >= 0.0 {
        color = input.outline_color.rgb * mix(vec3<f32>(1.0), color, input.outline_color.a);
    }
    return output_color(color, opaque_alpha);
}
"#;
