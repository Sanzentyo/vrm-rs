//! Renderer-agnostic VRM domain model.

use glam::{Quat, Vec3};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raw;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Parsed;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Validated;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resolved;

#[derive(Clone, Debug, PartialEq)]
pub struct VrmAsset<State> {
    pub document: VrmDocument,
    state: PhantomData<State>,
}

impl<State> VrmAsset<State> {
    pub fn document(&self) -> &VrmDocument {
        &self.document
    }

    pub fn into_document(self) -> VrmDocument {
        self.document
    }
}

impl VrmAsset<Parsed> {
    pub fn new_parsed(document: VrmDocument) -> Self {
        Self {
            document,
            state: PhantomData,
        }
    }

    pub fn mark_validated(self) -> VrmAsset<Validated> {
        VrmAsset {
            document: self.document,
            state: PhantomData,
        }
    }
}

impl VrmAsset<Validated> {
    pub fn resolve(self) -> VrmModel<Resolved> {
        VrmModel {
            document: self.document,
            state: PhantomData,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VrmModel<State> {
    pub document: VrmDocument,
    state: PhantomData<State>,
}

impl<State> VrmModel<State> {
    pub fn document(&self) -> &VrmDocument {
        &self.document
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VrmDocument {
    pub kind: VrmKind,
    pub meta: Meta,
    pub humanoid: Humanoid,
    pub first_person: Feature<FirstPerson>,
    pub look_at: Feature<LookAt>,
    pub expressions: Feature<ExpressionSet>,
    pub spring_bone: Feature<SpringBoneSystem>,
    pub node_constraints: Vec<NodeConstraint>,
    pub materials: Vec<Material>,
    pub animation: Feature<VrmAnimation>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VrmKind {
    #[default]
    Vrm1,
    Vrm0Compat,
    Vrma,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Feature<T> {
    Present(T),
    #[default]
    Absent,
}

impl<T> Feature<T> {
    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Present(value) => Some(value),
            Self::Absent => None,
        }
    }

    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present(_))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeRef(pub usize);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MeshRef(pub usize);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MaterialRef(pub usize);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextureRef(pub usize);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub name: String,
    pub version: Option<String>,
    pub authors: Vec<String>,
    pub license_url: Option<String>,
    pub copyright_information: Option<String>,
    pub contact_information: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Humanoid {
    pub bones: IndexMap<HumanBoneName, HumanBone>,
}

impl Humanoid {
    pub fn required_bones_present(&self) -> bool {
        HumanBoneName::required()
            .iter()
            .all(|name| self.bones.contains_key(name))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HumanBoneName {
    Hips,
    Spine,
    Chest,
    UpperChest,
    Neck,
    Head,
    LeftEye,
    RightEye,
    Jaw,
    LeftUpperLeg,
    LeftLowerLeg,
    LeftFoot,
    LeftToes,
    RightUpperLeg,
    RightLowerLeg,
    RightFoot,
    RightToes,
    LeftShoulder,
    LeftUpperArm,
    LeftLowerArm,
    LeftHand,
    RightShoulder,
    RightUpperArm,
    RightLowerArm,
    RightHand,
    LeftThumbMetacarpal,
    LeftThumbProximal,
    LeftThumbDistal,
    LeftIndexProximal,
    LeftIndexIntermediate,
    LeftIndexDistal,
    LeftMiddleProximal,
    LeftMiddleIntermediate,
    LeftMiddleDistal,
    LeftRingProximal,
    LeftRingIntermediate,
    LeftRingDistal,
    LeftLittleProximal,
    LeftLittleIntermediate,
    LeftLittleDistal,
    RightThumbMetacarpal,
    RightThumbProximal,
    RightThumbDistal,
    RightIndexProximal,
    RightIndexIntermediate,
    RightIndexDistal,
    RightMiddleProximal,
    RightMiddleIntermediate,
    RightMiddleDistal,
    RightRingProximal,
    RightRingIntermediate,
    RightRingDistal,
    RightLittleProximal,
    RightLittleIntermediate,
    RightLittleDistal,
    Custom(String),
}

impl HumanBoneName {
    pub const fn required() -> &'static [HumanBoneName] {
        &[
            HumanBoneName::Hips,
            HumanBoneName::Spine,
            HumanBoneName::Head,
            HumanBoneName::LeftUpperLeg,
            HumanBoneName::LeftLowerLeg,
            HumanBoneName::LeftFoot,
            HumanBoneName::RightUpperLeg,
            HumanBoneName::RightLowerLeg,
            HumanBoneName::RightFoot,
            HumanBoneName::LeftUpperArm,
            HumanBoneName::LeftLowerArm,
            HumanBoneName::LeftHand,
            HumanBoneName::RightUpperArm,
            HumanBoneName::RightLowerArm,
            HumanBoneName::RightHand,
        ]
    }
}

impl From<&str> for HumanBoneName {
    fn from(value: &str) -> Self {
        match value {
            "hips" => Self::Hips,
            "spine" => Self::Spine,
            "chest" => Self::Chest,
            "upperChest" => Self::UpperChest,
            "neck" => Self::Neck,
            "head" => Self::Head,
            "leftEye" => Self::LeftEye,
            "rightEye" => Self::RightEye,
            "jaw" => Self::Jaw,
            "leftUpperLeg" => Self::LeftUpperLeg,
            "leftLowerLeg" => Self::LeftLowerLeg,
            "leftFoot" => Self::LeftFoot,
            "leftToes" => Self::LeftToes,
            "rightUpperLeg" => Self::RightUpperLeg,
            "rightLowerLeg" => Self::RightLowerLeg,
            "rightFoot" => Self::RightFoot,
            "rightToes" => Self::RightToes,
            "leftShoulder" => Self::LeftShoulder,
            "leftUpperArm" => Self::LeftUpperArm,
            "leftLowerArm" => Self::LeftLowerArm,
            "leftHand" => Self::LeftHand,
            "rightShoulder" => Self::RightShoulder,
            "rightUpperArm" => Self::RightUpperArm,
            "rightLowerArm" => Self::RightLowerArm,
            "rightHand" => Self::RightHand,
            other => Self::Custom(other.to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HumanBone {
    pub node: NodeRef,
    pub rest: Transform,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FirstPerson {
    pub mesh_annotations: Vec<FirstPersonMeshAnnotation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirstPersonMeshAnnotation {
    pub node: NodeRef,
    pub kind: FirstPersonAnnotation,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FirstPersonAnnotation {
    #[default]
    Auto,
    Both,
    ThirdPersonOnly,
    FirstPersonOnly,
    Unknown(String),
}

impl From<&str> for FirstPersonAnnotation {
    fn from(value: &str) -> Self {
        match value {
            "auto" => Self::Auto,
            "both" => Self::Both,
            "thirdPersonOnly" => Self::ThirdPersonOnly,
            "firstPersonOnly" => Self::FirstPersonOnly,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LookAt {
    pub offset_from_head: Vec3,
    pub kind: LookAtKind,
    pub horizontal_inner: RangeMap,
    pub horizontal_outer: RangeMap,
    pub vertical_down: RangeMap,
    pub vertical_up: RangeMap,
}

impl Default for LookAt {
    fn default() -> Self {
        Self {
            offset_from_head: Vec3::ZERO,
            kind: LookAtKind::Bone,
            horizontal_inner: RangeMap::default(),
            horizontal_outer: RangeMap::default(),
            vertical_down: RangeMap::default(),
            vertical_up: RangeMap::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LookAtKind {
    #[default]
    Bone,
    Expression,
    Unknown(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeMap {
    pub input_max_value: f32,
    pub output_scale: f32,
}

impl Default for RangeMap {
    fn default() -> Self {
        Self {
            input_max_value: 90.0,
            output_scale: 10.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExpressionSet {
    pub preset: IndexMap<ExpressionName, Expression>,
    pub custom: IndexMap<String, Expression>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ExpressionName {
    Happy,
    Angry,
    Sad,
    Relaxed,
    Surprised,
    Aa,
    Ih,
    Ou,
    Ee,
    Oh,
    Blink,
    BlinkLeft,
    BlinkRight,
    LookUp,
    LookDown,
    LookLeft,
    LookRight,
    Neutral,
    Unknown(String),
}

impl From<&str> for ExpressionName {
    fn from(value: &str) -> Self {
        match value {
            "happy" => Self::Happy,
            "angry" => Self::Angry,
            "sad" => Self::Sad,
            "relaxed" => Self::Relaxed,
            "surprised" => Self::Surprised,
            "aa" => Self::Aa,
            "ih" => Self::Ih,
            "ou" => Self::Ou,
            "ee" => Self::Ee,
            "oh" => Self::Oh,
            "blink" => Self::Blink,
            "blinkLeft" => Self::BlinkLeft,
            "blinkRight" => Self::BlinkRight,
            "lookUp" => Self::LookUp,
            "lookDown" => Self::LookDown,
            "lookLeft" => Self::LookLeft,
            "lookRight" => Self::LookRight,
            "neutral" => Self::Neutral,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

impl ExpressionName {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Happy => "happy",
            Self::Angry => "angry",
            Self::Sad => "sad",
            Self::Relaxed => "relaxed",
            Self::Surprised => "surprised",
            Self::Aa => "aa",
            Self::Ih => "ih",
            Self::Ou => "ou",
            Self::Ee => "ee",
            Self::Oh => "oh",
            Self::Blink => "blink",
            Self::BlinkLeft => "blinkLeft",
            Self::BlinkRight => "blinkRight",
            Self::LookUp => "lookUp",
            Self::LookDown => "lookDown",
            Self::LookLeft => "lookLeft",
            Self::LookRight => "lookRight",
            Self::Neutral => "neutral",
            Self::Unknown(name) => name.as_str(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Expression {
    pub binds: Vec<ExpressionBind>,
    pub is_binary: bool,
    pub override_blink: OverrideMode,
    pub override_look_at: OverrideMode,
    pub override_mouth: OverrideMode,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExpressionBind {
    MorphTarget {
        node: NodeRef,
        index: usize,
        weight: f32,
    },
    MaterialColor {
        material: MaterialRef,
        kind: String,
        target_value: Vec<f32>,
    },
    TextureTransform {
        material: MaterialRef,
        scale: Option<[f32; 2]>,
        offset: Option<[f32; 2]>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverrideMode {
    #[default]
    None,
    Block,
    Blend,
}

impl OverrideMode {
    pub fn amount(self, weight: f32) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Block => weight.clamp(0.0, 1.0),
            Self::Blend => weight.clamp(0.0, 1.0),
        }
    }
}

impl From<Option<String>> for OverrideMode {
    fn from(value: Option<String>) -> Self {
        match value.as_deref() {
            Some("block") => Self::Block,
            Some("blend") => Self::Blend,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpringBoneSystem {
    pub colliders: Vec<SpringCollider>,
    pub collider_groups: Vec<SpringColliderGroup>,
    pub springs: Vec<Spring>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpringCollider {
    pub node: NodeRef,
    pub shape: ColliderShape,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ColliderShape {
    Sphere {
        offset: Vec3,
        radius: f32,
    },
    Capsule {
        offset: Vec3,
        radius: f32,
        tail: Vec3,
    },
    Plane {
        offset: Vec3,
        normal: Vec3,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpringColliderGroup {
    pub name: Option<String>,
    pub colliders: Vec<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spring {
    pub name: Option<String>,
    pub joints: Vec<SpringJoint>,
    pub collider_groups: Vec<usize>,
    pub center: Option<NodeRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpringJoint {
    pub node: NodeRef,
    pub hit_radius: f32,
    pub stiffness: f32,
    pub gravity_power: f32,
    pub gravity_dir: Vec3,
    pub drag_force: f32,
}

impl Default for SpringJoint {
    fn default() -> Self {
        Self {
            node: NodeRef(0),
            hit_radius: 0.0,
            stiffness: 1.0,
            gravity_power: 0.0,
            gravity_dir: Vec3::new(0.0, -1.0, 0.0),
            drag_force: 0.4,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeConstraint {
    pub destination: NodeRef,
    pub source: NodeRef,
    pub kind: ConstraintKind,
    pub weight: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    Roll { axis: Axis },
    Aim { axis: Axis },
    Rotation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl Axis {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "PositiveX" | "+X" | "X" => Some(Self::PositiveX),
            "NegativeX" | "-X" => Some(Self::NegativeX),
            "PositiveY" | "+Y" | "Y" => Some(Self::PositiveY),
            "NegativeY" | "-Y" => Some(Self::NegativeY),
            "PositiveZ" | "+Z" | "Z" => Some(Self::PositiveZ),
            "NegativeZ" | "-Z" => Some(Self::NegativeZ),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Material {
    pub name: Option<String>,
    pub mtoon: Feature<MtoonMaterial>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MtoonMaterial {
    pub transparent_with_z_write: bool,
    pub render_queue_offset_number: i32,
    pub shade_color_factor: [f32; 3],
    pub shading_shift_factor: f32,
    pub shading_toony_factor: f32,
    pub gi_equalization_factor: f32,
    pub outline_width_mode: OutlineWidthMode,
    pub outline_width_factor: f32,
    pub outline_color_factor: [f32; 3],
    pub uv_animation: UvAnimation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutlineWidthMode {
    #[default]
    None,
    WorldCoordinates,
    ScreenCoordinates,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UvAnimation {
    pub scroll_x_speed: f32,
    pub scroll_y_speed: f32,
    pub rotation_speed: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VrmAnimation {
    pub duration: f32,
    pub rest_hips_position: Vec3,
    pub humanoid_rotation_tracks: IndexMap<HumanBoneName, RotationTrack>,
    pub hips_translation: Option<TranslationTrack>,
    pub preset_expression_tracks: IndexMap<ExpressionName, ScalarTrack>,
    pub custom_expression_tracks: IndexMap<String, ScalarTrack>,
    pub look_at_track: Option<RotationTrack>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RotationTrack {
    pub times: Vec<f32>,
    pub values: Vec<Quat>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TranslationTrack {
    pub times: Vec<f32>,
    pub values: Vec<Vec3>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScalarTrack {
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("required humanoid bones are missing")]
    MissingRequiredHumanBones,
    #[error("reference {kind}({index}) is out of range {len}")]
    ReferenceOutOfRange {
        kind: &'static str,
        index: usize,
        len: usize,
    },
    #[error("invalid axis: {0}")]
    InvalidAxis(String),
}

pub fn vec3(value: Option<[f32; 3]>) -> Vec3 {
    value.map_or(Vec3::ZERO, Vec3::from_array)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_state_transitions_are_explicit() {
        let asset = VrmAsset::<Parsed>::new_parsed(VrmDocument::default());
        let validated = asset.mark_validated();
        let model = validated.resolve();
        assert_eq!(model.document().kind, VrmKind::Vrm1);
    }

    #[test]
    fn expression_override_amount_is_saturated() {
        assert_eq!(OverrideMode::Block.amount(2.0), 1.0);
        assert_eq!(OverrideMode::None.amount(1.0), 0.0);
    }
}
