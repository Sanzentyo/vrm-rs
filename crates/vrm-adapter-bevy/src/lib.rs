//! Bevy integration skeleton for `vrm-rs`.
//!
//! This crate intentionally starts with registry, descriptor bridge, and runtime
//! plugin marker types. Runtime systems can build on these without pulling
//! renderer policy into `vrm-core` or `vrm-adapter`.

use bevy::prelude::{App, Asset, Entity, Handle, Plugin, Resource};
use glam::{Quat, Vec3};
use std::collections::{HashMap, HashSet};
use vrm_adapter::{
    MaterialAccess, MorphTargetAccess, MtoonMaterialDescriptor, MtoonMaterializationOptions,
    MtoonPipelineAccess, SceneGraph, TransformAccess, ViewMode, VisibilityAccess,
    WorldTransformAccess, WorldTransformUpdate,
};
use vrm_core::Transform;
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

    pub fn node_for_entity(&self, entity: Entity) -> Option<NodeRef> {
        self.nodes
            .iter()
            .find_map(|(node, candidate)| (*candidate == entity).then_some(*node))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BevyAdapterError {
    MissingNode(NodeRef),
    MissingEntity(Entity),
    CyclicHierarchy(NodeRef),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BevyTextureTransform {
    pub scale: Option<[f32; 2]>,
    pub offset: Option<[f32; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BevyRuntimeSceneState {
    pub nodes: BevyNodeMap,
    parents: HashMap<NodeRef, NodeRef>,
    children: HashMap<NodeRef, Vec<NodeRef>>,
    local_transforms: HashMap<Entity, Transform>,
    world_transforms: HashMap<Entity, Transform>,
    visibility: HashMap<Entity, bool>,
    morph_weights: HashMap<(NodeRef, usize), f32>,
    material_colors: HashMap<(MaterialRef, String), Vec<f32>>,
    texture_transforms: HashMap<MaterialRef, BevyTextureTransform>,
    emissive_intensities: HashMap<MaterialRef, f32>,
    mtoon_pipeline_passes: HashMap<MaterialRef, Vec<MtoonPipelinePass>>,
}

impl BevyRuntimeSceneState {
    pub fn insert_node(&mut self, node: NodeRef, entity: Entity, local: Transform) {
        self.nodes.insert(node, entity);
        self.local_transforms.insert(entity, local);
        self.world_transforms.insert(entity, local);
        self.visibility.entry(entity).or_insert(true);
    }

    pub fn set_parent(
        &mut self,
        node: NodeRef,
        parent: Option<NodeRef>,
    ) -> Result<(), BevyAdapterError> {
        self.entity(node)?;
        if let Some(parent) = parent {
            self.entity(parent)?;
        }
        if let Some(previous_parent) = self.parents.remove(&node)
            && let Some(siblings) = self.children.get_mut(&previous_parent)
        {
            siblings.retain(|child| *child != node);
        }
        if let Some(parent) = parent {
            self.parents.insert(node, parent);
            self.children.entry(parent).or_default().push(node);
        }
        Ok(())
    }

    pub fn is_visible(&self, node: NodeRef) -> Result<bool, BevyAdapterError> {
        let entity = self.entity(node)?;
        self.visibility
            .get(&entity)
            .copied()
            .ok_or(BevyAdapterError::MissingEntity(entity))
    }

    pub fn morph_weight(&self, node: NodeRef, morph_index: usize) -> Option<f32> {
        self.morph_weights.get(&(node, morph_index)).copied()
    }

    pub fn material_color(&self, material: MaterialRef, property: &str) -> Option<&[f32]> {
        self.material_colors
            .get(&(material, property.to_owned()))
            .map(Vec::as_slice)
    }

    pub fn texture_transform(&self, material: MaterialRef) -> Option<BevyTextureTransform> {
        self.texture_transforms.get(&material).copied()
    }

    pub fn emissive_intensity(&self, material: MaterialRef) -> Option<f32> {
        self.emissive_intensities.get(&material).copied()
    }

    pub fn mtoon_pipeline_passes(&self, material: MaterialRef) -> Option<&[MtoonPipelinePass]> {
        self.mtoon_pipeline_passes.get(&material).map(Vec::as_slice)
    }

    fn entity(&self, node: NodeRef) -> Result<Entity, BevyAdapterError> {
        self.nodes
            .entity(node)
            .ok_or(BevyAdapterError::MissingNode(node))
    }

    fn node_transform(
        transforms: &HashMap<Entity, Transform>,
        entity: Entity,
    ) -> Result<Transform, BevyAdapterError> {
        transforms
            .get(&entity)
            .copied()
            .ok_or(BevyAdapterError::MissingEntity(entity))
    }

    fn update_world_node(
        &mut self,
        node: NodeRef,
        parent_world: Option<Transform>,
        visiting: &mut HashSet<NodeRef>,
    ) -> Result<(), BevyAdapterError> {
        if !visiting.insert(node) {
            return Err(BevyAdapterError::CyclicHierarchy(node));
        }
        let entity = self.entity(node)?;
        let local = Self::node_transform(&self.local_transforms, entity)?;
        let world = parent_world.map_or(local, |parent| compose_transform(parent, local));
        self.world_transforms.insert(entity, world);
        for child in self.children.get(&node).cloned().unwrap_or_default() {
            self.update_world_node(child, Some(world), visiting)?;
        }
        visiting.remove(&node);
        Ok(())
    }
}

impl SceneGraph for BevyRuntimeSceneState {
    type Error = BevyAdapterError;

    fn parent(&self, node: NodeRef) -> Result<Option<NodeRef>, Self::Error> {
        self.entity(node)?;
        Ok(self.parents.get(&node).copied())
    }

    fn children(&self, node: NodeRef) -> Result<Vec<NodeRef>, Self::Error> {
        self.entity(node)?;
        Ok(self.children.get(&node).cloned().unwrap_or_default())
    }
}

impl TransformAccess for BevyRuntimeSceneState {
    type Error = BevyAdapterError;

    fn local_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
        let entity = self.entity(node)?;
        Self::node_transform(&self.local_transforms, entity)
    }

    fn set_local_transform(
        &mut self,
        node: NodeRef,
        transform: Transform,
    ) -> Result<(), Self::Error> {
        let entity = self.entity(node)?;
        self.local_transforms.insert(entity, transform);
        Ok(())
    }

    fn set_local_rotation(&mut self, node: NodeRef, rotation: Quat) -> Result<(), Self::Error> {
        let mut transform = self.local_transform(node)?;
        transform.rotation = rotation;
        self.set_local_transform(node, transform)
    }

    fn translate_local(&mut self, node: NodeRef, translation: Vec3) -> Result<(), Self::Error> {
        let mut transform = self.local_transform(node)?;
        transform.translation = translation;
        self.set_local_transform(node, transform)
    }
}

impl WorldTransformAccess for BevyRuntimeSceneState {
    type Error = BevyAdapterError;

    fn world_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
        let entity = self.entity(node)?;
        Self::node_transform(&self.world_transforms, entity)
    }
}

impl WorldTransformUpdate for BevyRuntimeSceneState {
    type Error = BevyAdapterError;

    fn update_world_transforms(&mut self) -> Result<(), Self::Error> {
        let roots = self
            .nodes
            .nodes
            .keys()
            .copied()
            .filter(|node| !self.parents.contains_key(node))
            .collect::<Vec<_>>();
        let mut visiting = HashSet::new();
        for root in roots {
            self.update_world_node(root, None, &mut visiting)?;
        }
        Ok(())
    }
}

impl VisibilityAccess for BevyRuntimeSceneState {
    type Error = BevyAdapterError;

    fn set_node_visible(&mut self, node: NodeRef, visible: bool) -> Result<(), Self::Error> {
        let entity = self.entity(node)?;
        self.visibility.insert(entity, visible);
        Ok(())
    }
}

impl MorphTargetAccess for BevyRuntimeSceneState {
    type Error = BevyAdapterError;

    fn set_morph_weight(
        &mut self,
        node: NodeRef,
        morph_index: usize,
        weight: f32,
    ) -> Result<(), Self::Error> {
        self.entity(node)?;
        self.morph_weights.insert((node, morph_index), weight);
        Ok(())
    }
}

impl MaterialAccess for BevyRuntimeSceneState {
    type Error = BevyAdapterError;

    fn set_material_color(
        &mut self,
        material: MaterialRef,
        property: &str,
        value: &[f32],
    ) -> Result<(), Self::Error> {
        self.material_colors
            .insert((material, property.to_owned()), value.to_vec());
        Ok(())
    }

    fn set_texture_transform(
        &mut self,
        material: MaterialRef,
        scale: Option<[f32; 2]>,
        offset: Option<[f32; 2]>,
    ) -> Result<(), Self::Error> {
        self.texture_transforms
            .insert(material, BevyTextureTransform { scale, offset });
        Ok(())
    }

    fn set_emissive_intensity(
        &mut self,
        material: MaterialRef,
        intensity: f32,
    ) -> Result<(), Self::Error> {
        self.emissive_intensities.insert(material, intensity);
        Ok(())
    }
}

impl MtoonPipelineAccess for BevyRuntimeSceneState {
    type Error = BevyAdapterError;

    fn set_mtoon_pipeline_passes(
        &mut self,
        material: MaterialRef,
        passes: &[MtoonPipelinePass],
    ) -> Result<(), Self::Error> {
        self.mtoon_pipeline_passes.insert(material, passes.to_vec());
        Ok(())
    }
}

fn compose_transform(parent: Transform, local: Transform) -> Transform {
    Transform {
        translation: parent.translation + parent.rotation * (parent.scale * local.translation),
        rotation: parent.rotation * local.rotation,
        scale: parent.scale * local.scale,
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
    use vrm_adapter::apply_mtoon_pipeline_hints;
    use vrm_core::{Feature, MtoonAlphaMode, MtoonMaterial, MtoonRenderQueue, MtoonTextureSet};

    #[test]
    fn node_map_round_trips_entity() {
        let mut map = BevyNodeMap::default();
        let entity = Entity::from_raw_u32(7).unwrap();

        map.insert(NodeRef(1), entity);

        assert_eq!(map.entity(NodeRef(1)), Some(entity));
        assert_eq!(map.entity(NodeRef(2)), None);
    }

    #[test]
    fn runtime_scene_state_implements_transform_graph_and_visibility_traits() {
        let mut scene = BevyRuntimeSceneState::default();
        let root = Entity::from_raw_u32(1).unwrap();
        let child = Entity::from_raw_u32(2).unwrap();
        scene.insert_node(
            NodeRef(0),
            root,
            Transform {
                translation: Vec3::new(1.0, 0.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        scene.insert_node(
            NodeRef(1),
            child,
            Transform {
                translation: Vec3::new(0.0, 2.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        scene.set_parent(NodeRef(1), Some(NodeRef(0))).unwrap();

        assert_eq!(scene.parent(NodeRef(1)).unwrap(), Some(NodeRef(0)));
        assert_eq!(scene.children(NodeRef(0)).unwrap(), vec![NodeRef(1)]);
        scene.update_world_transforms().unwrap();
        assert_eq!(
            scene.world_transform(NodeRef(1)).unwrap().translation,
            Vec3::new(1.0, 2.0, 0.0)
        );

        scene
            .translate_local(NodeRef(1), Vec3::new(0.0, 3.0, 0.0))
            .unwrap();
        scene.update_world_transforms().unwrap();
        assert_eq!(
            scene.world_transform(NodeRef(1)).unwrap().translation,
            Vec3::new(1.0, 3.0, 0.0)
        );
        scene.set_node_visible(NodeRef(1), false).unwrap();
        assert!(!scene.is_visible(NodeRef(1)).unwrap());
    }

    #[test]
    fn runtime_scene_state_records_morph_and_material_writeback() {
        let mut scene = BevyRuntimeSceneState::default();
        scene.insert_node(
            NodeRef(0),
            Entity::from_raw_u32(1).unwrap(),
            Transform::default(),
        );

        scene.set_morph_weight(NodeRef(0), 2, 40.0).unwrap();
        scene
            .set_material_color(MaterialRef(3), "_Color", &[1.0, 0.5, 0.25, 1.0])
            .unwrap();
        scene
            .set_texture_transform(MaterialRef(3), Some([2.0, 3.0]), Some([0.1, 0.2]))
            .unwrap();

        assert_eq!(scene.morph_weight(NodeRef(0), 2), Some(40.0));
        assert_eq!(
            scene.material_color(MaterialRef(3), "_Color"),
            Some([1.0, 0.5, 0.25, 1.0].as_slice())
        );
        assert_eq!(
            scene.texture_transform(MaterialRef(3)),
            Some(BevyTextureTransform {
                scale: Some([2.0, 3.0]),
                offset: Some([0.1, 0.2])
            })
        );
    }

    #[test]
    fn runtime_scene_state_records_mtoon_and_emissive_writeback() {
        let document = VrmDocument {
            materials: vec![vrm_core::Material {
                khr_emissive_strength: Feature::Present(vrm_core::EmissiveStrength(3.0)),
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    textures: MtoonTextureSet {
                        main_texture: Some(TextureRef(1)),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut scene = BevyRuntimeSceneState::default();

        apply_mtoon_pipeline_hints(&mut scene, &document).unwrap();
        vrm_adapter::apply_emissive_strengths(&mut scene, &document).unwrap();

        assert!(matches!(
            scene.mtoon_pipeline_passes(MaterialRef(0)),
            Some([MtoonPipelinePass::Base(_)])
        ));
        assert_eq!(scene.emissive_intensity(MaterialRef(0)), Some(3.0));
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
