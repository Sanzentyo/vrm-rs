//! Renderer-owned MToon materialization skeletons.
//!
//! This example intentionally avoids depending on `wgpu` or `ash`. It shows the
//! shape a renderer crate can use when translating renderer-agnostic
//! `vrm-rs` MToon descriptors into engine-owned pipeline/material tables.

use std::collections::HashMap;

use vrm_adapter::{MtoonMaterializationOptions, mtoon_material_descriptors};
use vrm_core::{
    EmissiveStrength, Feature, Material, MaterialRef, MtoonAlphaMode, MtoonCullMode, MtoonMaterial,
    MtoonPipelinePass, MtoonRenderQueue, MtoonTextureSet, OutlineWidthMode, TextureRef,
    VrmDocument,
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
        for descriptor in
            mtoon_material_descriptors(document, MtoonMaterializationOptions::default())
        {
            table
                .pipelines
                .entry(descriptor.material)
                .or_default()
                .push(wgpu_pipeline_descriptor(&descriptor));
            table
                .materials
                .entry(descriptor.material)
                .or_insert_with(|| renderer_material(&descriptor));
        }
        table
    }
}

impl AshMaterialTable {
    fn from_vrm(document: &VrmDocument) -> Self {
        let mut table = Self::default();
        for descriptor in
            mtoon_material_descriptors(document, MtoonMaterializationOptions::default())
        {
            table
                .pipelines
                .entry(descriptor.material)
                .or_default()
                .push(ash_pipeline_descriptor(&descriptor));
            table
                .materials
                .entry(descriptor.material)
                .or_insert_with(|| renderer_material(&descriptor));
        }
        table
    }
}

fn renderer_material(descriptor: &vrm_adapter::MtoonMaterialDescriptor) -> RendererMaterial {
    RendererMaterial {
        material: descriptor.material,
        base_color: descriptor.base_color_factor,
        emissive: descriptor
            .emissive_factor
            .map(|channel| channel * descriptor.emissive_strength.0),
        cutoff: descriptor.cutoff_factor,
        base_texture: descriptor.textures.main_texture,
        shade_texture: descriptor.textures.shade_multiply_texture,
        normal_texture: descriptor.textures.normal_texture,
        texture_bindings: texture_bindings(&descriptor.textures),
        uniform_bytes: mtoon_uniform_bytes(),
    }
}

fn wgpu_pipeline_descriptor(
    descriptor: &vrm_adapter::MtoonMaterialDescriptor,
) -> WgpuPipelineDescriptor {
    let key = match descriptor.pass {
        MtoonPipelinePass::Base(hints) => WgpuPipelineKey {
            pass: RendererPass::Base,
            blend: blend_state(hints.alpha_mode),
            cull: cull_state(hints.cull_mode),
            depth: depth_state(hints.depth_test, hints.depth_write),
            render_order: hints.render_order,
        },
        MtoonPipelinePass::Outline(hints) => WgpuPipelineKey {
            pass: RendererPass::Outline,
            blend: BlendState::Opaque,
            cull: cull_state(hints.cull_mode),
            depth: depth_state(true, true),
            render_order: hints.render_order,
        },
    };
    WgpuPipelineDescriptor {
        key,
        bind_layout: bind_layout(&descriptor.textures),
        vertex_layout: "position_normal_uv_skinning",
        fragment_entry: match key.pass {
            RendererPass::Base => "mtoon_base_frag",
            RendererPass::Outline => "mtoon_outline_frag",
        },
    }
}

fn ash_pipeline_descriptor(
    descriptor: &vrm_adapter::MtoonMaterialDescriptor,
) -> AshPipelineDescriptor {
    match descriptor.pass {
        MtoonPipelinePass::Base(hints) => AshPipelineDescriptor {
            pass: RendererPass::Base,
            blend: blend_state(hints.alpha_mode),
            cull: cull_state(hints.cull_mode),
            depth: depth_state(hints.depth_test, hints.depth_write),
            render_order: hints.render_order,
            descriptor_layout: bind_layout(&descriptor.textures),
            push_constant_bytes: 16,
            vertex_shader: "mtoon_base.vert.spv",
            fragment_shader: "mtoon_base.frag.spv",
        },
        MtoonPipelinePass::Outline(hints) => AshPipelineDescriptor {
            pass: RendererPass::Outline,
            blend: BlendState::Opaque,
            cull: cull_state(hints.cull_mode),
            depth: depth_state(true, true),
            render_order: hints.render_order,
            descriptor_layout: bind_layout(&descriptor.textures),
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

fn bind_layout(textures: &MtoonTextureSet) -> BindLayout {
    BindLayout {
        uniform_bytes: mtoon_uniform_bytes(),
        textures: texture_bindings(textures)
            .into_iter()
            .map(|binding| binding.slot)
            .collect(),
    }
}

fn texture_bindings(textures: &MtoonTextureSet) -> Vec<TextureBinding> {
    [
        (
            TextureSlot::Main,
            textures.main_texture,
            SamplerKind::LinearRepeat,
        ),
        (
            TextureSlot::ShadeMultiply,
            textures.shade_multiply_texture,
            SamplerKind::LinearRepeat,
        ),
        (
            TextureSlot::Normal,
            textures.normal_texture,
            SamplerKind::NormalMap,
        ),
        (
            TextureSlot::Matcap,
            textures.matcap_texture,
            SamplerKind::LinearRepeat,
        ),
        (
            TextureSlot::RimMultiply,
            textures.rim_multiply_texture,
            SamplerKind::LinearRepeat,
        ),
        (
            TextureSlot::OutlineWidth,
            textures.outline_width_multiply_texture,
            SamplerKind::LinearRepeat,
        ),
        (
            TextureSlot::UvAnimationMask,
            textures.uv_animation_mask_texture,
            SamplerKind::LinearRepeat,
        ),
    ]
    .into_iter()
    .filter_map(|(slot, texture, sampler)| {
        texture.map(|texture| TextureBinding {
            slot,
            texture,
            sampler,
        })
    })
    .collect()
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
            TextureSlot::Normal,
            TextureSlot::Matcap,
        ]
    );
    assert_eq!(wgpu.materials[&MaterialRef(0)].emissive, [0.2, 0.4, 0.6]);
    assert_eq!(wgpu.materials[&MaterialRef(0)].texture_bindings.len(), 4);
    assert_eq!(
        ash.materials[&MaterialRef(0)].base_texture,
        Some(TextureRef(1))
    );
}
