//! Renderer-owned MToon materialization skeletons.
//!
//! This example intentionally avoids depending on `wgpu` or `ash`. It shows the
//! shape a renderer crate can use when translating renderer-agnostic
//! `vrm-rs` MToon descriptors into engine-owned pipeline/material tables.

use std::collections::HashMap;

use vrm_adapter::{
    MtoonMaterializationOptions, MtoonRendererMaterialPlan, MtoonRendererPass, MtoonSamplerHint,
    MtoonTextureBindingPlan, MtoonTextureSlot, mtoon_renderer_material_plans,
};
use vrm_core::{
    EmissiveStrength, Feature, Material, MaterialRef, MtoonAlphaMode, MtoonCullMode, MtoonMaterial,
    MtoonRenderQueue, MtoonTextureSet, OutlineWidthMode, TextureRef, VrmDocument,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RendererPass {
    Base,
    Outline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BlendState {
    Opaque,
    AlphaTest,
    AlphaBlend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CullState {
    None,
    Front,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CompareOp {
    Always,
    LessEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TextureSlot {
    Main,
    ShadeMultiply,
    ShadingShift,
    Normal,
    Matcap,
    RimMultiply,
    OutlineWidth,
    UvAnimationMask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SamplerKind {
    LinearRepeat,
    NormalMap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TextureBinding {
    slot: TextureSlot,
    texture: TextureRef,
    sampler: SamplerKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct DepthState {
    test: bool,
    write: bool,
    compare: CompareOp,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BindLayout {
    uniform_bytes: usize,
    textures: Vec<TextureSlot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WgpuPipelineKey {
    pass: RendererPass,
    blend: BlendState,
    cull: CullState,
    depth: DepthState,
    render_order: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct WgpuPipelineDescriptor {
    key: WgpuPipelineKey,
    bind_layout: BindLayout,
    vertex_layout: &'static str,
    fragment_entry: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct AshPipelineDescriptor {
    pass: RendererPass,
    blend: BlendState,
    cull: CullState,
    depth: DepthState,
    render_order: i32,
    descriptor_layout: BindLayout,
    push_constant_bytes: usize,
    vertex_shader: &'static str,
    fragment_shader: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
struct RendererMaterial {
    material: MaterialRef,
    base_color: [f32; 4],
    emissive: [f32; 3],
    cutoff: f32,
    base_texture: Option<TextureRef>,
    shade_texture: Option<TextureRef>,
    normal_texture: Option<TextureRef>,
    texture_bindings: Vec<TextureBinding>,
    uniform_bytes: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct WgpuMaterialTable {
    pipelines: HashMap<MaterialRef, Vec<WgpuPipelineDescriptor>>,
    materials: HashMap<MaterialRef, RendererMaterial>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct AshMaterialTable {
    pipelines: HashMap<MaterialRef, Vec<AshPipelineDescriptor>>,
    materials: HashMap<MaterialRef, RendererMaterial>,
}

impl WgpuMaterialTable {
    fn from_vrm(document: &VrmDocument) -> Self {
        let mut table = Self::default();
        for plan in mtoon_renderer_material_plans(document, MtoonMaterializationOptions::default())
        {
            table
                .pipelines
                .entry(plan.material)
                .or_default()
                .push(wgpu_pipeline_descriptor(&plan));
            table
                .materials
                .entry(plan.material)
                .or_insert_with(|| renderer_material(&plan));
        }
        table
    }
}

impl AshMaterialTable {
    fn from_vrm(document: &VrmDocument) -> Self {
        let mut table = Self::default();
        for plan in mtoon_renderer_material_plans(document, MtoonMaterializationOptions::default())
        {
            table
                .pipelines
                .entry(plan.material)
                .or_default()
                .push(ash_pipeline_descriptor(&plan));
            table
                .materials
                .entry(plan.material)
                .or_insert_with(|| renderer_material(&plan));
        }
        table
    }
}

fn renderer_material(plan: &MtoonRendererMaterialPlan) -> RendererMaterial {
    RendererMaterial {
        material: plan.material,
        base_color: plan.shader.base_color_factor,
        emissive: plan.shader.emissive_color,
        cutoff: plan.shader.cutoff_factor,
        base_texture: plan.textures.main,
        shade_texture: plan.textures.shade_multiply,
        normal_texture: plan.textures.normal,
        texture_bindings: texture_bindings(&plan.texture_bindings),
        uniform_bytes: mtoon_uniform_bytes(),
    }
}

fn wgpu_pipeline_descriptor(plan: &MtoonRendererMaterialPlan) -> WgpuPipelineDescriptor {
    let key = match plan.pass {
        MtoonRendererPass::Base => WgpuPipelineKey {
            pass: RendererPass::Base,
            blend: blend_state(plan.pipeline.alpha_mode),
            cull: cull_state(plan.pipeline.cull_mode),
            depth: depth_state(plan.pipeline.depth_test, plan.pipeline.depth_write),
            render_order: plan.pipeline.render_order,
        },
        MtoonRendererPass::Outline => WgpuPipelineKey {
            pass: RendererPass::Outline,
            blend: BlendState::Opaque,
            cull: cull_state(plan.pipeline.cull_mode),
            depth: depth_state(true, true),
            render_order: plan.pipeline.render_order,
        },
    };
    WgpuPipelineDescriptor {
        key,
        bind_layout: bind_layout(&plan.texture_bindings),
        vertex_layout: "position_normal_uv_skinning",
        fragment_entry: match key.pass {
            RendererPass::Base => "mtoon_base_frag",
            RendererPass::Outline => "mtoon_outline_frag",
        },
    }
}

fn ash_pipeline_descriptor(plan: &MtoonRendererMaterialPlan) -> AshPipelineDescriptor {
    match plan.pass {
        MtoonRendererPass::Base => AshPipelineDescriptor {
            pass: RendererPass::Base,
            blend: blend_state(plan.pipeline.alpha_mode),
            cull: cull_state(plan.pipeline.cull_mode),
            depth: depth_state(plan.pipeline.depth_test, plan.pipeline.depth_write),
            render_order: plan.pipeline.render_order,
            descriptor_layout: bind_layout(&plan.texture_bindings),
            push_constant_bytes: 16,
            vertex_shader: "mtoon_base.vert.spv",
            fragment_shader: "mtoon_base.frag.spv",
        },
        MtoonRendererPass::Outline => AshPipelineDescriptor {
            pass: RendererPass::Outline,
            blend: BlendState::Opaque,
            cull: cull_state(plan.pipeline.cull_mode),
            depth: depth_state(true, true),
            render_order: plan.pipeline.render_order,
            descriptor_layout: bind_layout(&plan.texture_bindings),
            push_constant_bytes: 16,
            vertex_shader: "mtoon_outline.vert.spv",
            fragment_shader: "mtoon_outline.frag.spv",
        },
    }
}

fn blend_state(alpha: MtoonAlphaMode) -> BlendState {
    match alpha {
        MtoonAlphaMode::Opaque => BlendState::Opaque,
        MtoonAlphaMode::Mask => BlendState::AlphaTest,
        MtoonAlphaMode::Blend => BlendState::AlphaBlend,
    }
}

fn cull_state(cull: MtoonCullMode) -> CullState {
    match cull {
        MtoonCullMode::Off => CullState::None,
        MtoonCullMode::Front => CullState::Front,
        MtoonCullMode::Back => CullState::Back,
    }
}

fn depth_state(test: bool, write: bool) -> DepthState {
    DepthState {
        test,
        write,
        compare: if test {
            CompareOp::LessEqual
        } else {
            CompareOp::Always
        },
    }
}

fn mtoon_uniform_bytes() -> usize {
    256
}

fn bind_layout(bindings: &[MtoonTextureBindingPlan]) -> BindLayout {
    BindLayout {
        uniform_bytes: mtoon_uniform_bytes(),
        textures: bindings
            .iter()
            .map(|binding| binding.slot)
            .map(texture_slot)
            .collect(),
    }
}

fn texture_bindings(bindings: &[MtoonTextureBindingPlan]) -> Vec<TextureBinding> {
    bindings
        .iter()
        .map(|binding| TextureBinding {
            slot: texture_slot(binding.slot),
            texture: binding.texture,
            sampler: sampler_kind(binding.sampler),
        })
        .collect()
}

fn texture_slot(slot: MtoonTextureSlot) -> TextureSlot {
    match slot {
        MtoonTextureSlot::Main => TextureSlot::Main,
        MtoonTextureSlot::ShadeMultiply => TextureSlot::ShadeMultiply,
        MtoonTextureSlot::ShadingShift => TextureSlot::ShadingShift,
        MtoonTextureSlot::Normal => TextureSlot::Normal,
        MtoonTextureSlot::Matcap => TextureSlot::Matcap,
        MtoonTextureSlot::RimMultiply => TextureSlot::RimMultiply,
        MtoonTextureSlot::OutlineWidth => TextureSlot::OutlineWidth,
        MtoonTextureSlot::UvAnimationMask => TextureSlot::UvAnimationMask,
    }
}

fn sampler_kind(sampler: MtoonSamplerHint) -> SamplerKind {
    match sampler {
        MtoonSamplerHint::LinearRepeat => SamplerKind::LinearRepeat,
        MtoonSamplerHint::NormalMapLinearRepeat => SamplerKind::NormalMap,
    }
}

fn sample_document() -> VrmDocument {
    VrmDocument {
        materials: vec![Material {
            khr_emissive_strength: Feature::Present(EmissiveStrength(2.0)),
            mtoon: Feature::Present(MtoonMaterial {
                render_queue: MtoonRenderQueue::Transparent,
                transparent_with_z_write: true,
                base_color_factor: [1.0, 0.8, 0.6, 0.5],
                emissive_factor: [0.1, 0.2, 0.3],
                cutoff_factor: 0.42,
                outline_width_mode: OutlineWidthMode::WorldCoordinates,
                outline_width_factor: 0.01,
                textures: MtoonTextureSet {
                    main_texture: Some(TextureRef(1)),
                    shade_multiply_texture: Some(TextureRef(2)),
                    shading_shift_texture: Some(TextureRef(8)),
                    normal_texture: Some(TextureRef(3)),
                    matcap_texture: Some(TextureRef(4)),
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
    let document = sample_document();
    let wgpu = WgpuMaterialTable::from_vrm(&document);
    let ash = AshMaterialTable::from_vrm(&document);

    assert_eq!(wgpu.pipelines[&MaterialRef(0)].len(), 2);
    assert_eq!(ash.pipelines[&MaterialRef(0)].len(), 2);
    assert_eq!(
        wgpu.pipelines[&MaterialRef(0)][0].bind_layout.uniform_bytes,
        256
    );
    assert_eq!(
        ash.pipelines[&MaterialRef(0)][0].descriptor_layout.textures,
        vec![
            TextureSlot::Main,
            TextureSlot::ShadeMultiply,
            TextureSlot::ShadingShift,
            TextureSlot::Normal,
            TextureSlot::Matcap,
        ]
    );
    assert_eq!(wgpu.materials[&MaterialRef(0)].emissive, [0.2, 0.4, 0.6]);
    assert_eq!(wgpu.materials[&MaterialRef(0)].texture_bindings.len(), 5);
    assert_eq!(
        ash.materials[&MaterialRef(0)].base_texture,
        Some(TextureRef(1))
    );
}
