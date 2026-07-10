//! Interactive wgpu/winit viewer for a VRM avatar plus an optional VRMA clip.
//!
//! This intentionally does not depend on Bevy. It uses `vrm-io` for loading,
//! `vrm-adapter` for renderer-neutral animation application, and `wgpu` for a
//! small unlit textured preview pipeline.

use bytemuck::{Pod, Zeroable};
use clap::Parser;
use glam::{Mat4, Vec2, Vec3};
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use vrm_adapter::{
    HeadlessSceneState, HumanoidPoseRig, MTOON_REFERENCE_WGSL, MtoonGpuMaterial,
    MtoonGpuTextureBindingPlan, MtoonGpuUniform, MtoonMaterializationOptions, MtoonRendererPass,
    MtoonSamplerHint, MtoonTextureSlot, RENDER_OWNER_SAMPLE_OVERRIDE_BINDING,
    RenderOwnerSampleDrawKey, RenderOwnerSamplePass, RenderOwnerSampleSelectionPlan,
    RenderOwnerSampleSurfaceOverride, RenderOwnerSurfaceKey, RenderOwnerSurfaceRelation,
    WorldMatrixAccess, WorldTransformUpdate, apply_vrma_animation_frame_with_look_at,
    mtoon_gpu_materials,
};
use vrm_core::{Feature, MaterialRef, NodeRef, TextureRef, VrmAnimation, VrmDocument};
use vrm_io::{
    CpuRgba8Image, GltfAlphaMode, GltfNodeRest, GltfPrimitiveData, LoadedVrm, load_vrm_from_path,
};
use vrm_runtime::sample_vrm_animation;
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

#[derive(Clone, Debug, Parser)]
#[command(about = "Display a VRM avatar in wgpu and optionally play a VRMA clip")]
pub struct WgpuVrmViewerOptions {
    /// VRM avatar file.
    #[arg(long, default_value = ".external-fixtures/official/Seed-san.vrm")]
    pub avatar: PathBuf,
    /// Optional VRMA animation clip file.
    #[arg(long, default_value = ".external-fixtures/official/idle_loop.vrma")]
    pub animation: PathBuf,
    /// Disable VRMA playback after loading the avatar.
    #[arg(long)]
    pub no_animation: bool,
    /// Playback speed multiplier.
    #[arg(long, default_value_t = 1.0)]
    pub speed: f32,
    /// Camera Z distance.
    #[arg(long, default_value_t = 3.0)]
    pub camera_z: f32,
    /// Camera target height.
    #[arg(long, default_value_t = 1.1)]
    pub look_y: f32,
    /// Initial window width.
    #[arg(long, default_value_t = 1280)]
    pub width: u32,
    /// Initial window height.
    #[arg(long, default_value_t = 720)]
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WgpuMtoonResourcePlan {
    pub material: MaterialRef,
    pub name: Option<String>,
    pub pass: WgpuMtoonPass,
    pub render_order: i32,
    pub phase_order: i32,
    pub uniform: MtoonGpuUniform,
    pub uniform_usage: wgpu::BufferUsages,
    pub shader_source: &'static str,
    pub cull_mode: Option<wgpu::Face>,
    pub front_face: wgpu::FrontFace,
    pub depth_test: bool,
    pub depth_write: bool,
    pub depth_compare: wgpu::CompareFunction,
    pub blend: Option<wgpu::BlendState>,
    pub texture_bindings: Vec<WgpuMtoonTextureBindingPlan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WgpuMtoonPass {
    Base,
    Outline,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WgpuMtoonTextureBindingPlan {
    pub slot: MtoonTextureSlot,
    pub texture: TextureRef,
    pub sampler: WgpuMtoonSamplerPlan,
    pub texture_binding: u32,
    pub sampler_binding: u32,
    pub visibility: wgpu::ShaderStages,
    pub sample_type: wgpu::TextureSampleType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WgpuMtoonSamplerPlan {
    pub mag_filter: wgpu::FilterMode,
    pub min_filter: wgpu::FilterMode,
    pub mipmap_filter: wgpu::FilterMode,
    pub address_mode_u: wgpu::AddressMode,
    pub address_mode_v: wgpu::AddressMode,
    pub normal_map_decode: bool,
}

pub const WGPU_OWNER_SAMPLE_OVERRIDE_RECORD_SIZE: usize =
    std::mem::size_of::<WgpuOwnerSampleOverrideRecord>();

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct WgpuOwnerSampleOverrideRecord {
    pub pixel: [u32; 2],
    pub sample: [f32; 2],
    pub replacement_rgba: [f32; 4],
    pub relation_to_expected: u32,
    pub geometry_flags: u32,
    pub sample_pass: u32,
    pub _padding0: u32,
    pub geometry_ids: [u32; 4],
    pub geometry_indices: [u32; 4],
    pub barycentric_depth: [f32; 4],
    pub geometry_uvs: [f32; 4],
}

impl WgpuOwnerSampleOverrideRecord {
    pub fn from_override(
        value: RenderOwnerSampleSurfaceOverride,
    ) -> Result<Self, WgpuOwnerSampleOverridePlanError> {
        let geometry = wgpu_owner_sample_geometry_record(value.sample_geometry.as_ref())?;
        Ok(Self {
            pixel: [
                u32::try_from(value.pixel.x()).map_err(|_| {
                    WgpuOwnerSampleOverridePlanError::PixelOutOfRange {
                        x: value.pixel.x(),
                        y: value.pixel.y(),
                    }
                })?,
                u32::try_from(value.pixel.y()).map_err(|_| {
                    WgpuOwnerSampleOverridePlanError::PixelOutOfRange {
                        x: value.pixel.x(),
                        y: value.pixel.y(),
                    }
                })?,
            ],
            sample: [value.sample.x() as f32, value.sample.y() as f32],
            replacement_rgba: value
                .replacement_rgba
                .map(|channel| f32::from(channel) / 255.0),
            relation_to_expected: owner_sample_relation_code(value.relation_to_expected),
            geometry_flags: geometry.flags,
            sample_pass: geometry.pass,
            _padding0: 0,
            geometry_ids: geometry.ids,
            geometry_indices: geometry.indices,
            barycentric_depth: geometry.barycentric_depth,
            geometry_uvs: geometry.uvs,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WgpuOwnerSampleGeometryRecord {
    flags: u32,
    pass: u32,
    ids: [u32; 4],
    indices: [u32; 4],
    barycentric_depth: [f32; 4],
    uvs: [f32; 4],
}

fn wgpu_owner_sample_geometry_record(
    geometry: Option<&vrm_adapter::RenderOwnerSampleGeometry>,
) -> Result<WgpuOwnerSampleGeometryRecord, WgpuOwnerSampleOverridePlanError> {
    let Some(geometry) = geometry else {
        return Ok(WgpuOwnerSampleGeometryRecord {
            flags: 0,
            pass: 0,
            ids: [u32::MAX; 4],
            indices: [u32::MAX; 4],
            barycentric_depth: [0.0; 4],
            uvs: [0.0; 4],
        });
    };
    Ok(WgpuOwnerSampleGeometryRecord {
        flags: 1,
        pass: owner_sample_pass_code(&geometry.pass),
        ids: [
            u32_geometry_value("node", geometry.node)?,
            u32_geometry_value("mesh", geometry.mesh)?,
            u32_geometry_value("primitive", geometry.primitive)?,
            u32_geometry_value("triangle", geometry.triangle)?,
        ],
        indices: [
            u32_geometry_value("indices[0]", geometry.indices[0])?,
            u32_geometry_value("indices[1]", geometry.indices[1])?,
            u32_geometry_value("indices[2]", geometry.indices[2])?,
            u32::MAX,
        ],
        barycentric_depth: [
            geometry.barycentric[0] as f32,
            geometry.barycentric[1] as f32,
            geometry.barycentric[2] as f32,
            geometry.depth as f32,
        ],
        uvs: [
            geometry.raw_uv[0] as f32,
            geometry.raw_uv[1] as f32,
            geometry.base_uv[0] as f32,
            geometry.base_uv[1] as f32,
        ],
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct WgpuOwnerSampleOverrideBufferPlan {
    pub surface: RenderOwnerSurfaceKey,
    pub records: Vec<WgpuOwnerSampleOverrideRecord>,
    pub binding: u32,
    pub usage: wgpu::BufferUsages,
    pub visibility: wgpu::ShaderStages,
    pub binding_type: wgpu::BufferBindingType,
}

impl WgpuOwnerSampleOverrideBufferPlan {
    pub fn bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.records)
    }

    pub fn record_count(&self) -> usize {
        self.records.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WgpuOwnerSampleOverridePlanError {
    PixelOutOfRange { x: u64, y: u64 },
    GeometryIndexOutOfRange { field: &'static str, value: u64 },
}

pub const fn wgpu_owner_sample_override_binding() -> u32 {
    RENDER_OWNER_SAMPLE_OVERRIDE_BINDING
}

pub fn wgpu_owner_sample_override_bind_group_layout_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: wgpu_owner_sample_override_binding(),
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub fn wgpu_owner_sample_override_buffer_plans(
    selection: &RenderOwnerSampleSelectionPlan,
) -> Result<Vec<WgpuOwnerSampleOverrideBufferPlan>, WgpuOwnerSampleOverridePlanError> {
    selection
        .surfaces
        .iter()
        .map(|surface| {
            Ok(WgpuOwnerSampleOverrideBufferPlan {
                surface: surface.surface.clone(),
                records: surface
                    .overrides()
                    .map(WgpuOwnerSampleOverrideRecord::from_override)
                    .collect::<Result<Vec<_>, _>>()?,
                binding: wgpu_owner_sample_override_binding(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                visibility: wgpu::ShaderStages::FRAGMENT,
                binding_type: wgpu::BufferBindingType::Storage { read_only: true },
            })
        })
        .collect()
}

pub fn wgpu_owner_sample_override_buffer_plan_for_surfaces_and_draw<'a, I, S>(
    selection: &RenderOwnerSampleSelectionPlan,
    surfaces: I,
    draw: &RenderOwnerSampleDrawKey,
) -> Result<WgpuOwnerSampleOverrideBufferPlan, WgpuOwnerSampleOverridePlanError>
where
    I: IntoIterator<Item = S>,
    S: std::borrow::Borrow<RenderOwnerSurfaceKey> + 'a,
{
    let surfaces = surfaces
        .into_iter()
        .map(|surface| surface.borrow().clone())
        .collect::<Vec<_>>();
    let records = surfaces
        .iter()
        .flat_map(|surface| selection.overrides_for_surface_and_draw(surface, draw))
        .map(WgpuOwnerSampleOverrideRecord::from_override)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WgpuOwnerSampleOverrideBufferPlan {
        surface: surfaces
            .first()
            .cloned()
            .unwrap_or_else(|| RenderOwnerSurfaceKey::new("", 0)),
        records,
        binding: wgpu_owner_sample_override_binding(),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        visibility: wgpu::ShaderStages::FRAGMENT,
        binding_type: wgpu::BufferBindingType::Storage { read_only: true },
    })
}

pub fn wgpu_mtoon_resource_plans(
    document: &VrmDocument,
    options: MtoonMaterializationOptions,
) -> Vec<WgpuMtoonResourcePlan> {
    mtoon_gpu_materials(document, options)
        .into_iter()
        .map(wgpu_mtoon_resource_plan)
        .collect()
}

pub fn wgpu_mtoon_resource_plan(material: MtoonGpuMaterial) -> WgpuMtoonResourcePlan {
    WgpuMtoonResourcePlan {
        material: material.material,
        name: material.name,
        pass: wgpu_mtoon_pass(material.pass),
        render_order: material.pipeline.render_order,
        phase_order: material.pipeline.phase_order,
        uniform: material.uniform,
        uniform_usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        shader_source: MTOON_REFERENCE_WGSL,
        cull_mode: wgpu_cull_mode(material.pipeline.cull_mode),
        front_face: wgpu::FrontFace::Ccw,
        depth_test: material.pipeline.depth_test,
        depth_write: material.pipeline.depth_write,
        depth_compare: if material.pipeline.depth_test {
            wgpu::CompareFunction::LessEqual
        } else {
            wgpu::CompareFunction::Always
        },
        blend: material
            .pipeline
            .blend
            .then_some(wgpu::BlendState::ALPHA_BLENDING),
        texture_bindings: material
            .texture_bindings
            .iter()
            .copied()
            .map(wgpu_mtoon_texture_binding_plan)
            .collect(),
    }
}

fn wgpu_mtoon_pass(pass: MtoonRendererPass) -> WgpuMtoonPass {
    match pass {
        MtoonRendererPass::Base => WgpuMtoonPass::Base,
        MtoonRendererPass::Outline => WgpuMtoonPass::Outline,
    }
}

fn wgpu_cull_mode(mode: vrm_core::MtoonCullMode) -> Option<wgpu::Face> {
    match mode {
        vrm_core::MtoonCullMode::Off => None,
        vrm_core::MtoonCullMode::Front => Some(wgpu::Face::Front),
        vrm_core::MtoonCullMode::Back => Some(wgpu::Face::Back),
    }
}

fn wgpu_mtoon_texture_binding_plan(
    binding: MtoonGpuTextureBindingPlan,
) -> WgpuMtoonTextureBindingPlan {
    WgpuMtoonTextureBindingPlan {
        slot: binding.slot,
        texture: binding.texture,
        sampler: wgpu_mtoon_sampler_plan(binding.sampler),
        texture_binding: binding.texture_binding,
        sampler_binding: binding.sampler_binding,
        visibility: wgpu_texture_visibility(binding.slot),
        sample_type: wgpu_texture_sample_type(binding.sampler),
    }
}

fn wgpu_texture_visibility(slot: MtoonTextureSlot) -> wgpu::ShaderStages {
    match slot {
        MtoonTextureSlot::OutlineWidth => wgpu::ShaderStages::VERTEX_FRAGMENT,
        MtoonTextureSlot::Main
        | MtoonTextureSlot::ShadeMultiply
        | MtoonTextureSlot::ShadingShift
        | MtoonTextureSlot::Normal
        | MtoonTextureSlot::Matcap
        | MtoonTextureSlot::RimMultiply
        | MtoonTextureSlot::UvAnimationMask => wgpu::ShaderStages::FRAGMENT,
    }
}

fn wgpu_texture_sample_type(sampler: MtoonSamplerHint) -> wgpu::TextureSampleType {
    match sampler {
        MtoonSamplerHint::LinearRepeat => wgpu::TextureSampleType::Float { filterable: true },
        MtoonSamplerHint::NormalMapLinearRepeat => {
            wgpu::TextureSampleType::Float { filterable: true }
        }
    }
}

fn wgpu_mtoon_sampler_plan(sampler: MtoonSamplerHint) -> WgpuMtoonSamplerPlan {
    WgpuMtoonSamplerPlan {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        normal_map_decode: matches!(sampler, MtoonSamplerHint::NormalMapLinearRepeat),
    }
}

fn owner_sample_relation_code(relation: Option<RenderOwnerSurfaceRelation>) -> u32 {
    match relation {
        Some(RenderOwnerSurfaceRelation::SameSurface) => 1,
        Some(RenderOwnerSurfaceRelation::SameMaterialDifferentTriangle) => 2,
        Some(RenderOwnerSurfaceRelation::DifferentMaterial) => 3,
        Some(RenderOwnerSurfaceRelation::Missing) => 4,
        None => 0,
    }
}

fn owner_sample_pass_code(pass: &RenderOwnerSamplePass) -> u32 {
    match pass {
        RenderOwnerSamplePass::Base => 1,
        RenderOwnerSamplePass::Outline => 2,
        RenderOwnerSamplePass::Other(_) => 255,
    }
}

fn u32_geometry_value(
    field: &'static str,
    value: u64,
) -> Result<u32, WgpuOwnerSampleOverridePlanError> {
    u32::try_from(value)
        .map_err(|_| WgpuOwnerSampleOverridePlanError::GeometryIndexOutOfRange { field, value })
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    tex_coord: [f32; 2],
    color: [f32; 4],
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];

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
}

#[derive(Clone, Copy, Debug)]
struct OrbitCamera {
    target: Vec3,
    home_target: Vec3,
    radius: f32,
    home_radius: f32,
    yaw: f32,
    home_yaw: f32,
    pitch: f32,
    home_pitch: f32,
}

impl OrbitCamera {
    fn new(target: Vec3, radius: f32) -> Self {
        Self {
            target,
            home_target: target,
            radius,
            home_radius: radius,
            yaw: 0.0,
            home_yaw: 0.0,
            pitch: 0.0,
            home_pitch: 0.0,
        }
    }

    fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.006;
        self.pitch = (self.pitch + delta.y * 0.006).clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
    }

    fn pan(&mut self, delta: Vec2) {
        let view = Mat4::look_at_rh(self.position(), self.target, Vec3::Y);
        let world_from_view = view.inverse();
        let right = world_from_view.transform_vector3(Vec3::X);
        let up = world_from_view.transform_vector3(Vec3::Y);
        self.target += (-right * delta.x + up * delta.y) * self.radius * 0.0015;
    }

    fn zoom(&mut self, scroll_lines: f32) {
        self.radius = (self.radius * (-scroll_lines * 0.12).exp()).clamp(0.4, 20.0);
    }

    fn reset(&mut self) {
        self.target = self.home_target;
        self.radius = self.home_radius;
        self.yaw = self.home_yaw;
        self.pitch = self.home_pitch;
    }

    fn position(self) -> Vec3 {
        let cos_pitch = self.pitch.cos();
        self.target
            + Vec3::new(
                self.radius * cos_pitch * self.yaw.sin(),
                self.radius * self.pitch.sin(),
                self.radius * cos_pitch * self.yaw.cos(),
            )
    }

    fn view_projection(self, aspect: f32) -> Mat4 {
        let projection =
            Mat4::perspective_rh(35.0_f32.to_radians(), aspect.max(0.001), 0.01, 100.0);
        let view = Mat4::look_at_rh(self.position(), self.target, Vec3::Y);
        projection * view
    }
}

struct AvatarRuntime {
    loaded: LoadedVrm,
    scene: HeadlessSceneState,
    rig: HumanoidPoseRig,
    animation: Option<VrmAnimation>,
    animation_time: f32,
    animation_speed: f32,
}

impl AvatarRuntime {
    fn new(
        loaded: LoadedVrm,
        animation: Option<VrmAnimation>,
        animation_speed: f32,
    ) -> Result<Self, Box<dyn Error>> {
        let mut scene = headless_scene_from_loaded(&loaded)?;
        scene.update_world_transforms()?;
        let rig = HumanoidPoseRig::capture(&scene, loaded.model().document())?;
        Ok(Self {
            loaded,
            scene,
            rig,
            animation,
            animation_time: 0.0,
            animation_speed,
        })
    }

    fn update(&mut self, delta_seconds: f32) -> Result<(), Box<dyn Error>> {
        if let Some(animation) = &self.animation {
            self.animation_time += delta_seconds * self.animation_speed;
            let time = if animation.duration > f32::EPSILON {
                self.animation_time.rem_euclid(animation.duration)
            } else {
                0.0
            };
            let frame = sample_vrm_animation(animation, time);
            apply_vrma_animation_frame_with_look_at(
                &mut self.scene,
                &mut self.rig,
                self.loaded.model().document(),
                &frame,
            )?;
        }
        self.scene.update_world_transforms()?;
        Ok(())
    }

    fn world_matrices(&self) -> Vec<Mat4> {
        self.loaded
            .scene
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                self.scene
                    .world_matrix(NodeRef(index))
                    .unwrap_or(node.world_matrix)
            })
            .collect()
    }
}

struct GpuMaterial {
    bind_group: wgpu::BindGroup,
}

struct GpuPrimitive {
    node: usize,
    mesh: usize,
    primitive: usize,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    material: usize,
}

struct WgpuViewer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    depth_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    materials: Vec<GpuMaterial>,
    primitives: Vec<GpuPrimitive>,
    avatar: AvatarRuntime,
    camera: OrbitCamera,
    last_frame: Instant,
}

impl WgpuViewer {
    async fn new(
        window: Arc<Window>,
        options: &WgpuVrmViewerOptions,
        avatar: LoadedVrm,
        animation: Option<VrmAnimation>,
    ) -> Result<Self, Box<dyn Error>> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window)?;
        Self::new_from_surface(surface, size, options, avatar, animation).await
    }

    async fn new_from_surface(
        surface: wgpu::Surface<'static>,
        size: PhysicalSize<u32>,
        options: &WgpuVrmViewerOptions,
        avatar: LoadedVrm,
        animation: Option<VrmAnimation>,
    ) -> Result<Self, Box<dyn Error>> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("vrm-rs wgpu viewer device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await?;
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface is not supported by the selected adapter")?;
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vrm-rs wgpu viewer shader"),
            source: wgpu::ShaderSource::Wgsl(VIEWER_SHADER.into()),
        });
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vrm-rs wgpu viewer uniforms"),
            contents: bytemuck::bytes_of(&Uniforms {
                view_projection: Mat4::IDENTITY.to_cols_array_2d(),
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("vrm-rs wgpu viewer uniform layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vrm-rs wgpu viewer uniform bind group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let material_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("vrm-rs wgpu viewer material layout"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vrm-rs wgpu viewer pipeline layout"),
            bind_group_layouts: &[
                Some(&uniform_bind_group_layout),
                Some(&material_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vrm-rs wgpu viewer pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Vertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let depth_view = create_depth_view(&device, &config);
        let mut avatar = AvatarRuntime::new(avatar, animation, options.speed)?;
        avatar.update(0.0)?;
        let materials =
            create_materials(&device, &queue, &material_bind_group_layout, &avatar.loaded);
        let primitives = create_primitives(&device, &avatar)?;
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            depth_view,
            uniform_buffer,
            uniform_bind_group,
            materials,
            primitives,
            avatar,
            camera: OrbitCamera::new(Vec3::new(0.0, options.look_y, 0.0), options.camera_z),
            last_frame: Instant::now(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, &self.config);
    }

    fn update(&mut self) -> Result<(), Box<dyn Error>> {
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.avatar.update(delta)?;
        update_primitive_buffers(&self.device, &mut self.primitives, &self.avatar)?;
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&Uniforms {
                view_projection: self.camera.view_projection(aspect).to_cols_array_2d(),
            }),
        );
        Ok(())
    }

    fn render(&mut self) -> RenderOutcome {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return RenderOutcome::Skip;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                return RenderOutcome::Reconfigure;
            }
            wgpu::CurrentSurfaceTexture::Validation => return RenderOutcome::ValidationError,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vrm-rs wgpu viewer encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vrm-rs wgpu viewer render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.025,
                            b: 0.03,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            for primitive in &self.primitives {
                let material = self
                    .materials
                    .get(primitive.material)
                    .unwrap_or_else(|| &self.materials[0]);
                pass.set_bind_group(1, &material.bind_group, &[]);
                pass.set_vertex_buffer(0, primitive.vertex_buffer.slice(..));
                pass.set_index_buffer(primitive.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..primitive.index_count, 0, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
        RenderOutcome::Rendered
    }
}

#[cfg(target_arch = "wasm32")]
pub struct WgpuCanvasViewer {
    inner: WgpuViewer,
}

#[cfg(target_arch = "wasm32")]
impl WgpuCanvasViewer {
    pub async fn new(
        canvas: web_sys::HtmlCanvasElement,
        options: &WgpuVrmViewerOptions,
        avatar: LoadedVrm,
        animation: Option<VrmAnimation>,
    ) -> Result<Self, Box<dyn Error>> {
        let size = PhysicalSize::new(canvas.width().max(1), canvas.height().max(1));
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas))?;
        let inner = WgpuViewer::new_from_surface(surface, size, options, avatar, animation).await?;
        Ok(Self { inner })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.inner
            .resize(PhysicalSize::new(width.max(1), height.max(1)));
    }

    pub fn render_frame(&mut self) -> Result<bool, Box<dyn Error>> {
        self.inner.update()?;
        match self.inner.render() {
            RenderOutcome::Rendered => Ok(true),
            RenderOutcome::Skip => Ok(false),
            RenderOutcome::Reconfigure => {
                self.inner.resize(PhysicalSize::new(
                    self.inner.config.width,
                    self.inner.config.height,
                ));
                Ok(false)
            }
            RenderOutcome::ValidationError => {
                Err("failed to acquire a valid wgpu surface texture".into())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderOutcome {
    Rendered,
    Skip,
    Reconfigure,
    ValidationError,
}

struct App {
    options: WgpuVrmViewerOptions,
    avatar: Option<LoadedVrm>,
    animation: Option<VrmAnimation>,
    window: Option<Arc<Window>>,
    viewer: Option<WgpuViewer>,
    last_cursor: Option<PhysicalPosition<f64>>,
    left_drag: bool,
    pan_drag: bool,
}

impl App {
    fn new(options: WgpuVrmViewerOptions) -> Result<Self, Box<dyn Error>> {
        let avatar = load_vrm_from_path(&options.avatar)?;
        let animation = if options.no_animation {
            None
        } else {
            let loaded = load_vrm_from_path(&options.animation)?;
            animation_from_loaded(&loaded)
        };
        Ok(Self {
            options,
            avatar: Some(avatar),
            animation,
            window: None,
            viewer: None,
            last_cursor: None,
            left_drag: false,
            pan_drag: false,
        })
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title("vrm-rs wgpu VRMA Viewer")
            .with_inner_size(PhysicalSize::new(self.options.width, self.options.height));
        let window = Arc::new(event_loop.create_window(attributes).expect("create window"));
        let avatar = self.avatar.take().expect("avatar should be loaded once");
        let animation = self.animation.take();
        let viewer = pollster::block_on(WgpuViewer::new(
            Arc::clone(&window),
            &self.options,
            avatar,
            animation,
        ))
        .expect("initialize wgpu viewer");
        self.viewer = Some(viewer);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = &self.window else {
            return;
        };
        if window.id() != window_id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(viewer) = &mut self.viewer {
                    viewer.resize(size);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (Some(previous), Some(viewer)) = (self.last_cursor, &mut self.viewer) {
                    let delta = Vec2::new(
                        (position.x - previous.x) as f32,
                        (position.y - previous.y) as f32,
                    );
                    if self.left_drag {
                        viewer.camera.orbit(delta);
                    }
                    if self.pan_drag {
                        viewer.camera.pan(delta);
                    }
                }
                self.last_cursor = Some(position);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => self.left_drag = pressed,
                    MouseButton::Right | MouseButton::Middle => self.pan_drag = pressed,
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(viewer) = &mut self.viewer {
                    let lines = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(position) => position.y as f32 / 100.0,
                    };
                    viewer.camera.zoom(lines);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let Some(viewer) = &mut self.viewer {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::KeyF) => {
                            viewer.camera.target = Vec3::new(0.0, self.options.look_y, 0.0);
                        }
                        PhysicalKey::Code(KeyCode::KeyR) => viewer.camera.reset(),
                        _ => {}
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(viewer) = &mut self.viewer {
                    if let Err(error) = viewer.update() {
                        eprintln!("failed to update VRM viewer: {error}");
                    }
                    match viewer.render() {
                        RenderOutcome::Rendered | RenderOutcome::Skip => {}
                        RenderOutcome::Reconfigure => {
                            viewer.resize(PhysicalSize::new(
                                viewer.config.width,
                                viewer.config.height,
                            ));
                        }
                        RenderOutcome::ValidationError => {
                            eprintln!("failed to acquire a valid wgpu surface texture");
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

pub fn run_vrma_viewer(options: WgpuVrmViewerOptions) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    let mut app = App::new(options)?;
    event_loop.run_app(&mut app)?;
    Ok(())
}

pub fn animation_from_loaded(loaded: &LoadedVrm) -> Option<VrmAnimation> {
    match &loaded.model().document().animation {
        Feature::Present(animation) => Some(animation.clone()),
        Feature::Absent => loaded.model().document().animations.first().cloned(),
    }
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("vrm-rs wgpu viewer depth"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_materials(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    loaded: &LoadedVrm,
) -> Vec<GpuMaterial> {
    let mut materials = Vec::with_capacity(loaded.gltf_materials.len().max(1));
    materials.push(create_material(device, queue, layout, None));
    for index in 0..loaded.gltf_materials.len() {
        materials.push(create_material(
            device,
            queue,
            layout,
            loaded.material_base_texture_rgba8_image(Some(index)),
        ));
    }
    materials
}

fn create_material(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    image: Option<CpuRgba8Image>,
) -> GpuMaterial {
    let image = image.unwrap_or(CpuRgba8Image {
        width: 1,
        height: 1,
        rgba: vec![255, 255, 255, 255],
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vrm-rs wgpu viewer material texture"),
        size: wgpu::Extent3d {
            width: image.width,
            height: image.height,
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
        &image.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width * 4),
            rows_per_image: Some(image.height),
        },
        wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("vrm-rs wgpu viewer material sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("vrm-rs wgpu viewer material bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    GpuMaterial { bind_group }
}

fn create_primitives(
    device: &wgpu::Device,
    avatar: &AvatarRuntime,
) -> Result<Vec<GpuPrimitive>, Box<dyn Error>> {
    let mut primitives = Vec::new();
    for (node_index, node) in avatar.loaded.scene.nodes.iter().enumerate() {
        let Some(mesh_index) = node.mesh else {
            continue;
        };
        let mesh = &avatar.loaded.meshes[mesh_index];
        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            let (vertices, indices) = primitive_vertices(avatar, node_index, primitive)?;
            primitives.push(GpuPrimitive {
                node: node_index,
                mesh: mesh_index,
                primitive: primitive_index,
                vertex_buffer: vertex_buffer(device, &vertices),
                index_buffer: index_buffer(device, &indices),
                index_count: indices.len() as u32,
                material: primitive.material.map(|index| index + 1).unwrap_or(0),
            });
        }
    }
    Ok(primitives)
}

fn update_primitive_buffers(
    device: &wgpu::Device,
    primitives: &mut [GpuPrimitive],
    avatar: &AvatarRuntime,
) -> Result<(), Box<dyn Error>> {
    for primitive in primitives {
        let gltf_primitive = &avatar.loaded.meshes[primitive.mesh].primitives[primitive.primitive];
        let (vertices, indices) = primitive_vertices(avatar, primitive.node, gltf_primitive)?;
        primitive.vertex_buffer = vertex_buffer(device, &vertices);
        primitive.index_buffer = index_buffer(device, &indices);
        primitive.index_count = indices.len() as u32;
    }
    Ok(())
}

fn primitive_vertices(
    avatar: &AvatarRuntime,
    node_index: usize,
    primitive: &GltfPrimitiveData,
) -> Result<(Vec<Vertex>, Vec<u32>), Box<dyn Error>> {
    let node = &avatar.loaded.scene.nodes[node_index];
    let mesh = &avatar.loaded.meshes[node.mesh.ok_or("node has no mesh")?];
    let morph_weights = active_morph_weights(&avatar.scene, node_index, node, mesh);
    let world_matrices = avatar.world_matrices();
    let orientation = Mat4::from_rotation_y(std::f32::consts::PI);
    let world = orientation * world_matrices[node_index];
    let skin_matrices = node.skin.and_then(|skin| {
        avatar
            .loaded
            .skins
            .get(skin)
            .map(|skin| skin.joint_matrices(&avatar.loaded.scene, &world_matrices, orientation))
    });
    let material = primitive
        .material
        .and_then(|index| avatar.loaded.gltf_materials.get(index));
    let base_color = material
        .map(|material| material.base_color_factor)
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let alpha_enabled = material
        .map(|material| material.alpha_mode != GltfAlphaMode::Opaque)
        .unwrap_or(false);
    let vertices = primitive
        .transformed_vertices(&morph_weights, world, skin_matrices.as_deref())
        .ok_or("primitive geometry is inconsistent")?
        .into_iter()
        .map(|vertex| {
            let alpha = if alpha_enabled {
                base_color[3] * vertex.color_0[3]
            } else {
                1.0
            };
            Vertex {
                position: vertex.position.to_array(),
                tex_coord: vertex.tex_coord_0,
                color: [
                    base_color[0] * vertex.color_0[0],
                    base_color[1] * vertex.color_0[1],
                    base_color[2] * vertex.color_0[2],
                    alpha,
                ],
            }
        })
        .collect();
    Ok((vertices, primitive.indices.clone()))
}

fn vertex_buffer(device: &wgpu::Device, vertices: &[Vertex]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vrm-rs wgpu viewer vertices"),
        contents: bytemuck::cast_slice(vertices),
        usage: wgpu::BufferUsages::VERTEX,
    })
}

fn index_buffer(device: &wgpu::Device, indices: &[u32]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vrm-rs wgpu viewer indices"),
        contents: bytemuck::cast_slice(indices),
        usage: wgpu::BufferUsages::INDEX,
    })
}

fn active_morph_weights(
    scene: &HeadlessSceneState,
    node_index: usize,
    node: &GltfNodeRest,
    mesh: &vrm_io::GltfMeshData,
) -> Vec<f32> {
    let mut weights = if node.weights.is_empty() {
        mesh.weights.clone()
    } else {
        node.weights.clone()
    };
    for index in 0..mesh
        .primitives
        .iter()
        .map(|primitive| primitive.morph_targets.len())
        .max()
        .unwrap_or(0)
    {
        if let Some(weight) = scene.morph_weight(NodeRef(node_index), index) {
            if weights.len() <= index {
                weights.resize(index + 1, 0.0);
            }
            weights[index] = weight;
        }
    }
    weights
}

fn headless_scene_from_loaded(loaded: &LoadedVrm) -> Result<HeadlessSceneState, Box<dyn Error>> {
    let mut scene = HeadlessSceneState::default();
    for (index, node) in loaded.scene.nodes.iter().enumerate() {
        scene.insert_node(NodeRef(index), node.local);
    }
    for (index, node) in loaded.scene.nodes.iter().enumerate() {
        scene.set_parent(NodeRef(index), node.parent.map(NodeRef))?;
    }
    scene.update_world_transforms()?;
    Ok(scene)
}

const VIEWER_SHADER: &str = r#"
struct Uniforms {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(1) @binding(0)
var base_texture: texture_2d<f32>;
@group(1) @binding(1)
var base_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coord: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
    output.tex_coord = input.tex_coord;
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let sampled = textureSample(base_texture, base_sampler, input.tex_coord);
    return sampled * input.color;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use vrm_core::{
        EmissiveStrength, Feature, Material, MtoonCullMode, MtoonMaterial, MtoonRenderQueue,
        MtoonTextureSet,
    };

    fn sample_document() -> VrmDocument {
        VrmDocument {
            materials: vec![Material {
                khr_emissive_strength: Feature::Present(EmissiveStrength(2.0)),
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    transparent_with_z_write: true,
                    cull_mode: MtoonCullMode::Off,
                    base_color_factor: [0.2, 0.4, 0.6, 0.5],
                    emissive_factor: [0.1, 0.2, 0.3],
                    textures: MtoonTextureSet {
                        main_texture: Some(TextureRef(1)),
                        normal_texture: Some(TextureRef(2)),
                        outline_width_multiply_texture: Some(TextureRef(3)),
                        ..MtoonTextureSet::default()
                    },
                    ..MtoonMaterial::default()
                }),
                ..Material::default()
            }],
            ..VrmDocument::default()
        }
    }

    #[test]
    fn orbit_camera_vertical_drag_follows_pointer_direction() {
        let mut camera = OrbitCamera::new(Vec3::ZERO, 3.0);

        camera.orbit(Vec2::new(0.0, 10.0));

        assert!(camera.position().y > camera.target.y);
    }

    #[test]
    fn mtoon_resource_plans_expose_wgpu_state_and_uniforms() {
        let plans = wgpu_mtoon_resource_plans(&sample_document(), Default::default());

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].pass, WgpuMtoonPass::Base);
        assert_eq!(plans[0].render_order, 3000);
        assert_eq!(plans[0].phase_order, 0);
        assert_eq!(plans[0].cull_mode, None);
        assert_eq!(plans[0].depth_compare, wgpu::CompareFunction::LessEqual);
        assert_eq!(plans[0].blend, Some(wgpu::BlendState::ALPHA_BLENDING));
        assert_eq!(
            plans[0].uniform.emissive_color_outline_width[0..3],
            [0.2, 0.4, 0.6]
        );
        assert!(plans[0].shader_source.contains("MtoonGpuUniform"));
        assert!(
            plans[0]
                .uniform_usage
                .contains(wgpu::BufferUsages::COPY_DST)
        );
    }

    #[test]
    fn mtoon_resource_plans_map_texture_sampler_bindings() {
        let plans = wgpu_mtoon_resource_plans(&sample_document(), Default::default());
        let bindings = &plans[0].texture_bindings;

        assert_eq!(bindings.len(), 3);
        assert_eq!(bindings[0].slot, MtoonTextureSlot::Main);
        assert_eq!(bindings[0].texture_binding, 1);
        assert_eq!(bindings[0].sampler_binding, 2);
        assert_eq!(bindings[1].slot, MtoonTextureSlot::Normal);
        assert!(bindings[1].sampler.normal_map_decode);
        assert_eq!(bindings[2].slot, MtoonTextureSlot::OutlineWidth);
        assert!(bindings[2].visibility.contains(wgpu::ShaderStages::VERTEX));
    }

    #[test]
    fn owner_sample_override_buffer_plans_are_storage_ready() {
        let surface = RenderOwnerSurfaceKey::new("body", 7);
        let plan = RenderOwnerSampleSelectionPlan {
            surfaces: vec![vrm_adapter::RenderOwnerSampleSurfaceSelection {
                surface: surface.clone(),
                entries: vec![vrm_adapter::RenderOwnerSampleCorrectionManifestEntry {
                    correction: vrm_adapter::RenderRgba8Correction::new(
                        vrm_adapter::RenderPixel::new(12, 34),
                        [64, 128, 255, 255],
                    ),
                    sample: vrm_adapter::RenderOwnerSampleKey::from_pair(
                        surface.clone(),
                        [0.25, 0.75],
                    ),
                    selection_source: None,
                    relation_to_expected: Some(RenderOwnerSurfaceRelation::SameSurface),
                    sample_geometry: Some(owner_sample_geometry()),
                }],
            }],
            unmatched_entries: Vec::new(),
        };

        let buffers = wgpu_owner_sample_override_buffer_plans(&plan).unwrap();

        assert_eq!(buffers.len(), 1);
        assert_eq!(buffers[0].surface, surface);
        assert_eq!(buffers[0].record_count(), 1);
        assert_eq!(buffers[0].binding, wgpu_owner_sample_override_binding());
        assert!(buffers[0].binding > 19);
        assert!(buffers[0].usage.contains(wgpu::BufferUsages::STORAGE));
        assert!(buffers[0].visibility.contains(wgpu::ShaderStages::FRAGMENT));
        assert_eq!(
            buffers[0].binding_type,
            wgpu::BufferBindingType::Storage { read_only: true }
        );
        assert_eq!(buffers[0].records[0].pixel, [12, 34]);
        assert_eq!(buffers[0].records[0].sample, [0.25, 0.75]);
        assert_eq!(buffers[0].records[0].replacement_rgba[2], 1.0);
        assert_eq!(buffers[0].records[0].relation_to_expected, 1);
        assert_eq!(buffers[0].records[0].geometry_flags, 1);
        assert_eq!(buffers[0].records[0].sample_pass, 1);
        assert_eq!(buffers[0].records[0].geometry_ids, [2, 3, 4, 7]);
        assert_eq!(
            buffers[0].records[0].geometry_indices,
            [10, 11, 12, u32::MAX]
        );
        assert_eq!(
            buffers[0].records[0].barycentric_depth,
            [0.2, 0.3, 0.5, 0.42]
        );
        assert_eq!(buffers[0].records[0].geometry_uvs, [0.1, 0.2, 0.7, 0.8]);
        assert_eq!(
            buffers[0].bytes().len(),
            WGPU_OWNER_SAMPLE_OVERRIDE_RECORD_SIZE
        );
        let layout_entry = wgpu_owner_sample_override_bind_group_layout_entry();
        assert_eq!(layout_entry.binding, wgpu_owner_sample_override_binding());
        assert!(
            layout_entry
                .visibility
                .contains(wgpu::ShaderStages::FRAGMENT)
        );
        assert_eq!(
            layout_entry.ty,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            }
        );
        let matching_draw = RenderOwnerSampleDrawKey::new(2, 3, 4, RenderOwnerSamplePass::Base);
        let draw_plan = wgpu_owner_sample_override_buffer_plan_for_surfaces_and_draw(
            &plan,
            [RenderOwnerSurfaceKey::new("body", 7)],
            &matching_draw,
        )
        .unwrap();
        assert_eq!(draw_plan.record_count(), 1);
        let other_draw = RenderOwnerSampleDrawKey::new(9, 3, 4, RenderOwnerSamplePass::Base);
        let filtered_plan = wgpu_owner_sample_override_buffer_plan_for_surfaces_and_draw(
            &plan,
            [RenderOwnerSurfaceKey::new("body", 7)],
            &other_draw,
        )
        .unwrap();
        assert_eq!(filtered_plan.record_count(), 0);
    }

    fn owner_sample_geometry() -> vrm_adapter::RenderOwnerSampleGeometry {
        vrm_adapter::RenderOwnerSampleGeometry {
            node: 2,
            mesh: 3,
            primitive: 4,
            triangle: 7,
            indices: [10, 11, 12],
            barycentric: [0.2, 0.3, 0.5],
            raw_uv: [0.1, 0.2],
            base_uv: [0.7, 0.8],
            depth: 0.42,
            pass: RenderOwnerSamplePass::Base,
        }
    }
}
