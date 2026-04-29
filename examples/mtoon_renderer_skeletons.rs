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
struct WgpuPipelineKey {
    pass: RendererPass,
    blend: BlendState,
    cull: CullState,
    depth_write: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct AshPipelineKey {
    pass: RendererPass,
    blend: BlendState,
    cull: CullState,
    depth_write: bool,
    render_order: i32,
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
}

#[derive(Clone, Debug, Default, PartialEq)]
struct WgpuMaterialTable {
    pipelines: HashMap<MaterialRef, Vec<WgpuPipelineKey>>,
    materials: HashMap<MaterialRef, RendererMaterial>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct AshMaterialTable {
    pipelines: HashMap<MaterialRef, Vec<AshPipelineKey>>,
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
                .push(wgpu_pipeline_key(descriptor.pass));
            table
                .materials
                .entry(descriptor.material)
                .or_insert_with(|| RendererMaterial {
                    material: descriptor.material,
                    base_color: descriptor.base_color_factor,
                    emissive: descriptor
                        .emissive_factor
                        .map(|channel| channel * descriptor.emissive_strength.0),
                    cutoff: descriptor.cutoff_factor,
                    base_texture: descriptor.textures.main_texture,
                    shade_texture: descriptor.textures.shade_multiply_texture,
                    normal_texture: descriptor.textures.normal_texture,
                });
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
                .push(ash_pipeline_key(descriptor.pass));
            table
                .materials
                .entry(descriptor.material)
                .or_insert_with(|| RendererMaterial {
                    material: descriptor.material,
                    base_color: descriptor.base_color_factor,
                    emissive: descriptor
                        .emissive_factor
                        .map(|channel| channel * descriptor.emissive_strength.0),
                    cutoff: descriptor.cutoff_factor,
                    base_texture: descriptor.textures.main_texture,
                    shade_texture: descriptor.textures.shade_multiply_texture,
                    normal_texture: descriptor.textures.normal_texture,
                });
        }
        table
    }
}

fn wgpu_pipeline_key(pass: MtoonPipelinePass) -> WgpuPipelineKey {
    match pass {
        MtoonPipelinePass::Base(hints) => WgpuPipelineKey {
            pass: RendererPass::Base,
            blend: blend_state(hints.alpha_mode),
            cull: cull_state(hints.cull_mode),
            depth_write: hints.depth_write,
        },
        MtoonPipelinePass::Outline(hints) => WgpuPipelineKey {
            pass: RendererPass::Outline,
            blend: BlendState::Opaque,
            cull: cull_state(hints.cull_mode),
            depth_write: true,
        },
    }
}

fn ash_pipeline_key(pass: MtoonPipelinePass) -> AshPipelineKey {
    match pass {
        MtoonPipelinePass::Base(hints) => AshPipelineKey {
            pass: RendererPass::Base,
            blend: blend_state(hints.alpha_mode),
            cull: cull_state(hints.cull_mode),
            depth_write: hints.depth_write,
            render_order: hints.render_order,
        },
        MtoonPipelinePass::Outline(hints) => AshPipelineKey {
            pass: RendererPass::Outline,
            blend: BlendState::Opaque,
            cull: cull_state(hints.cull_mode),
            depth_write: true,
            render_order: hints.render_order,
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
    assert_eq!(wgpu.materials[&MaterialRef(0)].emissive, [0.2, 0.4, 0.6]);
    assert_eq!(
        ash.materials[&MaterialRef(0)].base_texture,
        Some(TextureRef(1))
    );
}
