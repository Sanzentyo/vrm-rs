//! Side-effect-free conversion from protocol data into core VRM models.

use glam::Vec3;
use thiserror::Error;
use vrm_core::*;
use vrm_protocol::{VrmExtension, node_constraint, spring_bone, vrm0, vrm1, vrma};

#[derive(Clone, Debug, Default)]
pub struct ValidatedAssetBuilder {
    node_count: Option<usize>,
    material_count: Option<usize>,
}

impl ValidatedAssetBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_node_count(mut self, node_count: usize) -> Self {
        self.node_count = Some(node_count);
        self
    }

    pub fn with_material_count(mut self, material_count: usize) -> Self {
        self.material_count = Some(material_count);
        self
    }

    pub fn build(
        self,
        bundle: vrm_protocol::ExtensionBundle,
    ) -> Result<VrmAsset<Validated>, BuildError> {
        let extension = bundle.vrm.ok_or(BuildError::MissingVrm)?;
        let mut document = match extension {
            VrmExtension::Vrm1(vrm) => map_vrm1(*vrm)?,
            VrmExtension::Vrm0(vrm) => map_vrm0(*vrm)?,
            VrmExtension::Vrma(animation) => map_vrma(*animation),
        };

        if let Some(spring_bone) = bundle.spring_bone {
            document.spring_bone = Feature::Present(map_spring_bone(spring_bone));
        }

        document.node_constraints = bundle
            .node_constraints
            .into_iter()
            .filter_map(|extension| {
                map_node_constraint(extension.node, extension.constraint).transpose()
            })
            .collect::<Result<_, _>>()?;

        document.materials = bundle
            .mtoon_materials
            .into_iter()
            .map(|(index, material)| {
                let mut core = Material {
                    name: Some(format!("material_{index}")),
                    mtoon: Feature::Present(map_mtoon(material)),
                };
                if let Some(count) = self.material_count {
                    ensure_ref("material", index, count)?;
                }
                Ok::<_, BuildError>(std::mem::take(&mut core))
            })
            .collect::<Result<_, _>>()?;

        if !document.humanoid.bones.is_empty() && !document.humanoid.required_bones_present() {
            return Err(BuildError::Core(CoreError::MissingRequiredHumanBones));
        }

        if let Some(node_count) = self.node_count {
            validate_nodes(&document, node_count)?;
        }
        if let Some(material_count) = self.material_count {
            validate_materials(&document, material_count)?;
        }
        validate_spring_references(&document)?;

        Ok(VrmAsset::<Parsed>::new_parsed(document).mark_validated())
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum BuildError {
    #[error("missing VRM extension")]
    MissingVrm,
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("invalid protocol data: {0}")]
    Protocol(String),
}

fn map_vrm1(vrm: vrm1::VrmcVrm) -> Result<VrmDocument, BuildError> {
    let mut document = VrmDocument {
        kind: VrmKind::Vrm1,
        meta: Meta {
            name: vrm.meta.name,
            version: vrm.meta.version,
            authors: vrm.meta.authors,
            license_url: vrm.meta.license_url,
            copyright_information: vrm.meta.copyright_information,
            contact_information: vrm.meta.contact_information,
        },
        humanoid: map_vrm1_humanoid(vrm.humanoid)?,
        ..VrmDocument::default()
    };

    if let Some(first_person) = vrm.first_person {
        document.first_person = Feature::Present(FirstPerson {
            mesh_annotations: first_person
                .mesh_annotations
                .unwrap_or_default()
                .into_iter()
                .map(|annotation| FirstPersonMeshAnnotation {
                    node: NodeRef(annotation.node),
                    kind: FirstPersonAnnotation::from(annotation.kind.as_str()),
                })
                .collect(),
        });
    }

    if let Some(look_at) = vrm.look_at {
        document.look_at = Feature::Present(LookAt {
            offset_from_head: vec3(look_at.offset_from_head_bone),
            kind: match look_at.kind.as_deref() {
                Some("expression") => LookAtKind::Expression,
                Some("bone") | None => LookAtKind::Bone,
                Some(other) => LookAtKind::Unknown(other.to_owned()),
            },
            horizontal_inner: look_at.horizontal_inner(),
            horizontal_outer: look_at.horizontal_outer(),
            vertical_down: look_at.vertical_down(),
            vertical_up: look_at.vertical_up(),
        });
    }

    if let Some(expressions) = vrm.expressions {
        document.expressions = Feature::Present(map_vrm1_expressions(expressions));
    }

    Ok(document)
}

trait LookAtRanges {
    fn horizontal_inner(&self) -> RangeMap;
    fn horizontal_outer(&self) -> RangeMap;
    fn vertical_down(&self) -> RangeMap;
    fn vertical_up(&self) -> RangeMap;
}

impl LookAtRanges for vrm1::LookAt {
    fn horizontal_inner(&self) -> RangeMap {
        map_range(self.range_map_horizontal_inner)
    }
    fn horizontal_outer(&self) -> RangeMap {
        map_range(self.range_map_horizontal_outer)
    }
    fn vertical_down(&self) -> RangeMap {
        map_range(self.range_map_vertical_down)
    }
    fn vertical_up(&self) -> RangeMap {
        map_range(self.range_map_vertical_up)
    }
}

fn map_range(range: Option<vrm1::LookAtRangeMap>) -> RangeMap {
    range.map_or_else(RangeMap::default, |range| RangeMap {
        input_max_value: range.input_max_value,
        output_scale: range.output_scale,
    })
}

fn map_vrm1_humanoid(humanoid: vrm1::Humanoid) -> Result<Humanoid, BuildError> {
    let bones = humanoid
        .human_bones
        .bones
        .into_iter()
        .map(|(name, value)| {
            let bone: vrm1::HumanBone = serde_json::from_value(value)
                .map_err(|err| BuildError::Protocol(err.to_string()))?;
            Ok((
                HumanBoneName::from(name.as_str()),
                HumanBone {
                    node: NodeRef(bone.node),
                    rest: Transform::default(),
                },
            ))
        })
        .collect::<Result<_, BuildError>>()?;

    Ok(Humanoid { bones })
}

fn map_vrm1_expressions(expressions: vrm1::Expressions) -> ExpressionSet {
    ExpressionSet {
        preset: expressions
            .preset
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(name, value)| {
                serde_json::from_value::<vrm1::Expression>(value)
                    .ok()
                    .map(|expression| {
                        (
                            ExpressionName::from(name.as_str()),
                            map_expression(expression),
                        )
                    })
            })
            .collect(),
        custom: expressions
            .custom
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(name, value)| {
                serde_json::from_value::<vrm1::Expression>(value)
                    .ok()
                    .map(|expression| (name, map_expression(expression)))
            })
            .collect(),
    }
}

fn map_expression(expression: vrm1::Expression) -> Expression {
    let morphs = expression
        .morph_target_binds
        .unwrap_or_default()
        .into_iter()
        .map(|bind| ExpressionBind::MorphTarget {
            node: NodeRef(bind.node),
            index: bind.index,
            weight: bind.weight,
        });
    let colors = expression
        .material_color_binds
        .unwrap_or_default()
        .into_iter()
        .map(|bind| ExpressionBind::MaterialColor {
            material: MaterialRef(bind.material),
            kind: bind.kind,
            target_value: bind.target_value,
        });
    let transforms = expression
        .texture_transform_binds
        .unwrap_or_default()
        .into_iter()
        .map(|bind| ExpressionBind::TextureTransform {
            material: MaterialRef(bind.material),
            scale: bind.scale,
            offset: bind.offset,
        });

    Expression {
        binds: morphs.chain(colors).chain(transforms).collect(),
        is_binary: expression.is_binary.unwrap_or(false),
        override_blink: expression.override_blink.into(),
        override_look_at: expression.override_look_at.into(),
        override_mouth: expression.override_mouth.into(),
    }
}

fn map_vrm0(vrm: vrm0::Vrm) -> Result<VrmDocument, BuildError> {
    let meta = vrm.meta.unwrap_or_default();
    let humanoid = vrm.humanoid.unwrap_or_default();
    let mut document = VrmDocument {
        kind: VrmKind::Vrm0Compat,
        meta: Meta {
            name: meta.title.unwrap_or_else(|| "VRM 0.0 Avatar".to_owned()),
            version: meta.version,
            authors: meta.author.into_iter().collect(),
            license_url: meta.other_license_url,
            copyright_information: None,
            contact_information: meta.contact_information,
        },
        humanoid: Humanoid {
            bones: humanoid
                .human_bones
                .into_iter()
                .map(|bone| {
                    (
                        HumanBoneName::from(bone.bone.as_str()),
                        HumanBone {
                            node: NodeRef(bone.node),
                            rest: Transform::default(),
                        },
                    )
                })
                .collect(),
        },
        ..VrmDocument::default()
    };

    if let Some(blend_shape) = vrm.blend_shape_master {
        document.expressions = Feature::Present(ExpressionSet {
            preset: blend_shape
                .blend_shape_groups
                .into_iter()
                .filter_map(|group| {
                    group.preset_name.map(|name| {
                        (
                            ExpressionName::from(name.as_str()),
                            Expression {
                                is_binary: group.is_binary.unwrap_or(false),
                                binds: group
                                    .binds
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|bind| ExpressionBind::MorphTarget {
                                        node: NodeRef(bind.mesh),
                                        index: bind.index,
                                        weight: bind.weight,
                                    })
                                    .collect(),
                                ..Expression::default()
                            },
                        )
                    })
                })
                .collect(),
            custom: Default::default(),
        });
    }

    Ok(document)
}

fn map_spring_bone(spring_bone: spring_bone::VrmcSpringBone) -> SpringBoneSystem {
    SpringBoneSystem {
        colliders: spring_bone
            .colliders
            .unwrap_or_default()
            .into_iter()
            .filter_map(|collider| {
                let shape = collider
                    .shape
                    .sphere
                    .map(|sphere| ColliderShape::Sphere {
                        offset: vec3(sphere.offset),
                        radius: sphere.radius.unwrap_or(0.0),
                    })
                    .or_else(|| {
                        collider
                            .shape
                            .capsule
                            .map(|capsule| ColliderShape::Capsule {
                                offset: vec3(capsule.offset),
                                radius: capsule.radius.unwrap_or(0.0),
                                tail: Vec3::from_array(capsule.tail),
                            })
                    })
                    .or_else(|| {
                        collider.shape.plane.map(|plane| ColliderShape::Plane {
                            offset: vec3(plane.offset),
                            normal: plane.normal.map_or(Vec3::Y, Vec3::from_array),
                        })
                    })?;
                Some(SpringCollider {
                    node: NodeRef(collider.node),
                    shape,
                })
            })
            .collect(),
        collider_groups: spring_bone
            .collider_groups
            .unwrap_or_default()
            .into_iter()
            .map(|group| SpringColliderGroup {
                name: group.name,
                colliders: group.colliders,
            })
            .collect(),
        springs: spring_bone
            .springs
            .unwrap_or_default()
            .into_iter()
            .map(|spring| Spring {
                name: spring.name,
                joints: spring
                    .joints
                    .into_iter()
                    .map(|joint| SpringJoint {
                        node: NodeRef(joint.node),
                        hit_radius: joint.hit_radius.unwrap_or(0.0),
                        stiffness: joint.stiffness.unwrap_or(1.0),
                        gravity_power: joint.gravity_power.unwrap_or(0.0),
                        gravity_dir: joint.gravity_dir.map_or(Vec3::NEG_Y, Vec3::from_array),
                        drag_force: joint.drag_force.unwrap_or(0.4),
                    })
                    .collect(),
                collider_groups: spring.collider_groups.unwrap_or_default(),
                center: spring.center.map(NodeRef),
            })
            .collect(),
    }
}

fn map_node_constraint(
    node: usize,
    extension: node_constraint::VrmcNodeConstraint,
) -> Result<Option<NodeConstraint>, BuildError> {
    let constraint = extension.constraint;
    if let Some(rotation) = constraint.rotation {
        return Ok(Some(NodeConstraint {
            destination: NodeRef(node),
            source: NodeRef(rotation.source),
            kind: ConstraintKind::Rotation,
            weight: rotation.weight.unwrap_or(1.0),
        }));
    }
    if let Some(roll) = constraint.roll {
        return Ok(Some(NodeConstraint {
            destination: NodeRef(node),
            source: NodeRef(roll.source),
            kind: ConstraintKind::Roll {
                axis: Axis::parse(&roll.roll_axis)
                    .ok_or_else(|| CoreError::InvalidAxis(roll.roll_axis.clone()))?,
            },
            weight: roll.weight.unwrap_or(1.0),
        }));
    }
    if let Some(aim) = constraint.aim {
        return Ok(Some(NodeConstraint {
            destination: NodeRef(node),
            source: NodeRef(aim.source),
            kind: ConstraintKind::Aim {
                axis: Axis::parse(&aim.aim_axis)
                    .ok_or_else(|| CoreError::InvalidAxis(aim.aim_axis.clone()))?,
            },
            weight: aim.weight.unwrap_or(1.0),
        }));
    }
    Ok(None)
}

fn map_mtoon(material: vrm_protocol::materials_mtoon::VrmcMaterialsMtoon) -> MtoonMaterial {
    MtoonMaterial {
        transparent_with_z_write: material.transparent_with_z_write.unwrap_or(false),
        render_queue_offset_number: material.render_queue_offset_number.unwrap_or(0),
        shade_color_factor: material.shade_color_factor.unwrap_or([0.97, 0.81, 0.86]),
        shading_shift_factor: material.shading_shift_factor.unwrap_or(0.0),
        shading_toony_factor: material.shading_toony_factor.unwrap_or(0.9),
        gi_equalization_factor: material.gi_equalization_factor.unwrap_or(0.9),
        outline_width_mode: match material.outline_width_mode.as_deref() {
            Some("worldCoordinates") => OutlineWidthMode::WorldCoordinates,
            Some("screenCoordinates") => OutlineWidthMode::ScreenCoordinates,
            Some("none") | None => OutlineWidthMode::None,
            Some(_) => OutlineWidthMode::Unknown,
        },
        outline_width_factor: material.outline_width_factor.unwrap_or(0.0),
        outline_color_factor: material.outline_color_factor.unwrap_or([0.0, 0.0, 0.0]),
        uv_animation: UvAnimation {
            scroll_x_speed: material.uv_animation_scroll_x_speed_factor.unwrap_or(0.0),
            scroll_y_speed: material.uv_animation_scroll_y_speed_factor.unwrap_or(0.0),
            rotation_speed: material.uv_animation_rotation_speed_factor.unwrap_or(0.0),
        },
    }
}

fn map_vrma(animation: vrma::VrmcVrmAnimation) -> VrmDocument {
    let mut document = VrmDocument {
        kind: VrmKind::Vrma,
        meta: Meta {
            name: "VRM Animation".to_owned(),
            ..Meta::default()
        },
        ..VrmDocument::default()
    };

    if let Some(humanoid) = animation.humanoid {
        document.humanoid = Humanoid {
            bones: humanoid
                .human_bones
                .into_iter()
                .filter_map(|(name, value)| {
                    value
                        .get("node")
                        .and_then(|node| node.as_u64())
                        .map(|node| {
                            (
                                HumanBoneName::from(name.as_str()),
                                HumanBone {
                                    node: NodeRef(node as usize),
                                    rest: Transform::default(),
                                },
                            )
                        })
                })
                .collect(),
        };
    }

    document.animation = Feature::Present(VrmAnimation::default());
    document
}

fn validate_nodes(document: &VrmDocument, node_count: usize) -> Result<(), BuildError> {
    for bone in document.humanoid.bones.values() {
        ensure_ref("node", bone.node.0, node_count)?;
    }
    if let Feature::Present(first_person) = &document.first_person {
        for annotation in &first_person.mesh_annotations {
            ensure_ref("node", annotation.node.0, node_count)?;
        }
    }
    for constraint in &document.node_constraints {
        ensure_ref("node", constraint.destination.0, node_count)?;
        ensure_ref("node", constraint.source.0, node_count)?;
    }
    if let Feature::Present(expressions) = &document.expressions {
        for expression in expressions
            .preset
            .values()
            .chain(expressions.custom.values())
        {
            for bind in &expression.binds {
                if let ExpressionBind::MorphTarget { node, .. } = bind {
                    ensure_ref("node", node.0, node_count)?;
                }
            }
        }
    }
    if let Feature::Present(spring_bone) = &document.spring_bone {
        for collider in &spring_bone.colliders {
            ensure_ref("node", collider.node.0, node_count)?;
        }
        for spring in &spring_bone.springs {
            if let Some(center) = spring.center {
                ensure_ref("node", center.0, node_count)?;
            }
            for joint in &spring.joints {
                ensure_ref("node", joint.node.0, node_count)?;
            }
        }
    }
    Ok(())
}

fn validate_materials(document: &VrmDocument, material_count: usize) -> Result<(), BuildError> {
    if let Feature::Present(expressions) = &document.expressions {
        for expression in expressions
            .preset
            .values()
            .chain(expressions.custom.values())
        {
            for bind in &expression.binds {
                match bind {
                    ExpressionBind::MaterialColor { material, .. }
                    | ExpressionBind::TextureTransform { material, .. } => {
                        ensure_ref("material", material.0, material_count)?;
                    }
                    ExpressionBind::MorphTarget { .. } => {}
                }
            }
        }
    }
    Ok(())
}

fn validate_spring_references(document: &VrmDocument) -> Result<(), BuildError> {
    let Feature::Present(spring_bone) = &document.spring_bone else {
        return Ok(());
    };

    for group in &spring_bone.collider_groups {
        for collider in &group.colliders {
            ensure_ref("collider", *collider, spring_bone.colliders.len())?;
        }
    }
    for spring in &spring_bone.springs {
        for group in &spring.collider_groups {
            ensure_ref("collider_group", *group, spring_bone.collider_groups.len())?;
        }
    }

    Ok(())
}

fn ensure_ref(kind: &'static str, index: usize, len: usize) -> Result<(), BuildError> {
    if index < len {
        Ok(())
    } else {
        Err(CoreError::ReferenceOutOfRange { kind, index, len }.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrm_protocol::{ExtensionBundle, VrmExtension};

    #[test]
    fn rejects_missing_required_humanoid_bones() {
        let bundle = ExtensionBundle {
            vrm: Some(VrmExtension::Vrm1(Box::new(vrm1::VrmcVrm {
                spec_version: "1.0".to_owned(),
                meta: vrm1::Meta {
                    name: "avatar".to_owned(),
                    authors: vec![],
                    ..Default::default()
                },
                humanoid: vrm1::Humanoid {
                    human_bones: vrm1::HumanBones {
                        bones: [("hips".to_owned(), serde_json::json!({ "node": 0 }))]
                            .into_iter()
                            .collect(),
                    },
                    ..Default::default()
                },
                first_person: None,
                look_at: None,
                expressions: None,
                extensions: None,
                extras: None,
            }))),
            ..Default::default()
        };

        let err = ValidatedAssetBuilder::new().build(bundle).unwrap_err();
        assert!(matches!(
            err,
            BuildError::Core(CoreError::MissingRequiredHumanBones)
        ));
    }
}
