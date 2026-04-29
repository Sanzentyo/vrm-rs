//! Bevy integration skeleton for `vrm-rs`.
//!
//! This crate intentionally starts with registry, descriptor bridge, and runtime
//! plugin marker types. Runtime systems can build on these without pulling
//! renderer policy into `vrm-core` or `vrm-adapter`.

use bevy::prelude::{App, Asset, Entity, Handle, Plugin, Resource};
use std::collections::HashMap;
use vrm_adapter::{MtoonMaterialDescriptor, MtoonMaterializationOptions, ViewMode};
use vrm_core::{
    MaterialRef, MtoonAlphaMode, MtoonCullMode, MtoonPipelinePass, NodeRef, TextureRef, VrmDocument,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BevyNodeMap {
    nodes: HashMap<NodeRef, Entity>,
}

impl BevyNodeMap {
    pub fn insert(&mut self, node: NodeRef, entity: Entity) {
        self.nodes.insert(node, entity);
    }

    pub fn entity(&self, node: NodeRef) -> Option<Entity> {
        self.nodes.get(&node).copied()
    }
}

#[derive(Clone, Debug)]
pub struct BevyAssetMap<M: Asset, I: Asset> {
    materials: HashMap<MaterialRef, Handle<M>>,
    textures: HashMap<TextureRef, Handle<I>>,
}

impl<M: Asset, I: Asset> Default for BevyAssetMap<M, I> {
    fn default() -> Self {
        Self {
            materials: HashMap::new(),
            textures: HashMap::new(),
        }
    }
}

impl<M: Asset, I: Asset> BevyAssetMap<M, I> {
    pub fn insert_material(&mut self, material: MaterialRef, handle: Handle<M>) {
        self.materials.insert(material, handle);
    }

    pub fn material(&self, material: MaterialRef) -> Option<&Handle<M>> {
        self.materials.get(&material)
    }

    pub fn insert_texture(&mut self, texture: TextureRef, handle: Handle<I>) {
        self.textures.insert(texture, handle);
    }

    pub fn texture(&self, texture: TextureRef) -> Option<&Handle<I>> {
        self.textures.get(&texture)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BevyMtoonDescriptor {
    pub descriptor: MtoonMaterialDescriptor,
}

pub fn bevy_mtoon_descriptors(
    document: &VrmDocument,
    options: MtoonMaterializationOptions,
) -> Vec<BevyMtoonDescriptor> {
    vrm_adapter::mtoon_material_descriptors(document, options)
        .into_iter()
        .map(|descriptor| BevyMtoonDescriptor { descriptor })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct BevyMtoonMaterialPlan {
    pub material: MaterialRef,
    pub name: Option<String>,
    pub pass: BevyMtoonPass,
    pub render_order: i32,
    pub alpha_mode: MtoonAlphaMode,
    pub cull_mode: MtoonCullMode,
    pub depth_write: bool,
    pub base_color: [f32; 4],
    pub shade_color: [f32; 3],
    pub emissive_color: [f32; 3],
    pub cutoff: f32,
    pub textures: BevyMtoonTextureRefs,
    pub outline_width: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BevyMtoonPass {
    Base,
    Outline,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BevyMtoonTextureRefs {
    pub base_color: Option<TextureRef>,
    pub shade: Option<TextureRef>,
    pub normal: Option<TextureRef>,
    pub matcap: Option<TextureRef>,
    pub rim: Option<TextureRef>,
    pub outline_width: Option<TextureRef>,
    pub uv_animation_mask: Option<TextureRef>,
}

impl BevyMtoonMaterialPlan {
    pub fn from_descriptor(descriptor: &MtoonMaterialDescriptor) -> Self {
        let (pass, render_order, alpha_mode, cull_mode, depth_write, outline_width) =
            match descriptor.pass {
                MtoonPipelinePass::Base(hints) => (
                    BevyMtoonPass::Base,
                    hints.render_order,
                    hints.alpha_mode,
                    hints.cull_mode,
                    hints.depth_write,
                    None,
                ),
                MtoonPipelinePass::Outline(hints) => (
                    BevyMtoonPass::Outline,
                    hints.render_order,
                    MtoonAlphaMode::Opaque,
                    hints.cull_mode,
                    true,
                    Some(descriptor.outline_width_factor),
                ),
            };
        let emissive_color = descriptor
            .emissive_factor
            .map(|channel| channel * descriptor.emissive_strength.0);

        Self {
            material: descriptor.material,
            name: descriptor.name.clone(),
            pass,
            render_order,
            alpha_mode,
            cull_mode,
            depth_write,
            base_color: descriptor.base_color_factor,
            shade_color: descriptor.shade_color_factor,
            emissive_color,
            cutoff: descriptor.cutoff_factor,
            textures: BevyMtoonTextureRefs {
                base_color: descriptor.textures.main_texture,
                shade: descriptor.textures.shade_multiply_texture,
                normal: descriptor.textures.normal_texture,
                matcap: descriptor.textures.matcap_texture,
                rim: descriptor.textures.rim_multiply_texture,
                outline_width: descriptor.textures.outline_width_multiply_texture,
                uv_animation_mask: descriptor.textures.uv_animation_mask_texture,
            },
            outline_width,
        }
    }
}

pub fn bevy_mtoon_material_plans(
    document: &VrmDocument,
    options: MtoonMaterializationOptions,
) -> Vec<BevyMtoonMaterialPlan> {
    vrm_adapter::mtoon_material_descriptors(document, options)
        .iter()
        .map(BevyMtoonMaterialPlan::from_descriptor)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Resource)]
pub struct BevyVrmRuntimeConfig {
    pub view_mode: ViewMode,
    pub apply_vrm0_orientation: bool,
    pub use_spring_parity: bool,
}

impl Default for BevyVrmRuntimeConfig {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::ThirdPerson,
            apply_vrm0_orientation: true,
            use_spring_parity: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct VrmRuntimePlugin {
    pub config: BevyVrmRuntimeConfig,
}

impl Plugin for VrmRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrm_core::{Feature, MtoonAlphaMode, MtoonMaterial, MtoonRenderQueue};

    #[test]
    fn node_map_round_trips_entity() {
        let mut map = BevyNodeMap::default();
        let entity = Entity::from_raw_u32(7).unwrap();

        map.insert(NodeRef(1), entity);

        assert_eq!(map.entity(NodeRef(1)), Some(entity));
        assert_eq!(map.entity(NodeRef(2)), None);
    }

    #[test]
    fn mtoon_descriptor_bridge_uses_renderer_agnostic_adapter_data() {
        let document = VrmDocument {
            materials: vec![vrm_core::Material {
                mtoon: Feature::Present(MtoonMaterial::default()),
                ..vrm_core::Material::default()
            }],
            ..VrmDocument::default()
        };

        let descriptors = bevy_mtoon_descriptors(&document, MtoonMaterializationOptions::default());

        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].descriptor.material, MaterialRef(0));
    }

    #[test]
    fn mtoon_material_plan_preserves_bevy_facing_state() {
        let document = VrmDocument {
            materials: vec![vrm_core::Material {
                name: Some("mtoon".to_owned()),
                khr_emissive_strength: Feature::Present(vrm_core::EmissiveStrength(2.0)),
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    transparent_with_z_write: true,
                    base_color_factor: [1.0, 0.8, 0.6, 0.5],
                    shade_color_factor: [0.5, 0.4, 0.3],
                    emissive_factor: [0.1, 0.2, 0.3],
                    cutoff_factor: 0.42,
                    outline_width_mode: vrm_core::OutlineWidthMode::WorldCoordinates,
                    outline_width_factor: 0.01,
                    textures: vrm_core::MtoonTextureSet {
                        main_texture: Some(TextureRef(1)),
                        shade_multiply_texture: Some(TextureRef(2)),
                        normal_texture: Some(TextureRef(3)),
                        ..Default::default()
                    },
                    ..MtoonMaterial::default()
                }),
                ..vrm_core::Material::default()
            }],
            ..VrmDocument::default()
        };

        let plans = bevy_mtoon_material_plans(&document, MtoonMaterializationOptions::default());

        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].material, MaterialRef(0));
        assert_eq!(plans[0].name.as_deref(), Some("mtoon"));
        assert_eq!(plans[0].pass, BevyMtoonPass::Base);
        assert_eq!(plans[0].alpha_mode, MtoonAlphaMode::Blend);
        assert!(plans[0].depth_write);
        assert_eq!(plans[0].base_color, [1.0, 0.8, 0.6, 0.5]);
        assert_eq!(plans[0].shade_color, [0.5, 0.4, 0.3]);
        assert_eq!(plans[0].emissive_color, [0.2, 0.4, 0.6]);
        assert_eq!(plans[0].cutoff, 0.42);
        assert_eq!(plans[0].textures.base_color, Some(TextureRef(1)));
        assert_eq!(plans[0].textures.shade, Some(TextureRef(2)));
        assert_eq!(plans[0].textures.normal, Some(TextureRef(3)));
        assert_eq!(plans[1].pass, BevyMtoonPass::Outline);
        assert_eq!(plans[1].outline_width, Some(0.01));
    }

    #[test]
    fn runtime_plugin_installs_config_resource() {
        let mut app = App::new();
        let config = BevyVrmRuntimeConfig {
            view_mode: ViewMode::FirstPerson,
            apply_vrm0_orientation: false,
            use_spring_parity: true,
        };

        app.add_plugins(VrmRuntimePlugin {
            config: config.clone(),
        });

        assert_eq!(app.world().resource::<BevyVrmRuntimeConfig>(), &config);
    }
}
