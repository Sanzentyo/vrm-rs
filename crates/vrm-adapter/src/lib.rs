//! Traits for connecting `vrm-rs` runtime output to external engines.

use glam::{Mat4, Quat, Vec3};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use vrm_core::{
    ColliderShape, ConstraintKind, EmissiveStrength, ExpressionBind, ExpressionName, Feature,
    FirstPersonAnnotation, HumanBoneName, MaterialRef, MtoonMaterial, MtoonPipelinePass,
    MtoonTextureSet, NodeConstraint, NodeRef, RawAbsolutePose, RawPose, Spring, SpringBoneSystem,
    TextureRef, Transform, VrmDocument,
};
use vrm_runtime::{
    AimConstraintInput, AppliedExpression, CenterSpringParticleState, CenterSpringRuntimeState,
    ConstraintRestState, DeltaTime, RuntimeEvents, SpringJointParityInput, SpringJointRestState,
    SpringJointSimulationInput, SpringParticleState, SpringRuntimeState, VrmAnimationFrame,
    collider_shape_in_simulation_space, solve_aim_constraint, solve_roll_constraint,
    solve_rotation_constraint, solve_spring_joint_rotation, step_spring_joint,
    step_spring_joint_parity,
};

pub trait SceneGraph {
    type Error;

    fn parent(&self, node: NodeRef) -> Result<Option<NodeRef>, Self::Error>;
    fn children(&self, node: NodeRef) -> Result<Vec<NodeRef>, Self::Error>;
}

pub trait TransformAccess {
    type Error;

    fn local_transform(&self, node: NodeRef) -> Result<Transform, Self::Error>;
    fn set_local_transform(
        &mut self,
        node: NodeRef,
        transform: Transform,
    ) -> Result<(), Self::Error>;
    fn set_local_rotation(&mut self, node: NodeRef, rotation: Quat) -> Result<(), Self::Error>;
    fn translate_local(&mut self, node: NodeRef, translation: Vec3) -> Result<(), Self::Error>;
}

pub trait WorldTransformAccess {
    type Error;

    fn world_transform(&self, node: NodeRef) -> Result<Transform, Self::Error>;
}

pub trait WorldTransformUpdate {
    type Error;

    fn update_world_transforms(&mut self) -> Result<(), Self::Error>;
}

pub trait ConstraintRestAccess {
    type Error;

    fn constraint_rest_state(
        &self,
        destination: NodeRef,
        source: NodeRef,
    ) -> Result<ConstraintRestState, Self::Error>;
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConstraintRestMap {
    states: HashMap<(NodeRef, NodeRef), ConstraintRestState>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringRestEntry {
    pub rest: SpringJointRestState,
    pub initial_center_state: CenterSpringParticleState,
    pub child: Option<NodeRef>,
    pub center: Option<NodeRef>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpringRestMap {
    states: HashMap<(usize, usize), SpringRestEntry>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HumanoidPoseRig {
    raw_rest: RawAbsolutePose,
    normalized_rest: vrm_core::NormalizedAbsolutePose,
    normalized_current: vrm_core::NormalizedAbsolutePose,
    parent_world_rotations: HashMap<HumanBoneName, Quat>,
    raw_rest_rotations: HashMap<HumanBoneName, Quat>,
    raw_nodes: HashMap<HumanBoneName, NodeRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HumanoidPoseSnapshot {
    pub raw: RawPose,
    pub normalized: vrm_core::NormalizedPose,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseTolerance {
    pub translation: f32,
    pub rotation_radians: f32,
}

impl Default for PoseTolerance {
    fn default() -> Self {
        Self {
            translation: 0.0001,
            rotation_radians: 0.0001,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PoseMismatch {
    pub bone: HumanBoneName,
    pub translation_delta: f32,
    pub rotation_delta: f32,
}

impl HumanoidPoseRig {
    pub fn capture<T, E>(target: &T, document: &VrmDocument) -> Result<Self, AdapterError<E>>
    where
        T: TransformAccess<Error = E> + WorldTransformAccess<Error = E> + SceneGraph<Error = E>,
    {
        let raw_nodes = document
            .humanoid
            .bones
            .iter()
            .map(|(name, bone)| (name.clone(), bone.node))
            .collect::<HashMap<_, _>>();
        let raw_rest = capture_raw_absolute_pose(target, document)?;
        let parent_world_rotations = document
            .humanoid
            .bones
            .iter()
            .map(|(name, bone)| {
                let parent_rotation = target
                    .parent(bone.node)
                    .map_err(AdapterError::Target)?
                    .map(|parent| {
                        target
                            .world_transform(parent)
                            .map(|transform| transform.rotation)
                            .map_err(AdapterError::Target)
                    })
                    .transpose()?
                    .unwrap_or(Quat::IDENTITY);
                Ok((name.clone(), parent_rotation))
            })
            .collect::<Result<HashMap<_, _>, AdapterError<E>>>()?;
        let raw_rest_rotations = raw_rest
            .bones
            .iter()
            .map(|(name, transform)| (name.clone(), transform.rotation))
            .collect::<HashMap<_, _>>();
        let normalized_rest = capture_normalized_absolute_pose(target, document)?;
        Ok(Self {
            normalized_current: normalized_rest.clone(),
            raw_rest,
            normalized_rest,
            parent_world_rotations,
            raw_rest_rotations,
            raw_nodes,
        })
    }

    pub fn raw_rest_pose(&self) -> &RawAbsolutePose {
        &self.raw_rest
    }

    pub fn normalized_rest_pose(&self) -> &vrm_core::NormalizedAbsolutePose {
        &self.normalized_rest
    }

    pub fn get_raw_absolute_pose<T, E>(
        &self,
        target: &T,
    ) -> Result<RawAbsolutePose, AdapterError<E>>
    where
        T: TransformAccess<Error = E>,
    {
        self.raw_nodes
            .iter()
            .map(|(name, node)| {
                target
                    .local_transform(*node)
                    .map(|transform| {
                        (
                            name.clone(),
                            vrm_core::PoseTransform {
                                translation: transform.translation,
                                rotation: transform.rotation,
                            },
                        )
                    })
                    .map_err(AdapterError::Target)
            })
            .collect::<Result<IndexMap<_, _>, _>>()
            .map(pose_from_iter)
    }

    pub fn get_raw_pose<T, E>(&self, target: &T) -> Result<RawPose, AdapterError<E>>
    where
        T: TransformAccess<Error = E>,
    {
        let absolute = self.get_raw_absolute_pose(target)?;
        Ok(relative_pose(&absolute, &self.raw_rest))
    }

    pub fn set_raw_pose<T, E>(&self, target: &mut T, pose: &RawPose) -> Result<(), AdapterError<E>>
    where
        T: TransformAccess<Error = E>,
    {
        let absolute = absolute_pose(pose, &self.raw_rest);
        self.set_raw_absolute_pose(target, &absolute)
    }

    pub fn reset_raw_pose<T, E>(&self, target: &mut T) -> Result<(), AdapterError<E>>
    where
        T: TransformAccess<Error = E>,
    {
        self.set_raw_absolute_pose(target, &self.raw_rest)
    }

    pub fn get_normalized_absolute_pose(&self) -> vrm_core::NormalizedAbsolutePose {
        self.normalized_current.clone()
    }

    pub fn get_normalized_pose(&self) -> vrm_core::NormalizedPose {
        relative_pose(&self.normalized_current, &self.normalized_rest)
    }

    pub fn get_normalized_pose_from_raw<T, E>(
        &self,
        target: &T,
    ) -> Result<vrm_core::NormalizedPose, AdapterError<E>>
    where
        T: TransformAccess<Error = E> + WorldTransformAccess<Error = E>,
    {
        Ok(relative_pose(
            &self.get_normalized_absolute_pose_from_raw(target)?,
            &self.normalized_rest,
        ))
    }

    pub fn get_normalized_absolute_pose_from_raw<T, E>(
        &self,
        target: &T,
    ) -> Result<vrm_core::NormalizedAbsolutePose, AdapterError<E>>
    where
        T: TransformAccess<Error = E> + WorldTransformAccess<Error = E>,
    {
        self.raw_nodes
            .iter()
            .filter_map(|(name, node)| {
                self.normalized_rest
                    .get(name)
                    .map(|rest| (name.clone(), *node, *rest))
            })
            .map(|(name, node, rest)| {
                let transform = target.local_transform(node).map_err(AdapterError::Target)?;
                let parent_world = self
                    .parent_world_rotations
                    .get(&name)
                    .copied()
                    .unwrap_or(Quat::IDENTITY);
                let raw_rest = self
                    .raw_rest_rotations
                    .get(&name)
                    .copied()
                    .unwrap_or(Quat::IDENTITY);
                let translation = if name == HumanBoneName::Hips {
                    target
                        .world_transform(node)
                        .map(|transform| transform.translation)
                        .map_err(AdapterError::Target)?
                } else {
                    rest.translation
                };
                Ok((
                    name,
                    vrm_core::PoseTransform {
                        translation,
                        rotation: parent_world
                            * transform.rotation
                            * raw_rest.inverse()
                            * parent_world.inverse(),
                    },
                ))
            })
            .collect::<Result<IndexMap<_, _>, AdapterError<E>>>()
            .map(pose_from_iter)
    }

    pub fn set_normalized_pose(&mut self, pose: &vrm_core::NormalizedPose) {
        self.normalized_current = absolute_pose(pose, &self.normalized_rest);
    }

    pub fn reset_normalized_pose(&mut self) {
        self.normalized_current = self.normalized_rest.clone();
    }

    pub fn snapshot<T, E>(&self, target: &T) -> Result<HumanoidPoseSnapshot, AdapterError<E>>
    where
        T: TransformAccess<Error = E>,
    {
        Ok(HumanoidPoseSnapshot {
            raw: self.get_raw_pose(target)?,
            normalized: self.get_normalized_pose(),
        })
    }

    pub fn apply_normalized_to_raw<T, E>(&self, target: &mut T) -> Result<(), AdapterError<E>>
    where
        T: TransformAccess<Error = E> + WorldTransformAccess<Error = E> + SceneGraph<Error = E>,
    {
        for (name, node) in &self.raw_nodes {
            let Some(normalized) = self.normalized_current.get(name) else {
                continue;
            };
            let parent_world = self
                .parent_world_rotations
                .get(name)
                .copied()
                .unwrap_or(Quat::IDENTITY);
            let raw_rest = self
                .raw_rest_rotations
                .get(name)
                .copied()
                .unwrap_or(Quat::IDENTITY);
            let mut transform = target
                .local_transform(*node)
                .map_err(AdapterError::Target)?;
            transform.rotation =
                parent_world.inverse() * normalized.rotation * parent_world * raw_rest;
            if *name == HumanBoneName::Hips {
                let parent_world_transform = target
                    .parent(*node)
                    .map_err(AdapterError::Target)?
                    .map(|parent| target.world_transform(parent).map_err(AdapterError::Target))
                    .transpose()?
                    .unwrap_or_default();
                transform.translation = transform_matrix(parent_world_transform)
                    .inverse()
                    .transform_point3(normalized.translation);
            }
            target
                .set_local_transform(*node, transform)
                .map_err(AdapterError::Target)?;
        }
        Ok(())
    }

    fn set_raw_absolute_pose<T, E>(
        &self,
        target: &mut T,
        pose: &RawAbsolutePose,
    ) -> Result<(), AdapterError<E>>
    where
        T: TransformAccess<Error = E>,
    {
        for (name, transform) in &pose.bones {
            let Some(node) = self.raw_nodes.get(name).copied() else {
                continue;
            };
            target
                .set_local_transform(
                    node,
                    Transform {
                        translation: transform.translation,
                        rotation: transform.rotation,
                        scale: target
                            .local_transform(node)
                            .map_err(AdapterError::Target)?
                            .scale,
                    },
                )
                .map_err(AdapterError::Target)?;
        }
        Ok(())
    }
}

impl HumanoidPoseSnapshot {
    pub fn mismatches(
        &self,
        expected: &HumanoidPoseSnapshot,
        tolerance: PoseTolerance,
    ) -> Vec<PoseMismatch> {
        self.raw
            .bones
            .iter()
            .filter_map(|(bone, actual)| {
                expected
                    .raw
                    .get(bone)
                    .and_then(|expected| pose_mismatch(bone, actual, expected, tolerance))
            })
            .chain(self.normalized.bones.iter().filter_map(|(bone, actual)| {
                expected
                    .normalized
                    .get(bone)
                    .and_then(|expected| pose_mismatch(bone, actual, expected, tolerance))
            }))
            .collect()
    }
}

fn pose_mismatch(
    bone: &HumanBoneName,
    actual: &vrm_core::PoseTransform,
    expected: &vrm_core::PoseTransform,
    tolerance: PoseTolerance,
) -> Option<PoseMismatch> {
    let translation_delta = actual.translation.distance(expected.translation);
    let rotation_delta = actual.rotation.angle_between(expected.rotation).abs();
    (translation_delta > tolerance.translation || rotation_delta > tolerance.rotation_radians).then(
        || PoseMismatch {
            bone: bone.clone(),
            translation_delta,
            rotation_delta,
        },
    )
}

fn capture_raw_absolute_pose<T, E>(
    target: &T,
    document: &VrmDocument,
) -> Result<RawAbsolutePose, AdapterError<E>>
where
    T: TransformAccess<Error = E>,
{
    document
        .humanoid
        .bones
        .iter()
        .map(|(name, bone)| {
            target
                .local_transform(bone.node)
                .map(|transform| {
                    (
                        name.clone(),
                        vrm_core::PoseTransform {
                            translation: transform.translation,
                            rotation: transform.rotation,
                        },
                    )
                })
                .map_err(AdapterError::Target)
        })
        .collect::<Result<IndexMap<_, _>, _>>()
        .map(pose_from_iter)
}

fn capture_normalized_absolute_pose<T, E>(
    target: &T,
    document: &VrmDocument,
) -> Result<vrm_core::NormalizedAbsolutePose, AdapterError<E>>
where
    T: WorldTransformAccess<Error = E>,
{
    let world_positions = document
        .humanoid
        .bones
        .iter()
        .map(|(name, bone)| {
            target
                .world_transform(bone.node)
                .map(|transform| (name.clone(), transform.translation))
                .map_err(AdapterError::Target)
        })
        .collect::<Result<HashMap<_, _>, _>>()?;
    let entries = document
        .humanoid
        .bones
        .iter()
        .filter_map(|(name, bone)| {
            let world_position = world_positions.get(name)?;
            let parent_position = nearest_humanoid_parent_position(name, &world_positions);
            Some((
                name.clone(),
                vrm_core::PoseTransform {
                    translation: parent_position
                        .map_or(*world_position, |parent| *world_position - parent),
                    rotation: bone.rest.rotation,
                },
            ))
        })
        .collect::<IndexMap<_, _>>();
    Ok(pose_from_iter(entries))
}

fn nearest_humanoid_parent_position(
    bone: &HumanBoneName,
    positions: &HashMap<HumanBoneName, Vec3>,
) -> Option<Vec3> {
    let mut parent = bone.parent();
    while let Some(parent_name) = parent {
        if let Some(position) = positions.get(&parent_name).copied() {
            return Some(position);
        }
        parent = parent_name.parent();
    }
    None
}

fn pose_from_iter<Space, Basis>(
    bones: impl IntoIterator<Item = (HumanBoneName, vrm_core::PoseTransform)>,
) -> vrm_core::HumanoidPose<Space, Basis> {
    let mut pose = vrm_core::HumanoidPose::new();
    for (name, transform) in bones {
        pose.insert(name, transform);
    }
    pose
}

fn relative_pose<Space>(
    absolute: &vrm_core::HumanoidPose<Space, vrm_core::AbsolutePoseBasis>,
    rest: &vrm_core::HumanoidPose<Space, vrm_core::AbsolutePoseBasis>,
) -> vrm_core::HumanoidPose<Space, vrm_core::RestRelativePoseBasis> {
    pose_from_iter(absolute.bones.iter().filter_map(|(name, current)| {
        rest.get(name).map(|rest| {
            (
                name.clone(),
                vrm_core::PoseTransform {
                    translation: current.translation - rest.translation,
                    rotation: current.rotation * rest.rotation.inverse(),
                },
            )
        })
    }))
}

fn absolute_pose<Space>(
    relative: &vrm_core::HumanoidPose<Space, vrm_core::RestRelativePoseBasis>,
    rest: &vrm_core::HumanoidPose<Space, vrm_core::AbsolutePoseBasis>,
) -> vrm_core::HumanoidPose<Space, vrm_core::AbsolutePoseBasis> {
    pose_from_iter(relative.bones.iter().filter_map(|(name, current)| {
        rest.get(name).map(|rest| {
            (
                name.clone(),
                vrm_core::PoseTransform {
                    translation: current.translation + rest.translation,
                    rotation: current.rotation * rest.rotation,
                },
            )
        })
    }))
}

fn transform_matrix(transform: Transform) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    )
}

fn first_child<T, E>(target: &T, node: NodeRef) -> Result<Option<NodeRef>, AdapterError<E>>
where
    T: SceneGraph<Error = E>,
{
    Ok(target
        .children(node)
        .map_err(AdapterError::Target)?
        .first()
        .copied())
}

fn initial_local_child_position<T, E>(
    target: &T,
    joint_local: Transform,
    child: Option<NodeRef>,
) -> Result<Vec3, AdapterError<E>>
where
    T: TransformAccess<Error = E>,
{
    child
        .map(|child| {
            target
                .local_transform(child)
                .map_err(AdapterError::Target)
                .map(|child_local| child_local.translation)
        })
        .transpose()
        .map(|local| {
            local.unwrap_or_else(|| {
                SpringJointRestState::vrm0_tail_fallback(joint_local).initial_local_child_position
            })
        })
}

fn center_space_tail(
    joint_world: Transform,
    center_world: Option<Transform>,
    rest: SpringJointRestState,
) -> Vec3 {
    let tail_world =
        transform_matrix(joint_world).transform_point3(rest.initial_local_child_position);
    center_world
        .map(transform_matrix)
        .unwrap_or(Mat4::IDENTITY)
        .inverse()
        .transform_point3(tail_world)
}

impl ConstraintRestMap {
    pub fn capture<T, E>(
        target: &T,
        constraints: &[NodeConstraint],
    ) -> Result<Self, AdapterError<E>>
    where
        T: TransformAccess<Error = E>,
    {
        let states = constraints
            .iter()
            .map(|constraint| {
                let destination = target
                    .local_transform(constraint.destination)
                    .map_err(AdapterError::Target)?;
                let source = target
                    .local_transform(constraint.source)
                    .map_err(AdapterError::Target)?;
                Ok((
                    (constraint.destination, constraint.source),
                    ConstraintRestState {
                        destination_rest_rotation: destination.rotation,
                        source_rest_rotation: source.rotation,
                    },
                ))
            })
            .collect::<Result<HashMap<_, _>, AdapterError<E>>>()?;
        Ok(Self { states })
    }

    pub fn get(&self, destination: NodeRef, source: NodeRef) -> Option<ConstraintRestState> {
        self.states.get(&(destination, source)).copied()
    }
}

impl SpringRestMap {
    pub fn capture<T, E>(target: &T, system: &SpringBoneSystem) -> Result<Self, AdapterError<E>>
    where
        T: TransformAccess<Error = E> + WorldTransformAccess<Error = E> + SceneGraph<Error = E>,
    {
        let states = system
            .springs
            .iter()
            .enumerate()
            .flat_map(|(spring_index, spring)| {
                spring
                    .joints
                    .iter()
                    .enumerate()
                    .map(move |(joint_index, joint)| (spring_index, spring, joint_index, joint))
            })
            .map(|(spring_index, spring, joint_index, joint)| {
                let joint_local = target
                    .local_transform(joint.node)
                    .map_err(AdapterError::Target)?;
                let joint_world = target
                    .world_transform(joint.node)
                    .map_err(AdapterError::Target)?;
                let child = if let Some(next_joint) = spring.joints.get(joint_index + 1) {
                    Some(next_joint.node)
                } else {
                    first_child(target, joint.node)?
                };
                let initial_local_child_position =
                    initial_local_child_position(target, joint_local, child)?;
                let rest = SpringJointRestState::from_local_child(
                    joint_local,
                    initial_local_child_position,
                );
                let center_world = spring
                    .center
                    .map(|center| target.world_transform(center).map_err(AdapterError::Target))
                    .transpose()?;
                let center_tail = center_space_tail(joint_world, center_world, rest);
                Ok((
                    (spring_index, joint_index),
                    SpringRestEntry {
                        rest,
                        initial_center_state: CenterSpringParticleState::at_rest(center_tail),
                        child,
                        center: spring.center,
                    },
                ))
            })
            .collect::<Result<HashMap<_, _>, AdapterError<E>>>()?;
        Ok(Self { states })
    }

    pub fn get(&self, spring_index: usize, joint_index: usize) -> Option<SpringRestEntry> {
        self.states.get(&(spring_index, joint_index)).copied()
    }

    pub fn runtime_state(&self, system: &SpringBoneSystem) -> CenterSpringRuntimeState {
        CenterSpringRuntimeState::from_system(system, |spring_index, joint_index, _| {
            self.get(spring_index, joint_index)
                .map(|entry| entry.initial_center_state)
                .unwrap_or_default()
        })
    }
}

pub trait MorphTargetAccess {
    type Error;

    fn set_morph_weight(
        &mut self,
        node: NodeRef,
        morph_index: usize,
        weight: f32,
    ) -> Result<(), Self::Error>;
}

pub trait MaterialAccess {
    type Error;

    fn set_material_color(
        &mut self,
        material: MaterialRef,
        property: &str,
        value: &[f32],
    ) -> Result<(), Self::Error>;

    fn set_texture_transform(
        &mut self,
        material: MaterialRef,
        scale: Option<[f32; 2]>,
        offset: Option<[f32; 2]>,
    ) -> Result<(), Self::Error>;

    fn set_emissive_intensity(
        &mut self,
        material: MaterialRef,
        intensity: f32,
    ) -> Result<(), Self::Error>;
}

pub trait MtoonPipelineAccess {
    type Error;

    fn set_mtoon_pipeline_passes(
        &mut self,
        material: MaterialRef,
        passes: &[MtoonPipelinePass],
    ) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct MtoonMaterialDescriptor {
    pub material: MaterialRef,
    pub name: Option<String>,
    pub pass: MtoonPipelinePass,
    pub textures: MtoonTextureSet,
    pub base_color_factor: [f32; 4],
    pub emissive_factor: [f32; 3],
    pub cutoff_factor: f32,
    pub shade_color_factor: [f32; 3],
    pub receive_shadow_rate_factor: f32,
    pub shading_grade_rate_factor: f32,
    pub shading_shift_factor: f32,
    pub shading_toony_factor: f32,
    pub light_color_attenuation_factor: f32,
    pub gi_equalization_factor: f32,
    pub matcap_factor: [f32; 3],
    pub parametric_rim_color_factor: [f32; 3],
    pub rim_lighting_mix_factor: f32,
    pub parametric_rim_fresnel_power_factor: f32,
    pub parametric_rim_lift_factor: f32,
    pub outline_color_factor: [f32; 3],
    pub outline_lighting_mix_factor: f32,
    pub uv_animation: vrm_core::UvAnimation,
    pub emissive_strength: EmissiveStrength,
    pub debug_mode: MtoonDebugMode,
    pub v0_compat_shade: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MtoonDebugMode {
    #[default]
    None,
    LitShadeRate,
    Lighting,
    Normal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MtoonMaterializationOptions {
    pub debug_mode: MtoonDebugMode,
    pub v0_compat_shade: bool,
}

pub trait MtoonMaterializer {
    type Descriptor;
    type Error;

    fn materialize_mtoon(
        &mut self,
        descriptor: &MtoonMaterialDescriptor,
    ) -> Result<Self::Descriptor, Self::Error>;
}

pub trait TextureResolver {
    type Texture;
    type Error;

    fn resolve_texture(&self, texture: TextureRef) -> Result<Self::Texture, Self::Error>;
}

pub trait VisibilityAccess {
    type Error;

    fn set_node_visible(&mut self, node: NodeRef, visible: bool) -> Result<(), Self::Error>;
}

pub trait LookAtAccess {
    type Error;

    fn set_look_at_rotation(&mut self, rotation: Quat) -> Result<(), Self::Error>;
}

pub trait AnimationSink {
    type Error;

    fn apply_expression(&mut self, expression: &AppliedExpression) -> Result<(), Self::Error>;
    fn apply_runtime_events(&mut self, events: &RuntimeEvents) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug)]
pub struct VrmRuntimeDriver<'a> {
    pub document: &'a VrmDocument,
    pub animation_frame: Option<&'a VrmAnimationFrame>,
    pub runtime_events: Option<&'a RuntimeEvents>,
    pub root: Option<NodeRef>,
    pub view_mode: ViewMode,
    pub apply_vrm0_orientation: bool,
    pub vrm0_orientation_applied: bool,
}

impl<'a> VrmRuntimeDriver<'a> {
    pub fn new(document: &'a VrmDocument) -> Self {
        Self {
            document,
            animation_frame: None,
            runtime_events: None,
            root: None,
            view_mode: ViewMode::ThirdPerson,
            apply_vrm0_orientation: true,
            vrm0_orientation_applied: false,
        }
    }

    pub fn with_animation_frame(mut self, frame: &'a VrmAnimationFrame) -> Self {
        self.animation_frame = Some(frame);
        self
    }

    pub fn with_runtime_events(mut self, events: &'a RuntimeEvents) -> Self {
        self.runtime_events = Some(events);
        self
    }

    pub fn with_root(mut self, root: NodeRef) -> Self {
        self.root = Some(root);
        self
    }

    pub fn with_view_mode(mut self, mode: ViewMode) -> Self {
        self.view_mode = mode;
        self
    }

    pub fn with_vrm0_orientation(mut self, enabled: bool) -> Self {
        self.apply_vrm0_orientation = enabled;
        self
    }

    pub fn tick<T, E>(
        &mut self,
        target: &mut T,
        spring_state: Option<&mut SpringRuntimeState>,
    ) -> Result<(), AdapterError<E>>
    where
        T: TransformAccess<Error = E>
            + WorldTransformAccess<Error = E>
            + SceneGraph<Error = E>
            + ConstraintRestAccess<Error = E>
            + MorphTargetAccess<Error = E>
            + MaterialAccess<Error = E>
            + MtoonPipelineAccess<Error = E>
            + VisibilityAccess<Error = E>,
    {
        if self.apply_vrm0_orientation
            && !self.vrm0_orientation_applied
            && let Some(root) = self.root
        {
            apply_vrm0_orientation_compensation(target, self.document, root)?;
            self.vrm0_orientation_applied = true;
        }
        if let Some(frame) = self.animation_frame {
            apply_animation_frame(target, self.document, frame)?;
        }
        if let Some(events) = self.runtime_events {
            for expression in &events.expressions {
                apply_expression_binds(target, expression)?;
            }
            apply_node_constraints(target, &events.constraints)?;
            if let (Feature::Present(system), Some(state)) =
                (&self.document.spring_bone, spring_state)
            {
                step_spring_bone_system(target, system, state, events.delta)?;
            }
        }
        apply_mtoon_pipeline_hints(target, self.document)?;
        apply_emissive_strengths(target, self.document)?;
        apply_first_person_annotations(target, self.document, self.view_mode)
    }

    pub fn tick_with_spring_parity<T, E>(
        &mut self,
        target: &mut T,
        spring: Option<(&SpringRestMap, &mut CenterSpringRuntimeState)>,
    ) -> Result<(), AdapterError<E>>
    where
        T: TransformAccess<Error = E>
            + WorldTransformAccess<Error = E>
            + WorldTransformUpdate<Error = E>
            + SceneGraph<Error = E>
            + ConstraintRestAccess<Error = E>
            + MorphTargetAccess<Error = E>
            + MaterialAccess<Error = E>
            + MtoonPipelineAccess<Error = E>
            + VisibilityAccess<Error = E>,
    {
        if self.apply_vrm0_orientation
            && !self.vrm0_orientation_applied
            && let Some(root) = self.root
        {
            apply_vrm0_orientation_compensation(target, self.document, root)?;
            self.vrm0_orientation_applied = true;
        }
        if let Some(frame) = self.animation_frame {
            apply_animation_frame(target, self.document, frame)?;
        }
        if let Some(events) = self.runtime_events {
            for expression in &events.expressions {
                apply_expression_binds(target, expression)?;
            }
            apply_node_constraints(target, &events.constraints)?;
            if let (Feature::Present(system), Some((rest, state))) =
                (&self.document.spring_bone, spring)
            {
                target
                    .update_world_transforms()
                    .map_err(AdapterError::Target)?;
                step_spring_bone_system_parity(target, system, rest, state, events.delta)?;
            }
        }
        apply_mtoon_pipeline_hints(target, self.document)?;
        apply_emissive_strengths(target, self.document)?;
        apply_first_person_annotations(target, self.document, self.view_mode)
    }
}

pub fn apply_expression_binds<T, E>(
    target: &mut T,
    expression: &AppliedExpression,
) -> Result<(), AdapterError<E>>
where
    T: MorphTargetAccess<Error = E> + MaterialAccess<Error = E>,
{
    for bind in &expression.binds {
        match bind {
            ExpressionBind::MorphTarget {
                node,
                index,
                weight,
            } => target
                .set_morph_weight(*node, *index, expression.effective_weight * *weight)
                .map_err(AdapterError::Target)?,
            ExpressionBind::MaterialColor {
                material,
                kind,
                target_value,
            } => target
                .set_material_color(*material, kind, target_value)
                .map_err(AdapterError::Target)?,
            ExpressionBind::TextureTransform {
                material,
                scale,
                offset,
            } => target
                .set_texture_transform(*material, *scale, *offset)
                .map_err(AdapterError::Target)?,
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    FirstPerson,
    #[default]
    ThirdPerson,
}

pub fn apply_first_person_annotations<T, E>(
    target: &mut T,
    document: &VrmDocument,
    mode: ViewMode,
) -> Result<(), AdapterError<E>>
where
    T: VisibilityAccess<Error = E> + SceneGraph<Error = E>,
{
    let Some(first_person) = document.first_person.as_ref() else {
        return Ok(());
    };

    for annotation in &first_person.mesh_annotations {
        let visible = match (&annotation.kind, mode) {
            (FirstPersonAnnotation::Both, _) => true,
            (FirstPersonAnnotation::Auto, ViewMode::FirstPerson) => {
                !is_head_or_descendant(target, document, annotation.node)?
            }
            (FirstPersonAnnotation::Auto, ViewMode::ThirdPerson) => true,
            (FirstPersonAnnotation::FirstPersonOnly, ViewMode::FirstPerson) => true,
            (FirstPersonAnnotation::FirstPersonOnly, ViewMode::ThirdPerson) => false,
            (FirstPersonAnnotation::ThirdPersonOnly, ViewMode::FirstPerson) => false,
            (FirstPersonAnnotation::ThirdPersonOnly, ViewMode::ThirdPerson) => true,
            (FirstPersonAnnotation::Unknown(_), _) => true,
        };
        target
            .set_node_visible(annotation.node, visible)
            .map_err(AdapterError::Target)?;
    }

    Ok(())
}

pub fn is_head_or_descendant<T, E>(
    target: &T,
    document: &VrmDocument,
    node: NodeRef,
) -> Result<bool, AdapterError<E>>
where
    T: SceneGraph<Error = E>,
{
    let Some(head) = document
        .humanoid
        .bones
        .get(&HumanBoneName::Head)
        .map(|bone| bone.node)
    else {
        return Ok(false);
    };

    let mut current = Some(node);
    let mut visited = HashSet::new();
    while let Some(node) = current {
        if node == head {
            return Ok(true);
        }
        if !visited.insert(node) {
            return Ok(false);
        }
        current = target.parent(node).map_err(AdapterError::Target)?;
    }
    Ok(false)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkinVertexInfluence {
    pub joints: [usize; 4],
    pub weights: [f32; 4],
}

impl SkinVertexInfluence {
    pub fn references_any(self, erase_joints: &HashSet<usize>) -> bool {
        self.joints
            .into_iter()
            .zip(self.weights)
            .any(|(joint, weight)| weight > 0.0 && erase_joints.contains(&joint))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeadlessMeshPlan {
    pub indices: Vec<u32>,
    pub removed_triangles: usize,
}

pub trait FirstPersonMeshAccess {
    type Error;
    type Mesh: Clone;

    fn skinned_meshes_under(&self, node: NodeRef) -> Result<Vec<Self::Mesh>, Self::Error>;
    fn skin_joints(&self, mesh: &Self::Mesh) -> Result<Vec<NodeRef>, Self::Error>;
    fn mesh_indices(&self, mesh: &Self::Mesh) -> Result<Vec<u32>, Self::Error>;
    fn skin_influences(&self, mesh: &Self::Mesh) -> Result<Vec<SkinVertexInfluence>, Self::Error>;
    fn set_third_person_only(&mut self, mesh: &Self::Mesh) -> Result<(), Self::Error>;
    fn set_first_person_and_third_person(&mut self, mesh: &Self::Mesh) -> Result<(), Self::Error>;
    fn create_first_person_headless_clone(
        &mut self,
        source: &Self::Mesh,
        plan: &HeadlessMeshPlan,
    ) -> Result<(), Self::Error>;
}

pub fn plan_headless_mesh(
    indices: &[u32],
    influences: &[SkinVertexInfluence],
    erase_joints: &HashSet<usize>,
) -> HeadlessMeshPlan {
    let mut kept = Vec::with_capacity(indices.len());
    let mut removed_triangles = 0;

    for triangle in indices.chunks_exact(3) {
        let erase = triangle.iter().any(|index| {
            influences
                .get(*index as usize)
                .copied()
                .is_some_and(|influence| influence.references_any(erase_joints))
        });
        if erase {
            removed_triangles += 1;
        } else {
            kept.extend_from_slice(triangle);
        }
    }

    HeadlessMeshPlan {
        indices: kept,
        removed_triangles,
    }
}

pub fn apply_first_person_auto_headless_meshes<T, E>(
    target: &mut T,
    document: &VrmDocument,
    annotation_node: NodeRef,
) -> Result<(), AdapterError<E>>
where
    T: FirstPersonMeshAccess<Error = E> + SceneGraph<Error = E>,
{
    for mesh in target
        .skinned_meshes_under(annotation_node)
        .map_err(AdapterError::Target)?
    {
        let erase_joints = target
            .skin_joints(&mesh)
            .map_err(AdapterError::Target)?
            .into_iter()
            .enumerate()
            .filter_map(
                |(index, joint)| match is_head_or_descendant(target, document, joint) {
                    Ok(true) => Some(Ok(index)),
                    Ok(false) => None,
                    Err(err) => Some(Err(err)),
                },
            )
            .collect::<Result<HashSet<_>, AdapterError<E>>>()?;

        if erase_joints.is_empty() {
            target
                .set_first_person_and_third_person(&mesh)
                .map_err(AdapterError::Target)?;
            continue;
        }

        let plan = plan_headless_mesh(
            &target.mesh_indices(&mesh).map_err(AdapterError::Target)?,
            &target
                .skin_influences(&mesh)
                .map_err(AdapterError::Target)?,
            &erase_joints,
        );
        target
            .set_third_person_only(&mesh)
            .map_err(AdapterError::Target)?;
        target
            .create_first_person_headless_clone(&mesh, &plan)
            .map_err(AdapterError::Target)?;
    }
    Ok(())
}

pub fn apply_mtoon_pipeline_hints<T, E>(
    target: &mut T,
    document: &VrmDocument,
) -> Result<(), AdapterError<E>>
where
    T: MtoonPipelineAccess<Error = E>,
{
    for (index, material) in document.materials.iter().enumerate() {
        if let Feature::Present(mtoon) = &material.mtoon {
            target
                .set_mtoon_pipeline_passes(MaterialRef(index), &mtoon.pipeline_passes())
                .map_err(AdapterError::Target)?;
        }
    }
    Ok(())
}

pub fn mtoon_material_descriptors(
    document: &VrmDocument,
    options: MtoonMaterializationOptions,
) -> Vec<MtoonMaterialDescriptor> {
    document
        .materials
        .iter()
        .enumerate()
        .flat_map(|(index, material)| {
            let material_ref = MaterialRef(index);
            let (emissive_strength, _) = material.effective_emissive_strength();
            material.mtoon.as_ref().into_iter().flat_map(move |mtoon| {
                mtoon.pipeline_passes().into_iter().map(move |pass| {
                    mtoon_material_descriptor(
                        material_ref,
                        material.name.clone(),
                        mtoon,
                        pass,
                        emissive_strength,
                        options,
                    )
                })
            })
        })
        .collect()
}

fn mtoon_material_descriptor(
    material: MaterialRef,
    name: Option<String>,
    mtoon: &MtoonMaterial,
    pass: MtoonPipelinePass,
    emissive_strength: EmissiveStrength,
    options: MtoonMaterializationOptions,
) -> MtoonMaterialDescriptor {
    MtoonMaterialDescriptor {
        material,
        name,
        pass,
        textures: mtoon.textures.clone(),
        base_color_factor: mtoon.base_color_factor,
        emissive_factor: mtoon.emissive_factor,
        cutoff_factor: mtoon.cutoff_factor,
        shade_color_factor: mtoon.shade_color_factor,
        receive_shadow_rate_factor: mtoon.receive_shadow_rate_factor,
        shading_grade_rate_factor: mtoon.shading_grade_rate_factor,
        shading_shift_factor: mtoon.shading_shift_factor,
        shading_toony_factor: mtoon.shading_toony_factor,
        light_color_attenuation_factor: mtoon.light_color_attenuation_factor,
        gi_equalization_factor: mtoon.gi_equalization_factor,
        matcap_factor: mtoon.matcap_factor,
        parametric_rim_color_factor: mtoon.parametric_rim_color_factor,
        rim_lighting_mix_factor: mtoon.rim_lighting_mix_factor,
        parametric_rim_fresnel_power_factor: mtoon.parametric_rim_fresnel_power_factor,
        parametric_rim_lift_factor: mtoon.parametric_rim_lift_factor,
        outline_color_factor: mtoon.outline_color_factor,
        outline_lighting_mix_factor: mtoon.outline_lighting_mix_factor,
        uv_animation: mtoon.uv_animation,
        emissive_strength,
        debug_mode: options.debug_mode,
        v0_compat_shade: options.v0_compat_shade,
    }
}

pub fn apply_hdr_emissive_multipliers<T, E>(
    target: &mut T,
    document: &VrmDocument,
) -> Result<(), AdapterError<E>>
where
    T: MaterialAccess<Error = E>,
{
    apply_emissive_strengths(target, document)
}

pub fn apply_emissive_strengths<T, E>(
    target: &mut T,
    document: &VrmDocument,
) -> Result<(), AdapterError<E>>
where
    T: MaterialAccess<Error = E>,
{
    for (index, material) in document.materials.iter().enumerate() {
        let (strength, source) = material.effective_emissive_strength();
        if source != vrm_core::EmissiveStrengthSource::Default {
            target
                .set_emissive_intensity(MaterialRef(index), strength.0)
                .map_err(AdapterError::Target)?;
        }
    }
    Ok(())
}

pub fn apply_vrm0_orientation_compensation<T, E>(
    target: &mut T,
    document: &VrmDocument,
    root: NodeRef,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E>,
{
    let Some(compatibility) = document.compatibility.vrm0 else {
        return Ok(());
    };
    let mut transform = target.local_transform(root).map_err(AdapterError::Target)?;
    transform.translation += compatibility.orientation_correction.translation;
    transform.rotation = compatibility.orientation_correction.rotation * transform.rotation;
    transform.scale *= compatibility.orientation_correction.scale;
    target
        .set_local_transform(root, transform)
        .map_err(AdapterError::Target)
}

pub fn collect_spring_colliders<T, E>(
    target: &T,
    system: &SpringBoneSystem,
    spring: &Spring,
) -> Result<Vec<ColliderShape>, AdapterError<E>>
where
    T: WorldTransformAccess<Error = E>,
{
    let center_world = spring
        .center
        .map(|node| target.world_transform(node).map_err(AdapterError::Target))
        .transpose()?;

    spring
        .collider_groups
        .iter()
        .filter_map(|group_index| system.collider_groups.get(*group_index))
        .flat_map(|group| &group.colliders)
        .filter_map(|collider_index| system.colliders.get(*collider_index))
        .map(|collider| {
            target
                .world_transform(collider.node)
                .map(|world| collider_shape_in_simulation_space(collider, world, center_world))
                .map_err(AdapterError::Target)
        })
        .collect()
}

pub fn collect_spring_colliders_world<T, E>(
    target: &T,
    system: &SpringBoneSystem,
    spring: &Spring,
) -> Result<Vec<ColliderShape>, AdapterError<E>>
where
    T: WorldTransformAccess<Error = E>,
{
    spring
        .collider_groups
        .iter()
        .filter_map(|group_index| system.collider_groups.get(*group_index))
        .flat_map(|group| &group.colliders)
        .filter_map(|collider_index| system.colliders.get(*collider_index))
        .map(|collider| {
            target
                .world_transform(collider.node)
                .map(|world| collider_shape_in_simulation_space(collider, world, None))
                .map_err(AdapterError::Target)
        })
        .collect()
}

pub fn apply_node_constraints<T, E>(
    target: &mut T,
    constraints: &[NodeConstraint],
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E>
        + WorldTransformAccess<Error = E>
        + SceneGraph<Error = E>
        + ConstraintRestAccess<Error = E>,
{
    for constraint in constraints {
        apply_node_constraint(target, constraint)?;
    }
    Ok(())
}

pub fn apply_node_constraint<T, E>(
    target: &mut T,
    constraint: &NodeConstraint,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E>
        + WorldTransformAccess<Error = E>
        + SceneGraph<Error = E>
        + ConstraintRestAccess<Error = E>,
{
    let rest = target
        .constraint_rest_state(constraint.destination, constraint.source)
        .map_err(AdapterError::Target)?;
    let source_local = target
        .local_transform(constraint.source)
        .map_err(AdapterError::Target)?;

    let rotation = match constraint.kind {
        ConstraintKind::Rotation => {
            solve_rotation_constraint(rest, source_local.rotation, constraint.weight)
        }
        ConstraintKind::Roll { axis } => {
            solve_roll_constraint(rest, source_local.rotation, axis, constraint.weight)
        }
        ConstraintKind::Aim { axis } => {
            let destination_world = target
                .world_transform(constraint.destination)
                .map_err(AdapterError::Target)?;
            let source_world = target
                .world_transform(constraint.source)
                .map_err(AdapterError::Target)?;
            let parent_rotation = target
                .parent(constraint.destination)
                .map_err(AdapterError::Target)?
                .map(|parent| {
                    target
                        .world_transform(parent)
                        .map(|transform| transform.rotation)
                        .map_err(AdapterError::Target)
                })
                .transpose()?
                .unwrap_or(Quat::IDENTITY);
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: rest.destination_rest_rotation,
                destination_world_position: destination_world.translation,
                source_world_position: source_world.translation,
                destination_parent_world_rotation: parent_rotation,
                axis,
                weight: constraint.weight,
            })
        }
    };

    target
        .set_local_rotation(constraint.destination, rotation)
        .map_err(AdapterError::Target)
}

pub fn apply_spring_joint_tail<T, E>(
    target: &mut T,
    joint: NodeRef,
    local_axis: Vec3,
    tail_world_position: Vec3,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E> + WorldTransformAccess<Error = E> + SceneGraph<Error = E>,
{
    let parent = target.parent(joint).map_err(AdapterError::Target)?;
    let parent_world = parent
        .map(|node| target.world_transform(node).map_err(AdapterError::Target))
        .transpose()?
        .unwrap_or_default();
    let joint_world = target
        .world_transform(joint)
        .map_err(AdapterError::Target)?;
    let joint_local = target
        .local_transform(joint)
        .map_err(AdapterError::Target)?;

    let rotation = solve_spring_joint_rotation(vrm_runtime::SpringJointRotationInput {
        parent_world_rotation: parent_world.rotation,
        joint_rest_rotation: joint_local.rotation,
        local_axis,
        parent_world_position: joint_world.translation,
        tail_world_position,
    });

    target
        .set_local_rotation(joint, rotation)
        .map_err(AdapterError::Target)
}

pub fn step_spring_bone_system<T, E>(
    target: &mut T,
    system: &SpringBoneSystem,
    state: &mut SpringRuntimeState,
    delta: DeltaTime,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E> + WorldTransformAccess<Error = E> + SceneGraph<Error = E>,
{
    for (spring_index, spring) in system.springs.iter().enumerate() {
        let colliders = collect_spring_colliders(target, system, spring)?;
        for (joint_index, joint) in spring.joints.iter().enumerate() {
            let joint_world = target
                .world_transform(joint.node)
                .map_err(AdapterError::Target)?;
            let joint_local = target
                .local_transform(joint.node)
                .map_err(AdapterError::Target)?;
            let child_world = target
                .children(joint.node)
                .map_err(AdapterError::Target)?
                .first()
                .copied()
                .map(|child| target.world_transform(child).map_err(AdapterError::Target))
                .transpose()?;
            let (local_axis, bone_length) =
                spring_axis_and_length(joint_world, joint_local, child_world);
            let particle = state.get_mut(spring_index, joint_index).ok_or(
                AdapterError::InvalidSpringJoint {
                    spring_index,
                    joint_index,
                },
            )?;
            initialize_spring_particle_if_needed(
                particle,
                joint_world.translation,
                joint_world.rotation,
                local_axis,
                bone_length,
            );
            let tail = step_spring_joint(
                particle,
                SpringJointSimulationInput {
                    joint,
                    parent_position: joint_world.translation,
                    parent_rotation: joint_world.rotation,
                    local_axis,
                    bone_length,
                    colliders: &colliders,
                    delta,
                },
            );
            apply_spring_joint_tail(target, joint.node, local_axis, tail)?;
        }
    }
    Ok(())
}

pub fn step_spring_bone_system_parity<T, E>(
    target: &mut T,
    system: &SpringBoneSystem,
    rest_map: &SpringRestMap,
    state: &mut CenterSpringRuntimeState,
    delta: DeltaTime,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E>
        + WorldTransformAccess<Error = E>
        + WorldTransformUpdate<Error = E>
        + SceneGraph<Error = E>,
{
    if delta.0 <= 0.0 {
        return Ok(());
    }

    for (spring_index, spring) in system.springs.iter().enumerate() {
        let colliders = collect_spring_colliders_world(target, system, spring)?;
        for (joint_index, joint) in spring.joints.iter().enumerate() {
            let entry = rest_map.get(spring_index, joint_index).ok_or(
                AdapterError::InvalidSpringJoint {
                    spring_index,
                    joint_index,
                },
            )?;
            let particle = state.get_mut(spring_index, joint_index).ok_or(
                AdapterError::InvalidSpringJoint {
                    spring_index,
                    joint_index,
                },
            )?;
            let parent_world = target
                .parent(joint.node)
                .map_err(AdapterError::Target)?
                .map(|parent| target.world_transform(parent).map_err(AdapterError::Target))
                .transpose()?
                .unwrap_or_default();
            let joint_world = target
                .world_transform(joint.node)
                .map_err(AdapterError::Target)?;
            let child_world = entry
                .child
                .map(|child| target.world_transform(child).map_err(AdapterError::Target))
                .transpose()?;
            let center_world = entry
                .center
                .map(|center| target.world_transform(center).map_err(AdapterError::Target))
                .transpose()?;
            let (_, rotation) = step_spring_joint_parity(
                particle,
                SpringJointParityInput {
                    joint,
                    rest: entry.rest,
                    parent_world,
                    joint_world,
                    child_world,
                    center_world,
                    colliders: &colliders,
                    delta,
                },
            );
            target
                .set_local_rotation(joint.node, rotation)
                .map_err(AdapterError::Target)?;
            target
                .update_world_transforms()
                .map_err(AdapterError::Target)?;
        }
    }
    Ok(())
}

fn spring_axis_and_length(
    joint_world: Transform,
    joint_local: Transform,
    child_world: Option<Transform>,
) -> (Vec3, f32) {
    let Some(child_world) = child_world else {
        return (joint_local.translation.normalize_or(Vec3::Y), 0.07);
    };
    let world_delta = child_world.translation - joint_world.translation;
    let bone_length = world_delta.length();
    if bone_length <= f32::EPSILON {
        (Vec3::Y, 1.0)
    } else {
        (
            joint_world.rotation.inverse() * (world_delta / bone_length),
            bone_length,
        )
    }
}

fn initialize_spring_particle_if_needed(
    particle: &mut SpringParticleState,
    joint_position: Vec3,
    joint_rotation: Quat,
    local_axis: Vec3,
    bone_length: f32,
) {
    if particle.current_tail == Vec3::ZERO && particle.previous_tail == Vec3::ZERO {
        let tail = joint_position + (joint_rotation * local_axis).normalize_or_zero() * bone_length;
        particle.current_tail = tail;
        particle.previous_tail = tail;
    }
}

pub fn apply_animation_frame<T, E>(
    target: &mut T,
    document: &VrmDocument,
    frame: &VrmAnimationFrame,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E> + MorphTargetAccess<Error = E> + MaterialAccess<Error = E>,
{
    apply_humanoid_frame(target, document, frame)?;
    apply_expression_frame(target, document, frame)?;
    Ok(())
}

pub fn apply_animation_frame_with_look_at<T, E>(
    target: &mut T,
    document: &VrmDocument,
    frame: &VrmAnimationFrame,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E>
        + MorphTargetAccess<Error = E>
        + MaterialAccess<Error = E>
        + LookAtAccess<Error = E>,
{
    apply_animation_frame(target, document, frame)?;
    apply_look_at_frame(target, frame)?;
    Ok(())
}

pub fn apply_vrma_animation_frame_with_look_at<T, E>(
    target: &mut T,
    rig: &mut HumanoidPoseRig,
    document: &VrmDocument,
    frame: &VrmAnimationFrame,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E>
        + WorldTransformAccess<Error = E>
        + SceneGraph<Error = E>
        + MorphTargetAccess<Error = E>
        + MaterialAccess<Error = E>
        + LookAtAccess<Error = E>,
{
    apply_vrma_humanoid_frame(target, rig, frame)?;
    apply_expression_frame(target, document, frame)?;
    apply_look_at_frame(target, frame)?;
    Ok(())
}

pub fn apply_vrma_humanoid_frame<T, E>(
    target: &mut T,
    rig: &mut HumanoidPoseRig,
    frame: &VrmAnimationFrame,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E> + WorldTransformAccess<Error = E> + SceneGraph<Error = E>,
{
    let mut pose = rig.get_normalized_pose();
    for (bone, rotation) in &frame.humanoid_rotations {
        let translation = pose
            .get(bone)
            .map(|transform| transform.translation)
            .unwrap_or(Vec3::ZERO);
        pose.insert(
            bone.clone(),
            vrm_core::PoseTransform {
                translation,
                rotation: *rotation,
            },
        );
    }
    if let Some(translation) = frame.hips_translation {
        let rest_translation = rig
            .normalized_rest_pose()
            .get(&HumanBoneName::Hips)
            .map(|transform| transform.translation)
            .unwrap_or(Vec3::ZERO);
        let rotation = pose
            .get(&HumanBoneName::Hips)
            .map(|transform| transform.rotation)
            .unwrap_or(Quat::IDENTITY);
        pose.insert(
            HumanBoneName::Hips,
            vrm_core::PoseTransform {
                translation: translation - rest_translation,
                rotation,
            },
        );
    }
    rig.set_normalized_pose(&pose);
    rig.apply_normalized_to_raw(target)
}

pub fn apply_humanoid_frame<T, E>(
    target: &mut T,
    document: &VrmDocument,
    frame: &VrmAnimationFrame,
) -> Result<(), AdapterError<E>>
where
    T: TransformAccess<Error = E>,
{
    for (bone, rotation) in &frame.humanoid_rotations {
        if let Some(human_bone) = document.humanoid.bones.get(bone) {
            target
                .set_local_rotation(human_bone.node, *rotation)
                .map_err(AdapterError::Target)?;
        }
    }

    if let Some(translation) = frame.hips_translation
        && let Some(hips) = document.humanoid.bones.get(&HumanBoneName::Hips)
    {
        let mut transform = target
            .local_transform(hips.node)
            .map_err(AdapterError::Target)?;
        transform.translation = translation;
        target
            .set_local_transform(hips.node, transform)
            .map_err(AdapterError::Target)?;
    }

    Ok(())
}

pub fn apply_expression_frame<T, E>(
    target: &mut T,
    document: &VrmDocument,
    frame: &VrmAnimationFrame,
) -> Result<(), AdapterError<E>>
where
    T: MorphTargetAccess<Error = E> + MaterialAccess<Error = E>,
{
    let Some(expressions) = document.expressions.as_ref() else {
        return Ok(());
    };

    for (name, weight) in &frame.preset_expressions {
        apply_preset_expression_value(target, expressions, name, *weight)?;
    }
    for (name, weight) in &frame.custom_expressions {
        if let Some(expression) = expressions.custom.get(name) {
            apply_expression_binds(
                target,
                &AppliedExpression {
                    name: name.clone(),
                    effective_weight: *weight,
                    binds: expression.binds.clone(),
                },
            )?;
        }
    }

    Ok(())
}

pub fn apply_look_at_frame<T, E>(
    target: &mut T,
    frame: &VrmAnimationFrame,
) -> Result<(), AdapterError<E>>
where
    T: LookAtAccess<Error = E>,
{
    if let Some(rotation) = frame.look_at {
        target
            .set_look_at_rotation(rotation)
            .map_err(AdapterError::Target)?;
    }
    Ok(())
}

fn apply_preset_expression_value<T, E>(
    target: &mut T,
    expressions: &vrm_core::ExpressionSet,
    name: &ExpressionName,
    weight: f32,
) -> Result<(), AdapterError<E>>
where
    T: MorphTargetAccess<Error = E> + MaterialAccess<Error = E>,
{
    if let Some(expression) = expressions.preset.get(name) {
        apply_expression_binds(
            target,
            &AppliedExpression {
                name: name.as_str().to_owned(),
                effective_weight: weight,
                binds: expression.binds.clone(),
            },
        )?;
    }
    Ok(())
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AdapterError<E> {
    #[error("target adapter error: {0}")]
    Target(E),
    #[error("spring joint state is missing for spring {spring_index}, joint {joint_index}")]
    InvalidSpringJoint {
        spring_index: usize,
        joint_index: usize,
    },
}

#[cfg(feature = "bevy")]
pub mod bevy {
    //! Optional Bevy adapter skeleton.
    //!
    //! This module intentionally contains only marker types until a concrete
    //! Bevy version is selected by downstream users.

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct BevyAdapter;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use vrm_core::{
        EmissiveStrength, Expression, ExpressionSet, Feature, FirstPerson,
        FirstPersonMeshAnnotation, HdrEmissiveMultiplier, HumanBone, Humanoid, MtoonMaterial,
        MtoonRenderQueue, PoseTransform, RotationTrack, VrmAnimation, VrmDocument,
    };
    use vrm_runtime::sample_vrm_animation;

    #[derive(Default)]
    struct Mock {
        morphs: Vec<(NodeRef, usize, f32)>,
        rotations: Vec<(NodeRef, Quat)>,
        translations: Vec<(NodeRef, Vec3)>,
        local_sets: Vec<(NodeRef, Transform)>,
        look_at_rotations: Vec<Quat>,
        mtoon_passes: Vec<(MaterialRef, Vec<MtoonPipelinePass>)>,
        emissive_intensities: Vec<(MaterialRef, f32)>,
        visibility: Vec<(NodeRef, bool)>,
        first_person_meshes: Vec<usize>,
        third_person_meshes: Vec<usize>,
        headless_meshes: Vec<(usize, HeadlessMeshPlan)>,
        world_updates: usize,
        skinned_meshes: std::collections::HashMap<NodeRef, Vec<usize>>,
        mesh_joints: std::collections::HashMap<usize, Vec<NodeRef>>,
        mesh_indices: std::collections::HashMap<usize, Vec<u32>>,
        mesh_influences: std::collections::HashMap<usize, Vec<SkinVertexInfluence>>,
        parents: std::collections::HashMap<NodeRef, NodeRef>,
        local_transforms: std::collections::HashMap<NodeRef, Transform>,
        world_transforms: std::collections::HashMap<NodeRef, Transform>,
        constraint_rest: std::collections::HashMap<(NodeRef, NodeRef), ConstraintRestState>,
    }

    impl TransformAccess for Mock {
        type Error = Infallible;

        fn local_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
            Ok(self
                .local_transforms
                .get(&node)
                .copied()
                .unwrap_or_default())
        }

        fn set_local_transform(
            &mut self,
            node: NodeRef,
            transform: Transform,
        ) -> Result<(), Self::Error> {
            self.local_sets.push((node, transform));
            Ok(())
        }

        fn set_local_rotation(&mut self, node: NodeRef, rotation: Quat) -> Result<(), Self::Error> {
            self.rotations.push((node, rotation));
            Ok(())
        }

        fn translate_local(&mut self, node: NodeRef, translation: Vec3) -> Result<(), Self::Error> {
            self.translations.push((node, translation));
            Ok(())
        }
    }

    impl WorldTransformAccess for Mock {
        type Error = Infallible;

        fn world_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
            Ok(self
                .world_transforms
                .get(&node)
                .copied()
                .unwrap_or_default())
        }
    }

    impl WorldTransformUpdate for Mock {
        type Error = Infallible;

        fn update_world_transforms(&mut self) -> Result<(), Self::Error> {
            self.world_updates += 1;
            Ok(())
        }
    }

    impl SceneGraph for Mock {
        type Error = Infallible;

        fn parent(&self, node: NodeRef) -> Result<Option<NodeRef>, Self::Error> {
            Ok(self.parents.get(&node).copied())
        }

        fn children(&self, node: NodeRef) -> Result<Vec<NodeRef>, Self::Error> {
            Ok(self
                .parents
                .iter()
                .filter_map(|(child, parent)| (*parent == node).then_some(*child))
                .collect())
        }
    }

    impl ConstraintRestAccess for Mock {
        type Error = Infallible;

        fn constraint_rest_state(
            &self,
            destination: NodeRef,
            source: NodeRef,
        ) -> Result<ConstraintRestState, Self::Error> {
            Ok(self
                .constraint_rest
                .get(&(destination, source))
                .copied()
                .unwrap_or(ConstraintRestState {
                    destination_rest_rotation: Quat::IDENTITY,
                    source_rest_rotation: Quat::IDENTITY,
                }))
        }
    }

    impl ConstraintRestAccess for ConstraintRestMap {
        type Error = Infallible;

        fn constraint_rest_state(
            &self,
            destination: NodeRef,
            source: NodeRef,
        ) -> Result<ConstraintRestState, Self::Error> {
            Ok(self
                .get(destination, source)
                .unwrap_or(ConstraintRestState {
                    destination_rest_rotation: Quat::IDENTITY,
                    source_rest_rotation: Quat::IDENTITY,
                }))
        }
    }

    impl MorphTargetAccess for Mock {
        type Error = Infallible;

        fn set_morph_weight(
            &mut self,
            node: NodeRef,
            morph_index: usize,
            weight: f32,
        ) -> Result<(), Self::Error> {
            self.morphs.push((node, morph_index, weight));
            Ok(())
        }
    }

    impl MaterialAccess for Mock {
        type Error = Infallible;

        fn set_material_color(
            &mut self,
            _material: MaterialRef,
            _property: &str,
            _value: &[f32],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn set_texture_transform(
            &mut self,
            _material: MaterialRef,
            _scale: Option<[f32; 2]>,
            _offset: Option<[f32; 2]>,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn set_emissive_intensity(
            &mut self,
            material: MaterialRef,
            intensity: f32,
        ) -> Result<(), Self::Error> {
            self.emissive_intensities.push((material, intensity));
            Ok(())
        }
    }

    impl MtoonPipelineAccess for Mock {
        type Error = Infallible;

        fn set_mtoon_pipeline_passes(
            &mut self,
            material: MaterialRef,
            passes: &[MtoonPipelinePass],
        ) -> Result<(), Self::Error> {
            self.mtoon_passes.push((material, passes.to_vec()));
            Ok(())
        }
    }

    impl VisibilityAccess for Mock {
        type Error = Infallible;

        fn set_node_visible(&mut self, node: NodeRef, visible: bool) -> Result<(), Self::Error> {
            self.visibility.push((node, visible));
            Ok(())
        }
    }

    impl LookAtAccess for Mock {
        type Error = Infallible;

        fn set_look_at_rotation(&mut self, rotation: Quat) -> Result<(), Self::Error> {
            self.look_at_rotations.push(rotation);
            Ok(())
        }
    }

    impl FirstPersonMeshAccess for Mock {
        type Error = Infallible;
        type Mesh = usize;

        fn skinned_meshes_under(&self, node: NodeRef) -> Result<Vec<Self::Mesh>, Self::Error> {
            Ok(self.skinned_meshes.get(&node).cloned().unwrap_or_default())
        }

        fn skin_joints(&self, mesh: &Self::Mesh) -> Result<Vec<NodeRef>, Self::Error> {
            Ok(self.mesh_joints.get(mesh).cloned().unwrap_or_default())
        }

        fn mesh_indices(&self, mesh: &Self::Mesh) -> Result<Vec<u32>, Self::Error> {
            Ok(self.mesh_indices.get(mesh).cloned().unwrap_or_default())
        }

        fn skin_influences(
            &self,
            mesh: &Self::Mesh,
        ) -> Result<Vec<SkinVertexInfluence>, Self::Error> {
            Ok(self.mesh_influences.get(mesh).cloned().unwrap_or_default())
        }

        fn set_third_person_only(&mut self, mesh: &Self::Mesh) -> Result<(), Self::Error> {
            self.third_person_meshes.push(*mesh);
            Ok(())
        }

        fn set_first_person_and_third_person(
            &mut self,
            mesh: &Self::Mesh,
        ) -> Result<(), Self::Error> {
            self.first_person_meshes.push(*mesh);
            self.third_person_meshes.push(*mesh);
            Ok(())
        }

        fn create_first_person_headless_clone(
            &mut self,
            source: &Self::Mesh,
            plan: &HeadlessMeshPlan,
        ) -> Result<(), Self::Error> {
            self.headless_meshes.push((*source, plan.clone()));
            Ok(())
        }
    }

    struct FixtureScene {
        scene: vrm_io::GltfSceneRest,
        local_overrides: HashMap<NodeRef, Transform>,
        world_overrides: HashMap<NodeRef, Transform>,
        rotations: Vec<(NodeRef, Quat)>,
        morphs: Vec<(NodeRef, usize, f32)>,
        look_at_rotations: Vec<Quat>,
    }

    impl FixtureScene {
        fn new(scene: vrm_io::GltfSceneRest) -> Self {
            Self {
                scene,
                local_overrides: HashMap::new(),
                world_overrides: HashMap::new(),
                rotations: Vec::new(),
                morphs: Vec::new(),
                look_at_rotations: Vec::new(),
            }
        }

        fn node(&self, node: NodeRef) -> &vrm_io::GltfNodeRest {
            self.scene
                .node(node.0)
                .unwrap_or_else(|| panic!("missing fixture node {}", node.0))
        }

        fn local(&self, node: NodeRef) -> Transform {
            self.local_overrides
                .get(&node)
                .copied()
                .unwrap_or_else(|| self.node(node).local)
        }

        fn refresh_node_world(&mut self, node: NodeRef) {
            let local = self.local(node);
            let world = self
                .node(node)
                .parent
                .map(NodeRef)
                .and_then(|parent| self.world_overrides.get(&parent).copied())
                .map(|parent| compose_transform(parent, local))
                .unwrap_or(local);
            self.world_overrides.insert(node, world);
            for child in self.node(node).children.clone() {
                self.refresh_node_world(NodeRef(child));
            }
        }
    }

    impl TransformAccess for FixtureScene {
        type Error = Infallible;

        fn local_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
            Ok(self.local(node))
        }

        fn set_local_transform(
            &mut self,
            node: NodeRef,
            transform: Transform,
        ) -> Result<(), Self::Error> {
            self.local_overrides.insert(node, transform);
            Ok(())
        }

        fn set_local_rotation(&mut self, node: NodeRef, rotation: Quat) -> Result<(), Self::Error> {
            let mut local = self.local(node);
            local.rotation = rotation;
            self.local_overrides.insert(node, local);
            self.rotations.push((node, rotation));
            Ok(())
        }

        fn translate_local(&mut self, node: NodeRef, translation: Vec3) -> Result<(), Self::Error> {
            let mut local = self.local(node);
            local.translation = translation;
            self.local_overrides.insert(node, local);
            Ok(())
        }
    }

    impl WorldTransformAccess for FixtureScene {
        type Error = Infallible;

        fn world_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
            Ok(self
                .world_overrides
                .get(&node)
                .copied()
                .unwrap_or_else(|| self.node(node).world))
        }
    }

    impl WorldTransformUpdate for FixtureScene {
        type Error = Infallible;

        fn update_world_transforms(&mut self) -> Result<(), Self::Error> {
            for index in 0..self.scene.nodes.len() {
                if self.scene.nodes[index].parent.is_none() {
                    self.refresh_node_world(NodeRef(index));
                }
            }
            Ok(())
        }
    }

    impl SceneGraph for FixtureScene {
        type Error = Infallible;

        fn parent(&self, node: NodeRef) -> Result<Option<NodeRef>, Self::Error> {
            Ok(self.node(node).parent.map(NodeRef))
        }

        fn children(&self, node: NodeRef) -> Result<Vec<NodeRef>, Self::Error> {
            Ok(self
                .node(node)
                .children
                .iter()
                .copied()
                .map(NodeRef)
                .collect())
        }
    }

    impl MorphTargetAccess for FixtureScene {
        type Error = Infallible;

        fn set_morph_weight(
            &mut self,
            node: NodeRef,
            morph_index: usize,
            weight: f32,
        ) -> Result<(), Self::Error> {
            self.morphs.push((node, morph_index, weight));
            Ok(())
        }
    }

    impl MaterialAccess for FixtureScene {
        type Error = Infallible;

        fn set_material_color(
            &mut self,
            _material: MaterialRef,
            _property: &str,
            _value: &[f32],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn set_texture_transform(
            &mut self,
            _material: MaterialRef,
            _scale: Option<[f32; 2]>,
            _offset: Option<[f32; 2]>,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn set_emissive_intensity(
            &mut self,
            _material: MaterialRef,
            _intensity: f32,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl LookAtAccess for FixtureScene {
        type Error = Infallible;

        fn set_look_at_rotation(&mut self, rotation: Quat) -> Result<(), Self::Error> {
            self.look_at_rotations.push(rotation);
            Ok(())
        }
    }

    fn compose_transform(parent: Transform, child: Transform) -> Transform {
        let matrix = transform_matrix(parent) * transform_matrix(child);
        let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
        Transform {
            translation,
            rotation,
            scale,
        }
    }

    #[test]
    fn humanoid_pose_rig_round_trips_raw_relative_pose() {
        let document = VrmDocument {
            humanoid: Humanoid {
                bones: [(
                    HumanBoneName::Hips,
                    HumanBone {
                        node: NodeRef(0),
                        rest: Transform::default(),
                    },
                )]
                .into_iter()
                .collect(),
            },
            ..VrmDocument::default()
        };
        let mut mock = Mock {
            local_transforms: [(
                NodeRef(0),
                Transform {
                    translation: Vec3::Y,
                    rotation: Quat::from_rotation_y(0.25),
                    scale: Vec3::ONE,
                },
            )]
            .into_iter()
            .collect(),
            world_transforms: [(NodeRef(0), Transform::default())].into_iter().collect(),
            ..Mock::default()
        };
        let rig = HumanoidPoseRig::capture(&mock, &document).unwrap();
        mock.local_transforms.insert(
            NodeRef(0),
            Transform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::from_rotation_y(0.75),
                scale: Vec3::ONE,
            },
        );

        let pose = rig.get_raw_pose(&mock).unwrap();
        rig.set_raw_pose(&mut mock, &pose).unwrap();

        assert_eq!(
            pose.get(&HumanBoneName::Hips).unwrap().translation,
            Vec3::new(1.0, 1.0, 3.0)
        );
        assert_eq!(mock.local_sets.last().unwrap().0, NodeRef(0));
        assert!(
            mock.local_sets
                .last()
                .unwrap()
                .1
                .translation
                .abs_diff_eq(Vec3::new(1.0, 2.0, 3.0), 0.0001)
        );
    }

    #[test]
    fn humanoid_pose_snapshot_reports_numeric_mismatches() {
        let mut actual = RawPose::new();
        actual.insert(
            HumanBoneName::Hips,
            PoseTransform {
                translation: Vec3::X,
                rotation: Quat::IDENTITY,
            },
        );
        let mut expected = RawPose::new();
        expected.insert(
            HumanBoneName::Hips,
            PoseTransform {
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            },
        );
        let snapshot = HumanoidPoseSnapshot {
            raw: actual,
            normalized: vrm_core::NormalizedPose::new(),
        };
        let expected = HumanoidPoseSnapshot {
            raw: expected,
            normalized: vrm_core::NormalizedPose::new(),
        };

        let mismatches = snapshot.mismatches(
            &expected,
            PoseTolerance {
                translation: 0.5,
                rotation_radians: 0.001,
            },
        );

        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].bone, HumanBoneName::Hips);
        assert_eq!(mismatches[0].translation_delta, 1.0);
    }

    #[test]
    fn humanoid_pose_rig_applies_normalized_pose_to_raw_bones() {
        let document = VrmDocument {
            humanoid: Humanoid {
                bones: [
                    (
                        HumanBoneName::Hips,
                        HumanBone {
                            node: NodeRef(1),
                            rest: Transform::default(),
                        },
                    ),
                    (
                        HumanBoneName::Head,
                        HumanBone {
                            node: NodeRef(2),
                            rest: Transform::default(),
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            },
            ..VrmDocument::default()
        };
        let mut mock = Mock {
            parents: [(NodeRef(1), NodeRef(0)), (NodeRef(2), NodeRef(1))]
                .into_iter()
                .collect(),
            local_transforms: [
                (NodeRef(1), Transform::default()),
                (NodeRef(2), Transform::default()),
            ]
            .into_iter()
            .collect(),
            world_transforms: [
                (
                    NodeRef(0),
                    Transform {
                        translation: Vec3::X,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
                (
                    NodeRef(1),
                    Transform {
                        translation: Vec3::new(1.0, 1.0, 0.0),
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
                (
                    NodeRef(2),
                    Transform {
                        translation: Vec3::new(1.0, 2.0, 0.0),
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Mock::default()
        };
        let mut rig = HumanoidPoseRig::capture(&mock, &document).unwrap();
        let mut normalized = rig.get_normalized_pose();
        normalized.insert(
            HumanBoneName::Hips,
            PoseTransform {
                translation: Vec3::new(0.0, 2.0, 0.0),
                rotation: Quat::IDENTITY,
            },
        );
        normalized.insert(
            HumanBoneName::Head,
            PoseTransform {
                translation: Vec3::ZERO,
                rotation: Quat::from_rotation_z(0.5),
            },
        );

        rig.set_normalized_pose(&normalized);
        rig.apply_normalized_to_raw(&mut mock).unwrap();

        assert!(mock.local_sets.iter().any(|(node, transform)| {
            *node == NodeRef(1)
                && transform
                    .translation
                    .abs_diff_eq(Vec3::new(0.0, 3.0, 0.0), 0.0001)
        }));
        assert!(mock.local_sets.iter().any(|(node, transform)| {
            *node == NodeRef(2)
                && (transform.rotation * Vec3::X)
                    .abs_diff_eq(Quat::from_rotation_z(0.5) * Vec3::X, 0.0001)
        }));
    }

    #[test]
    fn expression_bind_applies_to_mock() {
        let expression = AppliedExpression {
            name: "blink".to_owned(),
            effective_weight: 0.5,
            binds: vec![ExpressionBind::MorphTarget {
                node: NodeRef(3),
                index: 2,
                weight: 100.0,
            }],
        };
        let mut mock = Mock::default();
        apply_expression_binds(&mut mock, &expression).unwrap();
        assert_eq!(mock.morphs, vec![(NodeRef(3), 2, 50.0)]);
    }

    #[test]
    fn animation_frame_applies_humanoid_and_expression_binds() {
        let document = VrmDocument {
            humanoid: Humanoid {
                bones: [
                    (
                        HumanBoneName::Hips,
                        HumanBone {
                            node: NodeRef(0),
                            rest: Transform::default(),
                        },
                    ),
                    (
                        HumanBoneName::Head,
                        HumanBone {
                            node: NodeRef(1),
                            rest: Transform::default(),
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            },
            expressions: Feature::Present(ExpressionSet {
                preset: [(
                    ExpressionName::Blink,
                    Expression {
                        binds: vec![ExpressionBind::MorphTarget {
                            node: NodeRef(1),
                            index: 0,
                            weight: 100.0,
                        }],
                        ..Expression::default()
                    },
                )]
                .into_iter()
                .collect(),
                custom: Default::default(),
            }),
            ..VrmDocument::default()
        };
        let animation = VrmAnimation {
            humanoid_rotation_tracks: [(
                HumanBoneName::Head,
                RotationTrack {
                    times: vec![0.0],
                    values: vec![Quat::from_rotation_y(0.5)],
                },
            )]
            .into_iter()
            .collect(),
            hips_translation: Some(vrm_core::TranslationTrack {
                times: vec![0.0],
                values: vec![Vec3::new(1.0, 2.0, 3.0)],
            }),
            preset_expression_tracks: [(
                ExpressionName::Blink,
                vrm_core::ScalarTrack {
                    times: vec![0.0],
                    values: vec![0.25],
                },
            )]
            .into_iter()
            .collect(),
            look_at_track: Some(RotationTrack {
                times: vec![0.0],
                values: vec![Quat::from_rotation_x(0.125)],
            }),
            ..VrmAnimation::default()
        };
        let frame = sample_vrm_animation(&animation, 0.0);
        let mut mock = Mock::default();

        apply_animation_frame_with_look_at(&mut mock, &document, &frame).unwrap();

        assert_eq!(mock.rotations.len(), 1);
        assert_eq!(mock.rotations[0].0, NodeRef(1));
        assert!(mock.translations.is_empty());
        assert_eq!(mock.local_sets.len(), 1);
        assert_eq!(mock.local_sets[0].0, NodeRef(0));
        assert_eq!(mock.local_sets[0].1.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(mock.morphs, vec![(NodeRef(1), 0, 25.0)]);
        assert_eq!(mock.look_at_rotations, vec![Quat::from_rotation_x(0.125)]);
    }

    #[test]
    fn vrma_humanoid_frame_applies_through_normalized_pose_rig() {
        let document = VrmDocument {
            humanoid: Humanoid {
                bones: [
                    (
                        HumanBoneName::Hips,
                        HumanBone {
                            node: NodeRef(0),
                            rest: Transform::default(),
                        },
                    ),
                    (
                        HumanBoneName::Head,
                        HumanBone {
                            node: NodeRef(1),
                            rest: Transform::default(),
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            },
            ..VrmDocument::default()
        };
        let mut mock = Mock {
            local_transforms: [
                (
                    NodeRef(0),
                    Transform {
                        translation: Vec3::Y,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
                (
                    NodeRef(1),
                    Transform {
                        translation: Vec3::Y,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            world_transforms: [
                (
                    NodeRef(0),
                    Transform {
                        translation: Vec3::Y,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
                (
                    NodeRef(1),
                    Transform {
                        translation: Vec3::new(0.0, 2.0, 0.0),
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            parents: [(NodeRef(1), NodeRef(0))].into_iter().collect(),
            ..Mock::default()
        };
        let mut rig = HumanoidPoseRig::capture(&mock, &document).unwrap();
        let frame = VrmAnimationFrame {
            humanoid_rotations: [(HumanBoneName::Head, Quat::from_rotation_y(0.25))]
                .into_iter()
                .collect(),
            hips_translation: Some(Vec3::new(0.0, 1.25, 0.0)),
            ..VrmAnimationFrame::default()
        };

        apply_vrma_humanoid_frame(&mut mock, &mut rig, &frame).unwrap();

        let hips = mock
            .local_sets
            .iter()
            .find(|(node, _)| *node == NodeRef(0))
            .expect("hips writeback");
        let head = mock
            .local_sets
            .iter()
            .find(|(node, _)| *node == NodeRef(1))
            .expect("head writeback");
        assert_eq!(hips.1.translation, Vec3::new(0.0, 1.25, 0.0));
        assert!(
            head.1
                .rotation
                .abs_diff_eq(Quat::from_rotation_y(0.25), 0.0001)
        );
    }

    #[test]
    fn humanoid_frame_sets_hips_translation_without_accumulation() {
        let document = VrmDocument {
            humanoid: Humanoid {
                bones: [(
                    HumanBoneName::Hips,
                    HumanBone {
                        node: NodeRef(0),
                        rest: Transform::default(),
                    },
                )]
                .into_iter()
                .collect(),
            },
            ..VrmDocument::default()
        };
        let frame = VrmAnimationFrame {
            hips_translation: Some(Vec3::new(1.0, 2.0, 3.0)),
            ..VrmAnimationFrame::default()
        };
        let mut mock = Mock::default();

        apply_humanoid_frame(&mut mock, &document, &frame).unwrap();
        apply_humanoid_frame(&mut mock, &document, &frame).unwrap();

        assert!(mock.translations.is_empty());
        assert_eq!(mock.local_sets.len(), 2);
        assert!(
            mock.local_sets
                .iter()
                .all(|(node, transform)| *node == NodeRef(0)
                    && transform.translation == Vec3::new(1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn first_person_annotations_apply_visibility() {
        let document = VrmDocument {
            first_person: Feature::Present(FirstPerson {
                mesh_annotations: vec![
                    FirstPersonMeshAnnotation {
                        node: NodeRef(1),
                        kind: FirstPersonAnnotation::FirstPersonOnly,
                    },
                    FirstPersonMeshAnnotation {
                        node: NodeRef(2),
                        kind: FirstPersonAnnotation::ThirdPersonOnly,
                    },
                    FirstPersonMeshAnnotation {
                        node: NodeRef(3),
                        kind: FirstPersonAnnotation::Both,
                    },
                ],
            }),
            ..VrmDocument::default()
        };
        let mut mock = Mock::default();

        apply_first_person_annotations(&mut mock, &document, ViewMode::FirstPerson).unwrap();

        assert_eq!(
            mock.visibility,
            vec![(NodeRef(1), true), (NodeRef(2), false), (NodeRef(3), true)]
        );
    }

    #[test]
    fn first_person_auto_hides_head_subtree_in_first_person() {
        let document = VrmDocument {
            humanoid: Humanoid {
                bones: [(
                    HumanBoneName::Head,
                    HumanBone {
                        node: NodeRef(10),
                        rest: Transform::default(),
                    },
                )]
                .into_iter()
                .collect(),
            },
            first_person: Feature::Present(FirstPerson {
                mesh_annotations: vec![
                    FirstPersonMeshAnnotation {
                        node: NodeRef(12),
                        kind: FirstPersonAnnotation::Auto,
                    },
                    FirstPersonMeshAnnotation {
                        node: NodeRef(20),
                        kind: FirstPersonAnnotation::Auto,
                    },
                ],
            }),
            ..VrmDocument::default()
        };
        let mut mock = Mock {
            parents: [
                (NodeRef(11), NodeRef(10)),
                (NodeRef(12), NodeRef(11)),
                (NodeRef(20), NodeRef(0)),
            ]
            .into_iter()
            .collect(),
            ..Mock::default()
        };

        apply_first_person_annotations(&mut mock, &document, ViewMode::FirstPerson).unwrap();

        assert_eq!(
            mock.visibility,
            vec![(NodeRef(12), false), (NodeRef(20), true)]
        );
    }

    #[test]
    fn first_person_auto_keeps_head_subtree_visible_in_third_person() {
        let document = VrmDocument {
            humanoid: Humanoid {
                bones: [(
                    HumanBoneName::Head,
                    HumanBone {
                        node: NodeRef(10),
                        rest: Transform::default(),
                    },
                )]
                .into_iter()
                .collect(),
            },
            first_person: Feature::Present(FirstPerson {
                mesh_annotations: vec![FirstPersonMeshAnnotation {
                    node: NodeRef(10),
                    kind: FirstPersonAnnotation::Auto,
                }],
            }),
            ..VrmDocument::default()
        };
        let mut mock = Mock::default();

        apply_first_person_annotations(&mut mock, &document, ViewMode::ThirdPerson).unwrap();

        assert_eq!(mock.visibility, vec![(NodeRef(10), true)]);
    }

    #[test]
    fn headless_mesh_plan_removes_triangles_weighted_to_erase_joints() {
        let influences = vec![
            SkinVertexInfluence {
                joints: [0, 1, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
            SkinVertexInfluence {
                joints: [1, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
            SkinVertexInfluence {
                joints: [0, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
            SkinVertexInfluence {
                joints: [0, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
        ];
        let plan = plan_headless_mesh(&[0, 2, 3, 0, 1, 2], &influences, &[1].into_iter().collect());

        assert_eq!(plan.indices, vec![0, 2, 3]);
        assert_eq!(plan.removed_triangles, 1);
    }

    #[test]
    fn first_person_headless_meshes_create_clone_for_head_weighted_mesh() {
        let document = VrmDocument {
            humanoid: Humanoid {
                bones: [(
                    HumanBoneName::Head,
                    HumanBone {
                        node: NodeRef(10),
                        rest: Transform::default(),
                    },
                )]
                .into_iter()
                .collect(),
            },
            ..VrmDocument::default()
        };
        let mut mock = Mock {
            parents: [(NodeRef(11), NodeRef(10))].into_iter().collect(),
            skinned_meshes: [(NodeRef(1), vec![7])].into_iter().collect(),
            mesh_joints: [(7, vec![NodeRef(0), NodeRef(11)])].into_iter().collect(),
            mesh_indices: [(7, vec![0, 1, 2, 2, 3, 0])].into_iter().collect(),
            mesh_influences: [(
                7,
                vec![
                    SkinVertexInfluence {
                        joints: [0, 0, 0, 0],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    },
                    SkinVertexInfluence {
                        joints: [1, 0, 0, 0],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    },
                    SkinVertexInfluence {
                        joints: [0, 0, 0, 0],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    },
                    SkinVertexInfluence {
                        joints: [0, 0, 0, 0],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    },
                ],
            )]
            .into_iter()
            .collect(),
            ..Mock::default()
        };

        apply_first_person_auto_headless_meshes(&mut mock, &document, NodeRef(1)).unwrap();

        assert_eq!(mock.third_person_meshes, vec![7]);
        assert_eq!(mock.headless_meshes[0].0, 7);
        assert_eq!(mock.headless_meshes[0].1.indices, vec![2, 3, 0]);
    }

    #[test]
    fn mtoon_pipeline_hints_apply_to_material_refs() {
        let document = VrmDocument {
            materials: vec![vrm_core::Material {
                name: Some("mtoon".to_owned()),
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    ..MtoonMaterial::default()
                }),
                ..vrm_core::Material::default()
            }],
            ..VrmDocument::default()
        };
        let mut mock = Mock::default();

        apply_mtoon_pipeline_hints(&mut mock, &document).unwrap();

        assert_eq!(mock.mtoon_passes.len(), 1);
        assert_eq!(mock.mtoon_passes[0].0, MaterialRef(0));
        assert!(matches!(
            mock.mtoon_passes[0].1.as_slice(),
            [MtoonPipelinePass::Base(_)]
        ));
    }

    #[test]
    fn mtoon_material_descriptors_include_pipeline_passes_and_parameters() {
        let document = VrmDocument {
            materials: vec![vrm_core::Material {
                name: Some("mtoon".to_owned()),
                khr_emissive_strength: Feature::Present(EmissiveStrength(2.0)),
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    outline_width_mode: vrm_core::OutlineWidthMode::WorldCoordinates,
                    outline_width_factor: 0.01,
                    base_color_factor: [1.0, 0.9, 0.8, 0.7],
                    emissive_factor: [0.1, 0.2, 0.3],
                    cutoff_factor: 0.42,
                    shade_color_factor: [0.5, 0.6, 0.7],
                    receive_shadow_rate_factor: 0.8,
                    shading_grade_rate_factor: 0.75,
                    light_color_attenuation_factor: 0.25,
                    matcap_factor: [0.4, 0.3, 0.2],
                    parametric_rim_color_factor: [0.2, 0.3, 0.4],
                    rim_lighting_mix_factor: 0.5,
                    parametric_rim_fresnel_power_factor: 2.0,
                    parametric_rim_lift_factor: 0.1,
                    outline_lighting_mix_factor: 0.6,
                    ..MtoonMaterial::default()
                }),
                ..vrm_core::Material::default()
            }],
            ..VrmDocument::default()
        };

        let descriptors = mtoon_material_descriptors(
            &document,
            MtoonMaterializationOptions {
                debug_mode: MtoonDebugMode::Lighting,
                v0_compat_shade: true,
            },
        );

        assert_eq!(descriptors.len(), 2);
        assert!(matches!(descriptors[0].pass, MtoonPipelinePass::Base(_)));
        assert!(matches!(descriptors[1].pass, MtoonPipelinePass::Outline(_)));
        assert_eq!(descriptors[0].material, MaterialRef(0));
        assert_eq!(descriptors[0].emissive_strength, EmissiveStrength(2.0));
        assert_eq!(descriptors[0].debug_mode, MtoonDebugMode::Lighting);
        assert!(descriptors[0].v0_compat_shade);
        assert_eq!(descriptors[0].base_color_factor, [1.0, 0.9, 0.8, 0.7]);
        assert_eq!(descriptors[0].emissive_factor, [0.1, 0.2, 0.3]);
        assert_eq!(descriptors[0].cutoff_factor, 0.42);
        assert_eq!(descriptors[0].shade_color_factor, [0.5, 0.6, 0.7]);
        assert_eq!(descriptors[0].receive_shadow_rate_factor, 0.8);
        assert_eq!(descriptors[0].shading_grade_rate_factor, 0.75);
        assert_eq!(descriptors[0].light_color_attenuation_factor, 0.25);
        assert_eq!(descriptors[0].matcap_factor, [0.4, 0.3, 0.2]);
        assert_eq!(descriptors[0].parametric_rim_color_factor, [0.2, 0.3, 0.4]);
        assert_eq!(descriptors[0].rim_lighting_mix_factor, 0.5);
        assert_eq!(descriptors[0].parametric_rim_fresnel_power_factor, 2.0);
        assert_eq!(descriptors[0].parametric_rim_lift_factor, 0.1);
        assert_eq!(descriptors[0].outline_lighting_mix_factor, 0.6);
    }

    #[test]
    fn hdr_emissive_multiplier_applies_to_material_refs() {
        let document = VrmDocument {
            materials: vec![
                vrm_core::Material::default(),
                vrm_core::Material {
                    name: Some("glow".to_owned()),
                    hdr_emissive_multiplier: Feature::Present(HdrEmissiveMultiplier(4.0)),
                    khr_emissive_strength: Feature::Present(EmissiveStrength(6.0)),
                    ..vrm_core::Material::default()
                },
            ],
            ..VrmDocument::default()
        };
        let mut mock = Mock::default();

        apply_hdr_emissive_multipliers(&mut mock, &document).unwrap();

        assert_eq!(mock.emissive_intensities, vec![(MaterialRef(1), 6.0)]);
    }

    #[test]
    fn vrm0_orientation_compensation_applies_root_transform() {
        let document = VrmDocument {
            kind: vrm_core::VrmKind::Vrm0Compat,
            compatibility: vrm_core::Compatibility {
                vrm0: Some(vrm_core::Vrm0Compatibility::default()),
            },
            ..VrmDocument::default()
        };
        let mut mock = Mock {
            local_transforms: [(
                NodeRef(0),
                Transform {
                    rotation: Quat::IDENTITY,
                    ..Transform::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Mock::default()
        };

        apply_vrm0_orientation_compensation(&mut mock, &document, NodeRef(0)).unwrap();

        assert_eq!(mock.local_sets.len(), 1);
        assert!((mock.local_sets[0].1.rotation * Vec3::Z).abs_diff_eq(Vec3::NEG_Z, 0.0001));
    }

    #[test]
    fn spring_colliders_are_collected_in_simulation_space() {
        let system = SpringBoneSystem {
            colliders: vec![vrm_core::SpringCollider {
                node: NodeRef(10),
                shape: ColliderShape::Sphere {
                    offset: Vec3::X,
                    radius: 0.5,
                    inside: false,
                },
            }],
            collider_groups: vec![vrm_core::SpringColliderGroup {
                name: None,
                colliders: vec![0],
            }],
            springs: vec![Spring {
                collider_groups: vec![0],
                center: Some(NodeRef(20)),
                ..Spring::default()
            }],
        };
        let mock = Mock {
            world_transforms: [
                (
                    NodeRef(10),
                    Transform {
                        translation: Vec3::new(3.0, 0.0, 0.0),
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
                (
                    NodeRef(20),
                    Transform {
                        translation: Vec3::new(1.0, 0.0, 0.0),
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Mock::default()
        };

        let colliders = collect_spring_colliders(&mock, &system, &system.springs[0]).unwrap();

        assert_eq!(
            colliders,
            vec![ColliderShape::Sphere {
                offset: Vec3::new(3.0, 0.0, 0.0),
                radius: 0.5,
                inside: false,
            }]
        );
    }

    #[test]
    fn node_constraints_apply_solver_output_to_destination() {
        let source_rotation = Quat::from_rotation_y(0.5);
        let mut mock = Mock {
            local_transforms: [(
                NodeRef(2),
                Transform {
                    rotation: source_rotation,
                    ..Transform::default()
                },
            )]
            .into_iter()
            .collect(),
            constraint_rest: [(
                (NodeRef(1), NodeRef(2)),
                ConstraintRestState {
                    destination_rest_rotation: Quat::IDENTITY,
                    source_rest_rotation: Quat::IDENTITY,
                },
            )]
            .into_iter()
            .collect(),
            ..Mock::default()
        };
        let constraints = vec![NodeConstraint {
            destination: NodeRef(1),
            source: NodeRef(2),
            kind: ConstraintKind::Rotation,
            weight: 1.0,
        }];

        apply_node_constraints(&mut mock, &constraints).unwrap();

        assert_eq!(mock.rotations, vec![(NodeRef(1), source_rotation)]);
    }

    #[test]
    fn constraint_rest_map_captures_initial_rotations() {
        let destination_rotation = Quat::from_rotation_x(0.25);
        let source_rotation = Quat::from_rotation_z(0.5);
        let mock = Mock {
            local_transforms: [
                (
                    NodeRef(1),
                    Transform {
                        rotation: destination_rotation,
                        ..Transform::default()
                    },
                ),
                (
                    NodeRef(2),
                    Transform {
                        rotation: source_rotation,
                        ..Transform::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Mock::default()
        };
        let constraints = vec![NodeConstraint {
            destination: NodeRef(1),
            source: NodeRef(2),
            kind: ConstraintKind::Rotation,
            weight: 1.0,
        }];

        let rest = ConstraintRestMap::capture(&mock, &constraints).unwrap();
        let captured = rest.get(NodeRef(1), NodeRef(2)).unwrap();

        assert_eq!(captured.destination_rest_rotation, destination_rotation);
        assert_eq!(captured.source_rest_rotation, source_rotation);
    }

    #[test]
    fn spring_tail_is_applied_as_joint_rotation() {
        let mut mock = Mock {
            parents: [(NodeRef(2), NodeRef(1))].into_iter().collect(),
            local_transforms: [(
                NodeRef(2),
                Transform {
                    rotation: Quat::IDENTITY,
                    ..Transform::default()
                },
            )]
            .into_iter()
            .collect(),
            world_transforms: [(
                NodeRef(1),
                Transform {
                    translation: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                },
            )]
            .into_iter()
            .collect(),
            ..Mock::default()
        };

        apply_spring_joint_tail(&mut mock, NodeRef(2), Vec3::Y, Vec3::X).unwrap();

        assert_eq!(mock.rotations.len(), 1);
        assert_eq!(mock.rotations[0].0, NodeRef(2));
        assert!((mock.rotations[0].1 * Vec3::Y).abs_diff_eq(Vec3::X, 0.0001));
    }

    #[test]
    fn spring_bone_system_steps_particles_and_writes_rotations() {
        let system = SpringBoneSystem {
            springs: vec![Spring {
                joints: vec![vrm_core::SpringJoint {
                    node: NodeRef(2),
                    stiffness: 0.0,
                    gravity_power: 1.0,
                    gravity_dir: Vec3::X,
                    drag_force: 1.0,
                    ..vrm_core::SpringJoint::default()
                }],
                ..Spring::default()
            }],
            ..SpringBoneSystem::default()
        };
        let mut state =
            SpringRuntimeState::from_system(&system, |_, _, _| SpringParticleState::default());
        let mut mock = Mock {
            parents: [(NodeRef(3), NodeRef(2)), (NodeRef(2), NodeRef(1))]
                .into_iter()
                .collect(),
            local_transforms: [(
                NodeRef(2),
                Transform {
                    rotation: Quat::IDENTITY,
                    ..Transform::default()
                },
            )]
            .into_iter()
            .collect(),
            world_transforms: [
                (
                    NodeRef(1),
                    Transform {
                        translation: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
                (
                    NodeRef(2),
                    Transform {
                        translation: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
                (
                    NodeRef(3),
                    Transform {
                        translation: Vec3::Y,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Mock::default()
        };

        step_spring_bone_system(&mut mock, &system, &mut state, DeltaTime(1.0)).unwrap();

        assert_eq!(mock.rotations.len(), 1);
        assert_eq!(mock.rotations[0].0, NodeRef(2));
        assert!(mock.rotations[0].1 * Vec3::Y != Vec3::Y);
    }

    #[test]
    fn spring_rest_map_captures_sparse_chain_and_center_state() {
        let system = SpringBoneSystem {
            springs: vec![Spring {
                joints: vec![
                    vrm_core::SpringJoint {
                        node: NodeRef(2),
                        ..vrm_core::SpringJoint::default()
                    },
                    vrm_core::SpringJoint {
                        node: NodeRef(4),
                        ..vrm_core::SpringJoint::default()
                    },
                    vrm_core::SpringJoint {
                        node: NodeRef(5),
                        ..vrm_core::SpringJoint::default()
                    },
                ],
                center: Some(NodeRef(10)),
                ..Spring::default()
            }],
            ..SpringBoneSystem::default()
        };
        let mock = Mock {
            local_transforms: [
                (NodeRef(2), Transform::default()),
                (
                    NodeRef(4),
                    Transform {
                        translation: Vec3::Z,
                        ..Transform::default()
                    },
                ),
                (
                    NodeRef(5),
                    Transform {
                        translation: Vec3::X * 2.0,
                        ..Transform::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            world_transforms: [
                (NodeRef(2), Transform::default()),
                (
                    NodeRef(4),
                    Transform {
                        translation: Vec3::Y * 2.0,
                        ..Transform::default()
                    },
                ),
                (
                    NodeRef(5),
                    Transform {
                        translation: Vec3::Y * 3.0,
                        ..Transform::default()
                    },
                ),
                (
                    NodeRef(10),
                    Transform {
                        translation: Vec3::Y,
                        ..Transform::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Mock::default()
        };

        let rest = SpringRestMap::capture(&mock, &system).unwrap();
        let first = rest.get(0, 0).unwrap();
        let final_joint = rest.get(0, 2).unwrap();

        assert_eq!(first.child, Some(NodeRef(4)));
        assert!(
            first
                .rest
                .initial_local_child_position
                .abs_diff_eq(Vec3::Z, 0.0001)
        );
        assert!(
            first
                .initial_center_state
                .current_tail
                .abs_diff_eq(Vec3::Z - Vec3::Y, 0.0001)
        );
        assert!(
            final_joint
                .rest
                .initial_local_child_position
                .abs_diff_eq(Vec3::X * 0.07, 0.0001)
        );
    }

    #[test]
    fn spring_bone_system_parity_steps_center_state_and_writes_local_rotation() {
        let system = SpringBoneSystem {
            springs: vec![Spring {
                joints: vec![vrm_core::SpringJoint {
                    node: NodeRef(2),
                    stiffness: 0.0,
                    gravity_power: 1.0,
                    gravity_dir: Vec3::X,
                    drag_force: 1.0,
                    ..vrm_core::SpringJoint::default()
                }],
                ..Spring::default()
            }],
            ..SpringBoneSystem::default()
        };
        let mut mock = Mock {
            parents: [(NodeRef(3), NodeRef(2)), (NodeRef(2), NodeRef(1))]
                .into_iter()
                .collect(),
            local_transforms: [(
                NodeRef(2),
                Transform {
                    rotation: Quat::IDENTITY,
                    ..Transform::default()
                },
            )]
            .into_iter()
            .collect(),
            world_transforms: [
                (NodeRef(1), Transform::default()),
                (NodeRef(2), Transform::default()),
                (
                    NodeRef(3),
                    Transform {
                        translation: Vec3::Y,
                        ..Transform::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Mock::default()
        };
        let rest = SpringRestMap::capture(&mock, &system).unwrap();
        let mut state = rest.runtime_state(&system);

        step_spring_bone_system_parity(&mut mock, &system, &rest, &mut state, DeltaTime(1.0))
            .unwrap();

        assert_eq!(mock.rotations.len(), 1);
        assert_eq!(mock.rotations[0].0, NodeRef(2));
        assert!(mock.rotations[0].1 * Vec3::Y != Vec3::Y);
        assert_ne!(state.get(0, 0).unwrap().current_tail, Vec3::Y);
    }

    #[test]
    fn spring_bone_system_parity_zero_delta_is_noop() {
        let system = SpringBoneSystem {
            springs: vec![Spring {
                joints: vec![vrm_core::SpringJoint {
                    node: NodeRef(2),
                    ..vrm_core::SpringJoint::default()
                }],
                ..Spring::default()
            }],
            ..SpringBoneSystem::default()
        };
        let mut mock = Mock {
            parents: [(NodeRef(3), NodeRef(2))].into_iter().collect(),
            local_transforms: [
                (NodeRef(2), Transform::default()),
                (
                    NodeRef(3),
                    Transform {
                        translation: Vec3::Y,
                        ..Transform::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            world_transforms: [
                (NodeRef(2), Transform::default()),
                (
                    NodeRef(3),
                    Transform {
                        translation: Vec3::Y,
                        ..Transform::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Mock::default()
        };
        let rest = SpringRestMap::capture(&mock, &system).unwrap();
        let mut state = rest.runtime_state(&system);

        step_spring_bone_system_parity(&mut mock, &system, &rest, &mut state, DeltaTime(0.0))
            .unwrap();

        assert!(mock.rotations.is_empty());
        assert_eq!(state.get(0, 0).unwrap().current_tail, Vec3::Y);
    }

    #[test]
    #[ignore = "requires local external fixtures; set VRM_RS_FIXTURE_DIR"]
    fn spring_parity_rest_map_captures_external_fixture_scenes() {
        let fixture_dir = std::env::var_os("VRM_RS_FIXTURE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(".external-fixtures/official"));
        let mut checked = 0;

        for path in fixture_files_under(&fixture_dir) {
            let is_vrm = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("vrm"));
            if !is_vrm {
                continue;
            }
            let Ok(loaded) = vrm_io::load_vrm_from_path(&path) else {
                continue;
            };
            let document = loaded.model().document();
            let Feature::Present(system) = &document.spring_bone else {
                continue;
            };
            let mut scene = FixtureScene::new(loaded.scene().clone());
            let rest = SpringRestMap::capture(&scene, system).unwrap_or_else(|err| {
                panic!(
                    "failed to capture spring rest for {}: {err:?}",
                    path.display()
                )
            });
            let mut state = rest.runtime_state(system);

            step_spring_bone_system_parity(
                &mut scene,
                system,
                &rest,
                &mut state,
                DeltaTime(1.0 / 60.0),
            )
            .unwrap_or_else(|err| panic!("failed to step spring for {}: {err:?}", path.display()));

            let joint_count: usize = system
                .springs
                .iter()
                .map(|spring| spring.joints.len())
                .sum();
            assert!(
                scene.rotations.len() <= joint_count,
                "fixture wrote more rotations than spring joints: {}",
                path.display()
            );
            checked += 1;
        }

        assert!(
            checked > 0,
            "no external VRM fixture with spring bone found in {}",
            fixture_dir.display()
        );
    }

    #[test]
    #[ignore = "requires three-vrm golden JSON; set VRM_RS_THREE_VRM_GOLDEN"]
    fn spring_parity_matches_three_vrm_golden_rotations() {
        let (golden_path, golden) = load_three_vrm_golden();
        compare_spring_golden(&golden_path, &golden);
    }

    #[test]
    #[ignore = "requires three-vrm golden JSON files; set VRM_RS_THREE_VRM_GOLDEN_DIR"]
    fn spring_parity_matches_three_vrm_golden_directory() {
        let golden_dir = std::env::var_os("VRM_RS_THREE_VRM_GOLDEN_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(".external-fixtures/golden"));
        let mut checked = 0;
        for entry in std::fs::read_dir(&golden_dir)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", golden_dir.display()))
        {
            let path = entry.unwrap().path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let golden: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&path)
                    .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display())),
            )
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
            if golden["springJoints"].as_array().is_none_or(Vec::is_empty) {
                continue;
            }
            compare_spring_golden(&path, &golden);
            checked += 1;
        }
        assert!(
            checked >= 2,
            "expected at least Seed-san and a collider-heavy spring golden in {}",
            golden_dir.display()
        );
    }

    fn compare_spring_golden(golden_path: &std::path::Path, golden: &serde_json::Value) {
        let tolerance = spring_golden_tolerance(golden_path);
        let report = spring_golden_report(golden_path, golden, tolerance);
        assert!(
            report.compared_rotations > 0,
            "golden did not contain stable spring joints"
        );
        assert!(
            report.max_tail_delta <= tolerance.tail,
            "{} max center tail delta {} exceeded {}",
            golden_path.display(),
            report.max_tail_delta,
            tolerance.tail
        );
        assert!(
            report.max_rotation_delta <= tolerance.rotation,
            "{} max rotation delta {} exceeded {}",
            golden_path.display(),
            report.max_rotation_delta,
            tolerance.rotation
        );
    }

    #[derive(Clone, Copy, Debug)]
    struct SpringGoldenTolerance {
        tail: f32,
        rotation: f32,
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct SpringGoldenReport {
        compared_rotations: usize,
        max_tail_delta: f32,
        max_rotation_delta: f32,
    }

    fn spring_golden_tolerance(golden_path: &std::path::Path) -> SpringGoldenTolerance {
        let file_name = golden_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if file_name.contains("Constraint") {
            SpringGoldenTolerance {
                tail: 0.003,
                rotation: 0.0015,
            }
        } else {
            SpringGoldenTolerance {
                tail: 0.001,
                rotation: 0.0015,
            }
        }
    }

    fn spring_golden_report(
        golden_path: &std::path::Path,
        golden: &serde_json::Value,
        tolerance: SpringGoldenTolerance,
    ) -> SpringGoldenReport {
        let fixture = golden["fixture"]
            .as_str()
            .unwrap_or_else(|| panic!("golden fixture is missing in {}", golden_path.display()));
        let delta = golden["delta"]
            .as_f64()
            .unwrap_or_else(|| panic!("golden delta is missing in {}", golden_path.display()))
            as f32;
        let loaded = vrm_io::load_vrm_from_path(fixture)
            .unwrap_or_else(|err| panic!("failed to load golden fixture {fixture}: {err:?}"));
        let document = loaded.model().document();
        let system = document
            .spring_bone
            .as_ref()
            .expect("golden fixture must have spring bone");
        let mut scene = FixtureScene::new(loaded.scene().clone());
        let rest = SpringRestMap::capture(&scene, system).unwrap();
        let mut state = rest.runtime_state(system);

        let mut report = SpringGoldenReport::default();
        for frame in golden_frames(golden) {
            scene.rotations.clear();
            step_spring_bone_system_parity(&mut scene, system, &rest, &mut state, DeltaTime(delta))
                .unwrap();
            let actual = scene.rotations.iter().copied().collect::<HashMap<_, _>>();
            let actual_tails = center_tail_map(system, &state);
            let frame_index = frame["frame"].as_u64().unwrap_or(1);
            for joint in frame["springJoints"]
                .as_array()
                .expect("golden frame springJoints must be an array")
            {
                let node = NodeRef(
                    joint["node"]
                        .as_u64()
                        .unwrap_or_else(|| panic!("golden joint node is missing: {joint}"))
                        as usize,
                );
                if let Some(expected_tail) = joint
                    .get("centerTail")
                    .and_then(|value| value.as_array())
                    .map(|values| vec3_from_json_array(values))
                {
                    let actual_tail = actual_tails
                        .get(&node)
                        .copied()
                        .unwrap_or_else(|| panic!("node {} has no center tail state", node.0));
                    let tail_delta = vec3_component_delta(actual_tail, expected_tail);
                    report.max_tail_delta = report.max_tail_delta.max(tail_delta);
                    assert!(
                        tail_delta <= tolerance.tail,
                        "frame {frame_index}, node {} center tail mismatch: actual={actual_tail:?} expected={expected_tail:?}",
                        node.0
                    );
                }
                if vec3_len_from_json(&joint["initialLocalChildPosition"]) <= 0.001 {
                    continue;
                }
                let expected = quat_from_json(&joint["localRotation"]);
                let actual = actual
                    .get(&node)
                    .copied()
                    .unwrap_or_else(|| panic!("node {} was not written by spring parity", node.0));
                let rotation_delta = quat_component_delta(actual, expected);
                report.max_rotation_delta = report.max_rotation_delta.max(rotation_delta);
                assert!(
                    rotation_delta <= tolerance.rotation,
                    "frame {frame_index}, node {} rotation mismatch: actual={actual:?} expected={expected:?}",
                    node.0
                );
                report.compared_rotations += 1;
            }
        }
        report
    }

    fn vec3_component_delta(actual: Vec3, expected: Vec3) -> f32 {
        actual
            .to_array()
            .into_iter()
            .zip(expected.to_array())
            .map(|(actual, expected)| (actual - expected).abs())
            .fold(0.0, f32::max)
    }

    fn quat_component_delta(actual: Quat, expected: Quat) -> f32 {
        quat_component_delta_same_sign(actual, expected)
            .min(quat_component_delta_same_sign(actual, -expected))
    }

    fn quat_component_delta_same_sign(actual: Quat, expected: Quat) -> f32 {
        let actual = actual.to_array();
        let expected = expected.to_array();
        actual
            .into_iter()
            .zip(expected)
            .map(|(actual, expected)| (actual - expected).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    #[ignore = "requires three-vrm golden JSON; set VRM_RS_THREE_VRM_GOLDEN"]
    fn humanoid_pose_matches_three_vrm_golden_rest_state() {
        let (golden_path, golden) = load_three_vrm_golden();
        let fixture = golden["fixture"]
            .as_str()
            .unwrap_or_else(|| panic!("golden fixture is missing in {}", golden_path.display()));
        let humanoid = golden["humanoid"].as_object().unwrap_or_else(|| {
            panic!(
                "golden humanoid snapshot is missing in {}",
                golden_path.display()
            )
        });
        let loaded = vrm_io::load_vrm_from_path(fixture)
            .unwrap_or_else(|err| panic!("failed to load golden fixture {fixture}: {err:?}"));
        let document = loaded.model().document();
        let scene = FixtureScene::new(loaded.scene().clone());
        let rig = HumanoidPoseRig::capture(&scene, document).unwrap();

        assert_pose_matches_json(
            rig.raw_rest_pose(),
            &humanoid["rawRestPose"],
            PoseTolerance::default(),
            "rawRestPose",
        );
        assert_pose_matches_json(
            &rig.get_raw_pose(&scene).unwrap(),
            &humanoid["rawPose"],
            PoseTolerance::default(),
            "rawPose",
        );
        assert_pose_matches_json(
            rig.normalized_rest_pose(),
            &humanoid["normalizedRestPose"],
            PoseTolerance::default(),
            "normalizedRestPose",
        );
        assert_pose_matches_json(
            &rig.get_normalized_pose(),
            &humanoid["normalizedPose"],
            PoseTolerance::default(),
            "normalizedPose",
        );
    }

    #[test]
    #[ignore = "requires three-vrm golden JSON; set VRM_RS_THREE_VRM_GOLDEN"]
    fn humanoid_pose_writeback_matches_three_vrm_golden() {
        let (golden_path, golden) = load_three_vrm_golden();
        let fixture = golden["fixture"]
            .as_str()
            .unwrap_or_else(|| panic!("golden fixture is missing in {}", golden_path.display()));
        let loaded = vrm_io::load_vrm_from_path(fixture)
            .unwrap_or_else(|err| panic!("failed to load golden fixture {fixture}: {err:?}"));
        let document = loaded.model().document();
        let tolerance = PoseTolerance {
            translation: 0.0005,
            rotation_radians: 0.0005,
        };
        let raw_scenario = golden_pose_scenario(&golden, "rawWriteback");
        let mut raw_scene = FixtureScene::new(loaded.scene().clone());
        let raw_rig = HumanoidPoseRig::capture(&raw_scene, document).unwrap();
        let raw_input: RawPose = pose_from_json(&raw_scenario["inputPose"]);
        raw_rig.set_raw_pose(&mut raw_scene, &raw_input).unwrap();

        assert_pose_matches_json(
            &raw_rig.get_raw_pose(&raw_scene).unwrap(),
            &raw_scenario["expected"]["rawPose"],
            tolerance,
            "rawWriteback.rawPose",
        );
        assert_pose_matches_json(
            &raw_rig.get_raw_absolute_pose(&raw_scene).unwrap(),
            &raw_scenario["expected"]["rawAbsolutePose"],
            tolerance,
            "rawWriteback.rawAbsolutePose",
        );

        let normalized_scenario = golden_pose_scenario(&golden, "normalizedWriteback");
        let mut normalized_scene = FixtureScene::new(loaded.scene().clone());
        let mut normalized_rig = HumanoidPoseRig::capture(&normalized_scene, document).unwrap();
        let normalized_input: vrm_core::NormalizedPose =
            pose_from_json(&normalized_scenario["inputPose"]);
        normalized_rig.set_normalized_pose(&normalized_input);
        normalized_rig
            .apply_normalized_to_raw(&mut normalized_scene)
            .unwrap();

        assert_pose_matches_json(
            &normalized_rig.get_normalized_pose(),
            &normalized_scenario["inputPose"],
            tolerance,
            "normalizedWriteback.normalizedPose",
        );
        assert_pose_matches_json(
            &normalized_rig
                .get_raw_absolute_pose(&normalized_scene)
                .unwrap(),
            &normalized_scenario["expected"]["rawAbsolutePose"],
            tolerance,
            "normalizedWriteback.rawAbsolutePose",
        );
    }

    #[test]
    #[ignore = "requires three-vrm VRMA golden JSON; set VRM_RS_THREE_VRM_VRMA_GOLDEN"]
    fn vrma_application_matches_three_vrm_golden() {
        let (golden_path, golden) = load_three_vrm_vrma_golden();
        compare_vrma_golden(&golden_path, &golden);
    }

    #[test]
    #[ignore = "requires three-vrm VRMA golden JSON files; set VRM_RS_THREE_VRM_VRMA_GOLDEN_DIR"]
    fn vrma_application_matches_three_vrm_golden_directory() {
        let golden_dir = std::env::var_os("VRM_RS_THREE_VRM_VRMA_GOLDEN_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(".external-fixtures/golden"));
        let mut checked = 0;
        for entry in std::fs::read_dir(&golden_dir)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", golden_dir.display()))
        {
            let path = entry.unwrap().path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let golden: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&path)
                    .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display())),
            )
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
            if golden["vrma"].as_str().is_none() {
                continue;
            }
            compare_vrma_golden(&path, &golden);
            checked += 1;
        }
        assert!(
            checked > 0,
            "expected at least one VRMA golden in {}",
            golden_dir.display()
        );
    }

    fn compare_vrma_golden(golden_path: &std::path::Path, golden: &serde_json::Value) {
        let fixture = golden["fixture"].as_str().unwrap_or_else(|| {
            panic!(
                "VRMA golden fixture is missing in {}",
                golden_path.display()
            )
        });
        let vrma = golden["vrma"].as_str().unwrap_or_else(|| {
            panic!(
                "VRMA golden clip path is missing in {}",
                golden_path.display()
            )
        });
        let loaded_vrm = vrm_io::load_vrm_from_path(fixture)
            .unwrap_or_else(|err| panic!("failed to load VRM fixture {fixture}: {err:?}"));
        let loaded_vrma = vrm_io::load_vrm_from_path(vrma)
            .unwrap_or_else(|err| panic!("failed to load VRMA fixture {vrma}: {err:?}"));
        let document = loaded_vrm.model().document();
        let animation = loaded_vrma
            .model()
            .document()
            .animation
            .as_ref()
            .unwrap_or_else(|| panic!("VRMA fixture has no animation: {vrma}"));
        let tolerance = PoseTolerance {
            translation: 0.001,
            rotation_radians: 0.0015,
        };
        let samples = golden["samples"]
            .as_array()
            .unwrap_or_else(|| panic!("VRMA golden samples must be an array"));

        for sample in samples {
            let time = sample["time"]
                .as_f64()
                .unwrap_or_else(|| panic!("VRMA golden sample missing time: {sample}"))
                as f32;
            let frame = sample_vrm_animation(animation, time);
            let mut scene = FixtureScene::new(loaded_vrm.scene().clone());
            let mut rig = HumanoidPoseRig::capture(&scene, document).unwrap();
            apply_vrma_animation_frame_with_look_at(&mut scene, &mut rig, document, &frame)
                .unwrap();
            scene.update_world_transforms().unwrap();
            assert_pose_matches_json(
                &rig.get_raw_absolute_pose(&scene).unwrap(),
                &sample["rawAbsolutePose"],
                tolerance,
                &format!("vrma@{time}.rawAbsolutePose"),
            );
            assert_pose_matches_json(
                &rig.get_normalized_pose_from_raw(&scene).unwrap(),
                &sample["normalizedPose"],
                tolerance,
                &format!("vrma@{time}.normalizedPose"),
            );
            assert_expression_weights_match(&frame, &sample["expressionWeights"], time);
            if let Some(expected) = sample["lookAtQuaternion"].as_array() {
                let expected = quat_from_json_array(expected);
                let actual = scene
                    .look_at_rotations
                    .last()
                    .copied()
                    .unwrap_or_else(|| panic!("VRMA sample at {time} did not write lookAt"));
                assert!(
                    actual.abs_diff_eq(expected, tolerance.rotation_radians)
                        || actual.abs_diff_eq(-expected, tolerance.rotation_radians),
                    "vrma@{time} lookAt mismatch: actual={actual:?} expected={expected:?}"
                );
            }
        }
        assert!(!samples.is_empty(), "VRMA golden did not contain samples");
    }

    fn assert_expression_weights_match(
        frame: &VrmAnimationFrame,
        expected: &serde_json::Value,
        time: f32,
    ) {
        let expected = expected
            .as_object()
            .unwrap_or_else(|| panic!("VRMA expressionWeights must be an object"));
        let actual_weights = frame_expression_weights(frame);
        let expected_keys = expected.keys().cloned().collect::<HashSet<_>>();
        let actual_keys = actual_weights.keys().cloned().collect::<HashSet<_>>();
        assert!(
            actual_keys.is_subset(&expected_keys),
            "vrma@{time} expression emitted unexpected keys: {:?}",
            actual_keys.difference(&expected_keys).collect::<Vec<_>>()
        );
        for (name, value) in expected {
            let expected_weight = value
                .as_f64()
                .unwrap_or_else(|| panic!("VRMA expression weight must be number: {value}"))
                as f32;
            let actual = actual_weights.get(name).copied().unwrap_or(0.0);
            assert!(
                (actual - expected_weight).abs() <= 0.0005,
                "vrma@{time} expression {name} mismatch: actual={actual} expected={expected_weight}"
            );
        }
    }

    fn frame_expression_weights(frame: &VrmAnimationFrame) -> HashMap<String, f32> {
        frame
            .preset_expressions
            .iter()
            .map(|(name, weight)| (expression_name_to_golden_key(name), *weight))
            .chain(
                frame
                    .custom_expressions
                    .iter()
                    .map(|(name, weight)| (name.clone(), *weight)),
            )
            .collect()
    }

    fn expression_name_to_golden_key(name: &ExpressionName) -> String {
        name.as_str().to_owned()
    }

    fn golden_pose_scenario<'a>(
        golden: &'a serde_json::Value,
        name: &str,
    ) -> &'a serde_json::Value {
        golden["humanoidPoseScenarios"]
            .as_array()
            .unwrap_or_else(|| panic!("golden humanoidPoseScenarios must be an array"))
            .iter()
            .find(|scenario| scenario["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("golden missing humanoid pose scenario {name}"))
    }

    fn load_three_vrm_golden() -> (std::path::PathBuf, serde_json::Value) {
        let golden_path = std::env::var_os("VRM_RS_THREE_VRM_GOLDEN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(".external-fixtures/golden/Seed-san.spring.json")
            });
        let golden: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&golden_path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", golden_path.display())),
        )
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", golden_path.display()));
        (golden_path, golden)
    }

    fn load_three_vrm_vrma_golden() -> (std::path::PathBuf, serde_json::Value) {
        let golden_path = std::env::var_os("VRM_RS_THREE_VRM_VRMA_GOLDEN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(".external-fixtures/golden/Seed-san.test-vrma.json")
            });
        let golden: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&golden_path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", golden_path.display())),
        )
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", golden_path.display()));
        (golden_path, golden)
    }

    fn assert_pose_matches_json<Space, Basis>(
        actual: &vrm_core::HumanoidPose<Space, Basis>,
        expected: &serde_json::Value,
        tolerance: PoseTolerance,
        label: &str,
    ) {
        let expected = expected
            .as_object()
            .unwrap_or_else(|| panic!("{label} must be a pose object"));
        let mut compared = 0;
        for (bone_name, transform) in expected {
            let bone: HumanBoneName =
                serde_json::from_value(serde_json::Value::String(bone_name.clone()))
                    .unwrap_or_else(|err| {
                        panic!("{label} has unsupported bone name {bone_name}: {err}")
                    });
            let expected_transform = pose_transform_from_json(transform);
            let actual_transform = actual
                .get(&bone)
                .unwrap_or_else(|| panic!("{label} missing bone {bone_name}"));
            let translation_delta = actual_transform
                .translation
                .distance(expected_transform.translation);
            let rotation_matches = actual_transform
                .rotation
                .abs_diff_eq(expected_transform.rotation, tolerance.rotation_radians)
                || actual_transform
                    .rotation
                    .abs_diff_eq(-expected_transform.rotation, tolerance.rotation_radians);
            assert!(
                translation_delta <= tolerance.translation,
                "{label} {bone_name} translation mismatch: actual={:?} expected={:?}",
                actual_transform.translation,
                expected_transform.translation
            );
            assert!(
                rotation_matches,
                "{label} {bone_name} rotation mismatch: actual={:?} expected={:?}",
                actual_transform.rotation, expected_transform.rotation
            );
            compared += 1;
        }
        assert!(compared > 0, "{label} did not contain any bones");
    }

    fn pose_from_json<Space, Basis>(
        value: &serde_json::Value,
    ) -> vrm_core::HumanoidPose<Space, Basis> {
        let entries = value
            .as_object()
            .unwrap_or_else(|| panic!("pose must be an object: {value}"))
            .iter()
            .map(|(bone_name, transform)| {
                let bone = serde_json::from_value(serde_json::Value::String(bone_name.clone()))
                    .unwrap_or_else(|err| panic!("unsupported bone name {bone_name}: {err}"));
                (bone, pose_transform_from_json(transform))
            })
            .collect::<IndexMap<_, _>>();
        pose_from_iter(entries)
    }

    fn pose_transform_from_json(value: &serde_json::Value) -> vrm_core::PoseTransform {
        vrm_core::PoseTransform {
            translation: vec3_from_json_array(
                value["position"]
                    .as_array()
                    .unwrap_or_else(|| panic!("pose position must be an array: {value}")),
            ),
            rotation: quat_from_json(&value["rotation"]),
        }
    }

    fn center_tail_map(
        system: &SpringBoneSystem,
        state: &CenterSpringRuntimeState,
    ) -> HashMap<NodeRef, Vec3> {
        system
            .springs
            .iter()
            .enumerate()
            .flat_map(|(spring_index, spring)| {
                spring
                    .joints
                    .iter()
                    .enumerate()
                    .map(move |(joint_index, joint)| (spring_index, joint_index, joint))
            })
            .filter_map(|(spring_index, joint_index, joint)| {
                state
                    .get(spring_index, joint_index)
                    .map(|particle| (joint.node, particle.current_tail))
            })
            .collect()
    }

    fn golden_frames(golden: &serde_json::Value) -> Vec<&serde_json::Value> {
        if let Some(frames) = golden["frameSnapshots"].as_array() {
            return frames.iter().collect();
        }
        vec![golden]
    }

    fn quat_from_json(value: &serde_json::Value) -> Quat {
        quat_from_json_array(
            value
                .as_array()
                .unwrap_or_else(|| panic!("expected quaternion array, got {value}")),
        )
    }

    fn quat_from_json_array(values: &[serde_json::Value]) -> Quat {
        let values = values
            .iter()
            .map(|value| value.as_f64().expect("quaternion component must be number") as f32)
            .collect::<Vec<_>>();
        Quat::from_xyzw(values[0], values[1], values[2], values[3])
    }

    fn vec3_len_from_json(value: &serde_json::Value) -> f32 {
        vec3_from_json_array(
            value
                .as_array()
                .unwrap_or_else(|| panic!("expected vector array, got {value}")),
        )
        .length()
    }

    fn vec3_from_json_array(values: &[serde_json::Value]) -> Vec3 {
        let values = values
            .iter()
            .map(|value| value.as_f64().expect("vector component must be number") as f32)
            .collect::<Vec<_>>();
        Vec3::new(values[0], values[1], values[2])
    }

    #[test]
    fn fixture_file_discovery_recurses_for_external_adapter_tests() {
        let root = std::env::temp_dir().join(format!(
            "vrm-rs-adapter-fixture-discovery-{}",
            std::process::id()
        ));
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("top.vrm"), b"").unwrap();
        std::fs::write(nested.join("clip.vrma"), b"").unwrap();

        let mut files = fixture_files_under(&root)
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        files.sort();

        assert_eq!(files, vec!["clip.vrma", "top.vrm"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn fixture_files_under(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut result = Vec::new();
        collect_fixture_files(root, &mut result);
        result
    }

    fn collect_fixture_files(path: &std::path::Path, result: &mut Vec<std::path::PathBuf>) {
        if path.is_file() {
            result.push(path.to_owned());
            return;
        }

        let entries = std::fs::read_dir(path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for entry in entries {
            collect_fixture_files(&entry.unwrap().path(), result);
        }
    }

    #[test]
    fn runtime_driver_combines_tick_side_effects() {
        let document = VrmDocument {
            compatibility: vrm_core::Compatibility {
                vrm0: Some(vrm_core::Vrm0Compatibility::default()),
            },
            humanoid: Humanoid {
                bones: [(
                    HumanBoneName::Hips,
                    HumanBone {
                        node: NodeRef(1),
                        rest: Transform::default(),
                    },
                )]
                .into_iter()
                .collect(),
            },
            first_person: Feature::Present(FirstPerson {
                mesh_annotations: vec![FirstPersonMeshAnnotation {
                    node: NodeRef(8),
                    kind: FirstPersonAnnotation::FirstPersonOnly,
                }],
            }),
            expressions: Feature::Present(ExpressionSet {
                preset: [(
                    ExpressionName::Blink,
                    Expression {
                        binds: vec![ExpressionBind::MorphTarget {
                            node: NodeRef(8),
                            index: 0,
                            weight: 100.0,
                        }],
                        ..Expression::default()
                    },
                )]
                .into_iter()
                .collect(),
                custom: Default::default(),
            }),
            spring_bone: Feature::Present(SpringBoneSystem {
                springs: vec![Spring {
                    joints: vec![vrm_core::SpringJoint {
                        node: NodeRef(3),
                        stiffness: 0.0,
                        gravity_power: 1.0,
                        gravity_dir: Vec3::X,
                        drag_force: 1.0,
                        ..vrm_core::SpringJoint::default()
                    }],
                    ..Spring::default()
                }],
                ..SpringBoneSystem::default()
            }),
            ..VrmDocument::default()
        };
        let frame = VrmAnimationFrame {
            hips_translation: Some(Vec3::Y),
            preset_expressions: [(ExpressionName::Blink, 0.25)].into_iter().collect(),
            ..VrmAnimationFrame::default()
        };
        let events = RuntimeEvents {
            delta: DeltaTime(1.0),
            expressions: vec![AppliedExpression {
                name: "blink".to_owned(),
                effective_weight: 0.5,
                binds: vec![ExpressionBind::MorphTarget {
                    node: NodeRef(8),
                    index: 0,
                    weight: 100.0,
                }],
            }],
            constraints: vec![NodeConstraint {
                destination: NodeRef(2),
                source: NodeRef(4),
                kind: ConstraintKind::Rotation,
                weight: 1.0,
            }],
            springs: Vec::new(),
        };
        let mut spring_state =
            SpringRuntimeState::from_system(document.spring_bone.as_ref().unwrap(), |_, _, _| {
                SpringParticleState::default()
            });
        let source_rotation = Quat::from_rotation_y(0.5);
        let mut mock = Mock {
            parents: [(NodeRef(5), NodeRef(3)), (NodeRef(3), NodeRef(1))]
                .into_iter()
                .collect(),
            local_transforms: [
                (NodeRef(0), Transform::default()),
                (
                    NodeRef(3),
                    Transform {
                        rotation: Quat::IDENTITY,
                        ..Transform::default()
                    },
                ),
                (
                    NodeRef(4),
                    Transform {
                        rotation: source_rotation,
                        ..Transform::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            world_transforms: [
                (NodeRef(1), Transform::default()),
                (NodeRef(3), Transform::default()),
                (
                    NodeRef(5),
                    Transform {
                        translation: Vec3::Y,
                        ..Transform::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            constraint_rest: [(
                (NodeRef(2), NodeRef(4)),
                ConstraintRestState::new(Quat::IDENTITY, Quat::IDENTITY),
            )]
            .into_iter()
            .collect(),
            ..Mock::default()
        };

        let mut driver = VrmRuntimeDriver::new(&document)
            .with_root(NodeRef(0))
            .with_view_mode(ViewMode::FirstPerson)
            .with_animation_frame(&frame)
            .with_runtime_events(&events);
        driver.tick(&mut mock, Some(&mut spring_state)).unwrap();

        assert!(mock.translations.is_empty());
        assert!(mock.local_sets.iter().any(|(node, transform)| {
            *node == NodeRef(0) && (transform.rotation * Vec3::Z).abs_diff_eq(Vec3::NEG_Z, 0.0001)
        }));
        assert!(
            mock.local_sets
                .iter()
                .any(|(node, transform)| *node == NodeRef(1) && transform.translation == Vec3::Y)
        );
        assert_eq!(
            mock.morphs,
            vec![(NodeRef(8), 0, 25.0), (NodeRef(8), 0, 50.0)]
        );
        assert!(mock.rotations.iter().any(|(node, _)| *node == NodeRef(2)));
        assert!(mock.rotations.iter().any(|(node, _)| *node == NodeRef(3)));
        assert_eq!(mock.visibility, vec![(NodeRef(8), true)]);
    }

    #[test]
    fn runtime_driver_can_use_spring_parity_state() {
        let document = VrmDocument {
            spring_bone: Feature::Present(SpringBoneSystem {
                springs: vec![Spring {
                    joints: vec![vrm_core::SpringJoint {
                        node: NodeRef(2),
                        stiffness: 0.0,
                        gravity_power: 1.0,
                        gravity_dir: Vec3::X,
                        drag_force: 1.0,
                        ..vrm_core::SpringJoint::default()
                    }],
                    ..Spring::default()
                }],
                ..SpringBoneSystem::default()
            }),
            ..VrmDocument::default()
        };
        let events = RuntimeEvents {
            delta: DeltaTime(1.0),
            ..RuntimeEvents::default()
        };
        let mut mock = Mock {
            parents: [(NodeRef(3), NodeRef(2)), (NodeRef(2), NodeRef(1))]
                .into_iter()
                .collect(),
            local_transforms: [(NodeRef(2), Transform::default())].into_iter().collect(),
            world_transforms: [
                (NodeRef(1), Transform::default()),
                (NodeRef(2), Transform::default()),
                (
                    NodeRef(3),
                    Transform {
                        translation: Vec3::Y,
                        ..Transform::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Mock::default()
        };
        let system = document.spring_bone.as_ref().unwrap();
        let rest = SpringRestMap::capture(&mock, system).unwrap();
        let mut spring_state = rest.runtime_state(system);
        let mut driver = VrmRuntimeDriver::new(&document).with_runtime_events(&events);

        driver
            .tick_with_spring_parity(&mut mock, Some((&rest, &mut spring_state)))
            .unwrap();

        assert!(mock.world_updates >= 1);
        assert!(mock.rotations.iter().any(|(node, _)| *node == NodeRef(2)));
    }

    #[test]
    fn runtime_driver_applies_vrm0_orientation_once() {
        let document = VrmDocument {
            compatibility: vrm_core::Compatibility {
                vrm0: Some(vrm_core::Vrm0Compatibility::default()),
            },
            ..VrmDocument::default()
        };
        let mut mock = Mock {
            local_transforms: [(NodeRef(0), Transform::default())].into_iter().collect(),
            ..Mock::default()
        };
        let mut driver = VrmRuntimeDriver::new(&document).with_root(NodeRef(0));

        driver.tick(&mut mock, None).unwrap();
        driver.tick(&mut mock, None).unwrap();

        assert_eq!(
            mock.local_sets
                .iter()
                .filter(|(node, _)| *node == NodeRef(0))
                .count(),
            1
        );
    }
}
