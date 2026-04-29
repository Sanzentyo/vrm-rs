//! Traits for connecting `vrm-rs` runtime output to external engines.

use glam::{Quat, Vec3};
use std::collections::HashMap;
use thiserror::Error;
use vrm_core::{
    ColliderShape, ConstraintKind, ExpressionBind, ExpressionName, Feature, FirstPersonAnnotation,
    HumanBoneName, MaterialRef, MtoonPipelinePass, NodeConstraint, NodeRef, Spring,
    SpringBoneSystem, TextureRef, Transform, VrmDocument,
};
use vrm_runtime::{
    AimConstraintInput, AppliedExpression, ConstraintRestState, DeltaTime, RuntimeEvents,
    SpringJointSimulationInput, SpringParticleState, SpringRuntimeState, VrmAnimationFrame,
    collider_shape_in_simulation_space, solve_aim_constraint, solve_roll_constraint,
    solve_rotation_constraint, solve_spring_joint_rotation, step_spring_joint,
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

pub trait TextureResolver {
    type Texture;
    type Error;

    fn resolve_texture(&self, texture: TextureRef) -> Result<Self::Texture, Self::Error>;
}

pub trait VisibilityAccess {
    type Error;

    fn set_node_visible(&mut self, node: NodeRef, visible: bool) -> Result<(), Self::Error>;
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
        apply_hdr_emissive_multipliers(target, self.document)?;
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
    T: VisibilityAccess<Error = E>,
{
    let Some(first_person) = document.first_person.as_ref() else {
        return Ok(());
    };

    for annotation in &first_person.mesh_annotations {
        let visible = match (&annotation.kind, mode) {
            (FirstPersonAnnotation::Both | FirstPersonAnnotation::Auto, _) => true,
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

pub fn apply_hdr_emissive_multipliers<T, E>(
    target: &mut T,
    document: &VrmDocument,
) -> Result<(), AdapterError<E>>
where
    T: MaterialAccess<Error = E>,
{
    for (index, material) in document.materials.iter().enumerate() {
        if let Feature::Present(multiplier) = material.hdr_emissive_multiplier {
            target
                .set_emissive_intensity(MaterialRef(index), multiplier.emissive_intensity())
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
            let child_world = target
                .children(joint.node)
                .map_err(AdapterError::Target)?
                .first()
                .copied()
                .map(|child| target.world_transform(child).map_err(AdapterError::Target))
                .transpose()?;
            let (local_axis, bone_length) = spring_axis_and_length(joint_world, child_world);
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

fn spring_axis_and_length(joint_world: Transform, child_world: Option<Transform>) -> (Vec3, f32) {
    let Some(child_world) = child_world else {
        return (Vec3::Y, 1.0);
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
        Expression, ExpressionSet, Feature, FirstPerson, FirstPersonMeshAnnotation,
        HdrEmissiveMultiplier, HumanBone, Humanoid, MtoonMaterial, MtoonRenderQueue, RotationTrack,
        VrmAnimation, VrmDocument,
    };
    use vrm_runtime::sample_vrm_animation;

    #[derive(Default)]
    struct Mock {
        morphs: Vec<(NodeRef, usize, f32)>,
        rotations: Vec<(NodeRef, Quat)>,
        translations: Vec<(NodeRef, Vec3)>,
        local_sets: Vec<(NodeRef, Transform)>,
        mtoon_passes: Vec<(MaterialRef, Vec<MtoonPipelinePass>)>,
        emissive_intensities: Vec<(MaterialRef, f32)>,
        visibility: Vec<(NodeRef, bool)>,
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
            ..VrmAnimation::default()
        };
        let frame = sample_vrm_animation(&animation, 0.0);
        let mut mock = Mock::default();

        apply_animation_frame(&mut mock, &document, &frame).unwrap();

        assert_eq!(mock.rotations.len(), 1);
        assert_eq!(mock.rotations[0].0, NodeRef(1));
        assert!(mock.translations.is_empty());
        assert_eq!(mock.local_sets.len(), 1);
        assert_eq!(mock.local_sets[0].0, NodeRef(0));
        assert_eq!(mock.local_sets[0].1.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(mock.morphs, vec![(NodeRef(1), 0, 25.0)]);
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
    fn hdr_emissive_multiplier_applies_to_material_refs() {
        let document = VrmDocument {
            materials: vec![
                vrm_core::Material::default(),
                vrm_core::Material {
                    name: Some("glow".to_owned()),
                    hdr_emissive_multiplier: Feature::Present(HdrEmissiveMultiplier(4.0)),
                    ..vrm_core::Material::default()
                },
            ],
            ..VrmDocument::default()
        };
        let mut mock = Mock::default();

        apply_hdr_emissive_multipliers(&mut mock, &document).unwrap();

        assert_eq!(mock.emissive_intensities, vec![(MaterialRef(1), 4.0)]);
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
