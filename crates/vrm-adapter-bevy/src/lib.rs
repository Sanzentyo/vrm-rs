//! Bevy integration skeleton for `vrm-rs`.
//!
//! This crate intentionally starts with registry, descriptor bridge, and runtime
//! plugin marker types. Runtime systems can build on these without pulling
//! renderer policy into `vrm-core` or `vrm-adapter`.

use bevy::prelude::{
    App, Asset, ChildOf, Component, Entity, Handle, IntoScheduleConfigs, Plugin, Quat as BevyQuat,
    Query, Res, ResMut, Resource, Transform as BevyTransform, Update, Vec3 as BevyVec3,
};
use glam::{Quat, Vec3};
use std::collections::{HashMap, HashSet};
use vrm_adapter::{
    ConstraintRestAccess, MaterialAccess, MorphTargetAccess, MtoonMaterialDescriptor,
    MtoonMaterializationOptions, MtoonPipelineAccess, SceneGraph, SpringRestMap, TransformAccess,
    ViewMode, VisibilityAccess, VrmRuntimeDriver, WorldTransformAccess, WorldTransformUpdate,
};
use vrm_core::Transform;
use vrm_core::{
    MaterialRef, MtoonAlphaMode, MtoonCullMode, MtoonPipelinePass, NodeRef, TextureRef, VrmDocument,
};
use vrm_runtime::{CenterSpringRuntimeState, ConstraintRestState, RuntimeEvents};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Component)]
pub struct VrmNode(pub NodeRef);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Component)]
pub struct VrmMaterialBinding(pub MaterialRef);

#[derive(Clone, Debug, Default, PartialEq, Component)]
pub struct BevyVrmMorphWeights {
    pub weights: HashMap<usize, f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Component)]
pub struct BevyVrmMaterialState {
    pub colors: HashMap<String, Vec<f32>>,
    pub texture_transform: Option<BevyTextureTransform>,
    pub emissive_intensity: Option<f32>,
    pub mtoon_pipeline_passes: Vec<MtoonPipelinePass>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Component)]
pub struct BevyVrmVisibility {
    pub visible: bool,
}

impl Default for BevyVrmVisibility {
    fn default() -> Self {
        Self { visible: true }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Resource)]
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

#[derive(Clone, Debug, Default, PartialEq, Resource)]
pub struct BevyVrmDocument {
    pub document: VrmDocument,
}

#[derive(Clone, Debug, Default, PartialEq, Resource)]
pub struct BevyVrmRuntimeEvents {
    pub events: RuntimeEvents,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Resource)]
pub struct BevyVrmRuntimeState {
    pub vrm0_orientation_applied: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Resource)]
pub struct BevyVrmSpringParityState {
    pub rest: Option<SpringRestMap>,
    pub runtime: Option<CenterSpringRuntimeState>,
}

impl BevyVrmSpringParityState {
    pub fn clear(&mut self) {
        self.rest = None;
        self.runtime = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BevyVrmSpringRecaptureReason {
    ModelChanged,
    RestPoseChanged,
    SpringSetupChanged,
    ManualReset,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Resource)]
pub struct BevyVrmSpringParityRecapture {
    reasons: Vec<BevyVrmSpringRecaptureReason>,
}

impl BevyVrmSpringParityRecapture {
    pub fn request(&mut self, reason: BevyVrmSpringRecaptureReason) {
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
    }

    pub fn request_model_changed(&mut self) {
        self.request(BevyVrmSpringRecaptureReason::ModelChanged);
    }

    pub fn request_rest_pose_changed(&mut self) {
        self.request(BevyVrmSpringRecaptureReason::RestPoseChanged);
    }

    pub fn request_spring_setup_changed(&mut self) {
        self.request(BevyVrmSpringRecaptureReason::SpringSetupChanged);
    }

    pub fn request_manual_reset(&mut self) {
        self.request(BevyVrmSpringRecaptureReason::ManualReset);
    }

    pub fn is_requested(&self) -> bool {
        !self.reasons.is_empty()
    }

    pub fn reasons(&self) -> &[BevyVrmSpringRecaptureReason] {
        &self.reasons
    }

    pub fn take(&mut self) -> Vec<BevyVrmSpringRecaptureReason> {
        std::mem::take(&mut self.reasons)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Resource)]
pub struct BevyVrmRuntimeError {
    pub message: Option<String>,
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

impl ConstraintRestAccess for BevyRuntimeSceneState {
    type Error = BevyAdapterError;

    fn constraint_rest_state(
        &self,
        destination: NodeRef,
        source: NodeRef,
    ) -> Result<ConstraintRestState, Self::Error> {
        Ok(ConstraintRestState::new(
            self.local_transform(destination)?.rotation,
            self.local_transform(source)?.rotation,
        ))
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

pub fn to_bevy_transform(transform: Transform) -> BevyTransform {
    BevyTransform {
        translation: BevyVec3::from_array(transform.translation.to_array()),
        rotation: BevyQuat::from_array(transform.rotation.to_array()),
        scale: BevyVec3::from_array(transform.scale.to_array()),
    }
}

pub fn from_bevy_transform(transform: &BevyTransform) -> Transform {
    Transform {
        translation: Vec3::from_array(transform.translation.to_array()),
        rotation: Quat::from_array(transform.rotation.to_array()),
        scale: Vec3::from_array(transform.scale.to_array()),
    }
}

pub type BevyTransformReadItem<'a> = (
    Entity,
    &'a VrmNode,
    &'a BevyTransform,
    Option<&'a BevyVrmVisibility>,
    Option<&'a ChildOf>,
);

pub fn read_bevy_transforms_into_scene_state(
    mut scene: ResMut<BevyRuntimeSceneState>,
    query: Query<BevyTransformReadItem<'_>>,
) {
    let rows = query
        .iter()
        .map(|(entity, node, transform, visibility, child_of)| {
            (
                entity,
                node.0,
                from_bevy_transform(transform),
                visibility.map(|visibility| visibility.visible),
                child_of.map(|child_of| child_of.0),
            )
        })
        .collect::<Vec<_>>();
    let nodes_by_entity = rows
        .iter()
        .map(|(entity, node, _, _, _)| (*entity, *node))
        .collect::<HashMap<_, _>>();

    scene.parents.clear();
    scene.children.clear();
    for (entity, node, local, visibility, _) in &rows {
        scene.nodes.insert(*node, *entity);
        scene.local_transforms.insert(*entity, *local);
        scene.world_transforms.insert(*entity, *local);
        if let Some(visible) = visibility {
            scene.visibility.insert(*entity, *visible);
        } else {
            scene.visibility.entry(*entity).or_insert(true);
        }
    }

    for (_, node, _, _, parent_entity) in rows {
        if let Some(parent) = parent_entity.and_then(|entity| nodes_by_entity.get(&entity).copied())
        {
            let _ = scene.set_parent(node, Some(parent));
        }
    }
    let _ = scene.update_world_transforms();
}

pub fn initialize_spring_parity_state(
    scene: Res<BevyRuntimeSceneState>,
    document: Res<BevyVrmDocument>,
    mut spring: ResMut<BevyVrmSpringParityState>,
    mut recapture: ResMut<BevyVrmSpringParityRecapture>,
    mut last_error: ResMut<BevyVrmRuntimeError>,
) {
    let recapture_requested = recapture.is_requested();
    if recapture_requested {
        spring.clear();
        recapture.take();
    }
    if spring.rest.is_some() && spring.runtime.is_some() {
        return;
    }
    let Some(system) = document.document.spring_bone.as_ref() else {
        spring.clear();
        return;
    };
    match SpringRestMap::capture(&*scene, system) {
        Ok(rest) => {
            spring.runtime = Some(rest.runtime_state(system));
            spring.rest = Some(rest);
            last_error.message = None;
        }
        Err(error) => {
            spring.clear();
            last_error.message = Some(format!("{error:?}"));
        }
    }
}

pub fn write_scene_state_transforms(
    scene: Res<BevyRuntimeSceneState>,
    mut query: Query<(&VrmNode, &mut BevyTransform)>,
) {
    for (node, mut transform) in &mut query {
        if let Ok(local) = scene.local_transform(node.0) {
            *transform = to_bevy_transform(local);
        }
    }
}

pub fn write_scene_state_visibility(
    scene: Res<BevyRuntimeSceneState>,
    mut query: Query<(&VrmNode, &mut BevyVrmVisibility)>,
) {
    for (node, mut visibility) in &mut query {
        if let Ok(visible) = scene.is_visible(node.0) {
            visibility.visible = visible;
        }
    }
}

pub fn write_scene_state_morph_weights(
    scene: Res<BevyRuntimeSceneState>,
    mut query: Query<(&VrmNode, &mut BevyVrmMorphWeights)>,
) {
    for (node, mut morphs) in &mut query {
        morphs.weights = scene
            .morph_weights
            .iter()
            .filter_map(|((candidate, index), weight)| {
                (*candidate == node.0).then_some((*index, *weight))
            })
            .collect();
    }
}

pub fn write_scene_state_materials(
    scene: Res<BevyRuntimeSceneState>,
    mut query: Query<(&VrmMaterialBinding, &mut BevyVrmMaterialState)>,
) {
    for (binding, mut material) in &mut query {
        material.colors = scene
            .material_colors
            .iter()
            .filter_map(|((candidate, property), color)| {
                (*candidate == binding.0).then_some((property.clone(), color.clone()))
            })
            .collect();
        material.texture_transform = scene.texture_transform(binding.0);
        material.emissive_intensity = scene.emissive_intensity(binding.0);
        material.mtoon_pipeline_passes = scene
            .mtoon_pipeline_passes(binding.0)
            .map_or_else(Vec::new, ToOwned::to_owned);
    }
}

pub fn tick_scene_state_runtime(
    mut scene: ResMut<BevyRuntimeSceneState>,
    document: Res<BevyVrmDocument>,
    events: Res<BevyVrmRuntimeEvents>,
    config: Res<BevyVrmRuntimeConfig>,
    mut state: ResMut<BevyVrmRuntimeState>,
    mut spring: ResMut<BevyVrmSpringParityState>,
    mut last_error: ResMut<BevyVrmRuntimeError>,
) {
    let mut driver = VrmRuntimeDriver::new(&document.document)
        .with_runtime_events(&events.events)
        .with_view_mode(config.view_mode)
        .with_vrm0_orientation(config.apply_vrm0_orientation);
    if let Some(root) = config.root_node {
        driver = driver.with_root(root);
    }
    driver.vrm0_orientation_applied = state.vrm0_orientation_applied;

    let result = if config.use_spring_parity {
        if let BevyVrmSpringParityState {
            rest: Some(rest),
            runtime: Some(runtime),
        } = &mut *spring
        {
            driver.tick_with_spring_parity(&mut *scene, Some((&*rest, runtime)))
        } else {
            driver.tick_with_spring_parity(&mut *scene, None)
        }
    } else {
        driver.tick(&mut *scene, None)
    };

    state.vrm0_orientation_applied = driver.vrm0_orientation_applied;
    last_error.message = result.err().map(|error| format!("{error:?}"));
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
    pub root_node: Option<NodeRef>,
    pub apply_vrm0_orientation: bool,
    pub use_spring_parity: bool,
}

impl Default for BevyVrmRuntimeConfig {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::ThirdPerson,
            root_node: Some(NodeRef(0)),
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
        app.insert_resource(self.config.clone())
            .init_resource::<BevyRuntimeSceneState>()
            .init_resource::<BevyVrmDocument>()
            .init_resource::<BevyVrmRuntimeEvents>()
            .init_resource::<BevyVrmRuntimeState>()
            .init_resource::<BevyVrmSpringParityState>()
            .init_resource::<BevyVrmSpringParityRecapture>()
            .init_resource::<BevyVrmRuntimeError>()
            .add_systems(
                Update,
                (
                    tick_scene_state_runtime,
                    write_scene_state_transforms,
                    write_scene_state_visibility,
                    write_scene_state_morph_weights,
                    write_scene_state_materials,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrm_adapter::{SpringRestMap, VrmRuntimeDriver, apply_mtoon_pipeline_hints};
    use vrm_core::{
        Feature, MtoonAlphaMode, MtoonMaterial, MtoonRenderQueue, MtoonTextureSet, Spring,
        SpringBoneSystem, SpringJoint,
    };
    use vrm_runtime::{AppliedExpression, DeltaTime, RuntimeEvents};

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
    fn runtime_driver_ticks_against_bevy_scene_state() {
        let mut scene = BevyRuntimeSceneState::default();
        scene.insert_node(
            NodeRef(0),
            Entity::from_raw_u32(1).unwrap(),
            Transform::default(),
        );
        scene.insert_node(
            NodeRef(1),
            Entity::from_raw_u32(2).unwrap(),
            Transform::default(),
        );
        scene.set_parent(NodeRef(1), Some(NodeRef(0))).unwrap();
        let document = VrmDocument {
            first_person: Feature::Present(vrm_core::FirstPerson {
                mesh_annotations: vec![vrm_core::FirstPersonMeshAnnotation {
                    node: NodeRef(1),
                    kind: vrm_core::FirstPersonAnnotation::FirstPersonOnly,
                }],
            }),
            materials: vec![vrm_core::Material {
                khr_emissive_strength: Feature::Present(vrm_core::EmissiveStrength(4.0)),
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let events = RuntimeEvents {
            expressions: vec![AppliedExpression {
                name: "blink".to_owned(),
                effective_weight: 0.25,
                binds: vec![vrm_core::ExpressionBind::MorphTarget {
                    node: NodeRef(0),
                    index: 3,
                    weight: 100.0,
                }],
            }],
            ..Default::default()
        };
        let mut driver = VrmRuntimeDriver::new(&document)
            .with_runtime_events(&events)
            .with_view_mode(ViewMode::ThirdPerson);

        driver.tick(&mut scene, None).unwrap();

        assert_eq!(scene.morph_weight(NodeRef(0), 3), Some(25.0));
        assert!(matches!(
            scene.mtoon_pipeline_passes(MaterialRef(0)),
            Some([MtoonPipelinePass::Base(_)])
        ));
        assert_eq!(scene.emissive_intensity(MaterialRef(0)), Some(4.0));
        assert!(!scene.is_visible(NodeRef(1)).unwrap());
    }

    #[test]
    fn runtime_driver_ticks_spring_parity_against_bevy_scene_state() {
        let mut scene = BevyRuntimeSceneState::default();
        scene.insert_node(
            NodeRef(0),
            Entity::from_raw_u32(1).unwrap(),
            Transform::default(),
        );
        scene.insert_node(
            NodeRef(1),
            Entity::from_raw_u32(2).unwrap(),
            Transform::default(),
        );
        scene.insert_node(
            NodeRef(2),
            Entity::from_raw_u32(3).unwrap(),
            Transform {
                translation: Vec3::Y,
                ..Transform::default()
            },
        );
        scene.set_parent(NodeRef(1), Some(NodeRef(0))).unwrap();
        scene.set_parent(NodeRef(2), Some(NodeRef(1))).unwrap();
        scene.update_world_transforms().unwrap();

        let document = VrmDocument {
            spring_bone: Feature::Present(SpringBoneSystem {
                springs: vec![Spring {
                    joints: vec![SpringJoint {
                        node: NodeRef(1),
                        stiffness: 0.0,
                        gravity_power: 1.0,
                        gravity_dir: Vec3::X,
                        drag_force: 1.0,
                        ..SpringJoint::default()
                    }],
                    ..Spring::default()
                }],
                ..SpringBoneSystem::default()
            }),
            ..VrmDocument::default()
        };
        let system = document.spring_bone.as_ref().unwrap();
        let rest = SpringRestMap::capture(&scene, system).unwrap();
        let mut spring_state = rest.runtime_state(system);
        let events = RuntimeEvents {
            delta: DeltaTime(1.0),
            ..RuntimeEvents::default()
        };
        let mut driver = VrmRuntimeDriver::new(&document).with_runtime_events(&events);

        driver
            .tick_with_spring_parity(&mut scene, Some((&rest, &mut spring_state)))
            .unwrap();

        assert_ne!(
            scene.local_transform(NodeRef(1)).unwrap().rotation,
            Quat::IDENTITY
        );
        let child_world = scene.world_transform(NodeRef(2)).unwrap().translation;
        assert!(child_world.x > 0.6);
        assert!(child_world.y > 0.6);
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
            root_node: None,
            apply_vrm0_orientation: false,
            use_spring_parity: true,
        };

        app.add_plugins(VrmRuntimePlugin {
            config: config.clone(),
        });

        assert_eq!(app.world().resource::<BevyVrmRuntimeConfig>(), &config);
    }

    #[test]
    fn runtime_plugin_ticks_scene_state_between_readback_and_writeback() {
        let mut app = App::new();
        app.add_plugins(VrmRuntimePlugin {
            config: BevyVrmRuntimeConfig {
                view_mode: ViewMode::ThirdPerson,
                root_node: None,
                apply_vrm0_orientation: false,
                use_spring_parity: true,
            },
        });
        app.add_systems(
            Update,
            read_bevy_transforms_into_scene_state.before(tick_scene_state_runtime),
        );

        *app.world_mut().resource_mut::<BevyVrmDocument>() = BevyVrmDocument {
            document: VrmDocument {
                materials: vec![vrm_core::Material {
                    khr_emissive_strength: Feature::Present(vrm_core::EmissiveStrength(6.0)),
                    mtoon: Feature::Present(MtoonMaterial::default()),
                    ..vrm_core::Material::default()
                }],
                ..VrmDocument::default()
            },
        };
        *app.world_mut().resource_mut::<BevyVrmRuntimeEvents>() = BevyVrmRuntimeEvents {
            events: RuntimeEvents {
                expressions: vec![AppliedExpression {
                    name: "blink".to_owned(),
                    effective_weight: 0.5,
                    binds: vec![vrm_core::ExpressionBind::MorphTarget {
                        node: NodeRef(0),
                        index: 1,
                        weight: 100.0,
                    }],
                }],
                ..RuntimeEvents::default()
            },
        };

        let node_entity = app
            .world_mut()
            .spawn((
                VrmNode(NodeRef(0)),
                BevyTransform {
                    translation: BevyVec3::new(2.0, 0.0, 0.0),
                    ..BevyTransform::default()
                },
                BevyVrmMorphWeights::default(),
            ))
            .id();
        let material_entity = app
            .world_mut()
            .spawn((
                VrmMaterialBinding(MaterialRef(0)),
                BevyVrmMaterialState::default(),
            ))
            .id();

        app.update();

        let scene = app.world().resource::<BevyRuntimeSceneState>();
        assert_eq!(
            scene.local_transform(NodeRef(0)).unwrap().translation,
            Vec3::new(2.0, 0.0, 0.0)
        );
        assert_eq!(scene.morph_weight(NodeRef(0), 1), Some(50.0));
        assert!(
            app.world()
                .resource::<BevyVrmRuntimeError>()
                .message
                .is_none()
        );

        assert_eq!(
            app.world()
                .entity(node_entity)
                .get::<BevyVrmMorphWeights>()
                .unwrap()
                .weights
                .get(&1)
                .copied(),
            Some(50.0)
        );
        let material = app
            .world()
            .entity(material_entity)
            .get::<BevyVrmMaterialState>()
            .unwrap();
        assert_eq!(material.emissive_intensity, Some(6.0));
        assert!(matches!(
            material.mtoon_pipeline_passes.as_slice(),
            [MtoonPipelinePass::Base(_)]
        ));
    }

    #[test]
    fn runtime_plugin_ticks_spring_parity_from_bevy_resources() {
        let mut app = App::new();
        app.add_plugins(VrmRuntimePlugin {
            config: BevyVrmRuntimeConfig {
                view_mode: ViewMode::ThirdPerson,
                root_node: None,
                apply_vrm0_orientation: false,
                use_spring_parity: true,
            },
        });
        app.add_systems(
            Update,
            (
                read_bevy_transforms_into_scene_state,
                initialize_spring_parity_state,
            )
                .chain()
                .before(tick_scene_state_runtime),
        );

        *app.world_mut().resource_mut::<BevyVrmDocument>() = BevyVrmDocument {
            document: VrmDocument {
                spring_bone: Feature::Present(SpringBoneSystem {
                    springs: vec![Spring {
                        joints: vec![SpringJoint {
                            node: NodeRef(1),
                            stiffness: 0.0,
                            gravity_power: 1.0,
                            gravity_dir: Vec3::X,
                            drag_force: 1.0,
                            ..SpringJoint::default()
                        }],
                        ..Spring::default()
                    }],
                    ..SpringBoneSystem::default()
                }),
                ..VrmDocument::default()
            },
        };
        *app.world_mut().resource_mut::<BevyVrmRuntimeEvents>() = BevyVrmRuntimeEvents {
            events: RuntimeEvents {
                delta: DeltaTime(1.0),
                ..RuntimeEvents::default()
            },
        };

        let joint_entity = app
            .world_mut()
            .spawn((VrmNode(NodeRef(1)), BevyTransform::default()))
            .id();
        app.world_mut().spawn((
            VrmNode(NodeRef(2)),
            BevyTransform {
                translation: BevyVec3::Y,
                ..BevyTransform::default()
            },
            ChildOf(joint_entity),
        ));

        app.update();

        let scene = app.world().resource::<BevyRuntimeSceneState>();
        assert_eq!(scene.parent(NodeRef(2)).unwrap(), Some(NodeRef(1)));
        assert_eq!(scene.children(NodeRef(1)).unwrap(), vec![NodeRef(2)]);
        assert!(
            app.world()
                .resource::<BevyVrmSpringParityState>()
                .rest
                .is_some()
        );
        assert!(
            app.world()
                .resource::<BevyVrmRuntimeError>()
                .message
                .is_none()
        );
        let transform = app
            .world()
            .entity(joint_entity)
            .get::<BevyTransform>()
            .unwrap();
        assert_ne!(transform.rotation, BevyQuat::IDENTITY);
    }

    #[test]
    fn spring_parity_recapture_marker_rebuilds_state() {
        let mut app = App::new();
        app.init_resource::<BevyRuntimeSceneState>()
            .init_resource::<BevyVrmDocument>()
            .init_resource::<BevyVrmSpringParityState>()
            .init_resource::<BevyVrmSpringParityRecapture>()
            .init_resource::<BevyVrmRuntimeError>()
            .add_systems(Update, initialize_spring_parity_state);

        *app.world_mut().resource_mut::<BevyVrmDocument>() = BevyVrmDocument {
            document: VrmDocument {
                spring_bone: Feature::Present(SpringBoneSystem {
                    springs: vec![Spring {
                        joints: vec![SpringJoint {
                            node: NodeRef(1),
                            ..SpringJoint::default()
                        }],
                        ..Spring::default()
                    }],
                    ..SpringBoneSystem::default()
                }),
                ..VrmDocument::default()
            },
        };
        {
            let mut scene = app.world_mut().resource_mut::<BevyRuntimeSceneState>();
            scene.insert_node(
                NodeRef(1),
                Entity::from_raw_u32(1).unwrap(),
                Transform::default(),
            );
            scene.insert_node(
                NodeRef(2),
                Entity::from_raw_u32(2).unwrap(),
                Transform {
                    translation: Vec3::Y,
                    ..Transform::default()
                },
            );
            scene.set_parent(NodeRef(2), Some(NodeRef(1))).unwrap();
            scene.update_world_transforms().unwrap();
        }

        app.update();
        let first = app
            .world()
            .resource::<BevyVrmSpringParityState>()
            .rest
            .clone();
        assert!(first.is_some());

        {
            let mut scene = app.world_mut().resource_mut::<BevyRuntimeSceneState>();
            scene
                .set_local_transform(
                    NodeRef(2),
                    Transform {
                        translation: Vec3::Z,
                        ..Transform::default()
                    },
                )
                .unwrap();
            scene.update_world_transforms().unwrap();
        }
        app.world_mut()
            .resource_mut::<BevyVrmSpringParityRecapture>()
            .request_rest_pose_changed();
        assert!(
            app.world()
                .resource::<BevyVrmSpringParityRecapture>()
                .is_requested()
        );

        app.update();

        let recapture = app.world().resource::<BevyVrmSpringParityRecapture>();
        assert!(!recapture.is_requested());
        let second = app
            .world()
            .resource::<BevyVrmSpringParityState>()
            .rest
            .clone();
        assert!(second.is_some());
        assert_ne!(first, second);
        assert!(
            app.world()
                .resource::<BevyVrmRuntimeError>()
                .message
                .is_none()
        );
    }

    #[test]
    fn runtime_plugin_writes_scene_state_to_bevy_components() {
        let mut app = App::new();
        app.add_plugins(VrmRuntimePlugin::default());

        let mut scene = BevyRuntimeSceneState::default();
        scene.insert_node(
            NodeRef(0),
            Entity::from_raw_u32(1).unwrap(),
            Transform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::from_rotation_z(0.5),
                scale: Vec3::splat(2.0),
            },
        );
        scene.set_node_visible(NodeRef(0), false).unwrap();
        scene.set_morph_weight(NodeRef(0), 4, 75.0).unwrap();
        scene
            .set_material_color(MaterialRef(2), "_Color", &[0.1, 0.2, 0.3, 0.4])
            .unwrap();
        scene.set_emissive_intensity(MaterialRef(2), 5.0).unwrap();
        let passes = MtoonMaterial::default().pipeline_passes();
        scene
            .set_mtoon_pipeline_passes(MaterialRef(2), &passes)
            .unwrap();
        *app.world_mut().resource_mut::<BevyRuntimeSceneState>() = scene;

        let node_entity = app
            .world_mut()
            .spawn((
                VrmNode(NodeRef(0)),
                BevyTransform::default(),
                BevyVrmVisibility::default(),
                BevyVrmMorphWeights::default(),
            ))
            .id();
        let material_entity = app
            .world_mut()
            .spawn((
                VrmMaterialBinding(MaterialRef(2)),
                BevyVrmMaterialState::default(),
            ))
            .id();

        app.update();

        let node = app.world().entity(node_entity);
        let transform = node.get::<BevyTransform>().unwrap();
        assert_eq!(transform.translation, BevyVec3::new(1.0, 2.0, 3.0));
        assert_eq!(transform.scale, BevyVec3::splat(2.0));
        assert!(!node.get::<BevyVrmVisibility>().unwrap().visible);
        assert_eq!(
            node.get::<BevyVrmMorphWeights>()
                .unwrap()
                .weights
                .get(&4)
                .copied(),
            Some(75.0)
        );

        let material = app
            .world()
            .entity(material_entity)
            .get::<BevyVrmMaterialState>()
            .unwrap();
        assert_eq!(
            material.colors.get("_Color").map(Vec::as_slice),
            Some([0.1, 0.2, 0.3, 0.4].as_slice())
        );
        assert_eq!(material.emissive_intensity, Some(5.0));
        assert!(matches!(
            material.mtoon_pipeline_passes.as_slice(),
            [MtoonPipelinePass::Base(_)]
        ));
    }

    #[test]
    fn scene_state_can_read_bevy_transform_components() {
        let mut app = App::new();
        app.init_resource::<BevyRuntimeSceneState>()
            .add_systems(Update, read_bevy_transforms_into_scene_state);

        let root = app
            .world_mut()
            .spawn((
                VrmNode(NodeRef(6)),
                BevyTransform {
                    translation: BevyVec3::new(1.0, 0.0, 0.0),
                    ..BevyTransform::default()
                },
            ))
            .id();
        let entity = app
            .world_mut()
            .spawn((
                VrmNode(NodeRef(7)),
                BevyTransform {
                    translation: BevyVec3::new(4.0, 5.0, 6.0),
                    rotation: BevyQuat::from_rotation_y(0.25),
                    scale: BevyVec3::splat(0.5),
                },
                BevyVrmVisibility { visible: false },
                ChildOf(root),
            ))
            .id();

        app.update();

        let scene = app.world().resource::<BevyRuntimeSceneState>();
        assert_eq!(scene.nodes.entity(NodeRef(6)), Some(root));
        assert_eq!(scene.nodes.entity(NodeRef(7)), Some(entity));
        assert_eq!(scene.parent(NodeRef(7)).unwrap(), Some(NodeRef(6)));
        assert_eq!(scene.children(NodeRef(6)).unwrap(), vec![NodeRef(7)]);
        assert_eq!(
            scene.local_transform(NodeRef(7)).unwrap().translation,
            Vec3::new(4.0, 5.0, 6.0)
        );
        assert_eq!(
            scene.world_transform(NodeRef(7)).unwrap().translation,
            Vec3::new(5.0, 5.0, 6.0)
        );
        assert_eq!(
            scene.local_transform(NodeRef(7)).unwrap().scale,
            Vec3::splat(0.5)
        );
        assert!(!scene.is_visible(NodeRef(7)).unwrap());
    }
}
