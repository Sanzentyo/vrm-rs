//! Serde-first protocol types for VRM, VRMC extensions, and VRMA.
//!
//! These types mirror wire data. They deliberately avoid renderer, scene graph,
//! and runtime concerns.

use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;

pub mod vrm0 {
    use super::{AnyMap, ExtensionMap};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Vrm {
        pub exporter_version: Option<String>,
        pub spec_version: Option<String>,
        pub meta: Option<Meta>,
        pub humanoid: Option<Humanoid>,
        pub first_person: Option<FirstPerson>,
        pub blend_shape_master: Option<BlendShape>,
        pub secondary_animation: Option<SecondaryAnimation>,
        pub material_properties: Option<Vec<Material>>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Meta {
        pub title: Option<String>,
        pub version: Option<String>,
        pub author: Option<String>,
        pub contact_information: Option<String>,
        pub reference: Option<String>,
        pub texture: Option<usize>,
        pub allowed_user_name: Option<String>,
        pub violent_usage_name: Option<String>,
        pub sexual_usage_name: Option<String>,
        pub commercial_usage_name: Option<String>,
        pub other_permission_url: Option<String>,
        pub license_name: Option<String>,
        pub other_license_url: Option<String>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Humanoid {
        pub human_bones: Vec<HumanBone>,
        pub arm_stretch: Option<f32>,
        pub leg_stretch: Option<f32>,
        pub upper_arm_twist: Option<f32>,
        pub lower_arm_twist: Option<f32>,
        pub upper_leg_twist: Option<f32>,
        pub lower_leg_twist: Option<f32>,
        pub feet_spacing: Option<f32>,
        pub has_translation_dof: Option<bool>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct HumanBone {
        pub bone: String,
        pub node: usize,
        pub use_default_values: Option<bool>,
        pub min: Option<[f32; 3]>,
        pub max: Option<[f32; 3]>,
        pub center: Option<[f32; 3]>,
        pub axis_length: Option<f32>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct FirstPerson {
        pub first_person_bone: Option<usize>,
        pub first_person_bone_offset: Option<[f32; 3]>,
        pub mesh_annotations: Option<Vec<FirstPersonMeshAnnotation>>,
        pub look_at_type_name: Option<String>,
        pub look_at_horizontal_inner: Option<FirstPersonDegreeMap>,
        pub look_at_horizontal_outer: Option<FirstPersonDegreeMap>,
        pub look_at_vertical_down: Option<FirstPersonDegreeMap>,
        pub look_at_vertical_up: Option<FirstPersonDegreeMap>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct FirstPersonMeshAnnotation {
        pub mesh: usize,
        pub first_person_flag: String,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct FirstPersonDegreeMap {
        pub curve: Option<[f32; 8]>,
        pub x_range: Option<f32>,
        pub y_range: Option<f32>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct BlendShape {
        pub blend_shape_groups: Vec<BlendShapeGroup>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct BlendShapeGroup {
        pub name: Option<String>,
        pub preset_name: Option<String>,
        pub binds: Option<Vec<BlendShapeBind>>,
        pub material_values: Option<Vec<BlendShapeMaterialBind>>,
        pub is_binary: Option<bool>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct BlendShapeBind {
        pub mesh: usize,
        pub index: usize,
        pub weight: f32,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct BlendShapeMaterialBind {
        pub material_name: String,
        pub property_name: String,
        pub target_value: Vec<f32>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SecondaryAnimation {
        pub bone_groups: Option<Vec<SecondaryAnimationSpring>>,
        pub collider_groups: Option<Vec<SecondaryAnimationColliderGroup>>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SecondaryAnimationSpring {
        pub comment: Option<String>,
        pub stiffiness: Option<f32>,
        pub gravity_power: Option<f32>,
        pub gravity_dir: Option<[f32; 3]>,
        pub drag_force: Option<f32>,
        pub center: Option<usize>,
        pub hit_radius: Option<f32>,
        pub bones: Option<Vec<usize>>,
        pub collider_groups: Option<Vec<usize>>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SecondaryAnimationColliderGroup {
        pub node: usize,
        pub colliders: Vec<SecondaryAnimationCollider>,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SecondaryAnimationCollider {
        pub offset: Option<[f32; 3]>,
        pub radius: Option<f32>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Material {
        pub name: Option<String>,
        pub shader: Option<String>,
        pub render_queue: Option<i32>,
        pub float_properties: Option<AnyMap>,
        pub vector_properties: Option<AnyMap>,
        pub texture_properties: Option<AnyMap>,
        pub keyword_map: Option<AnyMap>,
        pub tag_map: Option<AnyMap>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }
}

pub mod vrm1 {
    use super::{AnyMap, ExtensionMap};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct VrmcVrm {
        pub spec_version: String,
        pub meta: Meta,
        pub humanoid: Humanoid,
        pub first_person: Option<FirstPerson>,
        pub look_at: Option<LookAt>,
        pub expressions: Option<Expressions>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Meta {
        pub name: String,
        pub version: Option<String>,
        pub authors: Vec<String>,
        pub copyright_information: Option<String>,
        pub contact_information: Option<String>,
        pub references: Option<Vec<String>>,
        pub third_party_licenses: Option<String>,
        pub thumbnail_image: Option<usize>,
        pub license_url: Option<String>,
        pub avatar_permission: Option<String>,
        pub allow_excessively_violent_usage: Option<bool>,
        pub allow_excessively_sexual_usage: Option<bool>,
        pub commercial_usage: Option<String>,
        pub allow_political_or_religious_usage: Option<bool>,
        pub allow_antisocial_or_hate_usage: Option<bool>,
        pub credit_notation: Option<String>,
        pub allow_redistribution: Option<bool>,
        pub modification: Option<String>,
        pub other_license_url: Option<String>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Humanoid {
        pub human_bones: HumanBones,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct HumanBones {
        #[serde(flatten)]
        pub bones: AnyMap,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct HumanBone {
        pub node: usize,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct FirstPerson {
        pub mesh_annotations: Option<Vec<FirstPersonMeshAnnotation>>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct FirstPersonMeshAnnotation {
        pub node: usize,
        #[serde(rename = "type")]
        pub kind: String,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct LookAt {
        pub offset_from_head_bone: Option<[f32; 3]>,
        #[serde(rename = "type")]
        pub kind: Option<String>,
        pub range_map_horizontal_inner: Option<LookAtRangeMap>,
        pub range_map_horizontal_outer: Option<LookAtRangeMap>,
        pub range_map_vertical_down: Option<LookAtRangeMap>,
        pub range_map_vertical_up: Option<LookAtRangeMap>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct LookAtRangeMap {
        pub input_max_value: f32,
        pub output_scale: f32,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Expressions {
        pub preset: Option<AnyMap>,
        pub custom: Option<AnyMap>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Expression {
        pub morph_target_binds: Option<Vec<ExpressionMorphTargetBind>>,
        pub material_color_binds: Option<Vec<ExpressionMaterialColorBind>>,
        pub texture_transform_binds: Option<Vec<ExpressionTextureTransformBind>>,
        pub is_binary: Option<bool>,
        pub override_blink: Option<String>,
        pub override_look_at: Option<String>,
        pub override_mouth: Option<String>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ExpressionMorphTargetBind {
        pub node: usize,
        pub index: usize,
        pub weight: f32,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ExpressionMaterialColorBind {
        pub material: usize,
        #[serde(rename = "type")]
        pub kind: String,
        pub target_value: Vec<f32>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ExpressionTextureTransformBind {
        pub material: usize,
        pub scale: Option<[f32; 2]>,
        pub offset: Option<[f32; 2]>,
    }
}

pub mod spring_bone {
    use super::ExtensionMap;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct VrmcSpringBone {
        pub spec_version: String,
        pub colliders: Option<Vec<SpringBoneCollider>>,
        pub collider_groups: Option<Vec<SpringBoneColliderGroup>>,
        pub springs: Option<Vec<SpringBoneSpring>>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpringBoneCollider {
        pub node: usize,
        pub shape: SpringBoneColliderShape,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct VrmcSpringBoneExtendedCollider {
        pub spec_version: Option<String>,
        pub inside: Option<bool>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpringBoneColliderShape {
        pub sphere: Option<SpringBoneColliderSphere>,
        pub capsule: Option<SpringBoneColliderCapsule>,
        pub plane: Option<SpringBoneColliderPlane>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpringBoneColliderSphere {
        pub offset: Option<[f32; 3]>,
        pub radius: Option<f32>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpringBoneColliderCapsule {
        pub offset: Option<[f32; 3]>,
        pub radius: Option<f32>,
        pub tail: [f32; 3],
    }

    #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpringBoneColliderPlane {
        pub offset: Option<[f32; 3]>,
        pub normal: Option<[f32; 3]>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpringBoneColliderGroup {
        pub name: Option<String>,
        pub colliders: Vec<usize>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpringBoneSpring {
        pub name: Option<String>,
        pub joints: Vec<SpringBoneJoint>,
        pub collider_groups: Option<Vec<usize>>,
        pub center: Option<usize>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpringBoneJoint {
        pub node: usize,
        pub hit_radius: Option<f32>,
        pub stiffness: Option<f32>,
        pub gravity_power: Option<f32>,
        pub gravity_dir: Option<[f32; 3]>,
        pub drag_force: Option<f32>,
    }
}

pub mod node_constraint {
    use super::ExtensionMap;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct VrmcNodeConstraint {
        pub spec_version: String,
        pub constraint: Constraint,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Constraint {
        pub roll: Option<RollConstraint>,
        pub aim: Option<AimConstraint>,
        pub rotation: Option<RotationConstraint>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RollConstraint {
        pub source: usize,
        pub roll_axis: String,
        pub weight: Option<f32>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct AimConstraint {
        pub source: usize,
        pub aim_axis: String,
        pub weight: Option<f32>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RotationConstraint {
        pub source: usize,
        pub weight: Option<f32>,
    }
}

pub mod materials_mtoon {
    use super::ExtensionMap;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct VrmcMaterialsMtoon {
        pub spec_version: String,
        pub transparent_with_z_write: Option<bool>,
        pub render_queue_offset_number: Option<i32>,
        pub shade_color_factor: Option<[f32; 3]>,
        pub shade_multiply_texture: Option<TextureInfo>,
        pub shading_shift_factor: Option<f32>,
        pub shading_shift_texture: Option<ShadingShiftTextureInfo>,
        pub shading_toony_factor: Option<f32>,
        pub gi_equalization_factor: Option<f32>,
        pub matcap_factor: Option<[f32; 3]>,
        pub matcap_texture: Option<TextureInfo>,
        pub parametric_rim_color_factor: Option<[f32; 3]>,
        pub rim_multiply_texture: Option<TextureInfo>,
        pub rim_lighting_mix_factor: Option<f32>,
        pub parametric_rim_fresnel_power_factor: Option<f32>,
        pub parametric_rim_lift_factor: Option<f32>,
        pub outline_width_mode: Option<String>,
        pub outline_width_factor: Option<f32>,
        pub outline_width_multiply_texture: Option<TextureInfo>,
        pub outline_color_factor: Option<[f32; 3]>,
        pub outline_lighting_mix_factor: Option<f32>,
        pub uv_animation_mask_texture: Option<TextureInfo>,
        pub uv_animation_scroll_x_speed_factor: Option<f32>,
        pub uv_animation_scroll_y_speed_factor: Option<f32>,
        pub uv_animation_rotation_speed_factor: Option<f32>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct TextureInfo {
        pub index: usize,
        pub tex_coord: Option<u32>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ShadingShiftTextureInfo {
        pub index: usize,
        pub tex_coord: Option<u32>,
        pub scale: Option<f32>,
    }
}

pub mod materials_hdr_emissive_multiplier {
    use super::ExtensionMap;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct VrmcMaterialsHdrEmissiveMultiplier {
        pub emissive_multiplier: f32,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }
}

pub mod khr_materials_emissive_strength {
    use super::ExtensionMap;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct KhrMaterialsEmissiveStrength {
        pub emissive_strength: Option<f32>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }
}

pub mod vrma {
    use super::{AnyMap, ExtensionMap};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct VrmcVrmAnimation {
        #[serde(default = "default_spec_version")]
        pub spec_version: String,
        pub humanoid: Option<Humanoid>,
        pub expressions: Option<Expressions>,
        pub look_at: Option<LookAt>,
        pub extensions: Option<ExtensionMap>,
        pub extras: Option<serde_json::Value>,
    }

    fn default_spec_version() -> String {
        "1.0".to_owned()
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Humanoid {
        pub human_bones: AnyMap,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Expressions {
        pub preset: Option<AnyMap>,
        pub custom: Option<AnyMap>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct LookAt {
        pub node: usize,
    }
}

pub type ExtensionMap = IndexMap<String, Value>;
pub type AnyMap = IndexMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub enum VrmExtension {
    Vrm0(Box<vrm0::Vrm>),
    Vrm1(Box<vrm1::VrmcVrm>),
    Vrma(Box<vrma::VrmcVrmAnimation>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtensionBundle {
    pub vrm: Option<VrmExtension>,
    pub spring_bone: Option<spring_bone::VrmcSpringBone>,
    pub node_constraints: Vec<NodeConstraintExtension>,
    pub mtoon_materials: IndexMap<usize, materials_mtoon::VrmcMaterialsMtoon>,
    pub hdr_emissive_multipliers:
        IndexMap<usize, materials_hdr_emissive_multiplier::VrmcMaterialsHdrEmissiveMultiplier>,
    pub khr_emissive_strengths:
        IndexMap<usize, khr_materials_emissive_strength::KhrMaterialsEmissiveStrength>,
    pub unknown: ExtensionMap,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeConstraintExtension {
    pub node: usize,
    pub constraint: node_constraint::VrmcNodeConstraint,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("missing VRM extension")]
    MissingVrm,
    #[error("unsupported spec version for {extension}: {version}")]
    UnsupportedSpecVersion { extension: String, version: String },
    #[error("invalid extension {extension}: {message}")]
    InvalidExtension { extension: String, message: String },
}

pub fn parse_root_extensions(extensions: &ExtensionMap) -> Result<ExtensionBundle, ProtocolError> {
    let mut bundle = ExtensionBundle::default();

    for (name, value) in extensions {
        match name.as_str() {
            "VRM" => {
                let vrm = serde_json::from_value(value.clone()).map_err(|err| {
                    ProtocolError::InvalidExtension {
                        extension: name.clone(),
                        message: err.to_string(),
                    }
                })?;
                bundle.vrm = Some(VrmExtension::Vrm0(Box::new(vrm)));
            }
            "VRMC_vrm" => {
                let vrm: vrm1::VrmcVrm = serde_json::from_value(value.clone()).map_err(|err| {
                    ProtocolError::InvalidExtension {
                        extension: name.clone(),
                        message: err.to_string(),
                    }
                })?;
                if !matches!(vrm.spec_version.as_str(), "1.0" | "1.0-beta") {
                    return Err(ProtocolError::UnsupportedSpecVersion {
                        extension: name.clone(),
                        version: vrm.spec_version,
                    });
                }
                bundle.vrm = Some(VrmExtension::Vrm1(Box::new(vrm)));
            }
            "VRMC_springBone" => {
                let spring_bone = serde_json::from_value(value.clone()).map_err(|err| {
                    ProtocolError::InvalidExtension {
                        extension: name.clone(),
                        message: err.to_string(),
                    }
                })?;
                bundle.spring_bone = Some(spring_bone);
            }
            "VRMC_vrm_animation" => {
                let vrma: vrma::VrmcVrmAnimation =
                    serde_json::from_value(value.clone()).map_err(|err| {
                        ProtocolError::InvalidExtension {
                            extension: name.clone(),
                            message: err.to_string(),
                        }
                    })?;
                bundle.vrm = Some(VrmExtension::Vrma(Box::new(vrma)));
            }
            _ => {
                bundle.unknown.insert(name.clone(), value.clone());
            }
        }
    }

    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vrm1_round_trips_with_extras() {
        let input = serde_json::json!({
            "specVersion": "1.0",
            "meta": { "name": "avatar", "authors": ["pixiv"], "extras": { "x": 1 } },
            "humanoid": { "humanBones": { "hips": { "node": 0 } } },
            "extensions": { "vendor": true }
        });

        let vrm: vrm1::VrmcVrm = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(vrm.spec_version, "1.0");
        assert_eq!(vrm.meta.name, "avatar");
        assert_eq!(serde_json::to_value(vrm).unwrap()["extras"], Value::Null);
        assert_eq!(input["meta"]["extras"]["x"], 1);
    }

    #[test]
    fn root_extensions_detect_unsupported_vrm1() {
        let mut extensions = ExtensionMap::new();
        extensions.insert(
            "VRMC_vrm".to_owned(),
            serde_json::json!({
                "specVersion": "2.0",
                "meta": { "name": "avatar", "authors": [] },
                "humanoid": { "humanBones": {} }
            }),
        );

        let err = parse_root_extensions(&extensions).unwrap_err();
        assert!(matches!(err, ProtocolError::UnsupportedSpecVersion { .. }));
    }

    #[test]
    fn vrm0_round_trips_blend_shape_and_extras() {
        let input = serde_json::json!({
            "meta": { "title": "legacy" },
            "humanoid": { "humanBones": [{ "bone": "hips", "node": 0 }] },
            "blendShapeMaster": {
                "blendShapeGroups": [{
                    "name": "blink",
                    "presetName": "blink",
                    "binds": [{ "mesh": 1, "index": 2, "weight": 75.0 }],
                    "materialValues": [{
                        "materialName": "face",
                        "propertyName": "_Color",
                        "targetValue": [1.0, 0.0, 0.0, 1.0]
                    }]
                }]
            },
            "materialProperties": [{ "name": "face", "extras": { "kept": true } }]
        });

        let vrm: vrm0::Vrm = serde_json::from_value(input).unwrap();
        let value = serde_json::to_value(vrm).unwrap();

        assert_eq!(
            value["blendShapeMaster"]["blendShapeGroups"][0]["presetName"],
            "blink"
        );
        assert_eq!(value["materialProperties"][0]["extras"]["kept"], true);
    }

    #[test]
    fn spring_bone_round_trips_collider_shapes() {
        let input = serde_json::json!({
            "specVersion": "1.0",
            "colliders": [{
                "node": 0,
                "shape": {
                    "capsule": {
                        "offset": [0.0, 1.0, 0.0],
                        "radius": 0.25,
                        "tail": [0.0, 2.0, 0.0]
                    }
                }
            }],
            "colliderGroups": [{ "colliders": [0] }],
            "springs": [{ "joints": [{ "node": 0 }], "colliderGroups": [0] }]
        });

        let spring: spring_bone::VrmcSpringBone = serde_json::from_value(input).unwrap();
        let value = serde_json::to_value(spring).unwrap();

        assert_eq!(value["colliders"][0]["shape"]["capsule"]["radius"], 0.25);
        assert_eq!(value["springs"][0]["joints"][0]["node"], 0);
    }

    #[test]
    fn node_constraint_round_trips_aim_constraint() {
        let input = serde_json::json!({
            "specVersion": "1.0",
            "constraint": { "aim": { "source": 1, "aimAxis": "PositiveZ", "weight": 0.5 } }
        });

        let constraint: node_constraint::VrmcNodeConstraint =
            serde_json::from_value(input).unwrap();
        let value = serde_json::to_value(constraint).unwrap();

        assert_eq!(value["constraint"]["aim"]["source"], 1);
        assert_eq!(value["constraint"]["aim"]["aimAxis"], "PositiveZ");
    }

    #[test]
    fn materials_mtoon_round_trips_texture_infos() {
        let input = serde_json::json!({
            "specVersion": "1.0",
            "transparentWithZWrite": true,
            "shadeMultiplyTexture": { "index": 2, "texCoord": 1 },
            "outlineWidthMode": "worldCoordinates",
            "outlineWidthFactor": 0.01,
            "extras": { "note": "ok" }
        });

        let material: materials_mtoon::VrmcMaterialsMtoon = serde_json::from_value(input).unwrap();
        let value = serde_json::to_value(material).unwrap();

        assert_eq!(value["shadeMultiplyTexture"]["index"], 2);
        assert_eq!(value["extras"]["note"], "ok");
    }

    #[test]
    fn hdr_emissive_multiplier_round_trips_extras() {
        let input = serde_json::json!({
            "emissiveMultiplier": 4.0,
            "extras": { "archived": true }
        });

        let multiplier: materials_hdr_emissive_multiplier::VrmcMaterialsHdrEmissiveMultiplier =
            serde_json::from_value(input).unwrap();
        let value = serde_json::to_value(multiplier).unwrap();

        assert_eq!(value["emissiveMultiplier"], 4.0);
        assert_eq!(value["extras"]["archived"], true);
    }

    #[test]
    fn khr_emissive_strength_round_trips_defaultable_value() {
        let input = serde_json::json!({
            "emissiveStrength": 3.5,
            "extras": { "source": "khr" }
        });

        let strength: khr_materials_emissive_strength::KhrMaterialsEmissiveStrength =
            serde_json::from_value(input).unwrap();
        let value = serde_json::to_value(strength).unwrap();

        assert_eq!(value["emissiveStrength"], 3.5);
        assert_eq!(value["extras"]["source"], "khr");
    }

    #[test]
    fn vrm_animation_round_trips_node_maps() {
        let input = serde_json::json!({
            "specVersion": "1.0",
            "humanoid": { "humanBones": { "hips": { "node": 0 } } },
            "expressions": { "preset": { "blink": { "node": 1 } } },
            "lookAt": { "node": 2 },
            "extras": { "clip": "test" }
        });

        let animation: vrma::VrmcVrmAnimation = serde_json::from_value(input).unwrap();
        let value = serde_json::to_value(animation).unwrap();

        assert_eq!(value["humanoid"]["humanBones"]["hips"]["node"], 0);
        assert_eq!(value["extras"]["clip"], "test");
    }

    #[test]
    fn vrm_animation_defaults_missing_spec_version_to_one() {
        let input = serde_json::json!({
            "humanoid": { "humanBones": { "hips": { "node": 0 } } }
        });

        let animation: vrma::VrmcVrmAnimation = serde_json::from_value(input).unwrap();

        assert_eq!(animation.spec_version, "1.0");
    }

    #[test]
    fn root_parser_retains_unknown_extensions() {
        let mut extensions = ExtensionMap::new();
        extensions.insert("VENDOR_extension".to_owned(), serde_json::json!({ "x": 1 }));

        let bundle = parse_root_extensions(&extensions).unwrap();

        assert_eq!(bundle.unknown["VENDOR_extension"]["x"], 1);
    }

    #[test]
    fn root_parser_reports_invalid_extension_shape() {
        let mut extensions = ExtensionMap::new();
        extensions.insert(
            "VRMC_springBone".to_owned(),
            serde_json::json!({ "specVersion": 1 }),
        );

        let err = parse_root_extensions(&extensions).unwrap_err();

        assert!(matches!(
            err,
            ProtocolError::InvalidExtension { extension, .. } if extension == "VRMC_springBone"
        ));
    }
}
