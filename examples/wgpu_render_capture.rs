//! Offscreen wgpu render capture for render-parity experiments.
//!
//! This example intentionally keeps renderer policy small: it loads real glTF
//! primitive buffers from `vrm-io`, draws them with a fixed camera/light setup,
//! and writes the same RGBA JSON artifact consumed by
//! `tools/render-parity/compare-psnr.mjs`.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use vrm_core::{MtoonAlphaMode, MtoonCullMode};
use vrm_io::{
    GltfPrimitiveData, GltfSkinData, ImageData, ImageFormat, LoadedVrm, load_vrm_from_path,
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    tex_coord: [f32; 2],
    color: [f32; 4],
    shade_color: [f32; 4],
    shading: [f32; 4],
    emissive: [f32; 4],
    alpha_mode: f32,
    _padding: [f32; 3],
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32];

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
}

#[derive(Clone, Debug)]
struct CaptureOptions {
    fixture: PathBuf,
    out: PathBuf,
    png_out: Option<PathBuf>,
    width: u32,
    height: u32,
    camera_y: f32,
    camera_z: f32,
    target_y: f32,
}

#[derive(Clone, Debug)]
struct MeshDrawData {
    primitives: Vec<DrawPrimitive>,
}

#[derive(Clone, Debug)]
struct DrawPrimitive {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    image: Option<usize>,
    policy: MaterialPolicy,
}

struct GpuPrimitive {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    texture_bind_group_index: usize,
    pipeline_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MaterialPolicy {
    render_order: i32,
    cull_mode: CaptureCullMode,
    alpha_mode: CaptureAlphaMode,
    depth_write: bool,
    blend: bool,
}

impl Default for MaterialPolicy {
    fn default() -> Self {
        Self {
            render_order: 2000,
            cull_mode: CaptureCullMode::Back,
            alpha_mode: CaptureAlphaMode::Opaque,
            depth_write: true,
            blend: false,
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

struct TextureBindGroup {
    image: Option<usize>,
    bind_group: wgpu::BindGroup,
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

fn main() -> Result<(), Box<dyn Error>> {
    let options = CaptureOptions::parse()?;
    let loaded = load_vrm_from_path(&options.fixture)?;
    let mesh = mesh_draw_data(&loaded)?;
    let rgba = pollster::block_on(render_capture(&loaded, &mesh, &options))?;

    write_rgba_json(&options, &rgba)?;
    if let Some(path) = &options.png_out {
        write_png(path, options.width, options.height, &rgba)?;
    }
    Ok(())
}

impl CaptureOptions {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        let mut values = HashMap::new();
        let mut index = 0;
        while index < args.len() {
            let key = &args[index];
            if !key.starts_with("--") {
                return Err(format!("unexpected positional argument: {key}").into());
            }
            let Some(value) = args.get(index + 1) else {
                return Err(format!("missing value for {key}").into());
            };
            values.insert(key.trim_start_matches("--").to_string(), value.clone());
            index += 2;
        }

        let fixture = required_path(&values, "fixture")?;
        let out = required_path(&values, "out")?;
        Ok(Self {
            fixture,
            out,
            png_out: values.get("png-out").map(PathBuf::from),
            width: parse_u32(&values, "width", 512)?,
            height: parse_u32(&values, "height", 512)?,
            camera_y: parse_f32(&values, "camera-y", 1.0)?,
            camera_z: parse_f32(&values, "camera-z", 5.0)?,
            target_y: parse_f32(&values, "target-y", 1.0)?,
        })
    }
}

fn required_path(values: &HashMap<String, String>, name: &str) -> Result<PathBuf, Box<dyn Error>> {
    values
        .get(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing --{name}").into())
}

fn parse_u32(
    values: &HashMap<String, String>,
    name: &str,
    default: u32,
) -> Result<u32, Box<dyn Error>> {
    match values.get(name) {
        Some(value) => Ok(value.parse()?),
        None => Ok(default),
    }
}

fn parse_f32(
    values: &HashMap<String, String>,
    name: &str,
    default: f32,
) -> Result<f32, Box<dyn Error>> {
    match values.get(name) {
        Some(value) => Ok(value.parse()?),
        None => Ok(default),
    }
}

fn mesh_draw_data(loaded: &LoadedVrm) -> Result<MeshDrawData, Box<dyn Error>> {
    let mut primitives = Vec::new();

    for node in &loaded.scene.nodes {
        let Some(mesh_index) = node.mesh else {
            continue;
        };
        let Some(mesh) = loaded.meshes.get(mesh_index) else {
            continue;
        };
        let orientation = Mat4::from_rotation_y(std::f32::consts::PI);
        let world = orientation * node.world_matrix;
        let skin_matrices = node
            .skin
            .and_then(|skin| loaded.skins.get(skin))
            .map(|skin| skin_matrices(loaded, skin, orientation));
        for primitive in &mesh.primitives {
            let surface = draw_primitive(loaded, primitive, world, skin_matrices.as_deref())?;
            primitives.push(surface.clone());
            if let Some(outline) = outline_primitive(loaded, primitive, &surface) {
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
) -> Option<DrawPrimitive> {
    let material = primitive
        .material
        .and_then(|index| loaded.model().document().materials.get(index))?;
    let mtoon = material.mtoon.as_ref()?;
    if !mtoon.outline_enabled() {
        return None;
    }
    let color = [
        mtoon.outline_color_factor[0],
        mtoon.outline_color_factor[1],
        mtoon.outline_color_factor[2],
        mtoon.base_color_factor[3],
    ];
    let width_texture = mtoon
        .textures
        .outline_width_multiply_texture
        .and_then(|texture| sampled_image_for_texture(loaded, texture.0));
    let width = mtoon.outline_width_factor;
    let vertices = surface
        .vertices
        .iter()
        .map(|vertex| {
            let normal = Vec3::from_array(vertex.normal).normalize_or_zero();
            let width = width
                * width_texture
                    .as_ref()
                    .map(|image| image.sample_green(vertex.tex_coord))
                    .unwrap_or(1.0);
            Vertex {
                position: (Vec3::from_array(vertex.position) + normal * width).to_array(),
                normal: vertex.normal,
                tex_coord: vertex.tex_coord,
                color,
                shade_color: color,
                shading: [0.0, 0.0, 0.0, 0.0],
                emissive: [0.0, 0.0, 0.0, 0.0],
                alpha_mode: alpha_mode_code(CaptureAlphaMode::Opaque),
                _padding: [0.0; 3],
            }
        })
        .collect();
    Some(DrawPrimitive {
        vertices,
        indices: surface.indices.clone(),
        image: None,
        policy: MaterialPolicy {
            render_order: surface.policy.render_order,
            cull_mode: CaptureCullMode::Front,
            alpha_mode: CaptureAlphaMode::Opaque,
            depth_write: true,
            blend: false,
        },
    })
}

fn draw_primitive(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
) -> Result<DrawPrimitive, Box<dyn Error>> {
    let shading = material_shading(loaded, primitive.material);
    let policy = material_policy(loaded, primitive.material);
    let vertices = primitive
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
            let (position, normal) = transform_vertex(
                Vec3::from_array(*position),
                Vec3::from_array(normal),
                world,
                skin_matrices,
                primitive.joints_0.get(index).copied(),
                primitive.weights_0.get(index).copied(),
            );
            Vertex {
                position: position.to_array(),
                normal: normal.to_array(),
                tex_coord,
                color: shading.base_color,
                shade_color: shading.shade_color,
                shading: [
                    shading.shading_shift,
                    shading.shading_toony,
                    shading.gi_equalization,
                    0.0,
                ],
                emissive: [
                    shading.emissive[0],
                    shading.emissive[1],
                    shading.emissive[2],
                    0.0,
                ],
                alpha_mode: alpha_mode_code(policy.alpha_mode),
                _padding: [0.0; 3],
            }
        })
        .collect();
    Ok(DrawPrimitive {
        vertices,
        indices: primitive.indices.clone(),
        image: material_main_image(loaded, primitive.material),
        policy,
    })
}

fn material_policy(loaded: &LoadedVrm, material: Option<usize>) -> MaterialPolicy {
    material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .map(|mtoon| {
            let hints = mtoon.pipeline_hints();
            MaterialPolicy {
                render_order: hints.render_order,
                cull_mode: capture_cull_mode(hints.cull_mode),
                alpha_mode: capture_alpha_mode(hints.alpha_mode),
                depth_write: hints.depth_write,
                blend: hints.blend,
            }
        })
        .unwrap_or_default()
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

fn skin_matrices(loaded: &LoadedVrm, skin: &GltfSkinData, orientation: Mat4) -> Vec<Mat4> {
    skin.joints
        .iter()
        .enumerate()
        .map(|(index, joint)| {
            let joint_world = loaded
                .scene
                .node(*joint)
                .map(|node| node.world_matrix)
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

#[derive(Clone, Copy, Debug)]
struct MaterialShading {
    base_color: [f32; 4],
    shade_color: [f32; 4],
    shading_shift: f32,
    shading_toony: f32,
    gi_equalization: f32,
    emissive: [f32; 3],
}

fn material_shading(loaded: &LoadedVrm, material: Option<usize>) -> MaterialShading {
    if let Some(shading) = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| {
            let mtoon = material.mtoon.as_ref()?;
            let (emissive_strength, _) = material.effective_emissive_strength();
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
                gi_equalization: mtoon.gi_equalization_factor,
                emissive: [
                    mtoon.emissive_factor[0] * emissive_strength.0,
                    mtoon.emissive_factor[1] * emissive_strength.0,
                    mtoon.emissive_factor[2] * emissive_strength.0,
                ],
            })
        })
    {
        return shading;
    }
    let base_color = material
        .and_then(|index| loaded.gltf_materials.get(index))
        .map(|material| material.base_color_factor)
        .unwrap_or([0.78, 0.78, 0.78, 1.0]);
    MaterialShading {
        base_color,
        shade_color: base_color,
        shading_shift: 0.0,
        shading_toony: 0.0,
        gi_equalization: 0.0,
        emissive: [0.0, 0.0, 0.0],
    }
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

fn texture_bind_groups(
    loaded: &LoadedVrm,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> Result<Vec<TextureBindGroup>, Box<dyn Error>> {
    let mut groups = vec![texture_bind_group(
        device,
        queue,
        layout,
        sampler,
        TextureUpload {
            image: None,
            width: 1,
            height: 1,
            rgba: &[255, 255, 255, 255],
        },
    )];
    for (index, image) in loaded.images.iter().enumerate() {
        let rgba = image_rgba8(image)?;
        groups.push(texture_bind_group(
            device,
            queue,
            layout,
            sampler,
            TextureUpload {
                image: Some(index),
                width: image.width,
                height: image.height,
                rgba: &rgba,
            },
        ));
    }
    Ok(groups)
}

fn texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    upload: TextureUpload<'_>,
) -> TextureBindGroup {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render parity material texture"),
        size: wgpu::Extent3d {
            width: upload.width,
            height: upload.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        upload.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(upload.width * 4),
            rows_per_image: Some(upload.height),
        },
        wgpu::Extent3d {
            width: upload.width,
            height: upload.height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("render parity texture bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    TextureBindGroup {
        image: upload.image,
        bind_group,
    }
}

fn texture_bind_group_indices(groups: &[TextureBindGroup]) -> HashMap<usize, usize> {
    groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| group.image.map(|image| (image, index)))
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
            depth_compare: Some(wgpu::CompareFunction::Less),
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

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
        ..Default::default()
    });
    let texture_bind_groups = texture_bind_groups(
        loaded,
        &device,
        &queue,
        &texture_bind_group_layout,
        &sampler,
    )?;
    let texture_bind_group_indices = texture_bind_group_indices(&texture_bind_groups);
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
    let gpu_primitives = mesh
        .primitives
        .iter()
        .map(|primitive| {
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
                texture_bind_group_index: primitive
                    .image
                    .and_then(|image| texture_bind_group_indices.get(&image).copied())
                    .unwrap_or(0),
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
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
                &texture_bind_groups[primitive.texture_bind_group_index].bind_group,
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
    let eye = Vec3::new(0.0, options.camera_y, -options.camera_z);
    let center = Vec3::new(0.0, options.target_y, 0.0);
    let view = Mat4::look_at_rh(eye, center, Vec3::Y);
    let projection = Mat4::perspective_rh(
        30.0_f32.to_radians(),
        options.width as f32 / options.height as f32,
        0.1,
        20.0,
    );
    let light_dir = Vec3::new(-1.0, -1.0, -1.0).normalize();
    Uniforms {
        view_projection: (projection * view).to_cols_array_2d(),
        light_dir: Vec4::new(light_dir.x, light_dir.y, light_dir.z, 0.0).to_array(),
    }
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
    let artifact = json!({
        "generator": "vrm-rs examples/wgpu_render_capture.rs",
        "fixture": options.fixture.to_string_lossy(),
        "width": options.width,
        "height": options.height,
        "camera": { "y": options.camera_y, "z": options.camera_z, "targetY": options.target_y },
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
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(1) @binding(0)
var base_texture: texture_2d<f32>;

@group(1) @binding(1)
var base_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coord: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) shade_color: vec4<f32>,
    @location(5) shading: vec4<f32>,
    @location(6) emissive: vec4<f32>,
    @location(7) alpha_mode: f32,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) tex_coord: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) shade_color: vec4<f32>,
    @location(4) shading: vec4<f32>,
    @location(5) emissive: vec4<f32>,
    @location(6) alpha_mode: f32,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
    out.normal = normalize(input.normal);
    out.tex_coord = input.tex_coord;
    out.color = input.color;
    out.shade_color = input.shade_color;
    out.shading = input.shading;
    out.emissive = input.emissive;
    out.alpha_mode = input.alpha_mode;
    return out;
}

fn linearstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    return clamp((value - edge0) / max(edge1 - edge0, 0.00001), 0.0, 1.0);
}

const MTOON_REFERENCE_EXPOSURE: f32 = 0.80;

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let ndotl = clamp(dot(normalize(input.normal), normalize(uniforms.light_dir.xyz)), -1.0, 1.0);
    let texel = textureSample(base_texture, base_sampler, input.tex_coord);
    let alpha = input.color.a * texel.a;
    if input.alpha_mode > 0.5 && input.alpha_mode < 1.5 && alpha < 0.5 {
        discard;
    }
    let opaque_alpha = select(alpha, 1.0, input.alpha_mode < 0.5);
    let diffuse = input.color.rgb * texel.rgb;
    let shade = input.shade_color.rgb * texel.rgb;
    let shift = input.shading.x;
    let toony = input.shading.y;
    let gi = input.shading.z;
    let toon = linearstep(-1.0 + toony, 1.0 - toony, ndotl + shift);
    let direct = mix(shade, diffuse, toon);
    let ambient = diffuse * (0.1 + 0.15 * gi);
    let color = (direct + ambient + input.emissive.rgb) * MTOON_REFERENCE_EXPOSURE;
    return vec4<f32>(color, opaque_alpha);
}
"#;
