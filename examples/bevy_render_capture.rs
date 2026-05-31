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
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use vrm_adapter::{MtoonLightAccumulation as AdapterMtoonLightAccumulation, MtoonLightingConfig};
use vrm_core::{OutlineWidthMode, TextureTransform2d};
use vrm_io::{
    CpuRgba8Image, GltfExpressionRenderEffects, GltfMagFilter, GltfMaterialShadingOptions,
    GltfMaterialShadingPlan, GltfMaterialTextureBinding, GltfMaterialTextureBindingPlan,
    GltfMaterialTextureColorSpace, GltfMaterialTextureFallback, GltfMaterialTextureSlot,
    GltfMaterialUvTransforms, GltfMinFilter, GltfPrimitiveData, GltfSamplerData, GltfWrapMode,
    ImageData, LoadedVrm, Rgba8SamplingOrigin, generate_rgba_mip_chain, image_data_to_rgba8,
    load_vrm_from_path, transform_tex_coord_0,
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
    #[arg(long, value_enum, default_value_t = MtoonLightAccumulation::ThreeVrm)]
    mtoon_light_accumulation: MtoonLightAccumulation,
    #[arg(long, default_value_t = 0.0)]
    mtoon_time: f32,
    #[arg(long, value_enum, default_value_t = CaptureBackground::OpaqueBlack)]
    background: CaptureBackground,
    #[arg(long)]
    disable_outlines: bool,
    #[arg(long, default_value_t = 1.0)]
    outline_width_scale: f32,
    #[arg(long)]
    disable_normal_maps: bool,
    #[arg(long, value_enum, default_value_t = NormalMapMode::GeneratedTangents)]
    normal_map_mode: NormalMapMode,
    #[arg(long)]
    mtoon_v0_compat_shade: bool,
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
        AdapterMtoonLightAccumulation::from(self).as_str()
    }
}

impl From<MtoonLightAccumulation> for AdapterMtoonLightAccumulation {
    fn from(value: MtoonLightAccumulation) -> Self {
        match value {
            MtoonLightAccumulation::Tuned => Self::Tuned,
            MtoonLightAccumulation::ThreeVrm => Self::ThreeVrm,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum NormalMapMode {
    GeneratedTangents,
    Derivative,
}

impl NormalMapMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::GeneratedTangents => "generated-tangents",
            Self::Derivative => "derivative",
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
            .textures
            .iter()
            .map(|texture| {
                loaded
                    .images
                    .get(texture.image)
                    .and_then(|image| bevy_image(image, texture.sampler))
                    .map(|image| images.add(image))
            })
            .collect::<Vec<_>>(),
        linear_images: loaded
            .textures
            .iter()
            .map(|texture| {
                loaded
                    .images
                    .get(texture.image)
                    .and_then(|image| {
                        bevy_image_with_format(image, TextureFormat::Rgba8Unorm, texture.sampler)
                    })
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
        let morph_weights = expression_effects.active_morph_weights(node_index, node, mesh);
        let primitive_context = BevyPrimitiveContext {
            expression_effects: &expression_effects,
            world,
            skin_matrices: skin_matrices.as_deref(),
            options,
            image_handles: &image_handles,
        };
        for primitive in &mesh.primitives {
            let mut shading =
                material_shading(loaded, primitive.material, &expression_effects, options);
            if options.disable_normal_maps {
                shading.normal_scale = 0.0;
            }
            let render_order = material_render_order(loaded, primitive.material);
            let use_derivative_normals = options.normal_map_mode == NormalMapMode::Derivative
                && primitive.tangents.is_empty()
                && shading.normal_scale > 0.0;
            let (mesh, has_tangents) = bevy_mesh(
                primitive,
                &morph_weights,
                world,
                skin_matrices.as_deref(),
                shading.normal_scale > 0.0 && !use_derivative_normals,
            );
            let surface = BevyPrimitive {
                mesh,
                material: BevyPrimitiveMaterial::Mtoon(bevy_mtoon_material(
                    loaded,
                    primitive,
                    shading,
                    &primitive_context,
                    render_depth_bias(render_order),
                    if has_tangents || use_derivative_normals {
                        shading.normal_scale
                    } else {
                        0.0
                    },
                    use_derivative_normals,
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
    expression_effects: &'a GltfExpressionRenderEffects,
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
    let outline_color = context.expression_effects.apply_color4(
        [
            mtoon.outline_color_factor[0],
            mtoon.outline_color_factor[1],
            mtoon.outline_color_factor[2],
            mtoon.outline_lighting_mix_factor,
        ],
        primitive.material,
        "outlineColor",
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
            width: mtoon.outline_width_factor * context.options.outline_width_scale,
            width_mode: mtoon.outline_width_mode,
            capture: context.options,
            width_texture: width_texture.as_ref(),
            width_transform: uv_transforms.outline_width,
        },
    );
    let mut material = bevy_mtoon_material(
        loaded,
        primitive,
        material_shading(
            loaded,
            primitive.material,
            context.expression_effects,
            context.options,
        ),
        context,
        render_depth_bias(material_render_order(loaded, primitive.material) + 1),
        0.0,
        false,
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
            let morphed = primitive.morphed_vertex(index, morph_weights);
            let local_position = morphed.map_or(GVec3::ZERO, |vertex| vertex.position);
            let local_normal = morphed.map_or(GVec3::Z, |vertex| vertex.normal);
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
                        image.sample_green_repeat_linear(
                            transform_tex_coord_0(
                                primitive_tex_coord(primitive, index),
                                settings.width_transform,
                            ),
                            Rgba8SamplingOrigin::BottomLeft,
                        )
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
            let morphed = primitive.morphed_vertex(index, morph_weights);
            let local_position = morphed.map_or(GVec3::ZERO, |vertex| vertex.position);
            let local_normal = morphed.map_or(GVec3::Z, |vertex| vertex.normal);
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
    let tangents = (primitive.tangents.len() == primitive.positions.len()).then(|| {
        primitive
            .tangents
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let tangent = primitive
                    .morphed_vertex(index, morph_weights)
                    .map_or(GVec4::new(1.0, 0.0, 0.0, 1.0), |vertex| vertex.tangent);
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
    width_texture: Option<&'a CpuRgba8Image>,
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
    let morphed = primitive.morphed_vertex(index, morph_weights)?;
    let position = morphed.position;
    let normal = morphed.normal;
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
    let offset = normal * offset_scale;
    Some(transform.transform_point3(position + offset))
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

fn expression_render_effects(
    loaded: &LoadedVrm,
    expression_args: &[String],
) -> Result<GltfExpressionRenderEffects, Box<dyn Error>> {
    Ok(loaded.expression_render_effects(parse_expression_args(expression_args)?)?)
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
            let morphed = primitive.morphed_vertex(index, morph_weights);
            let local_position = morphed.map_or(GVec3::ZERO, |vertex| vertex.position);
            let local_normal = morphed.map_or(GVec3::Z, |vertex| vertex.normal);
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
            let morphed = primitive.morphed_vertex(index, morph_weights);
            let local_position = morphed.map_or(GVec3::ZERO, |vertex| vertex.position);
            let local_normal = morphed.map_or(GVec3::Z, |vertex| vertex.normal);
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
                    let tangent = primitive
                        .morphed_vertex(index, morph_weights)
                        .map_or(GVec4::new(1.0, 0.0, 0.0, 1.0), |vertex| vertex.tangent);
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

fn material_shading(
    loaded: &LoadedVrm,
    material: Option<usize>,
    expression_effects: &GltfExpressionRenderEffects,
    options: &CaptureOptions,
) -> GltfMaterialShadingPlan {
    let mut shading = loaded.material_shading_plan(
        material,
        GltfMaterialShadingOptions {
            v0_compat_shade: options.mtoon_v0_compat_shade,
        },
    );
    shading.base_color = expression_effects.apply_color4(shading.base_color, material, "color");
    if !shading.pbr_fallback {
        shading.shade_color =
            expression_effects.apply_color4(shading.shade_color, material, "shadeColor");
        shading.matcap_factor =
            expression_effects.apply_color3(shading.matcap_factor, material, "matcapColor");
        shading.parametric_rim_color =
            expression_effects.apply_color3(shading.parametric_rim_color, material, "rimColor");
    }
    shading.emissive = expression_effects.apply_color3(shading.emissive, material, "emissionColor");
    shading
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
    material_flags2: BVec4,
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
    material_flags2: BVec4,
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
            material_flags2: material.material_flags2,
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
    shading: GltfMaterialShadingPlan,
    context: &BevyPrimitiveContext<'_>,
    depth_bias: f32,
    normal_scale: f32,
    use_derivative_normals: bool,
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
    let uv_plan = uv_transforms.uniform_plan();
    let image_handles = context.image_handles;
    let texture_plan = loaded
        .material_texture_slots(primitive.material)
        .binding_plan();
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
            if AdapterMtoonLightAccumulation::from(context.options.mtoon_light_accumulation)
                .is_three_vrm()
            {
                1.0
            } else {
                0.0
            },
            if use_derivative_normals { 1.0 } else { 0.0 },
        ),
        material_flags2: BVec4::new(if shading.unlit { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
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
        base_uv_transform: BVec4::from_array(uv_plan.base_transform),
        shade_uv_transform: BVec4::from_array(uv_plan.shade_transform),
        shading_shift_uv_transform: BVec4::from_array(uv_plan.shading_shift_transform),
        normal_uv_transform: BVec4::from_array(uv_plan.normal_transform),
        matcap_uv_transform: BVec4::from_array(uv_plan.matcap_transform),
        rim_uv_transform: BVec4::from_array(uv_plan.rim_transform),
        emissive_uv_transform: BVec4::from_array(uv_plan.emissive_transform),
        occlusion_uv_transform: BVec4::from_array(uv_plan.occlusion_transform),
        uv_animation_mask_uv_transform: BVec4::from_array(uv_plan.uv_animation_mask_transform),
        uv_rotation_a: BVec4::from_array(uv_plan.rotation_a),
        uv_rotation_b: BVec4::from_array(uv_plan.rotation_b),
        uv_animation: BVec4::from_array(uv_plan.uv_animation),
        base_texture: bevy_texture_binding(
            &texture_plan,
            GltfMaterialTextureSlot::Base,
            image_handles,
        ),
        shade_texture: bevy_texture_binding(
            &texture_plan,
            GltfMaterialTextureSlot::Shade,
            image_handles,
        ),
        shading_shift_texture: bevy_texture_binding(
            &texture_plan,
            GltfMaterialTextureSlot::ShadingShift,
            image_handles,
        ),
        matcap_texture: bevy_texture_binding(
            &texture_plan,
            GltfMaterialTextureSlot::Matcap,
            image_handles,
        ),
        rim_texture: bevy_texture_binding(
            &texture_plan,
            GltfMaterialTextureSlot::Rim,
            image_handles,
        ),
        normal_texture: bevy_texture_binding(
            &texture_plan,
            GltfMaterialTextureSlot::Normal,
            image_handles,
        ),
        emissive_texture: bevy_texture_binding(
            &texture_plan,
            GltfMaterialTextureSlot::Emissive,
            image_handles,
        ),
        uv_animation_mask_texture: bevy_texture_binding(
            &texture_plan,
            GltfMaterialTextureSlot::UvAnimationMask,
            image_handles,
        ),
        occlusion_texture: bevy_texture_binding(
            &texture_plan,
            GltfMaterialTextureSlot::Occlusion,
            image_handles,
        ),
        alpha_mode,
        cull_mode,
        depth_write,
        depth_bias,
    }
}

fn bevy_texture_binding(
    plan: &GltfMaterialTextureBindingPlan,
    slot: GltfMaterialTextureSlot,
    handles: &BevyImageHandles,
) -> Handle<Image> {
    let binding = plan
        .binding(slot)
        .expect("MToon texture binding plan must contain every shader slot");
    bevy_texture_handle(binding, handles)
}

fn bevy_texture_handle(
    binding: GltfMaterialTextureBinding,
    handles: &BevyImageHandles,
) -> Handle<Image> {
    let images = match binding.color_space {
        GltfMaterialTextureColorSpace::Srgb => &handles.color_images,
        GltfMaterialTextureColorSpace::Linear => &handles.linear_images,
    };
    binding
        .texture
        .and_then(|texture| images.get(texture))
        .and_then(Clone::clone)
        .unwrap_or_else(|| bevy_fallback_texture(binding.fallback, handles))
}

fn bevy_fallback_texture(
    fallback: GltfMaterialTextureFallback,
    handles: &BevyImageHandles,
) -> Handle<Image> {
    match fallback {
        GltfMaterialTextureFallback::White => handles.white.clone(),
        GltfMaterialTextureFallback::Black => handles.black.clone(),
        GltfMaterialTextureFallback::NeutralNormal => handles.neutral_normal.clone(),
    }
}

fn bevy_mtoon_lighting(options: &CaptureOptions) -> BVec4 {
    let [exposure, ambient_base, ambient_gi_scale, pbr_ambient] = mtoon_lighting_values(options);
    BVec4::new(exposure, ambient_base, ambient_gi_scale, pbr_ambient)
}

fn mtoon_lighting_values(options: &CaptureOptions) -> [f32; 4] {
    MtoonLightingConfig {
        accumulation: options.mtoon_light_accumulation.into(),
        exposure: options.mtoon_exposure,
        ambient_base: options.mtoon_ambient_base,
        ambient_gi_scale: options.mtoon_ambient_gi_scale,
        pbr_ambient: options.pbr_ambient,
    }
    .effective_values()
    .to_array()
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
    let plan = render_capture_scene::capture_material_plan(loaded, material);
    if plan.alpha_mode == render_capture_scene::CaptureMaterialAlphaMode::Blend {
        plan.transparent_order_offset
            .map_or(plan.render_order.max(3000), |offset| 3000 + (1000 - offset))
    } else {
        plan.render_order
    }
}

fn material_phase_order(loaded: &LoadedVrm, material: Option<usize>) -> i32 {
    render_capture_scene::capture_material_plan(loaded, material).phase_order
}

fn material_depth_write(loaded: &LoadedVrm, material: Option<usize>) -> bool {
    render_capture_scene::capture_material_plan(loaded, material).depth_write
}

fn render_depth_bias(_render_order: i32) -> f32 {
    0.0
}

fn material_cull_mode(loaded: &LoadedVrm, material: Option<usize>) -> Option<Face> {
    match render_capture_scene::capture_material_plan(loaded, material).cull_mode {
        render_capture_scene::CaptureMaterialCullMode::Off => None,
        render_capture_scene::CaptureMaterialCullMode::Front => Some(Face::Front),
        render_capture_scene::CaptureMaterialCullMode::Back => Some(Face::Back),
    }
}

fn material_alpha_mode(loaded: &LoadedVrm, material: Option<usize>) -> AlphaMode {
    let plan = render_capture_scene::capture_material_plan(loaded, material);
    match plan.alpha_mode {
        render_capture_scene::CaptureMaterialAlphaMode::Opaque => AlphaMode::Opaque,
        render_capture_scene::CaptureMaterialAlphaMode::Mask => AlphaMode::Mask(plan.alpha_cutoff),
        render_capture_scene::CaptureMaterialAlphaMode::Blend => AlphaMode::Blend,
    }
}

fn material_uv_transforms(
    loaded: &LoadedVrm,
    material: Option<usize>,
    mtoon_time: f32,
    expression_effects: &GltfExpressionRenderEffects,
) -> GltfMaterialUvTransforms {
    let transforms = loaded.material_uv_transforms(material, mtoon_time);
    expression_effects.apply_uv_transforms(transforms, material)
}

fn material_outline_width_image(
    loaded: &LoadedVrm,
    material: Option<usize>,
) -> Option<CpuRgba8Image> {
    loaded
        .material_texture_slots(material)
        .outline_width
        .and_then(|texture| sampled_image_for_texture(loaded, texture))
}

fn sampled_image_for_texture(loaded: &LoadedVrm, texture: usize) -> Option<CpuRgba8Image> {
    let image = loaded.textures.get(texture)?.image;
    let image = loaded.images.get(image)?;
    CpuRgba8Image::from_image_data(image).ok()
}

fn bevy_image(image: &ImageData, sampler: GltfSamplerData) -> Option<Image> {
    bevy_image_with_format(image, TextureFormat::Rgba8UnormSrgb, sampler)
}

fn bevy_image_with_format(
    image: &ImageData,
    format: TextureFormat,
    sampler: GltfSamplerData,
) -> Option<Image> {
    Some(bevy_image_from_rgba(
        image.width,
        image.height,
        image_data_to_rgba8(image).ok()?,
        format,
        sampler,
    ))
}

fn bevy_image_from_rgba(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    format: TextureFormat,
    sampler: GltfSamplerData,
) -> Image {
    let levels = generate_rgba_mip_chain(width, height, &rgba)
        .expect("texture upload RGBA data should match its dimensions");
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
    image.sampler = ImageSampler::Descriptor(bevy_sampler_descriptor(sampler));
    image
}

fn single_pixel_image(rgba: [u8; 4], format: TextureFormat) -> Image {
    bevy_image_from_rgba(1, 1, rgba.to_vec(), format, GltfSamplerData::default())
}

fn bevy_sampler_descriptor(sampler: GltfSamplerData) -> ImageSamplerDescriptor {
    ImageSamplerDescriptor {
        address_mode_u: bevy_address_mode(sampler.wrap_s),
        address_mode_v: bevy_address_mode(sampler.wrap_t),
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: bevy_mag_filter(sampler.mag_filter),
        min_filter: bevy_min_filter(sampler.min_filter),
        mipmap_filter: bevy_mipmap_filter(sampler.min_filter),
        lod_max_clamp: if sampler.min_filter.uses_mipmaps() {
            32.0
        } else {
            0.0
        },
        ..Default::default()
    }
}

fn bevy_address_mode(mode: GltfWrapMode) -> ImageAddressMode {
    match mode {
        GltfWrapMode::ClampToEdge => ImageAddressMode::ClampToEdge,
        GltfWrapMode::MirroredRepeat => ImageAddressMode::MirrorRepeat,
        GltfWrapMode::Repeat => ImageAddressMode::Repeat,
    }
}

fn bevy_mag_filter(filter: GltfMagFilter) -> ImageFilterMode {
    match filter {
        GltfMagFilter::Nearest => ImageFilterMode::Nearest,
        GltfMagFilter::Linear => ImageFilterMode::Linear,
    }
}

fn bevy_min_filter(filter: GltfMinFilter) -> ImageFilterMode {
    match filter {
        GltfMinFilter::Nearest
        | GltfMinFilter::NearestMipmapNearest
        | GltfMinFilter::NearestMipmapLinear => ImageFilterMode::Nearest,
        GltfMinFilter::Linear
        | GltfMinFilter::LinearMipmapNearest
        | GltfMinFilter::LinearMipmapLinear => ImageFilterMode::Linear,
    }
}

fn bevy_mipmap_filter(filter: GltfMinFilter) -> ImageFilterMode {
    match filter {
        GltfMinFilter::Nearest
        | GltfMinFilter::Linear
        | GltfMinFilter::NearestMipmapNearest
        | GltfMinFilter::LinearMipmapNearest => ImageFilterMode::Nearest,
        GltfMinFilter::NearestMipmapLinear | GltfMinFilter::LinearMipmapLinear => {
            ImageFilterMode::Linear
        }
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
        "outlineWidthScale": options.outline_width_scale,
        "disableNormalMaps": options.disable_normal_maps,
        "normalMapMode": options.normal_map_mode.as_str(),
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
