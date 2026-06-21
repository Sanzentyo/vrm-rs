//! Ash/Vulkan-shaped frame planning for VRM assets.
//!
//! This crate intentionally does not create Vulkan instances, devices, swapchains,
//! or shader modules. It keeps the unsafe Vulkan boundary in the downstream ash
//! application while providing renderer-ready CPU vertices, indices, texture
//! uploads, and `ash::vk`-typed pipeline/descriptor plans.

use ash::vk;
use bytemuck::{Pod, Zeroable};
use clap::{Parser, ValueEnum};
use glam::{Mat4, Vec2, Vec3, Vec4};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    path::PathBuf,
    sync::Arc,
};
use vrm_adapter::{
    GltfMaterialAlphaMode, GltfMaterialPipelineOverride, HeadlessSceneState, HumanoidPoseRig,
    MTOON_GPU_UNIFORM_SIZE, MtoonGpuMaterial, MtoonGpuUniform, MtoonLightAccumulation,
    MtoonLightingConfig, MtoonMaterializationOptions, MtoonRendererPass, MtoonSamplerHint,
    MtoonTextureSlot, RENDER_OWNER_SAMPLE_OVERRIDE_BINDING, RenderOwnerId,
    RenderOwnerSampleDrawKey, RenderOwnerSamplePass, RenderOwnerSampleSelectionPlan,
    RenderOwnerSampleSurfaceOverride, RenderOwnerSurfaceKey, RenderOwnerSurfaceRelation,
    RendererFrontFace, RendererMaterialAlphaMode, RendererMaterialCullMode, ScreenProjectionBounds,
    ScreenProjectionSize, ScreenTriangleProjection, WorldMatrixAccess, WorldTransformUpdate,
    ZeroToOneDepth, apply_vrma_animation_frame_with_look_at, mtoon_renderer_material_plans,
    project_triangle_to_screen, renderer_material_pipeline_plan,
};
use vrm_core::{Feature, MaterialRef, MtoonAlphaMode, NodeRef, TextureRef, VrmAnimation};
use vrm_io::{
    CpuRgba8Image, GltfAlphaMode, GltfExpressionRenderEffects, GltfMagFilter,
    GltfMaterialRenderExtraOptions, GltfMaterialRenderExtraUniformPlan, GltfMaterialShadingOptions,
    GltfMaterialTextureBinding, GltfMaterialTextureColorSpace, GltfMaterialTextureFallback,
    GltfMaterialTextureSlot, GltfMaterialTextureSlots, GltfMaterialUvUniformPlan, GltfMinFilter,
    GltfNodeRest, GltfNormalMapMode, GltfOutlineScale, GltfOutlineVertexSettings,
    GltfPrimitiveData, GltfSamplerData, GltfTextureData, GltfWrapMode, LoadedVrm,
    Rgba8SamplingOrigin, generate_tangents, load_vrm_from_path,
};
use vrm_runtime::sample_vrm_animation;

#[derive(Clone, Debug, Parser)]
#[command(about = "Build an ash/Vulkan-shaped VRM frame plan without opening a renderer")]
pub struct AshVrmFramePlanOptions {
    /// VRM avatar file.
    #[arg(long, default_value = ".external-fixtures/official/Seed-san.vrm")]
    pub avatar: PathBuf,
    /// Optional VRMA animation clip file.
    #[arg(long, default_value = ".external-fixtures/official/idle_loop.vrma")]
    pub animation: PathBuf,
    /// Disable VRMA sampling after loading the avatar.
    #[arg(long)]
    pub no_animation: bool,
    /// Sample time in seconds.
    #[arg(long, default_value_t = 0.0)]
    pub time: f32,
    /// Camera Y position for parity captures.
    #[arg(long, default_value_t = 1.0)]
    pub camera_y: f32,
    /// Camera Z distance for parity captures.
    #[arg(long, default_value_t = 5.0)]
    pub camera_z: f32,
    /// Camera look-at target Y for parity captures.
    #[arg(long, default_value_t = 1.0)]
    pub target_y: f32,
    /// Direct light scale packed into the scene uniform.
    #[arg(long, default_value_t = 1.0)]
    pub direct_light_scale: f32,
    /// Directional light red channel.
    #[arg(long, default_value_t = 1.0)]
    pub directional_r: f32,
    /// Directional light green channel.
    #[arg(long, default_value_t = 1.0)]
    pub directional_g: f32,
    /// Directional light blue channel.
    #[arg(long, default_value_t = 1.0)]
    pub directional_b: f32,
    /// MToon exposure for tuned accumulation.
    #[arg(long, default_value_t = 0.78)]
    pub mtoon_exposure: f32,
    /// MToon ambient base for tuned accumulation.
    #[arg(long, default_value_t = 0.12)]
    pub mtoon_ambient_base: f32,
    /// MToon ambient GI scale for tuned accumulation.
    #[arg(long, default_value_t = 0.20)]
    pub mtoon_ambient_gi_scale: f32,
    /// PBR ambient level used by the three-vrm-compatible accumulator.
    #[arg(long, default_value_t = 0.03183099)]
    pub pbr_ambient: f32,
    /// MToon light accumulation mode.
    #[arg(long, value_enum, default_value_t = AshMtoonLightAccumulation::ThreeVrm)]
    pub mtoon_light_accumulation: AshMtoonLightAccumulation,
    /// Disable MToon outline primitives for diagnostic renders.
    #[arg(long)]
    pub disable_outlines: bool,
    /// Scalar applied to MToon outline width for diagnostics.
    #[arg(long, default_value_t = 1.0)]
    pub outline_width_scale: f32,
    /// Disable normal map contribution for diagnostic renders.
    #[arg(long)]
    pub disable_normal_maps: bool,
    /// Normal-map fallback mode used for primitives without authored tangents.
    #[arg(long, value_enum, default_value_t = AshNormalMapMode::GeneratedTangents)]
    pub normal_map_mode: AshNormalMapMode,
    /// Scalar applied to normal-map strength for diagnostics.
    #[arg(long, default_value_t = 1.0)]
    pub normal_map_scale: f32,
    /// Renderer diagnostic mode for render-parity investigations.
    #[arg(long, value_enum, default_value_t = AshDiagnosticRender::Shaded)]
    pub diagnostic_render: AshDiagnosticRender,
    /// Expression weights applied before baking renderer vertices/materials.
    #[arg(long = "expression")]
    pub expressions: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum AshMtoonLightAccumulation {
    Tuned,
    ThreeVrm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum AshDiagnosticRender {
    Shaded,
    Flat,
    BaseFactor,
    BaseColor,
    BaseColorFlipV,
    BaseColorRawSrgb,
    Uv,
    BaseUv,
    OwnerId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum AshNormalMapMode {
    GeneratedTangents,
    Derivative,
    ViewDerivative,
}

impl From<AshMtoonLightAccumulation> for MtoonLightAccumulation {
    fn from(value: AshMtoonLightAccumulation) -> Self {
        match value {
            AshMtoonLightAccumulation::Tuned => Self::Tuned,
            AshMtoonLightAccumulation::ThreeVrm => Self::ThreeVrm,
        }
    }
}

impl From<AshNormalMapMode> for GltfNormalMapMode {
    fn from(value: AshNormalMapMode) -> Self {
        match value {
            AshNormalMapMode::GeneratedTangents => Self::GeneratedTangents,
            AshNormalMapMode::Derivative => Self::Derivative,
            AshNormalMapMode::ViewDerivative => Self::ViewDerivative,
        }
    }
}

impl AshVrmFramePlanOptions {
    pub fn scene_options(&self, aspect_ratio: f32) -> AshSceneOptions {
        self.scene_options_with_screen_size(
            aspect_ratio,
            ScreenProjectionSize {
                width: aspect_ratio.max(0.0) * 64.0,
                height: 64.0,
            },
        )
    }

    pub fn scene_options_with_screen_size(
        &self,
        aspect_ratio: f32,
        screen_projection_size: ScreenProjectionSize,
    ) -> AshSceneOptions {
        AshSceneOptions {
            aspect_ratio,
            screen_projection_size,
            camera_y: self.camera_y,
            camera_z: self.camera_z,
            target_y: self.target_y,
            direct_light_scale: self.direct_light_scale,
            directional_color: [self.directional_r, self.directional_g, self.directional_b],
            lighting: MtoonLightingConfig {
                accumulation: self.mtoon_light_accumulation.into(),
                exposure: self.mtoon_exposure,
                ambient_base: self.mtoon_ambient_base,
                ambient_gi_scale: self.mtoon_ambient_gi_scale,
                pbr_ambient: self.pbr_ambient,
            },
        }
    }

    fn render_options(&self) -> AshRenderOptions {
        AshRenderOptions {
            diagnostic_render: self.diagnostic_render,
            disable_outlines: self.disable_outlines,
            outline_width_scale: self.outline_width_scale,
            disable_normal_maps: self.disable_normal_maps,
            normal_map_mode: self.normal_map_mode,
            normal_map_scale: self.normal_map_scale,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AshRenderOptions {
    pub diagnostic_render: AshDiagnosticRender,
    pub disable_outlines: bool,
    pub outline_width_scale: f32,
    pub disable_normal_maps: bool,
    pub normal_map_mode: AshNormalMapMode,
    pub normal_map_scale: f32,
}

impl Default for AshRenderOptions {
    fn default() -> Self {
        Self {
            diagnostic_render: AshDiagnosticRender::Shaded,
            disable_outlines: false,
            outline_width_scale: 1.0,
            disable_normal_maps: false,
            normal_map_mode: AshNormalMapMode::GeneratedTangents,
            normal_map_scale: 1.0,
        }
    }
}

impl AshDiagnosticRender {
    fn flat_flag(self) -> f32 {
        if self == Self::Flat { 1.0 } else { 0.0 }
    }

    fn mode_code(self) -> f32 {
        match self {
            Self::BaseFactor => -1.0,
            Self::BaseColor => 1.0,
            Self::BaseColorFlipV => 2.0,
            Self::BaseColorRawSrgb => 1.25,
            Self::Uv => 3.0,
            Self::BaseUv => 4.0,
            Self::OwnerId => 5.0,
            Self::Shaded | Self::Flat => 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct AshVrmVertex {
    pub position: [f32; 3],
    pub tex_coord_0: [f32; 2],
    pub tex_coord_0_dx: [f32; 2],
    pub tex_coord_0_dy: [f32; 2],
    pub color_0: [f32; 4],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub normal_scale: f32,
    pub double_sided: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshVrmPrimitive {
    pub node: NodeRef,
    pub mesh_index: usize,
    pub primitive_index: usize,
    pub material_name: Option<String>,
    pub material: Option<MaterialRef>,
    pub pass: AshMtoonPass,
    pub vertices: Vec<AshVrmVertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshDiagnosticOwnerSource {
    pub node: NodeRef,
    pub node_name: Option<Arc<str>>,
    pub mesh_index: usize,
    pub mesh_name: Option<Arc<str>>,
    pub primitive_index: usize,
    pub material: Option<MaterialRef>,
    pub material_name: Option<Arc<str>>,
    pub pass: AshMtoonPass,
    pub alpha_mode: GltfAlphaMode,
    pub alpha_cutoff: Option<f32>,
    pub opacity: f32,
    pub double_sided: bool,
    pub render_order: i32,
    pub phase_order: i32,
    pub draw_index: usize,
    pub cull_mode: vk::CullModeFlags,
    pub front_face: vk::FrontFace,
    pub depth_write: bool,
    pub depth_test: bool,
    pub depth_compare: vk::CompareOp,
    pub blend: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AshDiagnosticOwnerProjection {
    pub screen: [[f32; 2]; 3],
    pub bounds: ScreenProjectionBounds,
    pub ndc_depth: f32,
    pub webgl_depth: f32,
    pub screen_signed_area: f32,
    pub front_facing: bool,
    pub gpu_front_facing: bool,
    pub visible_by_cull_policy: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshDiagnosticOwnerId {
    pub id: u32,
    pub color: [u8; 4],
    pub source: AshDiagnosticOwnerSource,
    pub triangle: usize,
    pub indices: [u32; 3],
    pub projection: Option<AshDiagnosticOwnerProjection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshTextureUpload {
    pub texture: Option<TextureRef>,
    pub color_space: GltfMaterialTextureColorSpace,
    pub format: vk::Format,
    pub extent: vk::Extent3D,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AshTextureUploadKey {
    pub texture: TextureRef,
    pub color_space: GltfMaterialTextureColorSpace,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshMaterialRecord {
    pub material: MaterialRef,
    pub base_color_factor: [f32; 4],
    pub base_color_texture_upload: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AshMtoonPass {
    Base,
    Outline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshPipelineKey {
    pub pass: AshMtoonPass,
    pub render_order: i32,
    pub phase_order: i32,
    pub topology: vk::PrimitiveTopology,
    pub cull_mode: vk::CullModeFlags,
    pub front_face: vk::FrontFace,
    pub depth_test_enable: bool,
    pub depth_write_enable: bool,
    pub depth_compare_op: vk::CompareOp,
    pub blend_enable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshDescriptorBindingPlan {
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub stage_flags: vk::ShaderStageFlags,
    pub texture: Option<TextureRef>,
    pub color_space: GltfMaterialTextureColorSpace,
    pub sampler: Option<AshSamplerPlan>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AshSamplerPlan {
    pub mag_filter: vk::Filter,
    pub min_filter: vk::Filter,
    pub mipmap_mode: vk::SamplerMipmapMode,
    pub address_mode_u: vk::SamplerAddressMode,
    pub address_mode_v: vk::SamplerAddressMode,
    pub min_lod: f32,
    pub max_lod: f32,
    pub normal_map_decode: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshMtoonPipelinePlan {
    pub material: MaterialRef,
    pub name: Option<String>,
    pub key: AshPipelineKey,
    pub descriptor_bindings: Vec<AshDescriptorBindingPlan>,
    pub uniform: MtoonGpuUniform,
    pub uv_uniform: AshMaterialUvUniform,
    pub render_extra_uniform: AshMaterialExtraUniform,
    pub uniform_buffer_size: u32,
    pub alpha_cutoff: f32,
    pub outline_width: f32,
    pub base_color_factor: [f32; 4],
    pub emissive_color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct AshMaterialUvUniform {
    pub base_transform: [f32; 4],
    pub shade_transform: [f32; 4],
    pub shading_shift_transform: [f32; 4],
    pub normal_transform: [f32; 4],
    pub matcap_transform: [f32; 4],
    pub rim_transform: [f32; 4],
    pub emissive_transform: [f32; 4],
    pub occlusion_transform: [f32; 4],
    pub uv_animation_mask_transform: [f32; 4],
    pub rotation_a: [f32; 4],
    pub rotation_b: [f32; 4],
    pub uv_animation: [f32; 4],
}

impl AshMaterialUvUniform {
    pub fn from_plan(plan: GltfMaterialUvUniformPlan) -> Self {
        plan.into()
    }

    pub fn bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

impl From<GltfMaterialUvUniformPlan> for AshMaterialUvUniform {
    fn from(plan: GltfMaterialUvUniformPlan) -> Self {
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

impl Default for AshMaterialUvUniform {
    fn default() -> Self {
        Self::from_plan(GltfMaterialUvUniformPlan {
            base_transform: [0.0, 0.0, 1.0, 1.0],
            shade_transform: [0.0, 0.0, 1.0, 1.0],
            shading_shift_transform: [0.0, 0.0, 1.0, 1.0],
            normal_transform: [0.0, 0.0, 1.0, 1.0],
            matcap_transform: [0.0, 0.0, 1.0, 1.0],
            rim_transform: [0.0, 0.0, 1.0, 1.0],
            emissive_transform: [0.0, 0.0, 1.0, 1.0],
            occlusion_transform: [0.0, 0.0, 1.0, 1.0],
            uv_animation_mask_transform: [0.0, 0.0, 1.0, 1.0],
            rotation_a: [0.0; 4],
            rotation_b: [0.0; 4],
            uv_animation: [0.0; 4],
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct AshMaterialExtraUniform {
    pub flags: [f32; 4],
    pub pbr_params: [f32; 4],
    pub flags2: [f32; 4],
    pub owner_color: [f32; 4],
}

impl AshMaterialExtraUniform {
    pub fn from_plan(plan: GltfMaterialRenderExtraUniformPlan) -> Self {
        plan.into()
    }

    pub fn bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

impl From<GltfMaterialRenderExtraUniformPlan> for AshMaterialExtraUniform {
    fn from(plan: GltfMaterialRenderExtraUniformPlan) -> Self {
        Self {
            flags: plan.flags,
            pbr_params: plan.pbr_params,
            flags2: plan.flags2,
            owner_color: [0.0; 4],
        }
    }
}

impl Default for AshMaterialExtraUniform {
    fn default() -> Self {
        Self::from_plan(
            vrm_io::GltfMaterialRenderExtraPlan {
                flags: Default::default(),
                metallic: 0.0,
                roughness: 1.0,
                occlusion_strength: 1.0,
                direct_light_scale: 1.0,
            }
            .uniform_plan(),
        )
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct AshSceneUniform {
    pub view_projection: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub world_from_view: [[f32; 4]; 4],
    pub light_dir: [f32; 4],
    pub light_color: [f32; 4],
    pub camera_pos: [f32; 4],
    pub mtoon_lighting: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AshSceneOptions {
    pub aspect_ratio: f32,
    pub screen_projection_size: ScreenProjectionSize,
    pub camera_y: f32,
    pub camera_z: f32,
    pub target_y: f32,
    pub direct_light_scale: f32,
    pub directional_color: [f32; 3],
    pub lighting: MtoonLightingConfig,
}

impl Default for AshSceneOptions {
    fn default() -> Self {
        Self {
            aspect_ratio: 1.0,
            screen_projection_size: ScreenProjectionSize {
                width: 64.0,
                height: 64.0,
            },
            camera_y: 1.0,
            camera_z: 5.0,
            target_y: 1.0,
            direct_light_scale: 1.0,
            directional_color: [1.0, 1.0, 1.0],
            lighting: MtoonLightingConfig::default(),
        }
    }
}

impl AshSceneOptions {
    fn sanitized_aspect_ratio(self) -> f32 {
        if self.aspect_ratio.is_finite() && self.aspect_ratio > 0.0 {
            self.aspect_ratio
        } else {
            1.0
        }
    }

    fn sanitized_screen_projection_size(self) -> ScreenProjectionSize {
        if self.screen_projection_size.width.is_finite()
            && self.screen_projection_size.width > 0.0
            && self.screen_projection_size.height.is_finite()
            && self.screen_projection_size.height > 0.0
        {
            self.screen_projection_size
        } else {
            ScreenProjectionSize {
                width: self.sanitized_aspect_ratio() * 64.0,
                height: 64.0,
            }
        }
    }

    fn camera_eye(self) -> glam::Vec3 {
        glam::Vec3::new(0.0, self.camera_y, -self.camera_z)
    }

    fn camera_target(self) -> glam::Vec3 {
        glam::Vec3::new(0.0, self.target_y, 0.0)
    }

    fn view(self) -> Mat4 {
        Mat4::look_at_rh(self.camera_eye(), self.camera_target(), glam::Vec3::Y)
    }

    fn projection(self) -> Mat4 {
        let mut projection = Mat4::perspective_rh(
            30.0_f32.to_radians(),
            self.sanitized_aspect_ratio(),
            0.1,
            20.0,
        );
        projection.y_axis.y *= -1.0;
        projection
    }

    fn projection_y_scale(self) -> f32 {
        1.0 / (0.5 * 30.0_f32.to_radians()).tan()
    }
}

impl AshSceneUniform {
    pub fn parity_camera(aspect_ratio: f32) -> Self {
        Self::from_scene_options(AshSceneOptions {
            aspect_ratio,
            ..Default::default()
        })
    }

    pub fn from_scene_options(options: AshSceneOptions) -> Self {
        let eye = options.camera_eye();
        let view = options.view();
        let projection = options.projection();
        let light_dir = glam::Vec3::new(-1.0, 1.0, -1.0).normalize();
        Self {
            view_projection: (projection * view).to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            world_from_view: view.inverse().to_cols_array_2d(),
            light_dir: [
                light_dir.x,
                light_dir.y,
                light_dir.z,
                options.direct_light_scale,
            ],
            light_color: [
                options.directional_color[0],
                options.directional_color[1],
                options.directional_color[2],
                0.0,
            ],
            camera_pos: [eye.x, eye.y, eye.z, 1.0],
            mtoon_lighting: options.lighting.effective_values().to_array(),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

impl Default for AshSceneUniform {
    fn default() -> Self {
        Self::parity_camera(1.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AshVrmFramePlan {
    pub primitives: Vec<AshVrmPrimitive>,
    pub materials: Vec<AshMaterialRecord>,
    pub texture_uploads: Vec<AshTextureUpload>,
    pub mtoon_pipelines: Vec<AshMtoonPipelinePlan>,
    pub scene_uniform: AshSceneUniform,
    pub scene_options: AshSceneOptions,
    pub diagnostic_owner_ids: Vec<AshDiagnosticOwnerId>,
    pub render_surfaces: Vec<RenderOwnerSurfaceKey>,
}

pub const ASH_OWNER_SAMPLE_OVERRIDE_RECORD_SIZE: usize =
    std::mem::size_of::<AshOwnerSampleOverrideRecord>();

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct AshOwnerSampleOverrideRecord {
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

impl AshOwnerSampleOverrideRecord {
    pub fn from_override(
        value: RenderOwnerSampleSurfaceOverride,
    ) -> Result<Self, AshOwnerSampleOverridePlanError> {
        let geometry = ash_owner_sample_geometry_record(value.sample_geometry.as_ref())?;
        Ok(Self {
            pixel: [
                u32::try_from(value.pixel.x()).map_err(|_| {
                    AshOwnerSampleOverridePlanError::PixelOutOfRange {
                        x: value.pixel.x(),
                        y: value.pixel.y(),
                    }
                })?,
                u32::try_from(value.pixel.y()).map_err(|_| {
                    AshOwnerSampleOverridePlanError::PixelOutOfRange {
                        x: value.pixel.x(),
                        y: value.pixel.y(),
                    }
                })?,
            ],
            sample: [value.sample.x() as f32, value.sample.y() as f32],
            replacement_rgba: value
                .replacement_rgba
                .map(|channel| f32::from(channel) / 255.0),
            relation_to_expected: ash_owner_sample_relation_code(value.relation_to_expected),
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
struct AshOwnerSampleGeometryRecord {
    flags: u32,
    pass: u32,
    ids: [u32; 4],
    indices: [u32; 4],
    barycentric_depth: [f32; 4],
    uvs: [f32; 4],
}

fn ash_owner_sample_geometry_record(
    geometry: Option<&vrm_adapter::RenderOwnerSampleGeometry>,
) -> Result<AshOwnerSampleGeometryRecord, AshOwnerSampleOverridePlanError> {
    let Some(geometry) = geometry else {
        return Ok(AshOwnerSampleGeometryRecord {
            flags: 0,
            pass: 0,
            ids: [u32::MAX; 4],
            indices: [u32::MAX; 4],
            barycentric_depth: [0.0; 4],
            uvs: [0.0; 4],
        });
    };
    Ok(AshOwnerSampleGeometryRecord {
        flags: 1,
        pass: ash_owner_sample_pass_code(&geometry.pass),
        ids: [
            ash_u32_geometry_value("node", geometry.node)?,
            ash_u32_geometry_value("mesh", geometry.mesh)?,
            ash_u32_geometry_value("primitive", geometry.primitive)?,
            ash_u32_geometry_value("triangle", geometry.triangle)?,
        ],
        indices: [
            ash_u32_geometry_value("indices[0]", geometry.indices[0])?,
            ash_u32_geometry_value("indices[1]", geometry.indices[1])?,
            ash_u32_geometry_value("indices[2]", geometry.indices[2])?,
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
pub struct AshOwnerSampleOverrideBufferPlan {
    pub surface: RenderOwnerSurfaceKey,
    pub records: Vec<AshOwnerSampleOverrideRecord>,
    pub binding: u32,
    pub usage: vk::BufferUsageFlags,
    pub descriptor_type: vk::DescriptorType,
    pub stage_flags: vk::ShaderStageFlags,
}

impl AshOwnerSampleOverrideBufferPlan {
    pub fn bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.records)
    }

    pub fn record_count(&self) -> usize {
        self.records.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshOwnerSampleOverridePlanError {
    PixelOutOfRange { x: u64, y: u64 },
    GeometryIndexOutOfRange { field: &'static str, value: u64 },
}

pub const fn ash_owner_sample_override_binding() -> u32 {
    RENDER_OWNER_SAMPLE_OVERRIDE_BINDING
}

pub fn ash_owner_sample_override_descriptor_set_layout_binding()
-> vk::DescriptorSetLayoutBinding<'static> {
    vk::DescriptorSetLayoutBinding::default()
        .binding(ash_owner_sample_override_binding())
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::FRAGMENT)
}

pub fn ash_owner_sample_override_buffer_plans(
    selection: &RenderOwnerSampleSelectionPlan,
) -> Result<Vec<AshOwnerSampleOverrideBufferPlan>, AshOwnerSampleOverridePlanError> {
    selection
        .surfaces
        .iter()
        .map(|surface| {
            Ok(AshOwnerSampleOverrideBufferPlan {
                surface: surface.surface.clone(),
                records: surface
                    .overrides()
                    .map(AshOwnerSampleOverrideRecord::from_override)
                    .collect::<Result<Vec<_>, _>>()?,
                binding: ash_owner_sample_override_binding(),
                usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
            })
        })
        .collect()
}

pub fn ash_owner_sample_override_buffer_plan_for_surfaces_and_draw<'a, I, S>(
    selection: &RenderOwnerSampleSelectionPlan,
    surfaces: I,
    draw: &RenderOwnerSampleDrawKey,
) -> Result<AshOwnerSampleOverrideBufferPlan, AshOwnerSampleOverridePlanError>
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
        .map(AshOwnerSampleOverrideRecord::from_override)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AshOwnerSampleOverrideBufferPlan {
        surface: surfaces
            .first()
            .cloned()
            .unwrap_or_else(|| RenderOwnerSurfaceKey::new("", 0)),
        records,
        binding: ash_owner_sample_override_binding(),
        usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
    })
}

#[derive(Clone, Debug, PartialEq)]
struct AshOwnerSampleOverridePipelineUpload {
    material: MaterialRef,
    pipeline_plan_index: usize,
    records: Vec<AshOwnerSampleOverrideRecord>,
    usage: vk::BufferUsageFlags,
}

fn ash_owner_sample_override_buffers_for_pipelines(
    plan: &AshVrmFramePlan,
    owner_sample_selection: Option<&RenderOwnerSampleSelectionPlan>,
) -> Result<Vec<AshOwnerSampleOverridePipelineUpload>, AshOwnerSampleOverridePlanError> {
    plan.mtoon_pipelines
        .iter()
        .enumerate()
        .map(|(pipeline_plan_index, pipeline)| {
            let records = owner_sample_selection
                .map(|selection| {
                    selection
                        .surfaces
                        .iter()
                        .filter(|surface| {
                            pipeline
                                .name
                                .as_deref()
                                .is_some_and(|name| name == surface.surface.material_name())
                        })
                        .flat_map(|surface| surface.overrides())
                        .map(AshOwnerSampleOverrideRecord::from_override)
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .filter(|records| !records.is_empty())
                .unwrap_or_else(|| vec![empty_ash_owner_sample_override_record()]);
            Ok(AshOwnerSampleOverridePipelineUpload {
                material: pipeline.material,
                pipeline_plan_index,
                records,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            })
        })
        .collect()
}

fn empty_ash_owner_sample_override_record() -> AshOwnerSampleOverrideRecord {
    AshOwnerSampleOverrideRecord {
        pixel: [u32::MAX, u32::MAX],
        sample: [0.0, 0.0],
        replacement_rgba: [0.0, 0.0, 0.0, 0.0],
        relation_to_expected: 0,
        geometry_flags: 0,
        sample_pass: 0,
        _padding0: 0,
        geometry_ids: [u32::MAX; 4],
        geometry_indices: [u32::MAX; 4],
        barycentric_depth: [0.0; 4],
        geometry_uvs: [0.0; 4],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshBufferRole {
    Vertex,
    Index,
    OwnerSampleOverride,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshBufferUpload {
    pub role: AshBufferRole,
    pub usage: vk::BufferUsageFlags,
    pub stride: u32,
    pub count: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshTextureResourcePlan {
    pub upload: AshTextureUpload,
    pub image_usage: vk::ImageUsageFlags,
    pub image_layout_after_upload: vk::ImageLayout,
    pub aspect_mask: vk::ImageAspectFlags,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshDescriptorSetPlan {
    pub material: MaterialRef,
    pub pipeline_plan_index: usize,
    pub bindings: Vec<AshResolvedDescriptorBinding>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshResolvedDescriptorBinding {
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub stage_flags: vk::ShaderStageFlags,
    pub uniform_upload_index: Option<usize>,
    pub texture_upload_index: Option<usize>,
    pub buffer_upload_index: Option<usize>,
    pub sampler: Option<AshSamplerPlan>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshDrawCallPlan {
    pub primitive_index: usize,
    pub material: Option<MaterialRef>,
    pub pipeline_plan_index: Option<usize>,
    pub descriptor_set_index: Option<usize>,
    pub vertex_buffer_index: usize,
    pub index_buffer_index: usize,
    pub index_count: u32,
    pub render_order: i32,
    pub phase_order: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AshRendererFrame {
    pub buffers: Vec<AshBufferUpload>,
    pub textures: Vec<AshTextureResourcePlan>,
    pub uniforms: Vec<AshUniformUpload>,
    pub pipelines: Vec<AshGraphicsPipelinePlan>,
    pub descriptor_sets: Vec<AshDescriptorSetPlan>,
    pub draw_calls: Vec<AshDrawCallPlan>,
}

#[derive(Clone, Debug)]
pub struct AshDrawableFramePlan {
    pub render_pass: AshRenderPassPlan,
    pub commands: Vec<AshCommandPlan>,
    pub skipped_draws: Vec<AshSkippedDraw>,
}

#[derive(Clone, Debug)]
pub struct AshRenderPassPlan {
    pub render_area: vk::Rect2D,
    pub color_format: vk::Format,
    pub depth_format: Option<vk::Format>,
    pub color_clear: [f32; 4],
    pub depth_stencil_clear: Option<AshDepthStencilClear>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AshDepthStencilClear {
    pub depth: f32,
    pub stencil: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AshCommandPlan {
    BindGraphicsPipeline {
        pipeline_index: usize,
    },
    BindDescriptorSet {
        pipeline_index: usize,
        descriptor_set_index: usize,
    },
    BindVertexBuffer {
        buffer_index: usize,
        binding: u32,
        offset: vk::DeviceSize,
    },
    BindIndexBuffer {
        buffer_index: usize,
        offset: vk::DeviceSize,
        index_type: vk::IndexType,
    },
    DrawIndexed {
        primitive_index: usize,
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AshSkippedDraw {
    pub primitive_index: usize,
    pub reason: AshSkippedDrawReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshSkippedDrawReason {
    MissingPipeline,
    MissingDescriptorSet,
    MissingVertexBuffer,
    MissingIndexBuffer,
    EmptyIndexRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshUniformScope {
    Material {
        material: MaterialRef,
        pipeline_plan_index: usize,
    },
    MaterialUv {
        material: MaterialRef,
        pipeline_plan_index: usize,
    },
    MaterialExtra {
        material: MaterialRef,
        pipeline_plan_index: usize,
    },
    Scene,
}

impl AshUniformScope {
    pub const fn material(material: MaterialRef, pipeline_plan_index: usize) -> Self {
        Self::Material {
            material,
            pipeline_plan_index,
        }
    }

    pub const fn material_uv(material: MaterialRef, pipeline_plan_index: usize) -> Self {
        Self::MaterialUv {
            material,
            pipeline_plan_index,
        }
    }

    pub const fn material_extra(material: MaterialRef, pipeline_plan_index: usize) -> Self {
        Self::MaterialExtra {
            material,
            pipeline_plan_index,
        }
    }

    pub const fn binding(self) -> u32 {
        match self {
            Self::Material { .. } => ash_mtoon_uniform_binding(),
            Self::MaterialUv { .. } => ash_mtoon_uv_uniform_binding(),
            Self::MaterialExtra { .. } => ash_mtoon_render_extra_binding(),
            Self::Scene => ash_mtoon_scene_binding(),
        }
    }

    pub const fn material_ref(self) -> Option<MaterialRef> {
        match self {
            Self::Material { material, .. }
            | Self::MaterialUv { material, .. }
            | Self::MaterialExtra { material, .. } => Some(material),
            Self::Scene => None,
        }
    }

    pub const fn pipeline_plan_index(self) -> Option<usize> {
        match self {
            Self::Material {
                pipeline_plan_index,
                ..
            }
            | Self::MaterialUv {
                pipeline_plan_index,
                ..
            }
            | Self::MaterialExtra {
                pipeline_plan_index,
                ..
            } => Some(pipeline_plan_index),
            Self::Scene => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshUniformUpload {
    pub scope: AshUniformScope,
    pub binding: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshGraphicsPipelinePlan {
    pub material: MaterialRef,
    pub pipeline_plan_index: usize,
    pub descriptor_set_index: usize,
    pub key: AshPipelineKey,
    pub vertex_stride: u32,
    pub vertex_attributes: Vec<AshVertexAttributePlan>,
    pub color_format: vk::Format,
    pub depth_format: Option<vk::Format>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshVertexAttributePlan {
    pub location: u32,
    pub binding: u32,
    pub format: vk::Format,
    pub offset: u32,
}

pub fn ash_drawable_frame_from_renderer_frame(
    frame: &AshRendererFrame,
    extent: vk::Extent2D,
) -> AshDrawableFramePlan {
    let first_pipeline = frame.pipelines.first();
    let mut commands = Vec::new();
    let mut skipped_draws = Vec::new();
    let mut bound_pipeline = None;
    let mut bound_descriptor_set = None;
    let mut bound_vertex_buffer = None;
    let mut bound_index_buffer = None;

    for draw in &frame.draw_calls {
        let Some(pipeline_plan_index) = draw.pipeline_plan_index else {
            skipped_draws.push(AshSkippedDraw {
                primitive_index: draw.primitive_index,
                reason: AshSkippedDrawReason::MissingPipeline,
            });
            continue;
        };
        let Some(pipeline_index) = frame
            .pipelines
            .iter()
            .position(|pipeline| pipeline.pipeline_plan_index == pipeline_plan_index)
        else {
            skipped_draws.push(AshSkippedDraw {
                primitive_index: draw.primitive_index,
                reason: AshSkippedDrawReason::MissingPipeline,
            });
            continue;
        };
        let Some(descriptor_set_index) = draw
            .descriptor_set_index
            .filter(|index| *index < frame.descriptor_sets.len())
        else {
            skipped_draws.push(AshSkippedDraw {
                primitive_index: draw.primitive_index,
                reason: AshSkippedDrawReason::MissingDescriptorSet,
            });
            continue;
        };
        if draw.vertex_buffer_index >= frame.buffers.len() {
            skipped_draws.push(AshSkippedDraw {
                primitive_index: draw.primitive_index,
                reason: AshSkippedDrawReason::MissingVertexBuffer,
            });
            continue;
        }
        if draw.index_buffer_index >= frame.buffers.len() {
            skipped_draws.push(AshSkippedDraw {
                primitive_index: draw.primitive_index,
                reason: AshSkippedDrawReason::MissingIndexBuffer,
            });
            continue;
        }
        if draw.index_count == 0 {
            skipped_draws.push(AshSkippedDraw {
                primitive_index: draw.primitive_index,
                reason: AshSkippedDrawReason::EmptyIndexRange,
            });
            continue;
        }

        if bound_pipeline != Some(pipeline_index) {
            commands.push(AshCommandPlan::BindGraphicsPipeline { pipeline_index });
            bound_pipeline = Some(pipeline_index);
            bound_descriptor_set = None;
        }
        if bound_descriptor_set != Some(descriptor_set_index) {
            commands.push(AshCommandPlan::BindDescriptorSet {
                pipeline_index,
                descriptor_set_index,
            });
            bound_descriptor_set = Some(descriptor_set_index);
        }
        if bound_vertex_buffer != Some(draw.vertex_buffer_index) {
            commands.push(AshCommandPlan::BindVertexBuffer {
                buffer_index: draw.vertex_buffer_index,
                binding: 0,
                offset: 0,
            });
            bound_vertex_buffer = Some(draw.vertex_buffer_index);
        }
        if bound_index_buffer != Some(draw.index_buffer_index) {
            commands.push(AshCommandPlan::BindIndexBuffer {
                buffer_index: draw.index_buffer_index,
                offset: 0,
                index_type: vk::IndexType::UINT32,
            });
            bound_index_buffer = Some(draw.index_buffer_index);
        }
        commands.push(AshCommandPlan::DrawIndexed {
            primitive_index: draw.primitive_index,
            index_count: draw.index_count,
            instance_count: 1,
            first_index: 0,
            vertex_offset: 0,
            first_instance: 0,
        });
    }

    AshDrawableFramePlan {
        render_pass: AshRenderPassPlan {
            render_area: vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            },
            color_format: first_pipeline
                .map(|pipeline| pipeline.color_format)
                .unwrap_or(vk::Format::R8G8B8A8_UNORM),
            depth_format: first_pipeline.and_then(|pipeline| pipeline.depth_format),
            color_clear: [0.0, 0.0, 0.0, 1.0],
            depth_stencil_clear: Some(AshDepthStencilClear {
                depth: 1.0,
                stencil: 0,
            }),
        },
        commands,
        skipped_draws,
    }
}

pub fn ash_renderer_frame_from_plan(plan: &AshVrmFramePlan) -> AshRendererFrame {
    ash_renderer_frame_from_plan_with_owner_sample_selection(plan, None)
        .expect("empty owner/sample selection cannot fail")
}

pub fn ash_renderer_frame_from_plan_with_owner_sample_selection(
    plan: &AshVrmFramePlan,
    owner_sample_selection: Option<&RenderOwnerSampleSelectionPlan>,
) -> Result<AshRendererFrame, AshOwnerSampleOverridePlanError> {
    let texture_indices = texture_ref_upload_indices(&plan.texture_uploads);
    let material_uniform_count = plan.mtoon_pipelines.len() * ASH_MTOON_UNIFORMS_PER_PIPELINE;
    let scene_uniform_upload_index = material_uniform_count;
    let owner_sample_override_buffers =
        ash_owner_sample_override_buffers_for_pipelines(plan, owner_sample_selection)?;
    let owner_sample_override_buffer_indices = owner_sample_override_buffers
        .iter()
        .enumerate()
        .map(|(index, upload)| ((upload.material, upload.pipeline_plan_index), index))
        .collect::<HashMap<_, _>>();
    let descriptor_sets = plan
        .mtoon_pipelines
        .iter()
        .enumerate()
        .map(|(pipeline_plan_index, pipeline)| AshDescriptorSetPlan {
            material: pipeline.material,
            pipeline_plan_index,
            bindings: pipeline
                .descriptor_bindings
                .iter()
                .map(|binding| AshResolvedDescriptorBinding {
                    binding: binding.binding,
                    descriptor_type: binding.descriptor_type,
                    stage_flags: binding.stage_flags,
                    uniform_upload_index: (binding.descriptor_type
                        == vk::DescriptorType::UNIFORM_BUFFER)
                        .then(|| match binding.binding {
                            binding if binding == ash_mtoon_uniform_binding() => {
                                pipeline_plan_index * ASH_MTOON_UNIFORMS_PER_PIPELINE
                            }
                            binding if binding == ash_mtoon_uv_uniform_binding() => {
                                pipeline_plan_index * ASH_MTOON_UNIFORMS_PER_PIPELINE + 1
                            }
                            binding if binding == ash_mtoon_render_extra_binding() => {
                                pipeline_plan_index * ASH_MTOON_UNIFORMS_PER_PIPELINE + 2
                            }
                            binding if binding == ash_mtoon_scene_binding() => {
                                scene_uniform_upload_index
                            }
                            _ => pipeline_plan_index * ASH_MTOON_UNIFORMS_PER_PIPELINE,
                        }),
                    texture_upload_index: binding.texture.and_then(|texture| {
                        texture_indices
                            .get(&AshTextureUploadKey {
                                texture,
                                color_space: binding.color_space,
                            })
                            .copied()
                    }),
                    buffer_upload_index: (binding.descriptor_type
                        == vk::DescriptorType::STORAGE_BUFFER)
                        .then(|| {
                            owner_sample_override_buffer_indices
                                [&(pipeline.material, pipeline_plan_index)]
                        }),
                    sampler: binding.sampler,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let pipeline_indices = mtoon_pipeline_indices(&plan.mtoon_pipelines);
    let descriptor_indices = descriptor_set_indices(&descriptor_sets);
    let pipelines = plan
        .mtoon_pipelines
        .iter()
        .enumerate()
        .filter_map(|(pipeline_plan_index, pipeline)| {
            descriptor_indices
                .get(&(pipeline.material, pipeline_plan_index))
                .copied()
                .map(|descriptor_set_index| AshGraphicsPipelinePlan {
                    material: pipeline.material,
                    pipeline_plan_index,
                    descriptor_set_index,
                    key: pipeline.key,
                    vertex_stride: std::mem::size_of::<AshVrmVertex>() as u32,
                    vertex_attributes: ash_vrm_vertex_attributes(),
                    color_format: vk::Format::R8G8B8A8_UNORM,
                    depth_format: Some(ash_reference_depth_format()),
                })
        })
        .collect::<Vec<_>>();
    let mut pipelines = pipelines;
    let mut buffers = owner_sample_override_buffers
        .into_iter()
        .map(|upload| AshBufferUpload {
            role: AshBufferRole::OwnerSampleOverride,
            usage: upload.usage,
            stride: std::mem::size_of::<AshOwnerSampleOverrideRecord>() as u32,
            count: upload.records.len() as u32,
            bytes: bytemuck::cast_slice(&upload.records).to_vec(),
        })
        .collect::<Vec<_>>();
    buffers.reserve(plan.primitives.len() * 4);
    let mut draw_calls = Vec::with_capacity(plan.primitives.len());
    let mut next_resolve_pipeline_plan_index = plan.mtoon_pipelines.len();
    for (primitive_index, primitive) in plan.primitives.iter().enumerate() {
        let vertex_buffer_index = buffers.len();
        buffers.push(AshBufferUpload {
            role: AshBufferRole::Vertex,
            usage: vk::BufferUsageFlags::VERTEX_BUFFER,
            stride: std::mem::size_of::<AshVrmVertex>() as u32,
            count: primitive.vertices.len() as u32,
            bytes: bytemuck::cast_slice(&primitive.vertices).to_vec(),
        });
        let index_buffer_index = buffers.len();
        buffers.push(AshBufferUpload {
            role: AshBufferRole::Index,
            usage: vk::BufferUsageFlags::INDEX_BUFFER,
            stride: std::mem::size_of::<u32>() as u32,
            count: primitive.indices.len() as u32,
            bytes: bytemuck::cast_slice(&primitive.indices).to_vec(),
        });
        let pipeline_plan_index = primitive
            .material
            .and_then(|material| pipeline_indices.get(&(material, primitive.pass)).copied());
        let descriptor_set_index = pipeline_plan_index.and_then(|index| {
            primitive
                .material
                .and_then(|material| descriptor_indices.get(&(material, index)).copied())
        });
        let (render_order, phase_order) = pipeline_plan_index
            .and_then(|index| plan.mtoon_pipelines.get(index))
            .map(|pipeline| (pipeline.key.render_order, pipeline.key.phase_order))
            .unwrap_or((2000, 2000));
        draw_calls.push(AshDrawCallPlan {
            primitive_index,
            material: primitive.material,
            pipeline_plan_index,
            descriptor_set_index,
            vertex_buffer_index,
            index_buffer_index,
            index_count: primitive.indices.len() as u32,
            render_order,
            phase_order,
        });
        if let (
            Some(selection),
            Some(material),
            Some(pipeline_plan_index),
            Some(descriptor_set_index),
        ) = (
            owner_sample_selection,
            primitive.material,
            pipeline_plan_index,
            descriptor_set_index,
        ) {
            let material_name = primitive.material_name.as_deref().or_else(|| {
                plan.mtoon_pipelines
                    .get(pipeline_plan_index)
                    .and_then(|pipeline| pipeline.name.as_deref())
            });
            let records =
                ash_owner_sample_records_for_primitive(selection, primitive, material_name)?;
            let resolve_vertices = ash_owner_sample_resolve_vertices_for_primitive(
                primitive,
                &records,
                plan.scene_options,
            );
            if !resolve_vertices.is_empty() {
                let resolve_pipeline_plan_index = next_resolve_pipeline_plan_index;
                next_resolve_pipeline_plan_index += 1;
                let Some(source_pipeline) = plan.mtoon_pipelines.get(pipeline_plan_index) else {
                    continue;
                };
                pipelines.push(AshGraphicsPipelinePlan {
                    material,
                    pipeline_plan_index: resolve_pipeline_plan_index,
                    descriptor_set_index,
                    key: ash_owner_sample_resolve_pipeline_key(source_pipeline.key),
                    vertex_stride: std::mem::size_of::<AshVrmVertex>() as u32,
                    vertex_attributes: ash_vrm_vertex_attributes(),
                    color_format: vk::Format::R8G8B8A8_UNORM,
                    depth_format: Some(ash_reference_depth_format()),
                });
                let resolve_vertex_buffer_index = buffers.len();
                buffers.push(AshBufferUpload {
                    role: AshBufferRole::Vertex,
                    usage: vk::BufferUsageFlags::VERTEX_BUFFER,
                    stride: std::mem::size_of::<AshVrmVertex>() as u32,
                    count: resolve_vertices.len() as u32,
                    bytes: bytemuck::cast_slice(&resolve_vertices).to_vec(),
                });
                let resolve_indices =
                    (0..u32::try_from(resolve_vertices.len()).unwrap_or(0)).collect::<Vec<_>>();
                let resolve_index_buffer_index = buffers.len();
                buffers.push(AshBufferUpload {
                    role: AshBufferRole::Index,
                    usage: vk::BufferUsageFlags::INDEX_BUFFER,
                    stride: std::mem::size_of::<u32>() as u32,
                    count: resolve_indices.len() as u32,
                    bytes: bytemuck::cast_slice(&resolve_indices).to_vec(),
                });
                draw_calls.push(AshDrawCallPlan {
                    primitive_index,
                    material: primitive.material,
                    pipeline_plan_index: Some(resolve_pipeline_plan_index),
                    descriptor_set_index: Some(descriptor_set_index),
                    vertex_buffer_index: resolve_vertex_buffer_index,
                    index_buffer_index: resolve_index_buffer_index,
                    index_count: resolve_indices.len() as u32,
                    render_order: render_order.saturating_add(10_000),
                    phase_order: phase_order.saturating_add(10_000),
                });
            }
        }
    }
    draw_calls.sort_by_key(|draw| (draw.render_order, draw.primitive_index));
    Ok(AshRendererFrame {
        buffers,
        textures: plan
            .texture_uploads
            .iter()
            .cloned()
            .map(|upload| AshTextureResourcePlan {
                upload,
                image_usage: vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
                image_layout_after_upload: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                aspect_mask: vk::ImageAspectFlags::COLOR,
            })
            .collect(),
        uniforms: plan
            .mtoon_pipelines
            .iter()
            .enumerate()
            .flat_map(|(pipeline_plan_index, pipeline)| {
                [
                    AshUniformUpload {
                        scope: AshUniformScope::material(pipeline.material, pipeline_plan_index),
                        binding: ash_mtoon_uniform_binding(),
                        bytes: pipeline.uniform.bytes().to_vec(),
                    },
                    AshUniformUpload {
                        scope: AshUniformScope::material_uv(pipeline.material, pipeline_plan_index),
                        binding: ash_mtoon_uv_uniform_binding(),
                        bytes: pipeline.uv_uniform.bytes().to_vec(),
                    },
                    AshUniformUpload {
                        scope: AshUniformScope::material_extra(
                            pipeline.material,
                            pipeline_plan_index,
                        ),
                        binding: ash_mtoon_render_extra_binding(),
                        bytes: pipeline.render_extra_uniform.bytes().to_vec(),
                    },
                ]
            })
            .chain(std::iter::once(AshUniformUpload {
                scope: AshUniformScope::Scene,
                binding: ash_mtoon_scene_binding(),
                bytes: plan.scene_uniform.bytes().to_vec(),
            }))
            .collect(),
        pipelines,
        descriptor_sets,
        draw_calls,
    })
}

fn ash_owner_sample_records_for_primitive(
    selection: &RenderOwnerSampleSelectionPlan,
    primitive: &AshVrmPrimitive,
    material_name: Option<&str>,
) -> Result<Vec<AshOwnerSampleOverrideRecord>, AshOwnerSampleOverridePlanError> {
    let Some(material_name) = material_name else {
        return Ok(Vec::new());
    };
    let draw = RenderOwnerSampleDrawKey::new(
        u64::try_from(primitive.node.0).unwrap_or(u64::MAX),
        u64::try_from(primitive.mesh_index).unwrap_or(u64::MAX),
        u64::try_from(primitive.primitive_index).unwrap_or(u64::MAX),
        ash_owner_sample_render_pass(primitive.pass),
    );
    let surfaces = (0..primitive.indices.len() / 3)
        .filter_map(|triangle| {
            Some(RenderOwnerSurfaceKey::new(
                material_name,
                u64::try_from(triangle).ok()?,
            ))
        })
        .collect::<Vec<_>>();
    let strict_records = ash_owner_sample_override_buffer_plan_for_surfaces_and_draw(
        selection,
        surfaces.iter(),
        &draw,
    )?
    .records;
    if !strict_records.is_empty() {
        return Ok(strict_records);
    }
    surfaces
        .iter()
        .flat_map(|surface| {
            selection
                .selection_for_surface(surface)
                .into_iter()
                .flat_map(|surface_selection| surface_selection.entries.iter())
                .filter(|entry| ash_owner_sample_entry_matches_mesh_alias(entry, primitive))
                .map(RenderOwnerSampleSurfaceOverride::from)
        })
        .map(AshOwnerSampleOverrideRecord::from_override)
        .collect()
}

fn ash_owner_sample_render_pass(pass: AshMtoonPass) -> RenderOwnerSamplePass {
    match pass {
        AshMtoonPass::Base => RenderOwnerSamplePass::Base,
        AshMtoonPass::Outline => RenderOwnerSamplePass::Outline,
    }
}

fn ash_owner_sample_entry_matches_mesh_alias(
    entry: &vrm_adapter::RenderOwnerSampleCorrectionManifestEntry,
    primitive: &AshVrmPrimitive,
) -> bool {
    let Some(geometry) = &entry.sample_geometry else {
        return false;
    };
    geometry.node == u64::try_from(primitive.node.0).unwrap_or(u64::MAX)
        && geometry.primitive == u64::try_from(primitive.primitive_index).unwrap_or(u64::MAX)
        && geometry.pass == ash_owner_sample_render_pass(primitive.pass)
}

fn ash_owner_sample_resolve_pipeline_key(source: AshPipelineKey) -> AshPipelineKey {
    AshPipelineKey {
        topology: vk::PrimitiveTopology::TRIANGLE_LIST,
        cull_mode: vk::CullModeFlags::empty(),
        depth_write_enable: false,
        depth_compare_op: vk::CompareOp::ALWAYS,
        render_order: source.render_order.saturating_add(10_000),
        phase_order: source.phase_order.saturating_add(10_000),
        ..source
    }
}

fn ash_owner_sample_resolve_vertices_for_primitive(
    primitive: &AshVrmPrimitive,
    records: &[AshOwnerSampleOverrideRecord],
    scene_options: AshSceneOptions,
) -> Vec<AshVrmVertex> {
    records
        .iter()
        .filter(|record| record.geometry_flags != 0)
        .flat_map(|record| ash_owner_sample_resolve_vertices(primitive, record, scene_options))
        .collect()
}

fn ash_owner_sample_resolve_vertices(
    primitive: &AshVrmPrimitive,
    record: &AshOwnerSampleOverrideRecord,
    scene_options: AshSceneOptions,
) -> Vec<AshVrmVertex> {
    let Some(quad) = ash_owner_sample_pixel_quad_world(record.pixel, scene_options) else {
        return Vec::new();
    };
    let [ia, ib, ic] = [
        usize::try_from(record.geometry_indices[0]),
        usize::try_from(record.geometry_indices[1]),
        usize::try_from(record.geometry_indices[2]),
    ];
    let (Ok(ia), Ok(ib), Ok(ic)) = (ia, ib, ic) else {
        return Vec::new();
    };
    let (Some(a), Some(b), Some(c)) = (
        primitive.vertices.get(ia).copied(),
        primitive.vertices.get(ib).copied(),
        primitive.vertices.get(ic).copied(),
    ) else {
        return Vec::new();
    };
    let weights = [
        record.barycentric_depth[0],
        record.barycentric_depth[1],
        record.barycentric_depth[2],
    ];
    let sample_vertex = ash_interpolate_vertex(a, b, c, weights);
    let Some([tex_coord_0_dx, tex_coord_0_dy]) =
        ash_owner_sample_uv_gradient(a, b, c, scene_options)
    else {
        return Vec::new();
    };
    quad.into_iter()
        .map(|position| {
            let mut vertex = sample_vertex;
            vertex.position = position;
            vertex.tex_coord_0 = [record.geometry_uvs[0], record.geometry_uvs[1]];
            vertex.tex_coord_0_dx = tex_coord_0_dx;
            vertex.tex_coord_0_dy = tex_coord_0_dy;
            vertex
        })
        .collect()
}

fn ash_owner_sample_pixel_quad_world(
    pixel: [u32; 2],
    scene_options: AshSceneOptions,
) -> Option<[[f32; 3]; 6]> {
    let size = scene_options.sanitized_screen_projection_size();
    let x = pixel[0] as f32;
    let y = pixel[1] as f32;
    if !(x < size.width && y < size.height) {
        return None;
    }
    let left = ash_owner_sample_screen_world(x, y, scene_options)?;
    let right = ash_owner_sample_screen_world(x + 1.0, y, scene_options)?;
    let bottom_left = ash_owner_sample_screen_world(x, y + 1.0, scene_options)?;
    let bottom_right = ash_owner_sample_screen_world(x + 1.0, y + 1.0, scene_options)?;
    Some([left, bottom_left, right, right, bottom_left, bottom_right])
}

fn ash_owner_sample_screen_world(
    screen_x: f32,
    screen_y: f32,
    scene_options: AshSceneOptions,
) -> Option<[f32; 3]> {
    let size = scene_options.sanitized_screen_projection_size();
    if !(screen_x >= 0.0 && screen_x <= size.width && screen_y >= 0.0 && screen_y <= size.height) {
        return None;
    }
    let ndc_x = screen_x / size.width * 2.0 - 1.0;
    let ndc_y = screen_y / size.height * 2.0 - 1.0;
    let clip = Vec4::new(ndc_x, ndc_y, 0.5, 1.0);
    let world = (scene_options.projection() * scene_options.view()).inverse() * clip;
    (world.w.abs() > f32::EPSILON).then(|| (world.truncate() / world.w).to_array())
}

fn ash_owner_sample_uv_gradient(
    a: AshVrmVertex,
    b: AshVrmVertex,
    c: AshVrmVertex,
    scene_options: AshSceneOptions,
) -> Option<[[f32; 2]; 2]> {
    let pa = ash_project_world_to_pixel(a.position, scene_options)?;
    let pb = ash_project_world_to_pixel(b.position, scene_options)?;
    let pc = ash_project_world_to_pixel(c.position, scene_options)?;
    let dx1 = pb.x - pa.x;
    let dy1 = pb.y - pa.y;
    let dx2 = pc.x - pa.x;
    let dy2 = pc.y - pa.y;
    let det = dx1 * dy2 - dx2 * dy1;
    if det.abs() <= f32::EPSILON {
        return None;
    }
    let uv_a = Vec2::from_array(a.tex_coord_0);
    let uv_b = Vec2::from_array(b.tex_coord_0);
    let uv_c = Vec2::from_array(c.tex_coord_0);
    let duv1 = uv_b - uv_a;
    let duv2 = uv_c - uv_a;
    let duv_dx = (duv1 * dy2 - duv2 * dy1) / det;
    let duv_dy = (duv2 * dx1 - duv1 * dx2) / det;
    Some([duv_dx.to_array(), duv_dy.to_array()])
}

fn ash_project_world_to_pixel(position: [f32; 3], scene_options: AshSceneOptions) -> Option<Vec2> {
    let clip = scene_options.projection()
        * scene_options.view()
        * Vec4::new(position[0], position[1], position[2], 1.0);
    if clip.w.abs() <= f32::EPSILON {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    let size = scene_options.sanitized_screen_projection_size();
    Some(Vec2::new(
        (ndc.x + 1.0) * 0.5 * size.width,
        (ndc.y + 1.0) * 0.5 * size.height,
    ))
}

fn ash_interpolate_vertex(
    a: AshVrmVertex,
    b: AshVrmVertex,
    c: AshVrmVertex,
    weights: [f32; 3],
) -> AshVrmVertex {
    AshVrmVertex {
        position: interpolate_vec3(a.position, b.position, c.position, weights),
        tex_coord_0: interpolate_vec2(a.tex_coord_0, b.tex_coord_0, c.tex_coord_0, weights),
        tex_coord_0_dx: interpolate_vec2(
            a.tex_coord_0_dx,
            b.tex_coord_0_dx,
            c.tex_coord_0_dx,
            weights,
        ),
        tex_coord_0_dy: interpolate_vec2(
            a.tex_coord_0_dy,
            b.tex_coord_0_dy,
            c.tex_coord_0_dy,
            weights,
        ),
        color_0: interpolate_vec4(a.color_0, b.color_0, c.color_0, weights),
        normal: normalize_or_fallback(
            interpolate_vec3(a.normal, b.normal, c.normal, weights),
            a.normal,
        ),
        tangent: normalize_tangent(interpolate_vec4(a.tangent, b.tangent, c.tangent, weights)),
        normal_scale: interpolate_scalar(a.normal_scale, b.normal_scale, c.normal_scale, weights),
        double_sided: a.double_sided,
    }
}

fn interpolate_vec2(a: [f32; 2], b: [f32; 2], c: [f32; 2], weights: [f32; 3]) -> [f32; 2] {
    [
        interpolate_scalar(a[0], b[0], c[0], weights),
        interpolate_scalar(a[1], b[1], c[1], weights),
    ]
}

fn interpolate_vec3(a: [f32; 3], b: [f32; 3], c: [f32; 3], weights: [f32; 3]) -> [f32; 3] {
    [
        interpolate_scalar(a[0], b[0], c[0], weights),
        interpolate_scalar(a[1], b[1], c[1], weights),
        interpolate_scalar(a[2], b[2], c[2], weights),
    ]
}

fn interpolate_vec4(a: [f32; 4], b: [f32; 4], c: [f32; 4], weights: [f32; 3]) -> [f32; 4] {
    [
        interpolate_scalar(a[0], b[0], c[0], weights),
        interpolate_scalar(a[1], b[1], c[1], weights),
        interpolate_scalar(a[2], b[2], c[2], weights),
        interpolate_scalar(a[3], b[3], c[3], weights),
    ]
}

fn interpolate_scalar(a: f32, b: f32, c: f32, weights: [f32; 3]) -> f32 {
    weights[0] * a + weights[1] * b + weights[2] * c
}

fn normalize_or_fallback(value: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let vector = Vec3::from_array(value);
    if vector.length_squared() > f32::EPSILON {
        vector.normalize().to_array()
    } else {
        fallback
    }
}

fn normalize_tangent(value: [f32; 4]) -> [f32; 4] {
    let tangent = Vec3::new(value[0], value[1], value[2]);
    let normalized = if tangent.length_squared() > f32::EPSILON {
        tangent.normalize()
    } else {
        Vec3::X
    };
    [normalized.x, normalized.y, normalized.z, value[3].signum()]
}

pub fn ash_vrm_vertex_attributes() -> Vec<AshVertexAttributePlan> {
    vec![
        AshVertexAttributePlan {
            location: 0,
            binding: 0,
            format: vk::Format::R32G32B32_SFLOAT,
            offset: 0,
        },
        AshVertexAttributePlan {
            location: 1,
            binding: 0,
            format: vk::Format::R32G32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, tex_coord_0) as u32,
        },
        AshVertexAttributePlan {
            location: 2,
            binding: 0,
            format: vk::Format::R32G32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, tex_coord_0_dx) as u32,
        },
        AshVertexAttributePlan {
            location: 3,
            binding: 0,
            format: vk::Format::R32G32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, tex_coord_0_dy) as u32,
        },
        AshVertexAttributePlan {
            location: 4,
            binding: 0,
            format: vk::Format::R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, color_0) as u32,
        },
        AshVertexAttributePlan {
            location: 5,
            binding: 0,
            format: vk::Format::R32G32B32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, normal) as u32,
        },
        AshVertexAttributePlan {
            location: 6,
            binding: 0,
            format: vk::Format::R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, tangent) as u32,
        },
        AshVertexAttributePlan {
            location: 7,
            binding: 0,
            format: vk::Format::R32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, normal_scale) as u32,
        },
        AshVertexAttributePlan {
            location: 8,
            binding: 0,
            format: vk::Format::R32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, double_sided) as u32,
        },
    ]
}

pub struct AshVrmFramePlanner {
    loaded: LoadedVrm,
    scene: HeadlessSceneState,
    rig: HumanoidPoseRig,
    animation: Option<VrmAnimation>,
    expression_effects: GltfExpressionRenderEffects,
}

#[derive(Clone, Copy, Debug)]
struct AshPrimitiveBakeSettings {
    pass: AshMtoonPass,
    mtoon_time: f32,
    scene_options: AshSceneOptions,
    render_options: AshRenderOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AshPrimitiveSourceIndex {
    node: usize,
    mesh: usize,
    primitive: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AshPrimitiveDrawOrder {
    render_order: i32,
    phase_order: i32,
}

#[derive(Clone, Debug, PartialEq)]
struct AshPrimitiveSource {
    node: NodeRef,
    node_name: Option<Arc<str>>,
    mesh_index: usize,
    mesh_name: Option<Arc<str>>,
    primitive_index: usize,
    material: Option<MaterialRef>,
    material_name: Option<Arc<str>>,
    pass: AshMtoonPass,
    alpha_mode: GltfAlphaMode,
    alpha_cutoff: Option<f32>,
    opacity: f32,
    double_sided: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct AshPrimitiveRecord {
    primitive: AshVrmPrimitive,
    source: AshPrimitiveSource,
    draw_order: AshPrimitiveDrawOrder,
}

#[derive(Clone, Debug, PartialEq)]
struct AshBakedPrimitives {
    primitives: Vec<AshVrmPrimitive>,
    diagnostic_owner_ids: Vec<AshDiagnosticOwnerId>,
    render_surfaces: Vec<RenderOwnerSurfaceKey>,
}

#[derive(Clone, Debug)]
struct AshOwnerIdAssignmentContext {
    source: AshPrimitiveSource,
    draw_order: AshPrimitiveDrawOrder,
    draw_index: usize,
    pipeline_key: AshPipelineKey,
    view_projection: Mat4,
    scene_options: AshSceneOptions,
}

impl AshVrmFramePlanner {
    pub fn from_paths(
        avatar: impl Into<PathBuf>,
        animation: Option<impl Into<PathBuf>>,
    ) -> Result<Self, Box<dyn Error>> {
        let loaded = load_vrm_from_path(avatar.into())?;
        let animation = match animation {
            Some(path) => {
                let loaded_animation = load_vrm_from_path(path.into())?;
                animation_from_loaded(&loaded_animation)
            }
            None => None,
        };
        Self::new(loaded, animation)
    }

    pub fn new(loaded: LoadedVrm, animation: Option<VrmAnimation>) -> Result<Self, Box<dyn Error>> {
        let mut scene = headless_scene_from_loaded(&loaded)?;
        scene.update_world_transforms()?;
        let rig = HumanoidPoseRig::capture(&scene, loaded.model().document())?;
        Ok(Self {
            loaded,
            scene,
            rig,
            animation,
            expression_effects: GltfExpressionRenderEffects::default(),
        })
    }

    pub fn set_expression_weights<I, N>(&mut self, weights: I) -> Result<(), Box<dyn Error>>
    where
        I: IntoIterator<Item = (N, f32)>,
        N: AsRef<str>,
    {
        self.expression_effects = self.loaded.expression_render_effects(weights)?;
        Ok(())
    }

    pub fn sample_frame(&mut self, time_seconds: f32) -> Result<AshVrmFramePlan, Box<dyn Error>> {
        self.sample_frame_with_scene_options(time_seconds, AshSceneOptions::default())
    }

    pub fn sample_frame_with_scene_options(
        &mut self,
        time_seconds: f32,
        scene_options: AshSceneOptions,
    ) -> Result<AshVrmFramePlan, Box<dyn Error>> {
        self.sample_frame_with_render_options(
            time_seconds,
            scene_options,
            AshDiagnosticRender::Shaded,
        )
    }

    pub fn sample_frame_with_render_options(
        &mut self,
        time_seconds: f32,
        scene_options: AshSceneOptions,
        diagnostic_render: AshDiagnosticRender,
    ) -> Result<AshVrmFramePlan, Box<dyn Error>> {
        self.sample_frame_with_full_render_options(
            time_seconds,
            scene_options,
            AshRenderOptions {
                diagnostic_render,
                ..Default::default()
            },
        )
    }

    pub fn sample_frame_with_full_render_options(
        &mut self,
        time_seconds: f32,
        scene_options: AshSceneOptions,
        render_options: AshRenderOptions,
    ) -> Result<AshVrmFramePlan, Box<dyn Error>> {
        if let Some(animation) = &self.animation {
            let time = if animation.duration > f32::EPSILON {
                time_seconds.rem_euclid(animation.duration)
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
        let mtoon_pipelines =
            self.mtoon_pipeline_plans(time_seconds, scene_options, render_options);
        let texture_uploads = self.texture_uploads(&mtoon_pipelines);
        let texture_upload_indices = texture_ref_upload_indices(&texture_uploads);
        let baked = self.bake_primitives(time_seconds, scene_options, render_options)?;
        Ok(AshVrmFramePlan {
            primitives: baked.primitives,
            materials: self.material_records(&texture_upload_indices),
            texture_uploads,
            mtoon_pipelines,
            scene_uniform: AshSceneUniform::from_scene_options(scene_options),
            scene_options,
            diagnostic_owner_ids: baked.diagnostic_owner_ids,
            render_surfaces: baked.render_surfaces,
        })
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

    fn bake_primitives(
        &self,
        mtoon_time: f32,
        scene_options: AshSceneOptions,
        render_options: AshRenderOptions,
    ) -> Result<AshBakedPrimitives, Box<dyn Error>> {
        let mut primitives = Vec::new();
        for (node_index, node) in self.loaded.scene.nodes.iter().enumerate() {
            let Some(mesh_index) = node.mesh else {
                continue;
            };
            let mesh = &self.loaded.meshes[mesh_index];
            for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
                let source_index = AshPrimitiveSourceIndex {
                    node: node_index,
                    mesh: mesh_index,
                    primitive: primitive_index,
                };
                let base = self.bake_primitive(
                    source_index,
                    node,
                    mesh,
                    primitive,
                    AshPrimitiveBakeSettings {
                        pass: AshMtoonPass::Base,
                        mtoon_time,
                        scene_options,
                        render_options,
                    },
                )?;
                let base_draw_order =
                    ash_base_draw_order(&self.loaded, primitive.material.map(MaterialRef));
                primitives.push(AshPrimitiveRecord {
                    primitive: base,
                    source: ash_primitive_source(
                        &self.loaded,
                        node_index,
                        mesh_index,
                        primitive_index,
                        AshMtoonPass::Base,
                    ),
                    draw_order: base_draw_order,
                });
                if !render_options.disable_outlines
                    && self
                        .loaded
                        .expression_mtoon_outline_plan(primitive.material, &self.expression_effects)
                        .is_some()
                {
                    let outline = self.bake_primitive(
                        source_index,
                        node,
                        mesh,
                        primitive,
                        AshPrimitiveBakeSettings {
                            pass: AshMtoonPass::Outline,
                            mtoon_time,
                            scene_options,
                            render_options,
                        },
                    )?;
                    let outline_draw_order =
                        ash_outline_draw_order(&self.loaded, primitive.material.map(MaterialRef));
                    primitives.push(AshPrimitiveRecord {
                        primitive: outline,
                        source: ash_primitive_source(
                            &self.loaded,
                            node_index,
                            mesh_index,
                            primitive_index,
                            AshMtoonPass::Outline,
                        ),
                        draw_order: outline_draw_order,
                    });
                }
            }
        }
        let diagnostic_owner_ids =
            if render_options.diagnostic_render == AshDiagnosticRender::OwnerId {
                assign_ash_owner_id_triangles(&mut primitives, scene_options)
            } else {
                Vec::new()
            };
        let render_surfaces = ash_render_surfaces(&primitives);
        Ok(AshBakedPrimitives {
            primitives: primitives
                .into_iter()
                .map(|record| record.primitive)
                .collect(),
            diagnostic_owner_ids,
            render_surfaces,
        })
    }

    fn bake_primitive(
        &self,
        source_index: AshPrimitiveSourceIndex,
        node: &GltfNodeRest,
        mesh: &vrm_io::GltfMeshData,
        primitive: &GltfPrimitiveData,
        settings: AshPrimitiveBakeSettings,
    ) -> Result<AshVrmPrimitive, Box<dyn Error>> {
        let morph_weights = active_morph_weights(
            &self.scene,
            &self.expression_effects,
            source_index.node,
            node,
            mesh,
        );
        let world_matrices = self.world_matrices();
        let orientation = ash_model_orientation();
        let world = orientation * world_matrices[source_index.node];
        let skin_matrices = node.skin.and_then(|skin| {
            self.loaded
                .skins
                .get(skin)
                .map(|skin| skin.joint_matrices(&self.loaded.scene, &world_matrices, orientation))
        });
        let shading = self.loaded.expression_material_shading_plan(
            primitive.material,
            GltfMaterialShadingOptions::default(),
            &self.expression_effects,
        );
        let normal_scale = if settings.render_options.disable_normal_maps {
            0.0
        } else {
            shading.normal_scale * settings.render_options.normal_map_scale
        };
        let double_sided = primitive
            .material
            .and_then(|index| self.loaded.gltf_materials.get(index))
            .is_some_and(|material| material.double_sided);
        let mut source_vertices = match settings.pass {
            AshMtoonPass::Base => {
                primitive.transformed_vertices(&morph_weights, world, skin_matrices.as_deref())
            }
            AshMtoonPass::Outline
                if settings.render_options.diagnostic_render == AshDiagnosticRender::Shaded =>
            {
                self.outline_vertices(
                    primitive,
                    &morph_weights,
                    world,
                    skin_matrices.as_deref(),
                    settings,
                )
            }
            AshMtoonPass::Outline => {
                primitive.transformed_vertices(&morph_weights, world, skin_matrices.as_deref())
            }
        };
        let source_vertices = source_vertices
            .as_mut()
            .ok_or("primitive geometry is inconsistent")?;
        let mut normal_scales = ash_vertex_normal_scales(
            primitive,
            source_vertices.len(),
            normal_scale,
            settings.render_options.normal_map_mode.into(),
        );
        if settings.pass == AshMtoonPass::Base {
            apply_generated_tangents(
                primitive,
                source_vertices,
                normal_scale,
                settings.render_options.normal_map_mode.into(),
                &mut normal_scales,
            );
        }
        let vertices: Vec<AshVrmVertex> = source_vertices
            .iter()
            .zip(normal_scales)
            .map(|(vertex, normal_scale)| {
                let color = if settings.pass == AshMtoonPass::Outline {
                    [1.0, 1.0, 1.0, 1.0]
                } else if shading.pbr_fallback {
                    multiply_rgba(shading.base_color, vertex.color_0)
                } else {
                    shading.base_color
                };
                AshVrmVertex {
                    position: vertex.position.to_array(),
                    tex_coord_0: vertex.tex_coord_0,
                    tex_coord_0_dx: [0.0, 0.0],
                    tex_coord_0_dy: [0.0, 0.0],
                    color_0: color,
                    normal: vertex.normal.to_array(),
                    tangent: vertex.tangent.to_array(),
                    normal_scale,
                    double_sided: if double_sided { 1.0 } else { 0.0 },
                }
            })
            .collect();
        Ok(AshVrmPrimitive {
            node: NodeRef(source_index.node),
            mesh_index: source_index.mesh,
            primitive_index: source_index.primitive,
            material_name: self
                .loaded
                .material_display_name(primitive.material)
                .map(str::to_owned),
            material: primitive.material.map(MaterialRef),
            pass: settings.pass,
            vertices,
            indices: primitive.indices.clone(),
        })
    }

    fn outline_vertices(
        &self,
        primitive: &GltfPrimitiveData,
        morph_weights: &[f32],
        world: Mat4,
        skin_matrices: Option<&[Mat4]>,
        settings: AshPrimitiveBakeSettings,
    ) -> Option<Vec<vrm_io::GltfTransformedVertex>> {
        let outline = self
            .loaded
            .expression_mtoon_outline_plan(primitive.material, &self.expression_effects)?;
        let width_texture = self
            .loaded
            .material_outline_width_rgba8_image(primitive.material);
        let uv_transforms = self.loaded.expression_material_uv_transforms(
            primitive.material,
            settings.mtoon_time,
            &self.expression_effects,
        );
        primitive.outline_vertices(
            morph_weights,
            GltfOutlineVertexSettings {
                base_width: outline.width_factor * settings.render_options.outline_width_scale,
                scale: GltfOutlineScale::new(
                    outline.width_mode,
                    settings.scene_options.view(),
                    settings.scene_options.projection_y_scale(),
                ),
                width_texture: width_texture.as_ref(),
                width_transform: uv_transforms.outline_width,
                width_texture_origin: Rgba8SamplingOrigin::TopLeft,
            },
            world,
            skin_matrices,
        )
    }

    fn material_records(
        &self,
        texture_uploads: &HashMap<AshTextureUploadKey, usize>,
    ) -> Vec<AshMaterialRecord> {
        self.loaded
            .gltf_materials
            .iter()
            .enumerate()
            .map(|(index, material)| AshMaterialRecord {
                material: MaterialRef(index),
                base_color_factor: material.base_color_factor,
                base_color_texture_upload: self
                    .loaded
                    .material_texture_slots(Some(index))
                    .base
                    .and_then(|texture| {
                        texture_uploads
                            .get(&AshTextureUploadKey {
                                texture: TextureRef(texture),
                                color_space: GltfMaterialTextureColorSpace::Srgb,
                            })
                            .copied()
                    }),
            })
            .collect()
    }

    fn texture_uploads(&self, pipelines: &[AshMtoonPipelinePlan]) -> Vec<AshTextureUpload> {
        required_texture_uploads(&self.loaded, pipelines)
            .into_iter()
            .filter_map(|key| {
                self.loaded
                    .texture_rgba8_image(key.texture.0)
                    .map(|image| texture_upload(Some(key), image))
            })
            .collect()
    }

    fn mtoon_pipeline_plans(
        &self,
        mtoon_time: f32,
        scene_options: AshSceneOptions,
        render_options: AshRenderOptions,
    ) -> Vec<AshMtoonPipelinePlan> {
        let mut plans = mtoon_renderer_material_plans(
            self.loaded.model().document(),
            MtoonMaterializationOptions::default(),
        )
        .into_iter()
        .map(|plan| {
            let renderer_pipeline = ash_renderer_pipeline_plan(&self.loaded, Some(plan.material));
            let key = match plan.pass {
                MtoonRendererPass::Base => AshPipelineKey {
                    pass: AshMtoonPass::Base,
                    render_order: renderer_pipeline.render_order,
                    phase_order: renderer_pipeline.phase_order,
                    topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                    cull_mode: ash_renderer_cull_mode(renderer_pipeline.cull_mode),
                    front_face: vrm_vulkan_front_face(),
                    depth_test_enable: true,
                    depth_write_enable: renderer_pipeline.depth_write,
                    depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                    blend_enable: renderer_pipeline.blend,
                },
                MtoonRendererPass::Outline => {
                    let draw_order = ash_outline_draw_order(&self.loaded, Some(plan.material));
                    AshPipelineKey {
                        pass: AshMtoonPass::Outline,
                        render_order: draw_order.render_order,
                        phase_order: draw_order.phase_order,
                        topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                        cull_mode: vk::CullModeFlags::FRONT,
                        front_face: vrm_vulkan_front_face(),
                        depth_test_enable: true,
                        depth_write_enable: true,
                        depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                        blend_enable: false,
                    }
                }
            };
            let mut gpu = MtoonGpuMaterial::from_renderer_plan(&plan);
            apply_renderer_alpha_policy_to_uniform(&mut gpu.uniform, renderer_pipeline);
            let material = Some(plan.material.0);
            let expression_shading = self.loaded.expression_material_shading_plan(
                material,
                GltfMaterialShadingOptions::default(),
                &self.expression_effects,
            );
            apply_expression_shading_to_mtoon_uniform(
                &mut gpu.uniform,
                expression_shading,
                plan.pass,
            );
            let uv_uniform = AshMaterialUvUniform::from_plan(
                self.loaded
                    .expression_material_uv_transforms(
                        material,
                        mtoon_time,
                        &self.expression_effects,
                    )
                    .uniform_plan(),
            );
            let mut render_extra_uniform = AshMaterialExtraUniform::from_plan(
                self.loaded
                    .expression_material_shading_plan(
                        material,
                        GltfMaterialShadingOptions::default(),
                        &self.expression_effects,
                    )
                    .render_extra_plan(ash_material_render_extra_options(
                        scene_options,
                        render_options,
                    ))
                    .uniform_plan(),
            );
            render_extra_uniform.flags2[2] = render_options.diagnostic_render.flat_flag();
            render_extra_uniform.flags2[3] = render_options.diagnostic_render.mode_code();
            AshMtoonPipelinePlan {
                material: plan.material,
                name: plan.name,
                key,
                descriptor_bindings: descriptor_bindings(
                    &self.loaded.textures,
                    self.loaded.material_texture_slots(material),
                ),
                uniform: gpu.uniform,
                uv_uniform,
                render_extra_uniform,
                uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
                alpha_cutoff: renderer_pipeline.alpha_cutoff,
                outline_width: plan.shader.outline_width_factor,
                base_color_factor: plan.shader.base_color_factor,
                emissive_color: plan.shader.emissive_color,
            }
        })
        .collect::<Vec<_>>();

        let base_materials = plans
            .iter()
            .filter(|plan| plan.key.pass == AshMtoonPass::Base)
            .map(|plan| plan.material)
            .collect::<HashSet<_>>();
        plans.extend(
            (0..self.loaded.gltf_materials.len())
                .map(MaterialRef)
                .filter(|material| !base_materials.contains(material))
                .map(|material| {
                    ash_gltf_base_pipeline_plan(
                        &self.loaded,
                        material,
                        mtoon_time,
                        scene_options,
                        render_options,
                        &self.expression_effects,
                    )
                }),
        );
        plans
    }
}

fn ash_gltf_base_pipeline_plan(
    loaded: &LoadedVrm,
    material: MaterialRef,
    mtoon_time: f32,
    scene_options: AshSceneOptions,
    render_options: AshRenderOptions,
    expression_effects: &GltfExpressionRenderEffects,
) -> AshMtoonPipelinePlan {
    let material_index = Some(material.0);
    let renderer_pipeline = ash_renderer_pipeline_plan(loaded, Some(material));
    let shading = loaded.expression_material_shading_plan(
        material_index,
        GltfMaterialShadingOptions::default(),
        expression_effects,
    );
    let uv_uniform = AshMaterialUvUniform::from_plan(
        loaded
            .expression_material_uv_transforms(material_index, mtoon_time, expression_effects)
            .uniform_plan(),
    );
    let mut render_extra_uniform = AshMaterialExtraUniform::from_plan(
        shading
            .render_extra_plan(ash_material_render_extra_options(
                scene_options,
                render_options,
            ))
            .uniform_plan(),
    );
    render_extra_uniform.flags2[2] = render_options.diagnostic_render.flat_flag();
    render_extra_uniform.flags2[3] = render_options.diagnostic_render.mode_code();
    AshMtoonPipelinePlan {
        material,
        name: loaded
            .material_display_name(material_index)
            .map(str::to_owned),
        key: AshPipelineKey {
            pass: AshMtoonPass::Base,
            render_order: renderer_pipeline.render_order,
            phase_order: renderer_pipeline.phase_order,
            topology: vk::PrimitiveTopology::TRIANGLE_LIST,
            cull_mode: ash_renderer_cull_mode(renderer_pipeline.cull_mode),
            front_face: vrm_vulkan_front_face(),
            depth_test_enable: true,
            depth_write_enable: renderer_pipeline.depth_write,
            depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
            blend_enable: renderer_pipeline.blend,
        },
        descriptor_bindings: descriptor_bindings(
            &loaded.textures,
            loaded.material_texture_slots(material_index),
        ),
        uniform: ash_gltf_base_uniform(shading, renderer_pipeline),
        uv_uniform,
        render_extra_uniform,
        uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
        alpha_cutoff: renderer_pipeline.alpha_cutoff,
        outline_width: 0.0,
        base_color_factor: shading.base_color,
        emissive_color: shading.emissive,
    }
}

fn ash_gltf_base_uniform(
    shading: vrm_io::GltfMaterialShadingPlan,
    pipeline: vrm_adapter::RendererMaterialPipelinePlan,
) -> MtoonGpuUniform {
    MtoonGpuUniform {
        base_color_factor: shading.base_color,
        shade_color_factor_cutoff: [
            shading.shade_color[0],
            shading.shade_color[1],
            shading.shade_color[2],
            pipeline.alpha_cutoff,
        ],
        emissive_color_outline_width: [
            shading.emissive[0],
            shading.emissive[1],
            shading.emissive[2],
            0.0,
        ],
        shading: [1.0, 1.0, shading.shading_shift, shading.shading_toony],
        lighting: [
            shading.shading_shift_texture_scale,
            0.0,
            shading.gi_equalization,
            0.0,
        ],
        matcap_factor_debug: [
            shading.matcap_factor[0],
            shading.matcap_factor[1],
            shading.matcap_factor[2],
            0.0,
        ],
        rim_color_lighting_mix: [
            shading.parametric_rim_color[0],
            shading.parametric_rim_color[1],
            shading.parametric_rim_color[2],
            shading.rim_lighting_mix,
        ],
        rim_params: [
            shading.parametric_rim_fresnel_power,
            shading.parametric_rim_lift,
            pipeline.render_order as f32,
            pipeline.phase_order as f32,
        ],
        outline_color_lighting_mix: [0.0; 4],
        uv_animation: [0.0; 4],
        flags: [
            0,
            u32::from(shading.v0_compat_shade),
            0,
            ash_renderer_alpha_mode_code(pipeline.alpha_mode),
        ],
    }
}

fn apply_renderer_alpha_policy_to_uniform(
    uniform: &mut MtoonGpuUniform,
    pipeline: vrm_adapter::RendererMaterialPipelinePlan,
) {
    uniform.shade_color_factor_cutoff[3] = pipeline.alpha_cutoff;
    uniform.flags[3] = ash_renderer_alpha_mode_code(pipeline.alpha_mode);
}

fn apply_expression_shading_to_mtoon_uniform(
    uniform: &mut MtoonGpuUniform,
    shading: vrm_io::GltfMaterialShadingPlan,
    pass: MtoonRendererPass,
) {
    uniform.base_color_factor = shading.base_color;
    uniform.shade_color_factor_cutoff[0] = shading.shade_color[0];
    uniform.shade_color_factor_cutoff[1] = shading.shade_color[1];
    uniform.shade_color_factor_cutoff[2] = shading.shade_color[2];
    uniform.emissive_color_outline_width[0] = shading.emissive[0];
    uniform.emissive_color_outline_width[1] = shading.emissive[1];
    uniform.emissive_color_outline_width[2] = shading.emissive[2];
    uniform.shading[2] = shading.shading_shift;
    uniform.shading[3] = shading.shading_toony;
    uniform.lighting[0] = shading.shading_shift_texture_scale;
    uniform.lighting[2] = shading.gi_equalization;
    uniform.matcap_factor_debug[0] = shading.matcap_factor[0];
    uniform.matcap_factor_debug[1] = shading.matcap_factor[1];
    uniform.matcap_factor_debug[2] = shading.matcap_factor[2];
    uniform.rim_color_lighting_mix[0] = shading.parametric_rim_color[0];
    uniform.rim_color_lighting_mix[1] = shading.parametric_rim_color[1];
    uniform.rim_color_lighting_mix[2] = shading.parametric_rim_color[2];
    uniform.rim_color_lighting_mix[3] = shading.rim_lighting_mix;
    uniform.rim_params[0] = shading.parametric_rim_fresnel_power;
    uniform.rim_params[1] = shading.parametric_rim_lift;
    uniform.flags[1] = u32::from(shading.v0_compat_shade);
    uniform.flags[2] = match pass {
        MtoonRendererPass::Base => 0,
        MtoonRendererPass::Outline => 1,
    };
}

fn ash_material_render_extra_options(
    scene_options: AshSceneOptions,
    render_options: AshRenderOptions,
) -> GltfMaterialRenderExtraOptions {
    GltfMaterialRenderExtraOptions {
        light_accumulation: match scene_options.lighting.accumulation {
            MtoonLightAccumulation::Tuned => vrm_io::GltfMtoonLightAccumulation::Tuned,
            MtoonLightAccumulation::ThreeVrm => vrm_io::GltfMtoonLightAccumulation::ThreeVrm,
        },
        derivative_normals: render_options.normal_map_mode == AshNormalMapMode::Derivative,
        view_derivative_normals: render_options.normal_map_mode == AshNormalMapMode::ViewDerivative,
        direct_light_scale: scene_options.direct_light_scale,
    }
}

pub fn frame_plan_from_options(
    options: &AshVrmFramePlanOptions,
) -> Result<AshVrmFramePlan, Box<dyn Error>> {
    frame_plan_from_options_with_aspect(options, 1.0)
}

pub fn frame_plan_from_options_with_aspect(
    options: &AshVrmFramePlanOptions,
    aspect_ratio: f32,
) -> Result<AshVrmFramePlan, Box<dyn Error>> {
    let animation = (!options.no_animation).then_some(options.animation.clone());
    let mut planner = AshVrmFramePlanner::from_paths(options.avatar.clone(), animation)?;
    planner.set_expression_weights(parse_expression_args(&options.expressions)?)?;
    planner.sample_frame_with_full_render_options(
        options.time,
        options.scene_options(aspect_ratio),
        options.render_options(),
    )
}

pub fn frame_plan_from_options_with_viewport(
    options: &AshVrmFramePlanOptions,
    width: u32,
    height: u32,
) -> Result<AshVrmFramePlan, Box<dyn Error>> {
    let width = width.max(1);
    let height = height.max(1);
    let aspect_ratio = width as f32 / height as f32;
    let animation = (!options.no_animation).then_some(options.animation.clone());
    let mut planner = AshVrmFramePlanner::from_paths(options.avatar.clone(), animation)?;
    planner.set_expression_weights(parse_expression_args(&options.expressions)?)?;
    planner.sample_frame_with_full_render_options(
        options.time,
        options.scene_options_with_screen_size(
            aspect_ratio,
            ScreenProjectionSize::from_pixels(width, height),
        ),
        options.render_options(),
    )
}

fn required_texture_uploads(
    loaded: &LoadedVrm,
    pipelines: &[AshMtoonPipelinePlan],
) -> Vec<AshTextureUploadKey> {
    let mut textures = Vec::new();
    for material in 0..loaded.gltf_materials.len() {
        let plan = loaded.material_texture_slots(Some(material)).binding_plan();
        for binding in plan.bindings {
            if let Some(texture) = binding.texture {
                push_unique_texture(
                    &mut textures,
                    AshTextureUploadKey {
                        texture: TextureRef(texture),
                        color_space: binding.color_space,
                    },
                );
            }
        }
    }
    for pipeline in pipelines {
        for binding in &pipeline.descriptor_bindings {
            if let Some(texture) = binding.texture {
                push_unique_texture(
                    &mut textures,
                    AshTextureUploadKey {
                        texture,
                        color_space: binding.color_space,
                    },
                );
            }
        }
    }
    textures
}

fn push_unique_texture(textures: &mut Vec<AshTextureUploadKey>, texture: AshTextureUploadKey) {
    if !textures.contains(&texture) {
        textures.push(texture);
    }
}

fn multiply_rgba(left: [f32; 4], right: [f32; 4]) -> [f32; 4] {
    [
        left[0] * right[0],
        left[1] * right[1],
        left[2] * right[2],
        left[3] * right[3],
    ]
}

fn ash_primitive_source(
    loaded: &LoadedVrm,
    node_index: usize,
    mesh_index: usize,
    primitive_index: usize,
    pass: AshMtoonPass,
) -> AshPrimitiveSource {
    let node = loaded.scene.nodes.get(node_index);
    let mesh = loaded.meshes.get(mesh_index);
    let primitive = mesh.and_then(|mesh| mesh.primitives.get(primitive_index));
    let material = primitive
        .and_then(|primitive| primitive.material)
        .map(MaterialRef);
    let material_data = material.and_then(|material| loaded.gltf_materials.get(material.0));
    let alpha_mode = material_data
        .map(|material| material.alpha_mode)
        .unwrap_or(GltfAlphaMode::Opaque);
    AshPrimitiveSource {
        node: NodeRef(node_index),
        node_name: node
            .and_then(|node| node.name.as_deref())
            .map(Arc::<str>::from),
        mesh_index,
        mesh_name: mesh
            .and_then(|mesh| mesh.name.as_deref())
            .map(Arc::<str>::from),
        primitive_index,
        material,
        material_name: loaded
            .material_display_name(material.map(|material| material.0))
            .map(|name| {
                let suffix = match pass {
                    AshMtoonPass::Base => "",
                    AshMtoonPass::Outline => " (Outline)",
                };
                Arc::<str>::from(format!("{name}{suffix}"))
            }),
        pass,
        alpha_mode,
        alpha_cutoff: material_data.and_then(|material| material.alpha_cutoff),
        opacity: material_data
            .map(|material| material.base_color_factor[3])
            .unwrap_or(1.0),
        double_sided: material_data
            .map(|material| material.double_sided)
            .unwrap_or(false),
    }
}

fn ash_model_orientation() -> Mat4 {
    Mat4::from_rotation_y(std::f32::consts::PI)
}

fn ash_owner_id_color(id: u32) -> [f32; 4] {
    RenderOwnerId::new(id).to_rgba_f32()
}

fn ash_base_draw_order(loaded: &LoadedVrm, material: Option<MaterialRef>) -> AshPrimitiveDrawOrder {
    let plan = ash_renderer_pipeline_plan(loaded, material);
    AshPrimitiveDrawOrder {
        render_order: plan.render_order,
        phase_order: plan.phase_order,
    }
}

fn ash_outline_draw_order(
    loaded: &LoadedVrm,
    material: Option<MaterialRef>,
) -> AshPrimitiveDrawOrder {
    let base = ash_base_draw_order(loaded, material);
    AshPrimitiveDrawOrder {
        render_order: base.render_order.saturating_add(1),
        phase_order: base.phase_order.saturating_add(1),
    }
}

fn ash_renderer_pipeline_plan(
    loaded: &LoadedVrm,
    material: Option<MaterialRef>,
) -> vrm_adapter::RendererMaterialPipelinePlan {
    renderer_material_pipeline_plan(
        loaded.model().document(),
        material,
        MtoonMaterializationOptions::default(),
        ash_gltf_pipeline_override(loaded, material),
    )
}

fn ash_gltf_pipeline_override(
    loaded: &LoadedVrm,
    material: Option<MaterialRef>,
) -> Option<GltfMaterialPipelineOverride> {
    material
        .and_then(|material| loaded.gltf_materials.get(material.0))
        .map(|material| GltfMaterialPipelineOverride {
            alpha_mode: ash_gltf_alpha_mode(material.alpha_mode),
            alpha_cutoff: material.alpha_cutoff,
            double_sided: material.double_sided,
        })
}

fn ash_gltf_alpha_mode(mode: GltfAlphaMode) -> GltfMaterialAlphaMode {
    match mode {
        GltfAlphaMode::Opaque => GltfMaterialAlphaMode::Opaque,
        GltfAlphaMode::Mask => GltfMaterialAlphaMode::Mask,
        GltfAlphaMode::Blend => GltfMaterialAlphaMode::Blend,
    }
}

fn ash_renderer_cull_mode(mode: RendererMaterialCullMode) -> vk::CullModeFlags {
    match mode {
        RendererMaterialCullMode::Off => vk::CullModeFlags::NONE,
        RendererMaterialCullMode::Front => vk::CullModeFlags::FRONT,
        RendererMaterialCullMode::Back => vk::CullModeFlags::BACK,
    }
}

fn ash_renderer_alpha_mode_code(mode: RendererMaterialAlphaMode) -> u32 {
    match mode {
        RendererMaterialAlphaMode::Opaque => {
            vrm_adapter::mtoon_alpha_mode_code(MtoonAlphaMode::Opaque)
        }
        RendererMaterialAlphaMode::Mask => vrm_adapter::mtoon_alpha_mode_code(MtoonAlphaMode::Mask),
        RendererMaterialAlphaMode::Blend => {
            vrm_adapter::mtoon_alpha_mode_code(MtoonAlphaMode::Blend)
        }
    }
}

fn ash_owner_sample_relation_code(relation: Option<RenderOwnerSurfaceRelation>) -> u32 {
    match relation {
        Some(RenderOwnerSurfaceRelation::SameSurface) => 1,
        Some(RenderOwnerSurfaceRelation::SameMaterialDifferentTriangle) => 2,
        Some(RenderOwnerSurfaceRelation::DifferentMaterial) => 3,
        Some(RenderOwnerSurfaceRelation::Missing) => 4,
        None => 0,
    }
}

fn ash_owner_sample_pass_code(pass: &RenderOwnerSamplePass) -> u32 {
    match pass {
        RenderOwnerSamplePass::Base => 1,
        RenderOwnerSamplePass::Outline => 2,
        RenderOwnerSamplePass::Other(_) => 255,
    }
}

fn ash_u32_geometry_value(
    field: &'static str,
    value: u64,
) -> Result<u32, AshOwnerSampleOverridePlanError> {
    u32::try_from(value)
        .map_err(|_| AshOwnerSampleOverridePlanError::GeometryIndexOutOfRange { field, value })
}

fn assign_ash_owner_id_triangles(
    primitives: &mut [AshPrimitiveRecord],
    scene_options: AshSceneOptions,
) -> Vec<AshDiagnosticOwnerId> {
    let mut ordered_indices = (0..primitives.len()).collect::<Vec<_>>();
    ordered_indices.sort_by_key(|index| {
        let draw_order = primitives[*index].draw_order;
        (draw_order.render_order, *index)
    });

    let mut next_id = 1_u32;
    let mut owners = Vec::new();
    let view_projection = ash_reference_view_projection(scene_options);
    for (draw_index, primitive_index) in ordered_indices.into_iter().enumerate() {
        let record = &mut primitives[primitive_index];
        let (vertices, indices, next, mut primitive_owners) = ash_owner_id_triangles(
            &record.primitive.vertices,
            &record.primitive.indices,
            next_id,
            AshOwnerIdAssignmentContext {
                source: record.source.clone(),
                draw_order: record.draw_order,
                draw_index,
                pipeline_key: ash_diagnostic_pipeline_key(record),
                view_projection,
                scene_options,
            },
        );
        record.primitive.vertices = vertices;
        record.primitive.indices = indices;
        owners.append(&mut primitive_owners);
        next_id = next;
    }
    owners
}

fn ash_render_surfaces(primitives: &[AshPrimitiveRecord]) -> Vec<RenderOwnerSurfaceKey> {
    primitives
        .iter()
        .flat_map(|record| {
            let material_name = record.source.material_name.clone();
            (0..record.primitive.indices.len() / 3).filter_map(move |triangle| {
                Some(RenderOwnerSurfaceKey::new(
                    material_name.as_deref()?,
                    u64::try_from(triangle).ok()?,
                ))
            })
        })
        .collect()
}

fn ash_owner_id_triangles(
    vertices: &[AshVrmVertex],
    indices: &[u32],
    first_id: u32,
    context: AshOwnerIdAssignmentContext,
) -> (Vec<AshVrmVertex>, Vec<u32>, u32, Vec<AshDiagnosticOwnerId>) {
    let mut expanded_vertices = Vec::with_capacity(indices.len());
    let mut expanded_indices = Vec::with_capacity(indices.len());
    let mut owners = Vec::with_capacity(indices.len() / 3);
    let mut next_id = first_id;
    for (triangle_index, triangle) in indices.chunks_exact(3).enumerate() {
        let color = ash_owner_id_color(next_id);
        let indices = [triangle[0], triangle[1], triangle[2]];
        owners.push(AshDiagnosticOwnerId {
            id: next_id,
            color: ash_owner_id_color_u8(next_id),
            source: AshDiagnosticOwnerSource {
                node: context.source.node,
                node_name: context.source.node_name.clone(),
                mesh_index: context.source.mesh_index,
                mesh_name: context.source.mesh_name.clone(),
                primitive_index: context.source.primitive_index,
                material: context.source.material,
                material_name: context.source.material_name.clone(),
                pass: context.source.pass,
                alpha_mode: context.source.alpha_mode,
                alpha_cutoff: context.source.alpha_cutoff,
                opacity: context.source.opacity,
                double_sided: context.source.double_sided,
                render_order: context.draw_order.render_order,
                phase_order: context.draw_order.phase_order,
                draw_index: context.draw_index,
                cull_mode: context.pipeline_key.cull_mode,
                front_face: context.pipeline_key.front_face,
                depth_write: context.pipeline_key.depth_write_enable,
                depth_test: context.pipeline_key.depth_test_enable,
                depth_compare: context.pipeline_key.depth_compare_op,
                blend: context.pipeline_key.blend_enable,
            },
            triangle: triangle_index,
            indices,
            projection: ash_owner_triangle_projection(
                vertices,
                indices,
                context.view_projection,
                context.scene_options,
                context.pipeline_key.cull_mode,
            ),
        });
        for index in triangle {
            if let Some(vertex) = vertices.get(*index as usize) {
                let mut vertex = *vertex;
                vertex.color_0 = color;
                expanded_indices.push(u32::try_from(expanded_vertices.len()).unwrap_or(u32::MAX));
                expanded_vertices.push(vertex);
            }
        }
        next_id = next_id.saturating_add(1);
    }
    (expanded_vertices, expanded_indices, next_id, owners)
}

fn ash_owner_id_color_u8(id: u32) -> [u8; 4] {
    RenderOwnerId::new(id).to_rgba_u8()
}

fn ash_reference_view_projection(scene_options: AshSceneOptions) -> Mat4 {
    Mat4::perspective_rh(
        30.0_f32.to_radians(),
        scene_options.sanitized_aspect_ratio(),
        0.1,
        20.0,
    ) * scene_options.view()
}

fn ash_diagnostic_pipeline_key(record: &AshPrimitiveRecord) -> AshPipelineKey {
    AshPipelineKey {
        pass: record.source.pass,
        render_order: record.draw_order.render_order,
        phase_order: record.draw_order.phase_order,
        topology: vk::PrimitiveTopology::TRIANGLE_LIST,
        cull_mode: match record.source.pass {
            AshMtoonPass::Base => vk::CullModeFlags::BACK,
            AshMtoonPass::Outline => vk::CullModeFlags::FRONT,
        },
        front_face: vrm_vulkan_front_face(),
        depth_test_enable: true,
        depth_write_enable: true,
        depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
        blend_enable: false,
    }
}

fn ash_owner_triangle_projection(
    vertices: &[AshVrmVertex],
    indices: [u32; 3],
    view_projection: Mat4,
    scene_options: AshSceneOptions,
    cull_mode: vk::CullModeFlags,
) -> Option<AshDiagnosticOwnerProjection> {
    let projection = project_triangle_to_screen::<ZeroToOneDepth>(
        [
            vertices.get(indices[0] as usize)?.position,
            vertices.get(indices[1] as usize)?.position,
            vertices.get(indices[2] as usize)?.position,
        ],
        view_projection,
        scene_options.sanitized_screen_projection_size(),
        RendererFrontFace::Ccw,
    )?;
    Some(AshDiagnosticOwnerProjection {
        screen: projection.screen,
        bounds: projection.bounds,
        ndc_depth: projection.ndc_depth,
        webgl_depth: projection.webgl_depth,
        screen_signed_area: projection.screen_signed_area,
        front_facing: projection.front_facing,
        gpu_front_facing: projection.gpu_front_facing,
        visible_by_cull_policy: ash_visible_by_cull_policy(cull_mode, projection),
    })
}

fn ash_visible_by_cull_policy(
    cull_mode: vk::CullModeFlags,
    projection: ScreenTriangleProjection,
) -> bool {
    if cull_mode.is_empty() {
        true
    } else if cull_mode == vk::CullModeFlags::BACK {
        projection.gpu_front_facing
    } else if cull_mode == vk::CullModeFlags::FRONT {
        !projection.gpu_front_facing
    } else {
        false
    }
}

fn ash_vertex_normal_scales(
    primitive: &GltfPrimitiveData,
    vertex_count: usize,
    normal_scale: f32,
    normal_map_mode: GltfNormalMapMode,
) -> Vec<f32> {
    let normal_plan = primitive.normal_map_plan(normal_scale, normal_map_mode);
    (0..vertex_count)
        .map(|index| normal_plan.vertex_normal_scale(primitive.tangents.get(index).is_some()))
        .collect()
}

fn apply_generated_tangents(
    primitive: &GltfPrimitiveData,
    vertices: &mut [vrm_io::GltfTransformedVertex],
    normal_scale: f32,
    normal_map_mode: GltfNormalMapMode,
    normal_scales: &mut [f32],
) {
    let normal_plan = primitive.normal_map_plan(normal_scale, normal_map_mode);
    if !normal_plan.should_generate_tangents() {
        return;
    }

    let positions = vertices
        .iter()
        .map(|vertex| vertex.position.to_array())
        .collect::<Vec<_>>();
    let normals = vertices
        .iter()
        .map(|vertex| vertex.normal.to_array())
        .collect::<Vec<_>>();
    let tex_coords = vertices
        .iter()
        .map(|vertex| vertex.tex_coord_0)
        .collect::<Vec<_>>();
    let Some(generated) = generate_tangents(&positions, &normals, &tex_coords, &primitive.indices)
    else {
        return;
    };
    for ((vertex, normal_scale), tangent) in vertices
        .iter_mut()
        .zip(normal_scales.iter_mut())
        .zip(generated.tangents)
    {
        if let Some(tangent) = tangent {
            vertex.tangent = tangent.into();
            *normal_scale = normal_plan.normal_scale;
        }
    }
}

fn texture_ref_upload_indices(
    texture_uploads: &[AshTextureUpload],
) -> HashMap<AshTextureUploadKey, usize> {
    texture_uploads
        .iter()
        .enumerate()
        .filter_map(|(upload_index, upload)| {
            upload.texture.map(|texture| {
                (
                    AshTextureUploadKey {
                        texture,
                        color_space: upload.color_space,
                    },
                    upload_index,
                )
            })
        })
        .collect()
}

fn mtoon_pipeline_indices(
    pipelines: &[AshMtoonPipelinePlan],
) -> HashMap<(MaterialRef, AshMtoonPass), usize> {
    pipelines
        .iter()
        .enumerate()
        .map(|(index, pipeline)| ((pipeline.material, pipeline.key.pass), index))
        .collect()
}

fn descriptor_set_indices(
    descriptor_sets: &[AshDescriptorSetPlan],
) -> HashMap<(MaterialRef, usize), usize> {
    descriptor_sets
        .iter()
        .enumerate()
        .map(|(index, set)| ((set.material, set.pipeline_plan_index), index))
        .collect()
}

fn vrm_vulkan_front_face() -> vk::FrontFace {
    vk::FrontFace::COUNTER_CLOCKWISE
}

pub const fn ash_reference_depth_format() -> vk::Format {
    vk::Format::D24_UNORM_S8_UINT
}

pub const fn ash_mtoon_uniform_binding() -> u32 {
    0
}

pub const fn ash_mtoon_scene_binding() -> u32 {
    9
}

pub const fn ash_mtoon_uv_uniform_binding() -> u32 {
    10
}

pub const fn ash_mtoon_render_extra_binding() -> u32 {
    11
}

pub const fn ash_mtoon_texture_binding(slot: MtoonTextureSlot) -> u32 {
    match slot {
        MtoonTextureSlot::Main => 1,
        MtoonTextureSlot::ShadeMultiply => 2,
        MtoonTextureSlot::ShadingShift => 3,
        MtoonTextureSlot::Normal => 4,
        MtoonTextureSlot::Matcap => 5,
        MtoonTextureSlot::RimMultiply => 6,
        MtoonTextureSlot::OutlineWidth => 7,
        MtoonTextureSlot::UvAnimationMask => 8,
    }
}

pub const fn ash_material_texture_binding(slot: GltfMaterialTextureSlot) -> u32 {
    match slot {
        GltfMaterialTextureSlot::Base => 1,
        GltfMaterialTextureSlot::Shade => 2,
        GltfMaterialTextureSlot::ShadingShift => 3,
        GltfMaterialTextureSlot::Normal => 4,
        GltfMaterialTextureSlot::Matcap => 5,
        GltfMaterialTextureSlot::Rim => 6,
        GltfMaterialTextureSlot::UvAnimationMask => 8,
        GltfMaterialTextureSlot::Emissive => 12,
        GltfMaterialTextureSlot::Occlusion => 13,
    }
}

pub const fn ash_texture_fallback_for_binding(binding: u32) -> Option<GltfMaterialTextureFallback> {
    match binding {
        1 | 2 | 6 | 7 | 8 | 12 | 13 => Some(GltfMaterialTextureFallback::White),
        3 | 5 => Some(GltfMaterialTextureFallback::Black),
        4 => Some(GltfMaterialTextureFallback::NeutralNormal),
        _ => None,
    }
}

fn descriptor_bindings(
    textures: &[GltfTextureData],
    slots: GltfMaterialTextureSlots,
) -> Vec<AshDescriptorBindingPlan> {
    let plan = slots.binding_plan();
    let mut result = Vec::with_capacity(15);
    result.push(AshDescriptorBindingPlan {
        binding: ash_mtoon_uniform_binding(),
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: None,
    });
    result.extend(
        ASH_GLTF_TEXTURE_SLOTS_BEFORE_OUTLINE
            .iter()
            .filter_map(|slot| plan.binding(*slot))
            .map(|binding| descriptor_binding_for_texture(textures, binding)),
    );
    result.push(AshDescriptorBindingPlan {
        binding: ash_mtoon_texture_binding(MtoonTextureSlot::OutlineWidth),
        descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        texture: slots.outline_width.map(TextureRef),
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: Some(sampler_plan_for_texture(
            textures,
            slots.outline_width,
            MtoonSamplerHint::LinearRepeat,
        )),
    });
    result.extend(
        ASH_GLTF_TEXTURE_SLOTS_AFTER_OUTLINE
            .iter()
            .filter_map(|slot| plan.binding(*slot))
            .map(|binding| descriptor_binding_for_texture(textures, binding)),
    );
    result.push(AshDescriptorBindingPlan {
        binding: ash_mtoon_scene_binding(),
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: None,
    });
    result.push(AshDescriptorBindingPlan {
        binding: ash_mtoon_uv_uniform_binding(),
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: None,
    });
    result.push(AshDescriptorBindingPlan {
        binding: ash_mtoon_render_extra_binding(),
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: None,
    });
    result.push(AshDescriptorBindingPlan {
        binding: ash_owner_sample_override_binding(),
        descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: None,
    });
    result
}

const ASH_MTOON_UNIFORMS_PER_PIPELINE: usize = 3;

const ASH_GLTF_TEXTURE_SLOTS_BEFORE_OUTLINE: [GltfMaterialTextureSlot; 6] = [
    GltfMaterialTextureSlot::Base,
    GltfMaterialTextureSlot::Shade,
    GltfMaterialTextureSlot::ShadingShift,
    GltfMaterialTextureSlot::Normal,
    GltfMaterialTextureSlot::Matcap,
    GltfMaterialTextureSlot::Rim,
];

const ASH_GLTF_TEXTURE_SLOTS_AFTER_OUTLINE: [GltfMaterialTextureSlot; 3] = [
    GltfMaterialTextureSlot::Emissive,
    GltfMaterialTextureSlot::Occlusion,
    GltfMaterialTextureSlot::UvAnimationMask,
];

fn descriptor_binding_for_texture(
    textures: &[GltfTextureData],
    binding: GltfMaterialTextureBinding,
) -> AshDescriptorBindingPlan {
    AshDescriptorBindingPlan {
        binding: ash_material_texture_binding(binding.slot),
        descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        texture: binding.texture.map(TextureRef),
        color_space: binding.color_space,
        sampler: Some(sampler_plan_for_texture(
            textures,
            binding.texture,
            sampler_hint_for_material_slot(binding.slot),
        )),
    }
}

fn sampler_hint_for_material_slot(slot: GltfMaterialTextureSlot) -> MtoonSamplerHint {
    match slot {
        GltfMaterialTextureSlot::Normal => MtoonSamplerHint::NormalMapLinearRepeat,
        GltfMaterialTextureSlot::Base
        | GltfMaterialTextureSlot::Shade
        | GltfMaterialTextureSlot::ShadingShift
        | GltfMaterialTextureSlot::Matcap
        | GltfMaterialTextureSlot::Rim
        | GltfMaterialTextureSlot::Emissive
        | GltfMaterialTextureSlot::Occlusion
        | GltfMaterialTextureSlot::UvAnimationMask => MtoonSamplerHint::LinearRepeat,
    }
}

fn sampler_plan(hint: MtoonSamplerHint) -> AshSamplerPlan {
    AshSamplerPlan {
        mag_filter: vk::Filter::LINEAR,
        min_filter: vk::Filter::LINEAR,
        mipmap_mode: vk::SamplerMipmapMode::LINEAR,
        address_mode_u: vk::SamplerAddressMode::REPEAT,
        address_mode_v: vk::SamplerAddressMode::REPEAT,
        min_lod: 0.0,
        max_lod: 32.0,
        normal_map_decode: matches!(hint, MtoonSamplerHint::NormalMapLinearRepeat),
    }
}

fn sampler_plan_for_texture(
    textures: &[GltfTextureData],
    texture: Option<usize>,
    hint: MtoonSamplerHint,
) -> AshSamplerPlan {
    let mut plan = texture
        .and_then(|texture| textures.get(texture))
        .map(|texture| ash_sampler_plan(texture.sampler))
        .unwrap_or_else(|| sampler_plan(MtoonSamplerHint::LinearRepeat));
    plan.normal_map_decode = matches!(hint, MtoonSamplerHint::NormalMapLinearRepeat);
    plan
}

fn ash_sampler_plan(sampler: GltfSamplerData) -> AshSamplerPlan {
    AshSamplerPlan {
        mag_filter: ash_mag_filter(sampler.mag_filter),
        min_filter: ash_min_filter(sampler.min_filter),
        mipmap_mode: ash_mipmap_mode(sampler.min_filter),
        address_mode_u: ash_address_mode(sampler.wrap_s),
        address_mode_v: ash_address_mode(sampler.wrap_t),
        min_lod: 0.0,
        max_lod: if sampler.min_filter.uses_mipmaps() {
            32.0
        } else {
            0.0
        },
        normal_map_decode: false,
    }
}

fn ash_address_mode(mode: GltfWrapMode) -> vk::SamplerAddressMode {
    match mode {
        GltfWrapMode::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        GltfWrapMode::MirroredRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
        GltfWrapMode::Repeat => vk::SamplerAddressMode::REPEAT,
    }
}

fn ash_mag_filter(filter: GltfMagFilter) -> vk::Filter {
    match filter {
        GltfMagFilter::Nearest => vk::Filter::NEAREST,
        GltfMagFilter::Linear => vk::Filter::LINEAR,
    }
}

fn ash_min_filter(filter: GltfMinFilter) -> vk::Filter {
    match filter {
        GltfMinFilter::Nearest
        | GltfMinFilter::NearestMipmapNearest
        | GltfMinFilter::NearestMipmapLinear => vk::Filter::NEAREST,
        GltfMinFilter::Linear
        | GltfMinFilter::LinearMipmapNearest
        | GltfMinFilter::LinearMipmapLinear => vk::Filter::LINEAR,
    }
}

fn ash_mipmap_mode(filter: GltfMinFilter) -> vk::SamplerMipmapMode {
    match filter {
        GltfMinFilter::Nearest
        | GltfMinFilter::Linear
        | GltfMinFilter::NearestMipmapNearest
        | GltfMinFilter::LinearMipmapNearest => vk::SamplerMipmapMode::NEAREST,
        GltfMinFilter::NearestMipmapLinear | GltfMinFilter::LinearMipmapLinear => {
            vk::SamplerMipmapMode::LINEAR
        }
    }
}

fn texture_upload(texture: Option<AshTextureUploadKey>, image: CpuRgba8Image) -> AshTextureUpload {
    let color_space = texture
        .map(|texture| texture.color_space)
        .unwrap_or(GltfMaterialTextureColorSpace::Srgb);
    AshTextureUpload {
        texture: texture.map(|texture| texture.texture),
        color_space,
        format: ash_texture_format(color_space),
        extent: vk::Extent3D {
            width: image.width,
            height: image.height,
            depth: 1,
        },
        rgba: image.rgba,
    }
}

fn ash_texture_format(color_space: GltfMaterialTextureColorSpace) -> vk::Format {
    match color_space {
        GltfMaterialTextureColorSpace::Srgb => vk::Format::R8G8B8A8_SRGB,
        GltfMaterialTextureColorSpace::Linear => vk::Format::R8G8B8A8_UNORM,
    }
}

fn animation_from_loaded(loaded: &LoadedVrm) -> Option<VrmAnimation> {
    match &loaded.model().document().animation {
        Feature::Present(animation) => Some(animation.clone()),
        Feature::Absent => loaded.model().document().animations.first().cloned(),
    }
}

fn active_morph_weights(
    scene: &HeadlessSceneState,
    expression_effects: &GltfExpressionRenderEffects,
    node_index: usize,
    node: &GltfNodeRest,
    mesh: &vrm_io::GltfMeshData,
) -> Vec<f32> {
    let mut weights = expression_effects.active_morph_weights(node_index, node, mesh);
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

fn parse_expression_args(args: &[String]) -> Result<Vec<(String, f32)>, Box<dyn Error>> {
    args.iter()
        .map(|arg| {
            let Some((name, weight)) = arg.split_once('=') else {
                return Err(format!("invalid expression '{arg}', expected name=weight").into());
            };
            let weight = weight
                .parse::<f32>()
                .map_err(|err| format!("invalid expression weight in '{arg}': {err}"))?;
            Ok((name.to_owned(), weight))
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_primitive_source(pass: AshMtoonPass) -> AshPrimitiveSource {
        AshPrimitiveSource {
            node: NodeRef(0),
            node_name: Some(Arc::<str>::from("test-node")),
            mesh_index: 0,
            mesh_name: Some(Arc::<str>::from("test-mesh")),
            primitive_index: 0,
            material: Some(MaterialRef(0)),
            material_name: Some(Arc::<str>::from("test-material")),
            pass,
            alpha_mode: GltfAlphaMode::Opaque,
            alpha_cutoff: None,
            opacity: 1.0,
            double_sided: false,
        }
    }

    #[test]
    fn ash_sampler_hint_marks_normal_decode() {
        assert!(sampler_plan(MtoonSamplerHint::NormalMapLinearRepeat).normal_map_decode);
        assert!(!sampler_plan(MtoonSamplerHint::LinearRepeat).normal_map_decode);
    }

    #[test]
    fn owner_sample_override_buffer_plans_are_storage_ready() {
        let surface = RenderOwnerSurfaceKey::new("body", 7);
        let selection = RenderOwnerSampleSelectionPlan {
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

        let buffers = ash_owner_sample_override_buffer_plans(&selection).unwrap();

        assert_eq!(buffers.len(), 1);
        assert_eq!(buffers[0].surface, surface);
        assert_eq!(buffers[0].record_count(), 1);
        assert_eq!(buffers[0].binding, ash_owner_sample_override_binding());
        assert!(buffers[0].binding > 19);
        assert!(
            buffers[0]
                .usage
                .contains(vk::BufferUsageFlags::STORAGE_BUFFER)
        );
        assert_eq!(
            buffers[0].descriptor_type,
            vk::DescriptorType::STORAGE_BUFFER
        );
        assert!(
            buffers[0]
                .stage_flags
                .contains(vk::ShaderStageFlags::FRAGMENT)
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
            ASH_OWNER_SAMPLE_OVERRIDE_RECORD_SIZE
        );
        let layout_binding = ash_owner_sample_override_descriptor_set_layout_binding();
        assert_eq!(layout_binding.binding, ash_owner_sample_override_binding());
        assert_eq!(
            layout_binding.descriptor_type,
            vk::DescriptorType::STORAGE_BUFFER
        );
        assert_eq!(layout_binding.descriptor_count, 1);
        assert!(
            layout_binding
                .stage_flags
                .contains(vk::ShaderStageFlags::FRAGMENT)
        );
        let matching_draw = RenderOwnerSampleDrawKey::new(2, 3, 4, RenderOwnerSamplePass::Base);
        let draw_plan = ash_owner_sample_override_buffer_plan_for_surfaces_and_draw(
            &selection,
            [RenderOwnerSurfaceKey::new("body", 7)],
            &matching_draw,
        )
        .unwrap();
        assert_eq!(draw_plan.record_count(), 1);
        let other_draw = RenderOwnerSampleDrawKey::new(9, 3, 4, RenderOwnerSamplePass::Base);
        let filtered_plan = ash_owner_sample_override_buffer_plan_for_surfaces_and_draw(
            &selection,
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

    #[test]
    fn descriptor_bindings_start_with_uniform_buffer() {
        let textures = vec![GltfTextureData {
            sampler: GltfSamplerData {
                mag_filter: GltfMagFilter::Nearest,
                min_filter: GltfMinFilter::Linear,
                wrap_s: GltfWrapMode::ClampToEdge,
                wrap_t: GltfWrapMode::MirroredRepeat,
            },
            ..Default::default()
        }];
        let bindings = descriptor_bindings(
            &textures,
            GltfMaterialTextureSlots {
                base: Some(0),
                outline_width: Some(6),
                emissive: Some(7),
                occlusion: Some(8),
                ..Default::default()
            },
        );
        assert_eq!(bindings.len(), 15);
        assert_eq!(bindings[0].binding, ash_mtoon_uniform_binding());
        assert_eq!(
            bindings[0].descriptor_type,
            vk::DescriptorType::UNIFORM_BUFFER
        );
        assert_eq!(
            bindings[1].binding,
            ash_mtoon_texture_binding(MtoonTextureSlot::Main)
        );
        assert_eq!(bindings[1].texture, Some(TextureRef(0)));
        let base_sampler = bindings[1].sampler.unwrap();
        assert_eq!(base_sampler.mag_filter, vk::Filter::NEAREST);
        assert_eq!(base_sampler.min_filter, vk::Filter::LINEAR);
        assert_eq!(
            base_sampler.address_mode_u,
            vk::SamplerAddressMode::CLAMP_TO_EDGE
        );
        assert_eq!(
            base_sampler.address_mode_v,
            vk::SamplerAddressMode::MIRRORED_REPEAT
        );
        assert_eq!(base_sampler.max_lod, 0.0);
        assert_eq!(
            bindings[4].binding,
            ash_mtoon_texture_binding(MtoonTextureSlot::Normal)
        );
        assert!(bindings[4].sampler.unwrap().normal_map_decode);
        assert_eq!(
            bindings[7].binding,
            ash_mtoon_texture_binding(MtoonTextureSlot::OutlineWidth)
        );
        assert_eq!(bindings[7].texture, Some(TextureRef(6)));
        assert_eq!(
            bindings[8].binding,
            ash_material_texture_binding(GltfMaterialTextureSlot::Emissive)
        );
        assert_eq!(bindings[8].texture, Some(TextureRef(7)));
        assert_eq!(
            bindings[9].binding,
            ash_material_texture_binding(GltfMaterialTextureSlot::Occlusion)
        );
        assert_eq!(bindings[9].texture, Some(TextureRef(8)));
        assert_eq!(bindings[11].binding, ash_mtoon_scene_binding());
        assert_eq!(
            bindings[11].descriptor_type,
            vk::DescriptorType::UNIFORM_BUFFER
        );
        assert_eq!(bindings[12].binding, ash_mtoon_uv_uniform_binding());
        assert_eq!(bindings[13].binding, ash_mtoon_render_extra_binding());
        assert_eq!(bindings[14].binding, ash_owner_sample_override_binding());
        assert_eq!(
            bindings[14].descriptor_type,
            vk::DescriptorType::STORAGE_BUFFER
        );
        assert_eq!(bindings[14].stage_flags, vk::ShaderStageFlags::FRAGMENT);
        assert_eq!(
            ash_texture_fallback_for_binding(ash_material_texture_binding(
                GltfMaterialTextureSlot::Normal
            )),
            Some(GltfMaterialTextureFallback::NeutralNormal)
        );
        assert_eq!(
            ash_texture_fallback_for_binding(ash_material_texture_binding(
                GltfMaterialTextureSlot::ShadingShift
            )),
            Some(GltfMaterialTextureFallback::Black)
        );
        assert_eq!(
            ash_texture_fallback_for_binding(ash_material_texture_binding(
                GltfMaterialTextureSlot::Base
            )),
            Some(GltfMaterialTextureFallback::White)
        );
        assert!(
            bindings[12]
                .stage_flags
                .contains(vk::ShaderStageFlags::FRAGMENT)
        );
        assert!(
            bindings[13]
                .stage_flags
                .contains(vk::ShaderStageFlags::FRAGMENT)
        );
    }

    #[test]
    fn scene_uniform_exposes_stable_camera_light_abi() {
        let uniform = AshSceneUniform::parity_camera(1.0);
        assert_eq!(std::mem::size_of::<AshSceneUniform>(), 256);
        assert_eq!(uniform.bytes().len(), 256);
        assert_eq!(uniform.camera_pos, [0.0, 1.0, -5.0, 1.0]);
        assert_eq!(uniform.light_color, [1.0, 1.0, 1.0, 0.0]);
        assert_ne!(uniform.view_projection, Mat4::IDENTITY.to_cols_array_2d());

        let custom = AshSceneUniform::from_scene_options(AshSceneOptions {
            aspect_ratio: 2.0,
            screen_projection_size: ScreenProjectionSize::from_pixels(128, 64),
            camera_y: 1.25,
            camera_z: 3.0,
            target_y: 0.75,
            direct_light_scale: 0.5,
            directional_color: [1.0, 0.5, 0.25],
            lighting: MtoonLightingConfig {
                pbr_ambient: 0.2,
                ..Default::default()
            },
        });
        assert_eq!(custom.camera_pos, [0.0, 1.25, -3.0, 1.0]);
        assert_eq!(custom.light_dir[3], 0.5);
        assert_eq!(custom.light_color, [1.0, 0.5, 0.25, 0.0]);
        assert_eq!(custom.mtoon_lighting[3], 0.2);
    }

    #[test]
    fn scene_options_carry_diagnostic_projection_size() {
        let options = AshVrmFramePlanOptions::parse_from(["test"]);
        let aspect_only = options.scene_options(2.0);
        assert_eq!(
            aspect_only.screen_projection_size,
            ScreenProjectionSize {
                width: 128.0,
                height: 64.0,
            }
        );

        let viewport = options
            .scene_options_with_screen_size(2.0, ScreenProjectionSize::from_pixels(256, 128));
        assert_eq!(
            viewport.sanitized_screen_projection_size(),
            ScreenProjectionSize {
                width: 256.0,
                height: 128.0,
            }
        );

        let invalid = AshSceneOptions {
            screen_projection_size: ScreenProjectionSize {
                width: f32::NAN,
                height: 0.0,
            },
            ..Default::default()
        };
        assert_eq!(
            invalid.sanitized_screen_projection_size(),
            ScreenProjectionSize {
                width: 64.0,
                height: 64.0,
            }
        );
    }

    #[test]
    fn material_extra_uniforms_expose_stable_abi() {
        let uv = AshMaterialUvUniform::default();
        let extra = AshMaterialExtraUniform::default();
        assert_eq!(std::mem::size_of::<AshMaterialUvUniform>(), 192);
        assert_eq!(uv.bytes().len(), 192);
        assert_eq!(uv.base_transform, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(std::mem::size_of::<AshMaterialExtraUniform>(), 64);
        assert_eq!(extra.bytes().len(), 64);
        assert_eq!(extra.pbr_params, [0.0, 1.0, 1.0, 1.0]);
        assert_eq!(AshDiagnosticRender::Flat.flat_flag(), 1.0);
        assert_eq!(AshDiagnosticRender::Shaded.flat_flag(), 0.0);
        assert_eq!(AshDiagnosticRender::BaseFactor.mode_code(), -1.0);
        assert_eq!(AshDiagnosticRender::BaseColor.mode_code(), 1.0);
        assert_eq!(AshDiagnosticRender::BaseColorFlipV.mode_code(), 2.0);
        assert_eq!(AshDiagnosticRender::BaseColorRawSrgb.mode_code(), 1.25);
        assert_eq!(AshDiagnosticRender::Uv.mode_code(), 3.0);
        assert_eq!(AshDiagnosticRender::BaseUv.mode_code(), 4.0);
        assert_eq!(AshDiagnosticRender::OwnerId.mode_code(), 5.0);
        assert_eq!(
            ash_owner_id_color(0x000102),
            [2.0 / 255.0, 1.0 / 255.0, 0.0, 1.0]
        );
    }

    #[test]
    fn render_extra_options_preserve_scene_light_policy() {
        let scene_options = AshSceneOptions {
            direct_light_scale: 0.25,
            lighting: MtoonLightingConfig {
                accumulation: MtoonLightAccumulation::Tuned,
                ..Default::default()
            },
            ..Default::default()
        };

        let derivative = ash_material_render_extra_options(
            scene_options,
            AshRenderOptions {
                normal_map_mode: AshNormalMapMode::Derivative,
                ..Default::default()
            },
        );
        assert_eq!(
            derivative.light_accumulation,
            vrm_io::GltfMtoonLightAccumulation::Tuned
        );
        assert!(derivative.derivative_normals);
        assert!(!derivative.view_derivative_normals);
        assert_eq!(derivative.direct_light_scale, 0.25);

        let view_derivative = ash_material_render_extra_options(
            scene_options,
            AshRenderOptions {
                normal_map_mode: AshNormalMapMode::ViewDerivative,
                ..Default::default()
            },
        );
        assert!(!view_derivative.derivative_normals);
        assert!(view_derivative.view_derivative_normals);
    }

    #[test]
    fn gltf_base_uniform_preserves_alpha_and_pbr_inputs() {
        let shading = vrm_io::GltfMaterialShadingPlan {
            base_color: [0.25, 0.5, 0.75, 0.6],
            shade_color: [0.1, 0.2, 0.3, 1.0],
            shading_shift: 0.4,
            shading_toony: 0.5,
            shading_shift_texture_scale: 0.6,
            gi_equalization: 0.7,
            emissive: [0.8, 0.9, 1.0],
            matcap_factor: [0.0, 0.0, 0.0],
            parametric_rim_color: [0.0, 0.0, 0.0],
            rim_lighting_mix: 0.0,
            parametric_rim_fresnel_power: 1.0,
            parametric_rim_lift: 0.0,
            normal_scale: 1.0,
            metallic: 0.2,
            roughness: 0.3,
            occlusion_strength: 0.4,
            pbr_fallback: true,
            unlit: false,
            v0_compat_shade: true,
        };
        let pipeline = vrm_adapter::RendererMaterialPipelinePlan {
            alpha_mode: RendererMaterialAlphaMode::Mask,
            alpha_cutoff: 0.42,
            render_order: 2450,
            phase_order: 19,
            ..Default::default()
        };

        let uniform = ash_gltf_base_uniform(shading, pipeline);

        assert_eq!(uniform.base_color_factor, shading.base_color);
        assert_eq!(uniform.shade_color_factor_cutoff[3], 0.42);
        assert_eq!(uniform.emissive_color_outline_width, [0.8, 0.9, 1.0, 0.0]);
        assert_eq!(uniform.rim_params[2], 2450.0);
        assert_eq!(uniform.rim_params[3], 19.0);
        assert_eq!(
            uniform.flags[3],
            vrm_adapter::mtoon_alpha_mode_code(MtoonAlphaMode::Mask)
        );
        assert_eq!(uniform.flags[1], 1);
    }

    #[test]
    fn mtoon_uniform_uses_renderer_alpha_policy_override() {
        let mut uniform = MtoonGpuUniform::zeroed();
        uniform.shade_color_factor_cutoff[3] = 0.5;
        uniform.flags[3] = vrm_adapter::mtoon_alpha_mode_code(MtoonAlphaMode::Opaque);
        let pipeline = vrm_adapter::RendererMaterialPipelinePlan {
            alpha_mode: RendererMaterialAlphaMode::Blend,
            alpha_cutoff: 0.25,
            blend: true,
            depth_write: false,
            ..Default::default()
        };

        apply_renderer_alpha_policy_to_uniform(&mut uniform, pipeline);

        assert_eq!(uniform.shade_color_factor_cutoff[3], 0.25);
        assert_eq!(
            uniform.flags[3],
            vrm_adapter::mtoon_alpha_mode_code(MtoonAlphaMode::Blend)
        );
    }

    #[test]
    fn mtoon_uniform_can_apply_expression_shading_values() {
        let mut uniform = MtoonGpuUniform::zeroed();
        let shading = vrm_io::GltfMaterialShadingPlan {
            base_color: [0.9, 0.8, 0.7, 0.6],
            shade_color: [0.1, 0.2, 0.3, 1.0],
            shading_shift: -0.25,
            shading_toony: 0.75,
            shading_shift_texture_scale: 0.5,
            gi_equalization: 0.4,
            emissive: [0.3, 0.2, 0.1],
            matcap_factor: [0.6, 0.5, 0.4],
            parametric_rim_color: [0.7, 0.6, 0.5],
            rim_lighting_mix: 0.25,
            parametric_rim_fresnel_power: 2.0,
            parametric_rim_lift: 0.125,
            normal_scale: 1.0,
            metallic: 0.0,
            roughness: 1.0,
            occlusion_strength: 1.0,
            pbr_fallback: false,
            unlit: false,
            v0_compat_shade: true,
        };

        apply_expression_shading_to_mtoon_uniform(&mut uniform, shading, MtoonRendererPass::Base);

        assert_eq!(uniform.base_color_factor, [0.9, 0.8, 0.7, 0.6]);
        assert_eq!(&uniform.shade_color_factor_cutoff[0..3], &[0.1, 0.2, 0.3]);
        assert_eq!(
            &uniform.emissive_color_outline_width[0..3],
            &[0.3, 0.2, 0.1]
        );
        assert_eq!(uniform.shading[2], -0.25);
        assert_eq!(uniform.shading[3], 0.75);
        assert_eq!(uniform.lighting[0], 0.5);
        assert_eq!(uniform.lighting[2], 0.4);
        assert_eq!(&uniform.matcap_factor_debug[0..3], &[0.6, 0.5, 0.4]);
        assert_eq!(&uniform.rim_color_lighting_mix[0..3], &[0.7, 0.6, 0.5]);
        assert_eq!(uniform.rim_color_lighting_mix[3], 0.25);
        assert_eq!(uniform.rim_params[0], 2.0);
        assert_eq!(uniform.rim_params[1], 0.125);
        assert_eq!(uniform.flags[1], 1);
        assert_eq!(uniform.flags[2], 0);
    }

    #[test]
    fn owner_id_triangles_are_assigned_in_draw_order() {
        let vertex = AshVrmVertex {
            position: [0.0, 0.0, 0.0],
            tex_coord_0: [0.0, 0.0],
            tex_coord_0_dx: [0.0, 0.0],
            tex_coord_0_dy: [0.0, 0.0],
            color_0: [0.0, 0.0, 0.0, 1.0],
            normal: [0.0, 1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            normal_scale: 1.0,
            double_sided: 0.0,
        };
        let primitive = |render_order, phase_order| AshPrimitiveRecord {
            primitive: AshVrmPrimitive {
                node: NodeRef(0),
                mesh_index: 0,
                primitive_index: 0,
                material_name: None,
                material: Some(MaterialRef(0)),
                pass: AshMtoonPass::Base,
                vertices: vec![vertex; 3],
                indices: vec![0, 1, 2],
            },
            source: test_primitive_source(AshMtoonPass::Base),
            draw_order: AshPrimitiveDrawOrder {
                render_order,
                phase_order,
            },
        };
        let mut primitives = vec![primitive(3000, 0), primitive(1000, 99)];

        let owners = assign_ash_owner_id_triangles(&mut primitives, AshSceneOptions::default());

        assert_eq!(
            primitives[1].primitive.vertices[0].color_0,
            ash_owner_id_color(1)
        );
        assert_eq!(
            primitives[0].primitive.vertices[0].color_0,
            ash_owner_id_color(2)
        );
        assert_eq!(primitives[0].primitive.indices, [0, 1, 2]);
        assert_eq!(primitives[1].primitive.indices, [0, 1, 2]);
        assert_eq!(owners[0].id, 1);
        assert_eq!(owners[0].source.phase_order, 99);
        assert_eq!(owners[1].id, 2);
    }

    #[test]
    fn render_surfaces_are_exposed_without_owner_id_diagnostic() {
        let vertex = AshVrmVertex {
            position: [0.0, 0.0, 0.0],
            tex_coord_0: [0.0, 0.0],
            tex_coord_0_dx: [0.0, 0.0],
            tex_coord_0_dy: [0.0, 0.0],
            color_0: [0.0, 0.0, 0.0, 1.0],
            normal: [0.0, 1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            normal_scale: 1.0,
            double_sided: 0.0,
        };
        let record = AshPrimitiveRecord {
            primitive: AshVrmPrimitive {
                node: NodeRef(0),
                mesh_index: 0,
                primitive_index: 0,
                material_name: None,
                material: Some(MaterialRef(0)),
                pass: AshMtoonPass::Base,
                vertices: vec![vertex; 4],
                indices: vec![0, 1, 2, 1, 2, 3],
            },
            source: test_primitive_source(AshMtoonPass::Base),
            draw_order: AshPrimitiveDrawOrder {
                render_order: 2000,
                phase_order: 0,
            },
        };

        let surfaces = ash_render_surfaces(&[record]);

        assert_eq!(
            surfaces,
            vec![
                RenderOwnerSurfaceKey::new("test-material", 0),
                RenderOwnerSurfaceKey::new("test-material", 1),
            ]
        );
    }

    #[test]
    fn render_options_carry_normal_map_diagnostics() {
        let options = AshVrmFramePlanOptions::parse_from([
            "ash-plan",
            "--expression",
            "happy=1.0",
            "--normal-map-mode",
            "view-derivative",
            "--normal-map-scale",
            "0.25",
            "--disable-normal-maps",
            "--disable-outlines",
            "--outline-width-scale",
            "0.5",
            "--diagnostic-render",
            "owner-id",
        ]);

        assert_eq!(options.expressions, vec!["happy=1.0"]);
        assert_eq!(
            parse_expression_args(&options.expressions).unwrap(),
            vec![("happy".to_owned(), 1.0)]
        );
        let render_options = options.render_options();

        assert_eq!(
            render_options.normal_map_mode,
            AshNormalMapMode::ViewDerivative
        );
        assert_eq!(
            GltfNormalMapMode::from(render_options.normal_map_mode),
            GltfNormalMapMode::ViewDerivative
        );
        assert_eq!(render_options.normal_map_scale, 0.25);
        assert!(render_options.disable_normal_maps);
        assert!(render_options.disable_outlines);
        assert_eq!(render_options.outline_width_scale, 0.5);
        assert_eq!(
            render_options.diagnostic_render,
            AshDiagnosticRender::OwnerId
        );
    }

    #[test]
    fn source_mtoon_shader_matches_rust_binding_contract() {
        let vertex_shader = include_str!("../shaders/mtoon_base.vert.glsl");
        let fragment_shader = include_str!("../shaders/mtoon_base.frag.glsl");

        for (slot, expected_name) in [
            (MtoonTextureSlot::Main, "main_texture"),
            (MtoonTextureSlot::ShadeMultiply, "shade_multiply_texture"),
            (MtoonTextureSlot::ShadingShift, "shading_shift_texture"),
            (MtoonTextureSlot::Normal, "normal_texture"),
            (MtoonTextureSlot::Matcap, "matcap_texture"),
            (MtoonTextureSlot::RimMultiply, "rim_multiply_texture"),
            (MtoonTextureSlot::OutlineWidth, "outline_width_texture"),
            (
                MtoonTextureSlot::UvAnimationMask,
                "uv_animation_mask_texture",
            ),
        ] {
            let declaration = format!(
                "layout(set = 0, binding = {}) uniform sampler2D {expected_name};",
                ash_mtoon_texture_binding(slot)
            );
            assert!(fragment_shader.contains(&declaration));
        }
        for (slot, expected_name) in [
            (GltfMaterialTextureSlot::Emissive, "emissive_texture"),
            (GltfMaterialTextureSlot::Occlusion, "occlusion_texture"),
        ] {
            let declaration = format!(
                "layout(set = 0, binding = {}) uniform sampler2D {expected_name};",
                ash_material_texture_binding(slot)
            );
            assert!(fragment_shader.contains(&declaration));
        }

        assert!(fragment_shader.contains("layout(set = 0, binding = 0, std140)"));
        assert!(fragment_shader.contains("layout(set = 0, binding = 9, std140)"));
        assert!(fragment_shader.contains("layout(set = 0, binding = 10, std140)"));
        assert!(fragment_shader.contains("layout(set = 0, binding = 11, std140)"));
        assert!(fragment_shader.contains("textureGrad(source, uv, dx, dy)"));
        assert!(fragment_shader.contains("emissive_texture,\n        emissive_uv"));
        assert!(fragment_shader.contains("occlusion_texture,\n            occlusion_uv"));
        assert!(fragment_shader.contains("transform_uv_gradient(animated_uv_dx"));
        assert!(fragment_shader.contains("flip_v_gradient(base_uv_dx)"));
        assert!(fragment_shader.contains("srgb_to_linear_color(raw_main_texel.rgb)"));
        assert!(fragment_shader.contains("base_sample_uv = vec2(base_uv.x, 1.0 - base_uv.y)"));
        assert!(fragment_shader.contains("material_extra.flags2.z > 0.5"));
        assert!(fragment_shader.contains("material_extra.flags2.w > 4.5"));
        assert!(fragment_shader.contains("owner_id_output_color(in_color_0.rgb"));
        assert!(fragment_shader.contains("material_extra.flags2.w < -0.5"));
        assert!(fragment_shader.contains("alpha_mode < 2u ? 1.0 : alpha"));
        assert!(fragment_shader.contains("mtoon.flags.z == 1u"));
        assert!(fragment_shader.contains("transform_uv(animated_uv"));
        assert!(fragment_shader.contains("centered.x * c + centered.y * s"));
        assert!(fragment_shader.contains("-centered.x * s + centered.y * c"));
        assert!(fragment_shader.contains("pbr_direct("));
        assert!(fragment_shader.contains("output_color(color"));
        assert!(fragment_shader.contains("scene.light_color.rgb * scene.light_dir.w;"));
        assert!(!fragment_shader.contains("material_extra.pbr_params.w"));
        assert!(fragment_shader.contains("matcap_uv_from_view(normal)"));
        assert!(fragment_shader.contains("gl_FrontFacing"));
        assert!(fragment_shader.contains("in_normal_scale == 0.0"));
        assert!(fragment_shader.contains("front_facing || in_double_sided < 0.5"));
        assert!(fragment_shader.contains("material_extra.flags2.y > 0.5"));
        assert!(fragment_shader.contains("dFdx(derivative_position)"));
        assert!(fragment_shader.contains("scene.world_from_view"));
        assert!(fragment_shader.contains("material_extra.flags.x > 0.5"));
        assert!(fragment_shader.contains("material_extra.flags2.x > 0.5"));
        assert!(vertex_shader.contains("layout(set = 0, binding = 0, std140)"));
        assert!(vertex_shader.contains("layout(set = 0, binding = 9, std140)"));
        assert!(vertex_shader.contains("layout(location = 2) in vec2 in_tex_coord_0_dx;"));
        assert!(vertex_shader.contains("layout(location = 3) in vec2 in_tex_coord_0_dy;"));
        assert!(vertex_shader.contains("layout(location = 5) in vec3 in_normal;"));
        assert!(vertex_shader.contains("layout(location = 6) in vec4 in_tangent;"));
        assert!(vertex_shader.contains("layout(location = 7) in float in_normal_scale;"));
        assert!(vertex_shader.contains("layout(location = 8) in float in_double_sided;"));
        assert!(vertex_shader.contains("gl_PointSize = 1.0;"));
        assert!(vertex_shader.contains("mtoon.flags.z == 1u"));
        assert!(vertex_shader.contains("gl_Position.z += 0.000001 * gl_Position.w"));
    }

    #[test]
    fn renderer_frame_builds_buffers_and_sorted_draw_calls() {
        let plan = AshVrmFramePlan {
            primitives: vec![AshVrmPrimitive {
                node: NodeRef(0),
                mesh_index: 0,
                primitive_index: 0,
                material_name: None,
                material: Some(MaterialRef(0)),
                pass: AshMtoonPass::Base,
                vertices: vec![AshVrmVertex {
                    position: [0.0, 0.0, 0.0],
                    tex_coord_0: [0.0, 0.0],
                    tex_coord_0_dx: [0.0, 0.0],
                    tex_coord_0_dy: [0.0, 0.0],
                    color_0: [1.0, 1.0, 1.0, 1.0],
                    normal: [0.0, 0.0, 1.0],
                    tangent: [1.0, 0.0, 0.0, 1.0],
                    normal_scale: 1.0,
                    double_sided: 0.0,
                }],
                indices: vec![0],
            }],
            materials: vec![AshMaterialRecord {
                material: MaterialRef(0),
                base_color_factor: [1.0, 1.0, 1.0, 1.0],
                base_color_texture_upload: None,
            }],
            texture_uploads: Vec::new(),
            mtoon_pipelines: vec![AshMtoonPipelinePlan {
                material: MaterialRef(0),
                name: Some("mat".to_owned()),
                key: AshPipelineKey {
                    pass: AshMtoonPass::Base,
                    render_order: 2000,
                    phase_order: 2000,
                    topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                    cull_mode: vk::CullModeFlags::BACK,
                    front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                    depth_test_enable: true,
                    depth_write_enable: true,
                    depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                    blend_enable: false,
                },
                descriptor_bindings: descriptor_bindings(&[], GltfMaterialTextureSlots::default()),
                uniform: MtoonGpuUniform::zeroed(),
                uv_uniform: AshMaterialUvUniform::default(),
                render_extra_uniform: AshMaterialExtraUniform::default(),
                uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
                alpha_cutoff: 0.5,
                outline_width: 0.0,
                base_color_factor: [1.0, 1.0, 1.0, 1.0],
                emissive_color: [0.0, 0.0, 0.0],
            }],
            scene_uniform: AshSceneUniform::default(),
            scene_options: AshSceneOptions::default(),
            diagnostic_owner_ids: Vec::new(),
            render_surfaces: Vec::new(),
        };
        let renderer_frame = ash_renderer_frame_from_plan(&plan);
        assert_eq!(renderer_frame.buffers.len(), 3);
        assert_eq!(renderer_frame.uniforms.len(), 4);
        assert_eq!(renderer_frame.pipelines.len(), 1);
        assert_eq!(
            renderer_frame.uniforms[0].bytes.len(),
            MTOON_GPU_UNIFORM_SIZE
        );
        assert_eq!(
            renderer_frame.uniforms[0].scope,
            AshUniformScope::material(MaterialRef(0), 0)
        );
        assert_eq!(
            renderer_frame.uniforms[0].scope.material_ref(),
            Some(MaterialRef(0))
        );
        assert_eq!(
            renderer_frame.uniforms[0].scope.pipeline_plan_index(),
            Some(0)
        );
        assert_eq!(
            renderer_frame.uniforms[0].binding,
            ash_mtoon_uniform_binding()
        );
        assert_eq!(
            renderer_frame.uniforms[1].bytes.len(),
            std::mem::size_of::<AshMaterialUvUniform>()
        );
        assert_eq!(
            renderer_frame.uniforms[1].scope,
            AshUniformScope::material_uv(MaterialRef(0), 0)
        );
        assert_eq!(
            renderer_frame.uniforms[1].binding,
            ash_mtoon_uv_uniform_binding()
        );
        assert_eq!(
            renderer_frame.uniforms[2].bytes.len(),
            std::mem::size_of::<AshMaterialExtraUniform>()
        );
        assert_eq!(
            renderer_frame.uniforms[2].scope,
            AshUniformScope::material_extra(MaterialRef(0), 0)
        );
        assert_eq!(
            renderer_frame.uniforms[2].binding,
            ash_mtoon_render_extra_binding()
        );
        assert_eq!(
            renderer_frame.uniforms[3].bytes.len(),
            std::mem::size_of::<AshSceneUniform>()
        );
        assert_eq!(renderer_frame.uniforms[3].scope, AshUniformScope::Scene);
        assert_eq!(renderer_frame.uniforms[3].scope.material_ref(), None);
        assert_eq!(renderer_frame.uniforms[3].scope.pipeline_plan_index(), None);
        assert_eq!(
            renderer_frame.uniforms[3].binding,
            ash_mtoon_scene_binding()
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[0].uniform_upload_index,
            Some(0)
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[11].uniform_upload_index,
            Some(3)
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[12].uniform_upload_index,
            Some(1)
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[13].uniform_upload_index,
            Some(2)
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[14].binding,
            ash_owner_sample_override_binding()
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[14].descriptor_type,
            vk::DescriptorType::STORAGE_BUFFER
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[14].buffer_upload_index,
            Some(0)
        );
        assert_eq!(
            renderer_frame.pipelines[0].vertex_attributes,
            ash_vrm_vertex_attributes()
        );
        assert_eq!(
            renderer_frame.pipelines[0].depth_format,
            Some(ash_reference_depth_format())
        );
        assert_eq!(renderer_frame.pipelines[0].vertex_attributes.len(), 9);
        assert_eq!(
            renderer_frame.buffers[1].stride,
            std::mem::size_of::<AshVrmVertex>() as u32
        );
        assert_eq!(
            renderer_frame.buffers[1].usage,
            vk::BufferUsageFlags::VERTEX_BUFFER
        );
        assert_eq!(
            renderer_frame.buffers[0].role,
            AshBufferRole::OwnerSampleOverride
        );
        assert_eq!(
            renderer_frame.buffers[0].usage,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST
        );
        assert_eq!(renderer_frame.buffers[0].count, 1);
        assert_eq!(renderer_frame.draw_calls[0].pipeline_plan_index, Some(0));
        assert_eq!(renderer_frame.draw_calls[0].descriptor_set_index, Some(0));

        let drawable = ash_drawable_frame_from_renderer_frame(
            &renderer_frame,
            vk::Extent2D {
                width: 128,
                height: 64,
            },
        );
        assert_eq!(drawable.render_pass.render_area.extent.width, 128);
        assert_eq!(drawable.render_pass.render_area.extent.height, 64);
        assert_eq!(
            drawable.render_pass.color_format,
            vk::Format::R8G8B8A8_UNORM
        );
        assert_eq!(
            drawable.render_pass.depth_format,
            Some(ash_reference_depth_format())
        );
        assert!(drawable.skipped_draws.is_empty());
        assert_eq!(
            drawable.commands,
            vec![
                AshCommandPlan::BindGraphicsPipeline { pipeline_index: 0 },
                AshCommandPlan::BindDescriptorSet {
                    pipeline_index: 0,
                    descriptor_set_index: 0,
                },
                AshCommandPlan::BindVertexBuffer {
                    buffer_index: 1,
                    binding: 0,
                    offset: 0,
                },
                AshCommandPlan::BindIndexBuffer {
                    buffer_index: 2,
                    offset: 0,
                    index_type: vk::IndexType::UINT32,
                },
                AshCommandPlan::DrawIndexed {
                    primitive_index: 0,
                    index_count: 1,
                    instance_count: 1,
                    first_index: 0,
                    vertex_offset: 0,
                    first_instance: 0,
                },
            ]
        );
    }

    #[test]
    fn renderer_frame_binds_owner_sample_override_storage_buffer() {
        let surface = RenderOwnerSurfaceKey::new("mat", 7);
        let selection = RenderOwnerSampleSelectionPlan {
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
        let mut plan = AshVrmFramePlan {
            primitives: Vec::new(),
            materials: Vec::new(),
            texture_uploads: Vec::new(),
            mtoon_pipelines: vec![AshMtoonPipelinePlan {
                material: MaterialRef(0),
                name: Some("mat".to_owned()),
                key: AshPipelineKey {
                    pass: AshMtoonPass::Base,
                    render_order: 2000,
                    phase_order: 2000,
                    topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                    cull_mode: vk::CullModeFlags::BACK,
                    front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                    depth_test_enable: true,
                    depth_write_enable: true,
                    depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                    blend_enable: false,
                },
                descriptor_bindings: descriptor_bindings(&[], GltfMaterialTextureSlots::default()),
                uniform: MtoonGpuUniform::zeroed(),
                uv_uniform: AshMaterialUvUniform::default(),
                render_extra_uniform: AshMaterialExtraUniform::default(),
                uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
                alpha_cutoff: 0.5,
                outline_width: 0.0,
                base_color_factor: [1.0, 1.0, 1.0, 1.0],
                emissive_color: [0.0, 0.0, 0.0],
            }],
            scene_uniform: AshSceneUniform::default(),
            scene_options: AshSceneOptions::default(),
            diagnostic_owner_ids: Vec::new(),
            render_surfaces: vec![surface],
        };
        plan.primitives.push(AshVrmPrimitive {
            node: NodeRef(0),
            mesh_index: 3,
            primitive_index: 4,
            material_name: None,
            material: Some(MaterialRef(0)),
            pass: AshMtoonPass::Base,
            vertices: Vec::new(),
            indices: Vec::new(),
        });

        let renderer_frame =
            ash_renderer_frame_from_plan_with_owner_sample_selection(&plan, Some(&selection))
                .unwrap();
        let binding = renderer_frame
            .descriptor_sets
            .first()
            .and_then(|set| {
                set.bindings
                    .iter()
                    .find(|binding| binding.binding == ash_owner_sample_override_binding())
            })
            .unwrap();
        let buffer_index = binding.buffer_upload_index.unwrap();
        let buffer = &renderer_frame.buffers[buffer_index];
        let record = bytemuck::pod_read_unaligned::<AshOwnerSampleOverrideRecord>(
            &buffer.bytes[..std::mem::size_of::<AshOwnerSampleOverrideRecord>()],
        );

        assert_eq!(binding.descriptor_type, vk::DescriptorType::STORAGE_BUFFER);
        assert_eq!(buffer.role, AshBufferRole::OwnerSampleOverride);
        assert_eq!(buffer.count, 1);
        assert_eq!(record.pixel, [12, 34]);
        assert_eq!(record.sample, [0.25, 0.75]);
        assert_eq!(record.replacement_rgba[2], 1.0);
        assert_eq!(record.relation_to_expected, 1);
        assert_eq!(record.geometry_flags, 1);
        assert_eq!(record.geometry_ids, [2, 3, 4, 7]);
        assert_eq!(record.geometry_uvs, [0.1, 0.2, 0.7, 0.8]);
    }

    #[test]
    fn renderer_frame_adds_owner_sample_resolve_draw_from_geometry() {
        let surface = RenderOwnerSurfaceKey::new("mat", 0);
        let geometry = vrm_adapter::RenderOwnerSampleGeometry {
            node: 2,
            mesh: 99,
            primitive: 4,
            triangle: 0,
            indices: [0, 1, 2],
            barycentric: [0.2, 0.3, 0.5],
            raw_uv: [0.42, 0.43],
            base_uv: [0.42, 0.43],
            depth: 0.97,
            pass: RenderOwnerSamplePass::Base,
        };
        let selection = RenderOwnerSampleSelectionPlan {
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
                    sample_geometry: Some(geometry),
                }],
            }],
            unmatched_entries: Vec::new(),
        };
        let source_vertex = |position, tex_coord_0, color| AshVrmVertex {
            position,
            tex_coord_0,
            tex_coord_0_dx: [0.0, 0.0],
            tex_coord_0_dy: [0.0, 0.0],
            color_0: color,
            normal: [0.0, 0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            normal_scale: 1.0,
            double_sided: 0.0,
        };
        let scene_options = AshSceneOptions {
            aspect_ratio: 2.0,
            screen_projection_size: ScreenProjectionSize {
                width: 256.0,
                height: 128.0,
            },
            camera_z: 3.0,
            ..Default::default()
        };
        let plan = AshVrmFramePlan {
            primitives: vec![AshVrmPrimitive {
                node: NodeRef(2),
                mesh_index: 3,
                primitive_index: 4,
                material_name: Some("mat".to_owned()),
                material: Some(MaterialRef(0)),
                pass: AshMtoonPass::Base,
                vertices: vec![
                    source_vertex([0.0, 0.0, 0.0], [0.0, 0.0], [1.0, 0.0, 0.0, 1.0]),
                    source_vertex([1.0, 0.0, 0.0], [1.0, 0.0], [0.0, 1.0, 0.0, 1.0]),
                    source_vertex([0.0, 1.0, 0.0], [0.0, 1.0], [0.0, 0.0, 1.0, 1.0]),
                ],
                indices: vec![0, 1, 2],
            }],
            materials: Vec::new(),
            texture_uploads: Vec::new(),
            mtoon_pipelines: vec![AshMtoonPipelinePlan {
                material: MaterialRef(0),
                name: Some("mat".to_owned()),
                key: AshPipelineKey {
                    pass: AshMtoonPass::Base,
                    render_order: 2000,
                    phase_order: 2000,
                    topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                    cull_mode: vk::CullModeFlags::BACK,
                    front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                    depth_test_enable: true,
                    depth_write_enable: true,
                    depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                    blend_enable: false,
                },
                descriptor_bindings: descriptor_bindings(&[], GltfMaterialTextureSlots::default()),
                uniform: MtoonGpuUniform::zeroed(),
                uv_uniform: AshMaterialUvUniform::default(),
                render_extra_uniform: AshMaterialExtraUniform::default(),
                uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
                alpha_cutoff: 0.5,
                outline_width: 0.0,
                base_color_factor: [1.0, 1.0, 1.0, 1.0],
                emissive_color: [0.0, 0.0, 0.0],
            }],
            scene_uniform: AshSceneUniform::from_scene_options(scene_options),
            scene_options,
            diagnostic_owner_ids: Vec::new(),
            render_surfaces: vec![surface],
        };

        let renderer_frame =
            ash_renderer_frame_from_plan_with_owner_sample_selection(&plan, Some(&selection))
                .unwrap();

        assert_eq!(renderer_frame.pipelines.len(), 2);
        assert_eq!(
            renderer_frame.pipelines[1].key.topology,
            vk::PrimitiveTopology::TRIANGLE_LIST
        );
        assert_eq!(
            renderer_frame.pipelines[1].key.depth_compare_op,
            vk::CompareOp::ALWAYS
        );
        assert!(!renderer_frame.pipelines[1].key.depth_write_enable);
        assert_eq!(renderer_frame.draw_calls.len(), 2);
        assert_eq!(renderer_frame.draw_calls[1].index_count, 6);
        let vertex_buffer =
            &renderer_frame.buffers[renderer_frame.draw_calls[1].vertex_buffer_index];
        let vertex = bytemuck::pod_read_unaligned::<AshVrmVertex>(
            &vertex_buffer.bytes[..std::mem::size_of::<AshVrmVertex>()],
        );
        assert_eq!(vertex.tex_coord_0, [0.42, 0.43]);
        assert!(vertex.tex_coord_0_dx != [0.0, 0.0] || vertex.tex_coord_0_dy != [0.0, 0.0]);
        assert_eq!(vertex.color_0, [0.2, 0.3, 0.5, 1.0]);
        assert_eq!(
            vertex.position,
            ash_owner_sample_pixel_quad_world([12, 34], scene_options).unwrap()[0]
        );
        let clip = scene_options.projection()
            * scene_options.view()
            * Vec4::new(
                vertex.position[0],
                vertex.position[1],
                vertex.position[2],
                1.0,
            );
        let ndc = clip.truncate() / clip.w;
        let size = scene_options.sanitized_screen_projection_size();
        assert!(((ndc.x + 1.0) * 0.5 * size.width - 12.0).abs() < 0.001);
        assert!(((ndc.y + 1.0) * 0.5 * size.height - 34.0).abs() < 0.001);
    }

    #[test]
    fn renderer_frame_routes_outline_primitives_to_outline_pipeline() {
        let vertex = AshVrmVertex {
            position: [0.0, 0.0, 0.0],
            tex_coord_0: [0.0, 0.0],
            tex_coord_0_dx: [0.0, 0.0],
            tex_coord_0_dy: [0.0, 0.0],
            color_0: [1.0, 1.0, 1.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            normal_scale: 1.0,
            double_sided: 0.0,
        };
        let primitive = |pass| AshVrmPrimitive {
            node: NodeRef(0),
            mesh_index: 0,
            primitive_index: 0,
            material_name: None,
            material: Some(MaterialRef(0)),
            pass,
            vertices: vec![vertex],
            indices: vec![0],
        };
        let pipeline = |pass, render_order, phase_order| AshMtoonPipelinePlan {
            material: MaterialRef(0),
            name: Some(format!("{pass:?}")),
            key: AshPipelineKey {
                pass,
                render_order,
                phase_order,
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                cull_mode: match pass {
                    AshMtoonPass::Base => vk::CullModeFlags::BACK,
                    AshMtoonPass::Outline => vk::CullModeFlags::FRONT,
                },
                front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                depth_test_enable: true,
                depth_write_enable: true,
                depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                blend_enable: false,
            },
            descriptor_bindings: descriptor_bindings(&[], GltfMaterialTextureSlots::default()),
            uniform: MtoonGpuUniform::zeroed(),
            uv_uniform: AshMaterialUvUniform::default(),
            render_extra_uniform: AshMaterialExtraUniform::default(),
            uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
            alpha_cutoff: 0.5,
            outline_width: 0.01,
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            emissive_color: [0.0, 0.0, 0.0],
        };
        let plan = AshVrmFramePlan {
            primitives: vec![
                primitive(AshMtoonPass::Base),
                primitive(AshMtoonPass::Outline),
            ],
            materials: Vec::new(),
            texture_uploads: Vec::new(),
            mtoon_pipelines: vec![
                pipeline(AshMtoonPass::Base, 2000, 2000),
                pipeline(AshMtoonPass::Outline, 2001, 2001),
            ],
            scene_uniform: AshSceneUniform::default(),
            scene_options: AshSceneOptions::default(),
            diagnostic_owner_ids: Vec::new(),
            render_surfaces: Vec::new(),
        };

        let renderer_frame = ash_renderer_frame_from_plan(&plan);

        assert_eq!(renderer_frame.draw_calls.len(), 2);
        assert_eq!(renderer_frame.pipelines.len(), 2);
        assert_eq!(renderer_frame.draw_calls[0].pipeline_plan_index, Some(0));
        assert_eq!(renderer_frame.draw_calls[0].descriptor_set_index, Some(0));
        assert_eq!(renderer_frame.draw_calls[1].pipeline_plan_index, Some(1));
        assert_eq!(renderer_frame.draw_calls[1].descriptor_set_index, Some(1));
        assert_eq!(
            renderer_frame.pipelines[1].key.cull_mode,
            vk::CullModeFlags::FRONT
        );
    }

    #[test]
    fn drawable_frame_reports_skipped_draws_without_device_handles() {
        let mut frame = AshRendererFrame::default();
        frame.draw_calls.push(AshDrawCallPlan {
            primitive_index: 7,
            material: Some(MaterialRef(0)),
            pipeline_plan_index: Some(99),
            descriptor_set_index: Some(0),
            vertex_buffer_index: 0,
            index_buffer_index: 1,
            index_count: 3,
            render_order: 2000,
            phase_order: 2000,
        });

        let drawable = ash_drawable_frame_from_renderer_frame(
            &frame,
            vk::Extent2D {
                width: 16,
                height: 16,
            },
        );

        assert!(drawable.commands.is_empty());
        assert_eq!(
            drawable.skipped_draws,
            vec![AshSkippedDraw {
                primitive_index: 7,
                reason: AshSkippedDrawReason::MissingPipeline,
            }]
        );
    }

    #[test]
    fn renderer_frame_preserves_source_order_inside_same_render_order() {
        let vertex = AshVrmVertex {
            position: [0.0, 0.0, 0.0],
            tex_coord_0: [0.0, 0.0],
            tex_coord_0_dx: [0.0, 0.0],
            tex_coord_0_dy: [0.0, 0.0],
            color_0: [1.0, 1.0, 1.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            normal_scale: 1.0,
            double_sided: 0.0,
        };
        let primitive = |material| AshVrmPrimitive {
            node: NodeRef(0),
            mesh_index: 0,
            primitive_index: 0,
            material_name: None,
            material: Some(MaterialRef(material)),
            pass: AshMtoonPass::Base,
            vertices: vec![vertex],
            indices: vec![0],
        };
        let pipeline = |material, phase_order| AshMtoonPipelinePlan {
            material: MaterialRef(material),
            name: Some(format!("mat-{material}")),
            key: AshPipelineKey {
                pass: AshMtoonPass::Base,
                render_order: 2000,
                phase_order,
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                cull_mode: vk::CullModeFlags::BACK,
                front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                depth_test_enable: true,
                depth_write_enable: true,
                depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                blend_enable: false,
            },
            descriptor_bindings: descriptor_bindings(&[], GltfMaterialTextureSlots::default()),
            uniform: MtoonGpuUniform::zeroed(),
            uv_uniform: AshMaterialUvUniform::default(),
            render_extra_uniform: AshMaterialExtraUniform::default(),
            uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
            alpha_cutoff: 0.5,
            outline_width: 0.0,
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            emissive_color: [0.0, 0.0, 0.0],
        };
        let plan = AshVrmFramePlan {
            primitives: vec![primitive(0), primitive(1)],
            materials: Vec::new(),
            texture_uploads: Vec::new(),
            mtoon_pipelines: vec![pipeline(0, 19), pipeline(1, 0)],
            scene_uniform: AshSceneUniform::default(),
            scene_options: AshSceneOptions::default(),
            diagnostic_owner_ids: Vec::new(),
            render_surfaces: Vec::new(),
        };

        let renderer_frame = ash_renderer_frame_from_plan(&plan);

        assert_eq!(renderer_frame.draw_calls[0].primitive_index, 0);
        assert_eq!(renderer_frame.draw_calls[1].primitive_index, 1);
        assert_eq!(renderer_frame.draw_calls[0].phase_order, 19);
        assert_eq!(renderer_frame.draw_calls[1].phase_order, 0);
    }

    #[test]
    fn renderer_frame_resolves_texture_descriptor_uploads() {
        let plan = AshVrmFramePlan {
            primitives: Vec::new(),
            materials: Vec::new(),
            texture_uploads: vec![AshTextureUpload {
                texture: Some(TextureRef(7)),
                color_space: GltfMaterialTextureColorSpace::Srgb,
                format: vk::Format::R8G8B8A8_SRGB,
                extent: vk::Extent3D {
                    width: 1,
                    height: 1,
                    depth: 1,
                },
                rgba: vec![255, 255, 255, 255],
            }],
            mtoon_pipelines: vec![AshMtoonPipelinePlan {
                material: MaterialRef(0),
                name: Some("textured".to_owned()),
                key: AshPipelineKey {
                    pass: AshMtoonPass::Base,
                    render_order: 2000,
                    phase_order: 2000,
                    topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                    cull_mode: vk::CullModeFlags::BACK,
                    front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                    depth_test_enable: true,
                    depth_write_enable: true,
                    depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                    blend_enable: false,
                },
                descriptor_bindings: vec![
                    AshDescriptorBindingPlan {
                        binding: 0,
                        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
                        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                        texture: None,
                        color_space: GltfMaterialTextureColorSpace::Linear,
                        sampler: None,
                    },
                    AshDescriptorBindingPlan {
                        binding: 1,
                        descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                        stage_flags: vk::ShaderStageFlags::FRAGMENT,
                        texture: Some(TextureRef(7)),
                        color_space: GltfMaterialTextureColorSpace::Srgb,
                        sampler: Some(AshSamplerPlan {
                            mag_filter: vk::Filter::LINEAR,
                            min_filter: vk::Filter::LINEAR,
                            mipmap_mode: vk::SamplerMipmapMode::LINEAR,
                            address_mode_u: vk::SamplerAddressMode::REPEAT,
                            address_mode_v: vk::SamplerAddressMode::REPEAT,
                            min_lod: 0.0,
                            max_lod: 32.0,
                            normal_map_decode: false,
                        }),
                    },
                ],
                uniform: MtoonGpuUniform::zeroed(),
                uv_uniform: AshMaterialUvUniform::default(),
                render_extra_uniform: AshMaterialExtraUniform::default(),
                uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
                alpha_cutoff: 0.5,
                outline_width: 0.0,
                base_color_factor: [1.0, 1.0, 1.0, 1.0],
                emissive_color: [0.0, 0.0, 0.0],
            }],
            scene_uniform: AshSceneUniform::default(),
            scene_options: AshSceneOptions::default(),
            diagnostic_owner_ids: Vec::new(),
            render_surfaces: Vec::new(),
        };
        let renderer_frame = ash_renderer_frame_from_plan(&plan);
        assert_eq!(renderer_frame.textures.len(), 1);
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[1].texture_upload_index,
            Some(0)
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[1].descriptor_type,
            vk::DescriptorType::COMBINED_IMAGE_SAMPLER
        );
    }
}
