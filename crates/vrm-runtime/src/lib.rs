//! Renderer-agnostic runtime algorithms for VRM components.

use glam::{Quat, Vec3};
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

#[derive(Clone, Debug, Default)]
pub struct SpringBoneManager {
    system: SpringBoneSystem,
}

impl SpringBoneManager {
    pub fn new(system: SpringBoneSystem) -> Self {
        Self { system }
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringJointStep {
    pub spring_index: usize,
    pub joint_index: usize,
    pub node: NodeRef,
    pub gravity: Vec3,
}

pub fn calc_azimuth_altitude(direction: Vec3) -> (f32, f32) {
    let normalized = direction.normalize_or_zero();
    let azimuth = normalized.x.atan2(-normalized.z).to_degrees();
    let altitude = normalized.y.asin().to_degrees();
    (azimuth, altitude)
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
}
