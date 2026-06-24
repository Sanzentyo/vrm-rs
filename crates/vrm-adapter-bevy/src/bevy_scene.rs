//! Bevy scene helpers for loaded VRM/VRMA assets.
//!
//! The API in this module deliberately accepts `vrm_io::LoadedVrm` instead of
//! owning file IO. Apps can load from disk, network, or an asset server bridge,
//! then hand the parsed data to these spawn/update helpers.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::mesh::{Indices, Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::{
    AlphaMode, App, Assets, Color, Commands, Component, Entity, GlobalTransform, Handle,
    LinearRgba, Plugin, Query, Res, ResMut, Time, Transform, Update, default,
};
use bevy::render::render_resource::{
    Extent3d, Face, PrimitiveTopology, TextureDimension, TextureFormat,
};
use glam::Mat4;
use std::sync::Arc;
use vrm_adapter::{
    AdapterError, HeadlessAdapterError, HeadlessSceneState, HumanoidPoseRig, WorldMatrixAccess,
    WorldTransformUpdate, apply_vrma_animation_frame_with_look_at,
};
use vrm_core::{Feature, NodeRef, VrmAnimation};
use vrm_io::{CpuRgba8Image, GltfAlphaMode, GltfMaterialData, GltfPrimitiveData, LoadedVrm};
use vrm_runtime::sample_vrm_animation;

/// Adds the default VRM animation/update system for Bevy-rendered instances.
#[derive(Clone, Copy, Debug, Default)]
pub struct BevyVrmScenePlugin;

impl Plugin for BevyVrmScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, update_vrm_scene_instances);
    }
}

/// How mesh vertices are oriented when baked into Bevy world space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum BevyVrmOrientation {
    /// Use the glTF/VRM extracted transform matrices as-is.
    Identity,
    /// Rotate the baked mesh by 180 degrees around Y, matching the capture
    /// examples' default front-facing Bevy camera convention.
    #[default]
    FrontFacingBevy,
    /// Use an explicit orientation matrix.
    Custom(Mat4),
}

impl BevyVrmOrientation {
    pub fn matrix(self) -> Mat4 {
        match self {
            Self::Identity => Mat4::IDENTITY,
            Self::FrontFacingBevy => Mat4::from_rotation_y(std::f32::consts::PI),
            Self::Custom(matrix) => matrix,
        }
    }
}

/// Material policy for the stock Bevy scene bridge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BevyVrmMaterialMode {
    /// Use `StandardMaterial` in unlit mode with base color textures.
    #[default]
    UnlitBaseColor,
    /// Use Bevy PBR lighting with the extracted glTF base material values.
    StandardPbr,
}

/// Spawn-time options for `spawn_vrm_scene`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BevyVrmSpawnConfig {
    pub orientation: BevyVrmOrientation,
    pub material_mode: BevyVrmMaterialMode,
    pub root_transform: Transform,
    pub animation: Option<BevyVrmAnimationClip>,
}

/// Runtime playback options for an attached VRMA clip.
#[derive(Clone, Debug, PartialEq)]
pub struct BevyVrmAnimationPlayback {
    pub elapsed: f32,
    pub speed: f32,
    pub looping: bool,
    pub paused: bool,
}

impl Default for BevyVrmAnimationPlayback {
    fn default() -> Self {
        Self {
            elapsed: 0.0,
            speed: 1.0,
            looping: true,
            paused: false,
        }
    }
}

/// A VRMA clip attached to a Bevy VRM instance.
#[derive(Clone, Debug, PartialEq)]
pub struct BevyVrmAnimationClip {
    pub animation: VrmAnimation,
    pub playback: BevyVrmAnimationPlayback,
}

impl BevyVrmAnimationClip {
    pub fn new(animation: VrmAnimation) -> Self {
        Self {
            animation,
            playback: BevyVrmAnimationPlayback::default(),
        }
    }
}

/// Component placed on the root entity returned by `spawn_vrm_scene`.
#[derive(Clone, Debug, Component)]
pub struct BevyVrmInstance {
    pub loaded: Arc<LoadedVrm>,
    pub scene: HeadlessSceneState,
    pub rig: HumanoidPoseRig,
    pub animation: Option<BevyVrmAnimationClip>,
    pub orientation: BevyVrmOrientation,
    mesh_bindings: Vec<BevyVrmMeshBinding>,
}

impl BevyVrmInstance {
    pub fn set_animation(&mut self, animation: VrmAnimation) {
        self.animation = Some(BevyVrmAnimationClip::new(animation));
    }

    pub fn clear_animation(&mut self) {
        self.animation = None;
    }

    pub fn mesh_count(&self) -> usize {
        self.mesh_bindings.len()
    }

    pub fn tick(
        &mut self,
        delta_seconds: f32,
        meshes: &mut Assets<Mesh>,
    ) -> Result<(), BevyVrmSceneError> {
        if let Some(clip) = &mut self.animation
            && !clip.playback.paused
        {
            clip.playback.elapsed += delta_seconds * clip.playback.speed;
            let sample_time = normalized_animation_time(
                clip.playback.elapsed,
                clip.animation.duration,
                clip.playback.looping,
            );
            let frame = sample_vrm_animation(&clip.animation, sample_time);
            apply_vrma_animation_frame_with_look_at(
                &mut self.scene,
                &mut self.rig,
                self.loaded.model().document(),
                &frame,
            )?;
        }
        self.scene.update_world_transforms()?;
        self.update_meshes(meshes)
    }

    pub fn update_meshes(&self, meshes: &mut Assets<Mesh>) -> Result<(), BevyVrmSceneError> {
        let world_matrices = self.world_matrices()?;
        let orientation = self.orientation.matrix();
        for binding in &self.mesh_bindings {
            let node = self
                .loaded
                .scene
                .nodes
                .get(binding.node)
                .ok_or(BevyVrmSceneError::MissingNode(binding.node))?;
            let mesh = self
                .loaded
                .meshes
                .get(binding.mesh)
                .ok_or(BevyVrmSceneError::MissingMesh(binding.mesh))?;
            let primitive = mesh.primitives.get(binding.primitive).ok_or(
                BevyVrmSceneError::MissingPrimitive {
                    mesh: binding.mesh,
                    primitive: binding.primitive,
                },
            )?;
            let morph_weights = active_morph_weights(&self.scene, binding.node, node, mesh);
            let world = orientation
                * world_matrices
                    .get(binding.node)
                    .copied()
                    .unwrap_or(node.world_matrix);
            let skin_matrices = node.skin.and_then(|skin| {
                self.loaded.skins.get(skin).map(|skin| {
                    skin.joint_matrices(&self.loaded.scene, &world_matrices, orientation)
                })
            });
            let mut mesh_asset = meshes
                .get_mut(&binding.handle)
                .ok_or(BevyVrmSceneError::MissingBevyMesh)?;
            *mesh_asset = bevy_mesh_from_primitive(
                primitive,
                &morph_weights,
                world,
                skin_matrices.as_deref(),
            )?;
        }
        Ok(())
    }

    fn world_matrices(&self) -> Result<Vec<Mat4>, BevyVrmSceneError> {
        self.loaded
            .scene
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                self.scene
                    .world_matrix(NodeRef(index))
                    .map_or(Ok(node.world_matrix), Ok)
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
struct BevyVrmMeshBinding {
    node: usize,
    mesh: usize,
    primitive: usize,
    handle: Handle<Mesh>,
}

#[derive(Debug, thiserror::Error)]
pub enum BevyVrmSceneError {
    #[error(transparent)]
    Headless(#[from] HeadlessAdapterError),
    #[error(transparent)]
    Adapter(#[from] AdapterError<HeadlessAdapterError>),
    #[error("node {0} is not available in the loaded scene")]
    MissingNode(usize),
    #[error("mesh {0} is not available in the loaded scene")]
    MissingMesh(usize),
    #[error("primitive {primitive} is not available in mesh {mesh}")]
    MissingPrimitive { mesh: usize, primitive: usize },
    #[error("primitive geometry is internally inconsistent and cannot be baked into a Bevy mesh")]
    InvalidPrimitiveGeometry,
    #[error("a Bevy mesh handle owned by the VRM instance no longer resolves")]
    MissingBevyMesh,
    #[error("image {0} could not be converted to RGBA8")]
    InvalidImage(usize),
}

/// Spawn a loaded VRM as Bevy `Mesh3d` + `StandardMaterial` entities.
pub fn spawn_vrm_scene(
    commands: &mut Commands<'_, '_>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    loaded: LoadedVrm,
    config: BevyVrmSpawnConfig,
) -> Result<Entity, BevyVrmSceneError> {
    let loaded = Arc::new(loaded);
    let mut scene = headless_scene_from_loaded(&loaded)?;
    scene.update_world_transforms()?;
    let rig = HumanoidPoseRig::capture(&scene, loaded.model().document())?;
    let mut instance = BevyVrmInstance {
        loaded: Arc::clone(&loaded),
        scene,
        rig,
        animation: config.animation,
        orientation: config.orientation,
        mesh_bindings: Vec::new(),
    };

    let material_handles = material_handles(&loaded, materials, images, config.material_mode)?;
    let root = commands
        .spawn((config.root_transform, GlobalTransform::default()))
        .id();

    let world_matrices = instance.world_matrices()?;
    let orientation = config.orientation.matrix();
    for (node_index, node) in loaded.scene.nodes.iter().enumerate() {
        let Some(mesh_index) = node.mesh else {
            continue;
        };
        let mesh = loaded
            .meshes
            .get(mesh_index)
            .ok_or(BevyVrmSceneError::MissingMesh(mesh_index))?;
        let morph_weights = active_morph_weights(&instance.scene, node_index, node, mesh);
        let world = orientation
            * world_matrices
                .get(node_index)
                .copied()
                .unwrap_or(node.world_matrix);
        let skin_matrices = node.skin.and_then(|skin| {
            loaded
                .skins
                .get(skin)
                .map(|skin| skin.joint_matrices(&loaded.scene, &world_matrices, orientation))
        });
        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            let mesh_handle = meshes.add(bevy_mesh_from_primitive(
                primitive,
                &morph_weights,
                world,
                skin_matrices.as_deref(),
            )?);
            let material_handle = primitive
                .material
                .and_then(|material| material_handles.get(material).cloned())
                .unwrap_or_else(|| materials.add(StandardMaterial::default()));
            let child = commands
                .spawn((
                    Mesh3d(mesh_handle.clone()),
                    MeshMaterial3d(material_handle),
                    Transform::default(),
                    crate::VrmNode(NodeRef(node_index)),
                    crate::BevyVrmVisibility::default(),
                ))
                .id();
            commands.entity(root).add_child(child);
            instance.mesh_bindings.push(BevyVrmMeshBinding {
                node: node_index,
                mesh: mesh_index,
                primitive: primitive_index,
                handle: mesh_handle,
            });
        }
    }

    commands.entity(root).insert(instance);
    Ok(root)
}

/// System that advances all Bevy-spawned VRM instances and refreshes CPU-baked meshes.
pub fn update_vrm_scene_instances(
    time: Res<'_, Time>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut instances: Query<'_, '_, &mut BevyVrmInstance>,
) {
    for mut instance in &mut instances {
        if let Err(error) = instance.tick(time.delta_secs(), &mut meshes) {
            bevy::log::warn!("failed to update VRM scene instance: {error}");
        }
    }
}

/// Extract the first VRMA animation from a loaded `.vrma`/`.gltf` payload.
pub fn animation_from_loaded(loaded: &LoadedVrm) -> Option<VrmAnimation> {
    let document = loaded.model().document();
    match &document.animation {
        Feature::Present(animation) => Some(animation.clone()),
        Feature::Absent => document.animations.first().cloned(),
    }
}

fn normalized_animation_time(elapsed: f32, duration: f32, looping: bool) -> f32 {
    if duration <= f32::EPSILON {
        return 0.0;
    }
    if looping {
        elapsed.rem_euclid(duration)
    } else {
        elapsed.clamp(0.0, duration)
    }
}

fn material_handles(
    loaded: &LoadedVrm,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    mode: BevyVrmMaterialMode,
) -> Result<Vec<Handle<StandardMaterial>>, BevyVrmSceneError> {
    loaded
        .gltf_materials
        .iter()
        .enumerate()
        .map(|(index, material)| {
            let base_texture = loaded
                .material_base_texture_rgba8_image(Some(index))
                .map(|image| images.add(bevy_image_from_rgba8(image)));
            Ok(materials.add(bevy_standard_material(material, base_texture, mode)))
        })
        .collect()
}

fn bevy_standard_material(
    material: &GltfMaterialData,
    base_texture: Option<Handle<Image>>,
    mode: BevyVrmMaterialMode,
) -> StandardMaterial {
    let base = material.base_color_factor;
    StandardMaterial {
        base_color: Color::srgba(base[0], base[1], base[2], base[3]),
        base_color_texture: base_texture,
        alpha_mode: match material.alpha_mode {
            GltfAlphaMode::Opaque => AlphaMode::Opaque,
            GltfAlphaMode::Mask => AlphaMode::Mask(material.alpha_cutoff.unwrap_or(0.5)),
            GltfAlphaMode::Blend => AlphaMode::Blend,
        },
        double_sided: material.double_sided,
        cull_mode: (!material.double_sided).then_some(Face::Back),
        unlit: mode == BevyVrmMaterialMode::UnlitBaseColor,
        metallic: material.metallic_factor,
        perceptual_roughness: material.roughness_factor,
        emissive: LinearRgba::rgb(
            material.emissive_factor[0],
            material.emissive_factor[1],
            material.emissive_factor[2],
        ),
        ..default()
    }
}

fn bevy_image_from_rgba8(image: CpuRgba8Image) -> Image {
    Image::new(
        Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        image.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn bevy_mesh_from_primitive(
    primitive: &GltfPrimitiveData,
    morph_weights: &[f32],
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
) -> Result<Mesh, BevyVrmSceneError> {
    let transformed = primitive
        .transformed_vertices(morph_weights, world, skin_matrices)
        .ok_or(BevyVrmSceneError::InvalidPrimitiveGeometry)?;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        transformed
            .iter()
            .map(|vertex| vertex.position.to_array())
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        transformed
            .iter()
            .map(|vertex| vertex.normal.to_array())
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_TANGENT,
        transformed
            .iter()
            .map(|vertex| vertex.tangent.to_array())
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        transformed
            .iter()
            .map(|vertex| vertex.tex_coord_0)
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        transformed
            .iter()
            .map(|vertex| vertex.color_0)
            .collect::<Vec<_>>(),
    );
    mesh.insert_indices(Indices::U32(primitive.indices.clone()));
    Ok(mesh)
}

fn active_morph_weights(
    scene: &HeadlessSceneState,
    node_index: usize,
    node: &vrm_io::GltfNodeRest,
    mesh: &vrm_io::GltfMeshData,
) -> Vec<f32> {
    let mut weights = if node.weights.is_empty() {
        mesh.weights.clone()
    } else {
        node.weights.clone()
    };
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

fn headless_scene_from_loaded(
    loaded: &LoadedVrm,
) -> Result<HeadlessSceneState, HeadlessAdapterError> {
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

    #[test]
    fn normalized_animation_time_loops_and_clamps() {
        assert_eq!(normalized_animation_time(2.5, 1.0, true), 0.5);
        assert_eq!(normalized_animation_time(2.5, 1.0, false), 1.0);
        assert_eq!(normalized_animation_time(2.5, 0.0, true), 0.0);
    }
}
