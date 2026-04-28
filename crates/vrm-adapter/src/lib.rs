//! Traits for connecting `vrm-rs` runtime output to external engines.

use glam::{Quat, Vec3};
use thiserror::Error;
use vrm_core::{ExpressionBind, MaterialRef, NodeRef, TextureRef, Transform};
use vrm_runtime::{AppliedExpression, RuntimeEvents};

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
}

pub trait TextureResolver {
    type Texture;
    type Error;

    fn resolve_texture(&self, texture: TextureRef) -> Result<Self::Texture, Self::Error>;
}

pub trait AnimationSink {
    type Error;

    fn apply_expression(&mut self, expression: &AppliedExpression) -> Result<(), Self::Error>;
    fn apply_runtime_events(&mut self, events: &RuntimeEvents) -> Result<(), Self::Error>;
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

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AdapterError<E> {
    #[error("target adapter error: {0}")]
    Target(E),
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

    #[derive(Default)]
    struct Mock {
        morphs: Vec<(NodeRef, usize, f32)>,
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
}
