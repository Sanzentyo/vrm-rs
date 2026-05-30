//! Headless Bevy render capture for render-parity experiments.
//!
//! This example renders real `vrm-io` mesh primitives through Bevy's renderer
//! into an offscreen image and writes the shared RGBA JSON artifact consumed by
//! `tools/render-parity/compare-psnr.mjs`.

#[path = "common/render_capture_scene.rs"]
mod render_capture_scene;

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::{core_3d::Transparent3d, tonemapping::Tonemapping};
use bevy::ecs::system::SystemParam;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Vec4 as BVec4;
use bevy::mesh::Indices;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::Extract;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderSystems;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_graph::{
    self, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel,
};
use bevy::render::render_phase::ViewSortedRenderPhases;
use bevy::render::render_resource::{
    AsBindGroup, Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, Face,
    MapMode, PollType, PrimitiveTopology, RenderPipelineDescriptor, ShaderType,
    SpecializedMeshPipelineError, TexelCopyBufferInfo, TexelCopyBufferLayout, TextureDataOrder,
    TextureFormat, TextureUsages,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::shader::ShaderRef;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use clap::{Parser, ValueEnum};
use crossbeam_channel::{Receiver, Sender};
use glam::{Mat4, Vec3 as GVec3, Vec4 as GVec4};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use vrm_core::{
    ExpressionBind, ExpressionName, Feature, MtoonAlphaMode, MtoonCullMode, OutlineWidthMode,
    TextureTransform2d, VrmKind,
};
use vrm_io::{
    GltfAlphaMode, GltfMeshData, GltfNodeRest, GltfPrimitiveData, ImageData, ImageFormat,
    LoadedVrm, load_vrm_from_path,
};

const MTOON_SHADER_ASSET_PATH: &str = "shaders/vrm_mtoon_capture.wgsl";

fn main() -> Result<(), Box<dyn Error>> {
    let options = CaptureOptions::parse();
    let loaded = load_vrm_from_path(&options.fixture)?;
    let (tx, rx) = crossbeam_channel::bounded(1);

    App::new()
        .insert_resource(options.clone())
        .insert_resource(LoadedResource(loaded))
        .insert_resource(CaptureSender(tx))
        .insert_resource(SceneController::new(options.width, options.height, 40))
        .insert_resource(ClearColor(options.background.color()))
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins((ImageCopyPlugin, CaptureFramePlugin))
        .add_plugins(ExtractComponentPlugin::<BevyMtoonPhaseOrder>::default())
        .add_plugins(MaterialPlugin::<BevyMtoonMaterial>::default())
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .add_systems(Startup, setup)
        .run();

    rx.recv()?.map_err(|message| message.into())
}

#[derive(Clone, Debug, Parser, Resource)]
struct CaptureOptions {
    #[arg(long)]
    fixture: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    png_out: Option<PathBuf>,
    #[arg(long, default_value_t = 512)]
    width: u32,
    #[arg(long, default_value_t = 512)]
    height: u32,
    #[arg(long, default_value_t = 1.0)]
    camera_y: f32,
    #[arg(long, default_value_t = 5.0)]
    camera_z: f32,
    #[arg(long, default_value_t = 1.0)]
    target_y: f32,
    #[arg(long, default_value_t = 0.78)]
    mtoon_exposure: f32,
    #[arg(long, default_value_t = 0.12)]
    mtoon_ambient_base: f32,
    #[arg(long, default_value_t = 0.20)]
    mtoon_ambient_gi_scale: f32,
    #[arg(long, default_value_t = 0.03183099)]
    pbr_ambient: f32,
    #[arg(long, default_value_t = 1.0)]
    direct_light_scale: f32,
    #[arg(long, default_value_t = 1.0)]
    directional_r: f32,
    #[arg(long, default_value_t = 1.0)]
    directional_g: f32,
    #[arg(long, default_value_t = 1.0)]
    directional_b: f32,
    #[arg(long, value_enum, default_value_t = MtoonLightAccumulation::Tuned)]
    mtoon_light_accumulation: MtoonLightAccumulation,
    #[arg(long, default_value_t = 0.0)]
    mtoon_time: f32,
    #[arg(long, value_enum, default_value_t = CaptureBackground::OpaqueBlack)]
    background: CaptureBackground,
    #[arg(long)]
    disable_outlines: bool,
    #[arg(long = "expression")]
    expressions: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum MtoonLightAccumulation {
    Tuned,
    ThreeVrm,
}

impl MtoonLightAccumulation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tuned => "tuned",
            Self::ThreeVrm => "three-vrm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum CaptureBackground {
    OpaqueBlack,
    Transparent,
}

impl CaptureBackground {
    fn color(self) -> Color {
        match self {
            Self::OpaqueBlack => Color::BLACK,
            Self::Transparent => Color::NONE,
        }
    }
}

#[derive(Resource)]
struct LoadedResource(LoadedVrm);

#[derive(Resource)]
struct CaptureSender(Sender<Result<(), String>>);

#[derive(Debug, Resource)]
struct SceneController {
    state: SceneState,
    width: u32,
    height: u32,
}

struct CpuRgbaImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default)]
struct MaterialUvTransforms {
    base: Option<TextureTransform2d>,
    shade: Option<TextureTransform2d>,
    shading_shift: Option<TextureTransform2d>,
    normal: Option<TextureTransform2d>,
    matcap: Option<TextureTransform2d>,
    rim: Option<TextureTransform2d>,
    outline_width: Option<TextureTransform2d>,
    emissive: Option<TextureTransform2d>,
    occlusion: Option<TextureTransform2d>,
    uv_animation_mask: Option<TextureTransform2d>,
    uv_animation_scroll: [f32; 2],
    uv_animation_rotation: f32,
}

impl SceneController {
    fn new(width: u32, height: u32, pre_roll_frames: u32) -> Self {
        Self {
            state: SceneState::Render(pre_roll_frames),
            width,
            height,
        }
    }
}

#[derive(Debug)]
enum SceneState {
    Render(u32),
}

fn setup(
    mut commands: Commands,
    loaded: Res<LoadedResource>,
    options: Res<CaptureOptions>,
    mut assets: CaptureAssets<'_>,
    mut control: SetupControl<'_>,
) {
    let render_target = setup_render_target(
        &mut commands,
        &mut assets.images,
        &control.render_device,
        &mut control.scene_controller,
    );

    if let Err(error) = spawn_vrm_meshes(
        &mut commands,
        &loaded.0,
        &options,
        &mut assets.meshes,
        &mut assets.mtoon_materials,
        &mut assets.images,
    ) {
        let _ = control.sender.0.send(Err(error.to_string()));
        control.app_exit_writer.write(AppExit::Success);
        return;
    }

    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            ..default()
        },
        Transform::from_xyz(1.0, 1.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        render_target,
        Camera {
            clear_color: ClearColorConfig::Custom(options.background.color()),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 30.0_f32.to_radians(),
            aspect_ratio: options.width as f32 / options.height as f32,
            near: 0.1,
            far: 20.0,
            ..default()
        }),
        Tonemapping::None,
        Transform::from_xyz(0.0, options.camera_y, -options.camera_z)
            .looking_at(Vec3::new(0.0, options.target_y, 0.0), Vec3::Y),
    ));
}

#[derive(SystemParam)]
struct CaptureAssets<'w> {
    meshes: ResMut<'w, Assets<Mesh>>,
    mtoon_materials: ResMut<'w, Assets<BevyMtoonMaterial>>,
    images: ResMut<'w, Assets<Image>>,
}

#[derive(SystemParam)]
struct SetupControl<'w> {
    render_device: Res<'w, RenderDevice>,
    scene_controller: ResMut<'w, SceneController>,
    sender: Res<'w, CaptureSender>,
    app_exit_writer: MessageWriter<'w, AppExit>,
}

fn spawn_vrm_meshes(
    commands: &mut Commands,
    loaded: &LoadedVrm,
    options: &CaptureOptions,
    meshes: &mut Assets<Mesh>,
    mtoon_materials: &mut Assets<BevyMtoonMaterial>,
    images: &mut Assets<Image>,
) -> Result<(), Box<dyn Error>> {
    let image_handles = BevyImageHandles {
        color_images: loaded
            .images
            .iter()
            .map(|image| bevy_image(image).map(|image| images.add(image)))
            .collect::<Vec<_>>(),
        linear_images: loaded
            .images
            .iter()
            .map(|image| {
                bevy_image_with_format(image, TextureFormat::Rgba8Unorm)
                    .map(|image| images.add(image))
            })
            .collect::<Vec<_>>(),
        white: images.add(single_pixel_image(
            [255, 255, 255, 255],
            TextureFormat::Rgba8UnormSrgb,
        )),
        black: images.add(single_pixel_image(
            [0, 0, 0, 255],
            TextureFormat::Rgba8UnormSrgb,
        )),
        neutral_normal: images.add(single_pixel_image(
            [128, 128, 255, 255],
            TextureFormat::Rgba8Unorm,
        )),
    };
    let orientation = Mat4::from_rotation_y(std::f32::consts::PI);
    let mut primitives = Vec::new();
    let world_matrices = render_capture_scene::runtime_world_matrices(loaded)?;
    let expression_effects = expression_render_effects(loaded, &options.expressions)?;

    for (node_index, node) in loaded.scene.nodes.iter().enumerate() {
        let Some(mesh_index) = node.mesh else {
            continue;
        };
        let Some(mesh) = loaded.meshes.get(mesh_index) else {
            continue;
        };
        let node_world = world_matrices
            .get(node_index)
            .copied()
            .unwrap_or(node.world_matrix);
        let world = orientation * node_world;
        let skin_matrices = node
            .skin
            .and_then(|skin| loaded.skins.get(skin))
            .map(|skin| skin_matrices(loaded, skin, &world_matrices, orientation));
        let morph_weights = active_morph_weights(node_index, node, mesh, &expression_effects);
        let primitive_context = BevyPrimitiveContext {
            expression_effects: &expression_effects,
            world,
            skin_matrices: skin_matrices.as_deref(),
            options,
            image_handles: &image_handles,
        };
        for primitive in &mesh.primitives {
            let shading = material_shading(loaded, primitive.material, &expression_effects);
            let render_order = material_render_order(loaded, primitive.material);
            let (mesh, has_tangents) = bevy_mesh(
                primitive,
                &morph_weights,
                world,
                skin_matrices.as_deref(),
                shading.normal_scale > 0.0,
            );
            let surface = BevyPrimitive {
                mesh,
                material: BevyPrimitiveMaterial::Mtoon(bevy_mtoon_material(
                    loaded,
                    primitive,
                    shading,
                    &primitive_context,
                    render_depth_bias(render_order),
                    if has_tangents {
                        shading.normal_scale
                    } else {
                        0.0
                    },
                )),
                render_order,
                phase_order: material_phase_order(loaded, primitive.material),
            };
            primitives.push(surface);
            if !options.disable_outlines
                && let Some(outline) =
                    bevy_outline_primitive(loaded, primitive, &morph_weights, &primitive_context)
            {
                primitives.push(outline);
            }
        }
    }
    primitives.sort_by_key(|primitive| primitive.render_order);

    for primitive in primitives {
        let mesh = meshes.add(primitive.mesh);
        match primitive.material {
            BevyPrimitiveMaterial::Mtoon(material) => {
                commands.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(mtoon_materials.add(material)),
                    BevyMtoonPhaseOrder(primitive.phase_order),
                    Transform::IDENTITY,
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct BevyPrimitiveContext<'a> {
    expression_effects: &'a ExpressionRenderEffects,
    world: Mat4,
    skin_matrices: Option<&'a [Mat4]>,
    options: &'a CaptureOptions,
    image_handles: &'a BevyImageHandles,
}

fn bevy_outline_primitive(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    morph_weights: &[f32],
    context: &BevyPrimitiveContext<'_>,
) -> Option<BevyPrimitive> {
    let material = primitive
        .material
        .and_then(|index| loaded.model().document().materials.get(index))?;
    let mtoon = material.mtoon.as_ref()?;
    if !mtoon.outline_enabled() {
        return None;
    }
    let outline_color = apply_color_effect4(
        [
            mtoon.outline_color_factor[0],
            mtoon.outline_color_factor[1],
            mtoon.outline_color_factor[2],
            mtoon.outline_lighting_mix_factor,
        ],
        primitive.material,
        "outlineColor",
        context.expression_effects,
    );
    let width_texture = material_outline_width_image(loaded, primitive.material);
    let uv_transforms = material_uv_transforms(
        loaded,
        primitive.material,
        context.options.mtoon_time,
        context.expression_effects,
    );
    let mesh = bevy_outline_mesh(
        primitive,
        morph_weights,
        context.world,
        context.skin_matrices,
        BevyOutlineMeshSettings {
            width: mtoon.outline_width_factor,
            width_mode: mtoon.outline_width_mode,
            capture: context.options,
            width_texture: width_texture.as_ref(),
            width_transform: uv_transforms.outline_width,
        },
    );
    let mut material = bevy_mtoon_material(
        loaded,
        primitive,
        material_shading(loaded, primitive.material, context.expression_effects),
        context,
        render_depth_bias(material_render_order(loaded, primitive.material) + 1),
        0.0,
    );
    material.outline_color = BVec4::from_array(outline_color);
    material.alpha_mode = AlphaMode::Opaque;
    material.cull_mode = Some(Face::Front);
    material.pipeline.w = 0.0;
    Some(BevyPrimitive {
        mesh,
        material: BevyPrimitiveMaterial::Mtoon(material),
        render_order: material_render_order(loaded, primitive.material).saturating_add(1),
        phase_order: material_phase_order(loaded, primitive.material).saturating_add(1),
    })
}

fn bevy_outline_mesh(
    primitive: &GltfPrimitiveData,
    morph_weights: &[f32],
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    settings: BevyOutlineMeshSettings<'_>,
) -> Mesh {
    let outline_scale = OutlineScale::new(settings.width_mode, settings.capture);
    let positions = primitive
        .positions
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let (local_position, local_normal, _) = morphed_vertex(primitive, index, morph_weights)
                .unwrap_or((GVec3::ZERO, GVec3::Z, GVec4::new(1.0, 0.0, 0.0, 1.0)));
            let (position, normal) = transform_vertex(
                local_position,
                local_normal,
                world,
                skin_matrices,
                primitive.joints_0.get(index).copied(),
                primitive.weights_0.get(index).copied(),
            );
            let width = settings.width
                * settings
                    .width_texture
                    .map(|image| {
                        image.sample_green(transform_uv(
                            primitive_tex_coord(primitive, index),
                            settings.width_transform,
                        ))
                    })
                    .unwrap_or(1.0);
            outline_position(
                primitive,
                index,
                morph_weights,
                width,
                outline_scale,
                world,
                skin_matrices,
            )
            .unwrap_or(position + normal * width * outline_scale.at(position))
            .to_array()
        })
        .collect::<Vec<_>>();
    let normals = (0..primitive.positions.len())
        .map(|index| {
            let (local_position, local_normal, _) = morphed_vertex(primitive, index, morph_weights)
                .unwrap_or((GVec3::ZERO, GVec3::Z, GVec4::new(1.0, 0.0, 0.0, 1.0)));
            let (_, normal) = transform_vertex(
                local_position,
                local_normal,
                world,
                skin_matrices,
                primitive.joints_0.get(index).copied(),
                primitive.weights_0.get(index).copied(),
            );
            normal.to_array()
        })
        .collect::<Vec<_>>();
    let tangents =
        (primitive.tangents.len() == primitive.positions.len()).then(|| {
            primitive
                .tangents
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    let (_, _, tangent) = morphed_vertex(primitive, index, morph_weights)
                        .unwrap_or((GVec3::ZERO, GVec3::Z, GVec4::new(1.0, 0.0, 0.0, 1.0)));
                    let direction = transform_direction(
                        tangent.truncate(),
                        world,
                        skin_matrices,
                        primitive.joints_0.get(index).copied(),
                        primitive.weights_0.get(index).copied(),
                    );
                    [direction.x, direction.y, direction.z, tangent.w]
                })
                .collect::<Vec<_>>()
        });
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    if let Some(tangents) = tangents {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    }
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        tex_coords_or_default(primitive.positions.len(), &primitive.tex_coords_0),
    );
    mesh.insert_indices(Indices::U32(primitive.indices.clone()));
    mesh
}

#[derive(Clone, Copy)]
struct BevyOutlineMeshSettings<'a> {
    width: f32,
    width_mode: OutlineWidthMode,
    capture: &'a CaptureOptions,
    width_texture: Option<&'a CpuRgbaImage>,
    width_transform: Option<TextureTransform2d>,
}

#[derive(Clone, Copy, Debug)]
struct OutlineScale {
    mode: OutlineWidthMode,
    view: Mat4,
    projection_y: f32,
}

impl OutlineScale {
    fn new(mode: OutlineWidthMode, options: &CaptureOptions) -> Self {
        Self {
            mode,
            view: camera_view(options),
            projection_y: projection_y_scale(),
        }
    }

    fn at(self, world_position: GVec3) -> f32 {
        match self.mode {
            OutlineWidthMode::ScreenCoordinates => {
                let view_z = self.view.transform_point3(world_position).z;
                (-view_z / self.projection_y).max(0.0)
            }
            OutlineWidthMode::None
            | OutlineWidthMode::WorldCoordinates
            | OutlineWidthMode::Unknown => 1.0,
        }
    }
}

fn outline_position(
    primitive: &GltfPrimitiveData,
    index: usize,
    morph_weights: &[f32],
    width: f32,
    outline_scale: OutlineScale,
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
) -> Option<GVec3> {
    let (position, normal, _) = morphed_vertex(primitive, index, morph_weights)?;
    let normal = normal.normalize_or_zero();
    let transform = blended_vertex_transform(
        world,
        skin_matrices,
        primitive.joints_0.get(index).copied(),
        primitive.weights_0.get(index).copied(),
    );
    let world_position = transform.transform_point3(position);
    let normal_scale = normal_matrix_length(transform, normal);
    let offset_scale = width * normal_scale * outline_scale.at(world_position);
    if uses_weighted_skinning(
        skin_matrices,
        primitive.joints_0.get(index).copied(),
        primitive.weights_0.get(index).copied(),
    ) {
        let skinned_normal = transform.transform_vector3(normal).normalize_or_zero();
        if skinned_normal.length_squared() > f32::EPSILON {
            return Some(world_position + skinned_normal * offset_scale);
        }
    }

    let offset = normal * offset_scale;
    Some(transform.transform_point3(position + offset))
}

fn uses_weighted_skinning(
    skin_matrices: Option<&[Mat4]>,
    joints: Option<[u16; 4]>,
    weights: Option<[f32; 4]>,
) -> bool {
    let (Some(skin_matrices), Some(joints), Some(weights)) = (skin_matrices, joints, weights)
    else {
        return false;
    };

    joints
        .into_iter()
        .zip(weights)
        .any(|(joint, weight)| weight > 0.0 && skin_matrices.get(usize::from(joint)).is_some())
}

fn blended_vertex_transform(
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    joints: Option<[u16; 4]>,
    weights: Option<[f32; 4]>,
) -> Mat4 {
    let (Some(skin_matrices), Some(joints), Some(weights)) = (skin_matrices, joints, weights)
    else {
        return world;
    };

    let mut transform = Mat4::ZERO;
    let mut total_weight = 0.0;
    for (joint, weight) in joints.into_iter().zip(weights) {
        if weight <= 0.0 {
            continue;
        }
        let Some(matrix) = skin_matrices.get(usize::from(joint)) else {
            continue;
        };
        transform += *matrix * weight;
        total_weight += weight;
    }
    if total_weight > 0.0 { transform } else { world }
}

fn normal_matrix_length(transform: Mat4, normal: GVec3) -> f32 {
    if normal.length_squared() <= f32::EPSILON || transform.determinant().abs() <= 0.000001 {
        return 1.0;
    }
    let length = transform
        .inverse()
        .transpose()
        .transform_vector3(normal)
        .length();
    if length.is_finite() && length > 0.0 {
        length
    } else {
        1.0
    }
}

fn primitive_tex_coord(primitive: &GltfPrimitiveData, index: usize) -> [f32; 2] {
    primitive
        .tex_coords_0
        .get(index)
        .copied()
        .unwrap_or([0.0, 0.0])
}

struct BevyPrimitive {
    mesh: Mesh,
    material: BevyPrimitiveMaterial,
    render_order: i32,
    phase_order: i32,
}

enum BevyPrimitiveMaterial {
    Mtoon(BevyMtoonMaterial),
}

#[derive(Clone, Copy, Component, Debug, ExtractComponent)]
struct BevyMtoonPhaseOrder(i32);

struct BevyImageHandles {
    color_images: Vec<Option<Handle<Image>>>,
    linear_images: Vec<Option<Handle<Image>>>,
    white: Handle<Image>,
    black: Handle<Image>,
    neutral_normal: Handle<Image>,
}

#[derive(Clone, Debug, Default)]
struct ExpressionRenderEffects {
    cleared: HashMap<usize, HashSet<usize>>,
    weights: HashMap<(usize, usize), f32>,
    material_colors: Vec<MaterialColorEffect>,
    texture_transforms: Vec<TextureTransformEffect>,
}

#[derive(Clone, Debug)]
struct MaterialColorEffect {
    material: usize,
    kind: String,
    target_value: Vec<f32>,
    weight: f32,
}

#[derive(Clone, Debug)]
struct TextureTransformEffect {
    material: usize,
    scale: Option<[f32; 2]>,
    offset: Option<[f32; 2]>,
    weight: f32,
}

fn active_morph_weights(
    node_index: usize,
    node: &GltfNodeRest,
    mesh: &GltfMeshData,
    expressions: &ExpressionRenderEffects,
) -> Vec<f32> {
    let mut weights = if node.weights.is_empty() {
        mesh.weights.clone()
    } else {
        node.weights.clone()
    };

    if let Some(cleared) = expressions.cleared.get(&node_index) {
        for index in cleared {
            if weights.len() <= *index {
                weights.resize(index + 1, 0.0);
            }
            weights[*index] = 0.0;
        }
    }
    for ((node, index), weight) in &expressions.weights {
        if *node != node_index {
            continue;
        }
        if weights.len() <= *index {
            weights.resize(index + 1, 0.0);
        }
        weights[*index] += *weight;
    }
    weights
}

fn expression_render_effects(
    loaded: &LoadedVrm,
    expression_args: &[String],
) -> Result<ExpressionRenderEffects, Box<dyn Error>> {
    let mut result = ExpressionRenderEffects::default();
    let Feature::Present(expressions) = &loaded.model().document().expressions else {
        if expression_args.is_empty() {
            return Ok(result);
        }
        return Err("render expression was requested, but the VRM has no expressions".into());
    };

    for expression in expressions
        .preset
        .values()
        .chain(expressions.custom.values())
    {
        for bind in &expression.binds {
            match bind {
                ExpressionBind::MorphTarget { node, index, .. } => {
                    result.cleared.entry(node.0).or_default().insert(*index);
                }
                ExpressionBind::MaterialColor { .. } | ExpressionBind::TextureTransform { .. } => {}
            }
        }
    }

    for (name, weight) in parse_expression_args(expression_args)? {
        let (expression, binary) = if let Some(expression) =
            expressions.preset.get(&ExpressionName::from(name.as_str()))
        {
            (expression, expression.is_binary)
        } else if let Some(expression) = expressions.custom.get(&name) {
            (expression, expression.is_binary)
        } else {
            return Err(format!("unknown render expression: {name}").into());
        };
        let effective_weight = if binary {
            if weight >= 1.0 { 1.0 } else { 0.0 }
        } else {
            weight.clamp(0.0, 1.0)
        };
        for bind in &expression.binds {
            match bind {
                ExpressionBind::MorphTarget {
                    node,
                    index,
                    weight,
                } => {
                    *result.weights.entry((node.0, *index)).or_default() +=
                        effective_weight * *weight;
                }
                ExpressionBind::MaterialColor {
                    material,
                    kind,
                    target_value,
                } => {
                    result.material_colors.push(MaterialColorEffect {
                        material: material.0,
                        kind: kind.clone(),
                        target_value: target_value.clone(),
                        weight: effective_weight,
                    });
                }
                ExpressionBind::TextureTransform {
                    material,
                    scale,
                    offset,
                } => {
                    result.texture_transforms.push(TextureTransformEffect {
                        material: material.0,
                        scale: *scale,
                        offset: *offset,
                        weight: effective_weight,
                    });
                }
            }
        }
    }
    Ok(result)
}

fn parse_expression_args(args: &[String]) -> Result<Vec<(String, f32)>, Box<dyn Error>> {
    args.iter()
        .map(|arg| {
            let Some((name, value)) = arg.split_once('=') else {
                return Err(format!("invalid expression '{arg}', expected name=weight").into());
            };
            let weight = value
                .parse::<f32>()
                .map_err(|err| format!("invalid expression weight in '{arg}': {err}"))?;
            Ok((name.to_owned(), weight))
        })
        .collect()
}

fn apply_color_effect4(
    initial: [f32; 4],
    material: Option<usize>,
    kind: &str,
    effects: &ExpressionRenderEffects,
) -> [f32; 4] {
    let Some(material) = material else {
        return initial;
    };
    effects
        .material_colors
        .iter()
        .filter(|effect| effect.material == material && effect.kind == kind)
        .fold(initial, |mut color, effect| {
            let target = [
                effect.target_value.first().copied().unwrap_or(initial[0]),
                effect.target_value.get(1).copied().unwrap_or(initial[1]),
                effect.target_value.get(2).copied().unwrap_or(initial[2]),
                effect.target_value.get(3).copied().unwrap_or(1.0),
            ];
            for index in 0..4 {
                color[index] += (target[index] - initial[index]) * effect.weight;
            }
            color
        })
}

fn apply_color_effect3(
    initial: [f32; 3],
    material: Option<usize>,
    kind: &str,
    effects: &ExpressionRenderEffects,
) -> [f32; 3] {
    let Some(material) = material else {
        return initial;
    };
    effects
        .material_colors
        .iter()
        .filter(|effect| effect.material == material && effect.kind == kind)
        .fold(initial, |mut color, effect| {
            let target = [
                effect.target_value.first().copied().unwrap_or(initial[0]),
                effect.target_value.get(1).copied().unwrap_or(initial[1]),
                effect.target_value.get(2).copied().unwrap_or(initial[2]),
            ];
            for index in 0..3 {
                color[index] += (target[index] - initial[index]) * effect.weight;
            }
            color
        })
}

fn morphed_vertex(
    primitive: &GltfPrimitiveData,
    index: usize,
    morph_weights: &[f32],
) -> Option<(GVec3, GVec3, GVec4)> {
    let mut position = GVec3::from_array(*primitive.positions.get(index)?);
    let mut normal = primitive_normal(primitive, index);
    let base_tangent = primitive
        .tangents
        .get(index)
        .copied()
        .unwrap_or([1.0, 0.0, 0.0, 1.0]);
    let mut tangent = GVec3::new(base_tangent[0], base_tangent[1], base_tangent[2]);

    for (target, weight) in primitive
        .morph_targets
        .iter()
        .zip(morph_weights.iter().copied())
        .filter(|(_, weight)| weight.abs() > f32::EPSILON)
    {
        if let Some(delta) = target.positions.get(index).copied() {
            position += GVec3::from_array(delta) * weight;
        }
        if let Some(delta) = target.normals.get(index).copied() {
            normal += GVec3::from_array(delta) * weight;
        }
        if let Some(delta) = target.tangents.get(index).copied() {
            tangent += GVec3::from_array(delta) * weight;
        }
    }

    Some((position, normal, tangent.extend(base_tangent[3])))
}

fn bevy_mesh(
    primitive: &GltfPrimitiveData,
    morph_weights: &[f32],
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    generate_tangents: bool,
) -> (Mesh, bool) {
    let positions = primitive
        .positions
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let (local_position, local_normal, _) = morphed_vertex(primitive, index, morph_weights)
                .unwrap_or((GVec3::ZERO, GVec3::Z, GVec4::new(1.0, 0.0, 0.0, 1.0)));
            let (position, _) = transform_vertex(
                local_position,
                local_normal,
                world,
                skin_matrices,
                primitive.joints_0.get(index).copied(),
                primitive.weights_0.get(index).copied(),
            );
            position.to_array()
        })
        .collect::<Vec<_>>();
    let normals = (0..primitive.positions.len())
        .map(|index| {
            let (local_position, local_normal, _) = morphed_vertex(primitive, index, morph_weights)
                .unwrap_or((GVec3::ZERO, GVec3::Z, GVec4::new(1.0, 0.0, 0.0, 1.0)));
            let (_, normal) = transform_vertex(
                local_position,
                local_normal,
                world,
                skin_matrices,
                primitive.joints_0.get(index).copied(),
                primitive.weights_0.get(index).copied(),
            );
            normal.to_array()
        })
        .collect::<Vec<_>>();
    let tangents = if primitive.tangents.len() == primitive.positions.len() {
        Some(
            primitive
                .tangents
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    let (_, _, tangent) = morphed_vertex(primitive, index, morph_weights)
                        .unwrap_or((GVec3::ZERO, GVec3::Z, GVec4::new(1.0, 0.0, 0.0, 1.0)));
                    let direction = transform_direction(
                        tangent.truncate(),
                        world,
                        skin_matrices,
                        primitive.joints_0.get(index).copied(),
                        primitive.weights_0.get(index).copied(),
                    );
                    [direction.x, direction.y, direction.z, tangent.w]
                })
                .collect::<Vec<_>>(),
        )
    } else if generate_tangents {
        generated_tangents(
            &positions,
            &normals,
            &tex_coords_or_default(primitive.positions.len(), &primitive.tex_coords_0),
            &primitive.indices,
        )
    } else {
        None
    };
    let has_tangents = tangents.is_some();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    if let Some(tangents) = tangents {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    }
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        tex_coords_or_default(primitive.positions.len(), &primitive.tex_coords_0),
    );
    mesh.insert_indices(Indices::U32(primitive.indices.clone()));
    (mesh, has_tangents)
}

fn generated_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    tex_coords: &[[f32; 2]],
    indices: &[u32],
) -> Option<Vec<[f32; 4]>> {
    let mut tangents = vec![GVec3::ZERO; positions.len()];
    let mut bitangents = vec![GVec3::ZERO; positions.len()];
    let mut referenced = vec![false; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let [i0, i1, i2] = [
            usize::try_from(triangle[0]).ok()?,
            usize::try_from(triangle[1]).ok()?,
            usize::try_from(triangle[2]).ok()?,
        ];
        for index in [i0, i1, i2] {
            *referenced.get_mut(index)? = true;
        }
        let [p0, p1, p2] = [
            GVec3::from_array(*positions.get(i0)?),
            GVec3::from_array(*positions.get(i1)?),
            GVec3::from_array(*positions.get(i2)?),
        ];
        let [uv0, uv1, uv2] = [
            *tex_coords.get(i0)?,
            *tex_coords.get(i1)?,
            *tex_coords.get(i2)?,
        ];
        let delta_pos1 = p1 - p0;
        let delta_pos2 = p2 - p0;
        let delta_uv1 = GVec3::new(uv1[0] - uv0[0], uv1[1] - uv0[1], 0.0);
        let delta_uv2 = GVec3::new(uv2[0] - uv0[0], uv2[1] - uv0[1], 0.0);
        let determinant = delta_uv1.x * delta_uv2.y - delta_uv1.y * delta_uv2.x;
        if determinant.abs() <= f32::EPSILON {
            continue;
        }
        let scale = determinant.recip();
        let tangent = (delta_pos1 * delta_uv2.y - delta_pos2 * delta_uv1.y) * scale;
        let bitangent = (delta_pos2 * delta_uv1.x - delta_pos1 * delta_uv2.x) * scale;
        for index in [i0, i1, i2] {
            tangents[index] += tangent;
            bitangents[index] += bitangent;
        }
    }
    tangents
        .into_iter()
        .zip(bitangents)
        .zip(referenced)
        .zip(normals)
        .map(|(((tangent, bitangent), referenced), normal)| {
            let normal = GVec3::from_array(*normal).normalize_or_zero();
            let tangent = tangent - normal * normal.dot(tangent);
            if tangent.length_squared() <= f32::EPSILON {
                return (!referenced).then(|| fallback_tangent(normal));
            }
            let tangent = tangent.normalize();
            let handedness = if normal.cross(tangent).dot(bitangent) < 0.0 {
                -1.0
            } else {
                1.0
            };
            Some([tangent.x, tangent.y, tangent.z, handedness])
        })
        .collect()
}

fn fallback_tangent(normal: GVec3) -> [f32; 4] {
    let seed = if normal.x.abs() < 0.9 {
        GVec3::X
    } else {
        GVec3::Y
    };
    let tangent = (seed - normal * normal.dot(seed)).normalize_or_zero();
    [tangent.x, tangent.y, tangent.z, 1.0]
}

#[derive(Clone, Copy, Debug)]
struct MaterialShading {
    base_color: [f32; 4],
    shade_color: [f32; 4],
    shading_shift: f32,
    shading_toony: f32,
    shading_shift_texture_scale: f32,
    gi_equalization: f32,
    emissive: [f32; 3],
    matcap_factor: [f32; 3],
    parametric_rim_color: [f32; 3],
    rim_lighting_mix: f32,
    parametric_rim_fresnel_power: f32,
    parametric_rim_lift: f32,
    normal_scale: f32,
    metallic: f32,
    roughness: f32,
    occlusion_strength: f32,
    pbr_fallback: bool,
    v0_compat_shade: bool,
}

fn material_shading(
    loaded: &LoadedVrm,
    material: Option<usize>,
    expression_effects: &ExpressionRenderEffects,
) -> MaterialShading {
    if let Some(shading) = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|core_material| {
            let mtoon = core_material.mtoon.as_ref()?;
            let (emissive_strength, _) = core_material.effective_emissive_strength();
            let v0_compat_shade = loaded.model().document().kind == VrmKind::Vrm0Compat;
            let base_color = apply_color_effect4(
                mtoon.base_color_factor,
                material,
                "color",
                expression_effects,
            );
            let shade_color = apply_color_effect4(
                [
                    mtoon.shade_color_factor[0],
                    mtoon.shade_color_factor[1],
                    mtoon.shade_color_factor[2],
                    1.0,
                ],
                material,
                "shadeColor",
                expression_effects,
            );
            let emissive = apply_color_effect3(
                [
                    mtoon.emissive_factor[0] * emissive_strength.0,
                    mtoon.emissive_factor[1] * emissive_strength.0,
                    mtoon.emissive_factor[2] * emissive_strength.0,
                ],
                material,
                "emissionColor",
                expression_effects,
            );
            Some(MaterialShading {
                base_color,
                shade_color,
                shading_shift: mtoon.shading_shift_factor,
                shading_toony: mtoon.shading_toony_factor,
                shading_shift_texture_scale: mtoon.shading_shift_texture_scale,
                gi_equalization: mtoon.gi_equalization_factor,
                emissive,
                matcap_factor: apply_color_effect3(
                    mtoon.matcap_factor,
                    material,
                    "matcapColor",
                    expression_effects,
                ),
                parametric_rim_color: apply_color_effect3(
                    mtoon.parametric_rim_color_factor,
                    material,
                    "rimColor",
                    expression_effects,
                ),
                rim_lighting_mix: mtoon.rim_lighting_mix_factor,
                parametric_rim_fresnel_power: mtoon.parametric_rim_fresnel_power_factor,
                parametric_rim_lift: mtoon.parametric_rim_lift_factor,
                normal_scale: material_normal_texture(loaded, material).map_or(0.0, |_| {
                    material
                        .and_then(|index| loaded.gltf_materials.get(index))
                        .map_or(1.0, |gltf_material| gltf_material.normal_scale)
                }),
                metallic: 0.0,
                roughness: 1.0,
                occlusion_strength: 0.0,
                pbr_fallback: false,
                v0_compat_shade,
            })
        })
    {
        return shading;
    }
    let gltf = material.and_then(|index| loaded.gltf_materials.get(index));
    let base_color = gltf
        .map(|material| material.base_color_factor)
        .unwrap_or([0.78, 0.78, 0.78, 1.0]);
    let emissive = gltf
        .map(|material| {
            material
                .emissive_factor
                .map(|channel| channel * material.emissive_strength)
        })
        .unwrap_or([0.0, 0.0, 0.0]);
    MaterialShading {
        base_color: apply_color_effect4(base_color, material, "color", expression_effects),
        shade_color: base_color,
        shading_shift: 0.0,
        shading_toony: 0.0,
        shading_shift_texture_scale: 1.0,
        gi_equalization: 0.0,
        emissive: apply_color_effect3(emissive, material, "emissionColor", expression_effects),
        matcap_factor: [0.0, 0.0, 0.0],
        parametric_rim_color: [0.0, 0.0, 0.0],
        rim_lighting_mix: 1.0,
        parametric_rim_fresnel_power: 5.0,
        parametric_rim_lift: 0.0,
        normal_scale: material_normal_texture(loaded, material)
            .map_or(0.0, |_| gltf.map_or(1.0, |material| material.normal_scale)),
        metallic: gltf.map_or(0.0, |material| material.metallic_factor),
        roughness: gltf.map_or(1.0, |material| material.roughness_factor),
        occlusion_strength: gltf.map_or(1.0, |material| material.occlusion_strength),
        pbr_fallback: true,
        v0_compat_shade: false,
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[uniform(0, BevyMtoonUniform)]
#[bind_group_data(BevyMtoonKey)]
struct BevyMtoonMaterial {
    base_color: BVec4,
    shade_color: BVec4,
    shading: BVec4,
    emissive: BVec4,
    matcap_factor: BVec4,
    rim_color: BVec4,
    rim_params: BVec4,
    material_flags: BVec4,
    pbr_params: BVec4,
    outline_color: BVec4,
    pipeline: BVec4,
    lighting: BVec4,
    light_color: BVec4,
    base_uv_transform: BVec4,
    shade_uv_transform: BVec4,
    shading_shift_uv_transform: BVec4,
    normal_uv_transform: BVec4,
    matcap_uv_transform: BVec4,
    rim_uv_transform: BVec4,
    emissive_uv_transform: BVec4,
    occlusion_uv_transform: BVec4,
    uv_animation_mask_uv_transform: BVec4,
    uv_rotation_a: BVec4,
    uv_rotation_b: BVec4,
    uv_animation: BVec4,
    #[texture(1)]
    #[sampler(2)]
    base_texture: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    shade_texture: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    shading_shift_texture: Handle<Image>,
    #[texture(7)]
    #[sampler(8)]
    matcap_texture: Handle<Image>,
    #[texture(9)]
    #[sampler(10)]
    rim_texture: Handle<Image>,
    #[texture(11)]
    #[sampler(12)]
    normal_texture: Handle<Image>,
    #[texture(13)]
    #[sampler(14)]
    emissive_texture: Handle<Image>,
    #[texture(15)]
    #[sampler(16)]
    uv_animation_mask_texture: Handle<Image>,
    #[texture(17)]
    #[sampler(18)]
    occlusion_texture: Handle<Image>,
    alpha_mode: AlphaMode,
    cull_mode: Option<Face>,
    depth_write: bool,
    depth_bias: f32,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct BevyMtoonKey {
    cull_mode: Option<Face>,
    depth_write: bool,
}

#[derive(Clone, Copy, Debug, ShaderType)]
struct BevyMtoonUniform {
    base_color: BVec4,
    shade_color: BVec4,
    shading: BVec4,
    emissive: BVec4,
    matcap_factor: BVec4,
    rim_color: BVec4,
    rim_params: BVec4,
    material_flags: BVec4,
    pbr_params: BVec4,
    outline_color: BVec4,
    pipeline: BVec4,
    lighting: BVec4,
    light_color: BVec4,
    base_uv_transform: BVec4,
    shade_uv_transform: BVec4,
    shading_shift_uv_transform: BVec4,
    normal_uv_transform: BVec4,
    matcap_uv_transform: BVec4,
    rim_uv_transform: BVec4,
    emissive_uv_transform: BVec4,
    occlusion_uv_transform: BVec4,
    uv_animation_mask_uv_transform: BVec4,
    uv_rotation_a: BVec4,
    uv_rotation_b: BVec4,
    uv_animation: BVec4,
}

impl From<&BevyMtoonMaterial> for BevyMtoonUniform {
    fn from(material: &BevyMtoonMaterial) -> Self {
        Self {
            base_color: material.base_color,
            shade_color: material.shade_color,
            shading: material.shading,
            emissive: material.emissive,
            matcap_factor: material.matcap_factor,
            rim_color: material.rim_color,
            rim_params: material.rim_params,
            material_flags: material.material_flags,
            pbr_params: material.pbr_params,
            outline_color: material.outline_color,
            pipeline: material.pipeline,
            lighting: material.lighting,
            light_color: material.light_color,
            base_uv_transform: material.base_uv_transform,
            shade_uv_transform: material.shade_uv_transform,
            shading_shift_uv_transform: material.shading_shift_uv_transform,
            normal_uv_transform: material.normal_uv_transform,
            matcap_uv_transform: material.matcap_uv_transform,
            rim_uv_transform: material.rim_uv_transform,
            emissive_uv_transform: material.emissive_uv_transform,
            occlusion_uv_transform: material.occlusion_uv_transform,
            uv_animation_mask_uv_transform: material.uv_animation_mask_uv_transform,
            uv_rotation_a: material.uv_rotation_a,
            uv_rotation_b: material.uv_rotation_b,
            uv_animation: material.uv_animation,
        }
    }
}

impl From<&BevyMtoonMaterial> for BevyMtoonKey {
    fn from(material: &BevyMtoonMaterial) -> Self {
        Self {
            cull_mode: material.cull_mode,
            depth_write: material.depth_write,
        }
    }
}

impl Material for BevyMtoonMaterial {
    fn fragment_shader() -> ShaderRef {
        MTOON_SHADER_ASSET_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn depth_bias(&self) -> f32 {
        self.depth_bias
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = key.bind_group_data.cull_mode;
        if layout.0.contains(Mesh::ATTRIBUTE_TANGENT) {
            descriptor.vertex.shader_defs.push("VERTEX_TANGENTS".into());
            if let Some(fragment) = &mut descriptor.fragment {
                fragment.shader_defs.push("VERTEX_TANGENTS".into());
            }
        }
        if let Some(depth_stencil) = &mut descriptor.depth_stencil {
            depth_stencil.depth_write_enabled = key.bind_group_data.depth_write;
        }
        Ok(())
    }
}

fn bevy_mtoon_material(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    shading: MaterialShading,
    context: &BevyPrimitiveContext<'_>,
    depth_bias: f32,
    normal_scale: f32,
) -> BevyMtoonMaterial {
    let alpha_mode = material_alpha_mode(loaded, primitive.material);
    let cull_mode = material_cull_mode(loaded, primitive.material);
    let depth_write = material_depth_write(loaded, primitive.material);
    let uv_transforms = material_uv_transforms(
        loaded,
        primitive.material,
        context.options.mtoon_time,
        context.expression_effects,
    );
    let image_handles = context.image_handles;
    BevyMtoonMaterial {
        base_color: BVec4::from_array(shading.base_color),
        shade_color: BVec4::from_array(shading.shade_color),
        shading: BVec4::new(
            shading.shading_shift,
            shading.shading_toony,
            shading.gi_equalization,
            shading.shading_shift_texture_scale,
        ),
        emissive: BVec4::new(
            shading.emissive[0],
            shading.emissive[1],
            shading.emissive[2],
            0.0,
        ),
        matcap_factor: BVec4::new(
            shading.matcap_factor[0],
            shading.matcap_factor[1],
            shading.matcap_factor[2],
            0.0,
        ),
        rim_color: BVec4::new(
            shading.parametric_rim_color[0],
            shading.parametric_rim_color[1],
            shading.parametric_rim_color[2],
            0.0,
        ),
        rim_params: BVec4::new(
            shading.rim_lighting_mix,
            shading.parametric_rim_fresnel_power,
            shading.parametric_rim_lift,
            0.0,
        ),
        material_flags: BVec4::new(
            if shading.v0_compat_shade { 1.0 } else { 0.0 },
            if shading.pbr_fallback { 1.0 } else { 0.0 },
            if context.options.mtoon_light_accumulation == MtoonLightAccumulation::ThreeVrm {
                1.0
            } else {
                0.0
            },
            0.0,
        ),
        pbr_params: BVec4::new(
            shading.metallic,
            shading.roughness,
            shading.occlusion_strength,
            context.options.direct_light_scale,
        ),
        outline_color: BVec4::new(1.0, 1.0, 1.0, -1.0),
        pipeline: BVec4::new(
            alpha_mode_code(alpha_mode),
            alpha_cutoff(alpha_mode),
            normal_scale,
            if cull_mode.is_none() { 1.0 } else { 0.0 },
        ),
        lighting: bevy_mtoon_lighting(context.options),
        light_color: BVec4::new(
            context.options.directional_r,
            context.options.directional_g,
            context.options.directional_b,
            0.0,
        ),
        base_uv_transform: bevy_uv_transform(uv_transforms.base),
        shade_uv_transform: bevy_uv_transform(uv_transforms.shade),
        shading_shift_uv_transform: bevy_uv_transform(uv_transforms.shading_shift),
        normal_uv_transform: bevy_uv_transform(uv_transforms.normal),
        matcap_uv_transform: bevy_uv_transform(uv_transforms.matcap),
        rim_uv_transform: bevy_uv_transform(uv_transforms.rim),
        emissive_uv_transform: bevy_uv_transform(uv_transforms.emissive),
        occlusion_uv_transform: bevy_uv_transform(uv_transforms.occlusion),
        uv_animation_mask_uv_transform: bevy_uv_transform(uv_transforms.uv_animation_mask),
        uv_rotation_a: BVec4::new(
            bevy_uv_rotation(uv_transforms.base),
            bevy_uv_rotation(uv_transforms.shade),
            bevy_uv_rotation(uv_transforms.shading_shift),
            bevy_uv_rotation(uv_transforms.normal),
        ),
        uv_rotation_b: BVec4::new(
            bevy_uv_rotation(uv_transforms.rim),
            bevy_uv_rotation(uv_transforms.emissive),
            bevy_uv_rotation(uv_transforms.uv_animation_mask),
            bevy_uv_rotation(uv_transforms.matcap),
        ),
        uv_animation: BVec4::new(
            uv_transforms.uv_animation_scroll[0],
            uv_transforms.uv_animation_scroll[1],
            uv_transforms.uv_animation_rotation,
            bevy_uv_rotation(uv_transforms.occlusion),
        ),
        base_texture: material_main_image(loaded, primitive.material)
            .and_then(|image| image_handles.color_images.get(image))
            .and_then(Clone::clone)
            .unwrap_or_else(|| image_handles.white.clone()),
        shade_texture: material_shade_image(loaded, primitive.material)
            .and_then(|image| image_handles.color_images.get(image))
            .and_then(Clone::clone)
            .unwrap_or_else(|| image_handles.white.clone()),
        shading_shift_texture: material_shading_shift_image_index(loaded, primitive.material)
            .and_then(|image| image_handles.color_images.get(image))
            .and_then(Clone::clone)
            .unwrap_or_else(|| image_handles.black.clone()),
        matcap_texture: material_matcap_image(loaded, primitive.material)
            .and_then(|image| image_handles.color_images.get(image))
            .and_then(Clone::clone)
            .unwrap_or_else(|| image_handles.black.clone()),
        rim_texture: material_rim_image(loaded, primitive.material)
            .and_then(|image| image_handles.color_images.get(image))
            .and_then(Clone::clone)
            .unwrap_or_else(|| image_handles.white.clone()),
        normal_texture: material_normal_texture(loaded, primitive.material)
            .and_then(|texture| loaded.textures.get(texture))
            .map(|texture| texture.image)
            .and_then(|image| image_handles.linear_images.get(image))
            .and_then(Clone::clone)
            .unwrap_or_else(|| image_handles.neutral_normal.clone()),
        emissive_texture: material_emissive_image(loaded, primitive.material)
            .and_then(|image| image_handles.color_images.get(image))
            .and_then(Clone::clone)
            .unwrap_or_else(|| image_handles.white.clone()),
        uv_animation_mask_texture: material_uv_animation_mask_image(loaded, primitive.material)
            .and_then(|image| image_handles.color_images.get(image))
            .and_then(Clone::clone)
            .unwrap_or_else(|| image_handles.white.clone()),
        occlusion_texture: material_occlusion_image(loaded, primitive.material)
            .and_then(|image| image_handles.linear_images.get(image))
            .and_then(Clone::clone)
            .unwrap_or_else(|| image_handles.white.clone()),
        alpha_mode,
        cull_mode,
        depth_write,
        depth_bias,
    }
}

fn bevy_mtoon_lighting(options: &CaptureOptions) -> BVec4 {
    let [exposure, ambient_base, ambient_gi_scale, pbr_ambient] = mtoon_lighting_values(options);
    BVec4::new(exposure, ambient_base, ambient_gi_scale, pbr_ambient)
}

fn mtoon_lighting_values(options: &CaptureOptions) -> [f32; 4] {
    match options.mtoon_light_accumulation {
        MtoonLightAccumulation::Tuned => [
            options.mtoon_exposure,
            options.mtoon_ambient_base,
            options.mtoon_ambient_gi_scale,
            options.pbr_ambient,
        ],
        MtoonLightAccumulation::ThreeVrm => [1.0, options.pbr_ambient, 0.0, options.pbr_ambient],
    }
}

fn alpha_mode_code(mode: AlphaMode) -> f32 {
    match mode {
        AlphaMode::Opaque | AlphaMode::AlphaToCoverage => 0.0,
        AlphaMode::Mask(_) => 1.0,
        AlphaMode::Blend | AlphaMode::Premultiplied | AlphaMode::Add | AlphaMode::Multiply => 2.0,
    }
}

fn alpha_cutoff(mode: AlphaMode) -> f32 {
    match mode {
        AlphaMode::Mask(cutoff) => cutoff,
        AlphaMode::Opaque
        | AlphaMode::Blend
        | AlphaMode::Premultiplied
        | AlphaMode::Add
        | AlphaMode::Multiply
        | AlphaMode::AlphaToCoverage => 0.5,
    }
}

fn camera_eye(options: &CaptureOptions) -> GVec3 {
    GVec3::new(0.0, options.camera_y, -options.camera_z)
}

fn camera_view(options: &CaptureOptions) -> Mat4 {
    Mat4::look_at_rh(
        camera_eye(options),
        GVec3::new(0.0, options.target_y, 0.0),
        GVec3::Y,
    )
}

fn projection_y_scale() -> f32 {
    1.0 / (0.5 * 30.0_f32.to_radians()).tan()
}

fn primitive_normal(primitive: &GltfPrimitiveData, index: usize) -> GVec3 {
    primitive
        .normals
        .get(index)
        .copied()
        .map(GVec3::from_array)
        .unwrap_or(GVec3::Z)
}

fn skin_matrices(
    loaded: &LoadedVrm,
    skin: &vrm_io::GltfSkinData,
    world_matrices: &[Mat4],
    orientation: Mat4,
) -> Vec<Mat4> {
    skin.joints
        .iter()
        .enumerate()
        .map(|(index, joint)| {
            let joint_world = world_matrices
                .get(*joint)
                .copied()
                .or_else(|| loaded.scene.node(*joint).map(|node| node.world_matrix))
                .unwrap_or(Mat4::IDENTITY);
            let inverse_bind = skin
                .inverse_bind_matrices
                .get(index)
                .copied()
                .unwrap_or(Mat4::IDENTITY);
            orientation * joint_world * inverse_bind
        })
        .collect()
}

fn transform_vertex(
    position: GVec3,
    normal: GVec3,
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    joints: Option<[u16; 4]>,
    weights: Option<[f32; 4]>,
) -> (GVec3, GVec3) {
    let (Some(skin_matrices), Some(joints), Some(weights)) = (skin_matrices, joints, weights)
    else {
        return (
            world.transform_point3(position),
            world.transform_vector3(normal).normalize_or_zero(),
        );
    };

    let mut skinned_position = GVec3::ZERO;
    let mut skinned_normal = GVec3::ZERO;
    let mut total_weight = 0.0;
    for (joint, weight) in joints.into_iter().zip(weights) {
        if weight <= 0.0 {
            continue;
        }
        let Some(matrix) = skin_matrices.get(usize::from(joint)) else {
            continue;
        };
        skinned_position += matrix.transform_point3(position) * weight;
        skinned_normal += matrix.transform_vector3(normal) * weight;
        total_weight += weight;
    }
    if total_weight > 0.0 {
        (skinned_position, skinned_normal.normalize_or_zero())
    } else {
        (
            world.transform_point3(position),
            world.transform_vector3(normal).normalize_or_zero(),
        )
    }
}

fn transform_direction(
    direction: GVec3,
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    joints: Option<[u16; 4]>,
    weights: Option<[f32; 4]>,
) -> GVec3 {
    let Some(skin_matrices) = skin_matrices else {
        return world.transform_vector3(direction).normalize_or_zero();
    };
    let (Some(joints), Some(weights)) = (joints, weights) else {
        return world.transform_vector3(direction).normalize_or_zero();
    };

    let mut transformed = GVec3::ZERO;
    let mut total_weight = 0.0;
    for (joint, weight) in joints.into_iter().zip(weights) {
        if weight <= 0.0 {
            continue;
        }
        let Some(matrix) = skin_matrices.get(usize::from(joint)) else {
            continue;
        };
        transformed += matrix.transform_vector3(direction) * weight;
        total_weight += weight;
    }
    if total_weight > 0.0 {
        transformed.normalize_or_zero()
    } else {
        world.transform_vector3(direction).normalize_or_zero()
    }
}

fn tex_coords_or_default(vertex_count: usize, tex_coords: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if tex_coords.len() == vertex_count {
        tex_coords.to_vec()
    } else {
        vec![[0.0, 0.0]; vertex_count]
    }
}

fn material_render_order(loaded: &LoadedVrm, material: Option<usize>) -> i32 {
    let mtoon = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref());
    let render_order = mtoon
        .map(|mtoon| mtoon.pipeline_hints().render_order)
        .unwrap_or(2000);
    if material
        .and_then(|index| loaded.gltf_materials.get(index))
        .is_some_and(|material| material.alpha_mode == GltfAlphaMode::Blend)
    {
        mtoon.map_or(render_order.max(3000), |mtoon| {
            3000 + bevy_transparent_spawn_order_offset(mtoon)
        })
    } else {
        render_order
    }
}

fn bevy_transparent_spawn_order_offset(mtoon: &vrm_core::MtoonMaterial) -> i32 {
    1000 - mtoon_transparent_order_offset(mtoon)
}

fn material_phase_order(loaded: &LoadedVrm, material: Option<usize>) -> i32 {
    material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .map_or(2000, mtoon_transparent_order_offset)
}

fn mtoon_transparent_order_offset(mtoon: &vrm_core::MtoonMaterial) -> i32 {
    let queue_offset = if mtoon.transparent_with_z_write {
        0
    } else {
        19
    };
    queue_offset + mtoon.render_queue_offset_number
}

fn material_depth_write(loaded: &LoadedVrm, material: Option<usize>) -> bool {
    let mtoon = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref());
    let is_blend = material
        .and_then(|index| loaded.gltf_materials.get(index))
        .is_some_and(|material| material.alpha_mode == GltfAlphaMode::Blend)
        || mtoon.is_some_and(|mtoon| mtoon.pipeline_hints().alpha_mode == MtoonAlphaMode::Blend);
    if is_blend {
        mtoon.is_some_and(|mtoon| mtoon.transparent_with_z_write)
    } else {
        true
    }
}

fn render_depth_bias(_render_order: i32) -> f32 {
    0.0
}

fn material_cull_mode(loaded: &LoadedVrm, material: Option<usize>) -> Option<Face> {
    let cull_mode = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .map(|mtoon| match mtoon.pipeline_hints().cull_mode {
            MtoonCullMode::Off => None,
            MtoonCullMode::Front => Some(Face::Front),
            MtoonCullMode::Back => Some(Face::Back),
        })
        .unwrap_or(Some(Face::Back));
    if material
        .and_then(|index| loaded.gltf_materials.get(index))
        .is_some_and(|material| material.double_sided)
    {
        None
    } else {
        cull_mode
    }
}

fn material_alpha_mode(loaded: &LoadedVrm, material: Option<usize>) -> AlphaMode {
    let mtoon_alpha = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .map(|mtoon| match mtoon.pipeline_hints().alpha_mode {
            MtoonAlphaMode::Opaque => AlphaMode::Opaque,
            MtoonAlphaMode::Mask => AlphaMode::Mask(mtoon.cutoff_factor),
            MtoonAlphaMode::Blend => AlphaMode::Blend,
        })
        .unwrap_or(AlphaMode::Opaque);
    material
        .and_then(|index| loaded.gltf_materials.get(index))
        .map(|material| match material.alpha_mode {
            GltfAlphaMode::Opaque => mtoon_alpha,
            GltfAlphaMode::Mask => AlphaMode::Mask(material.alpha_cutoff.unwrap_or(0.5)),
            GltfAlphaMode::Blend => AlphaMode::Blend,
        })
        .unwrap_or(mtoon_alpha)
}

fn material_normal_texture(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let mtoon_texture = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.normal_texture)
        .map(|texture| texture.0);
    mtoon_texture.or_else(|| {
        material
            .and_then(|index| loaded.gltf_materials.get(index))
            .and_then(|material| material.normal_texture)
    })
}

fn material_uv_transforms(
    loaded: &LoadedVrm,
    material: Option<usize>,
    mtoon_time: f32,
    expression_effects: &ExpressionRenderEffects,
) -> MaterialUvTransforms {
    let mtoon = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref());
    let gltf = material.and_then(|index| loaded.gltf_materials.get(index));
    let base = mtoon
        .and_then(|mtoon| mtoon.texture_transforms.main_texture)
        .or_else(|| gltf.and_then(|material| material.base_color_texture_transform));
    let shade = mtoon
        .and_then(|mtoon| mtoon.texture_transforms.shade_multiply_texture)
        .or(base);
    let transforms = MaterialUvTransforms {
        base,
        shade,
        shading_shift: mtoon.and_then(|mtoon| mtoon.texture_transforms.shading_shift_texture),
        normal: mtoon
            .and_then(|mtoon| mtoon.texture_transforms.normal_texture)
            .or_else(|| gltf.and_then(|material| material.normal_texture_transform)),
        matcap: mtoon.and_then(|mtoon| mtoon.texture_transforms.matcap_texture),
        rim: mtoon.and_then(|mtoon| mtoon.texture_transforms.rim_multiply_texture),
        outline_width: mtoon
            .and_then(|mtoon| mtoon.texture_transforms.outline_width_multiply_texture),
        emissive: gltf.and_then(|material| material.emissive_texture_transform),
        occlusion: gltf.and_then(|material| material.occlusion_texture_transform),
        uv_animation_mask: mtoon
            .and_then(|mtoon| mtoon.texture_transforms.uv_animation_mask_texture),
        uv_animation_scroll: mtoon.map_or([0.0, 0.0], |mtoon| {
            [
                mtoon.uv_animation.scroll_x_speed * mtoon_time,
                mtoon.uv_animation.scroll_y_speed * mtoon_time,
            ]
        }),
        uv_animation_rotation: mtoon
            .map_or(0.0, |mtoon| mtoon.uv_animation.rotation_speed * mtoon_time),
    };
    apply_texture_transform_effects(transforms, material, expression_effects)
}

fn apply_texture_transform_effects(
    mut transforms: MaterialUvTransforms,
    material: Option<usize>,
    effects: &ExpressionRenderEffects,
) -> MaterialUvTransforms {
    let Some(material) = material else {
        return transforms;
    };
    for effect in effects
        .texture_transforms
        .iter()
        .filter(|effect| effect.material == material)
    {
        transforms.base = Some(apply_texture_transform_slot(transforms.base, effect));
        transforms.shade = Some(apply_texture_transform_slot(transforms.shade, effect));
        transforms.shading_shift = Some(apply_texture_transform_slot(
            transforms.shading_shift,
            effect,
        ));
        transforms.normal = Some(apply_texture_transform_slot(transforms.normal, effect));
        transforms.matcap = Some(apply_texture_transform_slot(transforms.matcap, effect));
        transforms.rim = Some(apply_texture_transform_slot(transforms.rim, effect));
        transforms.outline_width = Some(apply_texture_transform_slot(
            transforms.outline_width,
            effect,
        ));
        transforms.emissive = Some(apply_texture_transform_slot(transforms.emissive, effect));
        transforms.occlusion = Some(apply_texture_transform_slot(transforms.occlusion, effect));
        transforms.uv_animation_mask = Some(apply_texture_transform_slot(
            transforms.uv_animation_mask,
            effect,
        ));
    }
    transforms
}

fn apply_texture_transform_slot(
    initial: Option<TextureTransform2d>,
    effect: &TextureTransformEffect,
) -> TextureTransform2d {
    let initial = initial.unwrap_or_default();
    let target_scale = effect.scale.unwrap_or(initial.scale);
    let target_offset = effect.offset.unwrap_or(initial.offset);
    TextureTransform2d {
        offset: [
            initial.offset[0] + (target_offset[0] - initial.offset[0]) * effect.weight,
            initial.offset[1] + (target_offset[1] - initial.offset[1]) * effect.weight,
        ],
        scale: [
            initial.scale[0] + (target_scale[0] - initial.scale[0]) * effect.weight,
            initial.scale[1] + (target_scale[1] - initial.scale[1]) * effect.weight,
        ],
        rotation: initial.rotation,
        tex_coord: initial.tex_coord,
    }
}

fn transform_uv(uv: [f32; 2], transform: Option<TextureTransform2d>) -> [f32; 2] {
    let Some(transform) = transform else {
        return uv;
    };
    if transform.tex_coord.is_some_and(|tex_coord| tex_coord != 0) {
        return uv;
    }
    let (sin, cos) = transform.rotation.sin_cos();
    let scaled = [uv[0] * transform.scale[0], uv[1] * transform.scale[1]];
    [
        cos * scaled[0] - sin * scaled[1] + transform.offset[0],
        sin * scaled[0] + cos * scaled[1] + transform.offset[1],
    ]
}

fn bevy_uv_transform(transform: Option<TextureTransform2d>) -> BVec4 {
    let Some(transform) =
        transform.filter(|transform| transform.tex_coord.is_none_or(|tex_coord| tex_coord == 0))
    else {
        return BVec4::new(0.0, 0.0, 1.0, 1.0);
    };
    BVec4::new(
        transform.offset[0],
        transform.offset[1],
        transform.scale[0],
        transform.scale[1],
    )
}

fn bevy_uv_rotation(transform: Option<TextureTransform2d>) -> f32 {
    transform
        .filter(|transform| transform.tex_coord.is_none_or(|tex_coord| tex_coord == 0))
        .map_or(0.0, |transform| transform.rotation)
}

fn material_main_image(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let mtoon_texture = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.main_texture);
    let texture = mtoon_texture.map(|texture| texture.0).or_else(|| {
        material
            .and_then(|index| loaded.gltf_materials.get(index))
            .and_then(|material| material.base_color_texture)
    })?;
    loaded.textures.get(texture).map(|texture| texture.image)
}

fn material_shade_image(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let texture = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.shade_multiply_texture)?;
    loaded.textures.get(texture.0).map(|texture| texture.image)
}

fn material_shading_shift_image_index(
    loaded: &LoadedVrm,
    material: Option<usize>,
) -> Option<usize> {
    let texture = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.shading_shift_texture)?;
    loaded.textures.get(texture.0).map(|texture| texture.image)
}

fn material_matcap_image(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let texture = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.matcap_texture)?;
    loaded.textures.get(texture.0).map(|texture| texture.image)
}

fn material_rim_image(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let texture = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.rim_multiply_texture)?;
    loaded.textures.get(texture.0).map(|texture| texture.image)
}

fn material_emissive_image(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let texture = material
        .and_then(|index| loaded.gltf_materials.get(index))
        .and_then(|material| material.emissive_texture)?;
    loaded.textures.get(texture).map(|texture| texture.image)
}

fn material_occlusion_image(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let texture = material
        .and_then(|index| loaded.gltf_materials.get(index))
        .and_then(|material| material.occlusion_texture)?;
    loaded.textures.get(texture).map(|texture| texture.image)
}

fn material_uv_animation_mask_image(loaded: &LoadedVrm, material: Option<usize>) -> Option<usize> {
    let texture = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.uv_animation_mask_texture)?;
    loaded.textures.get(texture.0).map(|texture| texture.image)
}

fn material_outline_width_image(
    loaded: &LoadedVrm,
    material: Option<usize>,
) -> Option<CpuRgbaImage> {
    material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .and_then(|mtoon| mtoon.textures.outline_width_multiply_texture)
        .and_then(|texture| sampled_image_for_texture(loaded, texture.0))
}

fn sampled_image_for_texture(loaded: &LoadedVrm, texture: usize) -> Option<CpuRgbaImage> {
    let image = loaded.textures.get(texture)?.image;
    let image = loaded.images.get(image)?;
    Some(CpuRgbaImage {
        width: image.width,
        height: image.height,
        rgba: image_rgba8(image)?,
    })
}

impl CpuRgbaImage {
    fn sample_green(&self, tex_coord: [f32; 2]) -> f32 {
        self.sample_channel(tex_coord, 1, 255)
    }

    fn sample_channel(&self, tex_coord: [f32; 2], channel: usize, fallback: u8) -> f32 {
        let u = tex_coord[0].rem_euclid(1.0);
        let v = tex_coord[1].rem_euclid(1.0);
        let x = u * self.width as f32 - 0.5;
        let y = (1.0 - v) * self.height as f32 - 0.5;
        let x0 = x.floor();
        let y0 = y.floor();
        let tx = x - x0;
        let ty = y - y0;
        let x0 = x0 as i32;
        let y0 = y0 as i32;
        let top = lerp(
            self.channel_at(x0, y0, channel, fallback),
            self.channel_at(x0 + 1, y0, channel, fallback),
            tx,
        );
        let bottom = lerp(
            self.channel_at(x0, y0 + 1, channel, fallback),
            self.channel_at(x0 + 1, y0 + 1, channel, fallback),
            tx,
        );
        lerp(top, bottom, ty)
    }

    fn channel_at(&self, x: i32, y: i32, channel: usize, fallback: u8) -> f32 {
        let width = self.width as i32;
        let height = self.height as i32;
        let x = x.rem_euclid(width) as u32;
        let y = y.rem_euclid(height) as u32;
        let index = ((y * self.width + x) * 4) as usize + channel;
        self.rgba.get(index).copied().unwrap_or(fallback) as f32 / 255.0
    }
}

fn lerp(left: f32, right: f32, t: f32) -> f32 {
    left + (right - left) * t
}

fn bevy_image(image: &ImageData) -> Option<Image> {
    bevy_image_with_format(image, TextureFormat::Rgba8UnormSrgb)
}

fn bevy_image_with_format(image: &ImageData, format: TextureFormat) -> Option<Image> {
    Some(bevy_image_from_rgba(
        image.width,
        image.height,
        image_rgba8(image)?,
        format,
    ))
}

fn bevy_image_from_rgba(width: u32, height: u32, rgba: Vec<u8>, format: TextureFormat) -> Image {
    let levels = mip_chain(width, height, &rgba);
    let mip_level_count = u32::try_from(levels.len()).unwrap_or(1);
    let data = levels
        .into_iter()
        .flat_map(|level| level.rgba)
        .collect::<Vec<_>>();
    let mut image = Image::new_uninit(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        format,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.mip_level_count = mip_level_count;
    image.data_order = TextureDataOrder::MipMajor;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Nearest,
        ..Default::default()
    });
    image
}

fn single_pixel_image(rgba: [u8; 4], format: TextureFormat) -> Image {
    bevy_image_from_rgba(1, 1, rgba.to_vec(), format)
}

struct TextureMipLevel {
    rgba: Vec<u8>,
}

fn mip_chain(width: u32, height: u32, rgba: &[u8]) -> Vec<TextureMipLevel> {
    let mut levels = vec![TextureMipLevel {
        rgba: rgba.to_vec(),
    }];
    let mut current_width = width;
    let mut current_height = height;
    let mut current_rgba = rgba.to_vec();
    while current_width > 1 || current_height > 1 {
        let next_width = (current_width / 2).max(1);
        let next_height = (current_height / 2).max(1);
        let Some(image) = image::RgbaImage::from_raw(current_width, current_height, current_rgba)
        else {
            break;
        };
        let next = image::imageops::resize(
            &image,
            next_width,
            next_height,
            image::imageops::FilterType::Triangle,
        );
        current_width = next_width;
        current_height = next_height;
        current_rgba = next.into_raw();
        levels.push(TextureMipLevel {
            rgba: current_rgba.clone(),
        });
    }
    levels
}

fn image_rgba8(image: &ImageData) -> Option<Vec<u8>> {
    match image.format {
        ImageFormat::R8 => Some(
            image
                .bytes
                .iter()
                .flat_map(|value| [*value, *value, *value, 255])
                .collect(),
        ),
        ImageFormat::R8G8 => Some(
            image
                .bytes
                .chunks_exact(2)
                .flat_map(|chunk| [chunk[0], chunk[0], chunk[0], chunk[1]])
                .collect(),
        ),
        ImageFormat::R8G8B8 => Some(
            image
                .bytes
                .chunks_exact(3)
                .flat_map(|chunk| [chunk[0], chunk[1], chunk[2], 255])
                .collect(),
        ),
        ImageFormat::R8G8B8A8 => Some(image.bytes.clone()),
        ImageFormat::R16
        | ImageFormat::R16G16
        | ImageFormat::R16G16B16
        | ImageFormat::R16G16B16A16
        | ImageFormat::R32G32B32Float
        | ImageFormat::R32G32B32A32Float => None,
    }
}

#[derive(Resource)]
struct MainWorldReceiver(Receiver<Vec<u8>>);

#[derive(Resource)]
struct RenderWorldSender(Sender<Vec<u8>>);

pub struct ImageCopyPlugin;

impl Plugin for ImageCopyPlugin {
    fn build(&self, app: &mut App) {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let render_app = app
            .insert_resource(MainWorldReceiver(receiver))
            .sub_app_mut(RenderApp);

        let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();
        graph.add_node(ImageCopy, ImageCopyDriver);
        graph.add_node_edge(bevy::render::graph::CameraDriverLabel, ImageCopy);

        render_app
            .insert_resource(RenderWorldSender(sender))
            .add_systems(ExtractSchedule, image_copy_extract)
            .add_systems(
                Render,
                (
                    apply_mtoon_phase_order
                        .after(RenderSystems::Queue)
                        .before(RenderSystems::PhaseSort),
                    receive_image_from_buffer.after(RenderSystems::Render),
                ),
            );
    }
}

fn apply_mtoon_phase_order(
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    orders: Query<&BevyMtoonPhaseOrder>,
) {
    for phase in phases.values_mut() {
        for item in &mut phase.items {
            if let Ok(order) = orders.get(item.entity.0) {
                item.distance += order.0 as f32 * 0.000001;
            }
        }
    }
}

fn setup_render_target(
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    render_device: &Res<RenderDevice>,
    scene_controller: &mut ResMut<SceneController>,
) -> RenderTarget {
    let size = Extent3d {
        width: scene_controller.width,
        height: scene_controller.height,
        depth_or_array_layers: 1,
    };
    let mut render_target_image =
        Image::new_target_texture(size.width, size.height, TextureFormat::Rgba8Unorm, None);
    render_target_image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let render_target_image_handle = images.add(render_target_image);
    commands.spawn(ImageCopier::new(
        render_target_image_handle.clone(),
        size,
        render_device,
    ));
    RenderTarget::Image(render_target_image_handle.into())
}

pub struct CaptureFramePlugin;

impl Plugin for CaptureFramePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, update);
    }
}

#[derive(Clone, Default, Resource)]
struct ImageCopiers(Vec<ImageCopier>);

#[derive(Clone, Component)]
struct ImageCopier {
    buffer: Buffer,
    enabled: Arc<AtomicBool>,
    src_image: Handle<Image>,
}

impl ImageCopier {
    fn new(src_image: Handle<Image>, size: Extent3d, render_device: &RenderDevice) -> Self {
        let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(size.width as usize * 4);
        let buffer = render_device.create_buffer(&BufferDescriptor {
            label: Some("vrm-rs bevy render parity readback"),
            size: padded_bytes_per_row as u64 * size.height as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            src_image,
            enabled: Arc::new(AtomicBool::new(true)),
        }
    }
}

fn image_copy_extract(mut commands: Commands, image_copiers: Extract<Query<&ImageCopier>>) {
    commands.insert_resource(ImageCopiers(image_copiers.iter().cloned().collect()));
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, RenderLabel)]
struct ImageCopy;

#[derive(Default)]
struct ImageCopyDriver;

impl render_graph::Node for ImageCopyDriver {
    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let Some(image_copiers) = world.get_resource::<ImageCopiers>() else {
            return Ok(());
        };
        let Some(gpu_images) =
            world.get_resource::<RenderAssets<bevy::render::texture::GpuImage>>()
        else {
            return Ok(());
        };
        for image_copier in &image_copiers.0 {
            if !image_copier.enabled.load(Ordering::Relaxed) {
                continue;
            }
            let Some(src_image) = gpu_images.get(&image_copier.src_image) else {
                continue;
            };
            let mut encoder = render_context
                .render_device()
                .create_command_encoder(&CommandEncoderDescriptor::default());
            let block_dimensions = src_image.texture_format.block_dimensions();
            let block_size = src_image.texture_format.block_copy_size(None).unwrap_or(4);
            let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(
                (src_image.size.width as usize / block_dimensions.0 as usize) * block_size as usize,
            );
            encoder.copy_texture_to_buffer(
                src_image.texture.as_image_copy(),
                TexelCopyBufferInfo {
                    buffer: &image_copier.buffer,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row as u32),
                        rows_per_image: None,
                    },
                },
                src_image.size,
            );
            let render_queue = world.resource::<RenderQueue>();
            render_queue.submit(std::iter::once(encoder.finish()));
        }
        Ok(())
    }
}

fn receive_image_from_buffer(
    image_copiers: Res<ImageCopiers>,
    render_device: Res<RenderDevice>,
    sender: Res<RenderWorldSender>,
) {
    for image_copier in &image_copiers.0 {
        if !image_copier.enabled.load(Ordering::Relaxed) {
            continue;
        }
        let buffer_slice = image_copier.buffer.slice(..);
        let (sender_once, receiver_once) = crossbeam_channel::bounded(1);
        buffer_slice.map_async(MapMode::Read, move |result| {
            let _ = sender_once.send(result);
        });
        if render_device.poll(PollType::wait_indefinitely()).is_err() {
            continue;
        }
        if receiver_once.recv().ok().and_then(Result::ok).is_none() {
            continue;
        }
        let _ = sender.0.send(buffer_slice.get_mapped_range().to_vec());
        image_copier.buffer.unmap();
    }
}

fn update(
    receiver: Res<MainWorldReceiver>,
    options: Res<CaptureOptions>,
    sender: Res<CaptureSender>,
    mut scene_controller: ResMut<SceneController>,
    mut app_exit_writer: MessageWriter<AppExit>,
) {
    let SceneState::Render(frame_count) = scene_controller.state;
    if frame_count > 0 {
        while receiver.0.try_recv().is_ok() {}
        scene_controller.state = SceneState::Render(frame_count - 1);
        return;
    }

    let mut image_data = Vec::new();
    while let Ok(data) = receiver.0.try_recv() {
        image_data = data;
    }
    if image_data.is_empty() {
        return;
    }

    let result = write_capture(&options, &image_data).map_err(|error| error.to_string());
    let _ = sender.0.send(result);
    app_exit_writer.write(AppExit::Success);
}

fn write_capture(
    options: &CaptureOptions,
    image_data: &[u8],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let row_bytes = options.width as usize * 4;
    let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes);
    let rgba = if row_bytes == aligned_row_bytes {
        image_data.to_vec()
    } else {
        image_data
            .chunks(aligned_row_bytes)
            .take(options.height as usize)
            .flat_map(|row| row[..row_bytes.min(row.len())].iter().copied())
            .collect()
    };

    if let Some(parent) = options.out.parent() {
        fs::create_dir_all(parent)?;
    }
    let effective_lighting = mtoon_lighting_values(options);
    let artifact = json!({
        "generator": "vrm-rs examples/bevy_render_capture.rs",
        "fixture": options.fixture.to_string_lossy(),
        "width": options.width,
        "height": options.height,
        "disableOutlines": options.disable_outlines,
        "expressions": options.expressions,
        "camera": { "y": options.camera_y, "z": options.camera_z, "targetY": options.target_y },
        "mtoonLighting": {
            "exposure": options.mtoon_exposure,
            "ambientBase": options.mtoon_ambient_base,
            "ambientGiScale": options.mtoon_ambient_gi_scale,
            "pbrAmbient": options.pbr_ambient,
            "directLightScale": options.direct_light_scale,
            "directionalColor": [
                options.directional_r,
                options.directional_g,
                options.directional_b
            ],
            "lightAccumulation": options.mtoon_light_accumulation.as_str(),
            "effective": {
                "exposure": effective_lighting[0],
                "ambientBase": effective_lighting[1],
                "ambientGiScale": effective_lighting[2],
                "pbrAmbient": effective_lighting[3]
            },
            "time": options.mtoon_time
        },
        "format": "rgba8",
        "rgba": rgba,
    });
    fs::write(
        &options.out,
        format!("{}\n", serde_json::to_string_pretty(&artifact)?),
    )?;
    if let Some(path) = &options.png_out {
        write_png(path, options.width, options.height, &rgba)?;
    }
    Ok(())
}

fn write_png(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    image::save_buffer(path, rgba, width, height, image::ColorType::Rgba8)?;
    Ok(())
}
