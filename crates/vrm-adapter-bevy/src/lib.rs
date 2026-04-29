//! Bevy integration skeleton for `vrm-rs`.
//!
//! This crate intentionally starts with registry and descriptor bridge types.
//! Runtime systems can build on these without pulling renderer policy into
//! `vrm-core` or `vrm-adapter`.

use bevy::prelude::{Asset, Entity, Handle};
use std::collections::HashMap;
use vrm_adapter::{MtoonMaterialDescriptor, MtoonMaterializationOptions};
use vrm_core::{MaterialRef, NodeRef, TextureRef, VrmDocument};

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

#[cfg(test)]
mod tests {
    use super::*;
    use vrm_core::{Feature, MtoonMaterial};

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
}
