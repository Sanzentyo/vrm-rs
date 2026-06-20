//! wgpu-style MToon pipeline materialization.
//!
//! This example is dependency-free so it can run in the normal local gate
//! without requesting an adapter. The `Wgpu*` types mirror the concrete wgpu
//! concepts a renderer would fill: render-pipeline keys, bind-group layouts,
//! sampler descriptors, texture bindings, and uniform payloads.

use vrm_adapter::{
    MTOON_GPU_UNIFORM_SIZE, MTOON_REFERENCE_WGSL, MtoonGpuMaterial, MtoonGpuUniform,
    MtoonMaterializationOptions, MtoonRendererMaterialPlan, MtoonRendererPass, MtoonSamplerHint,
    MtoonTextureBindingPlan, MtoonTextureSlot, RENDER_OWNER_SAMPLE_OVERRIDE_BINDING,
    RendererMaterialAlphaMode, RendererMaterialCullMode, RendererMaterialPipelinePlan,
    mtoon_gpu_sampler_binding_number, mtoon_gpu_texture_binding_number,
    mtoon_renderer_material_plans,
};
use vrm_core::{
    EmissiveStrength, Feature, Material, MaterialRef, MtoonCullMode, MtoonMaterial,
    MtoonRenderQueue, MtoonTextureSet, OutlineWidthMode, TextureRef, UvAnimation, VrmDocument,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WgpuShaderStages {
    Fragment,
    VertexFragment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WgpuBindingType {
    UniformBuffer,
    StorageBuffer,
    Texture,
    Sampler,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WgpuTextureSampleType {
    FloatFilterable,
    NormalMap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WgpuAddressMode {
    Repeat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WgpuFilterMode {
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WgpuCullMode {
    None,
    Front,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WgpuCompareFunction {
    Always,
    LessEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WgpuBlendState {
    Replace,
    AlphaBlending,
    AlphaCutout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WgpuMtoonPass {
    Base,
    Outline,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct WgpuBindGroupLayoutEntry {
    binding: u32,
    visibility: WgpuShaderStages,
    binding_type: WgpuBindingType,
    texture_slot: Option<MtoonTextureSlot>,
    sample_type: Option<WgpuTextureSampleType>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WgpuPrimitiveState {
    cull_mode: WgpuCullMode,
    front_face_ccw: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WgpuDepthStencilState {
    depth_write_enabled: bool,
    depth_compare: WgpuCompareFunction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WgpuPipelineKey {
    pass: WgpuMtoonPass,
    render_order: i32,
    phase_order: i32,
    primitive: WgpuPrimitiveState,
    depth_stencil: WgpuDepthStencilState,
    blend: WgpuBlendState,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct WgpuRenderPipelineDescriptor {
    material: MaterialRef,
    key: WgpuPipelineKey,
    vertex_module: &'static str,
    fragment_module: &'static str,
    vertex_entry: &'static str,
    fragment_entry: &'static str,
    bind_group_layout: Vec<WgpuBindGroupLayoutEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WgpuSamplerDescriptor {
    address_mode_u: WgpuAddressMode,
    address_mode_v: WgpuAddressMode,
    mag_filter: WgpuFilterMode,
    min_filter: WgpuFilterMode,
    normal_map_decode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WgpuTextureBinding {
    texture_binding: u32,
    sampler_binding: u32,
    texture: TextureRef,
    sampler: WgpuSamplerDescriptor,
    slot: MtoonTextureSlot,
}

#[derive(Clone, Debug, PartialEq)]
struct WgpuMaterialBindGroupRecipe {
    material: MaterialRef,
    uniform: MtoonGpuUniform,
    uniform_size: usize,
    reference_wgsl: &'static str,
    texture_bindings: Vec<WgpuTextureBinding>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct WgpuMtoonPipelineTable {
    pipelines: Vec<WgpuRenderPipelineDescriptor>,
    material_bind_groups: Vec<WgpuMaterialBindGroupRecipe>,
}

impl WgpuMtoonPipelineTable {
    fn from_document(document: &VrmDocument) -> Self {
        let plans = mtoon_renderer_material_plans(document, MtoonMaterializationOptions::default());
        let mut pipelines = plans
            .iter()
            .map(wgpu_pipeline_descriptor)
            .collect::<Vec<_>>();
        pipelines.sort_by_key(|pipeline| {
            (
                pipeline.key.phase_order,
                pipeline.key.render_order,
                pass_sort_key(pipeline.key.pass),
                pipeline.material.0,
            )
        });
        let material_bind_groups = plans
            .iter()
            .filter(|plan| plan.pass == MtoonRendererPass::Base)
            .map(wgpu_material_bind_group)
            .collect();
        Self {
            pipelines,
            material_bind_groups,
        }
    }
}

fn wgpu_pipeline_descriptor(plan: &MtoonRendererMaterialPlan) -> WgpuRenderPipelineDescriptor {
    let primitive = RendererMaterialPipelinePlan::from_mtoon_plan(plan);
    WgpuRenderPipelineDescriptor {
        material: plan.material,
        key: WgpuPipelineKey {
            pass: wgpu_pass(plan.pass),
            render_order: primitive.render_order,
            phase_order: primitive.phase_order,
            primitive: WgpuPrimitiveState {
                cull_mode: wgpu_cull_mode(primitive.cull_mode),
                front_face_ccw: true,
            },
            depth_stencil: WgpuDepthStencilState {
                depth_write_enabled: primitive.depth_write,
                depth_compare: if plan.pipeline.depth_test {
                    WgpuCompareFunction::LessEqual
                } else {
                    WgpuCompareFunction::Always
                },
            },
            blend: wgpu_blend_state(primitive.alpha_mode, primitive.blend),
        },
        vertex_module: match plan.pass {
            MtoonRendererPass::Base => "mtoon_base.wgsl",
            MtoonRendererPass::Outline => "mtoon_outline.wgsl",
        },
        fragment_module: match plan.pass {
            MtoonRendererPass::Base => "mtoon_base.wgsl",
            MtoonRendererPass::Outline => "mtoon_outline.wgsl",
        },
        vertex_entry: match plan.pass {
            MtoonRendererPass::Base => "vs_main",
            MtoonRendererPass::Outline => "vs_outline",
        },
        fragment_entry: match plan.pass {
            MtoonRendererPass::Base => "fs_main",
            MtoonRendererPass::Outline => "fs_outline",
        },
        bind_group_layout: bind_group_layout(&plan.texture_bindings),
    }
}

fn wgpu_material_bind_group(plan: &MtoonRendererMaterialPlan) -> WgpuMaterialBindGroupRecipe {
    let gpu = MtoonGpuMaterial::from_renderer_plan(plan);
    WgpuMaterialBindGroupRecipe {
        material: plan.material,
        uniform: gpu.uniform,
        uniform_size: gpu.uniform_bytes().len(),
        reference_wgsl: MTOON_REFERENCE_WGSL,
        texture_bindings: texture_bindings(&plan.texture_bindings),
    }
}

fn bind_group_layout(bindings: &[MtoonTextureBindingPlan]) -> Vec<WgpuBindGroupLayoutEntry> {
    std::iter::once(WgpuBindGroupLayoutEntry {
        binding: 0,
        visibility: WgpuShaderStages::VertexFragment,
        binding_type: WgpuBindingType::UniformBuffer,
        texture_slot: None,
        sample_type: None,
    })
    .chain(std::iter::once(WgpuBindGroupLayoutEntry {
        binding: owner_sample_override_binding(),
        visibility: WgpuShaderStages::Fragment,
        binding_type: WgpuBindingType::StorageBuffer,
        texture_slot: None,
        sample_type: None,
    }))
    .chain(bindings.iter().enumerate().flat_map(|(index, binding)| {
        let texture_binding = texture_binding_number(index);
        let sampler_binding = sampler_binding_number(index);
        [
            WgpuBindGroupLayoutEntry {
                binding: texture_binding,
                visibility: texture_visibility(binding.slot),
                binding_type: WgpuBindingType::Texture,
                texture_slot: Some(binding.slot),
                sample_type: Some(texture_sample_type(binding.sampler)),
            },
            WgpuBindGroupLayoutEntry {
                binding: sampler_binding,
                visibility: texture_visibility(binding.slot),
                binding_type: WgpuBindingType::Sampler,
                texture_slot: Some(binding.slot),
                sample_type: None,
            },
        ]
    }))
    .collect()
}

fn texture_bindings(bindings: &[MtoonTextureBindingPlan]) -> Vec<WgpuTextureBinding> {
    bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| WgpuTextureBinding {
            texture_binding: texture_binding_number(index),
            sampler_binding: sampler_binding_number(index),
            texture: binding.texture,
            sampler: wgpu_sampler(binding.sampler),
            slot: binding.slot,
        })
        .collect()
}

fn texture_binding_number(index: usize) -> u32 {
    mtoon_gpu_texture_binding_number(index)
}

fn sampler_binding_number(index: usize) -> u32 {
    mtoon_gpu_sampler_binding_number(index)
}

fn owner_sample_override_binding() -> u32 {
    RENDER_OWNER_SAMPLE_OVERRIDE_BINDING
}

fn texture_visibility(slot: MtoonTextureSlot) -> WgpuShaderStages {
    match slot {
        MtoonTextureSlot::OutlineWidth => WgpuShaderStages::VertexFragment,
        MtoonTextureSlot::Main
        | MtoonTextureSlot::ShadeMultiply
        | MtoonTextureSlot::ShadingShift
        | MtoonTextureSlot::Normal
        | MtoonTextureSlot::Matcap
        | MtoonTextureSlot::RimMultiply
        | MtoonTextureSlot::UvAnimationMask => WgpuShaderStages::Fragment,
    }
}

fn texture_sample_type(sampler: MtoonSamplerHint) -> WgpuTextureSampleType {
    match sampler {
        MtoonSamplerHint::LinearRepeat => WgpuTextureSampleType::FloatFilterable,
        MtoonSamplerHint::NormalMapLinearRepeat => WgpuTextureSampleType::NormalMap,
    }
}

fn wgpu_sampler(sampler: MtoonSamplerHint) -> WgpuSamplerDescriptor {
    WgpuSamplerDescriptor {
        address_mode_u: WgpuAddressMode::Repeat,
        address_mode_v: WgpuAddressMode::Repeat,
        mag_filter: WgpuFilterMode::Linear,
        min_filter: WgpuFilterMode::Linear,
        normal_map_decode: matches!(sampler, MtoonSamplerHint::NormalMapLinearRepeat),
    }
}

fn wgpu_pass(pass: MtoonRendererPass) -> WgpuMtoonPass {
    match pass {
        MtoonRendererPass::Base => WgpuMtoonPass::Base,
        MtoonRendererPass::Outline => WgpuMtoonPass::Outline,
    }
}

fn pass_sort_key(pass: WgpuMtoonPass) -> u8 {
    match pass {
        WgpuMtoonPass::Base => 0,
        WgpuMtoonPass::Outline => 1,
    }
}

fn wgpu_cull_mode(cull: RendererMaterialCullMode) -> WgpuCullMode {
    match cull {
        RendererMaterialCullMode::Off => WgpuCullMode::None,
        RendererMaterialCullMode::Front => WgpuCullMode::Front,
        RendererMaterialCullMode::Back => WgpuCullMode::Back,
    }
}

fn wgpu_blend_state(alpha_mode: RendererMaterialAlphaMode, blend: bool) -> WgpuBlendState {
    match (alpha_mode, blend) {
        (RendererMaterialAlphaMode::Blend, true) => WgpuBlendState::AlphaBlending,
        (RendererMaterialAlphaMode::Mask, _) => WgpuBlendState::AlphaCutout,
        _ => WgpuBlendState::Replace,
    }
}

fn sample_document() -> VrmDocument {
    VrmDocument {
        materials: vec![Material {
            khr_emissive_strength: Feature::Present(EmissiveStrength(2.5)),
            mtoon: Feature::Present(MtoonMaterial {
                render_queue: MtoonRenderQueue::Transparent,
                transparent_with_z_write: true,
                cull_mode: MtoonCullMode::Off,
                base_color_factor: [0.25, 0.6, 1.0, 0.45],
                shade_color_factor: [0.05, 0.12, 0.22],
                emissive_factor: [0.2, 0.1, 0.05],
                cutoff_factor: 0.4,
                outline_width_mode: OutlineWidthMode::WorldCoordinates,
                outline_width_factor: 0.015,
                rim_lighting_mix_factor: 0.35,
                parametric_rim_fresnel_power_factor: 2.0,
                parametric_rim_lift_factor: 0.2,
                outline_lighting_mix_factor: 0.4,
                uv_animation: UvAnimation {
                    scroll_x_speed: 0.25,
                    scroll_y_speed: -0.5,
                    rotation_speed: 0.125,
                },
                textures: MtoonTextureSet {
                    main_texture: Some(TextureRef(1)),
                    shade_multiply_texture: Some(TextureRef(2)),
                    shading_shift_texture: Some(TextureRef(3)),
                    normal_texture: Some(TextureRef(4)),
                    matcap_texture: Some(TextureRef(5)),
                    rim_multiply_texture: Some(TextureRef(6)),
                    outline_width_multiply_texture: Some(TextureRef(7)),
                    uv_animation_mask_texture: Some(TextureRef(8)),
                },
                ..MtoonMaterial::default()
            }),
            ..Material::default()
        }],
        ..VrmDocument::default()
    }
}

fn main() {
    let table = WgpuMtoonPipelineTable::from_document(&sample_document());
    assert_eq!(table.pipelines.len(), 2);
    assert_eq!(table.material_bind_groups.len(), 1);
    assert_eq!(table.pipelines[0].key.pass, WgpuMtoonPass::Base);
    assert_eq!(table.pipelines[0].key.blend, WgpuBlendState::AlphaBlending);
    assert!(table.pipelines[0].key.depth_stencil.depth_write_enabled);
    assert_eq!(table.material_bind_groups[0].texture_bindings.len(), 8);
    assert_eq!(
        table.material_bind_groups[0].uniform_size,
        MTOON_GPU_UNIFORM_SIZE
    );
    assert_eq!(
        table.material_bind_groups[0]
            .uniform
            .emissive_color_outline_width[0..3],
        [0.5, 0.25, 0.125]
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materializes_base_and_outline_render_pipelines() {
        let table = WgpuMtoonPipelineTable::from_document(&sample_document());
        let base = &table.pipelines[0];
        let outline = &table.pipelines[1];

        assert_eq!(base.key.pass, WgpuMtoonPass::Base);
        assert_eq!(outline.key.pass, WgpuMtoonPass::Outline);
        assert_eq!(base.key.blend, WgpuBlendState::AlphaBlending);
        assert_eq!(outline.key.blend, WgpuBlendState::Replace);
        assert_eq!(base.key.primitive.cull_mode, WgpuCullMode::None);
        assert_eq!(outline.key.primitive.cull_mode, WgpuCullMode::Front);
        assert_eq!(base.fragment_entry, "fs_main");
        assert_eq!(outline.vertex_entry, "vs_outline");
    }

    #[test]
    fn exposes_wgpu_bind_group_layout_slots() {
        let table = WgpuMtoonPipelineTable::from_document(&sample_document());
        let layout = &table.pipelines[0].bind_group_layout;

        assert_eq!(layout[0].binding_type, WgpuBindingType::UniformBuffer);
        assert_eq!(layout[1].binding, owner_sample_override_binding());
        assert_eq!(layout[1].binding_type, WgpuBindingType::StorageBuffer);
        assert_eq!(layout[1].visibility, WgpuShaderStages::Fragment);
        assert_eq!(layout[2].texture_slot, Some(MtoonTextureSlot::Main));
        assert_eq!(layout[3].binding_type, WgpuBindingType::Sampler);
        assert_eq!(layout[8].texture_slot, Some(MtoonTextureSlot::Normal));
        assert_eq!(
            layout[8].sample_type,
            Some(WgpuTextureSampleType::NormalMap)
        );
        assert_eq!(
            layout[14].texture_slot,
            Some(MtoonTextureSlot::OutlineWidth)
        );
        assert_eq!(layout[14].visibility, WgpuShaderStages::VertexFragment);
        assert_eq!(
            layout
                .iter()
                .filter(|entry| entry.binding == owner_sample_override_binding())
                .count(),
            1
        );
    }

    #[test]
    fn carries_material_uniforms_and_texture_bindings() {
        let table = WgpuMtoonPipelineTable::from_document(&sample_document());
        let bind_group = &table.material_bind_groups[0];

        assert_eq!(bind_group.uniform.base_color_factor, [0.25, 0.6, 1.0, 0.45]);
        assert_eq!(
            bind_group.uniform.emissive_color_outline_width[0..3],
            [0.5, 0.25, 0.125]
        );
        assert_eq!(bind_group.uniform.shade_color_factor_cutoff[3], 0.4);
        assert_eq!(bind_group.uniform.uv_animation, [0.25, -0.5, 0.125, 0.0]);
        assert_eq!(bind_group.uniform_size, MTOON_GPU_UNIFORM_SIZE);
        assert!(bind_group.reference_wgsl.contains("struct MtoonGpuUniform"));
        assert_eq!(
            bind_group.texture_bindings[3].slot,
            MtoonTextureSlot::Normal
        );
        assert!(bind_group.texture_bindings[3].sampler.normal_map_decode);
        assert_eq!(bind_group.texture_bindings[7].texture, TextureRef(8));
    }
}
