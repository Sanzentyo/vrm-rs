//! glTF/GLB IO for VRM and VRMA assets.

use glam::{Mat4, Quat, Vec3};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;
use vrm_core::{
    ExpressionName, Feature, HumanBoneName, Resolved, RotationTrack, ScalarTrack, Transform,
    TranslationTrack, VrmAnimation, VrmKind, VrmModel,
};
use vrm_protocol::{
    ExtensionBundle, ExtensionMap, NodeConstraintExtension, ProtocolError, VrmExtension,
    parse_root_extensions,
};
use vrm_sans_io::{BuildError, ValidatedAssetBuilder};

#[derive(Clone, Debug)]
pub struct LoadedVrm {
    model: VrmModel<Resolved>,
    pub scene: GltfSceneRest,
    pub buffers: Vec<Vec<u8>>,
    pub images: Vec<ImageData>,
    pub warnings: Vec<VrmIoWarning>,
}

impl LoadedVrm {
    pub fn model(&self) -> &VrmModel<Resolved> {
        &self.model
    }

    pub fn into_model(self) -> VrmModel<Resolved> {
        self.model
    }

    pub fn warnings(&self) -> &[VrmIoWarning] {
        &self.warnings
    }

    pub fn scene(&self) -> &GltfSceneRest {
        &self.scene
    }
}

impl GltfSceneRest {
    fn from_document(document: &gltf::Document) -> Self {
        NodeRestGraph::from_document(document).into_scene_rest(document.nodes().count())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageData {
    pub mime_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfSceneRest {
    pub nodes: Vec<GltfNodeRest>,
}

impl GltfSceneRest {
    pub fn node(&self, index: usize) -> Option<&GltfNodeRest> {
        self.nodes.get(index)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfNodeRest {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub local: Transform,
    pub world: Transform,
    pub world_matrix: Mat4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VrmIoWarning {
    MissingSpecVersion { extension: String, assumed: String },
    DraftSpecVersion { extension: String, version: String },
    UnknownSpecVersion { extension: String, version: String },
    IgnoredAnimationChannel { node: usize, message: String },
}

fn vrma_extension_warnings(extensions: &ExtensionMap) -> Vec<VrmIoWarning> {
    let Some(value) = extensions.get("VRMC_vrm_animation") else {
        return Vec::new();
    };
    match value.get("specVersion").and_then(|value| value.as_str()) {
        None => vec![VrmIoWarning::MissingSpecVersion {
            extension: "VRMC_vrm_animation".to_owned(),
            assumed: "1.0".to_owned(),
        }],
        Some("1.0") => Vec::new(),
        Some("1.0-draft") => vec![VrmIoWarning::DraftSpecVersion {
            extension: "VRMC_vrm_animation".to_owned(),
            version: "1.0-draft".to_owned(),
        }],
        Some(version) => vec![VrmIoWarning::UnknownSpecVersion {
            extension: "VRMC_vrm_animation".to_owned(),
            version: version.to_owned(),
        }],
    }
}

pub fn load_vrm_from_slice(bytes: &[u8]) -> Result<LoadedVrm, VrmIoError> {
    let (document, buffers, images) = gltf::import_slice(bytes)?;
    let scene = GltfSceneRest::from_document(&document);
    let root_extensions = extension_map(document.as_json().extensions.as_ref());
    let mut warnings = vrma_extension_warnings(&root_extensions);
    let mut bundle = parse_root_extensions(&root_extensions)?;
    extract_node_constraints(&document, &mut bundle)?;
    extract_mtoon_materials(&document, &mut bundle)?;
    extract_hdr_emissive_multipliers(&document, &mut bundle)?;
    extract_khr_emissive_strengths(&document, &mut bundle)?;
    validate_vrmc_extension_versions(&bundle)?;
    let vrma_animations = extract_vrma_animations(&document, &buffers, &bundle, &mut warnings)?;

    let image_data = images
        .into_iter()
        .map(|image| ImageData {
            mime_type: image.format.to_mime_type().map(str::to_owned),
            bytes: image.pixels,
        })
        .collect();

    let node_count = document.nodes().count();
    let material_count = document.materials().count();
    let mut asset = ValidatedAssetBuilder::new()
        .with_node_count(node_count)
        .with_material_count(material_count)
        .build(bundle)?;
    expand_vrm0_spring_roots(&mut asset.document, &scene);
    if let Some(animations) = vrma_animations {
        asset.document.animation = animations
            .first()
            .cloned()
            .map_or(Feature::Absent, Feature::Present);
        asset.document.animations = animations;
    }
    let model = asset.resolve();

    Ok(LoadedVrm {
        model,
        scene,
        buffers: buffers.into_iter().map(|buffer| buffer.0).collect(),
        images: image_data,
        warnings,
    })
}

fn extract_vrma_animations(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    bundle: &ExtensionBundle,
    warnings: &mut Vec<VrmIoWarning>,
) -> Result<Option<Vec<VrmAnimation>>, VrmIoError> {
    let Some(VrmExtension::Vrma(vrma)) = &bundle.vrm else {
        return Ok(None);
    };

    let node_map = VrmaNodeMap::from_extension(vrma);
    let rest_pose = VrmaRestPose::from_document(document, &node_map);
    let animations = document
        .animations()
        .map(|animation| {
            let mut result = VrmAnimation {
                rest_hips_position: rest_pose.hips_world_position,
                ..VrmAnimation::default()
            };

            for channel in animation.channels() {
                let node_index = channel.target().node().index();
                let reader = channel
                    .reader(|buffer| buffers.get(buffer.index()).map(|data| data.0.as_slice()));
                let times = reader
                    .read_inputs()
                    .ok_or(VrmIoError::InvalidAnimationChannel {
                        message: "missing animation input accessor".to_owned(),
                    })?
                    .collect::<Vec<_>>();

                if let Some(bone_name) = node_map.humanoid.get(&node_index) {
                    match reader.read_outputs() {
                        Some(gltf::animation::util::ReadOutputs::Translations(values))
                            if *bone_name == HumanBoneName::Hips =>
                        {
                            result.hips_translation = Some(TranslationTrack {
                                times: times.clone(),
                                values: values
                                    .map(Vec3::from_array)
                                    .map(|translation| {
                                        rest_pose
                                            .hips_parent_world_matrix
                                            .transform_point3(translation)
                                    })
                                    .collect(),
                            });
                        }
                        Some(gltf::animation::util::ReadOutputs::Translations(_)) => {
                            warnings.push(VrmIoWarning::IgnoredAnimationChannel {
                                node: node_index,
                                message: "ignored non-hips humanoid translation track".to_owned(),
                            });
                        }
                        Some(gltf::animation::util::ReadOutputs::Rotations(values)) => {
                            let bone_rest = rest_pose
                                .bone_world_rotations
                                .get(bone_name)
                                .copied()
                                .unwrap_or(Quat::IDENTITY);
                            let parent_rest = rest_pose.parent_world_rotation(bone_name);
                            result.humanoid_rotation_tracks.insert(
                                bone_name.clone(),
                                RotationTrack {
                                    times: times.clone(),
                                    values: values
                                        .into_f32()
                                        .map(|[x, y, z, w]| {
                                            parent_rest
                                                * Quat::from_xyzw(x, y, z, w)
                                                * bone_rest.inverse()
                                        })
                                        .collect(),
                                },
                            );
                        }
                        Some(_) => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: format!(
                                    "invalid humanoid animation path for node {node_index}"
                                ),
                            });
                        }
                        None => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: "missing animation output accessor".to_owned(),
                            });
                        }
                    }
                    continue;
                }

                if let Some(expression) = node_map.expressions.get(&node_index) {
                    match reader.read_outputs() {
                        Some(gltf::animation::util::ReadOutputs::Translations(values)) => {
                            let track = ScalarTrack {
                                times: times.clone(),
                                values: values.map(|value| value[0]).collect(),
                            };
                            match expression {
                                VrmaExpressionTarget::Preset(name) => {
                                    result.preset_expression_tracks.insert(name.clone(), track);
                                }
                                VrmaExpressionTarget::Custom(name) => {
                                    result.custom_expression_tracks.insert(name.clone(), track);
                                }
                            }
                        }
                        Some(_) => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: format!(
                                    "invalid expression animation path for node {node_index}"
                                ),
                            });
                        }
                        None => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: "missing animation output accessor".to_owned(),
                            });
                        }
                    }
                    continue;
                }

                if Some(node_index) == node_map.look_at {
                    match reader.read_outputs() {
                        Some(gltf::animation::util::ReadOutputs::Rotations(values)) => {
                            result.look_at_track = Some(RotationTrack {
                                times: times.clone(),
                                values: values
                                    .into_f32()
                                    .map(|[x, y, z, w]| Quat::from_xyzw(x, y, z, w))
                                    .collect(),
                            });
                        }
                        Some(_) => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: format!(
                                    "invalid lookAt animation path for node {node_index}"
                                ),
                            });
                        }
                        None => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: "missing animation output accessor".to_owned(),
                            });
                        }
                    }
                }
            }

            result.duration = result
                .hips_translation
                .as_ref()
                .and_then(|track| track.times.last().copied())
                .into_iter()
                .chain(
                    result
                        .humanoid_rotation_tracks
                        .values()
                        .filter_map(|track| track.times.last().copied()),
                )
                .chain(
                    result
                        .preset_expression_tracks
                        .values()
                        .filter_map(|track| track.times.last().copied()),
                )
                .chain(
                    result
                        .custom_expression_tracks
                        .values()
                        .filter_map(|track| track.times.last().copied()),
                )
                .chain(
                    result
                        .look_at_track
                        .as_ref()
                        .and_then(|track| track.times.last().copied()),
                )
                .fold(0.0, f32::max);

            Ok(result)
        })
        .collect::<Result<Vec<_>, VrmIoError>>()?;

    Ok(Some(animations))
}

#[derive(Clone, Debug, Default)]
struct VrmaNodeMap {
    humanoid: HashMap<usize, HumanBoneName>,
    expressions: HashMap<usize, VrmaExpressionTarget>,
    look_at: Option<usize>,
}

impl VrmaNodeMap {
    fn from_extension(vrma: &vrm_protocol::vrma::VrmcVrmAnimation) -> Self {
        let mut map = Self::default();

        if let Some(humanoid) = &vrma.humanoid {
            for (name, value) in &humanoid.human_bones {
                if let Some(node) = node_from_value(value) {
                    map.humanoid
                        .insert(node, HumanBoneName::from(name.as_str()));
                }
            }
        }

        if let Some(expressions) = &vrma.expressions {
            for (name, value) in expressions.preset.as_ref().into_iter().flatten() {
                if let Some(node) = node_from_value(value) {
                    map.expressions.insert(
                        node,
                        VrmaExpressionTarget::Preset(ExpressionName::from(name.as_str())),
                    );
                }
            }
            for (name, value) in expressions.custom.as_ref().into_iter().flatten() {
                if let Some(node) = node_from_value(value) {
                    map.expressions
                        .insert(node, VrmaExpressionTarget::Custom(name.clone()));
                }
            }
        }

        map.look_at = vrma.look_at.map(|look_at| look_at.node);
        map
    }
}

#[derive(Clone, Debug, Default)]
struct VrmaRestPose {
    bone_world_rotations: HashMap<HumanBoneName, Quat>,
    hips_parent_world_rotation: Quat,
    hips_parent_world_matrix: Mat4,
    hips_world_position: Vec3,
}

impl VrmaRestPose {
    fn from_document(document: &gltf::Document, node_map: &VrmaNodeMap) -> Self {
        let graph = NodeRestGraph::from_document(document);
        let bone_world_rotations = node_map
            .humanoid
            .iter()
            .filter_map(|(node, bone)| {
                graph
                    .world_rotations
                    .get(*node)
                    .copied()
                    .map(|rotation| (bone.clone(), rotation))
            })
            .collect::<HashMap<_, _>>();
        let hips_parent_world_rotation = node_map
            .humanoid
            .iter()
            .find_map(|(node, bone)| {
                (*bone == HumanBoneName::Hips)
                    .then(|| graph.parents.get(*node).and_then(|parent| *parent))
                    .flatten()
            })
            .and_then(|parent| graph.world_rotations.get(parent).copied())
            .unwrap_or(Quat::IDENTITY);
        let hips_node = node_map
            .humanoid
            .iter()
            .find_map(|(node, bone)| (*bone == HumanBoneName::Hips).then_some(*node));
        let hips_parent_world_matrix = hips_node
            .and_then(|node| graph.parents.get(node).and_then(|parent| *parent))
            .and_then(|parent| graph.world_matrices.get(parent).copied())
            .unwrap_or(Mat4::IDENTITY);
        let hips_world_position = hips_node
            .and_then(|node| graph.world_matrices.get(node).copied())
            .map(|matrix| matrix.transform_point3(Vec3::ZERO))
            .unwrap_or(Vec3::ZERO);

        Self {
            bone_world_rotations,
            hips_parent_world_rotation,
            hips_parent_world_matrix,
            hips_world_position,
        }
    }

    fn parent_world_rotation(&self, bone: &HumanBoneName) -> Quat {
        let mut parent = human_bone_parent(bone);
        while let Some(parent_bone) = parent.as_ref() {
            if let Some(rotation) = self.bone_world_rotations.get(parent_bone) {
                return *rotation;
            }
            parent = human_bone_parent(parent_bone);
        }
        self.hips_parent_world_rotation
    }
}

#[derive(Clone, Debug, Default)]
struct NodeRestGraph {
    parents: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    local_transforms: Vec<Transform>,
    world_transforms: Vec<Transform>,
    world_rotations: Vec<Quat>,
    world_matrices: Vec<Mat4>,
}

impl NodeRestGraph {
    fn from_document(document: &gltf::Document) -> Self {
        let node_count = document.nodes().count();
        let mut graph = Self {
            parents: vec![None; node_count],
            children: vec![Vec::new(); node_count],
            local_transforms: vec![Transform::default(); node_count],
            world_transforms: vec![Transform::default(); node_count],
            world_rotations: vec![Quat::IDENTITY; node_count],
            world_matrices: vec![Mat4::IDENTITY; node_count],
        };

        for scene in document.scenes() {
            for node in scene.nodes() {
                graph.visit_node(node, None, Mat4::IDENTITY, Quat::IDENTITY);
            }
        }

        graph
    }

    fn into_scene_rest(self, node_count: usize) -> GltfSceneRest {
        GltfSceneRest {
            nodes: (0..node_count)
                .map(|index| GltfNodeRest {
                    parent: self.parents[index],
                    children: self.children[index].clone(),
                    local: self.local_transforms[index],
                    world: self.world_transforms[index],
                    world_matrix: self.world_matrices[index],
                })
                .collect(),
        }
    }

    fn visit_node(
        &mut self,
        node: gltf::Node<'_>,
        parent: Option<usize>,
        parent_matrix: Mat4,
        parent_rotation: Quat,
    ) {
        let index = node.index();
        let (translation, rotation, scale) = node.transform().decomposed();
        let local_rotation = Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]);
        let local_transform = Transform {
            translation: Vec3::from_array(translation),
            rotation: local_rotation,
            scale: Vec3::from_array(scale),
        };
        let local_matrix = Mat4::from_scale_rotation_translation(
            local_transform.scale,
            local_transform.rotation,
            local_transform.translation,
        );
        let world_matrix = parent_matrix * local_matrix;
        let world_rotation = parent_rotation * local_rotation;
        let (world_scale, world_rotation_decomposed, world_translation) =
            world_matrix.to_scale_rotation_translation();
        self.parents[index] = parent;
        if let Some(parent) = parent {
            self.children[parent].push(index);
        }
        self.local_transforms[index] = local_transform;
        self.world_transforms[index] = Transform {
            translation: world_translation,
            rotation: world_rotation_decomposed,
            scale: world_scale,
        };
        self.world_rotations[index] = world_rotation;
        self.world_matrices[index] = world_matrix;

        for child in node.children() {
            self.visit_node(child, Some(index), world_matrix, world_rotation);
        }
    }
}

fn expand_vrm0_spring_roots(document: &mut vrm_core::VrmDocument, scene: &GltfSceneRest) {
    if document.kind != VrmKind::Vrm0Compat {
        return;
    }
    let Feature::Present(system) = &mut document.spring_bone else {
        return;
    };
    for spring in &mut system.springs {
        spring.joints = spring
            .joints
            .iter()
            .flat_map(|joint| {
                let nodes = scene
                    .node(joint.node.0)
                    .map(|_| scene_descendants_preorder(scene, joint.node))
                    .unwrap_or_else(|| vec![joint.node]);
                nodes.into_iter().map(|node| {
                    let mut joint = joint.clone();
                    joint.node = node;
                    joint
                })
            })
            .collect();
    }
}

fn scene_descendants_preorder(
    scene: &GltfSceneRest,
    root: vrm_core::NodeRef,
) -> Vec<vrm_core::NodeRef> {
    let mut nodes = Vec::new();
    push_scene_descendants_preorder(scene, root, &mut nodes);
    nodes
}

fn push_scene_descendants_preorder(
    scene: &GltfSceneRest,
    node: vrm_core::NodeRef,
    nodes: &mut Vec<vrm_core::NodeRef>,
) {
    nodes.push(node);
    if let Some(rest) = scene.node(node.0) {
        for child in &rest.children {
            push_scene_descendants_preorder(scene, vrm_core::NodeRef(*child), nodes);
        }
    }
}

fn human_bone_parent(bone: &HumanBoneName) -> Option<HumanBoneName> {
    use HumanBoneName::*;
    match bone {
        Hips => None,
        Spine => Some(Hips),
        Chest => Some(Spine),
        UpperChest => Some(Chest),
        Neck => Some(UpperChest),
        Head => Some(Neck),
        LeftEye | RightEye | Jaw => Some(Head),
        LeftUpperLeg => Some(Hips),
        LeftLowerLeg => Some(LeftUpperLeg),
        LeftFoot => Some(LeftLowerLeg),
        LeftToes => Some(LeftFoot),
        RightUpperLeg => Some(Hips),
        RightLowerLeg => Some(RightUpperLeg),
        RightFoot => Some(RightLowerLeg),
        RightToes => Some(RightFoot),
        LeftShoulder => Some(UpperChest),
        LeftUpperArm => Some(LeftShoulder),
        LeftLowerArm => Some(LeftUpperArm),
        LeftHand => Some(LeftLowerArm),
        RightShoulder => Some(UpperChest),
        RightUpperArm => Some(RightShoulder),
        RightLowerArm => Some(RightUpperArm),
        RightHand => Some(RightLowerArm),
        LeftThumbMetacarpal => Some(LeftHand),
        LeftThumbProximal => Some(LeftThumbMetacarpal),
        LeftThumbDistal => Some(LeftThumbProximal),
        LeftIndexProximal => Some(LeftHand),
        LeftIndexIntermediate => Some(LeftIndexProximal),
        LeftIndexDistal => Some(LeftIndexIntermediate),
        LeftMiddleProximal => Some(LeftHand),
        LeftMiddleIntermediate => Some(LeftMiddleProximal),
        LeftMiddleDistal => Some(LeftMiddleIntermediate),
        LeftRingProximal => Some(LeftHand),
        LeftRingIntermediate => Some(LeftRingProximal),
        LeftRingDistal => Some(LeftRingIntermediate),
        LeftLittleProximal => Some(LeftHand),
        LeftLittleIntermediate => Some(LeftLittleProximal),
        LeftLittleDistal => Some(LeftLittleIntermediate),
        RightThumbMetacarpal => Some(RightHand),
        RightThumbProximal => Some(RightThumbMetacarpal),
        RightThumbDistal => Some(RightThumbProximal),
        RightIndexProximal => Some(RightHand),
        RightIndexIntermediate => Some(RightIndexProximal),
        RightIndexDistal => Some(RightIndexIntermediate),
        RightMiddleProximal => Some(RightHand),
        RightMiddleIntermediate => Some(RightMiddleProximal),
        RightMiddleDistal => Some(RightMiddleIntermediate),
        RightRingProximal => Some(RightHand),
        RightRingIntermediate => Some(RightRingProximal),
        RightRingDistal => Some(RightRingIntermediate),
        RightLittleProximal => Some(RightHand),
        RightLittleIntermediate => Some(RightLittleProximal),
        RightLittleDistal => Some(RightLittleIntermediate),
        Custom(_) => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum VrmaExpressionTarget {
    Preset(ExpressionName),
    Custom(String),
}

fn node_from_value(value: &Value) -> Option<usize> {
    value
        .get("node")
        .and_then(Value::as_u64)
        .and_then(|node| usize::try_from(node).ok())
}

pub fn load_vrm_from_path(path: impl AsRef<Path>) -> Result<LoadedVrm, VrmIoError> {
    let bytes = std::fs::read(path)?;
    load_vrm_from_slice(&bytes)
}

fn extension_map<T>(source: Option<&T>) -> ExtensionMap
where
    T: Serialize,
{
    source
        .and_then(|extensions| serde_json::to_value(extensions).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map.into_iter().collect()),
            _ => None,
        })
        .unwrap_or_default()
}

fn extract_node_constraints(
    document: &gltf::Document,
    bundle: &mut ExtensionBundle,
) -> Result<(), VrmIoError> {
    for node in document.nodes() {
        let extensions = extension_map(document.as_json().nodes[node.index()].extensions.as_ref());
        if let Some(value) = extensions.get("VRMC_node_constraint") {
            let constraint = serde_json::from_value(value.clone()).map_err(|err| {
                VrmIoError::InvalidExtension {
                    extension: "VRMC_node_constraint".to_owned(),
                    message: err.to_string(),
                }
            })?;
            bundle.node_constraints.push(NodeConstraintExtension {
                node: node.index(),
                constraint,
            });
        }
    }
    Ok(())
}

fn extract_mtoon_materials(
    document: &gltf::Document,
    bundle: &mut ExtensionBundle,
) -> Result<(), VrmIoError> {
    for material in document.materials() {
        let Some(material_index) = material.index() else {
            continue;
        };
        let extensions = extension_map(
            document.as_json().materials[material_index]
                .extensions
                .as_ref(),
        );
        if let Some(value) = extensions.get("VRMC_materials_mtoon") {
            let mtoon = serde_json::from_value(value.clone()).map_err(|err| {
                VrmIoError::InvalidExtension {
                    extension: "VRMC_materials_mtoon".to_owned(),
                    message: err.to_string(),
                }
            })?;
            bundle.mtoon_materials.insert(material_index, mtoon);
        }
    }
    Ok(())
}

fn extract_hdr_emissive_multipliers(
    document: &gltf::Document,
    bundle: &mut ExtensionBundle,
) -> Result<(), VrmIoError> {
    for material in document.materials() {
        let Some(material_index) = material.index() else {
            continue;
        };
        let extensions = extension_map(
            document.as_json().materials[material_index]
                .extensions
                .as_ref(),
        );
        if let Some(value) = extensions.get("VRMC_materials_hdr_emissiveMultiplier") {
            let multiplier = serde_json::from_value(value.clone()).map_err(|err| {
                VrmIoError::InvalidExtension {
                    extension: "VRMC_materials_hdr_emissiveMultiplier".to_owned(),
                    message: err.to_string(),
                }
            })?;
            bundle
                .hdr_emissive_multipliers
                .insert(material_index, multiplier);
        }
    }
    Ok(())
}

fn extract_khr_emissive_strengths(
    document: &gltf::Document,
    bundle: &mut ExtensionBundle,
) -> Result<(), VrmIoError> {
    for material in document.materials() {
        let Some(material_index) = material.index() else {
            continue;
        };
        let extensions = extension_map(
            document.as_json().materials[material_index]
                .extensions
                .as_ref(),
        );
        if let Some(value) = extensions.get("KHR_materials_emissive_strength") {
            let strength = serde_json::from_value(value.clone()).map_err(|err| {
                VrmIoError::InvalidExtension {
                    extension: "KHR_materials_emissive_strength".to_owned(),
                    message: err.to_string(),
                }
            })?;
            bundle
                .khr_emissive_strengths
                .insert(material_index, strength);
        }
    }
    Ok(())
}

fn validate_vrmc_extension_versions(bundle: &ExtensionBundle) -> Result<(), VrmIoError> {
    if let Some(spring_bone) = &bundle.spring_bone {
        ensure_vrmc_spec_version("VRMC_springBone", &spring_bone.spec_version)?;
    }
    for constraint in &bundle.node_constraints {
        ensure_vrmc_spec_version("VRMC_node_constraint", &constraint.constraint.spec_version)?;
    }
    for mtoon in bundle.mtoon_materials.values() {
        ensure_vrmc_spec_version("VRMC_materials_mtoon", &mtoon.spec_version)?;
    }
    Ok(())
}

fn ensure_vrmc_spec_version(extension: &'static str, spec_version: &str) -> Result<(), VrmIoError> {
    if matches!(spec_version, "1.0" | "1.0-beta") {
        Ok(())
    } else {
        Err(VrmIoError::UnsupportedExtensionSpecVersion {
            extension: extension.to_owned(),
            spec_version: spec_version.to_owned(),
        })
    }
}

#[derive(Debug, Error)]
pub enum VrmIoError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Gltf(#[from] gltf::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error("invalid extension {extension}: {message}")]
    InvalidExtension { extension: String, message: String },
    #[error("unsupported {extension} specVersion: {spec_version}")]
    UnsupportedExtensionSpecVersion {
        extension: String,
        spec_version: String,
    },
    #[error("invalid animation channel: {message}")]
    InvalidAnimationChannel { message: String },
}

trait ImageFormatExt {
    fn to_mime_type(&self) -> Option<&'static str>;
}

impl ImageFormatExt for gltf::image::Format {
    fn to_mime_type(&self) -> Option<&'static str> {
        match self {
            gltf::image::Format::R8 => None,
            gltf::image::Format::R8G8 => None,
            gltf::image::Format::R8G8B8 => None,
            gltf::image::Format::R8G8B8A8 => None,
            gltf::image::Format::R16 => None,
            gltf::image::Format::R16G16 => None,
            gltf::image::Format::R16G16B16 => None,
            gltf::image::Format::R16G16B16A16 => None,
            gltf::image::Format::R32G32B32FLOAT => None,
            gltf::image::Format::R32G32B32A32FLOAT => None,
        }
    }
}

#[allow(dead_code)]
fn _preserve_indexmap_dependency(_: IndexMap<String, Value>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{env, fs, path::PathBuf};
    use vrm_core::{
        ExpressionName, Feature, FirstPersonAnnotation, HumanBoneName, LookAtKind,
        MtoonRenderQueue, OutlineWidthMode, VrmKind,
    };

    #[test]
    fn loads_generated_vrm1_gltf_without_repo_fixture_asset() {
        let bytes = generated_vrm1_gltf().to_string().into_bytes();
        let loaded = load_vrm_from_slice(&bytes).unwrap();
        let document = loaded.model().document();

        assert!(loaded.scene().node(0).is_some());
        assert_eq!(document.kind, VrmKind::Vrm1);
        assert_eq!(document.meta.name, "Generated Test Avatar");
        assert!(document.humanoid.bones.contains_key(&HumanBoneName::Hips));
        assert!(matches!(
            document.look_at,
            Feature::Present(ref look_at) if look_at.kind == LookAtKind::Expression
        ));
        assert!(matches!(
            document.materials.first().map(|material| &material.mtoon),
            Some(Feature::Present(mtoon)) if mtoon.outline_width_mode == OutlineWidthMode::WorldCoordinates
        ));
        assert!(matches!(
            document
                .materials
                .first()
                .map(|material| material.hdr_emissive_multiplier.as_ref()),
            Some(Some(multiplier)) if multiplier.emissive_intensity() == 2.5
        ));
        let (emissive_strength, _) = document.materials[0].effective_emissive_strength();
        assert_eq!(emissive_strength.0, 5.0);
        assert_eq!(document.node_constraints.len(), 1);
        assert!(document.spring_bone.is_present());
    }

    #[test]
    fn generated_sample_reports_invalid_node_references() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VRMC_vrm"]["humanoid"]["humanBones"]["hips"]["node"] = json!(999);
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("node(999)"));
    }

    #[test]
    fn generated_sample_reports_invalid_node_constraint_extension() {
        let mut sample = generated_vrm1_gltf();
        sample["nodes"][14]["extensions"]["VRMC_node_constraint"]["constraint"]["rotation"]["source"] =
            json!("bad");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidExtension { extension, .. }
                if extension == "VRMC_node_constraint"
        ));
    }

    #[test]
    fn generated_sample_reports_invalid_mtoon_extension() {
        let mut sample = generated_vrm1_gltf();
        sample["materials"][0]["extensions"]["VRMC_materials_mtoon"]["specVersion"] = json!(1);
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidExtension { extension, .. }
                if extension == "VRMC_materials_mtoon"
        ));
    }

    #[test]
    fn generated_sample_rejects_unsupported_spring_bone_spec_version() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VRMC_springBone"]["specVersion"] = json!("2.0");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::UnsupportedExtensionSpecVersion {
                extension,
                spec_version
            } if extension == "VRMC_springBone" && spec_version == "2.0"
        ));
    }

    #[test]
    fn generated_sample_rejects_unsupported_node_constraint_spec_version() {
        let mut sample = generated_vrm1_gltf();
        sample["nodes"][14]["extensions"]["VRMC_node_constraint"]["specVersion"] = json!("2.0");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::UnsupportedExtensionSpecVersion {
                extension,
                spec_version
            } if extension == "VRMC_node_constraint" && spec_version == "2.0"
        ));
    }

    #[test]
    fn generated_sample_rejects_unsupported_mtoon_spec_version() {
        let mut sample = generated_vrm1_gltf();
        sample["materials"][0]["extensions"]["VRMC_materials_mtoon"]["specVersion"] = json!("2.0");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::UnsupportedExtensionSpecVersion {
                extension,
                spec_version
            } if extension == "VRMC_materials_mtoon" && spec_version == "2.0"
        ));
    }

    #[test]
    fn generated_sample_accepts_beta_secondary_extension_spec_versions() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VRMC_springBone"]["specVersion"] = json!("1.0-beta");
        sample["nodes"][14]["extensions"]["VRMC_node_constraint"]["specVersion"] =
            json!("1.0-beta");
        sample["materials"][0]["extensions"]["VRMC_materials_mtoon"]["specVersion"] =
            json!("1.0-beta");
        let bytes = sample.to_string().into_bytes();

        let loaded = load_vrm_from_slice(&bytes).unwrap();

        assert!(loaded.model().document().spring_bone.is_present());
        assert_eq!(loaded.model().document().node_constraints.len(), 1);
        assert!(loaded.model().document().materials[0].mtoon.is_present());
    }

    #[test]
    fn generated_sample_reports_invalid_hdr_emissive_extension() {
        let mut sample = generated_vrm1_gltf();
        sample["materials"][0]["extensions"]["VRMC_materials_hdr_emissiveMultiplier"]["emissiveMultiplier"] =
            json!("bright");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidExtension { extension, .. }
                if extension == "VRMC_materials_hdr_emissiveMultiplier"
        ));
    }

    #[test]
    fn generated_sample_reports_invalid_khr_emissive_extension() {
        let mut sample = generated_vrm1_gltf();
        sample["materials"][0]["extensions"]["KHR_materials_emissive_strength"]["emissiveStrength"] =
            json!("bright");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidExtension { extension, .. }
                if extension == "KHR_materials_emissive_strength"
        ));
    }

    #[test]
    fn vrma_node_map_extracts_humanoid_expression_and_look_at_nodes() {
        let vrma = vrm_protocol::vrma::VrmcVrmAnimation {
            spec_version: "1.0".to_owned(),
            humanoid: Some(vrm_protocol::vrma::Humanoid {
                human_bones: [("hips".to_owned(), json!({ "node": 1 }))]
                    .into_iter()
                    .collect(),
            }),
            expressions: Some(vrm_protocol::vrma::Expressions {
                preset: Some(
                    [("blink".to_owned(), json!({ "node": 2 }))]
                        .into_iter()
                        .collect(),
                ),
                custom: Some(
                    [("custom".to_owned(), json!({ "node": 3 }))]
                        .into_iter()
                        .collect(),
                ),
            }),
            look_at: Some(vrm_protocol::vrma::LookAt { node: 4 }),
            extensions: None,
            extras: None,
        };

        let map = VrmaNodeMap::from_extension(&vrma);

        assert_eq!(map.humanoid.get(&1), Some(&HumanBoneName::Hips));
        assert_eq!(
            map.expressions.get(&2),
            Some(&VrmaExpressionTarget::Preset(ExpressionName::Blink))
        );
        assert_eq!(
            map.expressions.get(&3),
            Some(&VrmaExpressionTarget::Custom("custom".to_owned()))
        );
        assert_eq!(map.look_at, Some(4));
    }

    #[test]
    fn vrma_extension_warnings_follow_three_vrm_fallback_policy() {
        let mut missing = ExtensionMap::new();
        missing.insert(
            "VRMC_vrm_animation".to_owned(),
            json!({ "humanoid": { "humanBones": {} } }),
        );
        assert_eq!(
            vrma_extension_warnings(&missing),
            vec![VrmIoWarning::MissingSpecVersion {
                extension: "VRMC_vrm_animation".to_owned(),
                assumed: "1.0".to_owned(),
            }]
        );

        let mut draft = ExtensionMap::new();
        draft.insert(
            "VRMC_vrm_animation".to_owned(),
            json!({ "specVersion": "1.0-draft" }),
        );
        assert!(matches!(
            vrma_extension_warnings(&draft).as_slice(),
            [VrmIoWarning::DraftSpecVersion { .. }]
        ));

        let mut unknown = ExtensionMap::new();
        unknown.insert(
            "VRMC_vrm_animation".to_owned(),
            json!({ "specVersion": "2.0" }),
        );
        assert!(matches!(
            vrma_extension_warnings(&unknown).as_slice(),
            [VrmIoWarning::UnknownSpecVersion { version, .. }] if version == "2.0"
        ));
    }

    #[test]
    fn node_rest_graph_tracks_parent_and_world_matrices() {
        let sample = generated_transform_hierarchy_gltf();
        let (document, _, _) = gltf::import_slice(sample.to_string().as_bytes()).unwrap();
        let graph = NodeRestGraph::from_document(&document);

        assert_eq!(graph.parents[1], Some(0));
        assert_eq!(graph.children[0], vec![1]);
        assert!(
            graph.local_transforms[1]
                .translation
                .abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 0.0001)
        );
        assert!(
            graph.world_transforms[1]
                .translation
                .abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 0.0001)
        );
        assert!(
            graph.world_matrices[1]
                .transform_point3(Vec3::ZERO)
                .abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 0.0001)
        );
    }

    #[test]
    fn vrma_rest_pose_captures_hips_parent_and_position() {
        let sample = generated_transform_hierarchy_gltf();
        let (document, _, _) = gltf::import_slice(sample.to_string().as_bytes()).unwrap();
        let node_map = VrmaNodeMap {
            humanoid: [(1, HumanBoneName::Hips)].into_iter().collect(),
            ..VrmaNodeMap::default()
        };

        let rest_pose = VrmaRestPose::from_document(&document, &node_map);

        assert!(
            rest_pose
                .hips_world_position
                .abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 0.0001)
        );
        assert!(
            rest_pose
                .hips_parent_world_matrix
                .transform_point3(Vec3::X)
                .abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 0.0001)
        );
    }

    #[test]
    fn human_bone_parent_handles_humanoid_chain_and_custom_bones() {
        assert_eq!(
            human_bone_parent(&HumanBoneName::Head),
            Some(HumanBoneName::Neck)
        );
        assert_eq!(
            human_bone_parent(&HumanBoneName::LeftIndexDistal),
            Some(HumanBoneName::LeftIndexIntermediate)
        );
        assert_eq!(
            human_bone_parent(&HumanBoneName::LeftThumbProximal),
            Some(HumanBoneName::LeftThumbMetacarpal)
        );
        assert_eq!(
            human_bone_parent(&HumanBoneName::Custom("x".to_owned())),
            None
        );
    }

    #[test]
    fn node_from_value_rejects_missing_or_overflowing_nodes() {
        assert_eq!(node_from_value(&json!({ "node": 7 })), Some(7));
        assert_eq!(node_from_value(&json!({ "notNode": 7 })), None);
        assert_eq!(node_from_value(&json!({ "node": "7" })), None);
    }

    #[test]
    fn supported_fixture_filter_accepts_only_known_extensions() {
        assert!(is_supported_fixture(std::path::Path::new("avatar.vrm")));
        assert!(is_supported_fixture(std::path::Path::new("clip.VRMA")));
        assert!(!is_supported_fixture(std::path::Path::new("texture.png")));
        assert!(!is_supported_fixture(std::path::Path::new("README")));
    }

    #[test]
    fn supported_fixture_discovery_recurses_into_subdirectories() {
        let root =
            std::env::temp_dir().join(format!("vrm-rs-fixture-discovery-{}", std::process::id()));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("top.vrm"), b"").unwrap();
        fs::write(nested.join("clip.vrma"), b"").unwrap();
        fs::write(nested.join("note.txt"), b"").unwrap();

        let mut fixtures = supported_fixtures_under(&root)
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        fixtures.sort();

        assert_eq!(fixtures, vec!["clip.vrma", "top.vrm"]);
        fs::remove_dir_all(root).unwrap();
    }

    fn generated_vrm1_gltf() -> Value {
        let required_bones = [
            ("hips", 0),
            ("spine", 1),
            ("head", 2),
            ("leftUpperLeg", 3),
            ("leftLowerLeg", 4),
            ("leftFoot", 5),
            ("rightUpperLeg", 6),
            ("rightLowerLeg", 7),
            ("rightFoot", 8),
            ("leftUpperArm", 9),
            ("leftLowerArm", 10),
            ("leftHand", 11),
            ("rightUpperArm", 12),
            ("rightLowerArm", 13),
            ("rightHand", 14),
        ];
        let human_bones = required_bones
            .into_iter()
            .map(|(name, node)| (name.to_owned(), json!({ "node": node })))
            .collect::<serde_json::Map<_, _>>();

        let mut nodes = (0..15)
            .map(|index| json!({ "name": format!("node_{index}") }))
            .collect::<Vec<_>>();
        nodes[14]["extensions"] = json!({
            "VRMC_node_constraint": {
                "specVersion": "1.0",
                "constraint": {
                    "rotation": { "source": 13, "weight": 0.75 }
                }
            }
        });

        json!({
            "asset": { "version": "2.0", "generator": "vrm-rs generated test data" },
            "extensionsUsed": [
                "VRMC_vrm",
                "VRMC_springBone",
                "VRMC_node_constraint",
                "VRMC_materials_mtoon",
                "VRMC_materials_hdr_emissiveMultiplier",
                "KHR_materials_emissive_strength"
            ],
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": nodes,
            "materials": [{
                "name": "mtoon",
                "extensions": {
                    "VRMC_materials_mtoon": {
                        "specVersion": "1.0",
                        "transparentWithZWrite": true,
                        "renderQueueOffsetNumber": 2,
                        "shadeColorFactor": [0.8, 0.7, 0.6],
                        "outlineWidthMode": "worldCoordinates",
                        "outlineWidthFactor": 0.01,
                        "outlineColorFactor": [0.1, 0.1, 0.1]
                    },
                    "VRMC_materials_hdr_emissiveMultiplier": {
                        "emissiveMultiplier": 2.5
                    },
                    "KHR_materials_emissive_strength": {
                        "emissiveStrength": 5.0
                    }
                }
            }],
            "extensions": {
                "VRMC_vrm": {
                    "specVersion": "1.0",
                    "meta": {
                        "name": "Generated Test Avatar",
                        "authors": ["vrm-rs"]
                    },
                    "humanoid": { "humanBones": human_bones },
                    "firstPerson": {
                        "meshAnnotations": [{ "node": 0, "type": "auto" }]
                    },
                    "lookAt": {
                        "type": "expression",
                        "offsetFromHeadBone": [0.0, 0.06, 0.0],
                        "rangeMapHorizontalInner": {
                            "inputMaxValue": 45.0,
                            "outputScale": 10.0
                        }
                    },
                    "expressions": {
                        "preset": {
                            "blink": {
                                "morphTargetBinds": [{
                                    "node": 2,
                                    "index": 0,
                                    "weight": 100.0
                                }],
                                "overrideLookAt": "block"
                            }
                        }
                    }
                },
                "VRMC_springBone": {
                    "specVersion": "1.0",
                    "colliders": [{
                        "node": 2,
                        "shape": {
                            "sphere": {
                                "offset": [0.0, 0.0, 0.0],
                                "radius": 0.1
                            }
                        }
                    }],
                    "colliderGroups": [{
                        "name": "head",
                        "colliders": [0]
                    }],
                    "springs": [{
                        "name": "hair",
                        "joints": [{
                            "node": 2,
                            "hitRadius": 0.02,
                            "stiffness": 0.8,
                            "gravityPower": 0.1,
                            "gravityDir": [0.0, -1.0, 0.0],
                            "dragForce": 0.4
                        }],
                        "colliderGroups": [0]
                    }]
                }
            }
        })
    }

    fn generated_transform_hierarchy_gltf() -> Value {
        json!({
            "asset": { "version": "2.0", "generator": "vrm-rs transform graph test" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [
                { "translation": [1.0, 0.0, 0.0], "children": [1] },
                { "translation": [0.0, 2.0, 0.0] }
            ]
        })
    }

    #[test]
    #[ignore = "requires local external fixtures; set VRM_RS_FIXTURE_DIR"]
    fn loads_external_fixture_directory() {
        let fixture_dir = env::var_os("VRM_RS_FIXTURE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".external-fixtures/official"));
        let mut loaded = Vec::new();
        for path in supported_fixtures_under(&fixture_dir) {
            if !is_supported_fixture(&path) {
                continue;
            }
            let result = load_vrm_from_path(&path);
            assert!(
                result.is_ok(),
                "failed to load external fixture {}: {:?}",
                path.display(),
                result.err()
            );
            let result = result.unwrap();
            assert_external_fixture_semantics(&path, &result);
            loaded.push(path);
        }

        assert!(
            !loaded.is_empty(),
            "no .vrm/.vrma/.glb/.gltf fixtures found in {}",
            fixture_dir.display()
        );
    }

    fn assert_external_fixture_semantics(path: &std::path::Path, loaded: &LoadedVrm) {
        let document = loaded.model().document();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();

        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("vrma"))
        {
            assert!(
                !document.animations.is_empty(),
                "VRMA fixture did not produce animations: {}",
                path.display()
            );
            let animation = &document.animations[0];
            assert!(
                animation.duration > 0.0,
                "VRMA fixture has zero duration: {}",
                path.display()
            );
            assert!(
                animation.hips_translation.is_some()
                    || !animation.humanoid_rotation_tracks.is_empty()
                    || !animation.preset_expression_tracks.is_empty()
                    || !animation.custom_expression_tracks.is_empty()
                    || animation.look_at_track.is_some(),
                "VRMA fixture has no extracted tracks: {}",
                path.display()
            );
            if file_name.eq_ignore_ascii_case("test.vrma") {
                let track_classes = [
                    !animation.humanoid_rotation_tracks.is_empty(),
                    animation.hips_translation.is_some(),
                    !animation.preset_expression_tracks.is_empty()
                        || !animation.custom_expression_tracks.is_empty(),
                    animation.look_at_track.is_some(),
                ]
                .into_iter()
                .filter(|present| *present)
                .count();
                assert!(
                    !animation.humanoid_rotation_tracks.is_empty(),
                    "test.vrma should expose humanoid rotation tracks"
                );
                assert!(
                    track_classes >= 2,
                    "test.vrma should expose multiple VRMA track classes"
                );
            }
            return;
        }

        assert!(
            !document.meta.name.is_empty(),
            "VRM fixture has empty meta name: {}",
            path.display()
        );
        assert!(
            !document.humanoid.bones.is_empty(),
            "VRM fixture has no humanoid bones: {}",
            path.display()
        );
        if file_name.eq_ignore_ascii_case("Seed-san.vrm") {
            assert!(
                !document.materials.is_empty(),
                "Seed-san should expose material data"
            );
            assert!(
                document.spring_bone.is_present(),
                "Seed-san should expose spring bone data"
            );
        }
        if file_name.eq_ignore_ascii_case("VRM1_Constraint_Twist_Sample.vrm") {
            assert!(
                !document.node_constraints.is_empty(),
                "constraint sample should expose node constraints"
            );
            assert!(
                document.spring_bone.is_present(),
                "constraint sample should expose spring bone data"
            );
        }
        if file_name.eq_ignore_ascii_case("VRMC_materials_mtoon_UV_Animation_Test.vrm") {
            let animated_mtoon = document
                .materials
                .iter()
                .filter_map(|material| material.mtoon.as_ref())
                .find(|mtoon| {
                    mtoon.uv_animation.scroll_x_speed != 0.0
                        || mtoon.uv_animation.scroll_y_speed != 0.0
                        || mtoon.uv_animation.rotation_speed != 0.0
                        || mtoon.textures.uv_animation_mask_texture.is_some()
                });
            assert!(
                animated_mtoon.is_some(),
                "MToon UV animation sample should expose UV animation parameters"
            );
        }
        if file_name.eq_ignore_ascii_case("VRMC_vrm_expressions_isBinary_Overrides.vrm")
            || file_name.eq_ignore_ascii_case("VRMC_vrm_expressions_isBinary_Overridden.vrm")
        {
            let expressions = document.expressions.as_ref().unwrap_or_else(|| {
                panic!(
                    "expression override sample should expose expressions: {}",
                    path.display()
                )
            });
            let preset_count = expressions.preset.len();
            let has_binary = expressions
                .preset
                .values()
                .chain(expressions.custom.values())
                .any(|expression| expression.is_binary);
            let has_override = expressions
                .preset
                .values()
                .chain(expressions.custom.values())
                .any(|expression| {
                    expression.override_blink != vrm_core::OverrideMode::None
                        || expression.override_look_at != vrm_core::OverrideMode::None
                        || expression.override_mouth != vrm_core::OverrideMode::None
                });
            assert!(
                preset_count > 0,
                "expression override sample should expose preset expressions"
            );
            assert!(
                has_binary || has_override,
                "expression override sample should expose binary or override metadata"
            );
        }
        if file_name.eq_ignore_ascii_case("VRM0_AliciaSolid.vrm")
            || file_name.eq_ignore_ascii_case("AliciaSolid_vrm-0.51.vrm")
        {
            assert_eq!(document.kind, vrm_core::VrmKind::Vrm0Compat);
            assert!(
                document.compatibility.vrm0.is_some(),
                "VRM0 fixture should expose compatibility metadata"
            );
            assert!(
                document.first_person.is_present() || document.expressions.is_present(),
                "VRM0 fixture should expose first-person or expression compatibility data"
            );
            assert_eq!(document.meta.name, "Alicia Solid");
            assert_eq!(document.humanoid.bones.len(), 55);
            assert!(
                document.humanoid.bones.contains_key(&HumanBoneName::Head)
                    && document
                        .humanoid
                        .bones
                        .contains_key(&HumanBoneName::LeftThumbMetacarpal)
                    && document
                        .humanoid
                        .bones
                        .contains_key(&HumanBoneName::LeftThumbProximal)
                    && document
                        .humanoid
                        .bones
                        .contains_key(&HumanBoneName::RightThumbMetacarpal)
                    && document
                        .humanoid
                        .bones
                        .contains_key(&HumanBoneName::RightThumbProximal),
                "Alicia VRM0 humanoid bones should include normalized head and thumb aliases"
            );

            let first_person = document.first_person.as_ref().unwrap_or_else(|| {
                panic!("Alicia VRM0 fixture should expose first-person annotations")
            });
            assert_eq!(first_person.mesh_annotations.len(), 12);
            assert!(
                first_person
                    .mesh_annotations
                    .iter()
                    .all(|annotation| annotation.kind == FirstPersonAnnotation::Auto),
                "Alicia VRM0 mesh annotations should preserve Auto flags"
            );

            let look_at = document
                .look_at
                .as_ref()
                .unwrap_or_else(|| panic!("Alicia VRM0 fixture should expose lookAt data"));
            assert_eq!(look_at.kind, LookAtKind::Bone);
            assert!((look_at.offset_from_head.y - 0.059999943).abs() < 0.000001);
            assert_eq!(look_at.horizontal_inner.input_max_value, 20.0);
            assert_eq!(look_at.horizontal_inner.output_scale, 5.0);

            let expressions = document
                .expressions
                .as_ref()
                .unwrap_or_else(|| panic!("Alicia VRM0 fixture should expose expressions"));
            assert_eq!(expressions.preset.len(), 17);
            for preset in [
                ExpressionName::Aa,
                ExpressionName::Ih,
                ExpressionName::Ou,
                ExpressionName::Ee,
                ExpressionName::Oh,
                ExpressionName::Happy,
                ExpressionName::Sad,
                ExpressionName::Relaxed,
                ExpressionName::BlinkLeft,
                ExpressionName::BlinkRight,
            ] {
                assert!(
                    expressions.preset.contains_key(&preset),
                    "Alicia VRM0 preset {} should map to canonical expression",
                    preset.as_str()
                );
            }

            assert_eq!(document.materials.len(), 12);
            assert!(
                document
                    .materials
                    .iter()
                    .all(|material| material.mtoon.is_present()),
                "Alicia VRM0 materials should map legacy VRM/MToon properties"
            );
            let body_mtoon = document.materials[0].mtoon.as_ref().unwrap();
            assert_eq!(body_mtoon.render_queue, MtoonRenderQueue::Opaque);
            assert!(body_mtoon.outline_enabled());
            assert_eq!(body_mtoon.base_color_factor, [1.0, 1.0, 1.0, 1.0]);
            assert_eq!(body_mtoon.emissive_factor, [0.0, 0.0, 0.0]);
            assert_eq!(body_mtoon.cutoff_factor, 0.5);
            assert_eq!(body_mtoon.shade_color_factor, [1.0, 0.8666667, 0.84000003]);
            assert_eq!(body_mtoon.receive_shadow_rate_factor, 1.0);
            assert_eq!(body_mtoon.shading_grade_rate_factor, 1.0);
            assert_eq!(body_mtoon.shading_shift_factor, 0.0);
            assert_eq!(body_mtoon.shading_toony_factor, 0.9);
            assert_eq!(body_mtoon.light_color_attenuation_factor, 0.0);
            assert_eq!(body_mtoon.gi_equalization_factor, 0.1);
            assert_eq!(
                body_mtoon.outline_width_mode,
                OutlineWidthMode::WorldCoordinates
            );
            assert_eq!(body_mtoon.outline_width_factor, 0.05);
            assert_eq!(
                body_mtoon.outline_color_factor,
                [0.671, 0.55702585, 0.53478694]
            );
            assert_eq!(body_mtoon.outline_lighting_mix_factor, 1.0);
            assert_eq!(
                body_mtoon.textures.main_texture,
                Some(vrm_core::TextureRef(0))
            );
            assert_eq!(
                body_mtoon.textures.shade_multiply_texture,
                Some(vrm_core::TextureRef(0))
            );
            assert_eq!(
                body_mtoon.textures.matcap_texture,
                Some(vrm_core::TextureRef(1))
            );
            assert_eq!(body_mtoon.textures.normal_texture, None);
            assert_eq!(body_mtoon.textures.outline_width_multiply_texture, None);
            assert_eq!(body_mtoon.uv_animation.scroll_x_speed, 0.0);
            assert_eq!(body_mtoon.uv_animation.scroll_y_speed, 0.0);
            assert_eq!(body_mtoon.uv_animation.rotation_speed, 0.0);

            let spring_bone = document.spring_bone.as_ref().unwrap_or_else(|| {
                panic!("Alicia VRM0 fixture should expose secondary animation as spring bone")
            });
            assert_eq!(spring_bone.springs.len(), 3);
            assert_eq!(spring_bone.collider_groups.len(), 6);
            assert_eq!(
                spring_bone
                    .springs
                    .iter()
                    .map(|spring| spring.joints.len())
                    .sum::<usize>(),
                48
            );
            assert!(
                spring_bone
                    .springs
                    .iter()
                    .all(|spring| spring.center.is_none())
            );
            assert!(
                spring_bone
                    .springs
                    .iter()
                    .any(|spring| spring.joints.len() >= 5),
                "Alicia VRM0 spring groups should retain multi-joint chains"
            );
        }
    }

    fn is_supported_fixture(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "vrm" | "vrma" | "glb" | "gltf"
                )
            })
    }

    fn supported_fixtures_under(root: &std::path::Path) -> Vec<PathBuf> {
        let mut result = Vec::new();
        collect_supported_fixtures(root, &mut result);
        result
    }

    fn collect_supported_fixtures(path: &std::path::Path, result: &mut Vec<PathBuf>) {
        if path.is_file() {
            if is_supported_fixture(path) {
                result.push(path.to_owned());
            }
            return;
        }

        let entries = fs::read_dir(path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for entry in entries {
            collect_supported_fixtures(&entry.unwrap().path(), result);
        }
    }
}
