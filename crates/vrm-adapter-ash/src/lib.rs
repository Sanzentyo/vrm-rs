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
    ffi::CStr,
    fmt,
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
    ZeroToOneDepth, apply_vrma_animation_frame_with_look_at, mtoon_gpu_sampler_binding_number,
    mtoon_gpu_texture_binding_number, mtoon_renderer_material_plans, project_triangle_to_screen,
    renderer_material_pipeline_plan,
};
use vrm_core::{Feature, MaterialRef, MtoonAlphaMode, NodeRef, TextureRef, VrmAnimation};
use vrm_io::{
    CpuRgba8Image, GltfAlphaMode, GltfExpressionRenderEffects, GltfMagFilter,
    GltfMaterialRenderExtraOptions, GltfMaterialRenderExtraUniformPlan, GltfMaterialShadingOptions,
    GltfMaterialTextureBinding, GltfMaterialTextureColorSpace, GltfMaterialTextureFallback,
    GltfMaterialTextureSlot, GltfMaterialTextureSlots, GltfMaterialUvUniformPlan, GltfMinFilter,
    GltfNodeRest, GltfNormalMapMode, GltfOutlineScale, GltfOutlineVertexSettings,
    GltfPrimitiveData, GltfSamplerData, GltfTextureData, GltfWrapMode, LoadedVrm,
    Rgba8SamplingOrigin, RgbaMipLevel, generate_tangents, load_vrm_from_path,
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
    /// Descriptor binding model used by emitted ash material plans.
    #[arg(long, value_enum, default_value_t = AshDescriptorBindingModel::SeparateImageSampler)]
    pub descriptor_binding_model: AshDescriptorBindingModel,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, ValueEnum)]
pub enum AshDescriptorBindingModel {
    /// Vulkan combined image sampler bindings, matching the legacy GLSL handoff shaders.
    CombinedImageSampler,
    /// Separate sampled-image and sampler bindings, matching WGSL/WebGPU and naga SPIR-V output.
    #[default]
    SeparateImageSampler,
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
            clip_space_policy: AshClipSpacePolicy::default(),
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
            descriptor_binding_model: self.descriptor_binding_model,
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
    pub descriptor_binding_model: AshDescriptorBindingModel,
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
            descriptor_binding_model: AshDescriptorBindingModel::SeparateImageSampler,
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

pub const ASH_MTOON_WGSL_SOURCE_PATH: &str = "crates/vrm-adapter-ash/shaders/mtoon_base.wgsl";
pub const ASH_MTOON_WGSL_PRELUDE_PATH: &str = "crates/vrm-adapter/src/mtoon_reference.wgsl";
pub const ASH_MTOON_WGSL_VERTEX_ENTRY: &str = "vs_main";
pub const ASH_MTOON_WGSL_FRAGMENT_ENTRY: &str = "fs_main";
pub const ASH_MTOON_WGSL_VERTEX_SPIRV_FILE: &str = "mtoon_base.wgsl.vert.spv";
pub const ASH_MTOON_WGSL_FRAGMENT_SPIRV_FILE: &str = "mtoon_base.wgsl.frag.spv";
pub const ASH_MTOON_WGSL_DEFAULT_SPIRV_DIR: &str = "target/ash-mtoon-wgsl-base-shaders";
pub const ASH_MTOON_WGSL_DEFAULT_VERTEX_SPIRV_PATH: &str =
    "target/ash-mtoon-wgsl-base-shaders/mtoon_base.wgsl.vert.spv";
pub const ASH_MTOON_WGSL_DEFAULT_FRAGMENT_SPIRV_PATH: &str =
    "target/ash-mtoon-wgsl-base-shaders/mtoon_base.wgsl.frag.spv";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AshClipSpacePolicy {
    #[default]
    CpuVulkanZeroToOneYDown,
    NagaVulkanZeroToOneYDown,
}

impl AshClipSpacePolicy {
    pub const fn projection_y_sign(self) -> f32 {
        match self {
            Self::CpuVulkanZeroToOneYDown => -1.0,
            Self::NagaVulkanZeroToOneYDown => 1.0,
        }
    }

    pub const fn spirv_coordinate_adjustment(self) -> AshSpirvCoordinateAdjustment {
        match self {
            Self::CpuVulkanZeroToOneYDown => AshSpirvCoordinateAdjustment::Disabled,
            Self::NagaVulkanZeroToOneYDown => AshSpirvCoordinateAdjustment::NagaWriter,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AshSpirvCoordinateAdjustment {
    Disabled,
    NagaWriter,
}

impl AshSpirvCoordinateAdjustment {
    pub const fn adjust_coordinate_space(self) -> bool {
        match self {
            Self::Disabled => false,
            Self::NagaWriter => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshWgslResourceKind {
    UniformBuffer,
    SampledImage,
    Sampler,
    StorageBuffer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshWgslResourceBinding {
    pub name: &'static str,
    pub group: u32,
    pub binding: u32,
    pub kind: AshWgslResourceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshMtoonWgslShaderAbi {
    pub prelude_path: &'static str,
    pub source_path: &'static str,
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub vertex_spirv_file: &'static str,
    pub fragment_spirv_file: &'static str,
    pub clip_space_policy: AshClipSpacePolicy,
    pub spirv_coordinate_adjustment: AshSpirvCoordinateAdjustment,
    pub descriptor_binding_model: AshDescriptorBindingModel,
}

impl Default for AshMtoonWgslShaderAbi {
    fn default() -> Self {
        Self {
            prelude_path: ASH_MTOON_WGSL_PRELUDE_PATH,
            source_path: ASH_MTOON_WGSL_SOURCE_PATH,
            vertex_entry: ASH_MTOON_WGSL_VERTEX_ENTRY,
            fragment_entry: ASH_MTOON_WGSL_FRAGMENT_ENTRY,
            vertex_spirv_file: ASH_MTOON_WGSL_VERTEX_SPIRV_FILE,
            fragment_spirv_file: ASH_MTOON_WGSL_FRAGMENT_SPIRV_FILE,
            clip_space_policy: AshClipSpacePolicy::CpuVulkanZeroToOneYDown,
            spirv_coordinate_adjustment: AshSpirvCoordinateAdjustment::Disabled,
            descriptor_binding_model: AshDescriptorBindingModel::SeparateImageSampler,
        }
    }
}

impl AshMtoonWgslShaderAbi {
    pub fn vertex_spirv_path(self, dir: impl AsRef<std::path::Path>) -> PathBuf {
        dir.as_ref().join(self.vertex_spirv_file)
    }

    pub fn fragment_spirv_path(self, dir: impl AsRef<std::path::Path>) -> PathBuf {
        dir.as_ref().join(self.fragment_spirv_file)
    }

    pub fn default_vertex_spirv_path(self) -> PathBuf {
        self.vertex_spirv_path(ASH_MTOON_WGSL_DEFAULT_SPIRV_DIR)
    }

    pub fn default_fragment_spirv_path(self) -> PathBuf {
        self.fragment_spirv_path(ASH_MTOON_WGSL_DEFAULT_SPIRV_DIR)
    }
}

pub const ASH_MTOON_WGSL_RESOURCE_BINDINGS: [AshWgslResourceBinding; 24] = [
    AshWgslResourceBinding {
        name: "mtoon",
        group: 0,
        binding: ash_mtoon_uniform_binding(),
        kind: AshWgslResourceKind::UniformBuffer,
    },
    AshWgslResourceBinding {
        name: "main_texture",
        group: 0,
        binding: ash_mtoon_sampled_image_binding(MtoonTextureSlot::Main),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "main_sampler",
        group: 0,
        binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Main),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "shade_multiply_texture",
        group: 0,
        binding: ash_mtoon_sampled_image_binding(MtoonTextureSlot::ShadeMultiply),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "shade_multiply_sampler",
        group: 0,
        binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::ShadeMultiply),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "shading_shift_texture",
        group: 0,
        binding: ash_mtoon_sampled_image_binding(MtoonTextureSlot::ShadingShift),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "shading_shift_sampler",
        group: 0,
        binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::ShadingShift),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "normal_texture",
        group: 0,
        binding: ash_mtoon_sampled_image_binding(MtoonTextureSlot::Normal),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "normal_sampler",
        group: 0,
        binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Normal),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "matcap_texture",
        group: 0,
        binding: ash_mtoon_sampled_image_binding(MtoonTextureSlot::Matcap),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "matcap_sampler",
        group: 0,
        binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Matcap),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "rim_multiply_texture",
        group: 0,
        binding: ash_mtoon_sampled_image_binding(MtoonTextureSlot::RimMultiply),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "rim_multiply_sampler",
        group: 0,
        binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::RimMultiply),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "outline_width_texture",
        group: 0,
        binding: ash_mtoon_sampled_image_binding(MtoonTextureSlot::OutlineWidth),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "outline_width_sampler",
        group: 0,
        binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::OutlineWidth),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "uv_animation_mask_texture",
        group: 0,
        binding: ash_mtoon_sampled_image_binding(MtoonTextureSlot::UvAnimationMask),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "uv_animation_mask_sampler",
        group: 0,
        binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::UvAnimationMask),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "emissive_texture",
        group: 0,
        binding: ash_material_sampled_image_binding(GltfMaterialTextureSlot::Emissive),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "emissive_sampler",
        group: 0,
        binding: ash_material_sampler_binding(GltfMaterialTextureSlot::Emissive),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "occlusion_texture",
        group: 0,
        binding: ash_material_sampled_image_binding(GltfMaterialTextureSlot::Occlusion),
        kind: AshWgslResourceKind::SampledImage,
    },
    AshWgslResourceBinding {
        name: "occlusion_sampler",
        group: 0,
        binding: ash_material_sampler_binding(GltfMaterialTextureSlot::Occlusion),
        kind: AshWgslResourceKind::Sampler,
    },
    AshWgslResourceBinding {
        name: "scene",
        group: 0,
        binding: ash_mtoon_wgsl_scene_binding(),
        kind: AshWgslResourceKind::UniformBuffer,
    },
    AshWgslResourceBinding {
        name: "material_uv",
        group: 0,
        binding: ash_mtoon_wgsl_uv_uniform_binding(),
        kind: AshWgslResourceKind::UniformBuffer,
    },
    AshWgslResourceBinding {
        name: "material_extra",
        group: 0,
        binding: ash_mtoon_wgsl_render_extra_binding(),
        kind: AshWgslResourceKind::UniformBuffer,
    },
];

pub const fn ash_mtoon_wgsl_resource_bindings() -> &'static [AshWgslResourceBinding] {
    &ASH_MTOON_WGSL_RESOURCE_BINDINGS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshShaderModulePlan<'a> {
    pub code_words: &'a [u32],
}

impl<'a> AshShaderModulePlan<'a> {
    pub fn shader_module_create_info(self) -> vk::ShaderModuleCreateInfo<'a> {
        vk::ShaderModuleCreateInfo::default().code(self.code_words)
    }
}

pub const fn ash_shader_module_plan(code_words: &[u32]) -> AshShaderModulePlan<'_> {
    AshShaderModulePlan { code_words }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshGraphicsShaderStagesPlan {
    pub vertex_shader: vk::ShaderModule,
    pub fragment_shader: vk::ShaderModule,
}

impl AshGraphicsShaderStagesPlan {
    pub fn shader_stage_create_infos<'a>(
        self,
        vertex_entry: &'a CStr,
        fragment_entry: &'a CStr,
    ) -> [vk::PipelineShaderStageCreateInfo<'a>; 2] {
        [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(self.vertex_shader)
                .name(vertex_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(self.fragment_shader)
                .name(fragment_entry),
        ]
    }
}

pub const fn ash_graphics_shader_stages_plan(
    vertex_shader: vk::ShaderModule,
    fragment_shader: vk::ShaderModule,
) -> AshGraphicsShaderStagesPlan {
    AshGraphicsShaderStagesPlan {
        vertex_shader,
        fragment_shader,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AshWindowedRunValidation {
    pub simple_preview: bool,
    pub require_cache_hits: bool,
    pub require_resize_recreate: bool,
    pub resize_after_frames: Option<u64>,
    pub frames_in_flight: usize,
}

impl AshWindowedRunValidation {
    pub fn validate(self) -> Result<(), String> {
        if self.simple_preview && self.require_cache_hits {
            return Err("--require-cache-hits is only supported by the MToon renderer path".into());
        }
        if self.require_resize_recreate && self.resize_after_frames.is_none() {
            return Err("--require-resize-recreate requires --resize-after-frames".into());
        }
        if self.frames_in_flight == 0 {
            return Err("--frames-in-flight must be at least 1".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshWindowedFrameSyncPlan {
    pub frames_in_flight: usize,
    pub swapchain_images: usize,
}

impl AshWindowedFrameSyncPlan {
    pub fn new(frames_in_flight: usize, swapchain_images: usize) -> Result<Self, String> {
        if frames_in_flight == 0 {
            return Err("frames_in_flight must be at least 1".to_owned());
        }
        if swapchain_images == 0 {
            return Err("swapchain_images must be at least 1".to_owned());
        }
        Ok(Self {
            frames_in_flight,
            swapchain_images,
        })
    }

    pub const fn semaphore_count(self) -> usize {
        self.frames_in_flight * 2
    }

    pub const fn fence_count(self) -> usize {
        self.frames_in_flight
    }

    pub const fn image_fence_slots(self) -> usize {
        self.swapchain_images
    }

    pub fn frame_slot(self, current_frame: usize) -> Result<usize, String> {
        if current_frame >= self.frames_in_flight {
            return Err(format!(
                "current_frame {current_frame} is outside frames_in_flight {}",
                self.frames_in_flight
            ));
        }
        Ok(current_frame)
    }

    pub fn next_frame_index(self, current_frame: usize) -> Result<usize, String> {
        let slot = self.frame_slot(current_frame)?;
        Ok((slot + 1) % self.frames_in_flight)
    }

    pub fn image_index_to_slot(self, image_index: u32) -> Result<usize, String> {
        let slot = usize::try_from(image_index)
            .map_err(|_| format!("swapchain image index {image_index} does not fit usize"))?;
        if slot >= self.swapchain_images {
            return Err(format!(
                "swapchain image index {slot} is outside swapchain_images {}",
                self.swapchain_images
            ));
        }
        Ok(slot)
    }

    pub fn select_acquired_frame(
        self,
        current_frame: usize,
        acquired: AshSwapchainAcquireStatus,
        image_fences: &[vk::Fence],
    ) -> Result<AshWindowedFrameAcquirePlan, String> {
        match acquired {
            AshSwapchainAcquireStatus::NeedsRecreate => {
                Ok(AshWindowedFrameAcquirePlan::NeedsRecreate)
            }
            AshSwapchainAcquireStatus::Acquired {
                image_index,
                suboptimal,
            } => {
                let frame_slot = self.frame_slot(current_frame)?;
                let image_slot = self.image_index_to_slot(image_index)?;
                let previous_image_fence = *image_fences.get(image_slot).ok_or_else(|| {
                    format!(
                        "swapchain image slot {image_slot} has no matching in-flight fence entry"
                    )
                })?;
                Ok(AshWindowedFrameAcquirePlan::Acquired(
                    AshWindowedFrameSyncSelection {
                        frame_slot,
                        swapchain_image_slot: image_slot,
                        image_index,
                        acquired_suboptimal: suboptimal,
                        previous_image_fence,
                        next_frame_index: self.next_frame_index(current_frame)?,
                    },
                ))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshWindowedFrameAcquirePlan {
    Acquired(AshWindowedFrameSyncSelection),
    NeedsRecreate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshWindowedFrameSyncSelection {
    pub frame_slot: usize,
    pub swapchain_image_slot: usize,
    pub image_index: u32,
    pub acquired_suboptimal: bool,
    pub previous_image_fence: vk::Fence,
    pub next_frame_index: usize,
}

impl AshWindowedFrameSyncSelection {
    pub const fn submit_plan(
        self,
        sync_handles: AshWindowedFrameSyncHandles,
        command_buffer: vk::CommandBuffer,
    ) -> AshWindowedSubmitPlan {
        ash_windowed_submit_plan(
            sync_handles.image_available,
            sync_handles.render_finished,
            command_buffer,
            sync_handles.in_flight,
        )
    }

    pub const fn present_plan(
        self,
        render_finished: vk::Semaphore,
        swapchain: vk::SwapchainKHR,
    ) -> AshWindowedPresentPlan {
        ash_windowed_present_plan(render_finished, swapchain, self.image_index)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshWindowedFrameSyncHandles {
    pub image_available: vk::Semaphore,
    pub render_finished: vk::Semaphore,
    pub in_flight: vk::Fence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshWindowedSubmitPlan {
    pub wait_semaphore: vk::Semaphore,
    pub wait_stage: vk::PipelineStageFlags,
    pub command_buffer: vk::CommandBuffer,
    pub signal_semaphore: vk::Semaphore,
    pub fence: vk::Fence,
}

impl AshWindowedSubmitPlan {
    pub const fn wait_semaphores(self) -> [vk::Semaphore; 1] {
        [self.wait_semaphore]
    }

    pub const fn wait_stages(self) -> [vk::PipelineStageFlags; 1] {
        [self.wait_stage]
    }

    pub const fn command_buffers(self) -> [vk::CommandBuffer; 1] {
        [self.command_buffer]
    }

    pub const fn signal_semaphores(self) -> [vk::Semaphore; 1] {
        [self.signal_semaphore]
    }

    pub const fn submit_info_plan(self) -> AshWindowedSubmitInfoPlan {
        AshWindowedSubmitInfoPlan {
            wait_semaphores: self.wait_semaphores(),
            wait_stages: self.wait_stages(),
            command_buffers: self.command_buffers(),
            signal_semaphores: self.signal_semaphores(),
            fence: self.fence,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshWindowedSubmitInfoPlan {
    pub wait_semaphores: [vk::Semaphore; 1],
    pub wait_stages: [vk::PipelineStageFlags; 1],
    pub command_buffers: [vk::CommandBuffer; 1],
    pub signal_semaphores: [vk::Semaphore; 1],
    pub fence: vk::Fence,
}

impl AshWindowedSubmitInfoPlan {
    pub fn submit_info(&self) -> vk::SubmitInfo<'_> {
        vk::SubmitInfo::default()
            .wait_semaphores(&self.wait_semaphores)
            .wait_dst_stage_mask(&self.wait_stages)
            .command_buffers(&self.command_buffers)
            .signal_semaphores(&self.signal_semaphores)
    }

    pub fn submit_infos(&self) -> [vk::SubmitInfo<'_>; 1] {
        [self.submit_info()]
    }
}

pub const fn ash_windowed_submit_plan(
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    command_buffer: vk::CommandBuffer,
    in_flight: vk::Fence,
) -> AshWindowedSubmitPlan {
    AshWindowedSubmitPlan {
        wait_semaphore: image_available,
        wait_stage: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
        command_buffer,
        signal_semaphore: render_finished,
        fence: in_flight,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshWindowedPresentPlan {
    pub wait_semaphore: vk::Semaphore,
    pub swapchain: vk::SwapchainKHR,
    pub image_index: u32,
}

impl AshWindowedPresentPlan {
    pub const fn wait_semaphores(self) -> [vk::Semaphore; 1] {
        [self.wait_semaphore]
    }

    pub const fn swapchains(self) -> [vk::SwapchainKHR; 1] {
        [self.swapchain]
    }

    pub const fn image_indices(self) -> [u32; 1] {
        [self.image_index]
    }

    pub const fn present_info_plan(self) -> AshWindowedPresentInfoPlan {
        AshWindowedPresentInfoPlan {
            wait_semaphores: self.wait_semaphores(),
            swapchains: self.swapchains(),
            image_indices: self.image_indices(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshWindowedPresentInfoPlan {
    pub wait_semaphores: [vk::Semaphore; 1],
    pub swapchains: [vk::SwapchainKHR; 1],
    pub image_indices: [u32; 1],
}

impl AshWindowedPresentInfoPlan {
    pub fn present_info(&self) -> vk::PresentInfoKHR<'_> {
        vk::PresentInfoKHR::default()
            .wait_semaphores(&self.wait_semaphores)
            .swapchains(&self.swapchains)
            .image_indices(&self.image_indices)
    }
}

pub const fn ash_windowed_present_plan(
    render_finished: vk::Semaphore,
    swapchain: vk::SwapchainKHR,
    image_index: u32,
) -> AshWindowedPresentPlan {
    AshWindowedPresentPlan {
        wait_semaphore: render_finished,
        swapchain,
        image_index,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshSwapchainAcquireStatus {
    Acquired { image_index: u32, suboptimal: bool },
    NeedsRecreate,
}

pub fn ash_classify_swapchain_acquire(
    result: Result<(u32, bool), vk::Result>,
) -> Result<AshSwapchainAcquireStatus, vk::Result> {
    match result {
        Ok((image_index, suboptimal)) => Ok(AshSwapchainAcquireStatus::Acquired {
            image_index,
            suboptimal,
        }),
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(AshSwapchainAcquireStatus::NeedsRecreate),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshSwapchainPresentStatus {
    Presented,
    NeedsRecreate,
}

pub fn ash_classify_swapchain_present(
    acquired_suboptimal: bool,
    result: Result<bool, vk::Result>,
) -> Result<AshSwapchainPresentStatus, vk::Result> {
    match result {
        Ok(present_suboptimal) if acquired_suboptimal || present_suboptimal => {
            Ok(AshSwapchainPresentStatus::NeedsRecreate)
        }
        Ok(_) => Ok(AshSwapchainPresentStatus::Presented),
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(AshSwapchainPresentStatus::NeedsRecreate),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AshWindowedResizeValidation {
    pub resize_after_frames: Option<u64>,
    pub resize_requested: bool,
    pub resize_events_after_request: u64,
    pub swapchain_recreates: u64,
}

impl AshWindowedResizeValidation {
    pub fn validate_recreate(self) -> Result<(), String> {
        if self.resize_after_frames.is_none() {
            return Err("--require-resize-recreate requires --resize-after-frames".to_owned());
        }
        if !self.resize_requested {
            return Err("resize was never requested".to_owned());
        }
        if self.resize_events_after_request == 0 {
            return Err("no WindowEvent::Resized was observed after resize request".to_owned());
        }
        if self.swapchain_recreates == 0 {
            return Err("renderer.recreate_swapchain was never called".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AshWindowedCacheCounter {
    pub hits: u64,
    pub rebuilds: u64,
}

impl AshWindowedCacheCounter {
    pub fn hit(&mut self) {
        self.hits = self.hits.saturating_add(1);
    }

    pub fn rebuild(&mut self) {
        self.rebuilds = self.rebuilds.saturating_add(1);
    }

    pub fn validate_hits(self, name: &str) -> Result<(), String> {
        (self.hits > 0)
            .then_some(())
            .ok_or_else(|| format!("{name} cache reported no hits; run at least two MToon frames"))
    }
}

impl fmt::Display for AshWindowedCacheCounter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "hits={},rebuilds={}", self.hits, self.rebuilds)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AshMtoonWindowedCacheStats {
    pub pipeline: AshWindowedCacheCounter,
    pub descriptors: AshWindowedCacheCounter,
    pub samplers: AshWindowedCacheCounter,
    pub buffers: AshWindowedCacheCounter,
    pub uniforms: AshWindowedCacheCounter,
    pub textures: AshWindowedCacheCounter,
    pub fallback_textures: AshWindowedCacheCounter,
    pub command_buffers: AshWindowedCacheCounter,
}

impl AshMtoonWindowedCacheStats {
    pub fn validate_steady_state_hits(self) -> Result<(), String> {
        self.pipeline.validate_hits("pipeline")?;
        self.descriptors.validate_hits("descriptor")?;
        self.samplers.validate_hits("sampler")?;
        self.buffers.validate_hits("buffer")?;
        self.uniforms.validate_hits("uniform")?;
        self.textures.validate_hits("texture")?;
        self.fallback_textures.validate_hits("fallback texture")?;
        self.command_buffers.validate_hits("draw command buffer")?;
        Ok(())
    }
}

impl fmt::Display for AshMtoonWindowedCacheStats {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "pipeline({}); descriptors({}); samplers({}); buffers({}); uniforms({}); textures({}); fallback_textures({}); command_buffers({})",
            self.pipeline,
            self.descriptors,
            self.samplers,
            self.buffers,
            self.uniforms,
            self.textures,
            self.fallback_textures,
            self.command_buffers
        )
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

impl Default for AshSamplerPlan {
    fn default() -> Self {
        Self {
            mag_filter: vk::Filter::LINEAR,
            min_filter: vk::Filter::LINEAR,
            mipmap_mode: vk::SamplerMipmapMode::LINEAR,
            address_mode_u: vk::SamplerAddressMode::REPEAT,
            address_mode_v: vk::SamplerAddressMode::REPEAT,
            min_lod: 0.0,
            max_lod: 32.0,
            normal_map_decode: false,
        }
    }
}

impl AshSamplerPlan {
    pub fn sampler_create_info(self) -> vk::SamplerCreateInfo<'static> {
        vk::SamplerCreateInfo::default()
            .mag_filter(self.mag_filter)
            .min_filter(self.min_filter)
            .mipmap_mode(self.mipmap_mode)
            .address_mode_u(self.address_mode_u)
            .address_mode_v(self.address_mode_v)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .min_lod(self.min_lod)
            .max_lod(self.max_lod)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AshSamplerResourcePlan {
    pub sampler_index: usize,
    pub descriptor_set_index: usize,
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub sampler: AshSamplerPlan,
}

pub const ASH_FALLBACK_TEXTURES: [GltfMaterialTextureFallback; 3] = [
    GltfMaterialTextureFallback::White,
    GltfMaterialTextureFallback::Black,
    GltfMaterialTextureFallback::NeutralNormal,
];

pub const fn ash_fallback_texture_rgba(fallback: GltfMaterialTextureFallback) -> [u8; 4] {
    match fallback {
        GltfMaterialTextureFallback::White => [255, 255, 255, 255],
        GltfMaterialTextureFallback::Black => [0, 0, 0, 255],
        GltfMaterialTextureFallback::NeutralNormal => [128, 128, 255, 255],
    }
}

pub fn ash_fallback_texture_mip_level(fallback: GltfMaterialTextureFallback) -> RgbaMipLevel {
    RgbaMipLevel {
        width: 1,
        height: 1,
        rgba: ash_fallback_texture_rgba(fallback).to_vec(),
    }
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
    pub clip_space_policy: AshClipSpacePolicy,
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
            clip_space_policy: AshClipSpacePolicy::default(),
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
        projection.y_axis.y *= self.clip_space_policy.projection_y_sign();
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Debug)]
pub struct AshTextureUploadCommandPlan {
    pub subresource_range: vk::ImageSubresourceRange,
    pub copy_regions: Vec<vk::BufferImageCopy>,
}

impl AshTextureUploadCommandPlan {
    pub fn transfer_dst_barrier(&self, image: vk::Image) -> vk::ImageMemoryBarrier<'static> {
        vk::ImageMemoryBarrier::default()
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .image(image)
            .subresource_range(self.subresource_range)
    }

    pub fn transfer_dst_barrier_command(&self, image: vk::Image) -> AshImageBarrierCommand {
        AshImageBarrierCommand {
            src_stage_mask: vk::PipelineStageFlags::TOP_OF_PIPE,
            dst_stage_mask: vk::PipelineStageFlags::TRANSFER,
            dependency_flags: vk::DependencyFlags::empty(),
            image_barriers: [self.transfer_dst_barrier(image)],
        }
    }

    pub fn shader_read_barrier(&self, image: vk::Image) -> vk::ImageMemoryBarrier<'static> {
        vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image(image)
            .subresource_range(self.subresource_range)
    }

    pub fn shader_read_barrier_command(&self, image: vk::Image) -> AshImageBarrierCommand {
        AshImageBarrierCommand {
            src_stage_mask: vk::PipelineStageFlags::TRANSFER,
            dst_stage_mask: vk::PipelineStageFlags::FRAGMENT_SHADER,
            dependency_flags: vk::DependencyFlags::empty(),
            image_barriers: [self.shader_read_barrier(image)],
        }
    }

    pub fn buffer_to_image_copy_command(
        &self,
        staging_buffer: vk::Buffer,
        image: vk::Image,
    ) -> AshBufferToImageCopyCommand<'_> {
        AshBufferToImageCopyCommand {
            buffer: staging_buffer,
            image,
            image_layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            regions: &self.copy_regions,
        }
    }

    pub fn command_sequence(
        &self,
        image: vk::Image,
        staging_buffer: vk::Buffer,
    ) -> AshTextureUploadCommandSequence<'_> {
        AshTextureUploadCommandSequence {
            transfer_dst_barrier: self.transfer_dst_barrier_command(image),
            copy: self.buffer_to_image_copy_command(staging_buffer, image),
            shader_read_barrier: self.shader_read_barrier_command(image),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AshImageBarrierCommand {
    pub src_stage_mask: vk::PipelineStageFlags,
    pub dst_stage_mask: vk::PipelineStageFlags,
    pub dependency_flags: vk::DependencyFlags,
    pub image_barriers: [vk::ImageMemoryBarrier<'static>; 1],
}

#[derive(Clone, Copy, Debug)]
pub struct AshBufferToImageCopyCommand<'a> {
    pub buffer: vk::Buffer,
    pub image: vk::Image,
    pub image_layout: vk::ImageLayout,
    pub regions: &'a [vk::BufferImageCopy],
}

#[derive(Clone, Copy, Debug)]
pub struct AshTextureUploadCommandSequence<'a> {
    pub transfer_dst_barrier: AshImageBarrierCommand,
    pub copy: AshBufferToImageCopyCommand<'a>,
    pub shader_read_barrier: AshImageBarrierCommand,
}

pub fn ash_texture_upload_command_plan(mip_levels: &[RgbaMipLevel]) -> AshTextureUploadCommandPlan {
    let level_count = u32::try_from(mip_levels.len()).unwrap_or(u32::MAX).max(1);
    AshTextureUploadCommandPlan {
        subresource_range: vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(level_count)
            .layer_count(1),
        copy_regions: ash_texture_mip_copy_regions(mip_levels),
    }
}

pub fn ash_texture_mip_upload_bytes(mip_levels: &[RgbaMipLevel]) -> Vec<u8> {
    let byte_len = mip_levels.iter().map(|level| level.rgba.len()).sum();
    let mut bytes = Vec::with_capacity(byte_len);
    mip_levels
        .iter()
        .for_each(|level| bytes.extend_from_slice(&level.rgba));
    bytes
}

pub fn ash_texture_mip_copy_regions(mip_levels: &[RgbaMipLevel]) -> Vec<vk::BufferImageCopy> {
    let mut offset = 0_u64;
    mip_levels
        .iter()
        .enumerate()
        .map(|(mip_level, level)| {
            let region = vk::BufferImageCopy::default()
                .buffer_offset(offset)
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(u32::try_from(mip_level).unwrap_or(u32::MAX))
                        .layer_count(1),
                )
                .image_extent(vk::Extent3D {
                    width: level.width,
                    height: level.height,
                    depth: 1,
                });
            offset = offset.saturating_add(u64::try_from(level.rgba.len()).unwrap_or(u64::MAX));
            region
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct AshColorAttachmentReadbackPlan {
    pub subresource_range: vk::ImageSubresourceRange,
    pub copy_region: vk::BufferImageCopy,
}

impl AshColorAttachmentReadbackPlan {
    pub fn transfer_src_barrier(&self, image: vk::Image) -> vk::ImageMemoryBarrier<'static> {
        vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .image(image)
            .subresource_range(self.subresource_range)
    }

    pub fn transfer_src_barrier_command(&self, image: vk::Image) -> AshImageBarrierCommand {
        AshImageBarrierCommand {
            src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            dst_stage_mask: vk::PipelineStageFlags::TRANSFER,
            dependency_flags: vk::DependencyFlags::empty(),
            image_barriers: [self.transfer_src_barrier(image)],
        }
    }

    pub fn copy_regions(&self) -> [vk::BufferImageCopy; 1] {
        [self.copy_region]
    }

    pub fn image_to_buffer_copy_command(
        &self,
        image: vk::Image,
        buffer: vk::Buffer,
    ) -> AshImageToBufferCopyCommand {
        AshImageToBufferCopyCommand {
            image,
            image_layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            buffer,
            regions: self.copy_regions(),
        }
    }

    pub fn command_sequence(
        &self,
        image: vk::Image,
        buffer: vk::Buffer,
    ) -> AshColorAttachmentReadbackCommandSequence {
        AshColorAttachmentReadbackCommandSequence {
            transfer_src_barrier: self.transfer_src_barrier_command(image),
            copy: self.image_to_buffer_copy_command(image, buffer),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AshImageToBufferCopyCommand {
    pub image: vk::Image,
    pub image_layout: vk::ImageLayout,
    pub buffer: vk::Buffer,
    pub regions: [vk::BufferImageCopy; 1],
}

#[derive(Clone, Copy, Debug)]
pub struct AshColorAttachmentReadbackCommandSequence {
    pub transfer_src_barrier: AshImageBarrierCommand,
    pub copy: AshImageToBufferCopyCommand,
}

pub fn ash_color_attachment_readback_plan(extent: vk::Extent2D) -> AshColorAttachmentReadbackPlan {
    let subresource_range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1);
    let copy_region = vk::BufferImageCopy::default()
        .image_subresource(
            vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .layer_count(1),
        )
        .image_extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        });
    AshColorAttachmentReadbackPlan {
        subresource_range,
        copy_region,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshDepthAttachmentPlan {
    pub format: vk::Format,
    pub extent: vk::Extent3D,
    pub image_usage: vk::ImageUsageFlags,
    pub aspect_mask: vk::ImageAspectFlags,
    pub final_layout: vk::ImageLayout,
}

pub fn ash_depth_aspect_mask(format: vk::Format) -> vk::ImageAspectFlags {
    match format {
        vk::Format::D16_UNORM_S8_UINT
        | vk::Format::D24_UNORM_S8_UINT
        | vk::Format::D32_SFLOAT_S8_UINT => {
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
        }
        _ => vk::ImageAspectFlags::DEPTH,
    }
}

pub fn ash_depth_attachment_plan(
    format: vk::Format,
    extent: vk::Extent2D,
) -> AshDepthAttachmentPlan {
    AshDepthAttachmentPlan {
        format,
        extent: vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        },
        image_usage: vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
        aspect_mask: ash_depth_aspect_mask(format),
        final_layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
    }
}

pub const fn ash_depth_format_candidates() -> [vk::Format; 3] {
    [
        ash_reference_depth_format(),
        vk::Format::X8_D24_UNORM_PACK32,
        vk::Format::D32_SFLOAT,
    ]
}

pub fn ash_select_depth_format(
    mut format_properties: impl FnMut(vk::Format) -> vk::FormatProperties,
) -> Result<vk::Format, String> {
    ash_depth_format_candidates()
        .into_iter()
        .find(|format| {
            format_properties(*format)
                .optimal_tiling_features
                .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT)
        })
        .ok_or_else(|| "no supported Vulkan depth attachment format found".to_owned())
}

pub fn ash_memory_type_index(
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required_properties: vk::MemoryPropertyFlags,
) -> Result<u32, String> {
    (0..memory_properties.memory_type_count)
        .find(|index| {
            let type_supported = (type_bits & (1_u32 << index)) != 0;
            let memory_type = memory_properties.memory_types[*index as usize];
            type_supported && memory_type.property_flags.contains(required_properties)
        })
        .ok_or_else(|| {
            format!("no Vulkan memory type supports {required_properties:?} for type bits 0x{type_bits:08x}")
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshColorAttachmentFinalLayout {
    Present,
    ColorAttachment,
}

impl AshColorAttachmentFinalLayout {
    pub const fn vk_layout(self) -> vk::ImageLayout {
        match self {
            Self::Present => vk::ImageLayout::PRESENT_SRC_KHR,
            Self::ColorAttachment => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshRenderPassDependencyPolicy {
    ColorOnly,
    ColorAndDepth,
}

impl AshRenderPassDependencyPolicy {
    pub fn dst_stage_mask(self) -> vk::PipelineStageFlags {
        match self {
            Self::ColorOnly => vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            Self::ColorAndDepth => {
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
            }
        }
    }

    pub fn dst_access_mask(self) -> vk::AccessFlags {
        match self {
            Self::ColorOnly => vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            Self::ColorAndDepth => {
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshRenderPassCreationPlan {
    pub color_format: vk::Format,
    pub depth_format: vk::Format,
    pub color_final_layout: AshColorAttachmentFinalLayout,
    pub dependency_policy: AshRenderPassDependencyPolicy,
}

impl AshRenderPassCreationPlan {
    pub fn attachment_descriptions(self) -> [vk::AttachmentDescription; 2] {
        [
            vk::AttachmentDescription::default()
                .format(self.color_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(self.color_final_layout.vk_layout()),
            vk::AttachmentDescription::default()
                .format(self.depth_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
        ]
    }

    pub const fn color_attachment_references(self) -> [vk::AttachmentReference; 1] {
        [vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        }]
    }

    pub const fn depth_attachment_reference(self) -> vk::AttachmentReference {
        vk::AttachmentReference {
            attachment: 1,
            layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
        }
    }

    pub fn subpass_dependency(self) -> vk::SubpassDependency {
        vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(self.dependency_policy.dst_stage_mask())
            .dst_stage_mask(self.dependency_policy.dst_stage_mask())
            .dst_access_mask(self.dependency_policy.dst_access_mask())
    }

    pub fn with_render_pass_create_info<R>(
        self,
        f: impl FnOnce(vk::RenderPassCreateInfo<'_>) -> R,
    ) -> R {
        let attachments = self.attachment_descriptions();
        let color_attachment = self.color_attachment_references();
        let depth_attachment = self.depth_attachment_reference();
        let subpass = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment)
            .depth_stencil_attachment(&depth_attachment)];
        let dependency = [self.subpass_dependency()];
        let info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpass)
            .dependencies(&dependency);
        f(info)
    }
}

pub const fn ash_render_pass_creation_plan(
    color_format: vk::Format,
    depth_format: vk::Format,
    color_final_layout: AshColorAttachmentFinalLayout,
    dependency_policy: AshRenderPassDependencyPolicy,
) -> AshRenderPassCreationPlan {
    AshRenderPassCreationPlan {
        color_format,
        depth_format,
        color_final_layout,
        dependency_policy,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshFramebufferPlan {
    pub extent: vk::Extent2D,
    pub layers: u32,
}

impl AshFramebufferPlan {
    pub const fn width(self) -> u32 {
        self.extent.width
    }

    pub const fn height(self) -> u32 {
        self.extent.height
    }

    pub fn framebuffer_create_info<'a>(
        self,
        render_pass: vk::RenderPass,
        attachments: &'a [vk::ImageView],
    ) -> vk::FramebufferCreateInfo<'a> {
        vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(attachments)
            .width(self.width())
            .height(self.height())
            .layers(self.layers)
    }
}

pub const fn ash_framebuffer_plan(extent: vk::Extent2D) -> AshFramebufferPlan {
    AshFramebufferPlan { extent, layers: 1 }
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

impl AshRendererFrame {
    pub fn resource_manifest(&self) -> AshRendererResourceManifest {
        ash_renderer_resource_manifest(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshRendererResourceLifetime {
    /// Resource shape is expected to be stable across animation frames for the same asset,
    /// shader ABI, render target format, and render options.
    Persistent,
    /// Resource contents or shape can change every sampled frame.
    FrameDynamic,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AshRendererResourceManifest {
    pub buffers: Vec<AshRendererBufferResource>,
    pub textures: Vec<AshRendererTextureResource>,
    pub uniforms: Vec<AshRendererUniformResource>,
    pub samplers: Vec<AshRendererSamplerResource>,
    pub descriptor_set_layouts: Vec<AshRendererDescriptorSetLayoutResource>,
    pub descriptor_sets: Vec<AshRendererDescriptorSetResource>,
    pub pipelines: Vec<AshRendererPipelineResource>,
}

impl AshRendererResourceManifest {
    pub fn persistent_resource_count(&self) -> usize {
        self.textures.len()
            + self.samplers.len()
            + self.descriptor_set_layouts.len()
            + self.pipelines.len()
    }

    pub fn dynamic_resource_count(&self) -> usize {
        self.buffers.len() + self.uniforms.len() + self.descriptor_sets.len()
    }

    pub fn persistent_handle_resource_count(&self) -> usize {
        self.buffers
            .iter()
            .filter(|resource| resource.handle_lifetime == AshRendererResourceLifetime::Persistent)
            .count()
            + self
                .textures
                .iter()
                .filter(|resource| {
                    resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                })
                .count()
            + self
                .uniforms
                .iter()
                .filter(|resource| {
                    resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                })
                .count()
            + self
                .samplers
                .iter()
                .filter(|resource| {
                    resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                })
                .count()
            + self
                .descriptor_set_layouts
                .iter()
                .filter(|resource| {
                    resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                })
                .count()
            + self
                .descriptor_sets
                .iter()
                .filter(|resource| {
                    resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                })
                .count()
            + self
                .pipelines
                .iter()
                .filter(|resource| {
                    resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                })
                .count()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshRendererBufferResource {
    pub index: usize,
    pub role: AshBufferRole,
    pub usage: vk::BufferUsageFlags,
    pub stride: u32,
    pub count: u32,
    pub byte_len: usize,
    pub handle_lifetime: AshRendererResourceLifetime,
    pub lifetime: AshRendererResourceLifetime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshRendererTextureResource {
    pub index: usize,
    pub texture: Option<TextureRef>,
    pub color_space: GltfMaterialTextureColorSpace,
    pub format: vk::Format,
    pub extent: vk::Extent3D,
    pub image_usage: vk::ImageUsageFlags,
    pub image_layout_after_upload: vk::ImageLayout,
    pub aspect_mask: vk::ImageAspectFlags,
    pub byte_len: usize,
    pub handle_lifetime: AshRendererResourceLifetime,
    pub lifetime: AshRendererResourceLifetime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshRendererUniformResource {
    pub index: usize,
    pub scope: AshUniformScope,
    pub binding: u32,
    pub byte_len: usize,
    pub handle_lifetime: AshRendererResourceLifetime,
    pub lifetime: AshRendererResourceLifetime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshRendererSamplerResource {
    pub descriptor_set_index: usize,
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub sampler: Option<AshSamplerPlan>,
    pub handle_lifetime: AshRendererResourceLifetime,
    pub lifetime: AshRendererResourceLifetime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AshDescriptorBindingLayoutKey {
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub stage_flags: vk::ShaderStageFlags,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshRendererDescriptorSetLayoutResource {
    pub descriptor_set_index: usize,
    pub material: MaterialRef,
    pub pipeline_plan_index: usize,
    pub bindings: Vec<AshDescriptorBindingLayoutKey>,
    pub handle_lifetime: AshRendererResourceLifetime,
    pub lifetime: AshRendererResourceLifetime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshRendererDescriptorBindingResource {
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub uniform_upload_index: Option<usize>,
    pub texture_upload_index: Option<usize>,
    pub buffer_upload_index: Option<usize>,
    pub lifetime: AshRendererResourceLifetime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshRendererDescriptorSetResource {
    pub index: usize,
    pub material: MaterialRef,
    pub pipeline_plan_index: usize,
    pub bindings: Vec<AshRendererDescriptorBindingResource>,
    pub handle_lifetime: AshRendererResourceLifetime,
    pub lifetime: AshRendererResourceLifetime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshRendererPipelineResource {
    pub index: usize,
    pub material: MaterialRef,
    pub pipeline_plan_index: usize,
    pub descriptor_set_index: usize,
    pub key: AshPipelineKey,
    pub vertex_stride: u32,
    pub vertex_attributes: Vec<AshVertexAttributePlan>,
    pub color_format: vk::Format,
    pub depth_format: Option<vk::Format>,
    pub handle_lifetime: AshRendererResourceLifetime,
    pub lifetime: AshRendererResourceLifetime,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AshDescriptorPoolPlan {
    pub max_sets: u32,
    pub pool_sizes: Vec<AshDescriptorPoolSizePlan>,
}

impl AshDescriptorPoolPlan {
    pub fn vk_pool_sizes(&self) -> Vec<vk::DescriptorPoolSize> {
        self.pool_sizes
            .iter()
            .map(|size| vk::DescriptorPoolSize {
                ty: size.descriptor_type,
                descriptor_count: size.descriptor_count,
            })
            .collect()
    }

    pub fn with_descriptor_pool_create_info<R>(
        &self,
        f: impl FnOnce(vk::DescriptorPoolCreateInfo<'_>) -> R,
    ) -> R {
        let sizes = self.vk_pool_sizes();
        let info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(self.max_sets)
            .pool_sizes(&sizes);
        f(info)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshDescriptorPoolSizePlan {
    pub descriptor_type: vk::DescriptorType,
    pub descriptor_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AshDescriptorSetLayoutPlan {
    pub descriptor_set_index: usize,
    pub material: MaterialRef,
    pub pipeline_plan_index: usize,
    pub bindings: Vec<AshDescriptorBindingLayoutKey>,
}

impl AshDescriptorSetLayoutPlan {
    pub fn vk_bindings(&self) -> Vec<vk::DescriptorSetLayoutBinding<'static>> {
        self.bindings
            .iter()
            .map(|binding| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding.binding)
                    .descriptor_type(binding.descriptor_type)
                    .descriptor_count(1)
                    .stage_flags(binding.stage_flags)
            })
            .collect()
    }

    pub fn with_descriptor_set_layout_create_info<R>(
        &self,
        f: impl FnOnce(vk::DescriptorSetLayoutCreateInfo<'_>) -> R,
    ) -> R {
        let bindings = self.vk_bindings();
        let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        f(info)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshPipelineLayoutPlan {
    pub descriptor_set_layout_index: usize,
}

impl AshPipelineLayoutPlan {
    pub fn vk_set_layouts(
        self,
        descriptor_set_layouts: &[vk::DescriptorSetLayout],
    ) -> Result<[vk::DescriptorSetLayout; 1], String> {
        descriptor_set_layouts
            .get(self.descriptor_set_layout_index)
            .copied()
            .map(|layout| [layout])
            .ok_or_else(|| {
                format!(
                    "pipeline layout references missing descriptor set layout {}",
                    self.descriptor_set_layout_index
                )
            })
    }

    pub fn with_pipeline_layout_create_info<R>(
        self,
        descriptor_set_layouts: &[vk::DescriptorSetLayout],
        f: impl FnOnce(vk::PipelineLayoutCreateInfo<'_>) -> R,
    ) -> Result<R, String> {
        let layouts = self.vk_set_layouts(descriptor_set_layouts)?;
        let info = vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts);
        Ok(f(info))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AshEmptyPipelineLayoutPlan;

impl AshEmptyPipelineLayoutPlan {
    pub fn pipeline_layout_create_info(self) -> vk::PipelineLayoutCreateInfo<'static> {
        vk::PipelineLayoutCreateInfo::default()
    }
}

pub const fn ash_empty_pipeline_layout_plan() -> AshEmptyPipelineLayoutPlan {
    AshEmptyPipelineLayoutPlan
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AshDescriptorSetAllocationPlan {
    pub descriptor_set_layout_indices: Vec<usize>,
}

impl AshDescriptorSetAllocationPlan {
    pub fn descriptor_set_count(&self) -> usize {
        self.descriptor_set_layout_indices.len()
    }

    pub fn vk_set_layouts(
        &self,
        descriptor_set_layouts: &[vk::DescriptorSetLayout],
    ) -> Result<Vec<vk::DescriptorSetLayout>, String> {
        self.descriptor_set_layout_indices
            .iter()
            .map(|index| {
                descriptor_set_layouts.get(*index).copied().ok_or_else(|| {
                    format!("descriptor set allocation references missing layout {index}")
                })
            })
            .collect()
    }

    pub fn with_descriptor_set_allocate_info<R>(
        &self,
        descriptor_pool: vk::DescriptorPool,
        descriptor_set_layouts: &[vk::DescriptorSetLayout],
        f: impl FnOnce(vk::DescriptorSetAllocateInfo<'_>) -> R,
    ) -> Result<R, String> {
        let layouts = self.vk_set_layouts(descriptor_set_layouts)?;
        let info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);
        Ok(f(info))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AshDescriptorWritePlan {
    pub descriptor_set_index: usize,
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub resource: AshDescriptorWriteResource,
}

#[derive(Clone, Copy, Debug)]
pub struct AshDescriptorWriteResources<'a, Uniform, Storage, Image> {
    pub uniform_buffers: &'a [Uniform],
    pub storage_buffers: &'a [Storage],
    pub texture_uploads: &'a [Image],
    pub samplers: &'a [vk::Sampler],
}

impl<'a, Uniform, Storage, Image> AshDescriptorWriteResources<'a, Uniform, Storage, Image> {
    pub const fn new(
        uniform_buffers: &'a [Uniform],
        storage_buffers: &'a [Storage],
        texture_uploads: &'a [Image],
        samplers: &'a [vk::Sampler],
    ) -> Self {
        Self {
            uniform_buffers,
            storage_buffers,
            texture_uploads,
            samplers,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AshDescriptorWriteHandleAccess<
    UniformBuffer,
    StorageBuffer,
    TextureImageView,
    FallbackImageView,
> {
    pub uniform_buffer: UniformBuffer,
    pub storage_buffer: StorageBuffer,
    pub texture_image_view: TextureImageView,
    pub fallback_image_view: FallbackImageView,
    pub image_layout: vk::ImageLayout,
}

impl AshDescriptorWritePlan {
    pub fn vk_descriptor_set(
        &self,
        descriptor_sets: &[vk::DescriptorSet],
    ) -> Result<vk::DescriptorSet, String> {
        descriptor_sets
            .get(self.descriptor_set_index)
            .copied()
            .ok_or_else(|| {
                format!(
                    "descriptor write references missing descriptor set {}",
                    self.descriptor_set_index
                )
            })
    }

    pub fn with_write_descriptor_set<R>(
        &self,
        descriptor_sets: &[vk::DescriptorSet],
        data: AshDescriptorWriteData,
        f: impl FnOnce(vk::WriteDescriptorSet<'_>) -> R,
    ) -> Result<R, String> {
        let descriptor_set = self.vk_descriptor_set(descriptor_sets)?;
        match (&self.resource, data) {
            (
                AshDescriptorWriteResource::UniformBuffer { .. }
                | AshDescriptorWriteResource::StorageBuffer { .. },
                AshDescriptorWriteData::Buffer {
                    buffer,
                    offset,
                    range,
                },
            ) => {
                let buffer_info = [vk::DescriptorBufferInfo::default()
                    .buffer(buffer)
                    .offset(offset)
                    .range(range)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_set)
                    .dst_binding(self.binding)
                    .descriptor_type(self.descriptor_type)
                    .buffer_info(&buffer_info);
                Ok(f(write))
            }
            (
                AshDescriptorWriteResource::CombinedImageSampler { .. },
                AshDescriptorWriteData::CombinedImageSampler {
                    sampler,
                    image_view,
                    image_layout,
                },
            ) => {
                let image_info = [vk::DescriptorImageInfo::default()
                    .sampler(sampler)
                    .image_view(image_view)
                    .image_layout(image_layout)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_set)
                    .dst_binding(self.binding)
                    .descriptor_type(self.descriptor_type)
                    .image_info(&image_info);
                Ok(f(write))
            }
            (
                AshDescriptorWriteResource::SampledImage { .. },
                AshDescriptorWriteData::SampledImage {
                    image_view,
                    image_layout,
                },
            ) => {
                let image_info = [vk::DescriptorImageInfo::default()
                    .image_view(image_view)
                    .image_layout(image_layout)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_set)
                    .dst_binding(self.binding)
                    .descriptor_type(self.descriptor_type)
                    .image_info(&image_info);
                Ok(f(write))
            }
            (
                AshDescriptorWriteResource::Sampler { .. },
                AshDescriptorWriteData::Sampler { sampler },
            ) => {
                let image_info = [vk::DescriptorImageInfo::default().sampler(sampler)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_set)
                    .dst_binding(self.binding)
                    .descriptor_type(self.descriptor_type)
                    .image_info(&image_info);
                Ok(f(write))
            }
            (resource, data) => Err(format!(
                "descriptor write data {data:?} does not match resource {resource:?}"
            )),
        }
    }

    pub fn resolve_write_data<
        Uniform,
        Storage,
        Image,
        UniformBuffer,
        StorageBuffer,
        TextureImageView,
        FallbackImageView,
    >(
        &self,
        resources: AshDescriptorWriteResources<'_, Uniform, Storage, Image>,
        access: AshDescriptorWriteHandleAccess<
            UniformBuffer,
            StorageBuffer,
            TextureImageView,
            FallbackImageView,
        >,
    ) -> Result<AshDescriptorWriteData, String>
    where
        UniformBuffer: FnOnce(&Uniform) -> vk::Buffer,
        StorageBuffer: FnOnce(&Storage) -> vk::Buffer,
        TextureImageView: FnOnce(&Image) -> vk::ImageView,
        FallbackImageView: FnOnce(GltfMaterialTextureFallback) -> vk::ImageView,
    {
        let AshDescriptorWriteHandleAccess {
            uniform_buffer,
            storage_buffer,
            texture_image_view,
            fallback_image_view,
            image_layout,
        } = access;

        match &self.resource {
            AshDescriptorWriteResource::UniformBuffer { .. } => {
                let uniform = self.resource.uniform_resource(resources.uniform_buffers)?;
                Ok(AshDescriptorWriteData::whole_buffer(uniform_buffer(
                    uniform,
                )))
            }
            AshDescriptorWriteResource::StorageBuffer { .. } => {
                let buffer = self
                    .resource
                    .storage_buffer_resource(resources.storage_buffers)?;
                Ok(AshDescriptorWriteData::whole_buffer(storage_buffer(buffer)))
            }
            AshDescriptorWriteResource::CombinedImageSampler { .. } => {
                let sampler = self.resource.sampler(resources.samplers)?;
                let image_view = self.resource.resolve_image_view(
                    resources.texture_uploads,
                    texture_image_view,
                    fallback_image_view,
                )?;
                Ok(AshDescriptorWriteData::combined_image_sampler(
                    sampler,
                    image_view,
                    image_layout,
                ))
            }
            AshDescriptorWriteResource::SampledImage { .. } => {
                let image_view = self.resource.resolve_image_view(
                    resources.texture_uploads,
                    texture_image_view,
                    fallback_image_view,
                )?;
                Ok(AshDescriptorWriteData::sampled_image(
                    image_view,
                    image_layout,
                ))
            }
            AshDescriptorWriteResource::Sampler { .. } => {
                let sampler = self.resource.sampler(resources.samplers)?;
                Ok(AshDescriptorWriteData::sampler(sampler))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshDescriptorWriteData {
    Buffer {
        buffer: vk::Buffer,
        offset: vk::DeviceSize,
        range: vk::DeviceSize,
    },
    CombinedImageSampler {
        sampler: vk::Sampler,
        image_view: vk::ImageView,
        image_layout: vk::ImageLayout,
    },
    SampledImage {
        image_view: vk::ImageView,
        image_layout: vk::ImageLayout,
    },
    Sampler {
        sampler: vk::Sampler,
    },
}

impl AshDescriptorWriteData {
    pub fn whole_buffer(buffer: vk::Buffer) -> Self {
        Self::Buffer {
            buffer,
            offset: 0,
            range: vk::WHOLE_SIZE,
        }
    }

    pub fn combined_image_sampler(
        sampler: vk::Sampler,
        image_view: vk::ImageView,
        image_layout: vk::ImageLayout,
    ) -> Self {
        Self::CombinedImageSampler {
            sampler,
            image_view,
            image_layout,
        }
    }

    pub fn sampled_image(image_view: vk::ImageView, image_layout: vk::ImageLayout) -> Self {
        Self::SampledImage {
            image_view,
            image_layout,
        }
    }

    pub fn sampler(sampler: vk::Sampler) -> Self {
        Self::Sampler { sampler }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AshDescriptorWriteResource {
    UniformBuffer {
        uniform_upload_index: usize,
    },
    StorageBuffer {
        buffer_upload_index: usize,
    },
    CombinedImageSampler {
        sampler_index: usize,
        image: AshDescriptorImageResource,
    },
    SampledImage {
        image: AshDescriptorImageResource,
    },
    Sampler {
        sampler_index: usize,
    },
}

impl AshDescriptorWriteResource {
    pub fn uniform_resource<'a, T>(&self, resources: &'a [T]) -> Result<&'a T, String> {
        match self {
            Self::UniformBuffer {
                uniform_upload_index,
            } => resources.get(*uniform_upload_index).ok_or_else(|| {
                format!("descriptor write references missing uniform buffer {uniform_upload_index}")
            }),
            other => Err(format!(
                "descriptor write resource is not a uniform buffer: {other:?}"
            )),
        }
    }

    pub fn storage_buffer_resource<'a, T>(&self, resources: &'a [T]) -> Result<&'a T, String> {
        match self {
            Self::StorageBuffer {
                buffer_upload_index,
            } => resources.get(*buffer_upload_index).ok_or_else(|| {
                format!("descriptor write references missing storage buffer {buffer_upload_index}")
            }),
            other => Err(format!(
                "descriptor write resource is not a storage buffer: {other:?}"
            )),
        }
    }

    pub fn sampler(&self, samplers: &[vk::Sampler]) -> Result<vk::Sampler, String> {
        match self {
            Self::CombinedImageSampler { sampler_index, .. } | Self::Sampler { sampler_index } => {
                samplers.get(*sampler_index).copied().ok_or_else(|| {
                    format!("descriptor write references missing sampler {sampler_index}")
                })
            }
            other => Err(format!(
                "descriptor write resource does not reference a sampler: {other:?}"
            )),
        }
    }

    pub fn image_resource(&self) -> Result<AshDescriptorImageResource, String> {
        match self {
            Self::CombinedImageSampler { image, .. } | Self::SampledImage { image } => Ok(*image),
            other => Err(format!(
                "descriptor write resource does not reference an image: {other:?}"
            )),
        }
    }

    pub fn resolve_image_view<Image>(
        &self,
        texture_uploads: &[Image],
        texture_image_view: impl FnOnce(&Image) -> vk::ImageView,
        fallback_image_view: impl FnOnce(GltfMaterialTextureFallback) -> vk::ImageView,
    ) -> Result<vk::ImageView, String> {
        match self.image_resource()? {
            AshDescriptorImageResource::TextureUpload {
                texture_upload_index,
            } => texture_uploads
                .get(texture_upload_index)
                .map(texture_image_view)
                .ok_or_else(|| {
                    format!(
                        "descriptor image references missing texture upload {texture_upload_index}"
                    )
                }),
            AshDescriptorImageResource::Fallback { fallback } => Ok(fallback_image_view(fallback)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshDescriptorImageResource {
    TextureUpload {
        texture_upload_index: usize,
    },
    Fallback {
        fallback: GltfMaterialTextureFallback,
    },
}

impl AshDescriptorImageResource {
    pub fn texture_upload_resource<'a, T>(
        &self,
        texture_uploads: &'a [T],
    ) -> Result<&'a T, String> {
        match self {
            Self::TextureUpload {
                texture_upload_index,
            } => texture_uploads.get(*texture_upload_index).ok_or_else(|| {
                format!("descriptor image references missing texture upload {texture_upload_index}")
            }),
            other => Err(format!(
                "descriptor image resource is not a texture upload: {other:?}"
            )),
        }
    }

    pub fn fallback(&self) -> Result<GltfMaterialTextureFallback, String> {
        match self {
            Self::Fallback { fallback } => Ok(*fallback),
            other => Err(format!(
                "descriptor image resource is not a fallback texture: {other:?}"
            )),
        }
    }

    pub fn resolve_resource<'a, T>(
        &self,
        texture_uploads: &'a [T],
        fallback_resource: impl FnOnce(GltfMaterialTextureFallback) -> &'a T,
    ) -> Result<&'a T, String> {
        match self {
            Self::TextureUpload { .. } => self.texture_upload_resource(texture_uploads),
            Self::Fallback { fallback } => Ok(fallback_resource(*fallback)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AshDrawableFramePlan {
    pub render_pass: AshRenderPassPlan,
    pub commands: Vec<AshCommandPlan>,
    pub skipped_draws: Vec<AshSkippedDraw>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshCommandBufferBeginPlan {
    pub flags: vk::CommandBufferUsageFlags,
}

impl AshCommandBufferBeginPlan {
    pub fn command_buffer_begin_info(self) -> vk::CommandBufferBeginInfo<'static> {
        vk::CommandBufferBeginInfo::default().flags(self.flags)
    }
}

pub const fn ash_command_buffer_begin_plan(
    flags: vk::CommandBufferUsageFlags,
) -> AshCommandBufferBeginPlan {
    AshCommandBufferBeginPlan { flags }
}

pub const fn ash_reusable_command_buffer_begin_plan() -> AshCommandBufferBeginPlan {
    ash_command_buffer_begin_plan(vk::CommandBufferUsageFlags::empty())
}

pub const fn ash_one_time_command_buffer_begin_plan() -> AshCommandBufferBeginPlan {
    ash_command_buffer_begin_plan(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshCommandPoolPlan {
    pub queue_family_index: u32,
    pub flags: vk::CommandPoolCreateFlags,
}

impl AshCommandPoolPlan {
    pub fn command_pool_create_info(self) -> vk::CommandPoolCreateInfo<'static> {
        vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.queue_family_index)
            .flags(self.flags)
    }
}

pub const fn ash_command_pool_plan(
    queue_family_index: u32,
    flags: vk::CommandPoolCreateFlags,
) -> AshCommandPoolPlan {
    AshCommandPoolPlan {
        queue_family_index,
        flags,
    }
}

pub const fn ash_resettable_command_pool_plan(queue_family_index: u32) -> AshCommandPoolPlan {
    ash_command_pool_plan(
        queue_family_index,
        vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshCommandBufferAllocationPlan {
    pub command_pool: vk::CommandPool,
    pub level: vk::CommandBufferLevel,
    pub command_buffer_count: u32,
}

impl AshCommandBufferAllocationPlan {
    pub fn command_buffer_allocate_info(&self) -> vk::CommandBufferAllocateInfo<'static> {
        vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(self.level)
            .command_buffer_count(self.command_buffer_count)
    }
}

pub const fn ash_command_buffer_allocation_plan(
    command_pool: vk::CommandPool,
    level: vk::CommandBufferLevel,
    command_buffer_count: u32,
) -> AshCommandBufferAllocationPlan {
    AshCommandBufferAllocationPlan {
        command_pool,
        level,
        command_buffer_count,
    }
}

pub const fn ash_primary_command_buffer_allocation_plan(
    command_pool: vk::CommandPool,
    command_buffer_count: u32,
) -> AshCommandBufferAllocationPlan {
    ash_command_buffer_allocation_plan(
        command_pool,
        vk::CommandBufferLevel::PRIMARY,
        command_buffer_count,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshFencePlan {
    pub flags: vk::FenceCreateFlags,
}

impl AshFencePlan {
    pub fn fence_create_info(self) -> vk::FenceCreateInfo<'static> {
        vk::FenceCreateInfo::default().flags(self.flags)
    }
}

pub const fn ash_fence_plan(flags: vk::FenceCreateFlags) -> AshFencePlan {
    AshFencePlan { flags }
}

pub const fn ash_unsignaled_fence_plan() -> AshFencePlan {
    ash_fence_plan(vk::FenceCreateFlags::empty())
}

pub const fn ash_signaled_fence_plan() -> AshFencePlan {
    ash_fence_plan(vk::FenceCreateFlags::SIGNALED)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AshSemaphorePlan;

impl AshSemaphorePlan {
    pub fn semaphore_create_info(self) -> vk::SemaphoreCreateInfo<'static> {
        vk::SemaphoreCreateInfo::default()
    }
}

pub const fn ash_binary_semaphore_plan() -> AshSemaphorePlan {
    AshSemaphorePlan
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshBufferResourcePlan {
    pub size: vk::DeviceSize,
    pub usage: vk::BufferUsageFlags,
    pub sharing_mode: vk::SharingMode,
    pub memory_property_flags: vk::MemoryPropertyFlags,
}

impl AshBufferResourcePlan {
    pub fn buffer_create_info(self) -> vk::BufferCreateInfo<'static> {
        vk::BufferCreateInfo::default()
            .size(self.size)
            .usage(self.usage)
            .sharing_mode(self.sharing_mode)
    }
}

pub fn ash_buffer_resource_plan(
    usage: vk::BufferUsageFlags,
    byte_len: usize,
    memory_property_flags: vk::MemoryPropertyFlags,
) -> AshBufferResourcePlan {
    AshBufferResourcePlan {
        size: byte_len.max(1) as vk::DeviceSize,
        usage,
        sharing_mode: vk::SharingMode::EXCLUSIVE,
        memory_property_flags,
    }
}

pub fn ash_host_visible_buffer_plan(
    usage: vk::BufferUsageFlags,
    byte_len: usize,
) -> AshBufferResourcePlan {
    ash_buffer_resource_plan(
        usage,
        byte_len,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshHostBufferPlan {
    pub size: vk::DeviceSize,
    pub usage: vk::BufferUsageFlags,
    pub sharing_mode: vk::SharingMode,
    pub memory_property_flags: vk::MemoryPropertyFlags,
}

impl AshHostBufferPlan {
    pub fn buffer_create_info(self) -> vk::BufferCreateInfo<'static> {
        vk::BufferCreateInfo::default()
            .size(self.size)
            .usage(self.usage)
            .sharing_mode(self.sharing_mode)
    }
}

pub fn ash_host_buffer_plan(usage: vk::BufferUsageFlags, byte_len: usize) -> AshHostBufferPlan {
    let plan = ash_host_visible_buffer_plan(usage | vk::BufferUsageFlags::TRANSFER_DST, byte_len);
    AshHostBufferPlan {
        size: plan.size,
        usage: plan.usage,
        sharing_mode: plan.sharing_mode,
        memory_property_flags: plan.memory_property_flags,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshImageResourcePlan {
    pub format: vk::Format,
    pub extent: vk::Extent3D,
    pub mip_levels: u32,
    pub usage: vk::ImageUsageFlags,
    pub aspect_mask: vk::ImageAspectFlags,
    pub image_type: vk::ImageType,
    pub view_type: vk::ImageViewType,
    pub array_layers: u32,
    pub samples: vk::SampleCountFlags,
    pub tiling: vk::ImageTiling,
    pub sharing_mode: vk::SharingMode,
    pub initial_layout: vk::ImageLayout,
    pub memory_property_flags: vk::MemoryPropertyFlags,
}

impl AshImageResourcePlan {
    pub fn image_create_info(self) -> vk::ImageCreateInfo<'static> {
        vk::ImageCreateInfo::default()
            .image_type(self.image_type)
            .format(self.format)
            .extent(self.extent)
            .mip_levels(self.mip_levels)
            .array_layers(self.array_layers)
            .samples(self.samples)
            .tiling(self.tiling)
            .usage(self.usage)
            .sharing_mode(self.sharing_mode)
            .initial_layout(self.initial_layout)
    }

    pub fn subresource_range(self) -> vk::ImageSubresourceRange {
        vk::ImageSubresourceRange::default()
            .aspect_mask(self.aspect_mask)
            .level_count(self.mip_levels)
            .layer_count(self.array_layers)
    }

    pub fn image_view_create_info(self, image: vk::Image) -> vk::ImageViewCreateInfo<'static> {
        vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(self.view_type)
            .format(self.format)
            .subresource_range(self.subresource_range())
    }
}

pub fn ash_2d_image_resource_plan(
    format: vk::Format,
    extent: vk::Extent3D,
    mip_levels: u32,
    usage: vk::ImageUsageFlags,
    aspect_mask: vk::ImageAspectFlags,
) -> AshImageResourcePlan {
    AshImageResourcePlan {
        format,
        extent,
        mip_levels: mip_levels.max(1),
        usage,
        aspect_mask,
        image_type: vk::ImageType::TYPE_2D,
        view_type: vk::ImageViewType::TYPE_2D,
        array_layers: 1,
        samples: vk::SampleCountFlags::TYPE_1,
        tiling: vk::ImageTiling::OPTIMAL,
        sharing_mode: vk::SharingMode::EXCLUSIVE,
        initial_layout: vk::ImageLayout::UNDEFINED,
        memory_property_flags: vk::MemoryPropertyFlags::DEVICE_LOCAL,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshImageViewPlan {
    pub format: vk::Format,
    pub aspect_mask: vk::ImageAspectFlags,
    pub view_type: vk::ImageViewType,
    pub base_mip_level: u32,
    pub level_count: u32,
    pub base_array_layer: u32,
    pub layer_count: u32,
}

impl AshImageViewPlan {
    pub fn subresource_range(self) -> vk::ImageSubresourceRange {
        vk::ImageSubresourceRange::default()
            .aspect_mask(self.aspect_mask)
            .base_mip_level(self.base_mip_level)
            .level_count(self.level_count)
            .base_array_layer(self.base_array_layer)
            .layer_count(self.layer_count)
    }

    pub fn image_view_create_info(self, image: vk::Image) -> vk::ImageViewCreateInfo<'static> {
        vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(self.view_type)
            .format(self.format)
            .subresource_range(self.subresource_range())
    }
}

pub const fn ash_2d_image_view_plan(
    format: vk::Format,
    aspect_mask: vk::ImageAspectFlags,
) -> AshImageViewPlan {
    AshImageViewPlan {
        format,
        aspect_mask,
        view_type: vk::ImageViewType::TYPE_2D,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshMemoryAllocationPlan {
    pub allocation_size: vk::DeviceSize,
    pub memory_type_index: u32,
}

impl AshMemoryAllocationPlan {
    pub fn memory_allocate_info(self) -> vk::MemoryAllocateInfo<'static> {
        vk::MemoryAllocateInfo::default()
            .allocation_size(self.allocation_size)
            .memory_type_index(self.memory_type_index)
    }
}

pub const fn ash_memory_allocation_plan(
    requirements: vk::MemoryRequirements,
    memory_type_index: u32,
) -> AshMemoryAllocationPlan {
    AshMemoryAllocationPlan {
        allocation_size: requirements.size,
        memory_type_index,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AshQueueSubmitPlan {
    pub command_buffers: Vec<vk::CommandBuffer>,
    pub fence: vk::Fence,
}

impl AshQueueSubmitPlan {
    pub fn submit_info(&self) -> vk::SubmitInfo<'_> {
        vk::SubmitInfo::default().command_buffers(&self.command_buffers)
    }

    pub fn submit_infos(&self) -> [vk::SubmitInfo<'_>; 1] {
        [self.submit_info()]
    }

    pub const fn wait_fences(&self) -> [vk::Fence; 1] {
        [self.fence]
    }
}

pub fn ash_queue_submit_plan(
    command_buffers: Vec<vk::CommandBuffer>,
    fence: vk::Fence,
) -> AshQueueSubmitPlan {
    AshQueueSubmitPlan {
        command_buffers,
        fence,
    }
}

#[derive(Clone)]
pub struct AshRenderPassBeginPlan {
    pub render_pass: vk::RenderPass,
    pub framebuffer: vk::Framebuffer,
    pub render_area: vk::Rect2D,
    pub clear_values: Vec<vk::ClearValue>,
}

impl fmt::Debug for AshRenderPassBeginPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AshRenderPassBeginPlan")
            .field("render_pass", &self.render_pass)
            .field("framebuffer", &self.framebuffer)
            .field("render_area", &self.render_area)
            .field("clear_value_count", &self.clear_values.len())
            .finish()
    }
}

impl AshRenderPassBeginPlan {
    pub fn render_pass_begin_info(&self) -> vk::RenderPassBeginInfo<'_> {
        vk::RenderPassBeginInfo::default()
            .render_pass(self.render_pass)
            .framebuffer(self.framebuffer)
            .render_area(self.render_area)
            .clear_values(&self.clear_values)
    }
}

pub fn ash_render_pass_begin_plan(
    plan: &AshRenderPassPlan,
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
) -> AshRenderPassBeginPlan {
    let mut clear_values = Vec::with_capacity(2);
    clear_values.push(vk::ClearValue {
        color: vk::ClearColorValue {
            float32: plan.color_clear,
        },
    });
    if let Some(depth_clear) = plan.depth_stencil_clear {
        clear_values.push(vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: depth_clear.depth,
                stencil: depth_clear.stencil,
            },
        });
    }
    ash_render_pass_begin_plan_from_clear_values(
        render_pass,
        framebuffer,
        plan.render_area,
        clear_values,
    )
}

pub fn ash_render_pass_begin_plan_from_clear_values(
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    render_area: vk::Rect2D,
    clear_values: Vec<vk::ClearValue>,
) -> AshRenderPassBeginPlan {
    AshRenderPassBeginPlan {
        render_pass,
        framebuffer,
        render_area,
        clear_values,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AshDrawableFrameOptions {
    pub color_clear: [f32; 4],
    pub depth_stencil_clear: Option<AshDepthStencilClear>,
}

impl Default for AshDrawableFrameOptions {
    fn default() -> Self {
        Self {
            color_clear: [0.0, 0.0, 0.0, 1.0],
            depth_stencil_clear: Some(AshDepthStencilClear {
                depth: 1.0,
                stencil: 0,
            }),
        }
    }
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

impl Default for AshDepthStencilClear {
    fn default() -> Self {
        Self {
            depth: 1.0,
            stencil: 0,
        }
    }
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

impl AshCommandPlan {
    pub fn vk_graphics_pipeline(&self, pipelines: &[vk::Pipeline]) -> Result<vk::Pipeline, String> {
        match self {
            Self::BindGraphicsPipeline { pipeline_index } => {
                pipelines.get(*pipeline_index).copied().ok_or_else(|| {
                    format!(
                        "drawable command references missing graphics pipeline {pipeline_index}"
                    )
                })
            }
            other => Err(format!(
                "drawable command is not a graphics-pipeline bind: {other:?}"
            )),
        }
    }

    pub fn bind_graphics_pipeline_command(
        &self,
        pipelines: &[vk::Pipeline],
    ) -> Result<AshBindGraphicsPipelineCommand, String> {
        Ok(AshBindGraphicsPipelineCommand {
            bind_point: vk::PipelineBindPoint::GRAPHICS,
            pipeline: self.vk_graphics_pipeline(pipelines)?,
        })
    }

    pub fn vk_pipeline_layout(
        &self,
        pipeline_plans: &[AshGraphicsPipelinePlan],
        pipeline_layouts: &[vk::PipelineLayout],
    ) -> Result<vk::PipelineLayout, String> {
        match self {
            Self::BindDescriptorSet { pipeline_index, .. } => {
                let pipeline = pipeline_plans.get(*pipeline_index).ok_or_else(|| {
                    format!("drawable command references missing pipeline plan {pipeline_index}")
                })?;
                pipeline_layouts
                    .get(pipeline.descriptor_set_index)
                    .copied()
                    .ok_or_else(|| {
                        format!(
                            "drawable command references missing pipeline layout {}",
                            pipeline.descriptor_set_index
                        )
                    })
            }
            other => Err(format!(
                "drawable command is not a descriptor-set bind: {other:?}"
            )),
        }
    }

    pub fn vk_descriptor_set(
        &self,
        descriptor_sets: &[vk::DescriptorSet],
    ) -> Result<vk::DescriptorSet, String> {
        match self {
            Self::BindDescriptorSet {
                descriptor_set_index,
                ..
            } => descriptor_sets
                .get(*descriptor_set_index)
                .copied()
                .ok_or_else(|| {
                    format!(
                        "drawable command references missing descriptor set {descriptor_set_index}"
                    )
                }),
            other => Err(format!(
                "drawable command is not a descriptor-set bind: {other:?}"
            )),
        }
    }

    pub fn bind_descriptor_set_command(
        &self,
        pipeline_plans: &[AshGraphicsPipelinePlan],
        pipeline_layouts: &[vk::PipelineLayout],
        descriptor_sets: &[vk::DescriptorSet],
    ) -> Result<AshBindDescriptorSetCommand, String> {
        Ok(AshBindDescriptorSetCommand {
            bind_point: vk::PipelineBindPoint::GRAPHICS,
            layout: self.vk_pipeline_layout(pipeline_plans, pipeline_layouts)?,
            first_set: 0,
            descriptor_sets: [self.vk_descriptor_set(descriptor_sets)?],
            dynamic_offsets: [],
        })
    }

    pub fn vertex_buffer_resource<'a, T>(&self, buffers: &'a [T]) -> Result<&'a T, String> {
        match self {
            Self::BindVertexBuffer { buffer_index, .. } => {
                buffers.get(*buffer_index).ok_or_else(|| {
                    format!("drawable command references missing vertex buffer {buffer_index}")
                })
            }
            other => Err(format!(
                "drawable command is not a vertex-buffer bind: {other:?}"
            )),
        }
    }

    pub fn bind_vertex_buffer_command(
        &self,
        buffer: vk::Buffer,
    ) -> Result<AshBindVertexBufferCommand, String> {
        match self {
            Self::BindVertexBuffer {
                binding, offset, ..
            } => Ok(AshBindVertexBufferCommand {
                first_binding: *binding,
                buffers: [buffer],
                offsets: [*offset],
            }),
            other => Err(format!(
                "drawable command is not a vertex-buffer bind: {other:?}"
            )),
        }
    }

    pub fn index_buffer_resource<'a, T>(&self, buffers: &'a [T]) -> Result<&'a T, String> {
        match self {
            Self::BindIndexBuffer { buffer_index, .. } => {
                buffers.get(*buffer_index).ok_or_else(|| {
                    format!("drawable command references missing index buffer {buffer_index}")
                })
            }
            other => Err(format!(
                "drawable command is not an index-buffer bind: {other:?}"
            )),
        }
    }

    pub fn bind_index_buffer_command(
        &self,
        buffer: vk::Buffer,
    ) -> Result<AshBindIndexBufferCommand, String> {
        match self {
            Self::BindIndexBuffer {
                offset, index_type, ..
            } => Ok(AshBindIndexBufferCommand {
                buffer,
                offset: *offset,
                index_type: *index_type,
            }),
            other => Err(format!(
                "drawable command is not an index-buffer bind: {other:?}"
            )),
        }
    }

    pub fn draw_indexed_args(&self) -> Result<AshDrawIndexedCommand, String> {
        match self {
            Self::DrawIndexed {
                index_count,
                instance_count,
                first_index,
                vertex_offset,
                first_instance,
                ..
            } => Ok(AshDrawIndexedCommand {
                index_count: *index_count,
                instance_count: *instance_count,
                first_index: *first_index,
                vertex_offset: *vertex_offset,
                first_instance: *first_instance,
            }),
            other => Err(format!(
                "drawable command is not an indexed draw: {other:?}"
            )),
        }
    }

    pub fn resolve_record_command<Buffer, BufferHandle>(
        &self,
        resources: AshCommandRecordResources<'_, Buffer>,
        access: AshCommandRecordHandleAccess<BufferHandle>,
    ) -> Result<AshResolvedCommand, String>
    where
        BufferHandle: FnOnce(&Buffer) -> vk::Buffer,
    {
        match self {
            Self::BindGraphicsPipeline { .. } => Ok(AshResolvedCommand::BindGraphicsPipeline(
                self.bind_graphics_pipeline_command(resources.pipelines)?,
            )),
            Self::BindDescriptorSet { .. } => Ok(AshResolvedCommand::BindDescriptorSet(
                self.bind_descriptor_set_command(
                    resources.pipeline_plans,
                    resources.pipeline_layouts,
                    resources.descriptor_sets,
                )?,
            )),
            Self::BindVertexBuffer { .. } => {
                let buffer = self.vertex_buffer_resource(resources.buffers)?;
                Ok(AshResolvedCommand::BindVertexBuffer(
                    self.bind_vertex_buffer_command((access.buffer)(buffer))?,
                ))
            }
            Self::BindIndexBuffer { .. } => {
                let buffer = self.index_buffer_resource(resources.buffers)?;
                Ok(AshResolvedCommand::BindIndexBuffer(
                    self.bind_index_buffer_command((access.buffer)(buffer))?,
                ))
            }
            Self::DrawIndexed { .. } => {
                Ok(AshResolvedCommand::DrawIndexed(self.draw_indexed_args()?))
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AshCommandRecordResources<'a, Buffer> {
    pub pipeline_plans: &'a [AshGraphicsPipelinePlan],
    pub pipelines: &'a [vk::Pipeline],
    pub pipeline_layouts: &'a [vk::PipelineLayout],
    pub descriptor_sets: &'a [vk::DescriptorSet],
    pub buffers: &'a [Buffer],
}

impl<'a, Buffer> AshCommandRecordResources<'a, Buffer> {
    pub const fn new(
        pipeline_plans: &'a [AshGraphicsPipelinePlan],
        pipelines: &'a [vk::Pipeline],
        pipeline_layouts: &'a [vk::PipelineLayout],
        descriptor_sets: &'a [vk::DescriptorSet],
        buffers: &'a [Buffer],
    ) -> Self {
        Self {
            pipeline_plans,
            pipelines,
            pipeline_layouts,
            descriptor_sets,
            buffers,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AshCommandRecordHandleAccess<BufferHandle> {
    pub buffer: BufferHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshResolvedCommand {
    BindGraphicsPipeline(AshBindGraphicsPipelineCommand),
    BindDescriptorSet(AshBindDescriptorSetCommand),
    BindVertexBuffer(AshBindVertexBufferCommand),
    BindIndexBuffer(AshBindIndexBufferCommand),
    DrawIndexed(AshDrawIndexedCommand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshBindGraphicsPipelineCommand {
    pub bind_point: vk::PipelineBindPoint,
    pub pipeline: vk::Pipeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshBindDescriptorSetCommand {
    pub bind_point: vk::PipelineBindPoint,
    pub layout: vk::PipelineLayout,
    pub first_set: u32,
    pub descriptor_sets: [vk::DescriptorSet; 1],
    pub dynamic_offsets: [u32; 0],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshBindVertexBufferCommand {
    pub first_binding: u32,
    pub buffers: [vk::Buffer; 1],
    pub offsets: [vk::DeviceSize; 1],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshBindIndexBufferCommand {
    pub buffer: vk::Buffer,
    pub offset: vk::DeviceSize,
    pub index_type: vk::IndexType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshDrawIndexedCommand {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Debug)]
pub struct AshGraphicsPipelineStatePlan {
    pub descriptor_set_index: usize,
    pub vertex_binding: vk::VertexInputBindingDescription,
    pub vertex_attributes: Vec<vk::VertexInputAttributeDescription>,
    pub topology: vk::PrimitiveTopology,
    pub primitive_restart_enable: bool,
    pub viewport: vk::Viewport,
    pub scissor: vk::Rect2D,
    pub polygon_mode: vk::PolygonMode,
    pub cull_mode: vk::CullModeFlags,
    pub front_face: vk::FrontFace,
    pub line_width: f32,
    pub rasterization_samples: vk::SampleCountFlags,
    pub depth_test_enable: bool,
    pub depth_write_enable: bool,
    pub depth_compare_op: vk::CompareOp,
    pub color_blend_attachment: vk::PipelineColorBlendAttachmentState,
}

pub fn ash_vertex_attribute_description(
    attribute: &AshVertexAttributePlan,
) -> vk::VertexInputAttributeDescription {
    vk::VertexInputAttributeDescription {
        location: attribute.location,
        binding: attribute.binding,
        format: attribute.format,
        offset: attribute.offset,
    }
}

pub fn ash_graphics_pipeline_state_plan(
    pipeline: &AshGraphicsPipelinePlan,
    extent: vk::Extent2D,
) -> AshGraphicsPipelineStatePlan {
    AshGraphicsPipelineStatePlan {
        descriptor_set_index: pipeline.descriptor_set_index,
        vertex_binding: vk::VertexInputBindingDescription {
            binding: 0,
            stride: pipeline.vertex_stride,
            input_rate: vk::VertexInputRate::VERTEX,
        },
        vertex_attributes: pipeline
            .vertex_attributes
            .iter()
            .map(ash_vertex_attribute_description)
            .collect(),
        topology: pipeline.key.topology,
        primitive_restart_enable: false,
        viewport: vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        },
        scissor: vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        },
        polygon_mode: vk::PolygonMode::FILL,
        cull_mode: pipeline.key.cull_mode,
        front_face: pipeline.key.front_face,
        line_width: 1.0,
        rasterization_samples: vk::SampleCountFlags::TYPE_1,
        depth_test_enable: pipeline.key.depth_test_enable,
        depth_write_enable: pipeline.key.depth_write_enable,
        depth_compare_op: pipeline.key.depth_compare_op,
        color_blend_attachment: vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(pipeline.key.blend_enable)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            ),
    }
}

pub fn ash_position_color_pipeline_state_plan(
    extent: vk::Extent2D,
    vertex_stride: u32,
    position_offset: u32,
    color_offset: u32,
) -> AshGraphicsPipelineStatePlan {
    AshGraphicsPipelineStatePlan {
        descriptor_set_index: 0,
        vertex_binding: vk::VertexInputBindingDescription {
            binding: 0,
            stride: vertex_stride,
            input_rate: vk::VertexInputRate::VERTEX,
        },
        vertex_attributes: vec![
            vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: position_offset,
            },
            vk::VertexInputAttributeDescription {
                location: 1,
                binding: 0,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: color_offset,
            },
        ],
        topology: vk::PrimitiveTopology::TRIANGLE_LIST,
        primitive_restart_enable: false,
        viewport: vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        },
        scissor: vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        },
        polygon_mode: vk::PolygonMode::FILL,
        cull_mode: vk::CullModeFlags::NONE,
        front_face: vk::FrontFace::COUNTER_CLOCKWISE,
        line_width: 1.0,
        rasterization_samples: vk::SampleCountFlags::TYPE_1,
        depth_test_enable: true,
        depth_write_enable: true,
        depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
        color_blend_attachment: vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            ),
    }
}

#[derive(Clone, Debug)]
pub struct AshGraphicsPipelineCreateInfoPlan<'a> {
    pub shader_stages: [vk::PipelineShaderStageCreateInfo<'a>; 2],
    pub state: AshGraphicsPipelineStatePlan,
    pub layout: vk::PipelineLayout,
    pub render_pass: vk::RenderPass,
    pub subpass: u32,
}

impl AshGraphicsPipelineCreateInfoPlan<'_> {
    pub fn with_graphics_pipeline_create_info<R>(
        &self,
        f: impl FnOnce(vk::GraphicsPipelineCreateInfo<'_>) -> R,
    ) -> R {
        let vertex_binding = [self.state.vertex_binding];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_binding)
            .vertex_attribute_descriptions(&self.state.vertex_attributes);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(self.state.topology)
            .primitive_restart_enable(self.state.primitive_restart_enable);
        let viewport = [self.state.viewport];
        let scissor = [self.state.scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewport)
            .scissors(&scissor);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(self.state.polygon_mode)
            .cull_mode(self.state.cull_mode)
            .front_face(self.state.front_face)
            .line_width(self.state.line_width);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(self.state.rasterization_samples);
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(self.state.depth_test_enable)
            .depth_write_enable(self.state.depth_write_enable)
            .depth_compare_op(self.state.depth_compare_op);
        let color_attachment = [self.state.color_blend_attachment];
        let color_blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&color_attachment);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&self.shader_stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend)
            .layout(self.layout)
            .render_pass(self.render_pass)
            .subpass(self.subpass);
        f(info)
    }
}

pub fn ash_graphics_pipeline_create_info_plan<'a>(
    pipeline: &AshGraphicsPipelinePlan,
    extent: vk::Extent2D,
    shader_stages: [vk::PipelineShaderStageCreateInfo<'a>; 2],
    pipeline_layouts: &[vk::PipelineLayout],
    render_pass: vk::RenderPass,
) -> Result<AshGraphicsPipelineCreateInfoPlan<'a>, String> {
    let state = ash_graphics_pipeline_state_plan(pipeline, extent);
    let layout = pipeline_layouts
        .get(state.descriptor_set_index)
        .copied()
        .ok_or_else(|| {
            format!(
                "pipeline descriptor set index {} is out of range for {} pipeline layouts",
                state.descriptor_set_index,
                pipeline_layouts.len()
            )
        })?;
    Ok(AshGraphicsPipelineCreateInfoPlan {
        shader_stages,
        state,
        layout,
        render_pass,
        subpass: 0,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshSwapchainSurfacePlan {
    pub format: vk::SurfaceFormatKHR,
    pub present_mode: vk::PresentModeKHR,
    pub extent: vk::Extent2D,
    pub image_count: u32,
    pub pre_transform: vk::SurfaceTransformFlagsKHR,
    pub composite_alpha: vk::CompositeAlphaFlagsKHR,
    pub image_usage: vk::ImageUsageFlags,
}

impl AshSwapchainSurfacePlan {
    pub fn create_info(
        self,
        surface: vk::SurfaceKHR,
        old_swapchain: vk::SwapchainKHR,
    ) -> vk::SwapchainCreateInfoKHR<'static> {
        vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(self.image_count)
            .image_format(self.format.format)
            .image_color_space(self.format.color_space)
            .image_extent(self.extent)
            .image_array_layers(1)
            .image_usage(self.image_usage)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(self.pre_transform)
            .composite_alpha(self.composite_alpha)
            .present_mode(self.present_mode)
            .clipped(true)
            .old_swapchain(old_swapchain)
    }
}

pub const fn ash_swapchain_composite_alpha_candidates() -> [vk::CompositeAlphaFlagsKHR; 4] {
    [
        vk::CompositeAlphaFlagsKHR::OPAQUE,
        vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::INHERIT,
    ]
}

pub fn ash_select_swapchain_composite_alpha(
    supported: vk::CompositeAlphaFlagsKHR,
) -> Result<vk::CompositeAlphaFlagsKHR, String> {
    ash_swapchain_composite_alpha_candidates()
        .into_iter()
        .find(|candidate| supported.contains(*candidate))
        .ok_or_else(|| "surface reports no supported composite alpha mode".to_owned())
}

pub fn ash_validate_swapchain_image_usage(
    supported: vk::ImageUsageFlags,
    required: vk::ImageUsageFlags,
) -> Result<vk::ImageUsageFlags, String> {
    if supported.contains(required) {
        Ok(required)
    } else {
        Err(format!(
            "surface image usage {supported:?} does not support required {required:?}"
        ))
    }
}

pub fn ash_swapchain_surface_plan(
    capabilities: vk::SurfaceCapabilitiesKHR,
    formats: &[vk::SurfaceFormatKHR],
    present_modes: &[vk::PresentModeKHR],
    requested_extent: vk::Extent2D,
) -> Result<AshSwapchainSurfacePlan, String> {
    let format = formats
        .iter()
        .copied()
        .find(|format| {
            matches!(
                format.format,
                vk::Format::B8G8R8A8_UNORM | vk::Format::R8G8B8A8_UNORM
            )
        })
        .or_else(|| formats.first().copied())
        .ok_or_else(|| "surface reports no formats".to_owned())?;
    let present_mode = present_modes
        .iter()
        .copied()
        .find(|mode| *mode == vk::PresentModeKHR::MAILBOX)
        .unwrap_or(vk::PresentModeKHR::FIFO);
    let extent = if capabilities.current_extent.width != u32::MAX {
        capabilities.current_extent
    } else {
        vk::Extent2D {
            width: requested_extent.width.clamp(
                capabilities.min_image_extent.width,
                capabilities.max_image_extent.width,
            ),
            height: requested_extent.height.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            ),
        }
    };
    let desired = capabilities.min_image_count.saturating_add(1);
    let image_count = if capabilities.max_image_count == 0 {
        desired
    } else {
        desired.min(capabilities.max_image_count)
    };
    let image_usage = ash_validate_swapchain_image_usage(
        capabilities.supported_usage_flags,
        vk::ImageUsageFlags::COLOR_ATTACHMENT,
    )?;
    let composite_alpha =
        ash_select_swapchain_composite_alpha(capabilities.supported_composite_alpha)?;
    Ok(AshSwapchainSurfacePlan {
        format,
        present_mode,
        extent,
        image_count,
        pre_transform: capabilities.current_transform,
        composite_alpha,
        image_usage,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonRenderTargetCacheKey {
    pub extent: [u32; 2],
}

impl AshMtoonRenderTargetCacheKey {
    pub const fn from_extent(extent: vk::Extent2D) -> Self {
        Self {
            extent: [extent.width, extent.height],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonShaderCacheKey {
    pub vertex_entry: String,
    pub fragment_entry: String,
    pub vertex_spirv_hash: Option<u64>,
    pub fragment_spirv_hash: Option<u64>,
    pub clip_space_policy: AshClipSpacePolicy,
    pub spirv_coordinate_adjustment: AshSpirvCoordinateAdjustment,
}

impl AshMtoonShaderCacheKey {
    pub fn from_entries(
        vertex_entry: impl Into<String>,
        fragment_entry: impl Into<String>,
    ) -> Self {
        let abi = AshMtoonWgslShaderAbi::default();
        Self {
            vertex_entry: vertex_entry.into(),
            fragment_entry: fragment_entry.into(),
            vertex_spirv_hash: None,
            fragment_spirv_hash: None,
            clip_space_policy: abi.clip_space_policy,
            spirv_coordinate_adjustment: abi.spirv_coordinate_adjustment,
        }
    }

    pub fn from_spirv_words(
        vertex_entry: impl Into<String>,
        fragment_entry: impl Into<String>,
        vertex_words: &[u32],
        fragment_words: &[u32],
    ) -> Self {
        let abi = AshMtoonWgslShaderAbi::default();
        Self {
            vertex_entry: vertex_entry.into(),
            fragment_entry: fragment_entry.into(),
            vertex_spirv_hash: Some(ash_mtoon_spirv_words_hash(vertex_words)),
            fragment_spirv_hash: Some(ash_mtoon_spirv_words_hash(fragment_words)),
            clip_space_policy: abi.clip_space_policy,
            spirv_coordinate_adjustment: abi.spirv_coordinate_adjustment,
        }
    }

    pub fn with_coordinate_policy(
        mut self,
        clip_space_policy: AshClipSpacePolicy,
        spirv_coordinate_adjustment: AshSpirvCoordinateAdjustment,
    ) -> Self {
        self.clip_space_policy = clip_space_policy;
        self.spirv_coordinate_adjustment = spirv_coordinate_adjustment;
        self
    }
}

impl Default for AshMtoonShaderCacheKey {
    fn default() -> Self {
        let abi = AshMtoonWgslShaderAbi::default();
        Self::from_entries(abi.vertex_entry, abi.fragment_entry)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonRendererCacheKeys {
    pub pipeline: AshMtoonPipelineCacheKey,
    pub descriptor_sets: AshMtoonDescriptorSetCacheKey,
    pub samplers: AshMtoonSamplerCacheKey,
    pub buffers: AshMtoonBufferCacheKey,
    pub uniforms: AshMtoonUniformCacheKey,
    pub textures: AshMtoonTextureCacheKey,
}

impl AshMtoonRendererCacheKeys {
    pub fn from_frame(
        frame: &AshRendererFrame,
        render_target: AshMtoonRenderTargetCacheKey,
        shader: AshMtoonShaderCacheKey,
    ) -> Self {
        let descriptor_set_layouts = ash_mtoon_descriptor_set_layout_cache_records(frame);
        let pipeline = AshMtoonPipelineCacheKey {
            render_target,
            shader,
            descriptor_set_layouts: descriptor_set_layouts.clone(),
            pipelines: frame
                .pipelines
                .iter()
                .map(AshMtoonGraphicsPipelineCacheKey::from_pipeline)
                .collect(),
        };
        Self {
            pipeline,
            descriptor_sets: AshMtoonDescriptorSetCacheKey {
                descriptor_set_layouts,
            },
            samplers: AshMtoonSamplerCacheKey::from_frame(frame),
            buffers: AshMtoonBufferCacheKey::from_frame(frame),
            uniforms: AshMtoonUniformCacheKey::from_frame(frame),
            textures: AshMtoonTextureCacheKey::from_frame(frame),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonPipelineCacheKey {
    pub render_target: AshMtoonRenderTargetCacheKey,
    pub shader: AshMtoonShaderCacheKey,
    pub descriptor_set_layouts: Vec<Vec<AshMtoonDescriptorLayoutBindingCacheKey>>,
    pub pipelines: Vec<AshMtoonGraphicsPipelineCacheKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonDescriptorSetCacheKey {
    pub descriptor_set_layouts: Vec<Vec<AshMtoonDescriptorLayoutBindingCacheKey>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonDescriptorLayoutBindingCacheKey {
    pub binding: u32,
    pub descriptor_type: i32,
    pub stage_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonGraphicsPipelineCacheKey {
    pub material: MaterialRef,
    pub pipeline_plan_index: usize,
    pub descriptor_set_index: usize,
    pub pass: AshMtoonPass,
    pub render_order: i32,
    pub phase_order: i32,
    pub topology: i32,
    pub cull_mode: u32,
    pub front_face: i32,
    pub depth_test_enable: bool,
    pub depth_write_enable: bool,
    pub depth_compare_op: i32,
    pub blend_enable: bool,
    pub vertex_stride: u32,
    pub vertex_attributes: Vec<AshMtoonVertexAttributeCacheKey>,
    pub color_format: i32,
    pub depth_format: Option<i32>,
}

impl AshMtoonGraphicsPipelineCacheKey {
    pub fn from_pipeline(pipeline: &AshGraphicsPipelinePlan) -> Self {
        Self {
            material: pipeline.material,
            pipeline_plan_index: pipeline.pipeline_plan_index,
            descriptor_set_index: pipeline.descriptor_set_index,
            pass: pipeline.key.pass,
            render_order: pipeline.key.render_order,
            phase_order: pipeline.key.phase_order,
            topology: pipeline.key.topology.as_raw(),
            cull_mode: pipeline.key.cull_mode.as_raw(),
            front_face: pipeline.key.front_face.as_raw(),
            depth_test_enable: pipeline.key.depth_test_enable,
            depth_write_enable: pipeline.key.depth_write_enable,
            depth_compare_op: pipeline.key.depth_compare_op.as_raw(),
            blend_enable: pipeline.key.blend_enable,
            vertex_stride: pipeline.vertex_stride,
            vertex_attributes: pipeline
                .vertex_attributes
                .iter()
                .map(AshMtoonVertexAttributeCacheKey::from_attribute)
                .collect(),
            color_format: pipeline.color_format.as_raw(),
            depth_format: pipeline.depth_format.map(vk::Format::as_raw),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonVertexAttributeCacheKey {
    pub location: u32,
    pub binding: u32,
    pub format: i32,
    pub offset: u32,
}

impl AshMtoonVertexAttributeCacheKey {
    pub fn from_attribute(attribute: &AshVertexAttributePlan) -> Self {
        Self {
            location: attribute.location,
            binding: attribute.binding,
            format: attribute.format.as_raw(),
            offset: attribute.offset,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonSamplerCacheKey {
    pub samplers: Vec<AshMtoonSamplerBindingCacheKey>,
}

impl AshMtoonSamplerCacheKey {
    pub fn from_frame(frame: &AshRendererFrame) -> Self {
        Self {
            samplers: ash_sampler_resource_plans(frame)
                .into_iter()
                .map(|plan| AshMtoonSamplerBindingCacheKey {
                    descriptor_set_index: plan.descriptor_set_index,
                    binding: plan.binding,
                    descriptor_type: plan.descriptor_type.as_raw(),
                    sampler: AshMtoonSamplerPlanCacheKey::from_sampler(plan.sampler),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonSamplerBindingCacheKey {
    pub descriptor_set_index: usize,
    pub binding: u32,
    pub descriptor_type: i32,
    pub sampler: AshMtoonSamplerPlanCacheKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonSamplerPlanCacheKey {
    pub mag_filter: i32,
    pub min_filter: i32,
    pub mipmap_mode: i32,
    pub address_mode_u: i32,
    pub address_mode_v: i32,
    pub min_lod_bits: u32,
    pub max_lod_bits: u32,
    pub normal_map_decode: bool,
}

impl AshMtoonSamplerPlanCacheKey {
    pub fn from_sampler(sampler: AshSamplerPlan) -> Self {
        Self {
            mag_filter: sampler.mag_filter.as_raw(),
            min_filter: sampler.min_filter.as_raw(),
            mipmap_mode: sampler.mipmap_mode.as_raw(),
            address_mode_u: sampler.address_mode_u.as_raw(),
            address_mode_v: sampler.address_mode_v.as_raw(),
            min_lod_bits: sampler.min_lod.to_bits(),
            max_lod_bits: sampler.max_lod.to_bits(),
            normal_map_decode: sampler.normal_map_decode,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonBufferCacheKey {
    pub buffers: Vec<AshMtoonBufferResourceCacheKey>,
}

impl AshMtoonBufferCacheKey {
    pub fn from_frame(frame: &AshRendererFrame) -> Self {
        Self {
            buffers: frame
                .buffers
                .iter()
                .map(|buffer| AshMtoonBufferResourceCacheKey {
                    role: buffer.role,
                    usage: buffer.usage.as_raw(),
                    stride: buffer.stride,
                    count: buffer.count,
                    byte_len: buffer.bytes.len(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonBufferResourceCacheKey {
    pub role: AshBufferRole,
    pub usage: u32,
    pub stride: u32,
    pub count: u32,
    pub byte_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonUniformCacheKey {
    pub uniforms: Vec<AshMtoonUniformResourceCacheKey>,
}

impl AshMtoonUniformCacheKey {
    pub fn from_frame(frame: &AshRendererFrame) -> Self {
        Self {
            uniforms: frame
                .uniforms
                .iter()
                .map(|uniform| AshMtoonUniformResourceCacheKey {
                    scope: uniform.scope,
                    binding: uniform.binding,
                    byte_len: uniform.bytes.len(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonUniformResourceCacheKey {
    pub scope: AshUniformScope,
    pub binding: u32,
    pub byte_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonTextureCacheKey {
    pub textures: Vec<AshMtoonTextureResourceCacheKey>,
}

impl AshMtoonTextureCacheKey {
    pub fn from_frame(frame: &AshRendererFrame) -> Self {
        Self {
            textures: frame
                .textures
                .iter()
                .map(|texture| AshMtoonTextureResourceCacheKey {
                    texture: texture.upload.texture,
                    color_space: texture.upload.color_space,
                    format: texture.upload.format.as_raw(),
                    extent: [
                        texture.upload.extent.width,
                        texture.upload.extent.height,
                        texture.upload.extent.depth,
                    ],
                    image_usage: texture.image_usage.as_raw(),
                    image_layout_after_upload: texture.image_layout_after_upload.as_raw(),
                    aspect_mask: texture.aspect_mask.as_raw(),
                    rgba_len: texture.upload.rgba.len(),
                    rgba_hash: ash_mtoon_bytes_hash(&texture.upload.rgba),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AshMtoonTextureResourceCacheKey {
    pub texture: Option<TextureRef>,
    pub color_space: GltfMaterialTextureColorSpace,
    pub format: i32,
    pub extent: [u32; 3],
    pub image_usage: u32,
    pub image_layout_after_upload: i32,
    pub aspect_mask: u32,
    pub rgba_len: usize,
    pub rgba_hash: u64,
}

pub fn ash_mtoon_renderer_cache_keys(
    frame: &AshRendererFrame,
    extent: vk::Extent2D,
    shader: AshMtoonShaderCacheKey,
) -> AshMtoonRendererCacheKeys {
    AshMtoonRendererCacheKeys::from_frame(
        frame,
        AshMtoonRenderTargetCacheKey::from_extent(extent),
        shader,
    )
}

#[derive(Clone, Debug)]
pub struct AshMtoonMaterializationPlan {
    pub render_target: AshMtoonRenderTargetCacheKey,
    pub shader: AshMtoonShaderCacheKey,
    pub cache_keys: AshMtoonRendererCacheKeys,
    pub resource_manifest: AshRendererResourceManifest,
    pub descriptor_pool: AshDescriptorPoolPlan,
    pub descriptor_set_layouts: Vec<AshDescriptorSetLayoutPlan>,
    pub pipeline_layouts: Vec<AshPipelineLayoutPlan>,
    pub descriptor_set_allocation: AshDescriptorSetAllocationPlan,
    pub sampler_resources: Vec<AshSamplerResourcePlan>,
    pub descriptor_writes: Vec<AshDescriptorWritePlan>,
    pub drawable: AshDrawableFramePlan,
}

impl AshMtoonMaterializationPlan {
    pub fn persistent_handle_resource_count(&self) -> usize {
        self.resource_manifest.persistent_handle_resource_count()
    }

    pub fn frame_dynamic_resource_count(&self) -> usize {
        self.resource_manifest.dynamic_resource_count()
    }

    pub fn draw_command_count(&self) -> usize {
        self.drawable.commands.len()
    }
}

pub fn ash_mtoon_materialization_plan(
    frame: &AshRendererFrame,
    extent: vk::Extent2D,
    shader: AshMtoonShaderCacheKey,
) -> Result<AshMtoonMaterializationPlan, String> {
    ash_mtoon_materialization_plan_with_options(
        frame,
        extent,
        shader,
        AshDrawableFrameOptions::default(),
    )
}

pub fn ash_mtoon_materialization_plan_with_options(
    frame: &AshRendererFrame,
    extent: vk::Extent2D,
    shader: AshMtoonShaderCacheKey,
    drawable_options: AshDrawableFrameOptions,
) -> Result<AshMtoonMaterializationPlan, String> {
    let render_target = AshMtoonRenderTargetCacheKey::from_extent(extent);
    let cache_keys = AshMtoonRendererCacheKeys::from_frame(frame, render_target, shader.clone());
    let descriptor_set_layouts = ash_descriptor_set_layout_plans(frame);
    let pipeline_layouts = ash_pipeline_layout_plans(&descriptor_set_layouts);
    let descriptor_set_allocation = ash_descriptor_set_allocation_plan(&descriptor_set_layouts);
    Ok(AshMtoonMaterializationPlan {
        render_target,
        shader,
        cache_keys,
        resource_manifest: ash_renderer_resource_manifest(frame),
        descriptor_pool: ash_descriptor_pool_plan(frame),
        descriptor_set_layouts,
        pipeline_layouts,
        descriptor_set_allocation,
        sampler_resources: ash_sampler_resource_plans(frame),
        descriptor_writes: ash_descriptor_write_plans(frame)?,
        drawable: ash_drawable_frame_from_renderer_frame_with_options(
            frame,
            extent,
            drawable_options,
        ),
    })
}

pub fn ash_mtoon_spirv_words_hash(words: &[u32]) -> u64 {
    words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .fold(FNV_OFFSET_BASIS_64, ash_mtoon_fnv1a_byte)
}

pub fn ash_mtoon_bytes_hash(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .copied()
        .fold(FNV_OFFSET_BASIS_64, ash_mtoon_fnv1a_byte)
}

const FNV_OFFSET_BASIS_64: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME_64: u64 = 0x0000_0100_0000_01b3;

fn ash_mtoon_fnv1a_byte(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME_64)
}

fn ash_mtoon_descriptor_set_layout_cache_records(
    frame: &AshRendererFrame,
) -> Vec<Vec<AshMtoonDescriptorLayoutBindingCacheKey>> {
    frame
        .descriptor_sets
        .iter()
        .map(|set| {
            set.bindings
                .iter()
                .map(|binding| AshMtoonDescriptorLayoutBindingCacheKey {
                    binding: binding.binding,
                    descriptor_type: binding.descriptor_type.as_raw(),
                    stage_flags: binding.stage_flags.as_raw(),
                })
                .collect()
        })
        .collect()
}

pub fn ash_renderer_resource_manifest(frame: &AshRendererFrame) -> AshRendererResourceManifest {
    let buffers = frame
        .buffers
        .iter()
        .enumerate()
        .map(|(index, buffer)| AshRendererBufferResource {
            index,
            role: buffer.role,
            usage: buffer.usage,
            stride: buffer.stride,
            count: buffer.count,
            byte_len: buffer.bytes.len(),
            handle_lifetime: AshRendererResourceLifetime::Persistent,
            lifetime: AshRendererResourceLifetime::FrameDynamic,
        })
        .collect();
    let textures = frame
        .textures
        .iter()
        .enumerate()
        .map(|(index, texture)| AshRendererTextureResource {
            index,
            texture: texture.upload.texture,
            color_space: texture.upload.color_space,
            format: texture.upload.format,
            extent: texture.upload.extent,
            image_usage: texture.image_usage,
            image_layout_after_upload: texture.image_layout_after_upload,
            aspect_mask: texture.aspect_mask,
            byte_len: texture.upload.rgba.len(),
            handle_lifetime: AshRendererResourceLifetime::Persistent,
            lifetime: AshRendererResourceLifetime::Persistent,
        })
        .collect();
    let uniforms = frame
        .uniforms
        .iter()
        .enumerate()
        .map(|(index, uniform)| AshRendererUniformResource {
            index,
            scope: uniform.scope,
            binding: uniform.binding,
            byte_len: uniform.bytes.len(),
            handle_lifetime: AshRendererResourceLifetime::Persistent,
            lifetime: AshRendererResourceLifetime::FrameDynamic,
        })
        .collect();
    let samplers = ash_sampler_resource_plans(frame)
        .into_iter()
        .map(|plan| AshRendererSamplerResource {
            descriptor_set_index: plan.descriptor_set_index,
            binding: plan.binding,
            descriptor_type: plan.descriptor_type,
            sampler: Some(plan.sampler),
            handle_lifetime: AshRendererResourceLifetime::Persistent,
            lifetime: AshRendererResourceLifetime::Persistent,
        })
        .collect();
    let descriptor_set_layouts = frame
        .descriptor_sets
        .iter()
        .enumerate()
        .map(
            |(descriptor_set_index, set)| AshRendererDescriptorSetLayoutResource {
                descriptor_set_index,
                material: set.material,
                pipeline_plan_index: set.pipeline_plan_index,
                bindings: set
                    .bindings
                    .iter()
                    .map(|binding| AshDescriptorBindingLayoutKey {
                        binding: binding.binding,
                        descriptor_type: binding.descriptor_type,
                        stage_flags: binding.stage_flags,
                    })
                    .collect(),
                handle_lifetime: AshRendererResourceLifetime::Persistent,
                lifetime: AshRendererResourceLifetime::Persistent,
            },
        )
        .collect();
    let descriptor_sets = frame
        .descriptor_sets
        .iter()
        .enumerate()
        .map(|(index, set)| AshRendererDescriptorSetResource {
            index,
            material: set.material,
            pipeline_plan_index: set.pipeline_plan_index,
            bindings: set
                .bindings
                .iter()
                .map(|binding| AshRendererDescriptorBindingResource {
                    binding: binding.binding,
                    descriptor_type: binding.descriptor_type,
                    uniform_upload_index: binding.uniform_upload_index,
                    texture_upload_index: binding.texture_upload_index,
                    buffer_upload_index: binding.buffer_upload_index,
                    lifetime: ash_descriptor_binding_lifetime(binding.descriptor_type),
                })
                .collect(),
            handle_lifetime: AshRendererResourceLifetime::Persistent,
            lifetime: AshRendererResourceLifetime::FrameDynamic,
        })
        .collect();
    let pipelines = frame
        .pipelines
        .iter()
        .enumerate()
        .map(|(index, pipeline)| AshRendererPipelineResource {
            index,
            material: pipeline.material,
            pipeline_plan_index: pipeline.pipeline_plan_index,
            descriptor_set_index: pipeline.descriptor_set_index,
            key: pipeline.key,
            vertex_stride: pipeline.vertex_stride,
            vertex_attributes: pipeline.vertex_attributes.clone(),
            color_format: pipeline.color_format,
            depth_format: pipeline.depth_format,
            handle_lifetime: AshRendererResourceLifetime::Persistent,
            lifetime: AshRendererResourceLifetime::Persistent,
        })
        .collect();
    AshRendererResourceManifest {
        buffers,
        textures,
        uniforms,
        samplers,
        descriptor_set_layouts,
        descriptor_sets,
        pipelines,
    }
}

pub fn ash_descriptor_pool_plan(frame: &AshRendererFrame) -> AshDescriptorPoolPlan {
    let descriptor_binding_count = |descriptor_type| {
        frame
            .descriptor_sets
            .iter()
            .flat_map(|set| &set.bindings)
            .filter(|binding| binding.descriptor_type == descriptor_type)
            .count()
            .max(1) as u32
    };
    let sampler_count = frame
        .descriptor_sets
        .iter()
        .flat_map(|set| &set.bindings)
        .filter(|binding| {
            matches!(
                binding.descriptor_type,
                vk::DescriptorType::COMBINED_IMAGE_SAMPLER | vk::DescriptorType::SAMPLER
            )
        })
        .count()
        .max(1) as u32;
    AshDescriptorPoolPlan {
        max_sets: frame.descriptor_sets.len().max(1) as u32,
        pool_sizes: vec![
            AshDescriptorPoolSizePlan {
                descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: descriptor_binding_count(vk::DescriptorType::UNIFORM_BUFFER),
            },
            AshDescriptorPoolSizePlan {
                descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: sampler_count,
            },
            AshDescriptorPoolSizePlan {
                descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: descriptor_binding_count(vk::DescriptorType::SAMPLED_IMAGE),
            },
            AshDescriptorPoolSizePlan {
                descriptor_type: vk::DescriptorType::SAMPLER,
                descriptor_count: sampler_count,
            },
            AshDescriptorPoolSizePlan {
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                descriptor_count: descriptor_binding_count(vk::DescriptorType::STORAGE_BUFFER),
            },
        ],
    }
}

pub fn ash_descriptor_set_layout_plans(
    frame: &AshRendererFrame,
) -> Vec<AshDescriptorSetLayoutPlan> {
    frame
        .descriptor_sets
        .iter()
        .enumerate()
        .map(|(descriptor_set_index, set)| AshDescriptorSetLayoutPlan {
            descriptor_set_index,
            material: set.material,
            pipeline_plan_index: set.pipeline_plan_index,
            bindings: set
                .bindings
                .iter()
                .map(|binding| AshDescriptorBindingLayoutKey {
                    binding: binding.binding,
                    descriptor_type: binding.descriptor_type,
                    stage_flags: binding.stage_flags,
                })
                .collect(),
        })
        .collect()
}

pub fn ash_pipeline_layout_plans(
    descriptor_set_layouts: &[AshDescriptorSetLayoutPlan],
) -> Vec<AshPipelineLayoutPlan> {
    descriptor_set_layouts
        .iter()
        .map(|layout| AshPipelineLayoutPlan {
            descriptor_set_layout_index: layout.descriptor_set_index,
        })
        .collect()
}

pub fn ash_descriptor_set_allocation_plan(
    descriptor_set_layouts: &[AshDescriptorSetLayoutPlan],
) -> AshDescriptorSetAllocationPlan {
    AshDescriptorSetAllocationPlan {
        descriptor_set_layout_indices: descriptor_set_layouts
            .iter()
            .map(|layout| layout.descriptor_set_index)
            .collect(),
    }
}

pub fn ash_sampler_resource_plans(frame: &AshRendererFrame) -> Vec<AshSamplerResourcePlan> {
    frame
        .descriptor_sets
        .iter()
        .enumerate()
        .flat_map(|(descriptor_set_index, set)| {
            set.bindings
                .iter()
                .filter(|binding| {
                    matches!(
                        binding.descriptor_type,
                        vk::DescriptorType::COMBINED_IMAGE_SAMPLER | vk::DescriptorType::SAMPLER
                    )
                })
                .map(move |binding| (descriptor_set_index, binding))
        })
        .enumerate()
        .map(
            |(sampler_index, (descriptor_set_index, binding))| AshSamplerResourcePlan {
                sampler_index,
                descriptor_set_index,
                binding: binding.binding,
                descriptor_type: binding.descriptor_type,
                sampler: binding.sampler.unwrap_or_default(),
            },
        )
        .collect()
}

pub fn ash_descriptor_write_plans(
    frame: &AshRendererFrame,
) -> Result<Vec<AshDescriptorWritePlan>, String> {
    let mut sampler_index = 0usize;
    let mut plans = Vec::new();
    for (descriptor_set_index, set) in frame.descriptor_sets.iter().enumerate() {
        for binding in &set.bindings {
            let resource = match binding.descriptor_type {
                vk::DescriptorType::UNIFORM_BUFFER => {
                    let uniform_upload_index = binding.uniform_upload_index.ok_or_else(|| {
                        format!(
                            "descriptor set {descriptor_set_index} binding {} is missing a uniform upload index",
                            binding.binding
                        )
                    })?;
                    if uniform_upload_index >= frame.uniforms.len() {
                        return Err(format!(
                            "descriptor set {descriptor_set_index} binding {} references missing uniform upload {uniform_upload_index}",
                            binding.binding
                        ));
                    }
                    AshDescriptorWriteResource::UniformBuffer {
                        uniform_upload_index,
                    }
                }
                vk::DescriptorType::STORAGE_BUFFER => {
                    let buffer_upload_index = binding.buffer_upload_index.ok_or_else(|| {
                        format!(
                            "descriptor set {descriptor_set_index} binding {} is missing a storage buffer upload index",
                            binding.binding
                        )
                    })?;
                    if buffer_upload_index >= frame.buffers.len() {
                        return Err(format!(
                            "descriptor set {descriptor_set_index} binding {} references missing storage buffer upload {buffer_upload_index}",
                            binding.binding
                        ));
                    }
                    AshDescriptorWriteResource::StorageBuffer {
                        buffer_upload_index,
                    }
                }
                vk::DescriptorType::COMBINED_IMAGE_SAMPLER => {
                    let current_sampler = sampler_index;
                    sampler_index = sampler_index.saturating_add(1);
                    AshDescriptorWriteResource::CombinedImageSampler {
                        sampler_index: current_sampler,
                        image: ash_descriptor_image_resource(frame, binding)?,
                    }
                }
                vk::DescriptorType::SAMPLED_IMAGE => AshDescriptorWriteResource::SampledImage {
                    image: ash_descriptor_image_resource(frame, binding)?,
                },
                vk::DescriptorType::SAMPLER => {
                    let current_sampler = sampler_index;
                    sampler_index = sampler_index.saturating_add(1);
                    AshDescriptorWriteResource::Sampler {
                        sampler_index: current_sampler,
                    }
                }
                other => {
                    return Err(format!(
                        "unsupported ash descriptor type in renderer frame: {other:?}"
                    ));
                }
            };
            plans.push(AshDescriptorWritePlan {
                descriptor_set_index,
                binding: binding.binding,
                descriptor_type: binding.descriptor_type,
                resource,
            });
        }
    }
    Ok(plans)
}

fn ash_descriptor_image_resource(
    frame: &AshRendererFrame,
    binding: &AshResolvedDescriptorBinding,
) -> Result<AshDescriptorImageResource, String> {
    if let Some(texture_upload_index) = binding.texture_upload_index {
        if texture_upload_index >= frame.textures.len() {
            return Err(format!(
                "descriptor binding {} references missing texture upload {texture_upload_index}",
                binding.binding
            ));
        }
        Ok(AshDescriptorImageResource::TextureUpload {
            texture_upload_index,
        })
    } else {
        Ok(AshDescriptorImageResource::Fallback {
            fallback: ash_texture_fallback_for_binding(binding.binding)
                .unwrap_or(GltfMaterialTextureFallback::White),
        })
    }
}

fn ash_descriptor_binding_lifetime(
    descriptor_type: vk::DescriptorType,
) -> AshRendererResourceLifetime {
    match descriptor_type {
        vk::DescriptorType::SAMPLED_IMAGE
        | vk::DescriptorType::SAMPLER
        | vk::DescriptorType::COMBINED_IMAGE_SAMPLER => AshRendererResourceLifetime::Persistent,
        _ => AshRendererResourceLifetime::FrameDynamic,
    }
}

pub fn ash_drawable_frame_from_renderer_frame(
    frame: &AshRendererFrame,
    extent: vk::Extent2D,
) -> AshDrawableFramePlan {
    ash_drawable_frame_from_renderer_frame_with_options(
        frame,
        extent,
        AshDrawableFrameOptions::default(),
    )
}

pub fn ash_drawable_frame_from_renderer_frame_with_options(
    frame: &AshRendererFrame,
    extent: vk::Extent2D,
    options: AshDrawableFrameOptions,
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
            color_clear: options.color_clear,
            depth_stencil_clear: options.depth_stencil_clear,
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
                            binding
                                if binding == ash_mtoon_uv_uniform_binding()
                                    || binding == ash_mtoon_wgsl_uv_uniform_binding() =>
                            {
                                pipeline_plan_index * ASH_MTOON_UNIFORMS_PER_PIPELINE + 1
                            }
                            binding
                                if binding == ash_mtoon_render_extra_binding()
                                    || binding == ash_mtoon_wgsl_render_extra_binding() =>
                            {
                                pipeline_plan_index * ASH_MTOON_UNIFORMS_PER_PIPELINE + 2
                            }
                            binding
                                if binding == ash_mtoon_scene_binding()
                                    || binding == ash_mtoon_wgsl_scene_binding() =>
                            {
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
                    render_options.descriptor_binding_model,
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
            render_options.descriptor_binding_model,
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

pub const fn ash_mtoon_wgsl_scene_binding() -> u32 {
    30
}

pub const fn ash_mtoon_wgsl_uv_uniform_binding() -> u32 {
    31
}

pub const fn ash_mtoon_wgsl_render_extra_binding() -> u32 {
    32
}

pub const fn ash_mtoon_wgsl_owner_sample_override_binding() -> u32 {
    40
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

pub const fn ash_mtoon_texture_sampler_binding(slot: MtoonTextureSlot) -> u32 {
    mtoon_gpu_sampler_binding_number(ash_mtoon_texture_slot_index(slot))
}

pub const fn ash_mtoon_sampled_image_binding(slot: MtoonTextureSlot) -> u32 {
    mtoon_gpu_texture_binding_number(ash_mtoon_texture_slot_index(slot))
}

pub const fn ash_material_sampled_image_binding(slot: GltfMaterialTextureSlot) -> u32 {
    match slot {
        GltfMaterialTextureSlot::Base => ash_mtoon_sampled_image_binding(MtoonTextureSlot::Main),
        GltfMaterialTextureSlot::Shade => {
            ash_mtoon_sampled_image_binding(MtoonTextureSlot::ShadeMultiply)
        }
        GltfMaterialTextureSlot::ShadingShift => {
            ash_mtoon_sampled_image_binding(MtoonTextureSlot::ShadingShift)
        }
        GltfMaterialTextureSlot::Normal => {
            ash_mtoon_sampled_image_binding(MtoonTextureSlot::Normal)
        }
        GltfMaterialTextureSlot::Matcap => {
            ash_mtoon_sampled_image_binding(MtoonTextureSlot::Matcap)
        }
        GltfMaterialTextureSlot::Rim => {
            ash_mtoon_sampled_image_binding(MtoonTextureSlot::RimMultiply)
        }
        GltfMaterialTextureSlot::UvAnimationMask => {
            ash_mtoon_sampled_image_binding(MtoonTextureSlot::UvAnimationMask)
        }
        GltfMaterialTextureSlot::Emissive => 17,
        GltfMaterialTextureSlot::Occlusion => 19,
    }
}

pub const fn ash_material_sampler_binding(slot: GltfMaterialTextureSlot) -> u32 {
    match slot {
        GltfMaterialTextureSlot::Base => ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Main),
        GltfMaterialTextureSlot::Shade => {
            ash_mtoon_texture_sampler_binding(MtoonTextureSlot::ShadeMultiply)
        }
        GltfMaterialTextureSlot::ShadingShift => {
            ash_mtoon_texture_sampler_binding(MtoonTextureSlot::ShadingShift)
        }
        GltfMaterialTextureSlot::Normal => {
            ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Normal)
        }
        GltfMaterialTextureSlot::Matcap => {
            ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Matcap)
        }
        GltfMaterialTextureSlot::Rim => {
            ash_mtoon_texture_sampler_binding(MtoonTextureSlot::RimMultiply)
        }
        GltfMaterialTextureSlot::UvAnimationMask => {
            ash_mtoon_texture_sampler_binding(MtoonTextureSlot::UvAnimationMask)
        }
        GltfMaterialTextureSlot::Emissive => 18,
        GltfMaterialTextureSlot::Occlusion => 20,
    }
}

const fn ash_mtoon_texture_slot_index(slot: MtoonTextureSlot) -> usize {
    match slot {
        MtoonTextureSlot::Main => 0,
        MtoonTextureSlot::ShadeMultiply => 1,
        MtoonTextureSlot::ShadingShift => 2,
        MtoonTextureSlot::Normal => 3,
        MtoonTextureSlot::Matcap => 4,
        MtoonTextureSlot::RimMultiply => 5,
        MtoonTextureSlot::OutlineWidth => 6,
        MtoonTextureSlot::UvAnimationMask => 7,
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
        1 | 2 | 6 | 7 | 8 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 => {
            Some(GltfMaterialTextureFallback::White)
        }
        3 | 5 | 9 | 10 => Some(GltfMaterialTextureFallback::Black),
        4 => Some(GltfMaterialTextureFallback::NeutralNormal),
        _ => None,
    }
}

fn descriptor_bindings(
    textures: &[GltfTextureData],
    slots: GltfMaterialTextureSlots,
    binding_model: AshDescriptorBindingModel,
) -> Vec<AshDescriptorBindingPlan> {
    let plan = slots.binding_plan();
    let mut result = Vec::with_capacity(match binding_model {
        AshDescriptorBindingModel::CombinedImageSampler => 15,
        AshDescriptorBindingModel::SeparateImageSampler => 25,
    });
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
            .flat_map(|binding| descriptor_bindings_for_texture(textures, binding, binding_model)),
    );
    result.extend(descriptor_bindings_for_mtoon_texture(
        textures,
        MtoonTextureSlot::OutlineWidth,
        slots.outline_width,
        GltfMaterialTextureColorSpace::Linear,
        MtoonSamplerHint::LinearRepeat,
        binding_model,
    ));
    result.extend(
        ASH_GLTF_TEXTURE_SLOTS_AFTER_OUTLINE
            .iter()
            .filter_map(|slot| plan.binding(*slot))
            .flat_map(|binding| descriptor_bindings_for_texture(textures, binding, binding_model)),
    );
    result.push(AshDescriptorBindingPlan {
        binding: ash_scene_binding_for_model(binding_model),
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: None,
    });
    result.push(AshDescriptorBindingPlan {
        binding: ash_uv_uniform_binding_for_model(binding_model),
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: None,
    });
    result.push(AshDescriptorBindingPlan {
        binding: ash_render_extra_binding_for_model(binding_model),
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: None,
    });
    result.push(AshDescriptorBindingPlan {
        binding: ash_owner_sample_override_binding_for_model(binding_model),
        descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        color_space: GltfMaterialTextureColorSpace::Linear,
        sampler: None,
    });
    result
}

const fn ash_scene_binding_for_model(model: AshDescriptorBindingModel) -> u32 {
    match model {
        AshDescriptorBindingModel::CombinedImageSampler => ash_mtoon_scene_binding(),
        AshDescriptorBindingModel::SeparateImageSampler => ash_mtoon_wgsl_scene_binding(),
    }
}

const fn ash_uv_uniform_binding_for_model(model: AshDescriptorBindingModel) -> u32 {
    match model {
        AshDescriptorBindingModel::CombinedImageSampler => ash_mtoon_uv_uniform_binding(),
        AshDescriptorBindingModel::SeparateImageSampler => ash_mtoon_wgsl_uv_uniform_binding(),
    }
}

const fn ash_render_extra_binding_for_model(model: AshDescriptorBindingModel) -> u32 {
    match model {
        AshDescriptorBindingModel::CombinedImageSampler => ash_mtoon_render_extra_binding(),
        AshDescriptorBindingModel::SeparateImageSampler => ash_mtoon_wgsl_render_extra_binding(),
    }
}

const fn ash_owner_sample_override_binding_for_model(model: AshDescriptorBindingModel) -> u32 {
    match model {
        AshDescriptorBindingModel::CombinedImageSampler => ash_owner_sample_override_binding(),
        AshDescriptorBindingModel::SeparateImageSampler => {
            ash_mtoon_wgsl_owner_sample_override_binding()
        }
    }
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

fn descriptor_bindings_for_texture(
    textures: &[GltfTextureData],
    binding: GltfMaterialTextureBinding,
    binding_model: AshDescriptorBindingModel,
) -> Vec<AshDescriptorBindingPlan> {
    descriptor_bindings_for_texture_slot(
        textures,
        binding.slot,
        binding.texture,
        binding.color_space,
        sampler_hint_for_material_slot(binding.slot),
        binding_model,
    )
}

fn descriptor_bindings_for_mtoon_texture(
    textures: &[GltfTextureData],
    slot: MtoonTextureSlot,
    texture: Option<usize>,
    color_space: GltfMaterialTextureColorSpace,
    sampler_hint: MtoonSamplerHint,
    binding_model: AshDescriptorBindingModel,
) -> Vec<AshDescriptorBindingPlan> {
    let texture_ref = texture.map(TextureRef);
    let sampler = Some(sampler_plan_for_texture(textures, texture, sampler_hint));
    match binding_model {
        AshDescriptorBindingModel::CombinedImageSampler => vec![AshDescriptorBindingPlan {
            binding: ash_mtoon_texture_binding(slot),
            descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            stage_flags: vk::ShaderStageFlags::FRAGMENT,
            texture: texture_ref,
            color_space,
            sampler,
        }],
        AshDescriptorBindingModel::SeparateImageSampler => vec![
            AshDescriptorBindingPlan {
                binding: ash_mtoon_sampled_image_binding(slot),
                descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
                texture: texture_ref,
                color_space,
                sampler: None,
            },
            AshDescriptorBindingPlan {
                binding: ash_mtoon_texture_sampler_binding(slot),
                descriptor_type: vk::DescriptorType::SAMPLER,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
                texture: None,
                color_space,
                sampler,
            },
        ],
    }
}

fn descriptor_bindings_for_texture_slot(
    textures: &[GltfTextureData],
    slot: GltfMaterialTextureSlot,
    texture: Option<usize>,
    color_space: GltfMaterialTextureColorSpace,
    sampler_hint: MtoonSamplerHint,
    binding_model: AshDescriptorBindingModel,
) -> Vec<AshDescriptorBindingPlan> {
    let texture_ref = texture.map(TextureRef);
    let sampler = Some(sampler_plan_for_texture(textures, texture, sampler_hint));
    match binding_model {
        AshDescriptorBindingModel::CombinedImageSampler => vec![AshDescriptorBindingPlan {
            binding: ash_material_texture_binding(slot),
            descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            stage_flags: vk::ShaderStageFlags::FRAGMENT,
            texture: texture_ref,
            color_space,
            sampler,
        }],
        AshDescriptorBindingModel::SeparateImageSampler => vec![
            AshDescriptorBindingPlan {
                binding: ash_material_sampled_image_binding(slot),
                descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
                texture: texture_ref,
                color_space,
                sampler: None,
            },
            AshDescriptorBindingPlan {
                binding: ash_material_sampler_binding(slot),
                descriptor_type: vk::DescriptorType::SAMPLER,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
                texture: None,
                color_space,
                sampler,
            },
        ],
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
    fn fallback_texture_helpers_expose_stable_rgba_and_order() {
        assert_eq!(
            ASH_FALLBACK_TEXTURES,
            [
                GltfMaterialTextureFallback::White,
                GltfMaterialTextureFallback::Black,
                GltfMaterialTextureFallback::NeutralNormal,
            ]
        );
        assert_eq!(
            ash_fallback_texture_rgba(GltfMaterialTextureFallback::White),
            [255, 255, 255, 255]
        );
        assert_eq!(
            ash_fallback_texture_rgba(GltfMaterialTextureFallback::Black),
            [0, 0, 0, 255]
        );
        let normal = ash_fallback_texture_mip_level(GltfMaterialTextureFallback::NeutralNormal);
        assert_eq!(normal.width, 1);
        assert_eq!(normal.height, 1);
        assert_eq!(normal.rgba, vec![128, 128, 255, 255]);
        assert_eq!(
            AshSamplerPlan::default().sampler_create_info().max_lod,
            32.0
        );
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
            AshDescriptorBindingModel::CombinedImageSampler,
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
    fn descriptor_bindings_can_match_wgsl_texture_sampler_pairs() {
        let bindings = descriptor_bindings(
            &[],
            GltfMaterialTextureSlots {
                base: Some(0),
                normal: Some(3),
                outline_width: Some(6),
                emissive: Some(7),
                occlusion: Some(8),
                uv_animation_mask: Some(9),
                ..Default::default()
            },
            AshDescriptorBindingModel::SeparateImageSampler,
        );
        assert_eq!(bindings.len(), 25);
        assert_eq!(
            bindings[1].binding,
            ash_mtoon_sampled_image_binding(MtoonTextureSlot::Main)
        );
        assert_eq!(
            bindings[1].descriptor_type,
            vk::DescriptorType::SAMPLED_IMAGE
        );
        assert_eq!(
            bindings[2].binding,
            ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Main)
        );
        assert_eq!(bindings[2].descriptor_type, vk::DescriptorType::SAMPLER);
        let normal_sampler = bindings
            .iter()
            .find(|binding| {
                binding.binding == ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Normal)
            })
            .expect("normal sampler binding");
        assert!(normal_sampler.sampler.unwrap().normal_map_decode);
        assert!(bindings.iter().any(|binding| {
            binding.binding == ash_mtoon_sampled_image_binding(MtoonTextureSlot::OutlineWidth)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.binding == ash_material_sampled_image_binding(GltfMaterialTextureSlot::Emissive)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.binding
                == ash_material_sampled_image_binding(GltfMaterialTextureSlot::Occlusion)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.binding == ash_mtoon_sampled_image_binding(MtoonTextureSlot::UvAnimationMask)
        }));
        assert!(
            bindings
                .iter()
                .any(|binding| binding.binding == ash_mtoon_wgsl_scene_binding())
        );
        assert!(
            bindings
                .iter()
                .any(|binding| binding.binding == ash_mtoon_wgsl_uv_uniform_binding())
        );
        assert!(
            bindings
                .iter()
                .any(|binding| binding.binding == ash_mtoon_wgsl_render_extra_binding())
        );
        assert_eq!(
            ash_owner_sample_override_binding_for_model(
                AshDescriptorBindingModel::SeparateImageSampler
            ),
            ash_mtoon_wgsl_owner_sample_override_binding()
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
            clip_space_policy: AshClipSpacePolicy::CpuVulkanZeroToOneYDown,
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
    fn scene_options_carry_typed_clip_space_policy() {
        let cpu_vulkan = AshSceneOptions::default();
        let naga_adjusted = AshSceneOptions {
            clip_space_policy: AshClipSpacePolicy::NagaVulkanZeroToOneYDown,
            ..Default::default()
        };
        assert_eq!(
            cpu_vulkan.clip_space_policy.spirv_coordinate_adjustment(),
            AshSpirvCoordinateAdjustment::Disabled
        );
        assert_eq!(
            naga_adjusted
                .clip_space_policy
                .spirv_coordinate_adjustment(),
            AshSpirvCoordinateAdjustment::NagaWriter
        );
        assert_eq!(cpu_vulkan.projection().y_axis.y.signum(), -1.0);
        assert_eq!(naga_adjusted.projection().y_axis.y.signum(), 1.0);
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
    fn windowed_run_validation_matches_viewer_contract() {
        assert_eq!(
            AshWindowedRunValidation {
                simple_preview: true,
                require_cache_hits: true,
                frames_in_flight: 1,
                ..Default::default()
            }
            .validate(),
            Err("--require-cache-hits is only supported by the MToon renderer path".to_owned())
        );
        assert_eq!(
            AshWindowedRunValidation {
                require_resize_recreate: true,
                frames_in_flight: 1,
                ..Default::default()
            }
            .validate(),
            Err("--require-resize-recreate requires --resize-after-frames".to_owned())
        );
        assert_eq!(
            AshWindowedRunValidation {
                frames_in_flight: 0,
                ..Default::default()
            }
            .validate(),
            Err("--frames-in-flight must be at least 1".to_owned())
        );
        assert!(
            AshWindowedRunValidation {
                require_cache_hits: true,
                require_resize_recreate: true,
                resize_after_frames: Some(8),
                frames_in_flight: 2,
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn windowed_frame_sync_plan_counts_and_slots_are_stable() {
        let plan = AshWindowedFrameSyncPlan::new(2, 3).unwrap();

        assert_eq!(plan.semaphore_count(), 4);
        assert_eq!(plan.fence_count(), 2);
        assert_eq!(plan.image_fence_slots(), 3);
        assert_eq!(plan.frame_slot(1), Ok(1));
        assert_eq!(plan.next_frame_index(0), Ok(1));
        assert_eq!(plan.next_frame_index(1), Ok(0));
        assert_eq!(plan.image_index_to_slot(2), Ok(2));
    }

    #[test]
    fn windowed_frame_sync_selection_exposes_acquire_slots_and_payloads() {
        let plan = AshWindowedFrameSyncPlan::new(2, 3).unwrap();
        let selection = match plan
            .select_acquired_frame(
                1,
                AshSwapchainAcquireStatus::Acquired {
                    image_index: 2,
                    suboptimal: true,
                },
                &[vk::Fence::null(), vk::Fence::null(), vk::Fence::null()],
            )
            .unwrap()
        {
            AshWindowedFrameAcquirePlan::Acquired(selection) => selection,
            AshWindowedFrameAcquirePlan::NeedsRecreate => panic!("expected acquired frame"),
        };

        assert_eq!(selection.frame_slot, 1);
        assert_eq!(selection.swapchain_image_slot, 2);
        assert_eq!(selection.image_index, 2);
        assert!(selection.acquired_suboptimal);
        assert_eq!(selection.previous_image_fence, vk::Fence::null());
        assert_eq!(selection.next_frame_index, 0);

        let sync_handles = AshWindowedFrameSyncHandles {
            image_available: vk::Semaphore::null(),
            render_finished: vk::Semaphore::null(),
            in_flight: vk::Fence::null(),
        };
        let submit = selection.submit_plan(sync_handles, vk::CommandBuffer::null());
        assert_eq!(submit.wait_semaphore, sync_handles.image_available);
        assert_eq!(submit.signal_semaphore, sync_handles.render_finished);
        assert_eq!(submit.fence, sync_handles.in_flight);
        let present = selection.present_plan(submit.signal_semaphore, vk::SwapchainKHR::null());
        assert_eq!(present.image_index, 2);
    }

    #[test]
    fn windowed_frame_sync_plan_rejects_invalid_counts_and_indices() {
        assert_eq!(
            AshWindowedFrameSyncPlan::new(0, 3),
            Err("frames_in_flight must be at least 1".to_owned())
        );
        assert_eq!(
            AshWindowedFrameSyncPlan::new(2, 0),
            Err("swapchain_images must be at least 1".to_owned())
        );
        let plan = AshWindowedFrameSyncPlan::new(2, 3).unwrap();
        assert_eq!(
            plan.frame_slot(2),
            Err("current_frame 2 is outside frames_in_flight 2".to_owned())
        );
        assert_eq!(
            plan.image_index_to_slot(3),
            Err("swapchain image index 3 is outside swapchain_images 3".to_owned())
        );
        assert_eq!(
            plan.select_acquired_frame(
                0,
                AshSwapchainAcquireStatus::NeedsRecreate,
                &[vk::Fence::null()]
            ),
            Ok(AshWindowedFrameAcquirePlan::NeedsRecreate)
        );
        assert_eq!(
            plan.select_acquired_frame(
                0,
                AshSwapchainAcquireStatus::Acquired {
                    image_index: 2,
                    suboptimal: false,
                },
                &[vk::Fence::null()]
            ),
            Err("swapchain image slot 2 has no matching in-flight fence entry".to_owned())
        );
    }

    #[test]
    fn swapchain_acquire_status_classifies_recreate_and_errors() {
        assert_eq!(
            ash_classify_swapchain_acquire(Ok((7, true))),
            Ok(AshSwapchainAcquireStatus::Acquired {
                image_index: 7,
                suboptimal: true
            })
        );
        assert_eq!(
            ash_classify_swapchain_acquire(Err(vk::Result::ERROR_OUT_OF_DATE_KHR)),
            Ok(AshSwapchainAcquireStatus::NeedsRecreate)
        );
        assert_eq!(
            ash_classify_swapchain_acquire(Err(vk::Result::ERROR_DEVICE_LOST)),
            Err(vk::Result::ERROR_DEVICE_LOST)
        );
    }

    #[test]
    fn swapchain_present_status_combines_acquire_and_present_suboptimal() {
        assert_eq!(
            ash_classify_swapchain_present(false, Ok(false)),
            Ok(AshSwapchainPresentStatus::Presented)
        );
        assert_eq!(
            ash_classify_swapchain_present(true, Ok(false)),
            Ok(AshSwapchainPresentStatus::NeedsRecreate)
        );
        assert_eq!(
            ash_classify_swapchain_present(false, Ok(true)),
            Ok(AshSwapchainPresentStatus::NeedsRecreate)
        );
        assert_eq!(
            ash_classify_swapchain_present(false, Err(vk::Result::ERROR_OUT_OF_DATE_KHR)),
            Ok(AshSwapchainPresentStatus::NeedsRecreate)
        );
        assert_eq!(
            ash_classify_swapchain_present(false, Err(vk::Result::ERROR_DEVICE_LOST)),
            Err(vk::Result::ERROR_DEVICE_LOST)
        );
    }

    #[test]
    fn windowed_submit_and_present_plans_expose_vk_sync_inputs() {
        let submit = ash_windowed_submit_plan(
            vk::Semaphore::null(),
            vk::Semaphore::null(),
            vk::CommandBuffer::null(),
            vk::Fence::null(),
        );

        assert_eq!(submit.wait_semaphores(), [vk::Semaphore::null()]);
        assert_eq!(
            submit.wait_stages(),
            [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT]
        );
        assert_eq!(submit.command_buffers(), [vk::CommandBuffer::null()]);
        assert_eq!(submit.signal_semaphores(), [vk::Semaphore::null()]);
        assert_eq!(submit.fence, vk::Fence::null());
        let submit_info_plan = submit.submit_info_plan();
        let submit_info = submit_info_plan.submit_info();
        assert_eq!(
            submit_info.p_wait_dst_stage_mask,
            submit_info_plan.wait_stages.as_ptr()
        );
        assert_eq!(submit_info.wait_semaphore_count, 1);
        assert_eq!(submit_info.command_buffer_count, 1);
        assert_eq!(submit_info.signal_semaphore_count, 1);

        let present = ash_windowed_present_plan(vk::Semaphore::null(), vk::SwapchainKHR::null(), 4);
        assert_eq!(present.wait_semaphores(), [vk::Semaphore::null()]);
        assert_eq!(present.swapchains(), [vk::SwapchainKHR::null()]);
        assert_eq!(present.image_indices(), [4]);
        let present_info_plan = present.present_info_plan();
        let present_info = present_info_plan.present_info();
        assert_eq!(present_info.wait_semaphore_count, 1);
        assert_eq!(present_info.swapchain_count, 1);
        assert_eq!(
            present_info.p_image_indices,
            present_info_plan.image_indices.as_ptr()
        );
    }

    #[test]
    fn buffer_image_and_sampler_plans_expose_resource_create_infos() {
        let host_visible = ash_host_visible_buffer_plan(vk::BufferUsageFlags::VERTEX_BUFFER, 0);
        assert_eq!(host_visible.size, 1);
        assert_eq!(host_visible.usage, vk::BufferUsageFlags::VERTEX_BUFFER);
        assert_eq!(
            host_visible.memory_property_flags,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
        );
        let host_visible_info = host_visible.buffer_create_info();
        assert_eq!(host_visible_info.size, 1);
        assert_eq!(host_visible_info.usage, vk::BufferUsageFlags::VERTEX_BUFFER);
        assert_eq!(host_visible_info.sharing_mode, vk::SharingMode::EXCLUSIVE);

        let buffer = ash_host_buffer_plan(vk::BufferUsageFlags::UNIFORM_BUFFER, 0);
        assert_eq!(buffer.size, 1);
        assert!(buffer.usage.contains(vk::BufferUsageFlags::UNIFORM_BUFFER));
        assert!(buffer.usage.contains(vk::BufferUsageFlags::TRANSFER_DST));
        assert_eq!(
            buffer.memory_property_flags,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
        );
        let buffer_info = buffer.buffer_create_info();
        assert_eq!(buffer_info.size, 1);
        assert_eq!(buffer_info.usage, buffer.usage);
        assert_eq!(buffer_info.sharing_mode, vk::SharingMode::EXCLUSIVE);
        let memory = ash_memory_allocation_plan(
            vk::MemoryRequirements {
                size: 4096,
                alignment: 256,
                memory_type_bits: 0b101,
            },
            2,
        );
        assert_eq!(memory.allocation_size, 4096);
        assert_eq!(memory.memory_type_index, 2);
        let memory_info = memory.memory_allocate_info();
        assert_eq!(memory_info.allocation_size, 4096);
        assert_eq!(memory_info.memory_type_index, 2);

        let image = ash_2d_image_resource_plan(
            vk::Format::R8G8B8A8_UNORM,
            vk::Extent3D {
                width: 16,
                height: 8,
                depth: 1,
            },
            0,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            vk::ImageAspectFlags::COLOR,
        );
        assert_eq!(image.mip_levels, 1);
        assert_eq!(
            image.memory_property_flags,
            vk::MemoryPropertyFlags::DEVICE_LOCAL
        );
        let image_info = image.image_create_info();
        assert_eq!(image_info.image_type, vk::ImageType::TYPE_2D);
        assert_eq!(image_info.format, vk::Format::R8G8B8A8_UNORM);
        assert_eq!(image_info.mip_levels, 1);
        assert_eq!(image_info.array_layers, 1);
        assert_eq!(image_info.tiling, vk::ImageTiling::OPTIMAL);
        assert_eq!(image_info.initial_layout, vk::ImageLayout::UNDEFINED);
        let view_info = image.image_view_create_info(vk::Image::null());
        assert_eq!(view_info.image, vk::Image::null());
        assert_eq!(view_info.view_type, vk::ImageViewType::TYPE_2D);
        assert_eq!(
            view_info.subresource_range.aspect_mask,
            vk::ImageAspectFlags::COLOR
        );
        assert_eq!(view_info.subresource_range.level_count, 1);
        assert_eq!(view_info.subresource_range.layer_count, 1);
        let swapchain_view =
            ash_2d_image_view_plan(vk::Format::B8G8R8A8_UNORM, vk::ImageAspectFlags::COLOR);
        let swapchain_view_info = swapchain_view.image_view_create_info(vk::Image::null());
        assert_eq!(swapchain_view_info.image, vk::Image::null());
        assert_eq!(swapchain_view_info.format, vk::Format::B8G8R8A8_UNORM);
        assert_eq!(
            swapchain_view_info.subresource_range.aspect_mask,
            vk::ImageAspectFlags::COLOR
        );
        assert_eq!(swapchain_view_info.subresource_range.level_count, 1);
        assert_eq!(swapchain_view_info.subresource_range.layer_count, 1);

        let sampler = AshSamplerPlan {
            mag_filter: vk::Filter::NEAREST,
            min_filter: vk::Filter::LINEAR,
            mipmap_mode: vk::SamplerMipmapMode::NEAREST,
            address_mode_u: vk::SamplerAddressMode::CLAMP_TO_EDGE,
            address_mode_v: vk::SamplerAddressMode::MIRRORED_REPEAT,
            min_lod: 0.25,
            max_lod: 4.0,
            normal_map_decode: true,
        };
        let sampler_info = sampler.sampler_create_info();
        assert_eq!(sampler_info.mag_filter, vk::Filter::NEAREST);
        assert_eq!(sampler_info.min_filter, vk::Filter::LINEAR);
        assert_eq!(sampler_info.mipmap_mode, vk::SamplerMipmapMode::NEAREST);
        assert_eq!(
            sampler_info.address_mode_u,
            vk::SamplerAddressMode::CLAMP_TO_EDGE
        );
        assert_eq!(
            sampler_info.address_mode_v,
            vk::SamplerAddressMode::MIRRORED_REPEAT
        );
        assert_eq!(sampler_info.address_mode_w, vk::SamplerAddressMode::REPEAT);
        assert_eq!(sampler_info.min_lod, 0.25);
        assert_eq!(sampler_info.max_lod, 4.0);
    }

    #[test]
    fn command_buffer_and_render_pass_begin_plans_expose_vk_infos() {
        let attachments = [vk::ImageView::null(), vk::ImageView::null()];
        let framebuffer = ash_framebuffer_plan(vk::Extent2D {
            width: 1280,
            height: 720,
        });
        let framebuffer_info =
            framebuffer.framebuffer_create_info(vk::RenderPass::null(), &attachments);
        assert_eq!(framebuffer_info.render_pass, vk::RenderPass::null());
        assert_eq!(framebuffer_info.attachment_count, 2);
        assert_eq!(framebuffer_info.p_attachments, attachments.as_ptr());
        assert_eq!(framebuffer_info.width, 1280);
        assert_eq!(framebuffer_info.height, 720);
        assert_eq!(framebuffer_info.layers, 1);

        let reusable = ash_reusable_command_buffer_begin_plan();
        assert_eq!(reusable.flags, vk::CommandBufferUsageFlags::empty());
        assert_eq!(
            reusable.command_buffer_begin_info().flags,
            vk::CommandBufferUsageFlags::empty()
        );
        assert_eq!(
            ash_one_time_command_buffer_begin_plan()
                .command_buffer_begin_info()
                .flags,
            vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT
        );

        let command_pool = ash_resettable_command_pool_plan(7);
        let command_pool_info = command_pool.command_pool_create_info();
        assert_eq!(command_pool.queue_family_index, 7);
        assert_eq!(
            command_pool.flags,
            vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER
        );
        assert_eq!(command_pool_info.queue_family_index, 7);
        assert_eq!(
            command_pool_info.flags,
            vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER
        );

        let allocation = ash_primary_command_buffer_allocation_plan(vk::CommandPool::null(), 3);
        let allocation_info = allocation.command_buffer_allocate_info();
        assert_eq!(allocation.command_pool, vk::CommandPool::null());
        assert_eq!(allocation.level, vk::CommandBufferLevel::PRIMARY);
        assert_eq!(allocation.command_buffer_count, 3);
        assert_eq!(allocation_info.command_pool, vk::CommandPool::null());
        assert_eq!(allocation_info.level, vk::CommandBufferLevel::PRIMARY);
        assert_eq!(allocation_info.command_buffer_count, 3);

        let signaled_fence = ash_signaled_fence_plan();
        assert_eq!(signaled_fence.flags, vk::FenceCreateFlags::SIGNALED);
        assert_eq!(
            signaled_fence.fence_create_info().flags,
            vk::FenceCreateFlags::SIGNALED
        );
        assert_eq!(
            ash_unsignaled_fence_plan().fence_create_info().flags,
            vk::FenceCreateFlags::empty()
        );
        let semaphore_info = ash_binary_semaphore_plan().semaphore_create_info();
        assert_eq!(semaphore_info.flags, vk::SemaphoreCreateFlags::empty());

        let submit = ash_queue_submit_plan(
            vec![vk::CommandBuffer::null(), vk::CommandBuffer::null()],
            vk::Fence::null(),
        );
        let submit_info = submit.submit_info();
        assert_eq!(submit.command_buffers.len(), 2);
        assert_eq!(submit.wait_fences(), [vk::Fence::null()]);
        assert_eq!(submit_info.command_buffer_count, 2);

        let render_pass_plan = AshRenderPassPlan {
            render_area: vk::Rect2D {
                offset: vk::Offset2D { x: 4, y: 8 },
                extent: vk::Extent2D {
                    width: 320,
                    height: 180,
                },
            },
            color_format: vk::Format::B8G8R8A8_UNORM,
            depth_format: Some(vk::Format::D32_SFLOAT),
            color_clear: [0.1, 0.2, 0.3, 0.4],
            depth_stencil_clear: Some(AshDepthStencilClear {
                depth: 0.5,
                stencil: 7,
            }),
        };
        let begin = ash_render_pass_begin_plan(
            &render_pass_plan,
            vk::RenderPass::null(),
            vk::Framebuffer::null(),
        );
        let info = begin.render_pass_begin_info();

        assert_eq!(begin.clear_values.len(), 2);
        assert_eq!(info.render_pass, vk::RenderPass::null());
        assert_eq!(info.framebuffer, vk::Framebuffer::null());
        assert_eq!(info.render_area.extent.width, 320);
        assert_eq!(info.clear_value_count, 2);
        assert!(!info.p_clear_values.is_null());
    }

    #[test]
    fn texture_upload_command_plan_exposes_mip_bytes_regions_and_barriers() {
        let levels = vec![
            RgbaMipLevel {
                width: 4,
                height: 2,
                rgba: vec![1; 32],
            },
            RgbaMipLevel {
                width: 2,
                height: 1,
                rgba: vec![2; 8],
            },
            RgbaMipLevel {
                width: 1,
                height: 1,
                rgba: vec![3; 4],
            },
        ];

        let bytes = ash_texture_mip_upload_bytes(&levels);
        assert_eq!(bytes.len(), 44);
        assert_eq!(&bytes[0..4], &[1, 1, 1, 1]);
        assert_eq!(&bytes[32..36], &[2, 2, 2, 2]);
        assert_eq!(&bytes[40..44], &[3, 3, 3, 3]);

        let plan = ash_texture_upload_command_plan(&levels);
        assert_eq!(
            plan.subresource_range.aspect_mask,
            vk::ImageAspectFlags::COLOR
        );
        assert_eq!(plan.subresource_range.level_count, 3);
        assert_eq!(plan.subresource_range.layer_count, 1);
        assert_eq!(plan.copy_regions.len(), 3);
        assert_eq!(plan.copy_regions[0].buffer_offset, 0);
        assert_eq!(plan.copy_regions[1].buffer_offset, 32);
        assert_eq!(plan.copy_regions[2].buffer_offset, 40);
        assert_eq!(plan.copy_regions[2].image_subresource.mip_level, 2);

        let image = vk::Image::null();
        let to_transfer = plan.transfer_dst_barrier(image);
        let to_transfer_command = plan.transfer_dst_barrier_command(image);
        assert_eq!(to_transfer.dst_access_mask, vk::AccessFlags::TRANSFER_WRITE);
        assert_eq!(to_transfer.old_layout, vk::ImageLayout::UNDEFINED);
        assert_eq!(
            to_transfer.new_layout,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL
        );
        assert_eq!(to_transfer.image, image);
        assert_eq!(
            to_transfer_command.src_stage_mask,
            vk::PipelineStageFlags::TOP_OF_PIPE
        );
        assert_eq!(
            to_transfer_command.dst_stage_mask,
            vk::PipelineStageFlags::TRANSFER
        );
        assert_eq!(to_transfer_command.image_barriers[0].image, image);

        let copy = plan.buffer_to_image_copy_command(vk::Buffer::null(), image);
        assert_eq!(copy.buffer, vk::Buffer::null());
        assert_eq!(copy.image, image);
        assert_eq!(copy.image_layout, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
        assert_eq!(copy.regions.len(), 3);

        let to_shader = plan.shader_read_barrier(image);
        let to_shader_command = plan.shader_read_barrier_command(image);
        assert_eq!(to_shader.src_access_mask, vk::AccessFlags::TRANSFER_WRITE);
        assert_eq!(to_shader.dst_access_mask, vk::AccessFlags::SHADER_READ);
        assert_eq!(to_shader.old_layout, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
        assert_eq!(
            to_shader.new_layout,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
        );
        assert_eq!(to_shader.image, image);
        assert_eq!(
            to_shader_command.src_stage_mask,
            vk::PipelineStageFlags::TRANSFER
        );
        assert_eq!(
            to_shader_command.dst_stage_mask,
            vk::PipelineStageFlags::FRAGMENT_SHADER
        );
        assert_eq!(to_shader_command.image_barriers[0].image, image);

        let sequence = plan.command_sequence(image, vk::Buffer::null());
        assert_eq!(
            sequence.transfer_dst_barrier.src_stage_mask,
            vk::PipelineStageFlags::TOP_OF_PIPE
        );
        assert_eq!(sequence.copy.image, image);
        assert_eq!(sequence.copy.buffer, vk::Buffer::null());
        assert_eq!(sequence.copy.regions.len(), 3);
        assert_eq!(
            sequence.shader_read_barrier.dst_stage_mask,
            vk::PipelineStageFlags::FRAGMENT_SHADER
        );
    }

    #[test]
    fn color_attachment_readback_plan_exposes_barrier_and_copy_region() {
        let plan = ash_color_attachment_readback_plan(vk::Extent2D {
            width: 64,
            height: 32,
        });
        let image = vk::Image::null();
        let barrier = plan.transfer_src_barrier(image);
        let barrier_command = plan.transfer_src_barrier_command(image);
        let regions = plan.copy_regions();
        let copy = plan.image_to_buffer_copy_command(image, vk::Buffer::null());

        assert_eq!(
            plan.subresource_range.aspect_mask,
            vk::ImageAspectFlags::COLOR
        );
        assert_eq!(plan.subresource_range.level_count, 1);
        assert_eq!(plan.subresource_range.layer_count, 1);
        assert_eq!(
            barrier.src_access_mask,
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE
        );
        assert_eq!(barrier.dst_access_mask, vk::AccessFlags::TRANSFER_READ);
        assert_eq!(
            barrier.old_layout,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
        );
        assert_eq!(barrier.new_layout, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
        assert_eq!(barrier.image, image);
        assert_eq!(
            barrier_command.src_stage_mask,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
        );
        assert_eq!(
            barrier_command.dst_stage_mask,
            vk::PipelineStageFlags::TRANSFER
        );
        assert_eq!(barrier_command.image_barriers[0].image, image);
        assert_eq!(
            regions[0].image_subresource.aspect_mask,
            vk::ImageAspectFlags::COLOR
        );
        assert_eq!(regions[0].image_subresource.layer_count, 1);
        assert_eq!(regions[0].image_extent.width, 64);
        assert_eq!(regions[0].image_extent.height, 32);
        assert_eq!(regions[0].image_extent.depth, 1);
        assert_eq!(copy.image, image);
        assert_eq!(copy.image_layout, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
        assert_eq!(copy.buffer, vk::Buffer::null());
        assert_eq!(copy.regions[0].image_extent.width, 64);

        let sequence = plan.command_sequence(image, vk::Buffer::null());
        assert_eq!(
            sequence.transfer_src_barrier.src_stage_mask,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
        );
        assert_eq!(sequence.copy.image, image);
        assert_eq!(
            sequence.copy.image_layout,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL
        );
        assert_eq!(sequence.copy.buffer, vk::Buffer::null());
        assert_eq!(sequence.copy.regions[0].image_extent.height, 32);
    }

    #[test]
    fn depth_attachment_plan_exposes_engine_owned_image_contract() {
        let extent = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let plan = ash_depth_attachment_plan(vk::Format::D24_UNORM_S8_UINT, extent);

        assert_eq!(plan.format, vk::Format::D24_UNORM_S8_UINT);
        assert_eq!(
            plan.extent,
            vk::Extent3D {
                width: 1280,
                height: 720,
                depth: 1
            }
        );
        assert_eq!(
            plan.image_usage,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
        );
        assert_eq!(
            plan.aspect_mask,
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
        );
        assert_eq!(
            plan.final_layout,
            vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
        );
        assert_eq!(
            ash_depth_attachment_plan(vk::Format::D32_SFLOAT, extent).aspect_mask,
            vk::ImageAspectFlags::DEPTH
        );
    }

    #[test]
    fn depth_format_selection_prefers_reference_then_fallbacks() {
        assert_eq!(
            ash_depth_format_candidates(),
            [
                ash_reference_depth_format(),
                vk::Format::X8_D24_UNORM_PACK32,
                vk::Format::D32_SFLOAT,
            ]
        );

        let selected = ash_select_depth_format(|format| {
            let supported = format == vk::Format::X8_D24_UNORM_PACK32;
            vk::FormatProperties {
                optimal_tiling_features: if supported {
                    vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT
                } else {
                    vk::FormatFeatureFlags::empty()
                },
                ..Default::default()
            }
        });
        assert_eq!(selected, Ok(vk::Format::X8_D24_UNORM_PACK32));
        assert_eq!(
            ash_select_depth_format(|_| vk::FormatProperties::default()),
            Err("no supported Vulkan depth attachment format found".to_owned())
        );
    }

    #[test]
    fn memory_type_selection_checks_type_bits_and_required_flags() {
        let mut properties = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: 3,
            ..Default::default()
        };
        properties.memory_types[0].property_flags = vk::MemoryPropertyFlags::DEVICE_LOCAL;
        properties.memory_types[1].property_flags =
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        properties.memory_types[2].property_flags =
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::DEVICE_LOCAL;

        assert_eq!(
            ash_memory_type_index(
                properties,
                0b110,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            ),
            Ok(1)
        );
        assert_eq!(
            ash_memory_type_index(
                properties,
                0b100,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            ),
            Err(
                "no Vulkan memory type supports HOST_VISIBLE | HOST_COHERENT for type bits 0x00000004"
                    .to_owned()
            )
        );
    }

    #[test]
    fn render_pass_creation_plan_exposes_target_specific_layouts_and_dependencies() {
        let windowed = ash_render_pass_creation_plan(
            vk::Format::B8G8R8A8_UNORM,
            ash_reference_depth_format(),
            AshColorAttachmentFinalLayout::Present,
            AshRenderPassDependencyPolicy::ColorOnly,
        );
        let windowed_attachments = windowed.attachment_descriptions();

        assert_eq!(windowed_attachments[0].format, vk::Format::B8G8R8A8_UNORM);
        assert_eq!(
            windowed_attachments[0].final_layout,
            vk::ImageLayout::PRESENT_SRC_KHR
        );
        assert_eq!(
            windowed_attachments[1].final_layout,
            vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
        );
        assert_eq!(
            windowed.subpass_dependency().dst_stage_mask,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
        );
        assert_eq!(
            windowed.subpass_dependency().dst_access_mask,
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE
        );

        let offscreen = ash_render_pass_creation_plan(
            vk::Format::R8G8B8A8_UNORM,
            ash_reference_depth_format(),
            AshColorAttachmentFinalLayout::ColorAttachment,
            AshRenderPassDependencyPolicy::ColorAndDepth,
        );
        let offscreen_attachments = offscreen.attachment_descriptions();
        let offscreen_dependency = offscreen.subpass_dependency();

        assert_eq!(
            offscreen_attachments[0].final_layout,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
        );
        assert!(
            offscreen_dependency
                .dst_stage_mask
                .contains(vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS)
        );
        assert!(
            offscreen_dependency
                .dst_access_mask
                .contains(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
        );
        assert_eq!(
            offscreen.color_attachment_references()[0].layout,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
        );
        assert_eq!(
            offscreen.depth_attachment_reference().layout,
            vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
        );
        offscreen.with_render_pass_create_info(|info| {
            assert_eq!(info.attachment_count, 2);
            assert_eq!(info.subpass_count, 1);
            assert_eq!(info.dependency_count, 1);
            assert!(!info.p_attachments.is_null());
            assert!(!info.p_subpasses.is_null());
            assert!(!info.p_dependencies.is_null());
        });
    }

    #[test]
    fn framebuffer_plan_exposes_extent_and_single_layer_contract() {
        let plan = ash_framebuffer_plan(vk::Extent2D {
            width: 640,
            height: 360,
        });

        assert_eq!(plan.width(), 640);
        assert_eq!(plan.height(), 360);
        assert_eq!(plan.layers, 1);
    }

    #[test]
    fn windowed_resize_validation_requires_observed_recreate() {
        assert_eq!(
            AshWindowedResizeValidation::default().validate_recreate(),
            Err("--require-resize-recreate requires --resize-after-frames".to_owned())
        );
        assert_eq!(
            AshWindowedResizeValidation {
                resize_after_frames: Some(8),
                ..Default::default()
            }
            .validate_recreate(),
            Err("resize was never requested".to_owned())
        );
        assert_eq!(
            AshWindowedResizeValidation {
                resize_after_frames: Some(8),
                resize_requested: true,
                ..Default::default()
            }
            .validate_recreate(),
            Err("no WindowEvent::Resized was observed after resize request".to_owned())
        );
        assert_eq!(
            AshWindowedResizeValidation {
                resize_after_frames: Some(8),
                resize_requested: true,
                resize_events_after_request: 1,
                ..Default::default()
            }
            .validate_recreate(),
            Err("renderer.recreate_swapchain was never called".to_owned())
        );
        assert!(
            AshWindowedResizeValidation {
                resize_after_frames: Some(8),
                resize_requested: true,
                resize_events_after_request: 1,
                swapchain_recreates: 1,
            }
            .validate_recreate()
            .is_ok()
        );
    }

    #[test]
    fn windowed_cache_stats_validate_steady_state_hits() {
        let mut stats = AshMtoonWindowedCacheStats::default();
        assert_eq!(
            stats.validate_steady_state_hits(),
            Err("pipeline cache reported no hits; run at least two MToon frames".to_owned())
        );
        stats.pipeline.hit();
        stats.descriptors.hit();
        stats.samplers.hit();
        stats.buffers.hit();
        stats.uniforms.hit();
        stats.textures.hit();
        stats.fallback_textures.hit();
        stats.command_buffers.hit();
        stats.pipeline.rebuild();

        assert!(stats.validate_steady_state_hits().is_ok());
        assert_eq!(
            stats.to_string(),
            "pipeline(hits=1,rebuilds=1); descriptors(hits=1,rebuilds=0); samplers(hits=1,rebuilds=0); buffers(hits=1,rebuilds=0); uniforms(hits=1,rebuilds=0); textures(hits=1,rebuilds=0); fallback_textures(hits=1,rebuilds=0); command_buffers(hits=1,rebuilds=0)"
        );
    }

    #[test]
    fn mtoon_renderer_cache_keys_split_shape_and_payload_lifetimes() {
        let frame = cache_key_test_frame(vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![255, 0, 0, 255]);
        let extent = vk::Extent2D {
            width: 640,
            height: 360,
        };
        let shader = AshMtoonShaderCacheKey::from_spirv_words(
            "vs_main",
            "fs_main",
            &[0x0723_0203, 1, 2, 3, 4],
            &[0x0723_0203, 5, 6, 7, 8],
        );
        let keys = ash_mtoon_renderer_cache_keys(&frame, extent, shader.clone());

        let dynamic_payload_frame =
            cache_key_test_frame(vec![9, 9, 9, 9], vec![8, 8, 8, 8], vec![255, 0, 0, 255]);
        let dynamic_payload_keys =
            ash_mtoon_renderer_cache_keys(&dynamic_payload_frame, extent, shader.clone());
        assert_eq!(keys.buffers, dynamic_payload_keys.buffers);
        assert_eq!(keys.uniforms, dynamic_payload_keys.uniforms);
        assert_eq!(keys.descriptor_sets, dynamic_payload_keys.descriptor_sets);
        assert_eq!(keys.pipeline, dynamic_payload_keys.pipeline);
        assert_eq!(keys.textures, dynamic_payload_keys.textures);

        let texture_payload_frame =
            cache_key_test_frame(vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![0, 255, 0, 255]);
        let texture_payload_keys =
            ash_mtoon_renderer_cache_keys(&texture_payload_frame, extent, shader.clone());
        assert_eq!(keys.buffers, texture_payload_keys.buffers);
        assert_eq!(keys.uniforms, texture_payload_keys.uniforms);
        assert_ne!(keys.textures, texture_payload_keys.textures);

        let changed_shader = AshMtoonShaderCacheKey::from_spirv_words(
            "vs_main",
            "fs_main",
            &[0x0723_0203, 1, 2, 3, 9],
            &[0x0723_0203, 5, 6, 7, 8],
        );
        let changed_shader_keys = ash_mtoon_renderer_cache_keys(&frame, extent, changed_shader);
        assert_ne!(keys.pipeline, changed_shader_keys.pipeline);
        assert_eq!(keys.descriptor_sets, changed_shader_keys.descriptor_sets);

        let changed_coordinate_policy = shader.clone().with_coordinate_policy(
            AshClipSpacePolicy::NagaVulkanZeroToOneYDown,
            AshSpirvCoordinateAdjustment::NagaWriter,
        );
        let changed_coordinate_keys =
            ash_mtoon_renderer_cache_keys(&frame, extent, changed_coordinate_policy);
        assert_ne!(keys.pipeline, changed_coordinate_keys.pipeline);
        assert_eq!(
            keys.descriptor_sets,
            changed_coordinate_keys.descriptor_sets
        );

        let resized_keys = ash_mtoon_renderer_cache_keys(
            &frame,
            vk::Extent2D {
                width: 800,
                height: 450,
            },
            shader,
        );
        assert_ne!(keys.pipeline, resized_keys.pipeline);
        assert_eq!(keys.descriptor_sets, resized_keys.descriptor_sets);
        assert_eq!(keys.samplers, resized_keys.samplers);
    }

    #[test]
    fn mtoon_materialization_plan_groups_renderer_edge_batches() {
        let frame = cache_key_test_frame(vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![255, 0, 0, 255]);
        let extent = vk::Extent2D {
            width: 640,
            height: 360,
        };
        let shader = AshMtoonShaderCacheKey::from_spirv_words(
            "vs_main",
            "fs_main",
            &[0x0723_0203, 1, 2, 3, 4],
            &[0x0723_0203, 5, 6, 7, 8],
        );
        let plan = ash_mtoon_materialization_plan(&frame, extent, shader.clone()).unwrap();

        assert_eq!(
            plan.render_target,
            AshMtoonRenderTargetCacheKey::from_extent(extent)
        );
        assert_eq!(plan.shader, shader);
        assert_eq!(
            plan.cache_keys,
            ash_mtoon_renderer_cache_keys(&frame, extent, shader)
        );
        assert_eq!(plan.resource_manifest, frame.resource_manifest());
        assert_eq!(plan.descriptor_pool, ash_descriptor_pool_plan(&frame));
        assert_eq!(
            plan.descriptor_set_layouts,
            ash_descriptor_set_layout_plans(&frame)
        );
        assert_eq!(
            plan.pipeline_layouts,
            ash_pipeline_layout_plans(&plan.descriptor_set_layouts)
        );
        assert_eq!(
            plan.descriptor_set_allocation,
            ash_descriptor_set_allocation_plan(&plan.descriptor_set_layouts)
        );
        assert_eq!(plan.sampler_resources, ash_sampler_resource_plans(&frame));
        assert_eq!(
            plan.descriptor_writes,
            ash_descriptor_write_plans(&frame).unwrap()
        );
        assert_eq!(plan.persistent_handle_resource_count(), 7);
        assert_eq!(plan.frame_dynamic_resource_count(), 3);
        assert_eq!(plan.draw_command_count(), plan.drawable.commands.len());
    }

    #[test]
    fn mtoon_materialization_plan_validates_descriptor_writes() {
        let mut frame =
            cache_key_test_frame(vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![255, 0, 0, 255]);
        frame.descriptor_sets[0].bindings[0].uniform_upload_index = Some(99);
        let error = ash_mtoon_materialization_plan(
            &frame,
            vk::Extent2D {
                width: 640,
                height: 360,
            },
            AshMtoonShaderCacheKey::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "descriptor set 0 binding 30 references missing uniform upload 99"
        );
    }

    #[test]
    fn descriptor_pool_plan_counts_renderer_frame_bindings() {
        let mut frame =
            cache_key_test_frame(vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![255, 0, 0, 255]);
        frame.descriptor_sets[0]
            .bindings
            .push(AshResolvedDescriptorBinding {
                binding: ash_mtoon_wgsl_owner_sample_override_binding(),
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
                uniform_upload_index: None,
                texture_upload_index: None,
                buffer_upload_index: Some(0),
                sampler: None,
            });

        let plan = ash_descriptor_pool_plan(&frame);
        assert_eq!(plan.max_sets, 1);
        assert_eq!(
            plan.pool_sizes,
            vec![
                AshDescriptorPoolSizePlan {
                    descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
                    descriptor_count: 1,
                },
                AshDescriptorPoolSizePlan {
                    descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                    descriptor_count: 1,
                },
                AshDescriptorPoolSizePlan {
                    descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
                    descriptor_count: 1,
                },
                AshDescriptorPoolSizePlan {
                    descriptor_type: vk::DescriptorType::SAMPLER,
                    descriptor_count: 1,
                },
                AshDescriptorPoolSizePlan {
                    descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                    descriptor_count: 1,
                },
            ]
        );
        assert_eq!(
            plan.vk_pool_sizes()
                .iter()
                .map(|size| (size.ty, size.descriptor_count))
                .collect::<Vec<_>>(),
            plan.pool_sizes
                .iter()
                .map(|size| (size.descriptor_type, size.descriptor_count))
                .collect::<Vec<_>>()
        );
        plan.with_descriptor_pool_create_info(|info| {
            assert_eq!(info.max_sets, plan.max_sets);
            assert_eq!(info.pool_size_count, plan.pool_sizes.len() as u32);
        });
        assert_eq!(
            ash_descriptor_pool_plan(&AshRendererFrame::default()).max_sets,
            1
        );
    }

    #[test]
    fn descriptor_set_and_pipeline_layout_plans_expose_vk_layout_contract() {
        let frame = cache_key_test_frame(vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![255, 0, 0, 255]);
        let plans = ash_descriptor_set_layout_plans(&frame);

        assert_eq!(plans.len(), frame.descriptor_sets.len());
        assert_eq!(plans[0].descriptor_set_index, 0);
        assert_eq!(plans[0].material, frame.descriptor_sets[0].material);
        assert_eq!(
            plans[0].pipeline_plan_index,
            frame.descriptor_sets[0].pipeline_plan_index
        );
        assert_eq!(
            plans[0].bindings.len(),
            frame.descriptor_sets[0].bindings.len()
        );

        let vk_bindings = plans[0].vk_bindings();
        assert_eq!(vk_bindings.len(), plans[0].bindings.len());
        assert_eq!(vk_bindings[0].binding, plans[0].bindings[0].binding);
        assert_eq!(
            vk_bindings[0].descriptor_type,
            plans[0].bindings[0].descriptor_type
        );
        assert_eq!(vk_bindings[0].descriptor_count, 1);
        assert_eq!(vk_bindings[0].stage_flags, plans[0].bindings[0].stage_flags);
        plans[0].with_descriptor_set_layout_create_info(|info| {
            assert_eq!(info.binding_count, plans[0].bindings.len() as u32);
        });

        assert_eq!(
            ash_pipeline_layout_plans(&plans),
            vec![AshPipelineLayoutPlan {
                descriptor_set_layout_index: 0
            }]
        );

        let layout_handles = vec![vk::DescriptorSetLayout::null()];
        let allocation = ash_descriptor_set_allocation_plan(&plans);
        assert_eq!(allocation.descriptor_set_count(), 1);
        assert_eq!(
            allocation.vk_set_layouts(&layout_handles),
            Ok(layout_handles.clone())
        );
        allocation
            .with_descriptor_set_allocate_info(
                vk::DescriptorPool::null(),
                &layout_handles,
                |info| {
                    assert_eq!(info.descriptor_pool, vk::DescriptorPool::null());
                    assert_eq!(info.descriptor_set_count, 1);
                },
            )
            .unwrap();
        assert_eq!(
            AshPipelineLayoutPlan {
                descriptor_set_layout_index: 0
            }
            .vk_set_layouts(&layout_handles),
            Ok([vk::DescriptorSetLayout::null()])
        );
        AshPipelineLayoutPlan {
            descriptor_set_layout_index: 0,
        }
        .with_pipeline_layout_create_info(&layout_handles, |info| {
            assert_eq!(info.set_layout_count, 1);
        })
        .unwrap();
        let empty_layout_info = ash_empty_pipeline_layout_plan().pipeline_layout_create_info();
        assert_eq!(empty_layout_info.set_layout_count, 0);
        assert!(empty_layout_info.p_set_layouts.is_null());
        assert_eq!(
            AshPipelineLayoutPlan {
                descriptor_set_layout_index: 3
            }
            .vk_set_layouts(&[]),
            Err("pipeline layout references missing descriptor set layout 3".to_owned())
        );
        assert_eq!(
            AshDescriptorSetAllocationPlan {
                descriptor_set_layout_indices: vec![2]
            }
            .vk_set_layouts(&[]),
            Err("descriptor set allocation references missing layout 2".to_owned())
        );
    }

    #[test]
    fn descriptor_write_plans_resolve_indices_samplers_and_fallbacks() {
        let mut frame =
            cache_key_test_frame(vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![255, 0, 0, 255]);
        frame.descriptor_sets[0]
            .bindings
            .push(AshResolvedDescriptorBinding {
                binding: ash_mtoon_wgsl_owner_sample_override_binding(),
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
                uniform_upload_index: None,
                texture_upload_index: None,
                buffer_upload_index: Some(0),
                sampler: None,
            });
        frame.descriptor_sets[0]
            .bindings
            .push(AshResolvedDescriptorBinding {
                binding: 3,
                descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
                uniform_upload_index: None,
                texture_upload_index: None,
                buffer_upload_index: None,
                sampler: Some(AshSamplerPlan::default()),
            });

        let sampler_plans = ash_sampler_resource_plans(&frame);
        assert_eq!(
            sampler_plans,
            vec![
                AshSamplerResourcePlan {
                    sampler_index: 0,
                    descriptor_set_index: 0,
                    binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Main),
                    descriptor_type: vk::DescriptorType::SAMPLER,
                    sampler: AshSamplerPlan::default(),
                },
                AshSamplerResourcePlan {
                    sampler_index: 1,
                    descriptor_set_index: 0,
                    binding: 3,
                    descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                    sampler: AshSamplerPlan::default(),
                },
            ]
        );

        let plans = ash_descriptor_write_plans(&frame).unwrap();
        assert_eq!(plans.len(), 5);
        assert_eq!(
            plans[0].resource,
            AshDescriptorWriteResource::UniformBuffer {
                uniform_upload_index: 0,
            }
        );
        assert_eq!(
            plans[1].resource,
            AshDescriptorWriteResource::SampledImage {
                image: AshDescriptorImageResource::TextureUpload {
                    texture_upload_index: 0,
                },
            }
        );
        assert_eq!(
            plans[2].resource,
            AshDescriptorWriteResource::Sampler { sampler_index: 0 }
        );
        assert_eq!(
            plans[3].resource,
            AshDescriptorWriteResource::StorageBuffer {
                buffer_upload_index: 0,
            }
        );
        assert_eq!(
            plans[4].resource,
            AshDescriptorWriteResource::CombinedImageSampler {
                sampler_index: 1,
                image: AshDescriptorImageResource::Fallback {
                    fallback: GltfMaterialTextureFallback::Black,
                },
            }
        );

        frame.descriptor_sets[0].bindings[1].texture_upload_index = Some(99);
        assert_eq!(
            ash_descriptor_write_plans(&frame),
            Err("descriptor binding 1 references missing texture upload 99".to_owned())
        );
    }

    #[test]
    fn descriptor_write_helpers_resolve_engine_owned_handles_with_checked_errors() {
        let plan = AshDescriptorWritePlan {
            descriptor_set_index: 0,
            binding: 7,
            descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
            resource: AshDescriptorWriteResource::UniformBuffer {
                uniform_upload_index: 0,
            },
        };
        let descriptor_sets = [vk::DescriptorSet::null()];

        assert_eq!(
            plan.vk_descriptor_set(&descriptor_sets),
            Ok(vk::DescriptorSet::null())
        );
        assert_eq!(
            AshDescriptorWritePlan {
                descriptor_set_index: 2,
                ..plan.clone()
            }
            .vk_descriptor_set(&descriptor_sets),
            Err("descriptor write references missing descriptor set 2".to_owned())
        );

        assert_eq!(plan.resource.uniform_resource(&[42_u32]), Ok(&42_u32));
        let empty_uniforms: [u32; 0] = [];
        assert_eq!(
            plan.resource.uniform_resource(&empty_uniforms),
            Err("descriptor write references missing uniform buffer 0".to_owned())
        );

        let storage = AshDescriptorWriteResource::StorageBuffer {
            buffer_upload_index: 1,
        };
        assert_eq!(storage.storage_buffer_resource(&[3_u32, 9_u32]), Ok(&9_u32));
        let empty_buffers: [u32; 0] = [];
        assert_eq!(
            storage.storage_buffer_resource(&empty_buffers),
            Err("descriptor write references missing storage buffer 1".to_owned())
        );

        let sampler = AshDescriptorWriteResource::Sampler { sampler_index: 0 };
        assert_eq!(
            sampler.sampler(&[vk::Sampler::null()]),
            Ok(vk::Sampler::null())
        );
        assert_eq!(
            sampler.sampler(&[]),
            Err("descriptor write references missing sampler 0".to_owned())
        );

        let image = AshDescriptorImageResource::Fallback {
            fallback: GltfMaterialTextureFallback::White,
        };
        assert_eq!(
            AshDescriptorWriteResource::SampledImage { image }.image_resource(),
            Ok(image)
        );
        assert_eq!(
            image.resolve_resource(&[1_u32], |fallback| match fallback {
                GltfMaterialTextureFallback::White => &9_u32,
                GltfMaterialTextureFallback::Black => &8_u32,
                GltfMaterialTextureFallback::NeutralNormal => &7_u32,
            }),
            Ok(&9_u32)
        );
        let uploaded = AshDescriptorImageResource::TextureUpload {
            texture_upload_index: 0,
        };
        assert_eq!(uploaded.texture_upload_resource(&[55_u32]), Ok(&55_u32));
        assert_eq!(
            uploaded.resolve_resource(&[55_u32], |_| &0_u32),
            Ok(&55_u32)
        );
        let empty_textures: [u32; 0] = [];
        assert_eq!(
            uploaded.texture_upload_resource(&empty_textures),
            Err("descriptor image references missing texture upload 0".to_owned())
        );
        assert_eq!(
            uploaded.fallback(),
            Err(
                "descriptor image resource is not a fallback texture: TextureUpload { texture_upload_index: 0 }"
                    .to_owned()
            )
        );
        assert_eq!(
            plan.resource.image_resource(),
            Err(
                "descriptor write resource does not reference an image: UniformBuffer { uniform_upload_index: 0 }"
                    .to_owned()
            )
        );

        assert_eq!(
            plan.resolve_write_data(
                AshDescriptorWriteResources::new(
                    &[vk::Buffer::null()],
                    &[vk::Buffer::null()],
                    &[vk::ImageView::null()],
                    &[vk::Sampler::null()],
                ),
                AshDescriptorWriteHandleAccess {
                    uniform_buffer: |buffer: &vk::Buffer| *buffer,
                    storage_buffer: |buffer: &vk::Buffer| *buffer,
                    texture_image_view: |view: &vk::ImageView| *view,
                    fallback_image_view: |_| vk::ImageView::null(),
                    image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                },
            ),
            Ok(AshDescriptorWriteData::whole_buffer(vk::Buffer::null()))
        );

        let wrote_buffer = plan
            .with_write_descriptor_set(
                &descriptor_sets,
                AshDescriptorWriteData::whole_buffer(vk::Buffer::null()),
                |write| {
                    assert_eq!(write.dst_set, vk::DescriptorSet::null());
                    assert_eq!(write.dst_binding, 7);
                    assert_eq!(write.descriptor_type, vk::DescriptorType::UNIFORM_BUFFER);
                    assert_eq!(write.descriptor_count, 1);
                    assert!(!write.p_buffer_info.is_null());
                    true
                },
            )
            .unwrap();
        assert!(wrote_buffer);

        let sampled_image_plan = AshDescriptorWritePlan {
            descriptor_set_index: 0,
            binding: 11,
            descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
            resource: AshDescriptorWriteResource::SampledImage { image },
        };
        assert_eq!(
            sampled_image_plan.resolve_write_data(
                AshDescriptorWriteResources::new(
                    &[vk::Buffer::null()],
                    &[vk::Buffer::null()],
                    &[vk::ImageView::null()],
                    &[vk::Sampler::null()],
                ),
                AshDescriptorWriteHandleAccess {
                    uniform_buffer: |buffer: &vk::Buffer| *buffer,
                    storage_buffer: |buffer: &vk::Buffer| *buffer,
                    texture_image_view: |view: &vk::ImageView| *view,
                    fallback_image_view: |_| vk::ImageView::null(),
                    image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                },
            ),
            Ok(AshDescriptorWriteData::sampled_image(
                vk::ImageView::null(),
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            ))
        );
        sampled_image_plan
            .with_write_descriptor_set(
                &descriptor_sets,
                AshDescriptorWriteData::sampled_image(
                    vk::ImageView::null(),
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                ),
                |write| {
                    assert_eq!(write.dst_binding, 11);
                    assert_eq!(write.descriptor_type, vk::DescriptorType::SAMPLED_IMAGE);
                    assert!(!write.p_image_info.is_null());
                },
            )
            .unwrap();

        let combined_plan = AshDescriptorWritePlan {
            descriptor_set_index: 0,
            binding: 12,
            descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            resource: AshDescriptorWriteResource::CombinedImageSampler {
                sampler_index: 0,
                image: AshDescriptorImageResource::TextureUpload {
                    texture_upload_index: 0,
                },
            },
        };
        assert_eq!(
            combined_plan.resolve_write_data(
                AshDescriptorWriteResources::new(
                    &[vk::Buffer::null()],
                    &[vk::Buffer::null()],
                    &[vk::ImageView::null()],
                    &[vk::Sampler::null()],
                ),
                AshDescriptorWriteHandleAccess {
                    uniform_buffer: |buffer: &vk::Buffer| *buffer,
                    storage_buffer: |buffer: &vk::Buffer| *buffer,
                    texture_image_view: |view: &vk::ImageView| *view,
                    fallback_image_view: |_| vk::ImageView::null(),
                    image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                },
            ),
            Ok(AshDescriptorWriteData::combined_image_sampler(
                vk::Sampler::null(),
                vk::ImageView::null(),
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            ))
        );

        let mismatch = sampled_image_plan
            .with_write_descriptor_set(
                &descriptor_sets,
                AshDescriptorWriteData::whole_buffer(vk::Buffer::null()),
                |_| (),
            )
            .unwrap_err();
        assert!(mismatch.contains("does not match resource"));
    }

    fn cache_key_test_frame(
        buffer_bytes: Vec<u8>,
        uniform_bytes: Vec<u8>,
        texture_rgba: Vec<u8>,
    ) -> AshRendererFrame {
        AshRendererFrame {
            buffers: vec![AshBufferUpload {
                role: AshBufferRole::Vertex,
                usage: vk::BufferUsageFlags::VERTEX_BUFFER,
                stride: 4,
                count: 1,
                bytes: buffer_bytes,
            }],
            textures: vec![AshTextureResourcePlan {
                upload: AshTextureUpload {
                    texture: Some(TextureRef(3)),
                    color_space: GltfMaterialTextureColorSpace::Srgb,
                    format: vk::Format::R8G8B8A8_SRGB,
                    extent: vk::Extent3D {
                        width: 1,
                        height: 1,
                        depth: 1,
                    },
                    rgba: texture_rgba,
                },
                image_usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
                image_layout_after_upload: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                aspect_mask: vk::ImageAspectFlags::COLOR,
            }],
            uniforms: vec![AshUniformUpload {
                scope: AshUniformScope::Scene,
                binding: ash_mtoon_wgsl_scene_binding(),
                bytes: uniform_bytes,
            }],
            pipelines: vec![AshGraphicsPipelinePlan {
                material: MaterialRef(0),
                pipeline_plan_index: 0,
                descriptor_set_index: 0,
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
                vertex_stride: 4,
                vertex_attributes: vec![AshVertexAttributePlan {
                    location: 0,
                    binding: 0,
                    format: vk::Format::R32_SFLOAT,
                    offset: 0,
                }],
                color_format: vk::Format::R8G8B8A8_UNORM,
                depth_format: Some(ash_reference_depth_format()),
            }],
            descriptor_sets: vec![AshDescriptorSetPlan {
                material: MaterialRef(0),
                pipeline_plan_index: 0,
                bindings: vec![
                    AshResolvedDescriptorBinding {
                        binding: ash_mtoon_wgsl_scene_binding(),
                        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
                        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                        uniform_upload_index: Some(0),
                        texture_upload_index: None,
                        buffer_upload_index: None,
                        sampler: None,
                    },
                    AshResolvedDescriptorBinding {
                        binding: ash_mtoon_sampled_image_binding(MtoonTextureSlot::Main),
                        descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
                        stage_flags: vk::ShaderStageFlags::FRAGMENT,
                        uniform_upload_index: None,
                        texture_upload_index: Some(0),
                        buffer_upload_index: None,
                        sampler: Some(AshSamplerPlan::default()),
                    },
                    AshResolvedDescriptorBinding {
                        binding: ash_mtoon_texture_sampler_binding(MtoonTextureSlot::Main),
                        descriptor_type: vk::DescriptorType::SAMPLER,
                        stage_flags: vk::ShaderStageFlags::FRAGMENT,
                        uniform_upload_index: None,
                        texture_upload_index: Some(0),
                        buffer_upload_index: None,
                        sampler: Some(AshSamplerPlan::default()),
                    },
                ],
            }],
            draw_calls: Vec::new(),
        }
    }

    #[test]
    fn shader_module_and_stage_plans_expose_vk_infos() {
        let words = [0x0723_0203_u32, 0, 1, 0];
        let module_plan = ash_shader_module_plan(&words);
        let module_info = module_plan.shader_module_create_info();
        assert_eq!(module_plan.code_words, &words);
        assert_eq!(
            module_info.code_size,
            words.len() * std::mem::size_of::<u32>()
        );
        assert_eq!(module_info.p_code, words.as_ptr());

        let vertex_module = vk::ShaderModule::null();
        let fragment_module = vk::ShaderModule::null();
        let stages_plan = ash_graphics_shader_stages_plan(vertex_module, fragment_module);
        let vertex_entry = std::ffi::CString::new("vs_main").unwrap();
        let fragment_entry = std::ffi::CString::new("fs_main").unwrap();
        let stages = stages_plan.shader_stage_create_infos(&vertex_entry, &fragment_entry);
        assert_eq!(stages[0].stage, vk::ShaderStageFlags::VERTEX);
        assert_eq!(stages[0].module, vertex_module);
        assert_eq!(stages[0].p_name, vertex_entry.as_ptr());
        assert_eq!(stages[1].stage, vk::ShaderStageFlags::FRAGMENT);
        assert_eq!(stages[1].module, fragment_module);
        assert_eq!(stages[1].p_name, fragment_entry.as_ptr());
    }

    #[test]
    fn source_mtoon_shader_matches_rust_binding_contract() {
        let vertex_shader = include_str!("../shaders/mtoon_base.vert.glsl");
        let fragment_shader = include_str!("../shaders/mtoon_base.frag.glsl");
        let wgsl_shader = include_str!("../shaders/mtoon_base.wgsl");
        let wgsl_abi = AshMtoonWgslShaderAbi::default();

        assert_eq!(wgsl_abi.prelude_path, ASH_MTOON_WGSL_PRELUDE_PATH);
        assert_eq!(wgsl_abi.source_path, ASH_MTOON_WGSL_SOURCE_PATH);
        assert_eq!(wgsl_abi.vertex_entry, ASH_MTOON_WGSL_VERTEX_ENTRY);
        assert_eq!(wgsl_abi.fragment_entry, ASH_MTOON_WGSL_FRAGMENT_ENTRY);
        assert_eq!(wgsl_abi.vertex_spirv_file, ASH_MTOON_WGSL_VERTEX_SPIRV_FILE);
        assert_eq!(
            wgsl_abi.fragment_spirv_file,
            ASH_MTOON_WGSL_FRAGMENT_SPIRV_FILE
        );
        assert_eq!(
            wgsl_abi.default_vertex_spirv_path(),
            PathBuf::from(ASH_MTOON_WGSL_DEFAULT_VERTEX_SPIRV_PATH)
        );
        assert_eq!(
            wgsl_abi.default_fragment_spirv_path(),
            PathBuf::from(ASH_MTOON_WGSL_DEFAULT_FRAGMENT_SPIRV_PATH)
        );
        assert_eq!(
            wgsl_abi.clip_space_policy,
            AshClipSpacePolicy::CpuVulkanZeroToOneYDown
        );
        assert_eq!(
            wgsl_abi.spirv_coordinate_adjustment,
            AshSpirvCoordinateAdjustment::Disabled
        );
        assert_eq!(
            wgsl_abi.clip_space_policy.spirv_coordinate_adjustment(),
            wgsl_abi.spirv_coordinate_adjustment
        );
        assert!(
            !wgsl_abi
                .spirv_coordinate_adjustment
                .adjust_coordinate_space()
        );
        assert_eq!(
            wgsl_abi.descriptor_binding_model,
            AshDescriptorBindingModel::SeparateImageSampler
        );
        let wgsl_resources = ash_mtoon_wgsl_resource_bindings();
        assert_eq!(wgsl_resources.len(), 24);
        assert_eq!(
            wgsl_resources[0],
            AshWgslResourceBinding {
                name: "mtoon",
                group: 0,
                binding: ash_mtoon_uniform_binding(),
                kind: AshWgslResourceKind::UniformBuffer,
            }
        );
        assert!(wgsl_resources.iter().any(|resource| {
            resource.name == "emissive_sampler"
                && resource.group == 0
                && resource.binding
                    == ash_material_sampler_binding(GltfMaterialTextureSlot::Emissive)
                && resource.kind == AshWgslResourceKind::Sampler
        }));
        for (index, left) in wgsl_resources.iter().enumerate() {
            assert!(
                wgsl_resources[index + 1..]
                    .iter()
                    .all(|right| (left.group, left.binding) != (right.group, right.binding))
            );
        }

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
        assert!(wgsl_shader.contains("@group(0) @binding(0)"));
        assert!(wgsl_shader.contains("@group(0) @binding(1)"));
        assert!(wgsl_shader.contains("@group(0) @binding(2)"));
        assert!(wgsl_shader.contains("@group(0) @binding(30)"));
        assert!(wgsl_shader.contains("@group(0) @binding(31)"));
        assert!(wgsl_shader.contains("@group(0) @binding(32)"));
        for binding in 1..=20 {
            let declaration = format!("@group(0) @binding({binding})");
            assert!(wgsl_shader.contains(&declaration));
        }
        assert!(wgsl_shader.contains("@vertex"));
        assert!(wgsl_shader.contains(&format!("fn {}", wgsl_abi.vertex_entry)));
        assert!(wgsl_shader.contains("@fragment"));
        assert!(wgsl_shader.contains(&format!("fn {}", wgsl_abi.fragment_entry)));
        assert!(wgsl_shader.contains("fn ash_mtoon_lit_shade_rate"));
        assert!(wgsl_shader.contains("fn ash_mtoon_normal"));
        assert!(wgsl_shader.contains("fn ash_pbr_direct"));
        assert!(wgsl_shader.contains("textureSampleGrad(source, source_sampler"));
        assert!(wgsl_shader.contains("ash_srgb_to_linear_color(raw_main_texel.rgb)"));
        assert!(wgsl_shader.contains("base_sample_uv = vec2<f32>(base_uv.x, 1.0 - base_uv.y)"));
        assert!(wgsl_shader.contains("material_extra.flags2.z > 0.5"));
        assert!(wgsl_shader.contains("material_extra.flags2.w > 4.5"));
        assert!(wgsl_shader.contains("ash_owner_id_output_color(input.color_0.rgb"));
        assert!(wgsl_shader.contains("material_extra.flags2.w < -0.5"));
        assert!(wgsl_shader.contains("alpha_mode < 2u"));
        assert!(wgsl_shader.contains("mtoon.flags.z == 1u"));
        assert!(wgsl_shader.contains("ash_transform_uv(animated_uv"));
        assert!(wgsl_shader.contains("centered.x * c + centered.y * s"));
        assert!(wgsl_shader.contains("-centered.x * s + centered.y * c"));
        assert!(wgsl_shader.contains("ash_matcap_uv_from_view(input, normal)"));
        assert!(wgsl_shader.contains("@builtin(front_facing) front_facing: bool"));
        assert!(wgsl_shader.contains("input.normal_scale == 0.0"));
        assert!(wgsl_shader.contains("input.front_facing || input.double_sided < 0.5"));
        assert!(wgsl_shader.contains("material_extra.flags2.y > 0.5"));
        assert!(wgsl_shader.contains("dpdx(derivative_position)"));
        assert!(wgsl_shader.contains("scene.world_from_view"));
        assert!(wgsl_shader.contains("material_extra.flags.x > 0.5"));
        assert!(wgsl_shader.contains("material_extra.flags2.x > 0.5"));
        assert!(wgsl_shader.contains("output.tex_coord_0 = input.tex_coord_0"));
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
                descriptor_bindings: descriptor_bindings(
                    &[],
                    GltfMaterialTextureSlots::default(),
                    AshDescriptorBindingModel::SeparateImageSampler,
                ),
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
        let descriptor_binding = |binding_number| {
            renderer_frame.descriptor_sets[0]
                .bindings
                .iter()
                .find(|binding| binding.binding == binding_number)
                .expect("descriptor binding")
        };
        assert_eq!(
            descriptor_binding(ash_mtoon_wgsl_scene_binding()).uniform_upload_index,
            Some(3)
        );
        assert_eq!(
            descriptor_binding(ash_mtoon_wgsl_uv_uniform_binding()).uniform_upload_index,
            Some(1)
        );
        assert_eq!(
            descriptor_binding(ash_mtoon_wgsl_render_extra_binding()).uniform_upload_index,
            Some(2)
        );
        assert_eq!(
            descriptor_binding(ash_mtoon_wgsl_owner_sample_override_binding()).descriptor_type,
            vk::DescriptorType::STORAGE_BUFFER
        );
        assert_eq!(
            descriptor_binding(ash_mtoon_wgsl_owner_sample_override_binding()).buffer_upload_index,
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

        let manifest = renderer_frame.resource_manifest();
        assert_eq!(manifest.buffers.len(), renderer_frame.buffers.len());
        assert_eq!(manifest.uniforms.len(), renderer_frame.uniforms.len());
        assert_eq!(
            manifest.descriptor_set_layouts.len(),
            renderer_frame.descriptor_sets.len()
        );
        assert_eq!(
            manifest.descriptor_sets.len(),
            renderer_frame.descriptor_sets.len()
        );
        assert_eq!(manifest.pipelines.len(), renderer_frame.pipelines.len());
        assert!(manifest.textures.is_empty());
        assert!(manifest.samplers.iter().all(|resource| {
            resource.lifetime == AshRendererResourceLifetime::Persistent
                && resource.descriptor_type == vk::DescriptorType::SAMPLER
        }));
        assert!(manifest.buffers.iter().all(|resource| {
            resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                && resource.lifetime == AshRendererResourceLifetime::FrameDynamic
        }));
        assert!(manifest.uniforms.iter().all(|resource| {
            resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                && resource.lifetime == AshRendererResourceLifetime::FrameDynamic
        }));
        assert!(manifest.descriptor_sets.iter().all(|resource| {
            resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                && resource.lifetime == AshRendererResourceLifetime::FrameDynamic
        }));
        assert!(manifest.pipelines.iter().all(|resource| {
            resource.handle_lifetime == AshRendererResourceLifetime::Persistent
                && resource.lifetime == AshRendererResourceLifetime::Persistent
                && resource.depth_format == Some(ash_reference_depth_format())
        }));
        assert_eq!(manifest.buffers[0].role, AshBufferRole::OwnerSampleOverride);
        assert_eq!(manifest.buffers[1].role, AshBufferRole::Vertex);
        assert_eq!(manifest.buffers[2].role, AshBufferRole::Index);
        assert_eq!(manifest.uniforms[3].scope, AshUniformScope::Scene);
        assert_eq!(
            manifest.descriptor_sets[0].bindings[0].lifetime,
            AshRendererResourceLifetime::FrameDynamic
        );
        assert!(manifest.descriptor_sets[0].bindings.iter().any(|binding| {
            binding.descriptor_type == vk::DescriptorType::SAMPLED_IMAGE
                && binding.lifetime == AshRendererResourceLifetime::Persistent
        }));
        assert!(manifest.persistent_resource_count() > 0);
        assert_eq!(
            manifest.persistent_handle_resource_count(),
            manifest.buffers.len()
                + manifest.textures.len()
                + manifest.uniforms.len()
                + manifest.samplers.len()
                + manifest.descriptor_set_layouts.len()
                + manifest.descriptor_sets.len()
                + manifest.pipelines.len()
        );
        assert_eq!(
            manifest.dynamic_resource_count(),
            manifest.buffers.len() + manifest.uniforms.len() + manifest.descriptor_sets.len()
        );

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
                descriptor_bindings: descriptor_bindings(
                    &[],
                    GltfMaterialTextureSlots::default(),
                    AshDescriptorBindingModel::SeparateImageSampler,
                ),
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
                set.bindings.iter().find(|binding| {
                    binding.binding == ash_mtoon_wgsl_owner_sample_override_binding()
                })
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
                descriptor_bindings: descriptor_bindings(
                    &[],
                    GltfMaterialTextureSlots::default(),
                    AshDescriptorBindingModel::SeparateImageSampler,
                ),
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
            descriptor_bindings: descriptor_bindings(
                &[],
                GltfMaterialTextureSlots::default(),
                AshDescriptorBindingModel::SeparateImageSampler,
            ),
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
    fn command_plan_helpers_resolve_engine_owned_handles_with_checked_errors() {
        let frame = cache_key_test_frame(vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![255, 0, 0, 255]);
        let pipeline_handles = [vk::Pipeline::null()];
        let pipeline_layouts = [vk::PipelineLayout::null()];
        let descriptor_sets = [vk::DescriptorSet::null()];

        let bind_pipeline = AshCommandPlan::BindGraphicsPipeline { pipeline_index: 0 };
        assert_eq!(
            bind_pipeline.vk_graphics_pipeline(&pipeline_handles),
            Ok(vk::Pipeline::null())
        );
        assert_eq!(
            bind_pipeline.bind_graphics_pipeline_command(&pipeline_handles),
            Ok(AshBindGraphicsPipelineCommand {
                bind_point: vk::PipelineBindPoint::GRAPHICS,
                pipeline: vk::Pipeline::null(),
            })
        );
        assert_eq!(
            bind_pipeline.resolve_record_command(
                AshCommandRecordResources::new(
                    &frame.pipelines,
                    &pipeline_handles,
                    &pipeline_layouts,
                    &descriptor_sets,
                    &[vk::Buffer::null(), vk::Buffer::null()],
                ),
                AshCommandRecordHandleAccess {
                    buffer: |buffer: &vk::Buffer| *buffer,
                },
            ),
            Ok(AshResolvedCommand::BindGraphicsPipeline(
                AshBindGraphicsPipelineCommand {
                    bind_point: vk::PipelineBindPoint::GRAPHICS,
                    pipeline: vk::Pipeline::null(),
                }
            ))
        );
        assert_eq!(
            AshCommandPlan::BindGraphicsPipeline { pipeline_index: 2 }
                .vk_graphics_pipeline(&pipeline_handles),
            Err("drawable command references missing graphics pipeline 2".to_owned())
        );
        assert_eq!(
            AshCommandPlan::BindGraphicsPipeline { pipeline_index: 2 }.resolve_record_command(
                AshCommandRecordResources::new(
                    &frame.pipelines,
                    &pipeline_handles,
                    &pipeline_layouts,
                    &descriptor_sets,
                    &[vk::Buffer::null()],
                ),
                AshCommandRecordHandleAccess {
                    buffer: |buffer: &vk::Buffer| *buffer,
                },
            ),
            Err("drawable command references missing graphics pipeline 2".to_owned())
        );

        let bind_descriptor = AshCommandPlan::BindDescriptorSet {
            pipeline_index: 0,
            descriptor_set_index: 0,
        };
        assert_eq!(
            bind_descriptor.vk_pipeline_layout(&frame.pipelines, &pipeline_layouts),
            Ok(vk::PipelineLayout::null())
        );
        assert_eq!(
            bind_descriptor.vk_descriptor_set(&descriptor_sets),
            Ok(vk::DescriptorSet::null())
        );
        assert_eq!(
            bind_descriptor.bind_descriptor_set_command(
                &frame.pipelines,
                &pipeline_layouts,
                &descriptor_sets
            ),
            Ok(AshBindDescriptorSetCommand {
                bind_point: vk::PipelineBindPoint::GRAPHICS,
                layout: vk::PipelineLayout::null(),
                first_set: 0,
                descriptor_sets: [vk::DescriptorSet::null()],
                dynamic_offsets: [],
            })
        );
        assert_eq!(
            bind_descriptor.resolve_record_command(
                AshCommandRecordResources::new(
                    &frame.pipelines,
                    &pipeline_handles,
                    &pipeline_layouts,
                    &descriptor_sets,
                    &[vk::Buffer::null()],
                ),
                AshCommandRecordHandleAccess {
                    buffer: |buffer: &vk::Buffer| *buffer,
                },
            ),
            Ok(AshResolvedCommand::BindDescriptorSet(
                AshBindDescriptorSetCommand {
                    bind_point: vk::PipelineBindPoint::GRAPHICS,
                    layout: vk::PipelineLayout::null(),
                    first_set: 0,
                    descriptor_sets: [vk::DescriptorSet::null()],
                    dynamic_offsets: [],
                }
            ))
        );
        assert_eq!(
            AshCommandPlan::BindDescriptorSet {
                pipeline_index: 3,
                descriptor_set_index: 0,
            }
            .vk_pipeline_layout(&frame.pipelines, &pipeline_layouts),
            Err("drawable command references missing pipeline plan 3".to_owned())
        );
        assert_eq!(
            bind_descriptor.vk_pipeline_layout(&frame.pipelines, &[]),
            Err("drawable command references missing pipeline layout 0".to_owned())
        );
        assert_eq!(
            AshCommandPlan::BindDescriptorSet {
                pipeline_index: 0,
                descriptor_set_index: 4,
            }
            .vk_descriptor_set(&descriptor_sets),
            Err("drawable command references missing descriptor set 4".to_owned())
        );

        let vertex = AshCommandPlan::BindVertexBuffer {
            buffer_index: 1,
            binding: 0,
            offset: 16,
        };
        assert_eq!(vertex.vertex_buffer_resource(&[11_u32, 22]), Ok(&22));
        assert_eq!(
            vertex.bind_vertex_buffer_command(vk::Buffer::null()),
            Ok(AshBindVertexBufferCommand {
                first_binding: 0,
                buffers: [vk::Buffer::null()],
                offsets: [16],
            })
        );
        assert_eq!(
            vertex.resolve_record_command(
                AshCommandRecordResources::new(
                    &frame.pipelines,
                    &pipeline_handles,
                    &pipeline_layouts,
                    &descriptor_sets,
                    &[vk::Buffer::null(), vk::Buffer::null()],
                ),
                AshCommandRecordHandleAccess {
                    buffer: |buffer: &vk::Buffer| *buffer,
                },
            ),
            Ok(AshResolvedCommand::BindVertexBuffer(
                AshBindVertexBufferCommand {
                    first_binding: 0,
                    buffers: [vk::Buffer::null()],
                    offsets: [16],
                }
            ))
        );
        assert_eq!(
            vertex.vertex_buffer_resource::<u32>(&[]),
            Err("drawable command references missing vertex buffer 1".to_owned())
        );

        let index = AshCommandPlan::BindIndexBuffer {
            buffer_index: 0,
            offset: 4,
            index_type: vk::IndexType::UINT32,
        };
        assert_eq!(index.index_buffer_resource(&[33_u32]), Ok(&33));
        assert_eq!(
            index.bind_index_buffer_command(vk::Buffer::null()),
            Ok(AshBindIndexBufferCommand {
                buffer: vk::Buffer::null(),
                offset: 4,
                index_type: vk::IndexType::UINT32,
            })
        );
        assert_eq!(
            index.resolve_record_command(
                AshCommandRecordResources::new(
                    &frame.pipelines,
                    &pipeline_handles,
                    &pipeline_layouts,
                    &descriptor_sets,
                    &[vk::Buffer::null()],
                ),
                AshCommandRecordHandleAccess {
                    buffer: |buffer: &vk::Buffer| *buffer,
                },
            ),
            Ok(AshResolvedCommand::BindIndexBuffer(
                AshBindIndexBufferCommand {
                    buffer: vk::Buffer::null(),
                    offset: 4,
                    index_type: vk::IndexType::UINT32,
                }
            ))
        );
        assert_eq!(
            index.index_buffer_resource::<u32>(&[]),
            Err("drawable command references missing index buffer 0".to_owned())
        );

        let draw = AshCommandPlan::DrawIndexed {
            primitive_index: 7,
            index_count: 12,
            instance_count: 1,
            first_index: 2,
            vertex_offset: -3,
            first_instance: 4,
        };
        assert_eq!(
            draw.draw_indexed_args(),
            Ok(AshDrawIndexedCommand {
                index_count: 12,
                instance_count: 1,
                first_index: 2,
                vertex_offset: -3,
                first_instance: 4,
            })
        );
        assert_eq!(
            draw.resolve_record_command(
                AshCommandRecordResources::new(
                    &frame.pipelines,
                    &pipeline_handles,
                    &pipeline_layouts,
                    &descriptor_sets,
                    &[vk::Buffer::null()],
                ),
                AshCommandRecordHandleAccess {
                    buffer: |buffer: &vk::Buffer| *buffer,
                },
            ),
            Ok(AshResolvedCommand::DrawIndexed(AshDrawIndexedCommand {
                index_count: 12,
                instance_count: 1,
                first_index: 2,
                vertex_offset: -3,
                first_instance: 4,
            }))
        );
        assert_eq!(
            bind_pipeline.draw_indexed_args(),
            Err("drawable command is not an indexed draw: BindGraphicsPipeline { pipeline_index: 0 }"
                .to_owned())
        );
    }

    #[test]
    fn drawable_frame_options_override_clear_values() {
        let drawable = ash_drawable_frame_from_renderer_frame_with_options(
            &AshRendererFrame::default(),
            vk::Extent2D {
                width: 4,
                height: 2,
            },
            AshDrawableFrameOptions {
                color_clear: [0.1, 0.2, 0.3, 0.4],
                depth_stencil_clear: Some(AshDepthStencilClear {
                    depth: 0.75,
                    stencil: 2,
                }),
            },
        );

        assert_eq!(drawable.render_pass.color_clear, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(
            drawable.render_pass.depth_stencil_clear,
            Some(AshDepthStencilClear {
                depth: 0.75,
                stencil: 2,
            })
        );
        assert_eq!(drawable.render_pass.render_area.extent.width, 4);
        assert_eq!(drawable.render_pass.render_area.extent.height, 2);
    }

    #[test]
    fn graphics_pipeline_state_plan_exposes_fixed_function_state() {
        let pipeline = AshGraphicsPipelinePlan {
            material: MaterialRef(3),
            pipeline_plan_index: 9,
            descriptor_set_index: 2,
            key: AshPipelineKey {
                pass: AshMtoonPass::Base,
                render_order: 2000,
                phase_order: 2000,
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                cull_mode: vk::CullModeFlags::FRONT,
                front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                depth_test_enable: true,
                depth_write_enable: false,
                depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                blend_enable: true,
            },
            vertex_stride: 64,
            vertex_attributes: vec![AshVertexAttributePlan {
                location: 3,
                binding: 0,
                format: vk::Format::R32G32_SFLOAT,
                offset: 16,
            }],
            color_format: vk::Format::R8G8B8A8_UNORM,
            depth_format: Some(ash_reference_depth_format()),
        };

        let state = ash_graphics_pipeline_state_plan(
            &pipeline,
            vk::Extent2D {
                width: 320,
                height: 180,
            },
        );

        assert_eq!(state.descriptor_set_index, 2);
        assert_eq!(state.vertex_binding.stride, 64);
        assert_eq!(state.vertex_attributes.len(), 1);
        assert_eq!(state.vertex_attributes[0].location, 3);
        assert_eq!(state.topology, vk::PrimitiveTopology::TRIANGLE_LIST);
        assert_eq!(state.viewport.width, 320.0);
        assert_eq!(state.viewport.height, 180.0);
        assert_eq!(state.scissor.extent.width, 320);
        assert_eq!(state.cull_mode, vk::CullModeFlags::FRONT);
        assert!(state.depth_test_enable);
        assert!(!state.depth_write_enable);
        assert_eq!(state.color_blend_attachment.blend_enable, vk::TRUE);
        assert_eq!(
            state.color_blend_attachment.src_color_blend_factor,
            vk::BlendFactor::SRC_ALPHA
        );

        let debug_state = ash_position_color_pipeline_state_plan(
            vk::Extent2D {
                width: 64,
                height: 32,
            },
            28,
            0,
            12,
        );
        assert_eq!(debug_state.vertex_binding.stride, 28);
        assert_eq!(debug_state.vertex_attributes.len(), 2);
        assert_eq!(debug_state.vertex_attributes[0].location, 0);
        assert_eq!(
            debug_state.vertex_attributes[0].format,
            vk::Format::R32G32B32_SFLOAT
        );
        assert_eq!(debug_state.vertex_attributes[1].location, 1);
        assert_eq!(debug_state.vertex_attributes[1].offset, 12);
        assert_eq!(
            debug_state.vertex_attributes[1].format,
            vk::Format::R32G32B32A32_SFLOAT
        );
        assert_eq!(debug_state.viewport.width, 64.0);
        assert_eq!(debug_state.scissor.extent.height, 32);
        assert_eq!(debug_state.cull_mode, vk::CullModeFlags::NONE);
        assert_eq!(debug_state.depth_compare_op, vk::CompareOp::LESS_OR_EQUAL);
    }

    #[test]
    fn graphics_pipeline_create_info_plan_wraps_state_and_layout_lookup() {
        let pipeline = AshGraphicsPipelinePlan {
            material: MaterialRef(3),
            pipeline_plan_index: 9,
            descriptor_set_index: 1,
            key: AshPipelineKey {
                pass: AshMtoonPass::Base,
                render_order: 2000,
                phase_order: 2000,
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                cull_mode: vk::CullModeFlags::FRONT,
                front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                depth_test_enable: true,
                depth_write_enable: false,
                depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                blend_enable: true,
            },
            vertex_stride: 64,
            vertex_attributes: vec![AshVertexAttributePlan {
                location: 3,
                binding: 0,
                format: vk::Format::R32G32_SFLOAT,
                offset: 16,
            }],
            color_format: vk::Format::R8G8B8A8_UNORM,
            depth_format: Some(ash_reference_depth_format()),
        };
        let vertex_entry = std::ffi::CString::new("vs_main").unwrap();
        let fragment_entry = std::ffi::CString::new("fs_main").unwrap();
        let shader_stages =
            ash_graphics_shader_stages_plan(vk::ShaderModule::null(), vk::ShaderModule::null())
                .shader_stage_create_infos(&vertex_entry, &fragment_entry);
        let layouts = [vk::PipelineLayout::null(), vk::PipelineLayout::null()];
        let plan = ash_graphics_pipeline_create_info_plan(
            &pipeline,
            vk::Extent2D {
                width: 320,
                height: 180,
            },
            shader_stages,
            &layouts,
            vk::RenderPass::null(),
        )
        .expect("valid descriptor-set layout index");

        assert_eq!(plan.state.descriptor_set_index, 1);
        assert_eq!(plan.layout, layouts[1]);
        plan.with_graphics_pipeline_create_info(|info| {
            assert_eq!(info.stage_count, 2);
            assert_eq!(info.p_stages, plan.shader_stages.as_ptr());
            assert_eq!(info.layout, layouts[1]);
            assert_eq!(info.render_pass, vk::RenderPass::null());
            assert_eq!(info.subpass, 0);
        });

        let mut out_of_range = pipeline.clone();
        out_of_range.descriptor_set_index = 2;
        let error = ash_graphics_pipeline_create_info_plan(
            &out_of_range,
            vk::Extent2D {
                width: 320,
                height: 180,
            },
            shader_stages,
            &layouts,
            vk::RenderPass::null(),
        )
        .expect_err("descriptor-set index should be checked");
        assert!(error.contains("out of range"));
    }

    #[test]
    fn swapchain_surface_plan_prefers_unorm_mailbox_and_clamps_extent() {
        let capabilities = vk::SurfaceCapabilitiesKHR {
            min_image_count: 2,
            max_image_count: 3,
            current_extent: vk::Extent2D {
                width: u32::MAX,
                height: u32::MAX,
            },
            min_image_extent: vk::Extent2D {
                width: 320,
                height: 200,
            },
            max_image_extent: vk::Extent2D {
                width: 1280,
                height: 720,
            },
            current_transform: vk::SurfaceTransformFlagsKHR::IDENTITY,
            supported_usage_flags: vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::TRANSFER_DST,
            supported_composite_alpha: vk::CompositeAlphaFlagsKHR::OPAQUE
                | vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            ..Default::default()
        };
        let formats = [
            vk::SurfaceFormatKHR {
                format: vk::Format::R8G8B8A8_SRGB,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            },
            vk::SurfaceFormatKHR {
                format: vk::Format::B8G8R8A8_UNORM,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            },
        ];
        let plan = ash_swapchain_surface_plan(
            capabilities,
            &formats,
            &[vk::PresentModeKHR::FIFO, vk::PresentModeKHR::MAILBOX],
            vk::Extent2D {
                width: 2048,
                height: 128,
            },
        )
        .unwrap();

        assert_eq!(plan.format.format, vk::Format::B8G8R8A8_UNORM);
        assert_eq!(plan.present_mode, vk::PresentModeKHR::MAILBOX);
        assert_eq!(plan.extent.width, 1280);
        assert_eq!(plan.extent.height, 200);
        assert_eq!(plan.image_count, 3);
        assert_eq!(plan.pre_transform, vk::SurfaceTransformFlagsKHR::IDENTITY);
        assert_eq!(plan.composite_alpha, vk::CompositeAlphaFlagsKHR::OPAQUE);
        assert_eq!(plan.image_usage, vk::ImageUsageFlags::COLOR_ATTACHMENT);
    }

    #[test]
    fn swapchain_surface_plan_uses_fixed_current_extent_and_fifo_fallback() {
        let capabilities = vk::SurfaceCapabilitiesKHR {
            min_image_count: 1,
            max_image_count: 0,
            current_extent: vk::Extent2D {
                width: 640,
                height: 480,
            },
            current_transform: vk::SurfaceTransformFlagsKHR::ROTATE_90,
            supported_usage_flags: vk::ImageUsageFlags::COLOR_ATTACHMENT,
            supported_composite_alpha: vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            ..Default::default()
        };
        let formats = [vk::SurfaceFormatKHR {
            format: vk::Format::A2B10G10R10_UNORM_PACK32,
            color_space: vk::ColorSpaceKHR::HDR10_ST2084_EXT,
        }];

        let plan = ash_swapchain_surface_plan(
            capabilities,
            &formats,
            &[vk::PresentModeKHR::IMMEDIATE],
            vk::Extent2D {
                width: 100,
                height: 100,
            },
        )
        .unwrap();

        assert_eq!(plan.format.format, vk::Format::A2B10G10R10_UNORM_PACK32);
        assert_eq!(plan.present_mode, vk::PresentModeKHR::FIFO);
        assert_eq!(plan.extent.width, 640);
        assert_eq!(plan.extent.height, 480);
        assert_eq!(plan.image_count, 2);
        assert_eq!(plan.pre_transform, vk::SurfaceTransformFlagsKHR::ROTATE_90);
        assert_eq!(
            plan.composite_alpha,
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED
        );
        assert!(ash_swapchain_surface_plan(capabilities, &[], &[], plan.extent).is_err());
    }

    #[test]
    fn swapchain_surface_plan_rejects_unsupported_color_attachment_usage() {
        let capabilities = vk::SurfaceCapabilitiesKHR {
            min_image_count: 1,
            max_image_count: 2,
            current_extent: vk::Extent2D {
                width: 320,
                height: 240,
            },
            supported_usage_flags: vk::ImageUsageFlags::TRANSFER_DST,
            supported_composite_alpha: vk::CompositeAlphaFlagsKHR::OPAQUE,
            ..Default::default()
        };
        let formats = [vk::SurfaceFormatKHR {
            format: vk::Format::B8G8R8A8_UNORM,
            color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
        }];

        let err = ash_swapchain_surface_plan(
            capabilities,
            &formats,
            &[vk::PresentModeKHR::FIFO],
            vk::Extent2D {
                width: 320,
                height: 240,
            },
        )
        .unwrap_err();

        assert!(err.contains("does not support required"));
    }

    #[test]
    fn swapchain_surface_plan_rejects_missing_composite_alpha_support() {
        let capabilities = vk::SurfaceCapabilitiesKHR {
            min_image_count: 1,
            max_image_count: 2,
            current_extent: vk::Extent2D {
                width: 320,
                height: 240,
            },
            supported_usage_flags: vk::ImageUsageFlags::COLOR_ATTACHMENT,
            supported_composite_alpha: vk::CompositeAlphaFlagsKHR::empty(),
            ..Default::default()
        };
        let formats = [vk::SurfaceFormatKHR {
            format: vk::Format::B8G8R8A8_UNORM,
            color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
        }];

        assert_eq!(
            ash_swapchain_surface_plan(
                capabilities,
                &formats,
                &[vk::PresentModeKHR::FIFO],
                vk::Extent2D {
                    width: 320,
                    height: 240,
                },
            ),
            Err("surface reports no supported composite alpha mode".to_owned())
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
            descriptor_bindings: descriptor_bindings(
                &[],
                GltfMaterialTextureSlots::default(),
                AshDescriptorBindingModel::SeparateImageSampler,
            ),
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
