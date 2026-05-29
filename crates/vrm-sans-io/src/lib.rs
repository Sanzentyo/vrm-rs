//! Side-effect-free conversion from protocol data into core VRM models.
//!
//! ```
//! use vrm_protocol::{ExtensionBundle, VrmExtension, vrma};
//! use vrm_sans_io::ValidatedAssetBuilder;
//!
//! let required_bones = [
//!     "hips", "head", "spine", "leftUpperLeg", "leftLowerLeg", "leftFoot",
//!     "rightUpperLeg", "rightLowerLeg", "rightFoot", "leftUpperArm",
//!     "leftLowerArm", "leftHand", "rightUpperArm", "rightLowerArm", "rightHand",
//! ];
//! let animation = vrma::VrmcVrmAnimation {
//!     spec_version: "1.0".to_owned(),
//!     humanoid: Some(vrma::Humanoid {
//!         human_bones: required_bones
//!             .iter()
//!             .enumerate()
//!             .map(|(node, bone)| ((*bone).to_owned(), serde_json::json!({ "node": node })))
//!             .collect(),
//!     }),
//!     ..Default::default()
//! };
//! let bundle = ExtensionBundle {
//!     vrm: Some(VrmExtension::Vrma(Box::new(animation))),
//!     ..Default::default()
//! };
//!
//! let model = ValidatedAssetBuilder::new()
//!     .with_node_count(required_bones.len())
//!     .build(bundle)
//!     .unwrap()
//!     .resolve();
//! assert!(model.document().animation.is_present());
//! ```

use glam::Vec3;
use thiserror::Error;
use vrm_core::*;
use vrm_protocol::{
    VrmExtension, khr_materials_emissive_strength, materials_hdr_emissive_multiplier,
    node_constraint, spring_bone, vrm0, vrm1, vrma,
};

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
        validate_secondary_extension_versions(&bundle)?;
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

        if let Some(count) = self.material_count {
            ensure_material_slots(&mut document.materials, count);
        }

        for (index, material) in bundle.mtoon_materials {
            if let Some(count) = self.material_count {
                ensure_ref("material", index, count)?;
            }
            let slot = material_slot(&mut document.materials, index);
            slot.name.get_or_insert_with(|| format!("material_{index}"));
            slot.mtoon = Feature::Present(map_mtoon(material));
        }

        for (index, multiplier) in bundle.hdr_emissive_multipliers {
            if let Some(count) = self.material_count {
                ensure_ref("material", index, count)?;
            }
            let slot = material_slot(&mut document.materials, index);
            slot.name.get_or_insert_with(|| format!("material_{index}"));
            slot.hdr_emissive_multiplier =
                Feature::Present(map_hdr_emissive_multiplier(multiplier));
        }

        for (index, strength) in bundle.khr_emissive_strengths {
            if let Some(count) = self.material_count {
                ensure_ref("material", index, count)?;
            }
            let slot = material_slot(&mut document.materials, index);
            slot.name.get_or_insert_with(|| format!("material_{index}"));
            slot.khr_emissive_strength = Feature::Present(map_khr_emissive_strength(strength));
        }

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
    let first_person = vrm.first_person;
    let secondary_animation = vrm.secondary_animation;
    let material_properties = vrm.material_properties.unwrap_or_default();
    let material_name_to_index = material_properties
        .iter()
        .enumerate()
        .filter_map(|(index, material)| material.name.clone().map(|name| (name, index)))
        .collect::<std::collections::HashMap<_, _>>();
    let material_queue_map = Vrm0MaterialQueueMap::from_materials(&material_properties);
    let mut document = VrmDocument {
        kind: VrmKind::Vrm0Compat,
        compatibility: Compatibility {
            vrm0: Some(Vrm0Compatibility::default()),
        },
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
                        map_vrm0_human_bone_name(&bone.bone),
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

    if let Some(first_person) = first_person {
        document.first_person = Feature::Present(FirstPerson {
            mesh_annotations: first_person
                .mesh_annotations
                .unwrap_or_default()
                .into_iter()
                .map(|annotation| FirstPersonMeshAnnotation {
                    node: NodeRef(annotation.mesh),
                    kind: map_vrm0_first_person_flag(&annotation.first_person_flag),
                })
                .collect(),
        });
        document.look_at = Feature::Present(LookAt {
            offset_from_head: vec3(first_person.first_person_bone_offset),
            kind: match first_person.look_at_type_name.as_deref() {
                Some("BlendShape") => LookAtKind::Expression,
                Some("Bone") | None => LookAtKind::Bone,
                Some(other) => LookAtKind::Unknown(other.to_owned()),
            },
            horizontal_inner: map_vrm0_degree_map(first_person.look_at_horizontal_inner),
            horizontal_outer: map_vrm0_degree_map(first_person.look_at_horizontal_outer),
            vertical_down: map_vrm0_degree_map(first_person.look_at_vertical_down),
            vertical_up: map_vrm0_degree_map(first_person.look_at_vertical_up),
        });
    }

    if let Some(blend_shape) = vrm.blend_shape_master {
        document.expressions =
            Feature::Present(map_vrm0_blend_shape(blend_shape, &material_name_to_index));
    }

    if let Some(secondary_animation) = secondary_animation {
        document.spring_bone = Feature::Present(map_vrm0_secondary_animation(secondary_animation));
    }

    document.materials = material_properties
        .into_iter()
        .map(|material| map_vrm0_material(material, &material_queue_map))
        .collect();

    Ok(document)
}

fn map_vrm0_human_bone_name(name: &str) -> HumanBoneName {
    match name {
        "leftThumbProximal" => HumanBoneName::LeftThumbMetacarpal,
        "leftThumbIntermediate" => HumanBoneName::LeftThumbProximal,
        "rightThumbProximal" => HumanBoneName::RightThumbMetacarpal,
        "rightThumbIntermediate" => HumanBoneName::RightThumbProximal,
        _ => HumanBoneName::from(name),
    }
}

fn map_vrm0_blend_shape(
    blend_shape: vrm0::BlendShape,
    material_name_to_index: &std::collections::HashMap<String, usize>,
) -> ExpressionSet {
    let mut expressions = ExpressionSet::default();

    for group in blend_shape.blend_shape_groups {
        let name = group
            .preset_name
            .clone()
            .or_else(|| group.name.clone())
            .unwrap_or_else(|| "unknown".to_owned());
        let expression = map_vrm0_blend_shape_group(group, material_name_to_index);
        if let Some(preset_name) = expression_preset_name(&name) {
            expressions.preset.insert(preset_name, expression);
        } else {
            expressions.custom.insert(name, expression);
        }
    }

    expressions
}

fn expression_preset_name(name: &str) -> Option<ExpressionName> {
    let canonical = match name {
        "a" => "aa",
        "i" => "ih",
        "u" => "ou",
        "e" => "ee",
        "o" => "oh",
        "joy" => "happy",
        "sorrow" => "sad",
        "fun" => "relaxed",
        "lookup" => "lookUp",
        "lookdown" => "lookDown",
        "lookleft" => "lookLeft",
        "lookright" => "lookRight",
        "blink_l" => "blinkLeft",
        "blink_r" => "blinkRight",
        other => other,
    };
    let expression = ExpressionName::from(canonical);
    (!matches!(expression, ExpressionName::Unknown(_))).then_some(expression)
}

fn map_vrm0_blend_shape_group(
    group: vrm0::BlendShapeGroup,
    material_name_to_index: &std::collections::HashMap<String, usize>,
) -> Expression {
    let morphs =
        group
            .binds
            .unwrap_or_default()
            .into_iter()
            .map(|bind| ExpressionBind::MorphTarget {
                node: NodeRef(bind.mesh),
                index: bind.index,
                weight: bind.weight,
            });
    let material_colors = group
        .material_values
        .unwrap_or_default()
        .into_iter()
        .filter_map(|bind| {
            material_name_to_index
                .get(&bind.material_name)
                .copied()
                .map(|material| map_vrm0_material_value_bind(material, bind))
        });

    Expression {
        is_binary: group.is_binary.unwrap_or(false),
        binds: morphs.chain(material_colors).collect(),
        ..Expression::default()
    }
}

fn map_vrm0_material_value_bind(
    material: usize,
    bind: vrm0::BlendShapeMaterialBind,
) -> ExpressionBind {
    if is_vrm0_texture_transform_property(&bind.property_name) {
        let scale =
            (bind.target_value.len() >= 2).then(|| [bind.target_value[0], bind.target_value[1]]);
        let offset = (bind.target_value.len() >= 4).then(|| {
            let x = bind.target_value[2];
            let y = if bind.property_name == "_MainTex_ST" && bind.target_value.len() >= 2 {
                1.0 - bind.target_value[3] - bind.target_value[1]
            } else {
                bind.target_value[3]
            };
            [x, y]
        });
        ExpressionBind::TextureTransform {
            material: MaterialRef(material),
            scale,
            offset,
        }
    } else {
        ExpressionBind::MaterialColor {
            material: MaterialRef(material),
            kind: bind.property_name,
            target_value: bind.target_value,
        }
    }
}

fn is_vrm0_texture_transform_property(property: &str) -> bool {
    matches!(property, "_MainTex_ST" | "_ShadeTexture_ST" | "_BumpMap_ST")
}

fn map_vrm0_degree_map(range: Option<vrm0::FirstPersonDegreeMap>) -> RangeMap {
    range.map_or_else(RangeMap::default, |range| RangeMap {
        input_max_value: range.x_range.unwrap_or(90.0),
        output_scale: range.y_range.unwrap_or(10.0),
    })
}

fn map_vrm0_first_person_flag(flag: &str) -> FirstPersonAnnotation {
    match flag {
        "Auto" | "auto" => FirstPersonAnnotation::Auto,
        "Both" | "both" => FirstPersonAnnotation::Both,
        "ThirdPersonOnly" | "thirdPersonOnly" => FirstPersonAnnotation::ThirdPersonOnly,
        "FirstPersonOnly" | "firstPersonOnly" => FirstPersonAnnotation::FirstPersonOnly,
        other => FirstPersonAnnotation::Unknown(other.to_owned()),
    }
}

fn map_vrm0_secondary_animation(animation: vrm0::SecondaryAnimation) -> SpringBoneSystem {
    let mut colliders = Vec::new();
    let collider_groups = animation
        .collider_groups
        .unwrap_or_default()
        .into_iter()
        .map(|group| {
            let start = colliders.len();
            colliders.extend(group.colliders.into_iter().map(|collider| SpringCollider {
                node: NodeRef(group.node),
                shape: ColliderShape::Sphere {
                    offset: vec3_vrm0_collider(collider.offset),
                    radius: collider.radius.unwrap_or(0.0),
                    inside: false,
                },
            }));
            SpringColliderGroup {
                name: None,
                colliders: (start..colliders.len()).collect(),
            }
        })
        .collect();

    let springs = animation
        .bone_groups
        .unwrap_or_default()
        .into_iter()
        .map(|spring| Spring {
            name: spring.comment,
            joints: spring
                .bones
                .unwrap_or_default()
                .into_iter()
                .map(|node| SpringJoint {
                    node: NodeRef(node),
                    hit_radius: spring.hit_radius.unwrap_or(0.0),
                    stiffness: spring.stiffiness.unwrap_or(1.0),
                    gravity_power: spring.gravity_power.unwrap_or(0.0),
                    gravity_dir: spring.gravity_dir.map_or(Vec3::NEG_Y, Vec3::from_array),
                    drag_force: spring.drag_force.unwrap_or(0.4),
                })
                .collect(),
            collider_groups: spring.collider_groups.unwrap_or_default(),
            center: spring.center.map(NodeRef),
        })
        .collect();

    SpringBoneSystem {
        colliders,
        collider_groups,
        springs,
    }
}

fn vec3_vrm0_collider(value: Option<[f32; 3]>) -> Vec3 {
    value.map_or(Vec3::ZERO, |value| Vec3::new(value[0], value[1], -value[2]))
}

#[derive(Clone, Debug, Default)]
struct Vrm0MaterialQueueMap {
    transparent: std::collections::HashMap<i32, i32>,
    transparent_z_write: std::collections::HashMap<i32, i32>,
}

impl Vrm0MaterialQueueMap {
    fn from_materials(materials: &[vrm0::Material]) -> Self {
        let mut transparent = std::collections::BTreeSet::new();
        let mut transparent_z_write = std::collections::BTreeSet::new();

        materials
            .iter()
            .filter(|material| is_vrm0_transparent_material(material))
            .filter_map(|material| material.render_queue.map(|queue| (material, queue)))
            .for_each(|(material, queue)| {
                if is_vrm0_z_write_enabled(material) {
                    transparent_z_write.insert(queue);
                } else {
                    transparent.insert(queue);
                }
            });

        Self {
            transparent: transparent
                .iter()
                .enumerate()
                .map(|(index, queue)| {
                    let offset = ((index as i32) - (transparent.len() as i32) + 1).clamp(-9, 0);
                    (*queue, offset)
                })
                .collect(),
            transparent_z_write: transparent_z_write
                .iter()
                .enumerate()
                .map(|(index, queue)| (*queue, (index as i32).clamp(0, 9)))
                .collect(),
        }
    }

    fn offset_for(&self, material: &vrm0::Material) -> i32 {
        if !is_vrm0_transparent_material(material) {
            return 0;
        }
        material
            .render_queue
            .and_then(|queue| {
                if is_vrm0_z_write_enabled(material) {
                    self.transparent_z_write.get(&queue).copied()
                } else {
                    self.transparent.get(&queue).copied()
                }
            })
            .unwrap_or(0)
    }
}

fn map_vrm0_material(
    material: vrm0::Material,
    material_queue_map: &Vrm0MaterialQueueMap,
) -> Material {
    let render_queue_offset_number = material_queue_map.offset_for(&material);
    let float_properties = material.float_properties.unwrap_or_default();
    let vector_properties = material.vector_properties.unwrap_or_default();
    let texture_properties = material.texture_properties.unwrap_or_default();
    let keyword_map = material.keyword_map.unwrap_or_default();
    let is_transparent =
        is_vrm0_transparent_from_parts(material.shader.as_deref(), Some(&keyword_map));
    let is_cutoff = keyword_map.contains_key("_ALPHATEST_ON");
    let queue = if is_transparent {
        MtoonRenderQueue::Transparent
    } else if is_cutoff {
        MtoonRenderQueue::AlphaTest
    } else {
        MtoonRenderQueue::Opaque
    };
    let shader = material.shader.unwrap_or_default();
    let (shading_shift_factor, shading_toony_factor) = map_vrm0_mtoon_shading(&float_properties);
    let gi_equalization_factor = map_vrm0_gi_equalization(&float_properties);
    let mtoon = shader.contains("MToon").then(|| MtoonMaterial {
        transparent_with_z_write: is_transparent
            && is_vrm0_z_write_enabled_from_parts(Some(shader.as_str()), Some(&float_properties)),
        render_queue_offset_number,
        render_queue: queue,
        cull_mode: map_vrm0_cull_mode(float_property(&float_properties, "_CullMode")),
        textures: MtoonTextureSet {
            main_texture: texture_property(&texture_properties, "_MainTex"),
            shade_multiply_texture: texture_property(&texture_properties, "_ShadeTexture"),
            shading_shift_texture: None,
            normal_texture: texture_property(&texture_properties, "_BumpMap"),
            matcap_texture: texture_property(&texture_properties, "_SphereAdd"),
            rim_multiply_texture: texture_property(&texture_properties, "_RimTexture"),
            outline_width_multiply_texture: texture_property(
                &texture_properties,
                "_OutlineWidthTexture",
            ),
            uv_animation_mask_texture: texture_property(&texture_properties, "_UvAnimMaskTexture"),
        },
        texture_transforms: MtoonTextureTransformSet::default(),
        base_color_factor: vec4_property_gamma_rgb(&vector_properties, "_Color")
            .unwrap_or(MtoonMaterial::default().base_color_factor),
        emissive_factor: vec3_property_gamma(&vector_properties, "_EmissionColor")
            .unwrap_or(MtoonMaterial::default().emissive_factor),
        cutoff_factor: float_property(&float_properties, "_Cutoff")
            .unwrap_or(MtoonMaterial::default().cutoff_factor),
        shade_color_factor: vec3_property_gamma(&vector_properties, "_ShadeColor")
            .unwrap_or_else(|| gamma_vec3([0.97, 0.81, 0.86])),
        receive_shadow_rate_factor: float_property(&float_properties, "_ReceiveShadowRate")
            .unwrap_or(MtoonMaterial::default().receive_shadow_rate_factor),
        shading_grade_rate_factor: float_property(&float_properties, "_ShadingGradeRate")
            .unwrap_or(MtoonMaterial::default().shading_grade_rate_factor),
        shading_shift_factor,
        shading_shift_texture_scale: MtoonMaterial::default().shading_shift_texture_scale,
        shading_toony_factor,
        light_color_attenuation_factor: float_property(&float_properties, "_LightColorAttenuation")
            .unwrap_or(MtoonMaterial::default().light_color_attenuation_factor),
        gi_equalization_factor,
        matcap_factor: MtoonMaterial::default().matcap_factor,
        parametric_rim_color_factor: vec3_property_gamma(&vector_properties, "_RimColor")
            .unwrap_or(MtoonMaterial::default().parametric_rim_color_factor),
        rim_lighting_mix_factor: float_property(&float_properties, "_RimLightingMix")
            .unwrap_or(MtoonMaterial::default().rim_lighting_mix_factor),
        parametric_rim_fresnel_power_factor: float_property(&float_properties, "_RimFresnelPower")
            .unwrap_or(MtoonMaterial::default().parametric_rim_fresnel_power_factor),
        parametric_rim_lift_factor: float_property(&float_properties, "_RimLift")
            .unwrap_or(MtoonMaterial::default().parametric_rim_lift_factor),
        outline_width_mode: match float_property(&float_properties, "_OutlineWidthMode")
            .unwrap_or(0.0) as i32
        {
            1 => OutlineWidthMode::WorldCoordinates,
            2 => OutlineWidthMode::ScreenCoordinates,
            _ => OutlineWidthMode::None,
        },
        outline_width_factor: 0.01
            * float_property(&float_properties, "_OutlineWidth").unwrap_or(0.0),
        outline_color_factor: vec3_property_gamma(&vector_properties, "_OutlineColor")
            .unwrap_or([0.0, 0.0, 0.0]),
        outline_lighting_mix_factor: match float_property(&float_properties, "_OutlineColorMode")
            .unwrap_or(0.0) as i32
        {
            1 => float_property(&float_properties, "_OutlineLightingMix").unwrap_or(1.0),
            _ => 0.0,
        },
        uv_animation: UvAnimation {
            scroll_x_speed: float_property(&float_properties, "_UvAnimScrollX").unwrap_or(0.0),
            scroll_y_speed: -float_property(&float_properties, "_UvAnimScrollY").unwrap_or(0.0),
            rotation_speed: float_property(&float_properties, "_UvAnimRotation").unwrap_or(0.0),
        },
    });

    Material {
        name: material.name,
        mtoon: mtoon.map_or(Feature::Absent, Feature::Present),
        ..Material::default()
    }
}

fn is_vrm0_transparent_material(material: &vrm0::Material) -> bool {
    is_vrm0_transparent_from_parts(material.shader.as_deref(), material.keyword_map.as_ref())
}

fn is_vrm0_transparent_from_parts(
    shader: Option<&str>,
    keyword_map: Option<&vrm_protocol::AnyMap>,
) -> bool {
    keyword_map.is_some_and(|map| map.contains_key("_ALPHABLEND_ON"))
        || shader == Some("VRM/UnlitTransparent")
        || shader == Some("VRM/UnlitTransparentZWrite")
}

fn is_vrm0_z_write_enabled(material: &vrm0::Material) -> bool {
    is_vrm0_z_write_enabled_from_parts(
        material.shader.as_deref(),
        material.float_properties.as_ref(),
    )
}

fn is_vrm0_z_write_enabled_from_parts(
    shader: Option<&str>,
    float_properties: Option<&vrm_protocol::AnyMap>,
) -> bool {
    shader == Some("VRM/UnlitTransparentZWrite")
        || float_properties.and_then(|map| float_property(map, "_ZWrite")) == Some(1.0)
}

fn map_vrm0_mtoon_shading(float_properties: &vrm_protocol::AnyMap) -> (f32, f32) {
    let legacy_shift = float_property(float_properties, "_ShadeShift").unwrap_or(0.0);
    let legacy_toony = float_property(float_properties, "_ShadeToony").unwrap_or(0.9);
    let shading_toony_factor = legacy_toony + (1.0 - legacy_toony) * (0.5 + 0.5 * legacy_shift);
    let shading_shift_factor = -legacy_shift - (1.0 - shading_toony_factor);
    (shading_shift_factor, shading_toony_factor)
}

fn map_vrm0_gi_equalization(float_properties: &vrm_protocol::AnyMap) -> f32 {
    let gi_intensity = float_property(float_properties, "_IndirectLightIntensity").unwrap_or(0.1);
    if gi_intensity == 0.0 {
        MtoonMaterial::default().gi_equalization_factor
    } else {
        1.0 - gi_intensity
    }
}

fn map_vrm0_cull_mode(value: Option<f32>) -> MtoonCullMode {
    match value.unwrap_or(2.0) as i32 {
        0 => MtoonCullMode::Off,
        1 => MtoonCullMode::Front,
        _ => MtoonCullMode::Back,
    }
}

fn float_property(map: &vrm_protocol::AnyMap, key: &str) -> Option<f32> {
    map.get(key)
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
}

fn vec3_property(map: &vrm_protocol::AnyMap, key: &str) -> Option<[f32; 3]> {
    let values = map.get(key)?.as_array()?;
    Some([
        values.first()?.as_f64()? as f32,
        values.get(1)?.as_f64()? as f32,
        values.get(2)?.as_f64()? as f32,
    ])
}

fn vec3_property_gamma(map: &vrm_protocol::AnyMap, key: &str) -> Option<[f32; 3]> {
    vec3_property(map, key).map(gamma_vec3)
}

fn vec4_property(map: &vrm_protocol::AnyMap, key: &str) -> Option<[f32; 4]> {
    let values = map.get(key)?.as_array()?;
    Some([
        values.first()?.as_f64()? as f32,
        values.get(1)?.as_f64()? as f32,
        values.get(2)?.as_f64()? as f32,
        values.get(3)?.as_f64()? as f32,
    ])
}

fn vec4_property_gamma_rgb(map: &vrm_protocol::AnyMap, key: &str) -> Option<[f32; 4]> {
    let mut values = vec4_property(map, key)?;
    values[0] = gamma_eotf(values[0]);
    values[1] = gamma_eotf(values[1]);
    values[2] = gamma_eotf(values[2]);
    Some(values)
}

fn gamma_vec3(values: [f32; 3]) -> [f32; 3] {
    values.map(gamma_eotf)
}

fn gamma_eotf(value: f32) -> f32 {
    value.powf(2.2)
}

fn texture_property(map: &vrm_protocol::AnyMap, key: &str) -> Option<TextureRef> {
    map.get(key)
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok())
        .map(TextureRef)
}

fn map_spring_bone(spring_bone: spring_bone::VrmcSpringBone) -> SpringBoneSystem {
    SpringBoneSystem {
        colliders: spring_bone
            .colliders
            .unwrap_or_default()
            .into_iter()
            .filter_map(|collider| {
                let inside = spring_collider_inside(&collider);
                let shape = collider
                    .shape
                    .sphere
                    .map(|sphere| ColliderShape::Sphere {
                        offset: vec3(sphere.offset),
                        radius: sphere.radius.unwrap_or(0.0),
                        inside,
                    })
                    .or_else(|| {
                        collider
                            .shape
                            .capsule
                            .map(|capsule| ColliderShape::Capsule {
                                offset: vec3(capsule.offset),
                                radius: capsule.radius.unwrap_or(0.0),
                                tail: Vec3::from_array(capsule.tail),
                                inside,
                            })
                    })
                    .or_else(|| {
                        collider.shape.plane.map(|plane| ColliderShape::Plane {
                            offset: vec3(plane.offset),
                            normal: plane.normal.map_or(Vec3::Y, Vec3::from_array),
                            inside,
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

fn spring_collider_inside(collider: &spring_bone::SpringBoneCollider) -> bool {
    collider
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("VRMC_springBone_extended_collider"))
        .and_then(|value| {
            serde_json::from_value::<spring_bone::VrmcSpringBoneExtendedCollider>(value.clone())
                .ok()
        })
        .and_then(|extension| extension.inside)
        .unwrap_or(false)
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
    let texture_transforms = MtoonTextureTransformSet {
        main_texture: None,
        shade_multiply_texture: material
            .shade_multiply_texture
            .as_ref()
            .and_then(mtoon_texture_transform),
        shading_shift_texture: material
            .shading_shift_texture
            .as_ref()
            .and_then(mtoon_shading_shift_texture_transform),
        normal_texture: None,
        matcap_texture: material
            .matcap_texture
            .as_ref()
            .and_then(mtoon_texture_transform),
        rim_multiply_texture: material
            .rim_multiply_texture
            .as_ref()
            .and_then(mtoon_texture_transform),
        outline_width_multiply_texture: material
            .outline_width_multiply_texture
            .as_ref()
            .and_then(mtoon_texture_transform),
        uv_animation_mask_texture: material
            .uv_animation_mask_texture
            .as_ref()
            .and_then(mtoon_texture_transform),
    };
    MtoonMaterial {
        transparent_with_z_write: material.transparent_with_z_write.unwrap_or(false),
        render_queue_offset_number: material.render_queue_offset_number.unwrap_or(0),
        render_queue: MtoonRenderQueue::Auto,
        cull_mode: MtoonCullMode::Back,
        textures: MtoonTextureSet {
            main_texture: None,
            shade_multiply_texture: material
                .shade_multiply_texture
                .map(|texture| TextureRef(texture.index)),
            shading_shift_texture: material
                .shading_shift_texture
                .as_ref()
                .map(|texture| TextureRef(texture.index)),
            normal_texture: None,
            matcap_texture: material
                .matcap_texture
                .map(|texture| TextureRef(texture.index)),
            rim_multiply_texture: material
                .rim_multiply_texture
                .map(|texture| TextureRef(texture.index)),
            outline_width_multiply_texture: material
                .outline_width_multiply_texture
                .map(|texture| TextureRef(texture.index)),
            uv_animation_mask_texture: material
                .uv_animation_mask_texture
                .map(|texture| TextureRef(texture.index)),
        },
        texture_transforms,
        base_color_factor: MtoonMaterial::default().base_color_factor,
        emissive_factor: MtoonMaterial::default().emissive_factor,
        cutoff_factor: MtoonMaterial::default().cutoff_factor,
        shade_color_factor: material.shade_color_factor.unwrap_or([0.97, 0.81, 0.86]),
        receive_shadow_rate_factor: MtoonMaterial::default().receive_shadow_rate_factor,
        shading_grade_rate_factor: MtoonMaterial::default().shading_grade_rate_factor,
        shading_shift_factor: material.shading_shift_factor.unwrap_or(0.0),
        shading_shift_texture_scale: material
            .shading_shift_texture
            .as_ref()
            .and_then(|texture| texture.scale)
            .unwrap_or(MtoonMaterial::default().shading_shift_texture_scale),
        shading_toony_factor: material.shading_toony_factor.unwrap_or(0.9),
        light_color_attenuation_factor: MtoonMaterial::default().light_color_attenuation_factor,
        gi_equalization_factor: material.gi_equalization_factor.unwrap_or(0.9),
        matcap_factor: material
            .matcap_factor
            .unwrap_or(MtoonMaterial::default().matcap_factor),
        parametric_rim_color_factor: material
            .parametric_rim_color_factor
            .unwrap_or(MtoonMaterial::default().parametric_rim_color_factor),
        rim_lighting_mix_factor: material
            .rim_lighting_mix_factor
            .unwrap_or(MtoonMaterial::default().rim_lighting_mix_factor),
        parametric_rim_fresnel_power_factor: material
            .parametric_rim_fresnel_power_factor
            .unwrap_or(MtoonMaterial::default().parametric_rim_fresnel_power_factor),
        parametric_rim_lift_factor: material
            .parametric_rim_lift_factor
            .unwrap_or(MtoonMaterial::default().parametric_rim_lift_factor),
        outline_width_mode: match material.outline_width_mode.as_deref() {
            Some("worldCoordinates") => OutlineWidthMode::WorldCoordinates,
            Some("screenCoordinates") => OutlineWidthMode::ScreenCoordinates,
            Some("none") | None => OutlineWidthMode::None,
            Some(_) => OutlineWidthMode::Unknown,
        },
        outline_width_factor: material.outline_width_factor.unwrap_or(0.0),
        outline_color_factor: material.outline_color_factor.unwrap_or([0.0, 0.0, 0.0]),
        outline_lighting_mix_factor: material
            .outline_lighting_mix_factor
            .unwrap_or(MtoonMaterial::default().outline_lighting_mix_factor),
        uv_animation: UvAnimation {
            scroll_x_speed: material.uv_animation_scroll_x_speed_factor.unwrap_or(0.0),
            scroll_y_speed: material.uv_animation_scroll_y_speed_factor.unwrap_or(0.0),
            rotation_speed: material.uv_animation_rotation_speed_factor.unwrap_or(0.0),
        },
    }
}

fn mtoon_texture_transform(
    texture: &vrm_protocol::materials_mtoon::TextureInfo,
) -> Option<TextureTransform2d> {
    texture_transform(texture.extensions.as_ref(), texture.tex_coord)
}

fn mtoon_shading_shift_texture_transform(
    texture: &vrm_protocol::materials_mtoon::ShadingShiftTextureInfo,
) -> Option<TextureTransform2d> {
    texture_transform(texture.extensions.as_ref(), texture.tex_coord)
}

fn texture_transform(
    extensions: Option<&vrm_protocol::ExtensionMap>,
    texture_tex_coord: Option<u32>,
) -> Option<TextureTransform2d> {
    let value = extensions?.get("KHR_texture_transform")?;
    let offset = vec2_value(value.get("offset")).unwrap_or([0.0, 0.0]);
    let scale = vec2_value(value.get("scale")).unwrap_or([1.0, 1.0]);
    let rotation = value
        .get("rotation")
        .and_then(serde_json::Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or(0.0);
    let tex_coord = value
        .get("texCoord")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .or(texture_tex_coord);
    Some(TextureTransform2d {
        offset,
        scale,
        rotation,
        tex_coord,
    })
}

fn vec2_value(value: Option<&serde_json::Value>) -> Option<[f32; 2]> {
    let array = value?.as_array()?;
    let [x, y] = array.as_slice() else {
        return None;
    };
    Some([x.as_f64()? as f32, y.as_f64()? as f32])
}

fn map_hdr_emissive_multiplier(
    multiplier: materials_hdr_emissive_multiplier::VrmcMaterialsHdrEmissiveMultiplier,
) -> HdrEmissiveMultiplier {
    HdrEmissiveMultiplier(multiplier.emissive_multiplier)
}

fn map_khr_emissive_strength(
    strength: khr_materials_emissive_strength::KhrMaterialsEmissiveStrength,
) -> EmissiveStrength {
    EmissiveStrength(strength.emissive_strength.unwrap_or(1.0))
}

fn ensure_material_slots(materials: &mut Vec<Material>, count: usize) {
    if materials.len() < count {
        materials.resize_with(count, Material::default);
    }
}

fn material_slot(materials: &mut Vec<Material>, index: usize) -> &mut Material {
    if index >= materials.len() {
        ensure_material_slots(materials, index + 1);
    }
    &mut materials[index]
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

fn validate_secondary_extension_versions(
    bundle: &vrm_protocol::ExtensionBundle,
) -> Result<(), BuildError> {
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

fn ensure_vrmc_spec_version(extension: &str, spec_version: &str) -> Result<(), BuildError> {
    if matches!(spec_version, "1.0" | "1.0-beta") {
        Ok(())
    } else {
        Err(BuildError::Protocol(format!(
            "unsupported spec version for {extension}: {spec_version}"
        )))
    }
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

    fn assert_f32_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 0.00001,
            "actual {actual} did not match expected {expected}"
        );
    }

    fn assert_vec3_close(actual: [f32; 3], expected: [f32; 3]) {
        actual
            .into_iter()
            .zip(expected)
            .for_each(|(actual, expected)| assert_f32_close(actual, expected));
    }

    fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
        actual
            .into_iter()
            .zip(expected)
            .for_each(|(actual, expected)| assert_f32_close(actual, expected));
    }

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

    #[test]
    fn maps_vrm0_secondary_animation_to_spring_bone() {
        let bundle = ExtensionBundle {
            vrm: Some(VrmExtension::Vrm0(Box::new(vrm0::Vrm {
                meta: Some(vrm0::Meta {
                    title: Some("legacy".to_owned()),
                    ..Default::default()
                }),
                humanoid: Some(vrm0::Humanoid {
                    human_bones: required_vrm0_bones(),
                    ..Default::default()
                }),
                secondary_animation: Some(vrm0::SecondaryAnimation {
                    collider_groups: Some(vec![vrm0::SecondaryAnimationColliderGroup {
                        node: 2,
                        colliders: vec![vrm0::SecondaryAnimationCollider {
                            offset: Some([0.0, 1.0, 0.5]),
                            radius: Some(0.25),
                        }],
                    }]),
                    bone_groups: Some(vec![vrm0::SecondaryAnimationSpring {
                        comment: Some("hair".to_owned()),
                        hit_radius: Some(0.05),
                        bones: Some(vec![2]),
                        collider_groups: Some(vec![0]),
                        ..Default::default()
                    }]),
                }),
                material_properties: Some(vec![vrm0::Material {
                    name: Some("legacy-mtoon".to_owned()),
                    shader: Some("VRM/MToon".to_owned()),
                    render_queue: Some(3001),
                    keyword_map: Some(
                        [("_ALPHABLEND_ON".to_owned(), serde_json::json!(true))]
                            .into_iter()
                            .collect(),
                    ),
                    float_properties: Some(
                        [
                            ("_OutlineWidthMode".to_owned(), serde_json::json!(1.0)),
                            ("_OutlineWidth".to_owned(), serde_json::json!(0.02)),
                            ("_Cutoff".to_owned(), serde_json::json!(0.42)),
                            ("_ReceiveShadowRate".to_owned(), serde_json::json!(0.8)),
                            ("_ShadingGradeRate".to_owned(), serde_json::json!(0.7)),
                            ("_ShadeShift".to_owned(), serde_json::json!(-0.2)),
                            ("_ShadeToony".to_owned(), serde_json::json!(0.65)),
                            ("_UvAnimScrollX".to_owned(), serde_json::json!(0.5)),
                            ("_UvAnimScrollY".to_owned(), serde_json::json!(-0.25)),
                            ("_UvAnimRotation".to_owned(), serde_json::json!(0.125)),
                            ("_LightColorAttenuation".to_owned(), serde_json::json!(0.6)),
                            ("_IndirectLightIntensity".to_owned(), serde_json::json!(0.4)),
                            ("_OutlineLightingMix".to_owned(), serde_json::json!(0.3)),
                            ("_RimLightingMix".to_owned(), serde_json::json!(0.35)),
                            ("_RimFresnelPower".to_owned(), serde_json::json!(2.5)),
                            ("_RimLift".to_owned(), serde_json::json!(0.15)),
                            ("_CullMode".to_owned(), serde_json::json!(0.0)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    vector_properties: Some(
                        [
                            ("_Color".to_owned(), serde_json::json!([1.0, 0.9, 0.8, 0.7])),
                            (
                                "_EmissionColor".to_owned(),
                                serde_json::json!([0.4, 0.5, 0.6, 1.0]),
                            ),
                            (
                                "_ShadeColor".to_owned(),
                                serde_json::json!([0.1, 0.2, 0.3, 1.0]),
                            ),
                            (
                                "_OutlineColor".to_owned(),
                                serde_json::json!([0.7, 0.6, 0.5, 1.0]),
                            ),
                            (
                                "_MatcapColor".to_owned(),
                                serde_json::json!([0.2, 0.3, 0.4, 1.0]),
                            ),
                            (
                                "_RimColor".to_owned(),
                                serde_json::json!([0.9, 0.8, 0.7, 1.0]),
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    texture_properties: Some(
                        [
                            ("_MainTex".to_owned(), serde_json::json!(3)),
                            ("_ShadeTexture".to_owned(), serde_json::json!(4)),
                            ("_BumpMap".to_owned(), serde_json::json!(5)),
                            ("_SphereAdd".to_owned(), serde_json::json!(6)),
                            ("_RimTexture".to_owned(), serde_json::json!(7)),
                            ("_OutlineWidthTexture".to_owned(), serde_json::json!(8)),
                            ("_UvAnimMaskTexture".to_owned(), serde_json::json!(9)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    ..Default::default()
                }]),
                ..Default::default()
            }))),
            ..Default::default()
        };

        let asset = ValidatedAssetBuilder::new()
            .with_node_count(15)
            .build(bundle)
            .unwrap();
        let spring_bone = asset.document.spring_bone.as_ref().unwrap();
        assert!(asset.document.compatibility.vrm0.is_some());
        assert_eq!(spring_bone.colliders.len(), 1);
        assert_eq!(
            spring_bone.colliders[0].shape,
            ColliderShape::Sphere {
                offset: Vec3::new(0.0, 1.0, -0.5),
                radius: 0.25,
                inside: false
            }
        );
        assert_eq!(spring_bone.collider_groups[0].colliders, vec![0]);
        assert_eq!(spring_bone.springs[0].joints[0].node, NodeRef(2));
        let mtoon = asset.document.materials[0].mtoon.as_ref().unwrap();
        assert!(mtoon.outline_enabled());
        assert_eq!(mtoon.render_order(), 3000);
        assert_eq!(mtoon.cull_mode, MtoonCullMode::Off);
        assert_eq!(mtoon.pipeline_hints().alpha_mode, MtoonAlphaMode::Blend);
        assert_vec4_close(
            mtoon.base_color_factor,
            [gamma_eotf(1.0), gamma_eotf(0.9), gamma_eotf(0.8), 0.7],
        );
        assert_vec3_close(
            mtoon.emissive_factor,
            [gamma_eotf(0.4), gamma_eotf(0.5), gamma_eotf(0.6)],
        );
        assert_eq!(mtoon.cutoff_factor, 0.42);
        assert_vec3_close(
            mtoon.shade_color_factor,
            [gamma_eotf(0.1), gamma_eotf(0.2), gamma_eotf(0.3)],
        );
        assert_eq!(mtoon.receive_shadow_rate_factor, 0.8);
        assert_eq!(mtoon.shading_grade_rate_factor, 0.7);
        assert_f32_close(mtoon.shading_shift_factor, -0.01);
        assert_f32_close(mtoon.shading_toony_factor, 0.79);
        assert_eq!(mtoon.light_color_attenuation_factor, 0.6);
        assert_f32_close(mtoon.gi_equalization_factor, 0.6);
        assert_eq!(mtoon.matcap_factor, [1.0, 1.0, 1.0]);
        assert_vec3_close(
            mtoon.parametric_rim_color_factor,
            [gamma_eotf(0.9), gamma_eotf(0.8), gamma_eotf(0.7)],
        );
        assert_eq!(mtoon.rim_lighting_mix_factor, 0.35);
        assert_eq!(mtoon.parametric_rim_fresnel_power_factor, 2.5);
        assert_eq!(mtoon.parametric_rim_lift_factor, 0.15);
        assert_f32_close(mtoon.outline_width_factor, 0.0002);
        assert_vec3_close(
            mtoon.outline_color_factor,
            [gamma_eotf(0.7), gamma_eotf(0.6), gamma_eotf(0.5)],
        );
        assert_eq!(mtoon.outline_lighting_mix_factor, 0.0);
        assert_eq!(mtoon.uv_animation.scroll_x_speed, 0.5);
        assert_eq!(mtoon.uv_animation.scroll_y_speed, 0.25);
        assert_eq!(mtoon.uv_animation.rotation_speed, 0.125);
        assert_eq!(mtoon.textures.main_texture, Some(TextureRef(3)));
        assert_eq!(mtoon.textures.shade_multiply_texture, Some(TextureRef(4)));
        assert_eq!(mtoon.textures.normal_texture, Some(TextureRef(5)));
        assert_eq!(mtoon.textures.matcap_texture, Some(TextureRef(6)));
        assert_eq!(mtoon.textures.rim_multiply_texture, Some(TextureRef(7)));
        assert_eq!(
            mtoon.textures.outline_width_multiply_texture,
            Some(TextureRef(8))
        );
        assert_eq!(
            mtoon.textures.uv_animation_mask_texture,
            Some(TextureRef(9))
        );
    }

    #[test]
    fn maps_vrm0_transparent_render_queues_like_three_vrm_v0compat() {
        fn transparent_material(name: &str, queue: i32, z_write: bool) -> vrm0::Material {
            let mut float_properties = vrm_protocol::AnyMap::default();
            if z_write {
                float_properties.insert("_ZWrite".to_owned(), serde_json::json!(1.0));
            }
            vrm0::Material {
                name: Some(name.to_owned()),
                shader: Some("VRM/MToon".to_owned()),
                render_queue: Some(queue),
                keyword_map: Some(
                    [("_ALPHABLEND_ON".to_owned(), serde_json::json!(true))]
                        .into_iter()
                        .collect(),
                ),
                float_properties: Some(float_properties),
                ..Default::default()
            }
        }

        let bundle = ExtensionBundle {
            vrm: Some(VrmExtension::Vrm0(Box::new(vrm0::Vrm {
                meta: Some(vrm0::Meta {
                    title: Some("legacy".to_owned()),
                    ..Default::default()
                }),
                humanoid: Some(vrm0::Humanoid {
                    human_bones: required_vrm0_bones(),
                    ..Default::default()
                }),
                material_properties: Some(vec![
                    transparent_material("blend-a", 3000, false),
                    transparent_material("blend-b", 3100, false),
                    transparent_material("blend-c", 3200, false),
                    transparent_material("zwrite-a", 3000, true),
                    transparent_material("zwrite-b", 3100, true),
                    vrm0::Material {
                        name: Some("queue-alone-is-opaque".to_owned()),
                        shader: Some("VRM/MToon".to_owned()),
                        render_queue: Some(2999),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }))),
            ..Default::default()
        };

        let asset = ValidatedAssetBuilder::new()
            .with_node_count(15)
            .build(bundle)
            .unwrap();
        let mtoon = |index: usize| asset.document.materials[index].mtoon.as_ref().unwrap();

        assert_eq!(mtoon(0).render_queue_offset_number, -2);
        assert_eq!(mtoon(1).render_queue_offset_number, -1);
        assert_eq!(mtoon(2).render_queue_offset_number, 0);
        assert_eq!(mtoon(0).render_order(), 2998);
        assert_eq!(mtoon(1).render_order(), 2999);
        assert_eq!(mtoon(2).render_order(), 3000);

        assert_eq!(mtoon(3).render_queue_offset_number, 0);
        assert_eq!(mtoon(4).render_queue_offset_number, 1);
        assert!(mtoon(3).pipeline_hints().depth_write);
        assert!(mtoon(4).pipeline_hints().depth_write);

        assert_eq!(mtoon(5).render_queue, MtoonRenderQueue::Opaque);
        assert_eq!(mtoon(5).render_queue_offset_number, 0);
        assert_eq!(mtoon(5).render_order(), 2000);
    }

    #[test]
    fn maps_vrm0_blend_shape_material_values_and_thumb_aliases() {
        let mut bones = required_vrm0_bones();
        bones.push(vrm0::HumanBone {
            bone: "leftThumbProximal".to_owned(),
            node: 14,
            use_default_values: None,
            min: None,
            max: None,
            center: None,
            axis_length: None,
        });
        bones.push(vrm0::HumanBone {
            bone: "leftThumbIntermediate".to_owned(),
            node: 15,
            use_default_values: None,
            min: None,
            max: None,
            center: None,
            axis_length: None,
        });
        let bundle = ExtensionBundle {
            vrm: Some(VrmExtension::Vrm0(Box::new(vrm0::Vrm {
                humanoid: Some(vrm0::Humanoid {
                    human_bones: bones,
                    ..Default::default()
                }),
                blend_shape_master: Some(vrm0::BlendShape {
                    blend_shape_groups: vec![
                        vrm0::BlendShapeGroup {
                            name: Some("Blink".to_owned()),
                            preset_name: Some("blink".to_owned()),
                            binds: Some(vec![vrm0::BlendShapeBind {
                                mesh: 2,
                                index: 1,
                                weight: 75.0,
                            }]),
                            material_values: Some(vec![
                                vrm0::BlendShapeMaterialBind {
                                    material_name: "face".to_owned(),
                                    property_name: "_Color".to_owned(),
                                    target_value: vec![1.0, 0.5, 0.25, 1.0],
                                },
                                vrm0::BlendShapeMaterialBind {
                                    material_name: "face".to_owned(),
                                    property_name: "_MainTex_ST".to_owned(),
                                    target_value: vec![2.0, 3.0, 0.1, 0.2],
                                },
                                vrm0::BlendShapeMaterialBind {
                                    material_name: "face".to_owned(),
                                    property_name: "_ShadeTexture_ST".to_owned(),
                                    target_value: vec![4.0, 5.0, 0.3, 0.4],
                                },
                                vrm0::BlendShapeMaterialBind {
                                    material_name: "face".to_owned(),
                                    property_name: "_BumpMap_ST".to_owned(),
                                    target_value: vec![6.0, 7.0, 0.5, 0.6],
                                },
                            ]),
                            is_binary: Some(true),
                        },
                        vrm0::BlendShapeGroup {
                            name: Some("customSmile".to_owned()),
                            preset_name: None,
                            binds: None,
                            material_values: None,
                            is_binary: None,
                        },
                    ],
                }),
                material_properties: Some(vec![vrm0::Material {
                    name: Some("face".to_owned()),
                    ..Default::default()
                }]),
                ..Default::default()
            }))),
            ..Default::default()
        };

        let asset = ValidatedAssetBuilder::new()
            .with_node_count(16)
            .with_material_count(1)
            .build(bundle)
            .unwrap();

        assert!(
            asset
                .document
                .humanoid
                .bones
                .contains_key(&HumanBoneName::LeftThumbMetacarpal)
        );
        assert!(
            asset
                .document
                .humanoid
                .bones
                .contains_key(&HumanBoneName::LeftThumbProximal)
        );
        let expressions = asset.document.expressions.as_ref().unwrap();
        let blink = expressions.preset.get(&ExpressionName::Blink).unwrap();
        assert!(blink.is_binary);
        assert!(matches!(
            blink.binds.as_slice(),
            [
                ExpressionBind::MorphTarget {
                    node: NodeRef(2),
                    index: 1,
                    weight
                },
                ExpressionBind::MaterialColor {
                    material: MaterialRef(0),
                    kind,
                    target_value
                },
                ExpressionBind::TextureTransform {
                    material: MaterialRef(0),
                    scale: Some([2.0, 3.0]),
                    offset: Some([0.1, -2.2])
                },
                ExpressionBind::TextureTransform {
                    material: MaterialRef(0),
                    scale: Some([4.0, 5.0]),
                    offset: Some([0.3, 0.4])
                },
                ExpressionBind::TextureTransform {
                    material: MaterialRef(0),
                    scale: Some([6.0, 7.0]),
                    offset: Some([0.5, 0.6])
                }
            ] if *weight == 75.0 && kind == "_Color" && target_value == &vec![1.0, 0.5, 0.25, 1.0]
        ));
        assert!(expressions.custom.contains_key("customSmile"));
    }

    #[test]
    fn maps_vrm0_legacy_expression_preset_aliases() {
        let legacy_presets = [
            ("Neutral", "neutral", ExpressionName::Neutral),
            ("A", "a", ExpressionName::Aa),
            ("I", "i", ExpressionName::Ih),
            ("U", "u", ExpressionName::Ou),
            ("E", "e", ExpressionName::Ee),
            ("O", "o", ExpressionName::Oh),
            ("Blink", "blink", ExpressionName::Blink),
            ("Joy", "joy", ExpressionName::Happy),
            ("Angry", "angry", ExpressionName::Angry),
            ("Sorrow", "sorrow", ExpressionName::Sad),
            ("Fun", "fun", ExpressionName::Relaxed),
            ("LookUp", "lookup", ExpressionName::LookUp),
            ("LookDown", "lookdown", ExpressionName::LookDown),
            ("LookLeft", "lookleft", ExpressionName::LookLeft),
            ("LookRight", "lookright", ExpressionName::LookRight),
            ("Blink_L", "blink_l", ExpressionName::BlinkLeft),
            ("Blink_R", "blink_r", ExpressionName::BlinkRight),
        ];
        let bundle = ExtensionBundle {
            vrm: Some(VrmExtension::Vrm0(Box::new(vrm0::Vrm {
                humanoid: Some(vrm0::Humanoid {
                    human_bones: required_vrm0_bones(),
                    ..Default::default()
                }),
                blend_shape_master: Some(vrm0::BlendShape {
                    blend_shape_groups: legacy_presets
                        .iter()
                        .enumerate()
                        .map(|(index, (name, preset_name, _))| vrm0::BlendShapeGroup {
                            name: Some((*name).to_owned()),
                            preset_name: Some((*preset_name).to_owned()),
                            binds: Some(vec![vrm0::BlendShapeBind {
                                mesh: 1,
                                index,
                                weight: 100.0,
                            }]),
                            ..Default::default()
                        })
                        .collect(),
                }),
                ..Default::default()
            }))),
            ..Default::default()
        };

        let asset = ValidatedAssetBuilder::new()
            .with_node_count(15)
            .build(bundle)
            .unwrap();
        let expressions = asset.document.expressions.as_ref().unwrap();

        for (_, _, expected) in legacy_presets {
            assert!(
                expressions.preset.contains_key(&expected),
                "missing mapped VRM0 preset {}",
                expected.as_str()
            );
        }
        assert!(expressions.custom.is_empty());
    }

    #[test]
    fn maps_vrm0_legacy_first_person_flags_and_look_at_ranges() {
        let bundle = ExtensionBundle {
            vrm: Some(VrmExtension::Vrm0(Box::new(vrm0::Vrm {
                humanoid: Some(vrm0::Humanoid {
                    human_bones: required_vrm0_bones(),
                    ..Default::default()
                }),
                first_person: Some(vrm0::FirstPerson {
                    first_person_bone_offset: Some([0.0, 0.1, 0.2]),
                    mesh_annotations: Some(vec![
                        vrm0::FirstPersonMeshAnnotation {
                            mesh: 1,
                            first_person_flag: "Auto".to_owned(),
                        },
                        vrm0::FirstPersonMeshAnnotation {
                            mesh: 2,
                            first_person_flag: "Both".to_owned(),
                        },
                        vrm0::FirstPersonMeshAnnotation {
                            mesh: 3,
                            first_person_flag: "ThirdPersonOnly".to_owned(),
                        },
                        vrm0::FirstPersonMeshAnnotation {
                            mesh: 4,
                            first_person_flag: "FirstPersonOnly".to_owned(),
                        },
                        vrm0::FirstPersonMeshAnnotation {
                            mesh: 5,
                            first_person_flag: "auto".to_owned(),
                        },
                        vrm0::FirstPersonMeshAnnotation {
                            mesh: 6,
                            first_person_flag: "thirdPersonOnly".to_owned(),
                        },
                        vrm0::FirstPersonMeshAnnotation {
                            mesh: 7,
                            first_person_flag: "firstPersonOnly".to_owned(),
                        },
                        vrm0::FirstPersonMeshAnnotation {
                            mesh: 8,
                            first_person_flag: "UnknownFlag".to_owned(),
                        },
                    ]),
                    look_at_type_name: Some("BlendShape".to_owned()),
                    look_at_horizontal_inner: Some(vrm0::FirstPersonDegreeMap {
                        x_range: Some(45.0),
                        y_range: Some(12.0),
                        ..Default::default()
                    }),
                    look_at_horizontal_outer: Some(vrm0::FirstPersonDegreeMap {
                        x_range: Some(60.0),
                        y_range: Some(15.0),
                        ..Default::default()
                    }),
                    look_at_vertical_down: Some(vrm0::FirstPersonDegreeMap::default()),
                    look_at_vertical_up: Some(vrm0::FirstPersonDegreeMap {
                        x_range: Some(30.0),
                        y_range: Some(8.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }))),
            ..Default::default()
        };

        let asset = ValidatedAssetBuilder::new()
            .with_node_count(15)
            .build(bundle)
            .unwrap();
        let first_person = asset.document.first_person.as_ref().unwrap();
        let annotations = first_person
            .mesh_annotations
            .iter()
            .map(|annotation| (annotation.node, annotation.kind.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            annotations,
            vec![
                (NodeRef(1), FirstPersonAnnotation::Auto),
                (NodeRef(2), FirstPersonAnnotation::Both),
                (NodeRef(3), FirstPersonAnnotation::ThirdPersonOnly),
                (NodeRef(4), FirstPersonAnnotation::FirstPersonOnly),
                (NodeRef(5), FirstPersonAnnotation::Auto),
                (NodeRef(6), FirstPersonAnnotation::ThirdPersonOnly),
                (NodeRef(7), FirstPersonAnnotation::FirstPersonOnly),
                (
                    NodeRef(8),
                    FirstPersonAnnotation::Unknown("UnknownFlag".to_owned())
                ),
            ]
        );
        let look_at = asset.document.look_at.as_ref().unwrap();
        assert_eq!(look_at.kind, LookAtKind::Expression);
        assert_eq!(look_at.offset_from_head, Vec3::new(0.0, 0.1, 0.2));
        assert_eq!(look_at.horizontal_inner.input_max_value, 45.0);
        assert_eq!(look_at.horizontal_inner.output_scale, 12.0);
        assert_eq!(look_at.horizontal_outer.input_max_value, 60.0);
        assert_eq!(look_at.horizontal_outer.output_scale, 15.0);
        assert_eq!(look_at.vertical_down.input_max_value, 90.0);
        assert_eq!(look_at.vertical_down.output_scale, 10.0);
        assert_eq!(look_at.vertical_up.input_max_value, 30.0);
        assert_eq!(look_at.vertical_up.output_scale, 8.0);
    }

    #[test]
    fn maps_material_extensions_by_gltf_material_index() {
        let bundle = ExtensionBundle {
            vrm: Some(VrmExtension::Vrm1(Box::new(vrm1::VrmcVrm {
                spec_version: "1.0".to_owned(),
                meta: vrm1::Meta {
                    name: "avatar".to_owned(),
                    authors: vec!["vrm-rs".to_owned()],
                    ..Default::default()
                },
                humanoid: vrm1::Humanoid {
                    human_bones: vrm1::HumanBones {
                        bones: required_vrm1_bones(),
                    },
                    ..Default::default()
                },
                first_person: None,
                look_at: None,
                expressions: None,
                extensions: None,
                extras: None,
            }))),
            mtoon_materials: [(
                2,
                vrm_protocol::materials_mtoon::VrmcMaterialsMtoon {
                    spec_version: "1.0".to_owned(),
                    shade_color_factor: Some([0.1, 0.2, 0.3]),
                    shading_shift_factor: Some(-0.2),
                    shading_shift_texture: Some(
                        vrm_protocol::materials_mtoon::ShadingShiftTextureInfo {
                            index: 11,
                            tex_coord: Some(0),
                            scale: Some(0.5),
                            extensions: Some(
                                [(
                                    "KHR_texture_transform".to_owned(),
                                    serde_json::json!({
                                        "offset": [0.25, 0.5],
                                        "scale": [2.0, 3.0],
                                        "rotation": 0.125,
                                        "texCoord": 1
                                    }),
                                )]
                                .into_iter()
                                .collect(),
                            ),
                            extras: None,
                        },
                    ),
                    shading_toony_factor: Some(0.7),
                    gi_equalization_factor: Some(0.4),
                    matcap_factor: Some([0.5, 0.6, 0.7]),
                    parametric_rim_color_factor: Some([0.8, 0.9, 1.0]),
                    rim_lighting_mix_factor: Some(0.25),
                    parametric_rim_fresnel_power_factor: Some(3.0),
                    parametric_rim_lift_factor: Some(0.15),
                    outline_width_mode: Some("worldCoordinates".to_owned()),
                    outline_width_factor: Some(0.01),
                    outline_color_factor: Some([0.2, 0.3, 0.4]),
                    outline_lighting_mix_factor: Some(0.35),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            hdr_emissive_multipliers: [(
                1,
                vrm_protocol::materials_hdr_emissive_multiplier::VrmcMaterialsHdrEmissiveMultiplier {
                    emissive_multiplier: 3.0,
                    extensions: None,
                    extras: None,
                },
            )]
            .into_iter()
            .collect(),
            khr_emissive_strengths: [(
                1,
                vrm_protocol::khr_materials_emissive_strength::KhrMaterialsEmissiveStrength {
                    emissive_strength: Some(5.0),
                    extensions: None,
                    extras: None,
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let asset = ValidatedAssetBuilder::new()
            .with_node_count(15)
            .with_material_count(3)
            .build(bundle)
            .unwrap();

        assert_eq!(asset.document.materials.len(), 3);
        assert!(asset.document.materials[0].mtoon.as_ref().is_none());
        let (strength, source) = asset.document.materials[1].effective_emissive_strength();
        assert_eq!(strength.0, 5.0);
        assert_eq!(source, EmissiveStrengthSource::KhrMaterialsEmissiveStrength);
        let mtoon = asset.document.materials[2].mtoon.as_ref().unwrap();
        assert_eq!(mtoon.shade_color_factor, [0.1, 0.2, 0.3]);
        assert_eq!(mtoon.shading_shift_factor, -0.2);
        assert_eq!(mtoon.shading_shift_texture_scale, 0.5);
        assert_eq!(mtoon.textures.shading_shift_texture, Some(TextureRef(11)));
        assert_eq!(
            mtoon.texture_transforms.shading_shift_texture,
            Some(TextureTransform2d {
                offset: [0.25, 0.5],
                scale: [2.0, 3.0],
                rotation: 0.125,
                tex_coord: Some(1)
            })
        );
        assert_eq!(mtoon.shading_toony_factor, 0.7);
        assert_eq!(mtoon.gi_equalization_factor, 0.4);
        assert_eq!(mtoon.matcap_factor, [0.5, 0.6, 0.7]);
        assert_eq!(mtoon.parametric_rim_color_factor, [0.8, 0.9, 1.0]);
        assert_eq!(mtoon.rim_lighting_mix_factor, 0.25);
        assert_eq!(mtoon.parametric_rim_fresnel_power_factor, 3.0);
        assert_eq!(mtoon.parametric_rim_lift_factor, 0.15);
        assert_eq!(mtoon.outline_color_factor, [0.2, 0.3, 0.4]);
        assert_eq!(mtoon.outline_lighting_mix_factor, 0.35);
    }

    #[test]
    fn empty_khr_emissive_strength_extension_defaults_to_one_and_takes_precedence() {
        let bundle = ExtensionBundle {
            vrm: Some(VrmExtension::Vrm1(Box::new(vrm1::VrmcVrm {
                spec_version: "1.0".to_owned(),
                meta: vrm1::Meta {
                    name: "avatar".to_owned(),
                    authors: vec!["vrm-rs".to_owned()],
                    ..Default::default()
                },
                humanoid: vrm1::Humanoid {
                    human_bones: vrm1::HumanBones {
                        bones: required_vrm1_bones(),
                    },
                    ..Default::default()
                },
                first_person: None,
                look_at: None,
                expressions: None,
                extensions: None,
                extras: None,
            }))),
            hdr_emissive_multipliers: [(
                0,
                vrm_protocol::materials_hdr_emissive_multiplier::VrmcMaterialsHdrEmissiveMultiplier {
                    emissive_multiplier: 3.0,
                    extensions: None,
                    extras: None,
                },
            )]
            .into_iter()
            .collect(),
            khr_emissive_strengths: [(
                0,
                vrm_protocol::khr_materials_emissive_strength::KhrMaterialsEmissiveStrength {
                    emissive_strength: None,
                    extensions: None,
                    extras: None,
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let asset = ValidatedAssetBuilder::new()
            .with_node_count(15)
            .with_material_count(1)
            .build(bundle)
            .unwrap();

        assert_eq!(
            asset.document.materials[0].effective_emissive_strength(),
            (
                EmissiveStrength(1.0),
                EmissiveStrengthSource::KhrMaterialsEmissiveStrength
            )
        );
    }

    #[test]
    fn rejects_unsupported_secondary_extension_spec_versions_in_sans_io() {
        let mut bundle = vrm1_bundle();
        bundle.spring_bone = Some(vrm_protocol::spring_bone::VrmcSpringBone {
            spec_version: "2.0".to_owned(),
            ..Default::default()
        });

        let err = ValidatedAssetBuilder::new()
            .with_node_count(15)
            .build(bundle)
            .unwrap_err();

        assert!(
            matches!(err, BuildError::Protocol(message) if message.contains("VRMC_springBone") && message.contains("2.0"))
        );
    }

    #[test]
    fn maps_spring_extended_collider_inside_flag() {
        let mut bundle = vrm1_bundle();
        bundle.spring_bone = Some(vrm_protocol::spring_bone::VrmcSpringBone {
            spec_version: "1.0".to_owned(),
            colliders: Some(vec![vrm_protocol::spring_bone::SpringBoneCollider {
                node: 1,
                shape: vrm_protocol::spring_bone::SpringBoneColliderShape {
                    sphere: Some(vrm_protocol::spring_bone::SpringBoneColliderSphere {
                        offset: Some([0.0, 1.0, 0.0]),
                        radius: Some(0.5),
                    }),
                    capsule: None,
                    plane: None,
                },
                extensions: Some(
                    [(
                        "VRMC_springBone_extended_collider".to_owned(),
                        serde_json::json!({
                            "specVersion": "1.0",
                            "inside": true
                        }),
                    )]
                    .into_iter()
                    .collect(),
                ),
                extras: None,
            }]),
            ..Default::default()
        });

        let asset = ValidatedAssetBuilder::new()
            .with_node_count(15)
            .build(bundle)
            .unwrap();

        assert_eq!(
            asset.document.spring_bone.as_ref().unwrap().colliders[0].shape,
            ColliderShape::Sphere {
                offset: Vec3::Y,
                radius: 0.5,
                inside: true,
            }
        );
    }

    #[test]
    fn material_slot_growth_preserves_existing_vrm0_materials() {
        let bundle = ExtensionBundle {
            vrm: Some(VrmExtension::Vrm0(Box::new(vrm0::Vrm {
                humanoid: Some(vrm0::Humanoid {
                    human_bones: required_vrm0_bones(),
                    ..Default::default()
                }),
                material_properties: Some(vec![
                    vrm0::Material {
                        name: Some("legacy_0".to_owned()),
                        ..Default::default()
                    },
                    vrm0::Material {
                        name: Some("legacy_1".to_owned()),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }))),
            mtoon_materials: [(
                2,
                vrm_protocol::materials_mtoon::VrmcMaterialsMtoon {
                    spec_version: "1.0".to_owned(),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let asset = ValidatedAssetBuilder::new()
            .with_node_count(15)
            .with_material_count(3)
            .build(bundle)
            .unwrap();

        assert_eq!(
            asset.document.materials[0].name.as_deref(),
            Some("legacy_0")
        );
        assert_eq!(
            asset.document.materials[1].name.as_deref(),
            Some("legacy_1")
        );
        assert!(asset.document.materials[2].mtoon.is_present());
    }

    fn vrm1_bundle() -> ExtensionBundle {
        ExtensionBundle {
            vrm: Some(VrmExtension::Vrm1(Box::new(vrm1::VrmcVrm {
                spec_version: "1.0".to_owned(),
                meta: vrm1::Meta {
                    name: "avatar".to_owned(),
                    authors: vec!["vrm-rs".to_owned()],
                    ..Default::default()
                },
                humanoid: vrm1::Humanoid {
                    human_bones: vrm1::HumanBones {
                        bones: required_vrm1_bones(),
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
        }
    }

    fn required_vrm0_bones() -> Vec<vrm0::HumanBone> {
        [
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
        ]
        .into_iter()
        .map(|(bone, node)| vrm0::HumanBone {
            bone: bone.to_owned(),
            node,
            use_default_values: None,
            min: None,
            max: None,
            center: None,
            axis_length: None,
        })
        .collect()
    }

    fn required_vrm1_bones() -> vrm_protocol::AnyMap {
        [
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
        ]
        .into_iter()
        .map(|(bone, node)| (bone.to_owned(), serde_json::json!({ "node": node })))
        .collect()
    }
}
