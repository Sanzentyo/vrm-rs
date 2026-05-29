//! Renderer-agnostic runtime algorithms for VRM components.
//!
//! ```
//! use vrm_core::VrmDocument;
//! use vrm_runtime::{DeltaTime, Runtime};
//!
//! let document = VrmDocument::default();
//! let mut runtime = Runtime::from_document(&document);
//! let events = runtime.update(DeltaTime(1.0 / 60.0)).unwrap();
//!
//! assert_eq!(events.delta, DeltaTime(1.0 / 60.0));
//! ```

use glam::{Mat4, Quat, Vec3};
use indexmap::IndexMap;
use thiserror::Error;
use vrm_core::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeltaTime(pub f32);

#[derive(Clone, Debug, Default)]
pub struct Runtime {
    pub expression_manager: ExpressionManager,
    pub constraint_manager: ConstraintManager,
    pub spring_manager: SpringBoneManager,
}

impl Runtime {
    pub fn from_document(document: &VrmDocument) -> Self {
        Self {
            expression_manager: ExpressionManager::from_document(document),
            constraint_manager: ConstraintManager::new(document.node_constraints.clone()),
            spring_manager: SpringBoneManager::new(
                document.spring_bone.as_ref().cloned().unwrap_or_default(),
            ),
        }
    }

    pub fn update(&mut self, delta: DeltaTime) -> Result<RuntimeEvents, RuntimeError> {
        let expressions = self.expression_manager.update();
        let constraints = self.constraint_manager.update_order()?;
        let springs = self.spring_manager.update_order();
        Ok(RuntimeEvents {
            delta,
            expressions,
            constraints,
            springs,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuntimeEvents {
    pub delta: DeltaTime,
    pub expressions: Vec<AppliedExpression>,
    pub constraints: Vec<NodeConstraint>,
    pub springs: Vec<SpringJointStep>,
}

#[derive(Clone, Debug, Default)]
pub struct ExpressionManager {
    expressions: IndexMap<String, ManagedExpression>,
}

impl ExpressionManager {
    pub fn from_document(document: &VrmDocument) -> Self {
        let mut manager = Self::default();
        if let Feature::Present(expressions) = &document.expressions {
            for (name, expression) in &expressions.preset {
                manager.expressions.insert(
                    name.as_str().to_owned(),
                    ManagedExpression::new(expression.clone()),
                );
            }
            for (name, expression) in &expressions.custom {
                manager
                    .expressions
                    .insert(name.clone(), ManagedExpression::new(expression.clone()));
            }
        }
        manager
    }

    pub fn set_value(&mut self, name: impl AsRef<str>, weight: f32) {
        if let Some(expression) = self.expressions.get_mut(name.as_ref()) {
            expression.weight = weight.clamp(0.0, 1.0);
        }
    }

    pub fn value(&self, name: impl AsRef<str>) -> Option<f32> {
        self.expressions.get(name.as_ref()).map(|expr| expr.weight)
    }

    pub fn update(&self) -> Vec<AppliedExpression> {
        let multipliers = self.weight_multipliers();
        self.expressions
            .iter()
            .map(|(name, expression)| {
                let mut multiplier = 1.0;
                if is_blink(name) {
                    multiplier *= multipliers.blink;
                }
                if is_look_at(name) {
                    multiplier *= multipliers.look_at;
                }
                if is_mouth(name) {
                    multiplier *= multipliers.mouth;
                }
                AppliedExpression {
                    name: name.clone(),
                    effective_weight: expression.weight * multiplier,
                    binds: expression.expression.binds.clone(),
                }
            })
            .collect()
    }

    fn weight_multipliers(&self) -> WeightMultipliers {
        self.expressions
            .values()
            .fold(WeightMultipliers::default(), |mut acc, expression| {
                acc.blink -= expression
                    .expression
                    .override_blink
                    .amount(expression.weight);
                acc.look_at -= expression
                    .expression
                    .override_look_at
                    .amount(expression.weight);
                acc.mouth -= expression
                    .expression
                    .override_mouth
                    .amount(expression.weight);
                acc.saturate()
            })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ManagedExpression {
    expression: Expression,
    weight: f32,
}

impl ManagedExpression {
    fn new(expression: Expression) -> Self {
        Self {
            expression,
            weight: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppliedExpression {
    pub name: String,
    pub effective_weight: f32,
    pub binds: Vec<ExpressionBind>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WeightMultipliers {
    blink: f32,
    look_at: f32,
    mouth: f32,
}

impl Default for WeightMultipliers {
    fn default() -> Self {
        Self {
            blink: 1.0,
            look_at: 1.0,
            mouth: 1.0,
        }
    }
}

impl WeightMultipliers {
    fn saturate(mut self) -> Self {
        self.blink = self.blink.max(0.0);
        self.look_at = self.look_at.max(0.0);
        self.mouth = self.mouth.max(0.0);
        self
    }
}

fn is_blink(name: &str) -> bool {
    matches!(name, "blink" | "blinkLeft" | "blinkRight")
}

fn is_look_at(name: &str) -> bool {
    matches!(name, "lookUp" | "lookDown" | "lookLeft" | "lookRight")
}

fn is_mouth(name: &str) -> bool {
    matches!(name, "aa" | "ih" | "ou" | "ee" | "oh")
}

#[derive(Clone, Debug, Default)]
pub struct ConstraintManager {
    constraints: Vec<NodeConstraint>,
}

impl ConstraintManager {
    pub fn new(constraints: Vec<NodeConstraint>) -> Self {
        Self { constraints }
    }

    pub fn update_order(&self) -> Result<Vec<NodeConstraint>, RuntimeError> {
        let mut order = Vec::with_capacity(self.constraints.len());
        let mut visiting = Vec::new();
        let mut done = vec![false; self.constraints.len()];

        for index in 0..self.constraints.len() {
            self.visit(index, &mut visiting, &mut done, &mut order)?;
        }

        Ok(order)
    }

    fn visit(
        &self,
        index: usize,
        visiting: &mut Vec<usize>,
        done: &mut [bool],
        order: &mut Vec<NodeConstraint>,
    ) -> Result<(), RuntimeError> {
        if done[index] {
            return Ok(());
        }
        if visiting.contains(&index) {
            return Err(RuntimeError::CircularConstraint);
        }
        visiting.push(index);
        let dependency_source = self.constraints[index].source;
        for dep_index in
            self.constraints
                .iter()
                .enumerate()
                .filter_map(|(candidate, constraint)| {
                    (constraint.destination == dependency_source).then_some(candidate)
                })
        {
            self.visit(dep_index, visiting, done, order)?;
        }
        visiting.pop();
        done[index] = true;
        order.push(self.constraints[index].clone());
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstraintRestState {
    pub destination_rest_rotation: Quat,
    pub source_rest_rotation: Quat,
}

impl ConstraintRestState {
    pub fn new(destination_rest_rotation: Quat, source_rest_rotation: Quat) -> Self {
        Self {
            destination_rest_rotation,
            source_rest_rotation,
        }
    }
}

pub fn solve_rotation_constraint(
    state: ConstraintRestState,
    source_rotation: Quat,
    weight: f32,
) -> Quat {
    let src_delta = state.source_rest_rotation.inverse() * source_rotation;
    let target = state.destination_rest_rotation * src_delta;
    state
        .destination_rest_rotation
        .slerp(target, weight.clamp(0.0, 1.0))
}

pub fn solve_roll_constraint(
    state: ConstraintRestState,
    source_rotation: Quat,
    axis: Axis,
    weight: f32,
) -> Quat {
    let dst_rest = state.destination_rest_rotation;
    let quat_delta =
        dst_rest.inverse() * source_rotation * state.source_rest_rotation.inverse() * dst_rest;
    let axis = axis.unsigned_vector();
    let n1 = quat_delta * axis;
    let quat_from_to = Quat::from_rotation_arc(n1.normalize_or_zero(), axis);
    let target = dst_rest * quat_from_to * quat_delta;
    dst_rest.slerp(target.normalize(), weight.clamp(0.0, 1.0))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimConstraintInput {
    pub destination_rest_rotation: Quat,
    pub destination_world_position: Vec3,
    pub source_world_position: Vec3,
    pub destination_parent_world_rotation: Quat,
    pub axis: Axis,
    pub weight: f32,
}

pub fn solve_aim_constraint(input: AimConstraintInput) -> Quat {
    let parent = input.destination_parent_world_rotation;
    let inv_parent = parent.inverse();
    let a0 = parent * (input.destination_rest_rotation * input.axis.vector());
    let a1 = (input.source_world_position - input.destination_world_position).normalize_or_zero();
    let from_to = Quat::from_rotation_arc(a0.normalize_or_zero(), a1);
    let target = inv_parent * from_to * parent * input.destination_rest_rotation;
    input
        .destination_rest_rotation
        .slerp(target.normalize(), input.weight.clamp(0.0, 1.0))
}

trait RuntimeAxisExt {
    fn vector(self) -> Vec3;
    fn unsigned_vector(self) -> Vec3;
}

impl RuntimeAxisExt for Axis {
    fn vector(self) -> Vec3 {
        match self {
            Axis::PositiveX => Vec3::X,
            Axis::NegativeX => Vec3::NEG_X,
            Axis::PositiveY => Vec3::Y,
            Axis::NegativeY => Vec3::NEG_Y,
            Axis::PositiveZ => Vec3::Z,
            Axis::NegativeZ => Vec3::NEG_Z,
        }
    }

    fn unsigned_vector(self) -> Vec3 {
        match self {
            Axis::PositiveX | Axis::NegativeX => Vec3::X,
            Axis::PositiveY | Axis::NegativeY => Vec3::Y,
            Axis::PositiveZ | Axis::NegativeZ => Vec3::Z,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SpringBoneManager {
    system: SpringBoneSystem,
    particles: SpringRuntimeState,
}

impl SpringBoneManager {
    pub fn new(system: SpringBoneSystem) -> Self {
        let particles =
            SpringRuntimeState::from_system(&system, |_, _, _| SpringParticleState::default());
        Self { system, particles }
    }

    pub fn update_order(&self) -> Vec<SpringJointStep> {
        self.system
            .springs
            .iter()
            .enumerate()
            .flat_map(|(spring_index, spring)| {
                spring
                    .joints
                    .iter()
                    .enumerate()
                    .map(move |(joint_index, joint)| SpringJointStep {
                        spring_index,
                        joint_index,
                        node: joint.node,
                        gravity: joint.gravity_dir * joint.gravity_power,
                    })
            })
            .collect()
    }

    pub fn particles(&self) -> &SpringRuntimeState {
        &self.particles
    }

    pub fn particles_mut(&mut self) -> &mut SpringRuntimeState {
        &mut self.particles
    }

    pub fn system(&self) -> &SpringBoneSystem {
        &self.system
    }

    pub fn spring_colliders(&self, spring_index: usize) -> Vec<&SpringCollider> {
        let Some(spring) = self.system.springs.get(spring_index) else {
            return Vec::new();
        };

        spring
            .collider_groups
            .iter()
            .filter_map(|group_index| self.system.collider_groups.get(*group_index))
            .flat_map(|group| &group.colliders)
            .filter_map(|collider_index| self.system.colliders.get(*collider_index))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringJointStep {
    pub spring_index: usize,
    pub joint_index: usize,
    pub node: NodeRef,
    pub gravity: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringParticleState {
    pub current_tail: Vec3,
    pub previous_tail: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CenterSpringParticleState {
    pub current_tail: Vec3,
    pub previous_tail: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringJointRestState {
    pub initial_local_matrix: Mat4,
    pub initial_local_rotation: Quat,
    pub initial_local_child_position: Vec3,
    pub bone_axis: Vec3,
    pub initial_parent_world_rotation: Quat,
    pub initial_world_bone_axis: Vec3,
    pub initial_world_bone_length: f32,
}

impl SpringJointRestState {
    pub fn from_local_child(initial_local: Transform, initial_local_child_position: Vec3) -> Self {
        let bone_axis = initial_local_child_position.normalize_or(Vec3::Y);
        Self {
            initial_local_matrix: transform_matrix(initial_local),
            initial_local_rotation: initial_local.rotation,
            initial_local_child_position,
            bone_axis,
            initial_parent_world_rotation: Quat::IDENTITY,
            initial_world_bone_axis: bone_axis,
            initial_world_bone_length: initial_local_child_position.length(),
        }
    }

    pub fn with_initial_world_bone(
        mut self,
        parent_world_rotation: Quat,
        world_bone_axis: Vec3,
        world_bone_length: f32,
    ) -> Self {
        self.initial_parent_world_rotation = parent_world_rotation;
        self.initial_world_bone_axis = world_bone_axis.normalize_or(self.bone_axis);
        self.initial_world_bone_length = world_bone_length;
        self
    }

    pub fn vrm0_tail_fallback(initial_local: Transform) -> Self {
        Self::from_local_child(
            initial_local,
            initial_local.translation.normalize_or(Vec3::Y) * 0.07,
        )
    }
}

impl Default for SpringParticleState {
    fn default() -> Self {
        Self {
            current_tail: Vec3::ZERO,
            previous_tail: Vec3::ZERO,
        }
    }
}

impl Default for CenterSpringParticleState {
    fn default() -> Self {
        Self {
            current_tail: Vec3::ZERO,
            previous_tail: Vec3::ZERO,
        }
    }
}

impl SpringParticleState {
    pub fn at_rest(parent_position: Vec3, local_axis: Vec3, bone_length: f32) -> Self {
        let tail = parent_position + local_axis.normalize_or_zero() * bone_length;
        Self {
            current_tail: tail,
            previous_tail: tail,
        }
    }
}

impl CenterSpringParticleState {
    pub fn at_rest(center_tail: Vec3) -> Self {
        Self {
            current_tail: center_tail,
            previous_tail: center_tail,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpringRuntimeState {
    states: Vec<Vec<SpringParticleState>>,
    initial_states: Vec<Vec<SpringParticleState>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CenterSpringRuntimeState {
    states: Vec<Vec<CenterSpringParticleState>>,
    initial_states: Vec<Vec<CenterSpringParticleState>>,
}

impl SpringRuntimeState {
    pub fn from_system(
        system: &SpringBoneSystem,
        mut init: impl FnMut(usize, usize, &SpringJoint) -> SpringParticleState,
    ) -> Self {
        let states = system
            .springs
            .iter()
            .enumerate()
            .map(|(spring_index, spring)| {
                spring
                    .joints
                    .iter()
                    .enumerate()
                    .map(|(joint_index, joint)| init(spring_index, joint_index, joint))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        Self {
            initial_states: states.clone(),
            states,
        }
    }

    pub fn get(&self, spring_index: usize, joint_index: usize) -> Option<&SpringParticleState> {
        self.states
            .get(spring_index)
            .and_then(|spring| spring.get(joint_index))
    }

    pub fn get_mut(
        &mut self,
        spring_index: usize,
        joint_index: usize,
    ) -> Option<&mut SpringParticleState> {
        self.states
            .get_mut(spring_index)
            .and_then(|spring| spring.get_mut(joint_index))
    }

    pub fn reset(&mut self) {
        self.states.clone_from(&self.initial_states);
    }

    pub fn set_init_state(&mut self) {
        self.initial_states.clone_from(&self.states);
    }
}

impl CenterSpringRuntimeState {
    pub fn from_system(
        system: &SpringBoneSystem,
        mut init: impl FnMut(usize, usize, &SpringJoint) -> CenterSpringParticleState,
    ) -> Self {
        let states = system
            .springs
            .iter()
            .enumerate()
            .map(|(spring_index, spring)| {
                spring
                    .joints
                    .iter()
                    .enumerate()
                    .map(|(joint_index, joint)| init(spring_index, joint_index, joint))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        Self {
            initial_states: states.clone(),
            states,
        }
    }

    pub fn get(
        &self,
        spring_index: usize,
        joint_index: usize,
    ) -> Option<&CenterSpringParticleState> {
        self.states
            .get(spring_index)
            .and_then(|spring| spring.get(joint_index))
    }

    pub fn get_mut(
        &mut self,
        spring_index: usize,
        joint_index: usize,
    ) -> Option<&mut CenterSpringParticleState> {
        self.states
            .get_mut(spring_index)
            .and_then(|spring| spring.get_mut(joint_index))
    }

    pub fn reset(&mut self) {
        self.states.clone_from(&self.initial_states);
    }

    pub fn set_init_state(&mut self) {
        self.initial_states.clone_from(&self.states);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpringParticleStep<'a> {
    pub joint: &'a SpringJoint,
    pub parent_position: Vec3,
    pub parent_rotation: Quat,
    pub local_axis: Vec3,
    pub bone_length: f32,
    pub colliders: &'a [ColliderShape],
    pub delta: DeltaTime,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringJointSimulationInput<'a> {
    pub joint: &'a SpringJoint,
    pub parent_position: Vec3,
    pub parent_rotation: Quat,
    pub local_axis: Vec3,
    pub bone_length: f32,
    pub colliders: &'a [ColliderShape],
    pub delta: DeltaTime,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringJointParityInput<'a> {
    pub joint: &'a SpringJoint,
    pub rest: SpringJointRestState,
    pub parent_world: Transform,
    pub joint_world: Transform,
    pub child_world: Option<Transform>,
    pub center_world: Option<Transform>,
    pub colliders: &'a [ColliderShape],
    pub delta: DeltaTime,
}

impl<'a> From<SpringJointSimulationInput<'a>> for SpringParticleStep<'a> {
    fn from(value: SpringJointSimulationInput<'a>) -> Self {
        Self {
            joint: value.joint,
            parent_position: value.parent_position,
            parent_rotation: value.parent_rotation,
            local_axis: value.local_axis,
            bone_length: value.bone_length,
            colliders: value.colliders,
            delta: value.delta,
        }
    }
}

pub fn step_spring_particle(state: &mut SpringParticleState, step: SpringParticleStep<'_>) -> Vec3 {
    let axis = (step.parent_rotation * step.local_axis).normalize_or_zero();
    let inertia = state.current_tail
        + (state.current_tail - state.previous_tail) * (1.0 - step.joint.drag_force);
    let stiffness = axis * step.joint.stiffness * step.delta.0;
    let gravity =
        step.joint.gravity_dir.normalize_or_zero() * step.joint.gravity_power * step.delta.0;
    let mut next_tail = inertia + stiffness + gravity;

    next_tail = constrain_length(step.parent_position, next_tail, step.bone_length);
    for collider in step.colliders {
        next_tail = resolve_collision(next_tail, step.joint.hit_radius, collider);
    }

    state.previous_tail = state.current_tail;
    state.current_tail = next_tail;
    next_tail
}

pub fn step_spring_joint(
    state: &mut SpringParticleState,
    input: SpringJointSimulationInput<'_>,
) -> Vec3 {
    step_spring_particle(state, input.into())
}

pub fn step_spring_joint_parity(
    state: &mut CenterSpringParticleState,
    input: SpringJointParityInput<'_>,
) -> (Vec3, Quat) {
    if input.delta.0 <= 0.0 {
        return (state.current_tail, input.rest.initial_local_rotation);
    }

    let center_to_world = input
        .center_world
        .map(transform_matrix)
        .unwrap_or(Mat4::IDENTITY);
    let world_to_center = center_to_world.inverse();
    let parent_rotation_delta =
        input.parent_world.rotation * input.rest.initial_parent_world_rotation.inverse();
    let world_space_bone_axis =
        (parent_rotation_delta * input.rest.initial_world_bone_axis).normalize_or_zero();
    let world_space_bone_length = input.rest.initial_world_bone_length;

    let inertia = state.current_tail
        + (state.current_tail - state.previous_tail) * (1.0 - input.joint.drag_force);
    let mut next_tail = center_to_world.transform_point3(inertia)
        + world_space_bone_axis * input.joint.stiffness * input.delta.0
        + input.joint.gravity_dir.normalize_or_zero() * input.joint.gravity_power * input.delta.0;

    next_tail = constrain_length(
        input.joint_world.translation,
        next_tail,
        world_space_bone_length,
    );
    for collider in input.colliders {
        next_tail = resolve_collision(next_tail, input.joint.hit_radius, collider);
        next_tail = constrain_length(
            input.joint_world.translation,
            next_tail,
            world_space_bone_length,
        );
    }

    state.previous_tail = state.current_tail;
    state.current_tail = world_to_center.transform_point3(next_tail);

    let world_initial_inverse =
        (transform_matrix(input.parent_world) * input.rest.initial_local_matrix).inverse();
    let local_tail = world_initial_inverse
        .transform_point3(next_tail)
        .normalize_or_zero();
    let rotation = input.rest.initial_local_rotation
        * Quat::from_rotation_arc(input.rest.bone_axis, local_tail);
    (state.current_tail, rotation.normalize())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringJointRotationInput {
    pub parent_world_rotation: Quat,
    pub joint_rest_rotation: Quat,
    pub local_axis: Vec3,
    pub parent_world_position: Vec3,
    pub tail_world_position: Vec3,
}

pub fn solve_spring_joint_rotation(input: SpringJointRotationInput) -> Quat {
    let rest_direction = input.parent_world_rotation
        * (input.joint_rest_rotation * input.local_axis).normalize_or_zero();
    let target_direction =
        (input.tail_world_position - input.parent_world_position).normalize_or_zero();
    if rest_direction.length_squared() <= f32::EPSILON
        || target_direction.length_squared() <= f32::EPSILON
    {
        return input.joint_rest_rotation;
    }

    let from_to = Quat::from_rotation_arc(rest_direction, target_direction);
    (input.parent_world_rotation.inverse()
        * from_to
        * input.parent_world_rotation
        * input.joint_rest_rotation)
        .normalize()
}

pub fn collider_shape_in_simulation_space(
    collider: &SpringCollider,
    collider_world: Transform,
    center_world: Option<Transform>,
) -> ColliderShape {
    let collider_matrix = transform_matrix(collider_world);
    let center_inverse = center_world
        .map(transform_matrix)
        .map(|matrix| matrix.inverse())
        .unwrap_or(Mat4::IDENTITY);
    let to_simulation_space = center_inverse * collider_matrix;
    let radius_scale = collider_world.scale.max_element().abs();

    match &collider.shape {
        ColliderShape::Sphere {
            offset,
            radius,
            inside,
        } => ColliderShape::Sphere {
            offset: to_simulation_space.transform_point3(*offset),
            radius: *radius * radius_scale,
            inside: *inside,
        },
        ColliderShape::Capsule {
            offset,
            radius,
            tail,
            inside,
        } => ColliderShape::Capsule {
            offset: to_simulation_space.transform_point3(*offset),
            radius: *radius * radius_scale,
            tail: to_simulation_space.transform_point3(*tail),
            inside: *inside,
        },
        ColliderShape::Plane {
            offset,
            normal,
            inside,
        } => ColliderShape::Plane {
            offset: to_simulation_space.transform_point3(*offset),
            normal: to_simulation_space
                .transform_vector3(*normal)
                .normalize_or_zero(),
            inside: *inside,
        },
    }
}

fn transform_matrix(transform: Transform) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    )
}

pub fn resolve_collision(position: Vec3, particle_radius: f32, collider: &ColliderShape) -> Vec3 {
    match collider {
        ColliderShape::Sphere {
            offset,
            radius,
            inside,
        } => {
            let radius = if *inside {
                (*radius - particle_radius).max(0.0)
            } else {
                particle_radius + *radius
            };
            resolve_sphere_collision(position, *offset, radius, *inside)
        }
        ColliderShape::Capsule {
            offset,
            radius,
            tail,
            inside,
        } => {
            let closest = closest_point_on_segment(position, *offset, *tail);
            let radius = if *inside {
                (*radius - particle_radius).max(0.0)
            } else {
                particle_radius + *radius
            };
            resolve_sphere_collision(position, closest, radius, *inside)
        }
        ColliderShape::Plane {
            offset,
            normal,
            inside,
        } => {
            let normal = normal.normalize_or_zero();
            let signed_distance = (position - *offset).dot(normal);
            if *inside && signed_distance > -particle_radius {
                position - normal * (particle_radius + signed_distance)
            } else if !inside && signed_distance < particle_radius {
                position + normal * (particle_radius - signed_distance)
            } else {
                position
            }
        }
    }
}

fn constrain_length(parent_position: Vec3, tail: Vec3, bone_length: f32) -> Vec3 {
    parent_position + (tail - parent_position).normalize_or_zero() * bone_length
}

fn resolve_sphere_collision(position: Vec3, center: Vec3, radius: f32, inside: bool) -> Vec3 {
    let delta = position - center;
    let distance = delta.length();
    if !inside && distance < radius {
        center + delta.normalize_or(Vec3::Y) * radius
    } else if inside && distance > radius && distance > f32::EPSILON {
        center + delta / distance * radius
    } else {
        position
    }
}

fn closest_point_on_segment(point: Vec3, start: Vec3, end: Vec3) -> Vec3 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f32::EPSILON {
        return start;
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    start + segment * t
}

pub fn calc_azimuth_altitude(direction: Vec3) -> (f32, f32) {
    let normalized = direction.normalize_or_zero();
    let azimuth = normalized.x.atan2(-normalized.z).to_degrees();
    let altitude = normalized.y.asin().to_degrees();
    (azimuth, altitude)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LookAtExpressionWeights {
    pub values: IndexMap<ExpressionName, f32>,
}

impl LookAtExpressionWeights {
    pub fn get(&self, name: &ExpressionName) -> f32 {
        self.values.get(name).copied().unwrap_or(0.0)
    }
}

pub fn calc_look_at_expression_weights(
    look_at: &LookAt,
    target_direction: Vec3,
) -> LookAtExpressionWeights {
    let (azimuth, altitude) = calc_azimuth_altitude(target_direction);
    let horizontal_inner = map_range(azimuth.abs(), look_at.horizontal_inner);
    let horizontal_outer = map_range(azimuth.abs(), look_at.horizontal_outer);
    let horizontal = horizontal_inner.max(horizontal_outer);
    let vertical_up = map_range(altitude.max(0.0), look_at.vertical_up);
    let vertical_down = map_range((-altitude).max(0.0), look_at.vertical_down);

    let values = [
        (
            ExpressionName::LookLeft,
            (azimuth < 0.0).then_some(horizontal),
        ),
        (
            ExpressionName::LookRight,
            (azimuth > 0.0).then_some(horizontal),
        ),
        (
            ExpressionName::LookUp,
            (altitude > 0.0).then_some(vertical_up),
        ),
        (
            ExpressionName::LookDown,
            (altitude < 0.0).then_some(vertical_down),
        ),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|value| (name, value)))
    .collect();

    LookAtExpressionWeights { values }
}

fn map_range(input: f32, range: RangeMap) -> f32 {
    if range.input_max_value <= f32::EPSILON {
        return 0.0;
    }
    (input / range.input_max_value).clamp(0.0, 1.0) * range.output_scale
}

pub fn sample_rotation_track(track: &RotationTrack, time: f32) -> Option<Quat> {
    sample_track(&track.times, &track.values, time, |a, b, t| a.slerp(b, t))
}

pub fn sample_translation_track(track: &TranslationTrack, time: f32) -> Option<Vec3> {
    sample_track(&track.times, &track.values, time, |a, b, t| a.lerp(b, t))
}

pub fn sample_scalar_track(track: &ScalarTrack, time: f32) -> Option<f32> {
    sample_track(&track.times, &track.values, time, |a, b, t| a + (b - a) * t)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VrmAnimationFrame {
    pub humanoid_rotations: IndexMap<HumanBoneName, Quat>,
    pub hips_translation: Option<Vec3>,
    pub source_rest_hips_position: Option<Vec3>,
    pub preset_expressions: IndexMap<ExpressionName, f32>,
    pub custom_expressions: IndexMap<String, f32>,
    pub look_at: Option<Quat>,
}

pub fn sample_vrm_animation(animation: &VrmAnimation, time: f32) -> VrmAnimationFrame {
    VrmAnimationFrame {
        humanoid_rotations: animation
            .humanoid_rotation_tracks
            .iter()
            .filter_map(|(bone, track)| {
                sample_rotation_track(track, time).map(|rotation| (bone.clone(), rotation))
            })
            .collect(),
        hips_translation: animation
            .hips_translation
            .as_ref()
            .and_then(|track| sample_translation_track(track, time)),
        source_rest_hips_position: animation
            .hips_translation
            .as_ref()
            .map(|_| animation.rest_hips_position),
        preset_expressions: animation
            .preset_expression_tracks
            .iter()
            .filter_map(|(name, track)| {
                sample_scalar_track(track, time).map(|value| (name.clone(), value))
            })
            .collect(),
        custom_expressions: animation
            .custom_expression_tracks
            .iter()
            .filter_map(|(name, track)| {
                sample_scalar_track(track, time).map(|value| (name.clone(), value))
            })
            .collect(),
        look_at: animation
            .look_at_track
            .as_ref()
            .and_then(|track| sample_rotation_track(track, time)),
    }
}

fn sample_track<T: Copy>(
    times: &[f32],
    values: &[T],
    time: f32,
    interpolate: impl Fn(T, T, f32) -> T,
) -> Option<T> {
    if times.is_empty() || times.len() != values.len() {
        return None;
    }
    if time <= times[0] {
        return Some(values[0]);
    }
    for window in times.windows(2).zip(values.windows(2)) {
        let ([t0, t1], [v0, v1]) = window else {
            continue;
        };
        if (*t0..=*t1).contains(&time) {
            let alpha = ((time - *t0) / (*t1 - *t0)).clamp(0.0, 1.0);
            return Some(interpolate(*v0, *v1, alpha));
        }
    }
    values.last().copied()
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("circular node constraint dependency")]
    CircularConstraint,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_constraint_cycle() {
        let manager = ConstraintManager::new(vec![
            NodeConstraint {
                destination: NodeRef(1),
                source: NodeRef(2),
                kind: ConstraintKind::Rotation,
                weight: 1.0,
            },
            NodeConstraint {
                destination: NodeRef(2),
                source: NodeRef(1),
                kind: ConstraintKind::Rotation,
                weight: 1.0,
            },
        ]);
        assert_eq!(
            manager.update_order().unwrap_err(),
            RuntimeError::CircularConstraint
        );
    }

    #[test]
    fn samples_scalar_track() {
        let track = ScalarTrack {
            times: vec![0.0, 1.0],
            values: vec![0.0, 10.0],
        };
        assert_eq!(sample_scalar_track(&track, 0.25), Some(2.5));
    }

    #[test]
    fn sampling_tracks_handles_bounds_and_invalid_shapes() {
        let scalar = ScalarTrack {
            times: vec![1.0, 2.0],
            values: vec![10.0, 20.0],
        };
        assert_eq!(sample_scalar_track(&scalar, 0.0), Some(10.0));
        assert_eq!(sample_scalar_track(&scalar, 3.0), Some(20.0));
        assert_eq!(
            sample_scalar_track(
                &ScalarTrack {
                    times: vec![0.0],
                    values: vec![],
                },
                0.0
            ),
            None
        );

        let rotation = RotationTrack {
            times: vec![0.0, 1.0],
            values: vec![Quat::IDENTITY, Quat::from_rotation_y(1.0)],
        };
        assert!(sample_rotation_track(&rotation, 0.5).is_some());

        let translation = TranslationTrack {
            times: vec![0.0, 1.0],
            values: vec![Vec3::ZERO, Vec3::X],
        };
        assert_eq!(
            sample_translation_track(&translation, 0.5),
            Some(Vec3::new(0.5, 0.0, 0.0))
        );
    }

    #[test]
    fn maps_look_at_direction_to_expression_weights() {
        let look_at = LookAt {
            horizontal_inner: RangeMap {
                input_max_value: 45.0,
                output_scale: 1.0,
            },
            horizontal_outer: RangeMap {
                input_max_value: 90.0,
                output_scale: 0.5,
            },
            vertical_up: RangeMap {
                input_max_value: 45.0,
                output_scale: 1.0,
            },
            vertical_down: RangeMap {
                input_max_value: 45.0,
                output_scale: 1.0,
            },
            ..LookAt::default()
        };

        let weights = calc_look_at_expression_weights(&look_at, Vec3::new(1.0, 1.0, -1.0));

        assert!(weights.get(&ExpressionName::LookRight) > 0.0);
        assert!(weights.get(&ExpressionName::LookUp) > 0.0);
        assert_eq!(weights.get(&ExpressionName::LookLeft), 0.0);
        assert_eq!(weights.get(&ExpressionName::LookDown), 0.0);
    }

    #[test]
    fn spring_particle_is_pushed_out_of_sphere_collider() {
        let joint = SpringJoint {
            hit_radius: 0.1,
            stiffness: 0.0,
            gravity_power: 0.0,
            drag_force: 1.0,
            ..SpringJoint::default()
        };
        let mut state = SpringParticleState {
            current_tail: Vec3::new(0.0, 0.0, 0.0),
            previous_tail: Vec3::new(0.0, 0.0, 0.0),
        };
        let tail = step_spring_particle(
            &mut state,
            SpringParticleStep {
                joint: &joint,
                parent_position: Vec3::ZERO,
                parent_rotation: Quat::IDENTITY,
                local_axis: Vec3::Y,
                bone_length: 1.0,
                colliders: &[ColliderShape::Sphere {
                    offset: Vec3::Y,
                    radius: 0.5,
                    inside: false,
                }],
                delta: DeltaTime(1.0),
            },
        );

        assert!(tail.distance(Vec3::Y) >= 0.6 - f32::EPSILON);
    }

    #[test]
    fn spring_collision_handles_capsule_and_plane() {
        let capsule = ColliderShape::Capsule {
            offset: Vec3::ZERO,
            radius: 0.5,
            tail: Vec3::Y,
            inside: false,
        };
        let capsule_result = resolve_collision(Vec3::new(0.1, 0.5, 0.0), 0.1, &capsule);
        assert!(capsule_result.distance(Vec3::new(0.0, 0.5, 0.0)) >= 0.6 - f32::EPSILON);

        let plane = ColliderShape::Plane {
            offset: Vec3::ZERO,
            normal: Vec3::Y,
            inside: false,
        };
        let plane_result = resolve_collision(Vec3::new(0.0, -0.2, 0.0), 0.1, &plane);
        assert!(plane_result.y >= 0.1 - f32::EPSILON);
    }

    #[test]
    fn spring_inside_sphere_collider_keeps_particle_inside_volume() {
        let collider = ColliderShape::Sphere {
            offset: Vec3::ZERO,
            radius: 1.0,
            inside: true,
        };

        let result = resolve_collision(Vec3::new(2.0, 0.0, 0.0), 0.1, &collider);

        assert!(result.abs_diff_eq(Vec3::new(0.9, 0.0, 0.0), 0.0001));
    }

    #[test]
    fn solves_spring_joint_rotation_from_tail_direction() {
        let rotation = solve_spring_joint_rotation(SpringJointRotationInput {
            parent_world_rotation: Quat::IDENTITY,
            joint_rest_rotation: Quat::IDENTITY,
            local_axis: Vec3::Y,
            parent_world_position: Vec3::ZERO,
            tail_world_position: Vec3::X,
        });

        assert!((rotation * Vec3::Y).abs_diff_eq(Vec3::X, 0.0001));
    }

    #[test]
    fn collider_shape_can_be_converted_to_center_space() {
        let collider = SpringCollider {
            node: NodeRef(1),
            shape: ColliderShape::Sphere {
                offset: Vec3::X,
                radius: 0.5,
                inside: true,
            },
        };
        let collider_world = Transform {
            translation: Vec3::new(3.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::splat(2.0),
        };
        let center_world = Transform {
            translation: Vec3::new(1.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };

        let shape =
            collider_shape_in_simulation_space(&collider, collider_world, Some(center_world));

        assert_eq!(
            shape,
            ColliderShape::Sphere {
                offset: Vec3::new(4.0, 0.0, 0.0),
                radius: 1.0,
                inside: true
            }
        );
    }

    #[test]
    fn spring_manager_resolves_colliders_for_spring_groups() {
        let system = SpringBoneSystem {
            colliders: vec![SpringCollider {
                node: NodeRef(1),
                shape: ColliderShape::Sphere {
                    offset: Vec3::ZERO,
                    radius: 1.0,
                    inside: false,
                },
            }],
            collider_groups: vec![SpringColliderGroup {
                name: Some("head".to_owned()),
                colliders: vec![0],
            }],
            springs: vec![Spring {
                collider_groups: vec![0],
                ..Spring::default()
            }],
        };

        let manager = SpringBoneManager::new(system);

        assert_eq!(manager.spring_colliders(0).len(), 1);
        assert!(manager.spring_colliders(99).is_empty());
    }

    #[test]
    fn samples_vrm_animation_frame() {
        let mut animation = VrmAnimation {
            hips_translation: Some(TranslationTrack {
                times: vec![0.0, 1.0],
                values: vec![Vec3::ZERO, Vec3::Y],
            }),
            ..VrmAnimation::default()
        };
        animation.preset_expression_tracks.insert(
            ExpressionName::Blink,
            ScalarTrack {
                times: vec![0.0, 1.0],
                values: vec![0.0, 1.0],
            },
        );

        let frame = sample_vrm_animation(&animation, 0.5);
        assert_eq!(frame.hips_translation, Some(Vec3::new(0.0, 0.5, 0.0)));
        assert_eq!(frame.preset_expressions[&ExpressionName::Blink], 0.5);
    }

    #[test]
    fn spring_runtime_state_can_reset_to_init() {
        let system = SpringBoneSystem {
            springs: vec![Spring {
                joints: vec![SpringJoint::default()],
                ..Spring::default()
            }],
            ..SpringBoneSystem::default()
        };
        let mut state = SpringRuntimeState::from_system(&system, |_, _, _| {
            SpringParticleState::at_rest(Vec3::ZERO, Vec3::Y, 1.0)
        });
        state.get_mut(0, 0).unwrap().current_tail = Vec3::X;
        state.reset();
        assert_eq!(state.get(0, 0).unwrap().current_tail, Vec3::Y);
    }

    #[test]
    fn center_spring_runtime_state_can_reset_to_init() {
        let system = SpringBoneSystem {
            springs: vec![Spring {
                joints: vec![SpringJoint::default()],
                ..Spring::default()
            }],
            ..SpringBoneSystem::default()
        };
        let mut state = CenterSpringRuntimeState::from_system(&system, |_, _, _| {
            CenterSpringParticleState::at_rest(Vec3::Y)
        });
        state.get_mut(0, 0).unwrap().current_tail = Vec3::X;
        state.reset();
        assert_eq!(state.get(0, 0).unwrap().current_tail, Vec3::Y);
    }

    #[test]
    fn spring_joint_parity_step_uses_center_space_and_rest_axis() {
        let joint = SpringJoint {
            stiffness: 1.0,
            gravity_power: 0.0,
            drag_force: 1.0,
            ..SpringJoint::default()
        };
        let rest = SpringJointRestState::from_local_child(Transform::default(), Vec3::Y);
        let mut state = CenterSpringParticleState::at_rest(Vec3::Y);

        let (center_tail, rotation) = step_spring_joint_parity(
            &mut state,
            SpringJointParityInput {
                joint: &joint,
                rest,
                parent_world: Transform::default(),
                joint_world: Transform::default(),
                child_world: Some(Transform {
                    translation: Vec3::Y,
                    ..Transform::default()
                }),
                center_world: Some(Transform {
                    translation: Vec3::X,
                    ..Transform::default()
                }),
                colliders: &[],
                delta: DeltaTime(1.0),
            },
        );

        assert!(center_tail.length() > 0.0);
        assert!(rotation.is_normalized());
    }

    #[test]
    fn spring_joint_parity_premultiplies_initial_local_rotation() {
        let joint = SpringJoint {
            stiffness: 0.0,
            gravity_power: 0.0,
            drag_force: 1.0,
            ..SpringJoint::default()
        };
        let initial = Transform {
            rotation: Quat::from_rotation_y(0.7),
            ..Transform::default()
        };
        let rest = SpringJointRestState::from_local_child(initial, Vec3::Y);
        let mut state = CenterSpringParticleState::at_rest(Vec3::Z);

        let (_, rotation) = step_spring_joint_parity(
            &mut state,
            SpringJointParityInput {
                joint: &joint,
                rest,
                parent_world: Transform::default(),
                joint_world: Transform::default(),
                child_world: Some(Transform {
                    translation: Vec3::Z,
                    ..Transform::default()
                }),
                center_world: None,
                colliders: &[],
                delta: DeltaTime(1.0),
            },
        );

        let local_tail = transform_matrix(initial)
            .inverse()
            .transform_point3(Vec3::Z)
            .normalize_or_zero();
        let arc = Quat::from_rotation_arc(Vec3::Y, local_tail);
        let expected = (initial.rotation * arc).normalize();
        let reversed = (arc * initial.rotation).normalize();

        assert!(rotation.abs_diff_eq(expected, 1e-5));
        assert!(!rotation.abs_diff_eq(reversed, 1e-5));
    }

    #[test]
    fn spring_joint_rest_state_uses_vrm0_tail_fallback() {
        let rest = SpringJointRestState::vrm0_tail_fallback(Transform {
            translation: Vec3::X * 2.0,
            ..Transform::default()
        });

        assert!(
            rest.initial_local_child_position
                .abs_diff_eq(Vec3::X * 0.07, 0.0001)
        );
        assert!(rest.bone_axis.abs_diff_eq(Vec3::X, 0.0001));
    }

    #[test]
    fn rotation_constraint_transfers_source_delta() {
        let state = ConstraintRestState::new(Quat::IDENTITY, Quat::IDENTITY);
        let source = Quat::from_rotation_y(1.0);
        let solved = solve_rotation_constraint(state, source, 1.0);
        assert!(solved.abs_diff_eq(source, 1e-6));
    }

    #[test]
    fn rotation_constraint_matches_three_vrm_rest_and_weight_cases() {
        let quat_a = quat(0.191, 0.462, 0.191, 0.845);
        let quat_b = quat(-0.462, -0.191, -0.462, 0.733);
        let identity = Quat::IDENTITY;

        assert_quat_close(
            solve_rotation_constraint(ConstraintRestState::new(identity, identity), quat_b, 1.0),
            quat_b,
        );
        assert_quat_close(
            solve_rotation_constraint(ConstraintRestState::new(identity, identity), quat_b, 0.5),
            identity.slerp(quat_b, 0.5),
        );
        assert_quat_close(
            solve_rotation_constraint(ConstraintRestState::new(quat_a, identity), quat_b, 1.0),
            quat_a * quat_b,
        );
        assert_quat_close(
            solve_rotation_constraint(ConstraintRestState::new(quat_a, identity), quat_b, 0.5),
            quat_a.slerp(quat_a * quat_b, 0.5),
        );
    }

    #[test]
    fn roll_constraint_uses_selected_axis_and_weight() {
        let state = ConstraintRestState::new(Quat::IDENTITY, Quat::IDENTITY);
        let source = Quat::from_rotation_x(1.0);
        let solved = solve_roll_constraint(state, source, Axis::PositiveX, 0.5);

        assert!((solved * Vec3::Y).angle_between(Vec3::Y) > 0.0);
        assert!((solved * Vec3::Y).angle_between(source * Vec3::Y) < 1.0);
    }

    #[test]
    fn roll_constraint_matches_three_vrm_axis_cases() {
        let quat_identity = Quat::IDENTITY;
        let quat_ny90 = quat(0.0, -0.707, 0.0, 0.707);
        let quat_ny45 = quat(0.0, -0.383, 0.0, 0.924);
        let quat_pz90 = quat(0.0, 0.0, 0.707, 0.707);
        let quat_px90 = quat(0.707, 0.0, 0.0, 0.707);
        let quat_px90_pz90 = quat(0.5, -0.5, 0.5, 0.5);

        let identity_state = ConstraintRestState::new(quat_identity, quat_identity);
        assert_quat_close(
            solve_roll_constraint(identity_state, quat_ny90, Axis::PositiveY, 1.0),
            quat_ny90,
        );
        assert_quat_close(
            solve_roll_constraint(identity_state, quat_ny90, Axis::PositiveY, 0.5),
            quat_ny45,
        );
        assert_quat_close(
            solve_roll_constraint(identity_state, quat_pz90, Axis::PositiveY, 1.0),
            quat_identity,
        );
        assert_quat_close(
            solve_roll_constraint(
                ConstraintRestState::new(quat_identity, quat_px90),
                quat_px90 * quat_pz90,
                Axis::PositiveY,
                1.0,
            ),
            quat_ny90,
        );
        assert_quat_close(
            solve_roll_constraint(
                ConstraintRestState::new(quat_px90, quat_identity),
                quat_ny90,
                Axis::PositiveZ,
                1.0,
            ),
            quat_px90_pz90,
        );
    }

    #[test]
    fn aim_constraint_points_axis_toward_source() {
        let solved = solve_aim_constraint(AimConstraintInput {
            destination_rest_rotation: Quat::IDENTITY,
            destination_world_position: Vec3::ZERO,
            source_world_position: Vec3::Y,
            destination_parent_world_rotation: Quat::IDENTITY,
            axis: Axis::PositiveX,
            weight: 1.0,
        });
        let aimed = solved * Vec3::X;
        assert!(aimed.abs_diff_eq(Vec3::Y, 1e-5));
    }

    #[test]
    fn aim_constraint_matches_three_vrm_axis_parent_and_weight_cases() {
        let quat_nz90 = quat(0.0, 0.0, -0.707, 0.707);
        let quat_nz45 = quat(0.0, 0.0, -0.383, 0.924);
        let quat_nz135 = quat(0.0, 0.0, -0.924, 0.383);
        let quat_px90 = quat(0.707, 0.0, 0.0, 0.707);
        let quat_ny90 = quat(0.0, -0.707, 0.0, 0.707);
        let quat_nz90_ny90 = quat(-0.5, -0.5, -0.5, 0.5);
        let quat_nz45_ny90 = quat(-0.271, -0.653, -0.271, 0.653);
        let quat_py180 = quat(0.0, 1.0, 0.0, 0.0);
        let quat_pz90 = quat(0.0, 0.0, 0.707, 0.707);
        let quat_90_around_xz = quat(0.5, 0.0, 0.5, 0.707);

        assert_quat_close(
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: Quat::IDENTITY,
                destination_world_position: Vec3::ZERO,
                source_world_position: Vec3::X,
                destination_parent_world_rotation: Quat::IDENTITY,
                axis: Axis::PositiveY,
                weight: 1.0,
            }),
            quat_nz90,
        );
        assert_quat_close(
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: Quat::IDENTITY,
                destination_world_position: Vec3::ZERO,
                source_world_position: Vec3::X,
                destination_parent_world_rotation: Quat::IDENTITY,
                axis: Axis::PositiveY,
                weight: 0.5,
            }),
            quat_nz45,
        );
        assert_quat_close(
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: Quat::IDENTITY,
                destination_world_position: Vec3::ZERO,
                source_world_position: Vec3::Z,
                destination_parent_world_rotation: Quat::IDENTITY,
                axis: Axis::PositiveY,
                weight: 1.0,
            }),
            quat_px90,
        );
        assert_quat_close(
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: Quat::IDENTITY,
                destination_world_position: Vec3::ZERO,
                source_world_position: Vec3::X,
                destination_parent_world_rotation: Quat::IDENTITY,
                axis: Axis::NegativeZ,
                weight: 1.0,
            }),
            quat_ny90,
        );
        assert_quat_close(
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: quat_ny90,
                destination_world_position: Vec3::ZERO,
                source_world_position: Vec3::X,
                destination_parent_world_rotation: Quat::IDENTITY,
                axis: Axis::PositiveY,
                weight: 1.0,
            }),
            quat_nz90_ny90,
        );
        assert_quat_close(
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: quat_ny90,
                destination_world_position: Vec3::ZERO,
                source_world_position: Vec3::X,
                destination_parent_world_rotation: Quat::IDENTITY,
                axis: Axis::PositiveY,
                weight: 0.5,
            }),
            quat_nz45_ny90,
        );
        assert_quat_close(
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: Quat::IDENTITY,
                destination_world_position: Vec3::Y,
                source_world_position: Vec3::X,
                destination_parent_world_rotation: Quat::IDENTITY,
                axis: Axis::PositiveY,
                weight: 1.0,
            }),
            quat_nz135,
        );
        assert_quat_close(
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: Quat::IDENTITY,
                destination_world_position: Vec3::ZERO,
                source_world_position: Vec3::X,
                destination_parent_world_rotation: quat_py180,
                axis: Axis::PositiveY,
                weight: 1.0,
            }),
            quat_pz90,
        );
        assert_quat_close(
            solve_aim_constraint(AimConstraintInput {
                destination_rest_rotation: Quat::IDENTITY,
                destination_world_position: Vec3::ZERO,
                source_world_position: Vec3::new(1.0, 0.0, -1.0),
                destination_parent_world_rotation: quat_py180,
                axis: Axis::PositiveY,
                weight: 1.0,
            }),
            quat_90_around_xz,
        );
    }

    fn quat(x: f32, y: f32, z: f32, w: f32) -> Quat {
        Quat::from_xyzw(x, y, z, w).normalize()
    }

    fn assert_quat_close(actual: Quat, expected: Quat) {
        assert!(
            actual.abs_diff_eq(expected, 0.003) || actual.abs_diff_eq(-expected, 0.003),
            "actual={actual:?}, expected={expected:?}"
        );
    }
}
