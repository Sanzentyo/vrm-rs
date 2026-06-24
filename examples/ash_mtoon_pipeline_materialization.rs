//! Ash/Vulkan-style MToon pipeline materialization.
//!
//! This example stays dependency-free so it can be compiled in the normal local
//! gate without requiring a Vulkan SDK. The local `Vk*` types mirror the Vulkan
//! concepts an `ash` backend would fill (`vk::Pipeline*CreateInfo`,
//! descriptor-set layouts, descriptor writes, and push constants). A real
//! renderer can replace these small enums/structs with `ash::vk` values at the
//! same conversion points.

use vrm_adapter::{
    MTOON_GPU_UNIFORM_SIZE, MTOON_REFERENCE_WGSL, MtoonGpuMaterial, MtoonGpuUniform,
    MtoonMaterializationOptions, MtoonRendererMaterialPlan, MtoonRendererPass, MtoonSamplerHint,
    MtoonTextureBindingPlan, MtoonTextureSlot, RENDER_OWNER_SAMPLE_OVERRIDE_BINDING,
    RendererMaterialAlphaMode, RendererMaterialCullMode, RendererMaterialPipelinePlan,
    mtoon_gpu_sampler_binding_number, mtoon_gpu_texture_binding_number,
    mtoon_renderer_material_plans,
};
use vrm_core::{
    EmissiveStrength, Feature, Material, MaterialRef, MtoonMaterial, MtoonRenderQueue,
    MtoonTextureSet, OutlineWidthMode, TextureRef, UvAnimation, VrmDocument,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VkShaderStage {
    Fragment,
    VertexFragment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VkDescriptorType {
    UniformBuffer,
    StorageBuffer,
    SampledImage,
    Sampler,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VkCullMode {
    None,
    Front,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VkFrontFace {
    CounterClockwise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VkCompareOp {
    Always,
    LessOrEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VkBlendPreset {
    Disabled,
    AlphaBlend,
    AlphaCutout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VkMtoonPass {
    Base,
    Outline,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct VkDescriptorSetLayoutBinding {
    binding: u32,
    descriptor_type: VkDescriptorType,
    stage_flags: VkShaderStage,
    slot: Option<MtoonTextureSlot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct VkPushConstants {
    size_bytes: u32,
    alpha_cutoff: f32,
    outline_width: f32,
    uv_animation: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct VkDepthStencilState {
    depth_test_enable: bool,
    depth_write_enable: bool,
    compare_op: VkCompareOp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct VkRasterizationState {
    cull_mode: VkCullMode,
    front_face: VkFrontFace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct VkPipelineKey {
    pass: VkMtoonPass,
    render_order: i32,
    phase_order: i32,
    rasterization: VkRasterizationState,
    depth_stencil: VkDepthStencilState,
    blend: VkBlendPreset,
}

#[derive(Clone, Debug, PartialEq)]
struct VkGraphicsPipelineRecipe {
    material: MaterialRef,
    key: VkPipelineKey,
    vertex_shader_spv: &'static str,
    fragment_shader_spv: &'static str,
    descriptor_set_layout: Vec<VkDescriptorSetLayoutBinding>,
    push_constants: VkPushConstants,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct VkSamplerKey {
    mag_linear: bool,
    min_linear: bool,
    repeat_u: bool,
    repeat_v: bool,
    normal_map_decode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct VkImageBinding {
    image_binding: u32,
    sampler_binding: u32,
    texture: TextureRef,
    sampler: VkSamplerKey,
    slot: MtoonTextureSlot,
}

#[derive(Clone, Debug, PartialEq)]
struct VkMtoonMaterialRecord {
    material: MaterialRef,
    uniform: MtoonGpuUniform,
    uniform_size: usize,
    reference_wgsl: &'static str,
    image_bindings: Vec<VkImageBinding>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct AshMtoonPipelineTable {
    pipelines: Vec<VkGraphicsPipelineRecipe>,
    materials: Vec<VkMtoonMaterialRecord>,
}

impl AshMtoonPipelineTable {
    fn from_document(document: &VrmDocument) -> Self {
        let plans = mtoon_renderer_material_plans(document, MtoonMaterializationOptions::default());
        let mut pipelines = plans.iter().map(vulkan_pipeline_recipe).collect::<Vec<_>>();
        pipelines.sort_by_key(|recipe| {
            (
                recipe.key.phase_order,
                recipe.key.render_order,
                pass_sort_key(recipe.key.pass),
                recipe.material.0,
            )
        });

        let materials = plans
            .iter()
            .filter(|plan| plan.pass == MtoonRendererPass::Base)
            .map(vulkan_material_record)
            .collect();

        Self {
            pipelines,
            materials,
        }
    }
}

fn vulkan_pipeline_recipe(plan: &MtoonRendererMaterialPlan) -> VkGraphicsPipelineRecipe {
    let primitive = RendererMaterialPipelinePlan::from_mtoon_plan(plan);
    VkGraphicsPipelineRecipe {
        material: plan.material,
        key: VkPipelineKey {
            pass: vk_pass(plan.pass),
            render_order: primitive.render_order,
            phase_order: primitive.phase_order,
            rasterization: VkRasterizationState {
                cull_mode: vk_cull_mode(primitive.cull_mode),
                front_face: VkFrontFace::CounterClockwise,
            },
            depth_stencil: VkDepthStencilState {
                depth_test_enable: plan.pipeline.depth_test,
                depth_write_enable: primitive.depth_write,
                compare_op: if plan.pipeline.depth_test {
                    VkCompareOp::LessOrEqual
                } else {
                    VkCompareOp::Always
                },
            },
            blend: vk_blend(primitive.alpha_mode, primitive.blend),
        },
        vertex_shader_spv: match plan.pass {
            MtoonRendererPass::Base => "shaders/mtoon_base.wgsl.vert.spv",
            MtoonRendererPass::Outline => "shaders/mtoon_outline.wgsl.vert.spv",
        },
        fragment_shader_spv: match plan.pass {
            MtoonRendererPass::Base => "shaders/mtoon_base.wgsl.frag.spv",
            MtoonRendererPass::Outline => "shaders/mtoon_outline.wgsl.frag.spv",
        },
        descriptor_set_layout: descriptor_set_layout(&plan.texture_bindings),
        push_constants: VkPushConstants {
            size_bytes: 32,
            alpha_cutoff: primitive.alpha_cutoff,
            outline_width: plan.shader.outline_width_factor,
            uv_animation: uv_animation_constants(plan.shader.uv_animation),
        },
    }
}

fn vulkan_material_record(plan: &MtoonRendererMaterialPlan) -> VkMtoonMaterialRecord {
    let gpu = MtoonGpuMaterial::from_renderer_plan(plan);
    VkMtoonMaterialRecord {
        material: plan.material,
        uniform: gpu.uniform,
        uniform_size: gpu.uniform_bytes().len(),
        reference_wgsl: MTOON_REFERENCE_WGSL,
        image_bindings: image_bindings(&plan.texture_bindings),
    }
}

fn descriptor_set_layout(
    bindings: &[MtoonTextureBindingPlan],
) -> Vec<VkDescriptorSetLayoutBinding> {
    std::iter::once(VkDescriptorSetLayoutBinding {
        binding: 0,
        descriptor_type: VkDescriptorType::UniformBuffer,
        stage_flags: VkShaderStage::VertexFragment,
        slot: None,
    })
    .chain(std::iter::once(VkDescriptorSetLayoutBinding {
        binding: owner_sample_override_binding(),
        descriptor_type: VkDescriptorType::StorageBuffer,
        stage_flags: VkShaderStage::Fragment,
        slot: None,
    }))
    .chain(bindings.iter().enumerate().flat_map(|(index, binding)| {
        [
            VkDescriptorSetLayoutBinding {
                binding: texture_binding_number(index),
                descriptor_type: VkDescriptorType::SampledImage,
                stage_flags: texture_stage_flags(binding.slot),
                slot: Some(binding.slot),
            },
            VkDescriptorSetLayoutBinding {
                binding: sampler_binding_number(index),
                descriptor_type: VkDescriptorType::Sampler,
                stage_flags: texture_stage_flags(binding.slot),
                slot: Some(binding.slot),
            },
        ]
    }))
    .collect()
}

fn image_bindings(bindings: &[MtoonTextureBindingPlan]) -> Vec<VkImageBinding> {
    bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| VkImageBinding {
            image_binding: texture_binding_number(index),
            sampler_binding: sampler_binding_number(index),
            texture: binding.texture,
            sampler: vk_sampler(binding.sampler),
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

fn texture_stage_flags(slot: MtoonTextureSlot) -> VkShaderStage {
    match slot {
        MtoonTextureSlot::OutlineWidth => VkShaderStage::VertexFragment,
        MtoonTextureSlot::Normal
        | MtoonTextureSlot::Main
        | MtoonTextureSlot::ShadeMultiply
        | MtoonTextureSlot::ShadingShift
        | MtoonTextureSlot::Matcap
        | MtoonTextureSlot::RimMultiply
        | MtoonTextureSlot::UvAnimationMask => VkShaderStage::Fragment,
    }
}

fn vk_sampler(sampler: MtoonSamplerHint) -> VkSamplerKey {
    match sampler {
        MtoonSamplerHint::LinearRepeat => VkSamplerKey {
            mag_linear: true,
            min_linear: true,
            repeat_u: true,
            repeat_v: true,
            normal_map_decode: false,
        },
        MtoonSamplerHint::NormalMapLinearRepeat => VkSamplerKey {
            normal_map_decode: true,
            ..vk_sampler(MtoonSamplerHint::LinearRepeat)
        },
    }
}

fn vk_pass(pass: MtoonRendererPass) -> VkMtoonPass {
    match pass {
        MtoonRendererPass::Base => VkMtoonPass::Base,
        MtoonRendererPass::Outline => VkMtoonPass::Outline,
    }
}

fn pass_sort_key(pass: VkMtoonPass) -> u8 {
    match pass {
        VkMtoonPass::Base => 0,
        VkMtoonPass::Outline => 1,
    }
}

fn vk_cull_mode(cull: RendererMaterialCullMode) -> VkCullMode {
    match cull {
        RendererMaterialCullMode::Off => VkCullMode::None,
        RendererMaterialCullMode::Front => VkCullMode::Front,
        RendererMaterialCullMode::Back => VkCullMode::Back,
    }
}

fn vk_blend(alpha_mode: RendererMaterialAlphaMode, blend: bool) -> VkBlendPreset {
    match (alpha_mode, blend) {
        (RendererMaterialAlphaMode::Blend, true) => VkBlendPreset::AlphaBlend,
        (RendererMaterialAlphaMode::Mask, _) => VkBlendPreset::AlphaCutout,
        _ => VkBlendPreset::Disabled,
    }
}

fn uv_animation_constants(uv_animation: UvAnimation) -> [f32; 4] {
    [
        uv_animation.scroll_x_speed,
        uv_animation.scroll_y_speed,
        uv_animation.rotation_speed,
        0.0,
    ]
}

fn sample_document() -> VrmDocument {
    VrmDocument {
        materials: vec![Material {
            khr_emissive_strength: Feature::Present(EmissiveStrength(3.0)),
            mtoon: Feature::Present(MtoonMaterial {
                render_queue: MtoonRenderQueue::Transparent,
                transparent_with_z_write: true,
                base_color_factor: [0.4, 0.7, 1.0, 0.5],
                shade_color_factor: [0.1, 0.2, 0.3],
                emissive_factor: [0.2, 0.1, 0.05],
                cutoff_factor: 0.33,
                outline_width_mode: OutlineWidthMode::WorldCoordinates,
                outline_width_factor: 0.02,
                matcap_factor: [0.3, 0.2, 0.1],
                parametric_rim_color_factor: [0.7, 0.5, 0.4],
                rim_lighting_mix_factor: 0.6,
                parametric_rim_fresnel_power_factor: 2.5,
                parametric_rim_lift_factor: 0.15,
                outline_color_factor: [0.02, 0.03, 0.04],
                outline_lighting_mix_factor: 0.25,
                uv_animation: UvAnimation {
                    scroll_x_speed: 0.1,
                    scroll_y_speed: -0.2,
                    rotation_speed: 0.3,
                },
                textures: MtoonTextureSet {
                    main_texture: Some(TextureRef(1)),
                    shade_multiply_texture: Some(TextureRef(2)),
                    normal_texture: Some(TextureRef(3)),
                    matcap_texture: Some(TextureRef(4)),
                    rim_multiply_texture: Some(TextureRef(5)),
                    outline_width_multiply_texture: Some(TextureRef(6)),
                    uv_animation_mask_texture: Some(TextureRef(7)),
                    ..MtoonTextureSet::default()
                },
                ..MtoonMaterial::default()
            }),
            ..Material::default()
        }],
        ..VrmDocument::default()
    }
}

fn main() {
    let table = AshMtoonPipelineTable::from_document(&sample_document());
    assert_eq!(table.pipelines.len(), 2);
    assert_eq!(table.materials.len(), 1);
    assert_eq!(table.pipelines[0].key.pass, VkMtoonPass::Base);
    assert_eq!(table.pipelines[1].key.pass, VkMtoonPass::Outline);
    assert_eq!(table.pipelines[0].key.blend, VkBlendPreset::AlphaBlend);
    assert!(table.pipelines[0].key.depth_stencil.depth_write_enable);
    assert_eq!(table.materials[0].uniform_size, MTOON_GPU_UNIFORM_SIZE);
    assert_eq!(
        table.materials[0].uniform.emissive_color_outline_width[0..3],
        [0.6, 0.3, 0.15]
    );
    assert_eq!(table.materials[0].image_bindings.len(), 7);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materializes_base_and_outline_pipeline_recipes() {
        let table = AshMtoonPipelineTable::from_document(&sample_document());

        let base = &table.pipelines[0];
        let outline = &table.pipelines[1];
        assert_eq!(base.key.pass, VkMtoonPass::Base);
        assert_eq!(outline.key.pass, VkMtoonPass::Outline);
        assert_eq!(base.key.blend, VkBlendPreset::AlphaBlend);
        assert_eq!(outline.key.blend, VkBlendPreset::Disabled);
        assert_eq!(base.key.depth_stencil.compare_op, VkCompareOp::LessOrEqual);
        assert_eq!(
            outline.vertex_shader_spv,
            "shaders/mtoon_outline.wgsl.vert.spv"
        );
    }

    #[test]
    fn exposes_descriptor_layout_and_texture_write_slots() {
        let table = AshMtoonPipelineTable::from_document(&sample_document());
        let base = &table.pipelines[0];
        let material = &table.materials[0];

        assert_eq!(
            base.descriptor_set_layout[0].descriptor_type,
            VkDescriptorType::UniformBuffer
        );
        assert_eq!(
            base.descriptor_set_layout[1].binding,
            owner_sample_override_binding()
        );
        assert!(base.descriptor_set_layout[1].binding > 19);
        assert_eq!(
            base.descriptor_set_layout[1].descriptor_type,
            VkDescriptorType::StorageBuffer
        );
        assert_eq!(
            base.descriptor_set_layout[1].stage_flags,
            VkShaderStage::Fragment
        );
        assert_eq!(
            base.descriptor_set_layout[2].slot,
            Some(MtoonTextureSlot::Main)
        );
        assert_eq!(
            base.descriptor_set_layout[2].descriptor_type,
            VkDescriptorType::SampledImage
        );
        assert_eq!(
            base.descriptor_set_layout[3].descriptor_type,
            VkDescriptorType::Sampler
        );
        let outline_image = base
            .descriptor_set_layout
            .iter()
            .find(|entry| {
                entry.slot == Some(MtoonTextureSlot::OutlineWidth)
                    && entry.descriptor_type == VkDescriptorType::SampledImage
            })
            .expect("outline sampled image binding");
        assert_eq!(outline_image.stage_flags, VkShaderStage::VertexFragment);
        let outline_sampler = base
            .descriptor_set_layout
            .iter()
            .find(|entry| {
                entry.slot == Some(MtoonTextureSlot::OutlineWidth)
                    && entry.descriptor_type == VkDescriptorType::Sampler
            })
            .expect("outline sampler binding");
        assert_eq!(outline_sampler.stage_flags, VkShaderStage::VertexFragment);
        assert_eq!(
            base.descriptor_set_layout
                .iter()
                .filter(|entry| entry.binding == owner_sample_override_binding())
                .count(),
            1
        );
        assert_eq!(material.image_bindings[2].slot, MtoonTextureSlot::Normal);
        assert!(material.image_bindings[2].sampler.normal_map_decode);
        assert_eq!(material.image_bindings[0].image_binding, 1);
        assert_eq!(material.image_bindings[0].sampler_binding, 2);
    }

    #[test]
    fn carries_shader_factors_into_uniform_and_push_constants() {
        let table = AshMtoonPipelineTable::from_document(&sample_document());
        let base = &table.pipelines[0];
        let material = &table.materials[0];

        assert_eq!(base.push_constants.alpha_cutoff, 0.33);
        assert_eq!(base.push_constants.outline_width, 0.02);
        assert_eq!(base.push_constants.uv_animation, [0.1, -0.2, 0.3, 0.0]);
        assert_eq!(material.uniform.base_color_factor, [0.4, 0.7, 1.0, 0.5]);
        assert_eq!(
            material.uniform.emissive_color_outline_width[0..3],
            [0.6, 0.3, 0.15]
        );
        assert_eq!(material.uniform.rim_params, [2.5, 0.15, 3000.0, 0.0]);
        assert_eq!(material.uniform_size, MTOON_GPU_UNIFORM_SIZE);
        assert!(material.reference_wgsl.contains("mtoon_lit_shade_rate"));
    }
}
