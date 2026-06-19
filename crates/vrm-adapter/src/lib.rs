//! Traits for connecting `vrm-rs` runtime output to external engines.
//!
//! ```
//! use vrm_adapter::VrmRuntimeDriver;
//! use vrm_core::VrmDocument;
//! use vrm_runtime::RuntimeEvents;
//!
//! let document = VrmDocument::default();
//! let events = RuntimeEvents::default();
//! let driver = VrmRuntimeDriver::new(&document).with_runtime_events(&events);
//!
//! assert!(driver.runtime_events.is_some());
//! ```

use glam::{Mat4, Quat, Vec3};
use indexmap::IndexMap;
use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
};
use thiserror::Error;
use vrm_core::{
    ColliderShape, ConstraintKind, EmissiveStrength, ExpressionBind, ExpressionName, Feature,
    FirstPersonAnnotation, HumanBoneName, MaterialRef, MtoonAlphaMode, MtoonCullMode,
    MtoonMaterial, MtoonPipelinePass, MtoonRenderQueue, MtoonTextureSet, NodeConstraint, NodeRef,
    OutlineWidthMode, RawAbsolutePose, RawPose, Spring, SpringBoneSystem, TextureRef, Transform,
    UvAnimation, VrmDocument,
};
use vrm_runtime::{
    AimConstraintInput, AnimationMixerFrame, AppliedExpression, CenterSpringParticleState,
    CenterSpringRuntimeState, ConstraintRestState, DeltaTime, Runtime, RuntimeEvents,
    SpringJointParityInput, SpringJointRestState, SpringJointSimulationInput, SpringParticleState,
    SpringRuntimeState, VrmAnimationFrame, VrmAnimationMixer, collider_shape_in_simulation_space,
    solve_aim_constraint, solve_roll_constraint, solve_rotation_constraint,
    solve_spring_joint_rotation, step_spring_joint, step_spring_joint_parity,
};

pub trait CoordinateSpaceMapping: Copy + std::fmt::Debug + 'static {
    const LABEL: &'static str;
    const MIRRORS_HANDEDNESS: bool;

    fn from_vrm_position(position: Vec3) -> Vec3;
    fn to_vrm_position(position: Vec3) -> Vec3;
    fn from_vrm_rotation(rotation: Quat) -> Quat;
    fn to_vrm_rotation(rotation: Quat) -> Quat;

    #[inline(always)]
    fn from_vrm_direction(direction: Vec3) -> Vec3 {
        Self::from_vrm_position(direction)
    }

    #[inline(always)]
    fn to_vrm_direction(direction: Vec3) -> Vec3 {
        Self::to_vrm_position(direction)
    }

    #[inline(always)]
    fn from_vrm_transform(transform: Transform) -> Transform {
        Transform {
            translation: Self::from_vrm_position(transform.translation),
            rotation: Self::from_vrm_rotation(transform.rotation),
            scale: transform.scale,
        }
    }

    #[inline(always)]
    fn to_vrm_transform(transform: Transform) -> Transform {
        Transform {
            translation: Self::to_vrm_position(transform.translation),
            rotation: Self::to_vrm_rotation(transform.rotation),
            scale: transform.scale,
        }
    }

    /// Converts an affine transform matrix from VRM/glTF space into this
    /// coordinate space.
    ///
    /// This intentionally treats `matrix` as affine data: translation and
    /// basis vectors are remapped, while projective/perspective components are
    /// not preserved. Use projection helpers for clip-space matrices.
    #[inline(always)]
    fn from_vrm_affine_matrix(matrix: Mat4) -> Mat4 {
        coordinate_space_matrix_from_vrm::<Self>(matrix)
    }

    /// Deprecated compatibility alias for [`Self::from_vrm_affine_matrix`].
    ///
    /// The old name accepted `Mat4`, but this conversion has always been an
    /// affine basis/translation remap rather than a projective matrix remap.
    #[deprecated(
        since = "0.1.0",
        note = "use from_vrm_affine_matrix; projective matrices are not preserved"
    )]
    #[inline(always)]
    fn from_vrm_matrix(matrix: Mat4) -> Mat4 {
        Self::from_vrm_affine_matrix(matrix)
    }

    /// Converts an affine transform matrix from this coordinate space back to
    /// VRM/glTF space.
    ///
    /// This intentionally treats `matrix` as affine data: translation and
    /// basis vectors are remapped, while projective/perspective components are
    /// not preserved. Use projection helpers for clip-space matrices.
    #[inline(always)]
    fn to_vrm_affine_matrix(matrix: Mat4) -> Mat4 {
        coordinate_space_matrix_to_vrm::<Self>(matrix)
    }

    /// Deprecated compatibility alias for [`Self::to_vrm_affine_matrix`].
    ///
    /// The old name accepted `Mat4`, but this conversion has always been an
    /// affine basis/translation remap rather than a projective matrix remap.
    #[deprecated(
        since = "0.1.0",
        note = "use to_vrm_affine_matrix; projective matrices are not preserved"
    )]
    #[inline(always)]
    fn to_vrm_matrix(matrix: Mat4) -> Mat4 {
        Self::to_vrm_affine_matrix(matrix)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VrmCoordinateSpace;

impl CoordinateSpaceMapping for VrmCoordinateSpace {
    const LABEL: &'static str = "vrm-gltf-right-handed-y-up";
    const MIRRORS_HANDEDNESS: bool = false;

    #[inline(always)]
    fn from_vrm_position(position: Vec3) -> Vec3 {
        position
    }

    #[inline(always)]
    fn to_vrm_position(position: Vec3) -> Vec3 {
        position
    }

    #[inline(always)]
    fn from_vrm_rotation(rotation: Quat) -> Quat {
        rotation
    }

    #[inline(always)]
    fn to_vrm_rotation(rotation: Quat) -> Quat {
        rotation
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlipZCoordinateSpace;

impl CoordinateSpaceMapping for FlipZCoordinateSpace {
    const LABEL: &'static str = "flip-z-left-handed-y-up";
    const MIRRORS_HANDEDNESS: bool = true;

    #[inline(always)]
    fn from_vrm_position(position: Vec3) -> Vec3 {
        Vec3::new(position.x, position.y, -position.z)
    }

    #[inline(always)]
    fn to_vrm_position(position: Vec3) -> Vec3 {
        Vec3::new(position.x, position.y, -position.z)
    }

    #[inline(always)]
    fn from_vrm_rotation(rotation: Quat) -> Quat {
        flip_z_rotation(rotation)
    }

    #[inline(always)]
    fn to_vrm_rotation(rotation: Quat) -> Quat {
        flip_z_rotation(rotation)
    }
}

pub type GltfCoordinateSpace = VrmCoordinateSpace;
pub type LeftHandedZForwardCoordinateSpace = FlipZCoordinateSpace;

#[inline(always)]
fn flip_z_rotation(rotation: Quat) -> Quat {
    Quat::from_xyzw(-rotation.x, -rotation.y, rotation.z, rotation.w).normalize()
}

#[inline(always)]
fn coordinate_space_matrix_from_vrm<C>(matrix: Mat4) -> Mat4
where
    C: CoordinateSpaceMapping,
{
    map_coordinate_space_affine_matrix::<VrmCoordinateSpace, C>(matrix)
}

#[inline(always)]
fn coordinate_space_matrix_to_vrm<C>(matrix: Mat4) -> Mat4
where
    C: CoordinateSpaceMapping,
{
    map_coordinate_space_affine_matrix::<C, VrmCoordinateSpace>(matrix)
}

#[inline(always)]
fn map_coordinate_space_affine_matrix<I, O>(matrix: Mat4) -> Mat4
where
    I: CoordinateSpaceMapping,
    O: CoordinateSpaceMapping,
{
    let origin = O::from_vrm_position(I::to_vrm_position(
        matrix.transform_point3(I::from_vrm_position(O::to_vrm_position(Vec3::ZERO))),
    ));
    Mat4::from_cols(
        O::from_vrm_direction(I::to_vrm_direction(
            matrix.transform_vector3(I::from_vrm_direction(O::to_vrm_direction(Vec3::X))),
        ))
        .extend(0.0),
        O::from_vrm_direction(I::to_vrm_direction(
            matrix.transform_vector3(I::from_vrm_direction(O::to_vrm_direction(Vec3::Y))),
        ))
        .extend(0.0),
        O::from_vrm_direction(I::to_vrm_direction(
            matrix.transform_vector3(I::from_vrm_direction(O::to_vrm_direction(Vec3::Z))),
        ))
        .extend(0.0),
        origin.extend(1.0),
    )
}

pub trait ClipDepthMapping: Copy + std::fmt::Debug + 'static {
    const DEPTH_RANGE_LABEL: &'static str;

    fn webgl_depth_from_ndc_z(ndc_z: f32) -> f32;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ZeroToOneDepth;

impl ClipDepthMapping for ZeroToOneDepth {
    const DEPTH_RANGE_LABEL: &'static str = "zero-to-one-ndc";

    #[inline(always)]
    fn webgl_depth_from_ndc_z(ndc_z: f32) -> f32 {
        ndc_z * 2.0 - 1.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReverseZeroToOneDepth;

impl ClipDepthMapping for ReverseZeroToOneDepth {
    const DEPTH_RANGE_LABEL: &'static str = "reverse-zero-to-one-ndc";

    #[inline(always)]
    fn webgl_depth_from_ndc_z(ndc_z: f32) -> f32 {
        (1.0 - ndc_z) * 2.0 - 1.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NegativeOneToOneDepth;

impl ClipDepthMapping for NegativeOneToOneDepth {
    const DEPTH_RANGE_LABEL: &'static str = "negative-one-to-one-ndc";

    #[inline(always)]
    fn webgl_depth_from_ndc_z(ndc_z: f32) -> f32 {
        ndc_z
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RendererFrontFace {
    #[default]
    Ccw,
    Cw,
}

impl RendererFrontFace {
    #[inline(always)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ccw => "ccw",
            Self::Cw => "cw",
        }
    }

    #[inline(always)]
    pub fn is_gpu_front_facing(self, y_down_screen_signed_area: f32) -> bool {
        match self {
            Self::Ccw => y_down_screen_signed_area < 0.0,
            Self::Cw => y_down_screen_signed_area > 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenProjectionSize {
    pub width: f32,
    pub height: f32,
}

impl ScreenProjectionSize {
    #[inline(always)]
    pub fn from_pixels(width: u32, height: u32) -> Self {
        Self {
            width: width as f32,
            height: height as f32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenProjectionBounds {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl ScreenProjectionBounds {
    #[inline(always)]
    pub fn from_triangle(screen: [[f32; 2]; 3]) -> Self {
        let [[ax, ay], [bx, by], [cx, cy]] = screen;
        Self {
            min_x: ax.min(bx).min(cx),
            min_y: ay.min(by).min(cy),
            max_x: ax.max(bx).max(cx),
            max_y: ay.max(by).max(cy),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenTriangleProjection {
    pub screen: [[f32; 2]; 3],
    pub bounds: ScreenProjectionBounds,
    pub ndc_depth: f32,
    pub webgl_depth: f32,
    pub screen_signed_area: f32,
    pub front_facing: bool,
    pub gpu_front_facing: bool,
}

#[inline(always)]
pub fn project_triangle_to_screen<D>(
    positions: [[f32; 3]; 3],
    view_projection: Mat4,
    size: ScreenProjectionSize,
    front_face: RendererFrontFace,
) -> Option<ScreenTriangleProjection>
where
    D: ClipDepthMapping,
{
    let points =
        positions.map(|position| project_position_to_screen::<D>(position, view_projection, size));
    project_screen_triangle_from_points::<D>(points, front_face)
}

#[inline(always)]
pub fn project_vrm_triangle_to_screen<C, D>(
    positions: [[f32; 3]; 3],
    view_projection: Mat4,
    size: ScreenProjectionSize,
    front_face: RendererFrontFace,
) -> Option<ScreenTriangleProjection>
where
    C: CoordinateSpaceMapping,
    D: ClipDepthMapping,
{
    let points = positions
        .map(|position| project_vrm_position_to_screen::<C, D>(position, view_projection, size));
    project_screen_triangle_from_points::<D>(points, front_face)
}

#[inline(always)]
fn project_screen_triangle_from_points<D>(
    points: [Option<[f32; 3]>; 3],
    front_face: RendererFrontFace,
) -> Option<ScreenTriangleProjection>
where
    D: ClipDepthMapping,
{
    let [Some(a), Some(b), Some(c)] = points else {
        return None;
    };
    let screen = [[a[0], a[1]], [b[0], b[1]], [c[0], c[1]]];
    let screen_signed_area = screen_triangle_signed_area(screen);
    let ndc_depth = (a[2] + b[2] + c[2]) / 3.0;
    Some(ScreenTriangleProjection {
        screen,
        bounds: ScreenProjectionBounds::from_triangle(screen),
        ndc_depth,
        webgl_depth: D::webgl_depth_from_ndc_z(ndc_depth),
        screen_signed_area,
        front_facing: screen_signed_area > 0.0,
        gpu_front_facing: front_face.is_gpu_front_facing(screen_signed_area),
    })
}

#[inline(always)]
pub fn project_position_to_screen<D>(
    position: [f32; 3],
    view_projection: Mat4,
    size: ScreenProjectionSize,
) -> Option<[f32; 3]>
where
    D: ClipDepthMapping,
{
    project_renderer_position_to_screen::<D>(position, view_projection, size)
}

#[inline(always)]
pub fn project_vrm_position_to_screen<C, D>(
    position: [f32; 3],
    view_projection: Mat4,
    size: ScreenProjectionSize,
) -> Option<[f32; 3]>
where
    C: CoordinateSpaceMapping,
    D: ClipDepthMapping,
{
    let renderer_position = C::from_vrm_position(Vec3::from_array(position));
    project_renderer_position_to_screen::<D>(renderer_position.to_array(), view_projection, size)
}

#[inline(always)]
pub fn project_renderer_position_to_screen<D>(
    position: [f32; 3],
    view_projection: Mat4,
    size: ScreenProjectionSize,
) -> Option<[f32; 3]>
where
    D: ClipDepthMapping,
{
    let clip = view_projection * Vec3::from_array(position).extend(1.0);
    if clip.w.abs() <= f32::EPSILON {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    let screen = [
        (ndc.x * 0.5 + 0.5) * size.width,
        (0.5 - ndc.y * 0.5) * size.height,
        ndc.z,
    ];
    (screen[0].is_finite() && screen[1].is_finite() && screen[2].is_finite()).then_some(screen)
}

#[inline(always)]
pub fn screen_triangle_signed_area(screen: [[f32; 2]; 3]) -> f32 {
    (screen[1][0] - screen[0][0]) * (screen[2][1] - screen[0][1])
        - (screen[1][1] - screen[0][1]) * (screen[2][0] - screen[0][0])
}

pub trait SceneGraph {
    type Error;

    fn parent(&self, node: NodeRef) -> Result<Option<NodeRef>, Self::Error>;
    fn children(&self, node: NodeRef) -> Result<Vec<NodeRef>, Self::Error>;

    /// Visits direct children of `node`.
    ///
    /// The default implementation preserves compatibility by delegating to
    /// [`Self::children`], which may allocate. Override this method in concrete
    /// scene backends when child traversal sits on a hot path.
    fn visit_children<F>(&self, node: NodeRef, mut visitor: F) -> Result<(), Self::Error>
    where
        Self: Sized,
        F: FnMut(NodeRef),
    {
        for child in self.children(node)? {
            visitor(child);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct CoordinateSpaceTarget<'a, T, C>
where
    C: CoordinateSpaceMapping,
{
    target: &'a mut T,
    space: PhantomData<C>,
}

impl<'a, T, C> CoordinateSpaceTarget<'a, T, C>
where
    C: CoordinateSpaceMapping,
{
    #[inline(always)]
    pub fn new(target: &'a mut T) -> Self {
        Self {
            target,
            space: PhantomData,
        }
    }

    #[inline(always)]
    pub fn target(&self) -> &T {
        self.target
    }

    #[inline(always)]
    pub fn target_mut(&mut self) -> &mut T {
        self.target
    }

    #[inline(always)]
    pub fn into_inner(self) -> &'a mut T {
        self.target
    }
}

#[inline(always)]
pub fn coordinate_space_target<C, T>(target: &mut T) -> CoordinateSpaceTarget<'_, T, C>
where
    C: CoordinateSpaceMapping,
{
    CoordinateSpaceTarget::new(target)
}

impl<T, C> SceneGraph for CoordinateSpaceTarget<'_, T, C>
where
    T: SceneGraph,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn parent(&self, node: NodeRef) -> Result<Option<NodeRef>, Self::Error> {
        self.target.parent(node)
    }

    #[inline(always)]
    fn children(&self, node: NodeRef) -> Result<Vec<NodeRef>, Self::Error> {
        self.target.children(node)
    }

    #[inline(always)]
    fn visit_children<F>(&self, node: NodeRef, visitor: F) -> Result<(), Self::Error>
    where
        F: FnMut(NodeRef),
    {
        self.target.visit_children(node, visitor)
    }
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

impl<T, C> TransformAccess for CoordinateSpaceTarget<'_, T, C>
where
    T: TransformAccess,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn local_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
        self.target.local_transform(node).map(C::to_vrm_transform)
    }

    #[inline(always)]
    fn set_local_transform(
        &mut self,
        node: NodeRef,
        transform: Transform,
    ) -> Result<(), Self::Error> {
        self.target
            .set_local_transform(node, C::from_vrm_transform(transform))
    }

    #[inline(always)]
    fn set_local_rotation(&mut self, node: NodeRef, rotation: Quat) -> Result<(), Self::Error> {
        self.target
            .set_local_rotation(node, C::from_vrm_rotation(rotation))
    }

    #[inline(always)]
    fn translate_local(&mut self, node: NodeRef, translation: Vec3) -> Result<(), Self::Error> {
        self.target
            .translate_local(node, C::from_vrm_position(translation))
    }
}

pub trait WorldTransformAccess {
    type Error;

    fn world_transform(&self, node: NodeRef) -> Result<Transform, Self::Error>;
}

impl<T, C> WorldTransformAccess for CoordinateSpaceTarget<'_, T, C>
where
    T: WorldTransformAccess,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn world_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
        self.target.world_transform(node).map(C::to_vrm_transform)
    }
}

pub trait WorldMatrixAccess {
    type Error;

    fn world_matrix(&self, node: NodeRef) -> Result<Mat4, Self::Error>;
}

impl<T, C> WorldMatrixAccess for CoordinateSpaceTarget<'_, T, C>
where
    T: WorldMatrixAccess,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn world_matrix(&self, node: NodeRef) -> Result<Mat4, Self::Error> {
        self.target.world_matrix(node).map(C::to_vrm_affine_matrix)
    }
}

pub trait WorldTransformUpdate {
    type Error;

    fn update_world_transforms(&mut self) -> Result<(), Self::Error>;
}

impl<T, C> WorldTransformUpdate for CoordinateSpaceTarget<'_, T, C>
where
    T: WorldTransformUpdate,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn update_world_transforms(&mut self) -> Result<(), Self::Error> {
        self.target.update_world_transforms()
    }
}

pub trait ConstraintRestAccess {
    type Error;

    fn constraint_rest_state(
        &self,
        destination: NodeRef,
        source: NodeRef,
    ) -> Result<ConstraintRestState, Self::Error>;
}

impl<T, C> ConstraintRestAccess for CoordinateSpaceTarget<'_, T, C>
where
    T: ConstraintRestAccess,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn constraint_rest_state(
        &self,
        destination: NodeRef,
        source: NodeRef,
    ) -> Result<ConstraintRestState, Self::Error> {
        self.target
            .constraint_rest_state(destination, source)
            .map(|state| ConstraintRestState {
                destination_rest_rotation: C::to_vrm_rotation(state.destination_rest_rotation),
                source_rest_rotation: C::to_vrm_rotation(state.source_rest_rotation),
            })
    }
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

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum HeadlessAdapterError {
    #[error("missing node {0:?}")]
    MissingNode(NodeRef),
    #[error("node {0:?} cannot be parented to itself")]
    SelfParent(NodeRef),
    #[error("cyclic hierarchy at node {0:?}")]
    CyclicHierarchy(NodeRef),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureTransformWrite {
    pub scale: Option<[f32; 2]>,
    pub offset: Option<[f32; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeadlessNodeState {
    pub parent: Option<NodeRef>,
    pub children: Vec<NodeRef>,
    pub local: Transform,
    pub world: Transform,
    pub visible: bool,
}

impl HeadlessNodeState {
    pub fn new(local: Transform) -> Self {
        Self {
            parent: None,
            children: Vec::new(),
            local,
            world: local,
            visible: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeadlessSceneState {
    nodes: HashMap<NodeRef, HeadlessNodeState>,
    morph_weights: HashMap<(NodeRef, usize), f32>,
    material_colors: HashMap<(MaterialRef, String), Vec<f32>>,
    texture_transforms: HashMap<MaterialRef, TextureTransformWrite>,
    emissive_intensities: HashMap<MaterialRef, f32>,
    mtoon_pipeline_passes: HashMap<MaterialRef, Vec<MtoonPipelinePass>>,
    look_at_rotation: Option<Quat>,
    constraint_rest: HashMap<(NodeRef, NodeRef), ConstraintRestState>,
}

impl HeadlessSceneState {
    pub fn insert_node(&mut self, node: NodeRef, local: Transform) {
        self.nodes.insert(node, HeadlessNodeState::new(local));
    }

    pub fn node(&self, node: NodeRef) -> Option<&HeadlessNodeState> {
        self.nodes.get(&node)
    }

    pub fn set_parent(
        &mut self,
        node: NodeRef,
        parent: Option<NodeRef>,
    ) -> Result<(), HeadlessAdapterError> {
        self.ensure_node(node)?;
        if parent == Some(node) {
            return Err(HeadlessAdapterError::SelfParent(node));
        }
        if let Some(parent) = parent {
            self.ensure_node(parent)?;
            self.ensure_no_descendant_parent(node, parent)?;
        }
        if let Some(previous_parent) = self.nodes.get(&node).and_then(|state| state.parent)
            && let Some(previous) = self.nodes.get_mut(&previous_parent)
        {
            previous.children.retain(|child| *child != node);
        }
        if let Some(state) = self.nodes.get_mut(&node) {
            state.parent = parent;
        }
        if let Some(parent) = parent {
            let children = &mut self
                .nodes
                .get_mut(&parent)
                .ok_or(HeadlessAdapterError::MissingNode(parent))?
                .children;
            if !children.contains(&node) {
                children.push(node);
            }
        }
        Ok(())
    }

    pub fn morph_weight(&self, node: NodeRef, morph_index: usize) -> Option<f32> {
        self.morph_weights.get(&(node, morph_index)).copied()
    }

    pub fn material_color(&self, material: MaterialRef, property: &str) -> Option<&[f32]> {
        self.material_colors
            .get(&(material, property.to_owned()))
            .map(Vec::as_slice)
    }

    pub fn texture_transform(&self, material: MaterialRef) -> Option<TextureTransformWrite> {
        self.texture_transforms.get(&material).copied()
    }

    pub fn emissive_intensity(&self, material: MaterialRef) -> Option<f32> {
        self.emissive_intensities.get(&material).copied()
    }

    pub fn mtoon_pipeline_passes(&self, material: MaterialRef) -> Option<&[MtoonPipelinePass]> {
        self.mtoon_pipeline_passes.get(&material).map(Vec::as_slice)
    }

    pub fn look_at_rotation(&self) -> Option<Quat> {
        self.look_at_rotation
    }

    pub fn set_constraint_rest_state(
        &mut self,
        destination: NodeRef,
        source: NodeRef,
        state: ConstraintRestState,
    ) -> Result<(), HeadlessAdapterError> {
        self.ensure_node(destination)?;
        self.ensure_node(source)?;
        self.constraint_rest.insert((destination, source), state);
        Ok(())
    }

    pub fn capture_constraint_rest_state(
        &mut self,
        destination: NodeRef,
        source: NodeRef,
    ) -> Result<(), HeadlessAdapterError> {
        let state = ConstraintRestState::new(
            self.local_transform(destination)?.rotation,
            self.local_transform(source)?.rotation,
        );
        self.set_constraint_rest_state(destination, source, state)
    }

    fn ensure_node(&self, node: NodeRef) -> Result<(), HeadlessAdapterError> {
        self.nodes
            .contains_key(&node)
            .then_some(())
            .ok_or(HeadlessAdapterError::MissingNode(node))
    }

    fn ensure_no_descendant_parent(
        &self,
        node: NodeRef,
        proposed_parent: NodeRef,
    ) -> Result<(), HeadlessAdapterError> {
        let mut current = Some(proposed_parent);
        let mut visited = HashSet::new();
        while let Some(candidate) = current {
            if !visited.insert(candidate) {
                return Err(HeadlessAdapterError::CyclicHierarchy(candidate));
            }
            if candidate == node {
                return Err(HeadlessAdapterError::CyclicHierarchy(node));
            }
            current = self
                .nodes
                .get(&candidate)
                .ok_or(HeadlessAdapterError::MissingNode(candidate))?
                .parent;
        }
        Ok(())
    }

    fn update_world_node(
        &mut self,
        node: NodeRef,
        parent_world: Option<Transform>,
        visiting: &mut HashSet<NodeRef>,
    ) -> Result<(), HeadlessAdapterError> {
        if !visiting.insert(node) {
            return Err(HeadlessAdapterError::CyclicHierarchy(node));
        }
        let local = self
            .nodes
            .get(&node)
            .ok_or(HeadlessAdapterError::MissingNode(node))?
            .local;
        let world = parent_world.map_or(local, |parent| compose_transform(parent, local));
        self.nodes
            .get_mut(&node)
            .ok_or(HeadlessAdapterError::MissingNode(node))?
            .world = world;
        let children = self
            .nodes
            .get(&node)
            .ok_or(HeadlessAdapterError::MissingNode(node))?
            .children
            .clone();
        for child in children {
            self.update_world_node(child, Some(world), visiting)?;
        }
        visiting.remove(&node);
        Ok(())
    }
}

impl SceneGraph for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn parent(&self, node: NodeRef) -> Result<Option<NodeRef>, Self::Error> {
        self.ensure_node(node)?;
        Ok(self.nodes.get(&node).and_then(|state| state.parent))
    }

    fn children(&self, node: NodeRef) -> Result<Vec<NodeRef>, Self::Error> {
        self.ensure_node(node)?;
        Ok(self
            .nodes
            .get(&node)
            .map(|state| state.children.clone())
            .unwrap_or_default())
    }

    fn visit_children<F>(&self, node: NodeRef, mut visitor: F) -> Result<(), Self::Error>
    where
        F: FnMut(NodeRef),
    {
        self.ensure_node(node)?;
        if let Some(state) = self.nodes.get(&node) {
            for &child in &state.children {
                visitor(child);
            }
        }
        Ok(())
    }
}

impl TransformAccess for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn local_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
        self.nodes
            .get(&node)
            .map(|state| state.local)
            .ok_or(HeadlessAdapterError::MissingNode(node))
    }

    fn set_local_transform(
        &mut self,
        node: NodeRef,
        transform: Transform,
    ) -> Result<(), Self::Error> {
        self.nodes
            .get_mut(&node)
            .map(|state| state.local = transform)
            .ok_or(HeadlessAdapterError::MissingNode(node))
    }

    fn set_local_rotation(&mut self, node: NodeRef, rotation: Quat) -> Result<(), Self::Error> {
        let mut transform = self.local_transform(node)?;
        transform.rotation = rotation;
        self.set_local_transform(node, transform)
    }

    fn translate_local(&mut self, node: NodeRef, translation: Vec3) -> Result<(), Self::Error> {
        let mut transform = self.local_transform(node)?;
        transform.translation = translation;
        self.set_local_transform(node, transform)
    }
}

impl WorldTransformAccess for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn world_transform(&self, node: NodeRef) -> Result<Transform, Self::Error> {
        self.nodes
            .get(&node)
            .map(|state| state.world)
            .ok_or(HeadlessAdapterError::MissingNode(node))
    }
}

impl WorldMatrixAccess for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn world_matrix(&self, node: NodeRef) -> Result<Mat4, Self::Error> {
        self.world_transform(node).map(transform_matrix)
    }
}

impl WorldTransformUpdate for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn update_world_transforms(&mut self) -> Result<(), Self::Error> {
        let roots = self
            .nodes
            .iter()
            .filter_map(|(node, state)| state.parent.is_none().then_some(*node))
            .collect::<Vec<_>>();
        let mut visiting = HashSet::new();
        for root in roots {
            self.update_world_node(root, None, &mut visiting)?;
        }
        Ok(())
    }
}

impl ConstraintRestAccess for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn constraint_rest_state(
        &self,
        destination: NodeRef,
        source: NodeRef,
    ) -> Result<ConstraintRestState, Self::Error> {
        if let Some(state) = self.constraint_rest.get(&(destination, source)).copied() {
            return Ok(state);
        }
        Ok(ConstraintRestState::new(
            self.local_transform(destination)?.rotation,
            self.local_transform(source)?.rotation,
        ))
    }
}

impl MorphTargetAccess for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn set_morph_weight(
        &mut self,
        node: NodeRef,
        morph_index: usize,
        weight: f32,
    ) -> Result<(), Self::Error> {
        self.ensure_node(node)?;
        self.morph_weights.insert((node, morph_index), weight);
        Ok(())
    }
}

impl MaterialAccess for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn set_material_color(
        &mut self,
        material: MaterialRef,
        property: &str,
        value: &[f32],
    ) -> Result<(), Self::Error> {
        self.material_colors
            .insert((material, property.to_owned()), value.to_vec());
        Ok(())
    }

    fn set_texture_transform(
        &mut self,
        material: MaterialRef,
        scale: Option<[f32; 2]>,
        offset: Option<[f32; 2]>,
    ) -> Result<(), Self::Error> {
        self.texture_transforms
            .insert(material, TextureTransformWrite { scale, offset });
        Ok(())
    }

    fn set_emissive_intensity(
        &mut self,
        material: MaterialRef,
        intensity: f32,
    ) -> Result<(), Self::Error> {
        self.emissive_intensities.insert(material, intensity);
        Ok(())
    }
}

impl MtoonPipelineAccess for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn set_mtoon_pipeline_passes(
        &mut self,
        material: MaterialRef,
        passes: &[MtoonPipelinePass],
    ) -> Result<(), Self::Error> {
        self.mtoon_pipeline_passes.insert(material, passes.to_vec());
        Ok(())
    }
}

impl VisibilityAccess for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn set_node_visible(&mut self, node: NodeRef, visible: bool) -> Result<(), Self::Error> {
        self.nodes
            .get_mut(&node)
            .map(|state| state.visible = visible)
            .ok_or(HeadlessAdapterError::MissingNode(node))
    }
}

impl LookAtAccess for HeadlessSceneState {
    type Error = HeadlessAdapterError;

    fn set_look_at_rotation(&mut self, rotation: Quat) -> Result<(), Self::Error> {
        self.look_at_rotation = Some(rotation);
        Ok(())
    }
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

fn compose_transform(parent: Transform, local: Transform) -> Transform {
    Transform {
        translation: parent.translation + parent.rotation * (parent.scale * local.translation),
        rotation: parent.rotation * local.rotation,
        scale: parent.scale * local.scale,
    }
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

fn spring_joint_child<T, E>(
    target: &T,
    node: NodeRef,
    next_joint: Option<NodeRef>,
) -> Result<Option<NodeRef>, AdapterError<E>>
where
    T: SceneGraph<Error = E>,
{
    let mut first_child = None;
    let mut has_next_joint_child = false;
    target
        .visit_children(node, |child| {
            first_child.get_or_insert(child);
            if Some(child) == next_joint {
                has_next_joint_child = true;
            }
        })
        .map_err(AdapterError::Target)?;
    if let Some(next_joint) = next_joint
        && (has_next_joint_child
            || target.parent(next_joint).map_err(AdapterError::Target)? == Some(node))
    {
        return Ok(Some(next_joint));
    }
    Ok(first_child)
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
                let child = spring_joint_child(
                    target,
                    joint.node,
                    spring.joints.get(joint_index + 1).map(|joint| joint.node),
                )?;
                let initial_local_child_position =
                    initial_local_child_position(target, joint_local, child)?;
                let parent_world = target
                    .parent(joint.node)
                    .map_err(AdapterError::Target)?
                    .map(|parent| target.world_transform(parent).map_err(AdapterError::Target))
                    .transpose()?
                    .unwrap_or_default();
                let mut rest = SpringJointRestState::from_local_child(
                    joint_local,
                    initial_local_child_position,
                );
                let center_world = spring
                    .center
                    .map(|center| target.world_transform(center).map_err(AdapterError::Target))
                    .transpose()?;
                let center_tail = center_space_tail(joint_world, center_world, rest);
                let tail_world = center_world
                    .map(transform_matrix)
                    .unwrap_or(Mat4::IDENTITY)
                    .transform_point3(center_tail);
                let world_bone_axis = (tail_world - joint_world.translation).normalize_or(
                    (joint_world.rotation * initial_local_child_position).normalize_or(Vec3::Y),
                );
                let world_bone_length = tail_world.distance(joint_world.translation);
                rest = rest.with_initial_world_bone(
                    parent_world.rotation,
                    world_bone_axis,
                    world_bone_length,
                );
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

impl<T, C> MorphTargetAccess for CoordinateSpaceTarget<'_, T, C>
where
    T: MorphTargetAccess,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn set_morph_weight(
        &mut self,
        node: NodeRef,
        morph_index: usize,
        weight: f32,
    ) -> Result<(), Self::Error> {
        self.target.set_morph_weight(node, morph_index, weight)
    }
}

impl<T, C> MaterialAccess for CoordinateSpaceTarget<'_, T, C>
where
    T: MaterialAccess,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn set_material_color(
        &mut self,
        material: MaterialRef,
        property: &str,
        value: &[f32],
    ) -> Result<(), Self::Error> {
        self.target.set_material_color(material, property, value)
    }

    #[inline(always)]
    fn set_texture_transform(
        &mut self,
        material: MaterialRef,
        scale: Option<[f32; 2]>,
        offset: Option<[f32; 2]>,
    ) -> Result<(), Self::Error> {
        self.target.set_texture_transform(material, scale, offset)
    }

    #[inline(always)]
    fn set_emissive_intensity(
        &mut self,
        material: MaterialRef,
        intensity: f32,
    ) -> Result<(), Self::Error> {
        self.target.set_emissive_intensity(material, intensity)
    }
}

impl<T, C> MtoonPipelineAccess for CoordinateSpaceTarget<'_, T, C>
where
    T: MtoonPipelineAccess,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn set_mtoon_pipeline_passes(
        &mut self,
        material: MaterialRef,
        passes: &[MtoonPipelinePass],
    ) -> Result<(), Self::Error> {
        self.target.set_mtoon_pipeline_passes(material, passes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MtoonLightAccumulation {
    Tuned,
    ThreeVrm,
}

impl MtoonLightAccumulation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tuned => "tuned",
            Self::ThreeVrm => "three-vrm",
        }
    }

    pub fn is_three_vrm(self) -> bool {
        matches!(self, Self::ThreeVrm)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MtoonLightingConfig {
    pub accumulation: MtoonLightAccumulation,
    pub exposure: f32,
    pub ambient_base: f32,
    pub ambient_gi_scale: f32,
    pub pbr_ambient: f32,
}

impl Default for MtoonLightingConfig {
    fn default() -> Self {
        Self {
            accumulation: MtoonLightAccumulation::ThreeVrm,
            exposure: 0.78,
            ambient_base: 0.12,
            ambient_gi_scale: 0.20,
            pbr_ambient: 0.03183099,
        }
    }
}

impl MtoonLightingConfig {
    pub fn effective_values(self) -> MtoonLightingValues {
        match self.accumulation {
            MtoonLightAccumulation::Tuned => MtoonLightingValues {
                exposure: self.exposure,
                ambient_base: self.ambient_base,
                ambient_gi_scale: self.ambient_gi_scale,
                pbr_ambient: self.pbr_ambient,
            },
            MtoonLightAccumulation::ThreeVrm => MtoonLightingValues {
                exposure: 1.0,
                ambient_base: self.pbr_ambient,
                ambient_gi_scale: 0.0,
                pbr_ambient: self.pbr_ambient,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MtoonLightingValues {
    pub exposure: f32,
    pub ambient_base: f32,
    pub ambient_gi_scale: f32,
    pub pbr_ambient: f32,
}

impl MtoonLightingValues {
    pub fn to_array(self) -> [f32; 4] {
        [
            self.exposure,
            self.ambient_base,
            self.ambient_gi_scale,
            self.pbr_ambient,
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MtoonMaterialDescriptor {
    pub material: MaterialRef,
    pub name: Option<String>,
    pub pass: MtoonPipelinePass,
    pub render_queue: MtoonRenderQueue,
    pub transparent_with_z_write: bool,
    pub render_queue_offset_number: i32,
    pub textures: MtoonTextureSet,
    pub base_color_factor: [f32; 4],
    pub emissive_factor: [f32; 3],
    pub cutoff_factor: f32,
    pub shade_color_factor: [f32; 3],
    pub receive_shadow_rate_factor: f32,
    pub shading_grade_rate_factor: f32,
    pub shading_shift_factor: f32,
    pub shading_shift_texture_scale: f32,
    pub shading_toony_factor: f32,
    pub light_color_attenuation_factor: f32,
    pub gi_equalization_factor: f32,
    pub matcap_factor: [f32; 3],
    pub parametric_rim_color_factor: [f32; 3],
    pub rim_lighting_mix_factor: f32,
    pub parametric_rim_fresnel_power_factor: f32,
    pub parametric_rim_lift_factor: f32,
    pub outline_width_factor: f32,
    pub outline_color_factor: [f32; 3],
    pub outline_lighting_mix_factor: f32,
    pub uv_animation: vrm_core::UvAnimation,
    pub emissive_strength: EmissiveStrength,
    pub debug_mode: MtoonDebugMode,
    pub v0_compat_shade: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MtoonRendererMaterialPlan {
    pub material: MaterialRef,
    pub name: Option<String>,
    pub pass: MtoonRendererPass,
    pub pipeline: MtoonRendererPipelineState,
    pub shader: MtoonShaderParameters,
    pub textures: MtoonRendererTextureRefs,
    pub texture_bindings: Vec<MtoonTextureBindingPlan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MtoonRendererPass {
    Base,
    Outline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MtoonRendererPipelineState {
    pub render_order: i32,
    pub phase_order: i32,
    pub alpha_mode: MtoonAlphaMode,
    pub cull_mode: MtoonCullMode,
    pub depth_test: bool,
    pub depth_write: bool,
    pub blend: bool,
    pub transparent_with_z_write: bool,
    pub outline_width_mode: Option<OutlineWidthMode>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MtoonShaderParameters {
    pub base_color_factor: [f32; 4],
    pub shade_color_factor: [f32; 3],
    pub emissive_color: [f32; 3],
    pub cutoff_factor: f32,
    pub receive_shadow_rate_factor: f32,
    pub shading_grade_rate_factor: f32,
    pub shading_shift_factor: f32,
    pub shading_shift_texture_scale: f32,
    pub shading_toony_factor: f32,
    pub light_color_attenuation_factor: f32,
    pub gi_equalization_factor: f32,
    pub matcap_factor: [f32; 3],
    pub parametric_rim_color_factor: [f32; 3],
    pub rim_lighting_mix_factor: f32,
    pub parametric_rim_fresnel_power_factor: f32,
    pub parametric_rim_lift_factor: f32,
    pub outline_width_factor: f32,
    pub outline_color_factor: [f32; 3],
    pub outline_lighting_mix_factor: f32,
    pub uv_animation: UvAnimation,
    pub debug_mode: MtoonDebugMode,
    pub v0_compat_shade: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MtoonRendererTextureRefs {
    pub main: Option<TextureRef>,
    pub shade_multiply: Option<TextureRef>,
    pub shading_shift: Option<TextureRef>,
    pub normal: Option<TextureRef>,
    pub matcap: Option<TextureRef>,
    pub rim_multiply: Option<TextureRef>,
    pub outline_width: Option<TextureRef>,
    pub uv_animation_mask: Option<TextureRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MtoonTextureBindingPlan {
    pub slot: MtoonTextureSlot,
    pub texture: TextureRef,
    pub sampler: MtoonSamplerHint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MtoonTextureSlot {
    Main,
    ShadeMultiply,
    ShadingShift,
    Normal,
    Matcap,
    RimMultiply,
    OutlineWidth,
    UvAnimationMask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MtoonSamplerHint {
    LinearRepeat,
    NormalMapLinearRepeat,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererMaterialPipelinePlan {
    pub render_order: i32,
    pub phase_order: i32,
    pub cull_mode: RendererMaterialCullMode,
    pub alpha_mode: RendererMaterialAlphaMode,
    pub depth_write: bool,
    pub blend: bool,
    pub alpha_cutoff: f32,
    pub transparent_order_offset: Option<i32>,
    pub mtoon_transparent_with_z_write: Option<bool>,
}

impl Default for RendererMaterialPipelinePlan {
    fn default() -> Self {
        Self {
            render_order: 2000,
            phase_order: 2000,
            cull_mode: RendererMaterialCullMode::Back,
            alpha_mode: RendererMaterialAlphaMode::Opaque,
            depth_write: true,
            blend: false,
            alpha_cutoff: 0.5,
            transparent_order_offset: None,
            mtoon_transparent_with_z_write: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererMaterialCullMode {
    Off,
    Front,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererMaterialAlphaMode {
    Opaque,
    Mask,
    Blend,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfMaterialPipelineOverride {
    pub alpha_mode: GltfMaterialAlphaMode,
    pub alpha_cutoff: Option<f32>,
    pub double_sided: bool,
}

impl Default for GltfMaterialPipelineOverride {
    fn default() -> Self {
        Self {
            alpha_mode: GltfMaterialAlphaMode::Opaque,
            alpha_cutoff: None,
            double_sided: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GltfMaterialAlphaMode {
    #[default]
    Opaque,
    Mask,
    Blend,
}

impl MtoonRendererMaterialPlan {
    pub fn from_descriptor(descriptor: &MtoonMaterialDescriptor) -> Self {
        let phase_order = mtoon_render_phase_order(
            descriptor.transparent_with_z_write,
            descriptor.render_queue_offset_number,
        );
        let (pass, pipeline) = match descriptor.pass {
            MtoonPipelinePass::Base(hints) => (
                MtoonRendererPass::Base,
                MtoonRendererPipelineState {
                    render_order: hints.render_order,
                    phase_order,
                    alpha_mode: hints.alpha_mode,
                    cull_mode: hints.cull_mode,
                    depth_test: hints.depth_test,
                    depth_write: hints.depth_write,
                    blend: hints.blend,
                    transparent_with_z_write: descriptor.transparent_with_z_write,
                    outline_width_mode: None,
                },
            ),
            MtoonPipelinePass::Outline(hints) => (
                MtoonRendererPass::Outline,
                MtoonRendererPipelineState {
                    render_order: hints.render_order,
                    phase_order,
                    alpha_mode: MtoonAlphaMode::Opaque,
                    cull_mode: hints.cull_mode,
                    depth_test: true,
                    depth_write: true,
                    blend: false,
                    transparent_with_z_write: descriptor.transparent_with_z_write,
                    outline_width_mode: Some(hints.width_mode),
                },
            ),
        };

        Self {
            material: descriptor.material,
            name: descriptor.name.clone(),
            pass,
            pipeline,
            shader: MtoonShaderParameters {
                base_color_factor: descriptor.base_color_factor,
                shade_color_factor: descriptor.shade_color_factor,
                emissive_color: descriptor
                    .emissive_factor
                    .map(|channel| channel * descriptor.emissive_strength.0),
                cutoff_factor: descriptor.cutoff_factor,
                receive_shadow_rate_factor: descriptor.receive_shadow_rate_factor,
                shading_grade_rate_factor: descriptor.shading_grade_rate_factor,
                shading_shift_factor: descriptor.shading_shift_factor,
                shading_shift_texture_scale: descriptor.shading_shift_texture_scale,
                shading_toony_factor: descriptor.shading_toony_factor,
                light_color_attenuation_factor: descriptor.light_color_attenuation_factor,
                gi_equalization_factor: descriptor.gi_equalization_factor,
                matcap_factor: descriptor.matcap_factor,
                parametric_rim_color_factor: descriptor.parametric_rim_color_factor,
                rim_lighting_mix_factor: descriptor.rim_lighting_mix_factor,
                parametric_rim_fresnel_power_factor: descriptor.parametric_rim_fresnel_power_factor,
                parametric_rim_lift_factor: descriptor.parametric_rim_lift_factor,
                outline_width_factor: descriptor.outline_width_factor,
                outline_color_factor: descriptor.outline_color_factor,
                outline_lighting_mix_factor: descriptor.outline_lighting_mix_factor,
                uv_animation: descriptor.uv_animation,
                debug_mode: descriptor.debug_mode,
                v0_compat_shade: descriptor.v0_compat_shade,
            },
            textures: MtoonRendererTextureRefs::from_set(&descriptor.textures),
            texture_bindings: mtoon_texture_binding_plans(&descriptor.textures),
        }
    }
}

impl RendererMaterialPipelinePlan {
    pub fn from_mtoon_plan(plan: &MtoonRendererMaterialPlan) -> Self {
        Self {
            render_order: plan.pipeline.render_order,
            phase_order: plan.pipeline.phase_order,
            cull_mode: renderer_material_cull_mode(plan.pipeline.cull_mode),
            alpha_mode: renderer_material_alpha_mode(plan.pipeline.alpha_mode),
            depth_write: plan.pipeline.depth_write,
            blend: plan.pipeline.blend,
            alpha_cutoff: plan.shader.cutoff_factor,
            transparent_order_offset: Some(plan.pipeline.phase_order),
            mtoon_transparent_with_z_write: Some(plan.pipeline.transparent_with_z_write),
        }
    }

    pub fn with_gltf_override(mut self, override_: GltfMaterialPipelineOverride) -> Self {
        match override_.alpha_mode {
            GltfMaterialAlphaMode::Opaque => {}
            GltfMaterialAlphaMode::Mask => {
                self.alpha_mode = RendererMaterialAlphaMode::Mask;
                self.depth_write = true;
                self.blend = false;
                self.alpha_cutoff = override_.alpha_cutoff.unwrap_or(0.5);
            }
            GltfMaterialAlphaMode::Blend => {
                self.alpha_mode = RendererMaterialAlphaMode::Blend;
                self.depth_write = self.mtoon_transparent_with_z_write.unwrap_or(false);
                self.blend = true;
                self.render_order = self
                    .transparent_order_offset
                    .map_or(self.render_order.max(3000), |offset| 3000 + offset);
            }
        }

        if override_.double_sided {
            self.cull_mode = RendererMaterialCullMode::Off;
        }

        self
    }
}

pub fn renderer_material_pipeline_plan(
    document: &VrmDocument,
    material: Option<MaterialRef>,
    options: MtoonMaterializationOptions,
    gltf_override: Option<GltfMaterialPipelineOverride>,
) -> RendererMaterialPipelinePlan {
    let mut plan = material
        .and_then(|material| {
            mtoon_renderer_material_plans(document, options)
                .into_iter()
                .find(|plan| plan.material == material && plan.pass == MtoonRendererPass::Base)
        })
        .as_ref()
        .map(RendererMaterialPipelinePlan::from_mtoon_plan)
        .unwrap_or_default();

    if let Some(gltf_override) = gltf_override {
        plan = plan.with_gltf_override(gltf_override);
    }

    plan
}

impl MtoonRendererTextureRefs {
    pub fn from_set(textures: &MtoonTextureSet) -> Self {
        Self {
            main: textures.main_texture,
            shade_multiply: textures.shade_multiply_texture,
            shading_shift: textures.shading_shift_texture,
            normal: textures.normal_texture,
            matcap: textures.matcap_texture,
            rim_multiply: textures.rim_multiply_texture,
            outline_width: textures.outline_width_multiply_texture,
            uv_animation_mask: textures.uv_animation_mask_texture,
        }
    }
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

impl<T, C> MtoonMaterializer for CoordinateSpaceTarget<'_, T, C>
where
    T: MtoonMaterializer,
    C: CoordinateSpaceMapping,
{
    type Descriptor = T::Descriptor;
    type Error = T::Error;

    #[inline(always)]
    fn materialize_mtoon(
        &mut self,
        descriptor: &MtoonMaterialDescriptor,
    ) -> Result<Self::Descriptor, Self::Error> {
        self.target.materialize_mtoon(descriptor)
    }
}

pub trait TextureResolver {
    type Texture;
    type Error;

    fn resolve_texture(&self, texture: TextureRef) -> Result<Self::Texture, Self::Error>;
}

impl<T, C> TextureResolver for CoordinateSpaceTarget<'_, T, C>
where
    T: TextureResolver,
    C: CoordinateSpaceMapping,
{
    type Texture = T::Texture;
    type Error = T::Error;

    #[inline(always)]
    fn resolve_texture(&self, texture: TextureRef) -> Result<Self::Texture, Self::Error> {
        self.target.resolve_texture(texture)
    }
}

pub trait VisibilityAccess {
    type Error;

    fn set_node_visible(&mut self, node: NodeRef, visible: bool) -> Result<(), Self::Error>;
}

impl<T, C> VisibilityAccess for CoordinateSpaceTarget<'_, T, C>
where
    T: VisibilityAccess,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn set_node_visible(&mut self, node: NodeRef, visible: bool) -> Result<(), Self::Error> {
        self.target.set_node_visible(node, visible)
    }
}

pub trait LookAtAccess {
    type Error;

    fn set_look_at_rotation(&mut self, rotation: Quat) -> Result<(), Self::Error>;
}

impl<T, C> LookAtAccess for CoordinateSpaceTarget<'_, T, C>
where
    T: LookAtAccess,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn set_look_at_rotation(&mut self, rotation: Quat) -> Result<(), Self::Error> {
        self.target
            .set_look_at_rotation(C::from_vrm_rotation(rotation))
    }
}

pub trait AnimationSink {
    type Error;

    fn apply_expression(&mut self, expression: &AppliedExpression) -> Result<(), Self::Error>;
    fn apply_runtime_events(&mut self, events: &RuntimeEvents) -> Result<(), Self::Error>;
}

impl<T, C> AnimationSink for CoordinateSpaceTarget<'_, T, C>
where
    T: AnimationSink,
    C: CoordinateSpaceMapping,
{
    type Error = T::Error;

    #[inline(always)]
    fn apply_expression(&mut self, expression: &AppliedExpression) -> Result<(), Self::Error> {
        self.target.apply_expression(expression)
    }

    #[inline(always)]
    fn apply_runtime_events(&mut self, events: &RuntimeEvents) -> Result<(), Self::Error> {
        self.target.apply_runtime_events(events)
    }
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
            + WorldMatrixAccess<Error = E>
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

    pub fn tick_with_spring_parity_and_look_at<T, E>(
        &mut self,
        target: &mut T,
        spring: Option<(&SpringRestMap, &mut CenterSpringRuntimeState)>,
    ) -> Result<(), AdapterError<E>>
    where
        T: TransformAccess<Error = E>
            + WorldTransformAccess<Error = E>
            + WorldMatrixAccess<Error = E>
            + WorldTransformUpdate<Error = E>
            + SceneGraph<Error = E>
            + ConstraintRestAccess<Error = E>
            + MorphTargetAccess<Error = E>
            + MaterialAccess<Error = E>
            + MtoonPipelineAccess<Error = E>
            + VisibilityAccess<Error = E>
            + LookAtAccess<Error = E>,
    {
        if self.apply_vrm0_orientation
            && !self.vrm0_orientation_applied
            && let Some(root) = self.root
        {
            apply_vrm0_orientation_compensation(target, self.document, root)?;
            self.vrm0_orientation_applied = true;
        }
        if let Some(frame) = self.animation_frame {
            apply_animation_frame_with_look_at(target, self.document, frame)?;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RuntimePipelineOptions {
    pub fixed_delta: DeltaTime,
    pub max_substeps: usize,
    pub view_mode: ViewMode,
    pub apply_vrm0_orientation: bool,
}

impl Default for RuntimePipelineOptions {
    fn default() -> Self {
        Self {
            fixed_delta: DeltaTime(1.0 / 60.0),
            max_substeps: 4,
            view_mode: ViewMode::ThirdPerson,
            apply_vrm0_orientation: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimePipelineStage {
    Vrm0Orientation,
    AnimationFrame,
    LookAt,
    RuntimeUpdate,
    Expressions,
    NodeConstraints,
    SpringBone,
    MtoonPipeline,
    EmissiveStrength,
    FirstPersonVisibility,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RuntimeStageReport {
    pub stage: RuntimePipelineStage,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimePipelineReport {
    pub requested_delta: DeltaTime,
    pub consumed_delta: DeltaTime,
    pub fixed_delta: DeltaTime,
    pub substeps: usize,
    pub accumulator: DeltaTime,
    pub dropped_substeps: usize,
    pub stages: Vec<RuntimeStageReport>,
}

impl RuntimePipelineReport {
    fn new(requested_delta: DeltaTime, fixed_delta: DeltaTime, accumulator: DeltaTime) -> Self {
        Self {
            requested_delta,
            consumed_delta: DeltaTime(0.0),
            fixed_delta,
            substeps: 0,
            accumulator,
            dropped_substeps: 0,
            stages: Vec::new(),
        }
    }

    fn push_stage(&mut self, stage: RuntimePipelineStage, count: usize) {
        self.stages.push(RuntimeStageReport { stage, count });
    }

    pub fn stage_count(&self, stage: RuntimePipelineStage) -> usize {
        self.stages
            .iter()
            .filter(|entry| entry.stage == stage)
            .map(|entry| entry.count)
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimePipelineMixerReport {
    pub mixer: AnimationMixerFrame,
    pub runtime: RuntimePipelineReport,
}

#[derive(Debug)]
pub struct VrmRuntimePipeline<'a> {
    document: &'a VrmDocument,
    runtime: Runtime,
    animation_mixer: VrmAnimationMixer,
    options: RuntimePipelineOptions,
    root: Option<NodeRef>,
    accumulator: f32,
    vrm0_orientation_applied: bool,
    spring_rest: Option<SpringRestMap>,
    spring_state: Option<CenterSpringRuntimeState>,
}

impl<'a> VrmRuntimePipeline<'a> {
    pub fn new(document: &'a VrmDocument) -> Self {
        Self::with_options(document, RuntimePipelineOptions::default())
    }

    pub fn with_options(document: &'a VrmDocument, options: RuntimePipelineOptions) -> Self {
        Self {
            document,
            runtime: Runtime::from_document(document),
            animation_mixer: VrmAnimationMixer::default(),
            options,
            root: None,
            accumulator: 0.0,
            vrm0_orientation_applied: false,
            spring_rest: None,
            spring_state: None,
        }
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    pub fn animation_mixer(&self) -> &VrmAnimationMixer {
        &self.animation_mixer
    }

    pub fn animation_mixer_mut(&mut self) -> &mut VrmAnimationMixer {
        &mut self.animation_mixer
    }

    pub fn options(&self) -> RuntimePipelineOptions {
        self.options
    }

    pub fn set_options(&mut self, options: RuntimePipelineOptions) {
        self.options = options;
    }

    pub fn set_root(&mut self, root: Option<NodeRef>) {
        self.root = root;
        self.vrm0_orientation_applied = false;
    }

    pub fn set_view_mode(&mut self, mode: ViewMode) {
        self.options.view_mode = mode;
    }

    pub fn spring_rest(&self) -> Option<&SpringRestMap> {
        self.spring_rest.as_ref()
    }

    pub fn spring_state(&self) -> Option<&CenterSpringRuntimeState> {
        self.spring_state.as_ref()
    }

    pub fn spring_state_mut(&mut self) -> Option<&mut CenterSpringRuntimeState> {
        self.spring_state.as_mut()
    }

    pub fn capture_spring_rest<T, E>(&mut self, target: &T) -> Result<(), AdapterError<E>>
    where
        T: TransformAccess<Error = E> + WorldTransformAccess<Error = E> + SceneGraph<Error = E>,
    {
        if let Feature::Present(system) = &self.document.spring_bone {
            let rest = SpringRestMap::capture(target, system)?;
            let state = rest.runtime_state(system);
            self.spring_rest = Some(rest);
            self.spring_state = Some(state);
        }
        Ok(())
    }

    pub fn reset_spring_state(&mut self) {
        if let (Feature::Present(system), Some(rest)) =
            (&self.document.spring_bone, &self.spring_rest)
        {
            self.spring_state = Some(rest.runtime_state(system));
        }
    }

    pub fn tick<T, E>(
        &mut self,
        target: &mut T,
        delta: DeltaTime,
        animation_frame: Option<&VrmAnimationFrame>,
    ) -> Result<RuntimePipelineReport, AdapterError<E>>
    where
        T: TransformAccess<Error = E>
            + WorldTransformAccess<Error = E>
            + WorldMatrixAccess<Error = E>
            + WorldTransformUpdate<Error = E>
            + SceneGraph<Error = E>
            + ConstraintRestAccess<Error = E>
            + MorphTargetAccess<Error = E>
            + MaterialAccess<Error = E>
            + MtoonPipelineAccess<Error = E>
            + VisibilityAccess<Error = E>
            + LookAtAccess<Error = E>,
    {
        let fixed_delta = normalized_fixed_delta(self.options.fixed_delta);
        let mut report =
            RuntimePipelineReport::new(delta, fixed_delta, DeltaTime(self.accumulator));
        let substeps = self.consume_substeps(delta, &mut report);

        if substeps == 0 {
            let events = self
                .runtime
                .update(DeltaTime(0.0))
                .map_err(AdapterError::Runtime)?;
            self.apply_step(target, &events, animation_frame, false, &mut report)?;
            return Ok(report);
        }

        for _ in 0..substeps {
            let events = self
                .runtime
                .update(fixed_delta)
                .map_err(AdapterError::Runtime)?;
            self.apply_step(target, &events, animation_frame, true, &mut report)?;
            report.substeps += 1;
            report.consumed_delta.0 += fixed_delta.0;
        }
        report.accumulator = DeltaTime(self.accumulator);
        Ok(report)
    }

    pub fn tick_mixer<T, E>(
        &mut self,
        target: &mut T,
        delta: DeltaTime,
    ) -> Result<RuntimePipelineMixerReport, AdapterError<E>>
    where
        T: TransformAccess<Error = E>
            + WorldTransformAccess<Error = E>
            + WorldMatrixAccess<Error = E>
            + WorldTransformUpdate<Error = E>
            + SceneGraph<Error = E>
            + ConstraintRestAccess<Error = E>
            + MorphTargetAccess<Error = E>
            + MaterialAccess<Error = E>
            + MtoonPipelineAccess<Error = E>
            + VisibilityAccess<Error = E>
            + LookAtAccess<Error = E>,
    {
        let mixer = self
            .animation_mixer
            .update(delta)
            .map_err(AdapterError::AnimationMixer)?;
        self.apply_mixer_expression_inputs(&mixer.frame);
        let mut adapter_frame = mixer.frame.clone();
        adapter_frame.preset_expressions.clear();
        adapter_frame.custom_expressions.clear();
        let runtime = self.tick(target, delta, Some(&adapter_frame))?;
        Ok(RuntimePipelineMixerReport { mixer, runtime })
    }

    fn apply_mixer_expression_inputs(&mut self, frame: &VrmAnimationFrame) {
        for (name, weight) in &frame.preset_expressions {
            self.runtime
                .expression_manager
                .set_value(name.as_str(), *weight);
        }
        for (name, weight) in &frame.custom_expressions {
            self.runtime.expression_manager.set_value(name, *weight);
        }
    }

    fn consume_substeps(&mut self, delta: DeltaTime, report: &mut RuntimePipelineReport) -> usize {
        self.accumulator += sanitized_seconds(delta);
        let fixed = normalized_fixed_delta(self.options.fixed_delta).0;
        let available = (self.accumulator / fixed).floor() as usize;
        let max_substeps = self.options.max_substeps.max(1);
        let substeps = available.min(max_substeps);
        self.accumulator -= substeps as f32 * fixed;
        report.dropped_substeps = available.saturating_sub(substeps);
        if report.dropped_substeps > 0 {
            self.accumulator = 0.0;
        }
        report.accumulator = DeltaTime(self.accumulator);
        substeps
    }

    fn apply_step<T, E>(
        &mut self,
        target: &mut T,
        events: &RuntimeEvents,
        animation_frame: Option<&VrmAnimationFrame>,
        step_spring: bool,
        report: &mut RuntimePipelineReport,
    ) -> Result<(), AdapterError<E>>
    where
        T: TransformAccess<Error = E>
            + WorldTransformAccess<Error = E>
            + WorldMatrixAccess<Error = E>
            + WorldTransformUpdate<Error = E>
            + SceneGraph<Error = E>
            + ConstraintRestAccess<Error = E>
            + MorphTargetAccess<Error = E>
            + MaterialAccess<Error = E>
            + MtoonPipelineAccess<Error = E>
            + VisibilityAccess<Error = E>
            + LookAtAccess<Error = E>,
    {
        self.push_static_stage_reports(animation_frame, events, step_spring, report);
        let mut driver = VrmRuntimeDriver::new(self.document)
            .with_runtime_events(events)
            .with_view_mode(self.options.view_mode)
            .with_vrm0_orientation(self.options.apply_vrm0_orientation);
        if let Some(frame) = animation_frame {
            driver = driver.with_animation_frame(frame);
        }
        if let Some(root) = self.root {
            driver = driver.with_root(root);
        }
        driver.vrm0_orientation_applied = self.vrm0_orientation_applied;

        let spring = if step_spring {
            self.spring_rest.as_ref().zip(self.spring_state.as_mut())
        } else {
            None
        };
        driver.tick_with_spring_parity_and_look_at(target, spring)?;
        self.vrm0_orientation_applied = driver.vrm0_orientation_applied;
        Ok(())
    }

    fn push_static_stage_reports(
        &self,
        animation_frame: Option<&VrmAnimationFrame>,
        events: &RuntimeEvents,
        step_spring: bool,
        report: &mut RuntimePipelineReport,
    ) {
        if self.options.apply_vrm0_orientation
            && !self.vrm0_orientation_applied
            && self.root.is_some()
        {
            report.push_stage(RuntimePipelineStage::Vrm0Orientation, 1);
        }
        if let Some(frame) = animation_frame {
            report.push_stage(RuntimePipelineStage::AnimationFrame, 1);
            report.push_stage(
                RuntimePipelineStage::LookAt,
                usize::from(frame.look_at.is_some()),
            );
        }
        report.push_stage(RuntimePipelineStage::RuntimeUpdate, 1);
        report.push_stage(RuntimePipelineStage::Expressions, events.expressions.len());
        report.push_stage(
            RuntimePipelineStage::NodeConstraints,
            events.constraints.len(),
        );
        report.push_stage(
            RuntimePipelineStage::SpringBone,
            usize::from(step_spring) * events.springs.len(),
        );
        report.push_stage(
            RuntimePipelineStage::MtoonPipeline,
            mtoon_material_count(self.document),
        );
        report.push_stage(
            RuntimePipelineStage::EmissiveStrength,
            self.document.materials.len(),
        );
        report.push_stage(
            RuntimePipelineStage::FirstPersonVisibility,
            first_person_annotation_count(self.document),
        );
    }
}

fn normalized_fixed_delta(delta: DeltaTime) -> DeltaTime {
    let seconds = sanitized_seconds(delta);
    if seconds > f32::EPSILON {
        DeltaTime(seconds)
    } else {
        DeltaTime(1.0 / 60.0)
    }
}

fn sanitized_seconds(delta: DeltaTime) -> f32 {
    if delta.0.is_finite() {
        delta.0.max(0.0)
    } else {
        0.0
    }
}

fn mtoon_material_count(document: &VrmDocument) -> usize {
    document
        .materials
        .iter()
        .filter(|material| matches!(material.mtoon, Feature::Present(_)))
        .count()
}

fn first_person_annotation_count(document: &VrmDocument) -> usize {
    document
        .first_person
        .as_ref()
        .map(|first_person| first_person.mesh_annotations.len())
        .unwrap_or(0)
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

pub fn mtoon_renderer_material_plans(
    document: &VrmDocument,
    options: MtoonMaterializationOptions,
) -> Vec<MtoonRendererMaterialPlan> {
    mtoon_material_descriptors(document, options)
        .iter()
        .map(MtoonRendererMaterialPlan::from_descriptor)
        .collect()
}

pub fn mtoon_texture_binding_plans(textures: &MtoonTextureSet) -> Vec<MtoonTextureBindingPlan> {
    [
        (
            MtoonTextureSlot::Main,
            textures.main_texture,
            MtoonSamplerHint::LinearRepeat,
        ),
        (
            MtoonTextureSlot::ShadeMultiply,
            textures.shade_multiply_texture,
            MtoonSamplerHint::LinearRepeat,
        ),
        (
            MtoonTextureSlot::ShadingShift,
            textures.shading_shift_texture,
            MtoonSamplerHint::LinearRepeat,
        ),
        (
            MtoonTextureSlot::Normal,
            textures.normal_texture,
            MtoonSamplerHint::NormalMapLinearRepeat,
        ),
        (
            MtoonTextureSlot::Matcap,
            textures.matcap_texture,
            MtoonSamplerHint::LinearRepeat,
        ),
        (
            MtoonTextureSlot::RimMultiply,
            textures.rim_multiply_texture,
            MtoonSamplerHint::LinearRepeat,
        ),
        (
            MtoonTextureSlot::OutlineWidth,
            textures.outline_width_multiply_texture,
            MtoonSamplerHint::LinearRepeat,
        ),
        (
            MtoonTextureSlot::UvAnimationMask,
            textures.uv_animation_mask_texture,
            MtoonSamplerHint::LinearRepeat,
        ),
    ]
    .into_iter()
    .filter_map(|(slot, texture, sampler)| {
        texture.map(|texture| MtoonTextureBindingPlan {
            slot,
            texture,
            sampler,
        })
    })
    .collect()
}

pub fn mtoon_render_phase_order(transparent_with_z_write: bool, render_queue_offset: i32) -> i32 {
    let queue_offset = if transparent_with_z_write { 0 } else { 19 };
    queue_offset + render_queue_offset
}

fn renderer_material_cull_mode(mode: MtoonCullMode) -> RendererMaterialCullMode {
    match mode {
        MtoonCullMode::Off => RendererMaterialCullMode::Off,
        MtoonCullMode::Front => RendererMaterialCullMode::Front,
        MtoonCullMode::Back => RendererMaterialCullMode::Back,
    }
}

fn renderer_material_alpha_mode(mode: MtoonAlphaMode) -> RendererMaterialAlphaMode {
    match mode {
        MtoonAlphaMode::Opaque => RendererMaterialAlphaMode::Opaque,
        MtoonAlphaMode::Mask => RendererMaterialAlphaMode::Mask,
        MtoonAlphaMode::Blend => RendererMaterialAlphaMode::Blend,
    }
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
        render_queue: mtoon.render_queue,
        transparent_with_z_write: mtoon.transparent_with_z_write,
        render_queue_offset_number: mtoon.render_queue_offset_number,
        textures: mtoon.textures.clone(),
        base_color_factor: mtoon.base_color_factor,
        emissive_factor: mtoon.emissive_factor,
        cutoff_factor: mtoon.cutoff_factor,
        shade_color_factor: mtoon.shade_color_factor,
        receive_shadow_rate_factor: mtoon.receive_shadow_rate_factor,
        shading_grade_rate_factor: mtoon.shading_grade_rate_factor,
        shading_shift_factor: mtoon.shading_shift_factor,
        shading_shift_texture_scale: mtoon.shading_shift_texture_scale,
        shading_toony_factor: mtoon.shading_toony_factor,
        light_color_attenuation_factor: mtoon.light_color_attenuation_factor,
        gi_equalization_factor: mtoon.gi_equalization_factor,
        matcap_factor: mtoon.matcap_factor,
        parametric_rim_color_factor: mtoon.parametric_rim_color_factor,
        rim_lighting_mix_factor: mtoon.rim_lighting_mix_factor,
        parametric_rim_fresnel_power_factor: mtoon.parametric_rim_fresnel_power_factor,
        parametric_rim_lift_factor: mtoon.parametric_rim_lift_factor,
        outline_width_factor: mtoon.outline_width_factor,
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
    T: WorldMatrixAccess<Error = E>,
{
    spring
        .collider_groups
        .iter()
        .filter_map(|group_index| system.collider_groups.get(*group_index))
        .flat_map(|group| &group.colliders)
        .filter_map(|collider_index| system.colliders.get(*collider_index))
        .map(|collider| {
            target
                .world_matrix(collider.node)
                .map(|world| collider_shape_from_world_matrix(collider, world))
                .map_err(AdapterError::Target)
        })
        .collect()
}

fn collider_shape_from_world_matrix(
    collider: &vrm_core::SpringCollider,
    world: Mat4,
) -> ColliderShape {
    match &collider.shape {
        ColliderShape::Sphere {
            offset,
            radius,
            inside,
        } => ColliderShape::Sphere {
            offset: world.transform_point3(*offset),
            radius: *radius,
            inside: *inside,
        },
        ColliderShape::Capsule {
            offset,
            radius,
            tail,
            inside,
        } => ColliderShape::Capsule {
            offset: world.transform_point3(*offset),
            radius: *radius,
            tail: world.transform_point3(*tail),
            inside: *inside,
        },
        ColliderShape::Plane {
            offset,
            normal,
            inside,
        } => ColliderShape::Plane {
            offset: world.transform_point3(*offset),
            normal: world.transform_vector3(*normal).normalize_or_zero(),
            inside: *inside,
        },
    }
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
            let mut first_child = None;
            target
                .visit_children(joint.node, |child| {
                    first_child.get_or_insert(child);
                })
                .map_err(AdapterError::Target)?;
            let child_world = first_child
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
        + WorldMatrixAccess<Error = E>
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
        let source_hips_y = frame
            .source_rest_hips_position
            .map(|position| position.y)
            .filter(|y| y.abs() > f32::EPSILON);
        let translation_scale = source_hips_y.map_or(1.0, |animation_y| {
            if rest_translation.y.abs() > f32::EPSILON {
                rest_translation.y / animation_y
            } else {
                1.0
            }
        });
        let rotation = pose
            .get(&HumanBoneName::Hips)
            .map(|transform| transform.rotation)
            .unwrap_or(Quat::IDENTITY);
        pose.insert(
            HumanBoneName::Hips,
            vrm_core::PoseTransform {
                translation: translation * translation_scale - rest_translation,
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
    #[error("runtime error: {0}")]
    Runtime(vrm_runtime::RuntimeError),
    #[error("animation mixer error: {0}")]
    AnimationMixer(vrm_runtime::AnimationMixerError),
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
        FirstPersonMeshAnnotation, HdrEmissiveMultiplier, HumanBone, Humanoid, Material,
        MtoonMaterial, MtoonRenderQueue, MtoonTextureSet, OutlineWidthMode, PoseTransform,
        RotationTrack, ScalarTrack, Transform, TranslationTrack, VrmAnimation, VrmDocument,
    };
    use vrm_runtime::sample_vrm_animation;

    #[test]
    fn clip_depth_mappings_convert_to_webgl_reference_depth() {
        assert_eq!(ZeroToOneDepth::DEPTH_RANGE_LABEL, "zero-to-one-ndc");
        assert_eq!(ZeroToOneDepth::webgl_depth_from_ndc_z(0.25), -0.5);
        assert_eq!(
            ReverseZeroToOneDepth::DEPTH_RANGE_LABEL,
            "reverse-zero-to-one-ndc"
        );
        assert_eq!(ReverseZeroToOneDepth::webgl_depth_from_ndc_z(0.25), 0.5);
        assert_eq!(
            NegativeOneToOneDepth::DEPTH_RANGE_LABEL,
            "negative-one-to-one-ndc"
        );
        assert_eq!(NegativeOneToOneDepth::webgl_depth_from_ndc_z(0.25), 0.25);
    }

    #[test]
    fn renderer_front_face_uses_y_down_screen_area() {
        assert_eq!(RendererFrontFace::Ccw.as_str(), "ccw");
        assert_eq!(RendererFrontFace::Cw.as_str(), "cw");
        assert!(RendererFrontFace::Ccw.is_gpu_front_facing(-1.0));
        assert!(!RendererFrontFace::Ccw.is_gpu_front_facing(1.0));
        assert!(RendererFrontFace::Cw.is_gpu_front_facing(1.0));
        assert!(!RendererFrontFace::Cw.is_gpu_front_facing(-1.0));
    }

    #[test]
    fn coordinate_space_mapping_preserves_vrm_space() {
        let transform = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_y(0.25),
            scale: Vec3::new(2.0, 3.0, 4.0),
        };

        assert_eq!(VrmCoordinateSpace::LABEL, "vrm-gltf-right-handed-y-up");
        assert!(!coordinate_space_mirrors::<VrmCoordinateSpace>());
        assert_eq!(VrmCoordinateSpace::from_vrm_transform(transform), transform);
        assert_eq!(VrmCoordinateSpace::to_vrm_transform(transform), transform);
    }

    #[test]
    fn coordinate_space_mapping_can_flip_z_handedness() {
        let rotation = Quat::from_rotation_x(0.4) * Quat::from_rotation_y(-0.7);
        let transform = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation,
            scale: Vec3::new(2.0, 3.0, 4.0),
        };
        let mapped = FlipZCoordinateSpace::from_vrm_transform(transform);

        assert_eq!(FlipZCoordinateSpace::LABEL, "flip-z-left-handed-y-up");
        assert!(coordinate_space_mirrors::<FlipZCoordinateSpace>());
        assert_eq!(mapped.translation, Vec3::new(1.0, 2.0, -3.0));
        assert_eq!(mapped.scale, transform.scale);

        let mirror = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
        let source_matrix = transform_matrix(transform);
        let mapped_matrix = transform_matrix(mapped);
        let expected_matrix = mirror * source_matrix * mirror;
        assert_matrix_abs_diff_eq(mapped_matrix, expected_matrix, 0.0001);

        let roundtrip = FlipZCoordinateSpace::to_vrm_transform(mapped);
        assert!(
            roundtrip
                .translation
                .abs_diff_eq(transform.translation, 0.0001)
        );
        assert!(
            roundtrip.rotation.abs_diff_eq(transform.rotation, 0.0001)
                || roundtrip.rotation.abs_diff_eq(-transform.rotation, 0.0001)
        );
    }

    #[test]
    fn coordinate_space_affine_matrix_helpers_convert_basis_directly() {
        let transform = Transform {
            translation: Vec3::new(1.0, -2.0, 3.0),
            rotation: Quat::from_rotation_x(0.35)
                * Quat::from_rotation_y(-0.6)
                * Quat::from_rotation_z(0.2),
            scale: Vec3::new(1.25, 0.75, 2.0),
        };
        let matrix = transform_matrix(transform);
        let mirror = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));

        assert_matrix_abs_diff_eq(
            VrmCoordinateSpace::from_vrm_affine_matrix(matrix),
            matrix,
            0.0001,
        );
        assert_matrix_abs_diff_eq(
            VrmCoordinateSpace::to_vrm_affine_matrix(matrix),
            matrix,
            0.0001,
        );

        let mapped = FlipZCoordinateSpace::from_vrm_affine_matrix(matrix);
        assert_matrix_abs_diff_eq(mapped, mirror * matrix * mirror, 0.0001);

        let roundtrip = FlipZCoordinateSpace::to_vrm_affine_matrix(mapped);
        assert_matrix_abs_diff_eq(roundtrip, matrix, 0.0001);
    }

    #[test]
    #[allow(deprecated)]
    fn coordinate_space_matrix_aliases_remain_compatible() {
        let matrix = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        assert_matrix_abs_diff_eq(
            FlipZCoordinateSpace::from_vrm_matrix(matrix),
            FlipZCoordinateSpace::from_vrm_affine_matrix(matrix),
            0.0001,
        );
        assert_matrix_abs_diff_eq(
            FlipZCoordinateSpace::to_vrm_matrix(matrix),
            FlipZCoordinateSpace::to_vrm_affine_matrix(matrix),
            0.0001,
        );
    }

    #[test]
    fn scene_graph_trait_remains_object_safe() {
        fn accepts_dyn_scene_graph(_scene: &dyn SceneGraph<Error = HeadlessAdapterError>) {}

        let scene = HeadlessSceneState::default();
        accepts_dyn_scene_graph(&scene);
    }

    #[test]
    fn coordinate_space_target_converts_engine_boundary() {
        let vrm_transform = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_x(0.2) * Quat::from_rotation_y(0.4),
            scale: Vec3::new(1.0, 2.0, 1.0),
        };
        let mut scene = HeadlessSceneState::default();
        scene.insert_node(
            NodeRef(0),
            FlipZCoordinateSpace::from_vrm_transform(vrm_transform),
        );
        scene.insert_node(NodeRef(1), Transform::default());
        scene.set_parent(NodeRef(1), Some(NodeRef(0))).unwrap();
        scene.update_world_transforms().unwrap();

        let next_vrm_transform = Transform {
            translation: Vec3::new(-2.0, 4.0, 6.0),
            rotation: Quat::from_rotation_z(0.5) * Quat::from_rotation_y(-0.25),
            scale: Vec3::splat(0.5),
        };
        {
            let mut target = coordinate_space_target::<FlipZCoordinateSpace, _>(&mut scene);
            let read = target.local_transform(NodeRef(0)).unwrap();
            assert_transform_abs_diff_eq(read, vrm_transform, 0.0001);
            let matrix = target.world_matrix(NodeRef(0)).unwrap();
            assert_matrix_abs_diff_eq(matrix, transform_matrix(vrm_transform), 0.0001);
            let mut children = Vec::new();
            target
                .visit_children(NodeRef(0), |child| children.push(child))
                .unwrap();
            assert_eq!(children, vec![NodeRef(1)]);
            target
                .set_local_transform(NodeRef(0), next_vrm_transform)
                .unwrap();
        }

        let stored = scene.node(NodeRef(0)).unwrap().local;
        assert_transform_abs_diff_eq(
            stored,
            FlipZCoordinateSpace::from_vrm_transform(next_vrm_transform),
            0.0001,
        );
    }

    #[test]
    fn screen_projection_maps_triangle_into_y_down_pixels() {
        let projection = project_triangle_to_screen::<ZeroToOneDepth>(
            [[-1.0, -1.0, 0.25], [1.0, -1.0, 0.25], [0.0, 1.0, 0.25]],
            Mat4::IDENTITY,
            ScreenProjectionSize {
                width: 100.0,
                height: 200.0,
            },
            RendererFrontFace::Ccw,
        )
        .unwrap();

        assert_eq!(
            projection.screen,
            [[0.0, 200.0], [100.0, 200.0], [50.0, 0.0]]
        );
        assert_eq!(projection.bounds.min_x, 0.0);
        assert_eq!(projection.bounds.max_y, 200.0);
        assert_eq!(projection.ndc_depth, 0.25);
        assert_eq!(projection.webgl_depth, -0.5);
        assert!(projection.screen_signed_area < 0.0);
        assert!(!projection.front_facing);
        assert!(projection.gpu_front_facing);
    }

    #[test]
    fn vrm_screen_projection_accepts_coordinate_policy() {
        let projection = project_vrm_triangle_to_screen::<FlipZCoordinateSpace, ZeroToOneDepth>(
            [[0.0, 0.0, -0.25], [1.0, 0.0, -0.25], [0.0, 1.0, -0.25]],
            Mat4::IDENTITY,
            ScreenProjectionSize {
                width: 100.0,
                height: 100.0,
            },
            RendererFrontFace::Ccw,
        )
        .unwrap();

        assert_eq!(projection.ndc_depth, 0.25);
        assert_eq!(projection.webgl_depth, -0.5);
    }

    #[test]
    fn reverse_zero_to_one_depth_projection_maps_to_webgl_depth() {
        let projection =
            project_vrm_triangle_to_screen::<FlipZCoordinateSpace, ReverseZeroToOneDepth>(
                [[0.0, 0.0, -0.25], [1.0, 0.0, -0.25], [0.0, 1.0, -0.25]],
                Mat4::IDENTITY,
                ScreenProjectionSize {
                    width: 100.0,
                    height: 100.0,
                },
                RendererFrontFace::Ccw,
            )
            .unwrap();

        assert_eq!(projection.ndc_depth, 0.25);
        assert_eq!(projection.webgl_depth, 0.5);
    }

    fn transform_matrix(transform: Transform) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            transform.scale,
            transform.rotation,
            transform.translation,
        )
    }

    fn assert_transform_abs_diff_eq(actual: Transform, expected: Transform, tolerance: f32) {
        assert!(
            actual
                .translation
                .abs_diff_eq(expected.translation, tolerance),
            "translation mismatch: actual={:?} expected={:?}",
            actual.translation,
            expected.translation
        );
        assert!(
            actual.rotation.abs_diff_eq(expected.rotation, tolerance)
                || actual.rotation.abs_diff_eq(-expected.rotation, tolerance),
            "rotation mismatch: actual={:?} expected={:?}",
            actual.rotation,
            expected.rotation
        );
        assert!(
            actual.scale.abs_diff_eq(expected.scale, tolerance),
            "scale mismatch: actual={:?} expected={:?}",
            actual.scale,
            expected.scale
        );
    }

    fn assert_matrix_abs_diff_eq(actual: Mat4, expected: Mat4, tolerance: f32) {
        for (actual, expected) in actual
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!(
                (actual - expected).abs() <= tolerance,
                "matrix component mismatch: actual={actual} expected={expected}"
            );
        }
    }

    fn coordinate_space_mirrors<C>() -> bool
    where
        C: CoordinateSpaceMapping,
    {
        C::MIRRORS_HANDEDNESS
    }

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

    impl WorldMatrixAccess for Mock {
        type Error = Infallible;

        fn world_matrix(&self, node: NodeRef) -> Result<Mat4, Self::Error> {
            Ok(transform_matrix(self.world_transform(node)?))
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
        world_matrix_overrides: HashMap<NodeRef, Mat4>,
        constraint_rest: ConstraintRestMap,
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
                world_matrix_overrides: HashMap::new(),
                constraint_rest: ConstraintRestMap::default(),
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

        fn with_constraint_rest(mut self, rest: ConstraintRestMap) -> Self {
            self.constraint_rest = rest;
            self
        }

        fn local(&self, node: NodeRef) -> Transform {
            self.local_overrides
                .get(&node)
                .copied()
                .unwrap_or_else(|| self.node(node).local)
        }

        fn refresh_node_world(&mut self, node: NodeRef) {
            let local = self.local(node);
            let local_matrix = transform_matrix(local);
            let world_matrix = self
                .node(node)
                .parent
                .map(NodeRef)
                .and_then(|parent| self.world_matrix_overrides.get(&parent).copied())
                .map(|parent| parent * local_matrix)
                .unwrap_or(local_matrix);
            let (scale, rotation, translation) = world_matrix.to_scale_rotation_translation();
            let world = Transform {
                translation,
                rotation,
                scale,
            };
            self.world_overrides.insert(node, world);
            self.world_matrix_overrides.insert(node, world_matrix);
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

    impl WorldMatrixAccess for FixtureScene {
        type Error = Infallible;

        fn world_matrix(&self, node: NodeRef) -> Result<Mat4, Self::Error> {
            Ok(self
                .world_matrix_overrides
                .get(&node)
                .copied()
                .unwrap_or_else(|| self.node(node).world_matrix))
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

    impl ConstraintRestAccess for FixtureScene {
        type Error = Infallible;

        fn constraint_rest_state(
            &self,
            destination: NodeRef,
            source: NodeRef,
        ) -> Result<ConstraintRestState, Self::Error> {
            self.constraint_rest
                .constraint_rest_state(destination, source)
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
    fn vrma_humanoid_frame_scales_hips_translation_to_target_rest_height() {
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
            local_transforms: [(NodeRef(0), Transform::default())].into_iter().collect(),
            world_transforms: [(
                NodeRef(0),
                Transform {
                    translation: Vec3::Y,
                    ..Transform::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Mock::default()
        };
        let mut rig = HumanoidPoseRig::capture(&mock, &document).unwrap();
        let frame = VrmAnimationFrame {
            hips_translation: Some(Vec3::new(0.0, 2.5, 0.0)),
            source_rest_hips_position: Some(Vec3::new(0.0, 2.0, 0.0)),
            ..VrmAnimationFrame::default()
        };

        apply_vrma_humanoid_frame(&mut mock, &mut rig, &frame).unwrap();

        let hips = mock
            .local_sets
            .iter()
            .find(|(node, _)| *node == NodeRef(0))
            .expect("hips writeback");
        assert_eq!(hips.1.translation, Vec3::new(0.0, 1.25, 0.0));
    }

    #[test]
    fn vrma_animation_frame_applies_humanoid_expressions_and_look_at_together() {
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
                            node: NodeRef(2),
                            index: 0,
                            weight: 100.0,
                        }],
                        ..Expression::default()
                    },
                )]
                .into_iter()
                .collect(),
                custom: [(
                    "joy".to_owned(),
                    Expression {
                        binds: vec![ExpressionBind::MorphTarget {
                            node: NodeRef(3),
                            index: 1,
                            weight: 80.0,
                        }],
                        ..Expression::default()
                    },
                )]
                .into_iter()
                .collect(),
            }),
            ..VrmDocument::default()
        };
        let animation = VrmAnimation {
            duration: 1.0,
            rest_hips_position: Vec3::Y,
            humanoid_rotation_tracks: [(
                HumanBoneName::Head,
                RotationTrack {
                    times: vec![0.0, 1.0],
                    values: vec![Quat::IDENTITY, Quat::from_rotation_y(0.5)],
                },
            )]
            .into_iter()
            .collect(),
            hips_translation: Some(vrm_core::TranslationTrack {
                times: vec![0.0, 1.0],
                values: vec![Vec3::ZERO, Vec3::new(0.0, 0.4, 0.0)],
            }),
            preset_expression_tracks: [(
                ExpressionName::Blink,
                vrm_core::ScalarTrack {
                    times: vec![0.0, 1.0],
                    values: vec![0.0, 0.5],
                },
            )]
            .into_iter()
            .collect(),
            custom_expression_tracks: [(
                "joy".to_owned(),
                vrm_core::ScalarTrack {
                    times: vec![0.0, 1.0],
                    values: vec![0.25, 0.75],
                },
            )]
            .into_iter()
            .collect(),
            look_at_track: Some(RotationTrack {
                times: vec![0.0, 1.0],
                values: vec![Quat::IDENTITY, Quat::from_rotation_x(0.25)],
            }),
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
        let frame = sample_vrm_animation(&animation, 0.5);

        apply_vrma_animation_frame_with_look_at(&mut mock, &mut rig, &document, &frame).unwrap();

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
        assert!(
            hips.1
                .translation
                .abs_diff_eq(Vec3::new(0.0, 0.2, 0.0), 0.0001)
        );
        assert!(
            head.1
                .rotation
                .abs_diff_eq(Quat::from_rotation_y(0.25), 0.0001)
                || head
                    .1
                    .rotation
                    .abs_diff_eq(-Quat::from_rotation_y(0.25), 0.0001)
        );
        assert_eq!(
            mock.morphs,
            vec![(NodeRef(2), 0, 25.0), (NodeRef(3), 1, 40.0)]
        );
        assert_eq!(mock.look_at_rotations.len(), 1);
        assert!(
            mock.look_at_rotations[0].abs_diff_eq(Quat::from_rotation_x(0.125), 0.0001)
                || mock.look_at_rotations[0].abs_diff_eq(-Quat::from_rotation_x(0.125), 0.0001)
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
    fn mtoon_lighting_config_resolves_reference_and_tuned_accumulators() {
        let tuned = MtoonLightingConfig {
            accumulation: MtoonLightAccumulation::Tuned,
            exposure: 0.5,
            ambient_base: 0.25,
            ambient_gi_scale: 0.75,
            pbr_ambient: 0.125,
        };
        assert_eq!(
            tuned.effective_values().to_array(),
            [0.5, 0.25, 0.75, 0.125]
        );
        assert_eq!(tuned.accumulation.as_str(), "tuned");
        assert!(!tuned.accumulation.is_three_vrm());

        let reference = MtoonLightingConfig {
            accumulation: MtoonLightAccumulation::ThreeVrm,
            ..tuned
        };
        assert_eq!(
            reference.effective_values().to_array(),
            [1.0, 0.125, 0.0, 0.125]
        );
        assert_eq!(reference.accumulation.as_str(), "three-vrm");
        assert!(reference.accumulation.is_three_vrm());
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
                    shading_shift_texture_scale: 0.45,
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
        assert_eq!(descriptors[0].shading_shift_texture_scale, 0.45);
        assert_eq!(descriptors[0].light_color_attenuation_factor, 0.25);
        assert_eq!(descriptors[0].matcap_factor, [0.4, 0.3, 0.2]);
        assert_eq!(descriptors[0].parametric_rim_color_factor, [0.2, 0.3, 0.4]);
        assert_eq!(descriptors[0].rim_lighting_mix_factor, 0.5);
        assert_eq!(descriptors[0].parametric_rim_fresnel_power_factor, 2.0);
        assert_eq!(descriptors[0].parametric_rim_lift_factor, 0.1);
        assert_eq!(descriptors[0].outline_width_factor, 0.01);
        assert_eq!(descriptors[0].outline_lighting_mix_factor, 0.6);
    }

    #[test]
    fn mtoon_renderer_material_plans_expose_pipeline_state_and_texture_bindings() {
        let document = VrmDocument {
            materials: vec![vrm_core::Material {
                name: Some("mtoon".to_owned()),
                khr_emissive_strength: Feature::Present(EmissiveStrength(2.0)),
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    transparent_with_z_write: true,
                    render_queue_offset_number: 7,
                    outline_width_mode: OutlineWidthMode::ScreenCoordinates,
                    outline_width_factor: 0.02,
                    base_color_factor: [0.8, 0.7, 0.6, 0.5],
                    emissive_factor: [0.1, 0.2, 0.3],
                    textures: MtoonTextureSet {
                        main_texture: Some(TextureRef(1)),
                        normal_texture: Some(TextureRef(2)),
                        uv_animation_mask_texture: Some(TextureRef(3)),
                        ..MtoonTextureSet::default()
                    },
                    ..MtoonMaterial::default()
                }),
                ..vrm_core::Material::default()
            }],
            ..VrmDocument::default()
        };

        let plans = mtoon_renderer_material_plans(
            &document,
            MtoonMaterializationOptions {
                debug_mode: MtoonDebugMode::Normal,
                v0_compat_shade: true,
            },
        );

        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].pass, MtoonRendererPass::Base);
        assert_eq!(plans[0].pipeline.render_order, 3007);
        assert_eq!(plans[0].pipeline.phase_order, 7);
        assert_eq!(plans[0].pipeline.alpha_mode, MtoonAlphaMode::Blend);
        assert!(plans[0].pipeline.blend);
        assert!(plans[0].pipeline.depth_write);
        assert!(plans[0].pipeline.transparent_with_z_write);
        assert_eq!(plans[0].shader.emissive_color, [0.2, 0.4, 0.6]);
        assert_eq!(plans[0].shader.debug_mode, MtoonDebugMode::Normal);
        assert!(plans[0].shader.v0_compat_shade);
        assert_eq!(plans[0].textures.main, Some(TextureRef(1)));
        assert_eq!(
            plans[0].texture_bindings,
            vec![
                MtoonTextureBindingPlan {
                    slot: MtoonTextureSlot::Main,
                    texture: TextureRef(1),
                    sampler: MtoonSamplerHint::LinearRepeat,
                },
                MtoonTextureBindingPlan {
                    slot: MtoonTextureSlot::Normal,
                    texture: TextureRef(2),
                    sampler: MtoonSamplerHint::NormalMapLinearRepeat,
                },
                MtoonTextureBindingPlan {
                    slot: MtoonTextureSlot::UvAnimationMask,
                    texture: TextureRef(3),
                    sampler: MtoonSamplerHint::LinearRepeat,
                },
            ]
        );
        assert_eq!(plans[1].pass, MtoonRendererPass::Outline);
        assert_eq!(plans[1].pipeline.render_order, 3008);
        assert_eq!(
            plans[1].pipeline.outline_width_mode,
            Some(OutlineWidthMode::ScreenCoordinates)
        );
        assert_eq!(plans[1].shader.outline_width_factor, 0.02);

        let capture_plan = RendererMaterialPipelinePlan::from_mtoon_plan(&plans[0])
            .with_gltf_override(GltfMaterialPipelineOverride {
                alpha_mode: GltfMaterialAlphaMode::Blend,
                alpha_cutoff: None,
                double_sided: true,
            });
        assert_eq!(capture_plan.render_order, 3007);
        assert_eq!(capture_plan.phase_order, 7);
        assert_eq!(capture_plan.alpha_mode, RendererMaterialAlphaMode::Blend);
        assert_eq!(capture_plan.cull_mode, RendererMaterialCullMode::Off);
        assert!(capture_plan.depth_write);
    }

    #[test]
    fn renderer_material_pipeline_plan_combines_mtoon_and_gltf_override() {
        let document = VrmDocument {
            materials: vec![vrm_core::Material {
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    transparent_with_z_write: false,
                    render_queue_offset_number: 3,
                    cull_mode: MtoonCullMode::Back,
                    cutoff_factor: 0.25,
                    ..MtoonMaterial::default()
                }),
                ..vrm_core::Material::default()
            }],
            ..VrmDocument::default()
        };

        let plan = renderer_material_pipeline_plan(
            &document,
            Some(MaterialRef(0)),
            MtoonMaterializationOptions::default(),
            Some(GltfMaterialPipelineOverride {
                alpha_mode: GltfMaterialAlphaMode::Blend,
                alpha_cutoff: None,
                double_sided: true,
            }),
        );

        assert_eq!(plan.render_order, 3022);
        assert_eq!(plan.phase_order, 22);
        assert_eq!(plan.alpha_mode, RendererMaterialAlphaMode::Blend);
        assert_eq!(plan.cull_mode, RendererMaterialCullMode::Off);
        assert!(!plan.depth_write);
        assert!(plan.blend);
        assert_eq!(plan.alpha_cutoff, 0.25);
    }

    #[test]
    fn renderer_material_pipeline_plan_handles_gltf_only_materials() {
        let document = VrmDocument::default();

        let plan = renderer_material_pipeline_plan(
            &document,
            Some(MaterialRef(9)),
            MtoonMaterializationOptions::default(),
            Some(GltfMaterialPipelineOverride {
                alpha_mode: GltfMaterialAlphaMode::Mask,
                alpha_cutoff: Some(0.8),
                double_sided: false,
            }),
        );

        assert_eq!(plan.render_order, 2000);
        assert_eq!(plan.alpha_mode, RendererMaterialAlphaMode::Mask);
        assert_eq!(plan.cull_mode, RendererMaterialCullMode::Back);
        assert!(plan.depth_write);
        assert!(!plan.blend);
        assert_eq!(plan.alpha_cutoff, 0.8);
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
            parents: [(NodeRef(4), NodeRef(2)), (NodeRef(5), NodeRef(4))]
                .into_iter()
                .collect(),
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
            local_transforms: [
                (
                    NodeRef(2),
                    Transform {
                        rotation: Quat::IDENTITY,
                        ..Transform::default()
                    },
                ),
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

    #[test]
    #[ignore = "requires three-vrm node constraint golden JSON; set VRM_RS_THREE_VRM_CONSTRAINT_GOLDEN"]
    fn node_constraint_manager_matches_three_vrm_golden() {
        let golden_path = std::env::var_os("VRM_RS_THREE_VRM_CONSTRAINT_GOLDEN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                workspace_path(
                    ".external-fixtures/golden/VRM1_Constraint_Twist_Sample.constraint.json",
                )
            });
        let golden: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&golden_path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", golden_path.display())),
        )
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", golden_path.display()));
        compare_constraint_golden(&golden_path, &golden);
    }

    fn compare_constraint_golden(golden_path: &std::path::Path, golden: &serde_json::Value) {
        let fixture = golden["fixture"].as_str().unwrap_or_else(|| {
            panic!(
                "constraint golden fixture is missing in {}",
                golden_path.display()
            )
        });
        let loaded = vrm_io::load_vrm_from_path(fixture)
            .unwrap_or_else(|err| panic!("failed to load constraint fixture {fixture}: {err:?}"));
        let document = loaded.model().document();
        let rest = ConstraintRestMap::capture(
            &FixtureScene::new(loaded.scene().clone()),
            &document.node_constraints,
        )
        .unwrap();
        let mut scene = FixtureScene::new(loaded.scene().clone()).with_constraint_rest(rest);
        for input in golden["sourceInputs"]
            .as_array()
            .unwrap_or_else(|| panic!("constraint golden sourceInputs must be an array"))
        {
            let node = NodeRef(
                input["node"]
                    .as_u64()
                    .unwrap_or_else(|| panic!("constraint source input missing node: {input}"))
                    as usize,
            );
            scene
                .set_local_rotation(node, quat_from_json(&input["localRotation"]))
                .unwrap();
        }
        scene.update_world_transforms().unwrap();
        scene.rotations.clear();

        let order = vrm_runtime::ConstraintManager::new(document.node_constraints.clone())
            .update_order()
            .unwrap_or_else(|err| {
                panic!(
                    "failed to order constraints for {}: {err:?}",
                    golden_path.display()
                )
            });
        let actual_order = order
            .iter()
            .map(|constraint| constraint.destination.0 as u64)
            .collect::<Vec<_>>();
        let expected_order = golden["updateOrder"]
            .as_array()
            .unwrap_or_else(|| panic!("constraint golden updateOrder must be an array"))
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .expect("constraint updateOrder entries must be nodes")
            })
            .collect::<Vec<_>>();
        assert_eq!(actual_order, expected_order);

        for constraint in &order {
            apply_node_constraint(&mut scene, constraint).unwrap();
            scene.update_world_transforms().unwrap();
        }
        let actual = scene.rotations.iter().copied().collect::<HashMap<_, _>>();
        for expected in golden["constraints"]
            .as_array()
            .unwrap_or_else(|| panic!("constraint golden constraints must be an array"))
        {
            let destination =
                NodeRef(expected["destination"].as_u64().unwrap_or_else(|| {
                    panic!("constraint expected destination missing: {expected}")
                }) as usize);
            let expected_rotation = quat_from_json(&expected["localRotation"]);
            let actual_rotation = actual.get(&destination).copied().unwrap_or_else(|| {
                panic!("constraint destination {} was not written", destination.0)
            });
            assert!(
                quat_component_delta(actual_rotation, expected_rotation) <= 0.0001,
                "constraint destination {} mismatch: actual={actual_rotation:?} expected={expected_rotation:?}",
                destination.0
            );
        }
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
                tail: 0.0022,
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
        compare_humanoid_rest_golden(&golden_path, &golden);
    }

    #[test]
    #[ignore = "requires Alicia three-vrm golden JSON in .external-fixtures/golden"]
    fn vrm0_alicia_humanoid_pose_matches_three_vrm_golden_rest_state() {
        let golden_path =
            workspace_path(".external-fixtures/golden/AliciaSolid_vrm-0.51.spring.json");
        let golden = load_three_vrm_golden_from_path(&golden_path);
        compare_humanoid_rest_golden(&golden_path, &golden);
    }

    fn compare_humanoid_rest_golden(golden_path: &std::path::Path, golden: &serde_json::Value) {
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
        compare_humanoid_writeback_golden(&golden_path, &golden);
    }

    #[test]
    #[ignore = "requires Alicia three-vrm golden JSON in .external-fixtures/golden"]
    fn vrm0_alicia_humanoid_pose_writeback_matches_three_vrm_golden() {
        let golden_path =
            workspace_path(".external-fixtures/golden/AliciaSolid_vrm-0.51.spring.json");
        let golden = load_three_vrm_golden_from_path(&golden_path);
        compare_humanoid_writeback_golden(&golden_path, &golden);
    }

    fn compare_humanoid_writeback_golden(
        golden_path: &std::path::Path,
        golden: &serde_json::Value,
    ) {
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
        let raw_scenario = golden_pose_scenario(golden, "rawWriteback");
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

        let normalized_scenario = golden_pose_scenario(golden, "normalizedWriteback");
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
            checked >= 2,
            "expected at least baseline and dense VRMA goldens in {}",
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
        let expected_duration = golden["duration"].as_f64().unwrap_or_else(|| {
            panic!(
                "VRMA golden duration is missing in {}",
                golden_path.display()
            )
        }) as f32;
        assert!(
            (animation.duration - expected_duration).abs() <= 0.0005,
            "VRMA duration mismatch for {}: actual={} expected={expected_duration}",
            golden_path.display(),
            animation.duration
        );
        let tolerance = PoseTolerance {
            translation: 0.001,
            rotation_radians: 0.0015,
        };
        let samples = golden["samples"]
            .as_array()
            .unwrap_or_else(|| panic!("VRMA golden samples must be an array"));
        let times = golden["times"]
            .as_array()
            .unwrap_or_else(|| panic!("VRMA golden times must be an array"));
        assert_eq!(
            samples.len(),
            times.len(),
            "VRMA golden sample/time count mismatch in {}",
            golden_path.display()
        );

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
                let actual = scene.look_at_rotations.last().copied().unwrap_or_else(|| {
                    if frame.look_at.is_none() && expected.abs_diff_eq(Quat::IDENTITY, 0.000001) {
                        Quat::IDENTITY
                    } else {
                        panic!("VRMA sample at {time} did not write lookAt")
                    }
                });
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
            .unwrap_or_else(|| workspace_path(".external-fixtures/golden/Seed-san.spring.json"));
        let golden = load_three_vrm_golden_from_path(&golden_path);
        (golden_path, golden)
    }

    fn workspace_path(relative: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join(relative)
    }

    fn load_three_vrm_golden_from_path(golden_path: &std::path::Path) -> serde_json::Value {
        serde_json::from_slice(
            &std::fs::read(golden_path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", golden_path.display())),
        )
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", golden_path.display()))
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
    fn headless_scene_state_drives_runtime_without_engine_framework() {
        let document = VrmDocument {
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
            materials: vec![Material {
                khr_emissive_strength: Feature::Present(EmissiveStrength(2.0)),
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    transparent_with_z_write: true,
                    emissive_factor: [0.25, 0.5, 0.75],
                    ..MtoonMaterial::default()
                }),
                ..Material::default()
            }],
            ..VrmDocument::default()
        };

        let mut scene = HeadlessSceneState::default();
        scene.insert_node(NodeRef(0), Transform::default());
        scene.insert_node(
            NodeRef(1),
            Transform {
                translation: Vec3::Y,
                ..Transform::default()
            },
        );
        scene.insert_node(NodeRef(2), Transform::default());
        scene.insert_node(
            NodeRef(4),
            Transform {
                rotation: Quat::IDENTITY,
                ..Transform::default()
            },
        );
        scene.insert_node(NodeRef(8), Transform::default());
        scene.set_parent(NodeRef(1), Some(NodeRef(0))).unwrap();
        scene.update_world_transforms().unwrap();
        assert_eq!(
            scene.world_transform(NodeRef(1)).unwrap().translation,
            Vec3::Y
        );

        scene
            .capture_constraint_rest_state(NodeRef(2), NodeRef(4))
            .unwrap();
        let source_rotation = Quat::from_rotation_y(0.5);
        scene
            .set_local_rotation(NodeRef(4), source_rotation)
            .unwrap();

        let events = RuntimeEvents {
            delta: DeltaTime(0.0),
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
        let mut driver = VrmRuntimeDriver::new(&document).with_runtime_events(&events);
        driver.tick(&mut scene, None).unwrap();

        assert_eq!(scene.morph_weight(NodeRef(8), 0), Some(50.0));
        assert!(
            scene
                .local_transform(NodeRef(2))
                .unwrap()
                .rotation
                .abs_diff_eq(source_rotation, 0.0001)
        );
        assert!(!scene.node(NodeRef(8)).unwrap().visible);
        assert_eq!(scene.emissive_intensity(MaterialRef(0)), Some(2.0));
        assert!(
            !scene
                .mtoon_pipeline_passes(MaterialRef(0))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn runtime_pipeline_keeps_visual_passes_observable_between_fixed_steps() {
        let document = VrmDocument {
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
            materials: vec![Material {
                khr_emissive_strength: Feature::Present(EmissiveStrength(2.0)),
                mtoon: Feature::Present(MtoonMaterial {
                    render_queue: MtoonRenderQueue::Transparent,
                    ..MtoonMaterial::default()
                }),
                ..Material::default()
            }],
            ..VrmDocument::default()
        };
        let frame = VrmAnimationFrame {
            look_at: Some(Quat::from_rotation_x(0.25)),
            ..VrmAnimationFrame::default()
        };
        let mut pipeline = VrmRuntimePipeline::with_options(
            &document,
            RuntimePipelineOptions {
                fixed_delta: DeltaTime(1.0),
                max_substeps: 2,
                view_mode: ViewMode::ThirdPerson,
                apply_vrm0_orientation: true,
            },
        );
        pipeline
            .runtime_mut()
            .expression_manager
            .set_value("blink", 0.25);
        let mut mock = Mock::default();

        let report = pipeline
            .tick(&mut mock, DeltaTime(0.5), Some(&frame))
            .unwrap();

        assert_eq!(report.substeps, 0);
        assert_eq!(report.consumed_delta, DeltaTime(0.0));
        assert_eq!(report.accumulator, DeltaTime(0.5));
        assert_eq!(report.stage_count(RuntimePipelineStage::RuntimeUpdate), 1);
        assert_eq!(report.stage_count(RuntimePipelineStage::LookAt), 1);
        assert_eq!(report.stage_count(RuntimePipelineStage::Expressions), 1);
        assert_eq!(
            report.stage_count(RuntimePipelineStage::FirstPersonVisibility),
            1
        );
        assert_eq!(report.stage_count(RuntimePipelineStage::MtoonPipeline), 1);
        assert_eq!(mock.look_at_rotations, vec![Quat::from_rotation_x(0.25)]);
        assert_eq!(mock.morphs, vec![(NodeRef(8), 0, 25.0)]);
        assert_eq!(mock.visibility, vec![(NodeRef(8), false)]);
        assert_eq!(mock.emissive_intensities, vec![(MaterialRef(0), 2.0)]);

        let report = pipeline
            .tick(&mut mock, DeltaTime(2.5), Some(&frame))
            .unwrap();

        assert_eq!(report.substeps, 2);
        assert_eq!(report.consumed_delta, DeltaTime(2.0));
        assert_eq!(report.dropped_substeps, 1);
        assert_eq!(report.accumulator, DeltaTime(0.0));
        assert_eq!(report.stage_count(RuntimePipelineStage::RuntimeUpdate), 2);
        assert_eq!(report.stage_count(RuntimePipelineStage::LookAt), 2);
        assert_eq!(mock.look_at_rotations.len(), 3);
    }

    #[test]
    fn runtime_pipeline_captures_and_steps_spring_parity_state() {
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
        let mut pipeline = VrmRuntimePipeline::with_options(
            &document,
            RuntimePipelineOptions {
                fixed_delta: DeltaTime(1.0),
                max_substeps: 4,
                view_mode: ViewMode::ThirdPerson,
                apply_vrm0_orientation: true,
            },
        );
        pipeline.capture_spring_rest(&mock).unwrap();

        let report = pipeline.tick(&mut mock, DeltaTime(1.0), None).unwrap();

        assert_eq!(report.substeps, 1);
        assert_eq!(report.stage_count(RuntimePipelineStage::SpringBone), 1);
        assert!(pipeline.spring_rest().is_some());
        assert!(pipeline.spring_state().is_some());
        assert!(mock.world_updates >= 1);
        assert!(mock.rotations.iter().any(|(node, _)| *node == NodeRef(2)));
    }

    #[test]
    fn runtime_pipeline_tick_mixer_applies_sampled_vrma_frame() {
        let document = VrmDocument {
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
            ..VrmDocument::default()
        };
        let mut animation = VrmAnimation {
            duration: 1.0,
            hips_translation: Some(TranslationTrack {
                times: vec![0.0, 1.0],
                values: vec![Vec3::ZERO, Vec3::Y],
            }),
            look_at_track: Some(RotationTrack {
                times: vec![0.0, 1.0],
                values: vec![Quat::IDENTITY, Quat::from_rotation_x(0.5)],
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
        let mut pipeline = VrmRuntimePipeline::with_options(
            &document,
            RuntimePipelineOptions {
                fixed_delta: DeltaTime(1.0),
                ..RuntimePipelineOptions::default()
            },
        );
        let clip = pipeline.animation_mixer_mut().add_clip(animation);
        pipeline
            .animation_mixer_mut()
            .play(clip, vrm_runtime::AnimationActionOptions::default())
            .unwrap();
        let mut mock = Mock::default();

        let report = pipeline.tick_mixer(&mut mock, DeltaTime(0.5)).unwrap();

        assert_eq!(report.runtime.substeps, 0);
        assert_eq!(
            report.mixer.frame.hips_translation,
            Some(Vec3::new(0.0, 0.5, 0.0))
        );
        assert!(mock.local_sets.iter().any(|(node, transform)| {
            *node == NodeRef(1)
                && transform
                    .translation
                    .abs_diff_eq(Vec3::new(0.0, 0.5, 0.0), 0.0001)
        }));
        assert_eq!(mock.morphs, vec![(NodeRef(8), 0, 50.0)]);
        assert_eq!(mock.look_at_rotations.len(), 1);
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
