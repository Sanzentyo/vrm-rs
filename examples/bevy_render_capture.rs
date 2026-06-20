//! Headless Bevy render capture for render-parity experiments.
//!
//! This example renders real `vrm-io` mesh primitives through Bevy's renderer
//! into an offscreen image and writes the shared RGBA JSON artifact consumed by
//! `tools/render-parity/compare-psnr.mjs`.

#[path = "common/render_capture_correction.rs"]
mod render_capture_correction;
#[path = "common/render_capture_imqraw.rs"]
mod render_capture_imqraw;
#[path = "common/render_capture_scene.rs"]
mod render_capture_scene;

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::asset::RenderAssetUsages;
use bevy::camera::{CameraProjection, RenderTarget};
use bevy::core_pipeline::{core_3d::Transparent3d, tonemapping::Tonemapping};
use bevy::ecs::system::SystemParam;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Vec4 as BVec4;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::Extract;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderSystems;
use bevy::render::batching::NoAutomaticBatching;
use bevy::render::batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport};
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_graph::{
    self, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel,
};
use bevy::render::render_phase::ViewSortedRenderPhases;
use bevy::render::render_resource::{
    AsBindGroup, Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, CompareFunction,
    Extent3d, Face, FrontFace, MapMode, PollType, PrimitiveTopology, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError, TexelCopyBufferInfo, TexelCopyBufferLayout,
    TextureDataOrder, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::storage::ShaderStorageBuffer;
use bevy::shader::ShaderRef;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use clap::{Parser, ValueEnum};
use crossbeam_channel::{Receiver, Sender};
use glam::{Mat4, Vec3 as GVec3};
use serde_json::json;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use vrm_adapter::{
    ClipDepthMapping, MtoonLightAccumulation as AdapterMtoonLightAccumulation, MtoonLightingConfig,
    RenderOwnerId, RenderOwnerSampleDrawKey, RenderOwnerSamplePass, RenderOwnerSurfaceKey,
    RendererFrontFace, ReverseZeroToOneDepth, ScreenProjectionBounds, ScreenProjectionSize,
    ScreenTriangleProjection, ZeroToOneDepth, project_triangle_to_screen,
};
use vrm_adapter_bevy::{
    bevy_owner_sample_override_buffer_plan_for_surfaces_and_draw,
    empty_bevy_owner_sample_override_record,
};
use vrm_core::{OutlineWidthMode, TextureTransform2d};
use vrm_io::{
    CpuRgba8Image, GltfExpressionRenderEffects, GltfMagFilter, GltfMaterialRenderExtraOptions,
    GltfMaterialShadingOptions, GltfMaterialShadingPlan, GltfMaterialTextureBinding,
    GltfMaterialTextureBindingPlan, GltfMaterialTextureColorSpace, GltfMaterialTextureFallback,
    GltfMaterialTextureSlot, GltfMinFilter, GltfMtoonLightAccumulation as GltfLightAccumulation,
    GltfNormalMapMode, GltfOutlineScale, GltfOutlineVertexSettings, GltfPrimitiveData,
    GltfSamplerData, GltfWrapMode, ImageData, LoadedVrm, Rgba8SamplingOrigin, fallback_tangent,
    generate_rgba_mip_chain, generate_tangents as generate_gltf_tangents, image_data_to_rgba8,
    load_vrm_from_path,
};

const MTOON_SHADER_ASSET_PATH: &str = "shaders/vrm_mtoon_capture.wgsl";
#[cfg(test)]
const MTOON_SHADER_SOURCE: &str = include_str!("../assets/shaders/vrm_mtoon_capture.wgsl");
const BEVY_PHASE_ORDER_OFFSET_SCALE: f32 = 0.000001;
const BEVY_OWNER_ID_RENDER_ORDER_SORT_SCALE: f32 = 0.01;
const BEVY_OWNER_ID_DRAW_INDEX_SORT_SCALE: f32 = 0.000001;
const OWNER_ID_PHASE_ORDER_OVERLAP_AREA_THRESHOLD: f32 = 4.0;
const OWNER_ID_PHASE_ORDER_DEPTH_TOLERANCE: f32 = 0.001;

fn main() -> Result<(), Box<dyn Error>> {
    let options = CaptureOptions::parse();
    let loaded = load_vrm_from_path(&options.fixture)?;
    let (tx, rx) = crossbeam_channel::bounded(1);

    let mut app = App::new();
    app.insert_resource(options.clone())
        .insert_resource(LoadedResource(loaded))
        .insert_resource(CaptureSender(tx))
        .insert_resource(SceneController::new(
            options.width,
            options.height,
            options.pre_roll_frames,
        ))
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
        .add_systems(Startup, setup);

    if options.diagnostic_render == DiagnosticRender::OwnerId {
        app.sub_app_mut(RenderApp)
            .insert_resource(GpuPreprocessingSupport {
                max_supported_mode: GpuPreprocessingMode::None,
            });
    }

    app.run();

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
    #[arg(long)]
    imqraw_out: Option<PathBuf>,
    #[arg(long)]
    owner_sample_correction_manifest: Option<PathBuf>,
    #[arg(long)]
    apply_owner_sample_readback_replacement: bool,
    #[arg(long, default_value_t = 512)]
    width: u32,
    #[arg(long, default_value_t = 512)]
    height: u32,
    #[arg(long, default_value_t = 40)]
    pre_roll_frames: u32,
    #[arg(long, default_value_t = 1.0)]
    camera_y: f32,
    #[arg(long, default_value_t = 5.0)]
    camera_z: f32,
    #[arg(long, default_value_t = 1.0)]
    target_y: f32,
    #[arg(long, default_value_t = 0.0)]
    screen_jitter_x: f32,
    #[arg(long, default_value_t = 0.0)]
    screen_jitter_y: f32,
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
    #[arg(long)]
    disable_texture_mips: bool,
    #[arg(long)]
    force_nearest_textures: bool,
    #[arg(long, value_enum, default_value_t = NormalMapMode::GeneratedTangents)]
    normal_map_mode: NormalMapMode,
    #[arg(long, default_value_t = 1.0)]
    normal_map_scale: f32,
    #[arg(long)]
    mtoon_v0_compat_shade: bool,
    #[arg(long = "expression")]
    expressions: Vec<String>,
    #[arg(long, value_enum, default_value_t = DiagnosticRender::Shaded)]
    diagnostic_render: DiagnosticRender,
    #[arg(long)]
    disable_owner_id_depth_bias: bool,
    #[arg(long)]
    owner_id_strict_depth_compare: bool,
    #[arg(long)]
    disable_owner_id_phase_order: bool,
    #[arg(long, value_enum, default_value_t = OwnerIdPhaseOrderPolicy::DrawIndex)]
    owner_id_phase_order_policy: OwnerIdPhaseOrderPolicy,
    #[arg(long, value_enum, default_value_t = OwnerIdColorSource::VertexColor)]
    owner_id_color_source: OwnerIdColorSource,
    #[arg(long, value_enum, default_value_t = CaptureFrontFace::Ccw)]
    front_face: CaptureFrontFace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum OwnerIdPhaseOrderPolicy {
    Full,
    Off,
    OverlapArea,
    OverlapTriangle,
    DrawIndex,
    DrawIndexReverse,
    DrawIndexReverseNoPhase,
    DrawIndexNoDepth,
}

impl OwnerIdPhaseOrderPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Off => "off",
            Self::OverlapArea => "overlap-area",
            Self::OverlapTriangle => "overlap-triangle",
            Self::DrawIndex => "draw-index",
            Self::DrawIndexReverse => "draw-index-reverse",
            Self::DrawIndexReverseNoPhase => "draw-index-reverse-no-phase",
            Self::DrawIndexNoDepth => "draw-index-no-depth",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum OwnerIdColorSource {
    VertexColor,
    Uniform,
}

impl OwnerIdColorSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::VertexColor => "vertex-color",
            Self::Uniform => "uniform",
        }
    }
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

impl From<MtoonLightAccumulation> for GltfLightAccumulation {
    fn from(value: MtoonLightAccumulation) -> Self {
        match value {
            MtoonLightAccumulation::Tuned => Self::Tuned,
            MtoonLightAccumulation::ThreeVrm => Self::ThreeVrm,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, ValueEnum)]
enum CaptureFrontFace {
    Ccw,
    Cw,
}

impl CaptureFrontFace {
    fn as_str(self) -> &'static str {
        self.renderer_policy().as_str()
    }

    fn renderer_policy(self) -> RendererFrontFace {
        match self {
            Self::Ccw => RendererFrontFace::Ccw,
            Self::Cw => RendererFrontFace::Cw,
        }
    }

    fn to_bevy(self) -> FrontFace {
        match self {
            Self::Ccw => FrontFace::Ccw,
            Self::Cw => FrontFace::Cw,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum NormalMapMode {
    GeneratedTangents,
    Derivative,
    ViewDerivative,
}

impl NormalMapMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::GeneratedTangents => "generated-tangents",
            Self::Derivative => "derivative",
            Self::ViewDerivative => "view-derivative",
        }
    }
}

impl From<NormalMapMode> for GltfNormalMapMode {
    fn from(value: NormalMapMode) -> Self {
        match value {
            NormalMapMode::GeneratedTangents => Self::GeneratedTangents,
            NormalMapMode::Derivative => Self::Derivative,
            NormalMapMode::ViewDerivative => Self::ViewDerivative,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum DiagnosticRender {
    Shaded,
    Flat,
    BaseFactor,
    BaseColor,
    BaseColorFlipV,
    BaseColorRawSrgb,
    Uv,
    BaseUv,
    OwnerId,
}

impl DiagnosticRender {
    fn as_str(self) -> &'static str {
        match self {
            Self::Shaded => "shaded",
            Self::Flat => "flat",
            Self::BaseFactor => "base-factor",
            Self::BaseColor => "base-color",
            Self::BaseColorFlipV => "base-color-flip-v",
            Self::BaseColorRawSrgb => "base-color-raw-srgb",
            Self::Uv => "uv",
            Self::BaseUv => "base-uv",
            Self::OwnerId => "owner-id",
        }
    }

    fn raw_base_color_filter(self) -> bool {
        matches!(self, Self::BaseColorRawSrgb)
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
        &mut assets.storage_buffers,
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
        bevy_camera_transform(&options),
    ));
}

#[derive(SystemParam)]
struct CaptureAssets<'w> {
    meshes: ResMut<'w, Assets<Mesh>>,
    mtoon_materials: ResMut<'w, Assets<BevyMtoonMaterial>>,
    images: ResMut<'w, Assets<Image>>,
    storage_buffers: ResMut<'w, Assets<ShaderStorageBuffer>>,
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
    storage_buffers: &mut Assets<ShaderStorageBuffer>,
) -> Result<(), Box<dyn Error>> {
    let owner_sample_sentinel = storage_buffers.add(ShaderStorageBuffer::new(
        bytemuck::bytes_of(&empty_bevy_owner_sample_override_record()),
        RenderAssetUsages::default(),
    ));
    let image_handles = BevyImageHandles {
        color_images: loaded
            .textures
            .iter()
            .map(|texture| {
                loaded
                    .images
                    .get(texture.image)
                    .and_then(|image| {
                        bevy_image_with_format(
                            image,
                            TextureFormat::Rgba8UnormSrgb,
                            effective_sampler_data(texture.sampler, options.force_nearest_textures),
                            !options.disable_texture_mips && !options.force_nearest_textures,
                        )
                    })
                    .map(|image| images.add(image))
            })
            .collect::<Vec<_>>(),
        raw_color_images: loaded
            .textures
            .iter()
            .map(|texture| {
                loaded
                    .images
                    .get(texture.image)
                    .and_then(|image| {
                        bevy_image_with_format(
                            image,
                            TextureFormat::Rgba8Unorm,
                            effective_sampler_data(texture.sampler, options.force_nearest_textures),
                            !options.disable_texture_mips && !options.force_nearest_textures,
                        )
                    })
                    .map(|image| images.add(image))
            })
            .collect::<Vec<_>>(),
        raw_base_color_filter: options.diagnostic_render.raw_base_color_filter(),
        linear_images: loaded
            .textures
            .iter()
            .map(|texture| {
                loaded
                    .images
                    .get(texture.image)
                    .and_then(|image| {
                        bevy_image_with_format(
                            image,
                            TextureFormat::Rgba8Unorm,
                            effective_sampler_data(texture.sampler, options.force_nearest_textures),
                            !options.disable_texture_mips && !options.force_nearest_textures,
                        )
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
            .map(|skin| skin.joint_matrices(&loaded.scene, &world_matrices, orientation));
        let morph_weights = expression_effects.active_morph_weights(node_index, node, mesh);
        let primitive_context = BevyPrimitiveContext {
            expression_effects: &expression_effects,
            world,
            skin_matrices: skin_matrices.as_deref(),
            options,
            image_handles: &image_handles,
            owner_sample_overrides: owner_sample_sentinel.clone(),
        };
        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            let material_plan = BevyMaterialPlan::new(loaded, primitive.material);
            let mut shading = loaded.expression_material_shading_plan(
                primitive.material,
                GltfMaterialShadingOptions {
                    v0_compat_shade: options.mtoon_v0_compat_shade,
                },
                &expression_effects,
            );
            if options.disable_normal_maps {
                shading.normal_scale = 0.0;
            } else {
                shading.normal_scale *= options.normal_map_scale;
            }
            let owner_source = OwnerSource {
                node_index,
                mesh_index,
                primitive_index,
                material: primitive.material,
                pass: OwnerPass::Base,
                render_order: material_plan.render_order,
                phase_order: material_plan.mtoon_phase_order,
            };
            let normal_plan =
                primitive.normal_map_plan(shading.normal_scale, options.normal_map_mode.into());
            let (mesh, has_tangents) = bevy_mesh(
                primitive,
                &morph_weights,
                world,
                skin_matrices.as_deref(),
                normal_plan.should_generate_tangents(),
            );
            let material = BevyPrimitiveMaterial::Mtoon(bevy_mtoon_material(
                loaded,
                primitive,
                shading,
                material_plan,
                &primitive_context,
                render_depth_bias(material_plan.render_order),
                BevyNormalMapMaterialPlan::from_normal_plan(normal_plan, has_tangents),
            ));
            let surface = BevyPrimitive {
                mesh,
                transparent_order_offset: bevy_source_order_offset(
                    &material,
                    material_plan.phase_order,
                    0,
                ),
                phase_order_offset_applied: bevy_source_order_offset(
                    &material,
                    material_plan.phase_order,
                    0,
                ),
                material,
                render_order: material_plan.render_order,
                owner_source,
                owner_ids: Vec::new(),
            };
            primitives.push(surface);
            if !options.disable_outlines
                && let Some(outline) = bevy_outline_primitive(
                    loaded,
                    primitive,
                    &morph_weights,
                    &primitive_context,
                    owner_source,
                    material_plan,
                )
            {
                primitives.push(outline);
            }
        }
    }
    primitives.sort_by_key(|primitive| primitive.render_order);
    if options.diagnostic_render == DiagnosticRender::OwnerId {
        assign_owner_id_triangles(&mut primitives, options.owner_id_color_source);
        configure_owner_id_phase_order_offsets(&mut primitives, options);
    } else {
        assign_owner_id_colors(&mut primitives);
    }
    if should_apply_owner_id_depth_bias(options) {
        for primitive in &mut primitives {
            primitive.apply_phase_order_depth_bias();
        }
    }
    configure_owner_id_depth_policy(&mut primitives, options);
    let render_surfaces = render_owner_surfaces(loaded, &primitives);
    if let Some(path) = &options.owner_sample_correction_manifest {
        let correction_plan =
            render_capture_correction::load_owner_sample_correction_manifest(path)?;
        let selection = correction_plan.surface_selection_plan(render_surfaces.iter());
        bind_owner_sample_override_buffers(loaded, &mut primitives, &selection, storage_buffers)?;
    }
    commands.insert_resource(RenderOwnerMetadata {
        diagnostic_owner_ids: diagnostic_owner_ids(loaded, &primitives, options),
        render_surfaces,
    });

    for (draw_index, primitive) in primitives.into_iter().enumerate() {
        let phase_order = owner_id_phase_order_offset(&primitive);
        let sort_distance_override =
            owner_id_sort_distance_override(&primitive, draw_index, options);
        let mesh = meshes.add(primitive.mesh);
        match primitive.material {
            BevyPrimitiveMaterial::Mtoon(material) => {
                let mut entity = commands.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(mtoon_materials.add(material)),
                    BevyMtoonPhaseOrder {
                        offset: phase_order,
                        distance_override: sort_distance_override,
                    },
                    Transform::IDENTITY,
                ));
                if options.diagnostic_render == DiagnosticRender::OwnerId {
                    entity.insert(NoAutomaticBatching);
                }
            }
        }
    }
    Ok(())
}

fn assign_owner_id_colors(primitives: &mut [BevyPrimitive]) {
    for (index, primitive) in primitives.iter_mut().enumerate() {
        match &mut primitive.material {
            BevyPrimitiveMaterial::Mtoon(material) => {
                material.owner_color =
                    BVec4::from_array(owner_id_color(u32::try_from(index + 1).unwrap_or(0)));
            }
        }
    }
}

fn assign_owner_id_triangles(
    primitives: &mut Vec<BevyPrimitive>,
    color_source: OwnerIdColorSource,
) {
    let mut next_id = 1;
    let mut owner_primitives = Vec::new();
    for mut primitive in primitives.drain(..) {
        let original_indices = mesh_indices_u32(&primitive.mesh);
        primitive.owner_ids.clear();
        duplicate_mesh_vertices_in_index_order(&mut primitive.mesh, &original_indices)
            .expect("owner-id diagnostic mesh should be writable before render extraction");
        let vertex_count = primitive.mesh.count_vertices();
        let triangle_count = vertex_count.div_ceil(3);
        for triangle_index in 0..triangle_count {
            let color = owner_id_color(next_id);
            let vertex_color = (color_source == OwnerIdColorSource::VertexColor).then_some(color);
            let Some(mesh) = owner_id_triangle_mesh(&primitive.mesh, triangle_index, vertex_color)
            else {
                continue;
            };
            let indices = original_indices
                .get(triangle_index * 3..triangle_index * 3 + 3)
                .and_then(|slice| <[u32; 3]>::try_from(slice).ok())
                .unwrap_or([0, 0, 0]);
            let mut material = primitive.material.clone();
            material.set_owner_color(match color_source {
                OwnerIdColorSource::VertexColor => BVec4::ZERO,
                OwnerIdColorSource::Uniform => BVec4::from_array(color),
            });
            let phase_order = primitive
                .owner_source
                .phase_order
                .unwrap_or(primitive.render_order);
            let transparent_order_offset =
                bevy_source_order_offset(&material, phase_order, triangle_index as i32);
            owner_primitives.push(BevyPrimitive {
                mesh,
                material,
                render_order: primitive.render_order,
                transparent_order_offset,
                phase_order_offset_applied: transparent_order_offset,
                owner_source: primitive.owner_source,
                owner_ids: vec![OwnerTriangle {
                    id: next_id,
                    triangle: 0,
                    source_triangle: triangle_index,
                    indices,
                }],
            });
            next_id += 1;
        }
    }
    *primitives = owner_primitives;
}

fn owner_id_triangle_mesh(source: &Mesh, triangle: usize, color: Option<[f32; 4]>) -> Option<Mesh> {
    let start = triangle.checked_mul(3)?;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        mesh_positions(source)?.get(start..start + 3)?.to_vec(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        mesh_normals(source)?.get(start..start + 3)?.to_vec(),
    );
    if let Some(tangents) = mesh_tangents(source).and_then(|items| items.get(start..start + 3)) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents.to_vec());
    }
    if let Some(uvs) = mesh_uv0(source).and_then(|items| items.get(start..start + 3)) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec());
    }
    if let Some(color) = color {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, [color; 3].to_vec());
    }
    Some(mesh)
}

fn configure_owner_id_phase_order_offsets(
    primitives: &mut [BevyPrimitive],
    options: &CaptureOptions,
) {
    let policy = if options.disable_owner_id_phase_order {
        OwnerIdPhaseOrderPolicy::Off
    } else {
        options.owner_id_phase_order_policy
    };
    match policy {
        OwnerIdPhaseOrderPolicy::Full | OwnerIdPhaseOrderPolicy::DrawIndexReverse => {
            for primitive in primitives {
                primitive.phase_order_offset_applied = primitive.transparent_order_offset;
            }
        }
        OwnerIdPhaseOrderPolicy::Off
        | OwnerIdPhaseOrderPolicy::DrawIndex
        | OwnerIdPhaseOrderPolicy::DrawIndexReverseNoPhase
        | OwnerIdPhaseOrderPolicy::DrawIndexNoDepth => {
            for primitive in primitives {
                primitive.phase_order_offset_applied = 0.0;
            }
        }
        OwnerIdPhaseOrderPolicy::OverlapArea | OwnerIdPhaseOrderPolicy::OverlapTriangle => {
            let keep_offsets = owner_id_phase_order_overlap_mask(primitives, options, policy);
            for (primitive, keep_offset) in primitives.iter_mut().zip(keep_offsets) {
                primitive.phase_order_offset_applied = owner_id_phase_order_offset_for_policy(
                    policy,
                    primitive.transparent_order_offset,
                    keep_offset,
                );
            }
        }
    }
}

fn owner_id_phase_order_offset_for_policy(
    policy: OwnerIdPhaseOrderPolicy,
    transparent_order_offset: f32,
    keep_overlap_offset: bool,
) -> f32 {
    match policy {
        OwnerIdPhaseOrderPolicy::Full | OwnerIdPhaseOrderPolicy::DrawIndexReverse => {
            transparent_order_offset
        }
        OwnerIdPhaseOrderPolicy::OverlapArea | OwnerIdPhaseOrderPolicy::OverlapTriangle => {
            if keep_overlap_offset {
                transparent_order_offset
            } else {
                0.0
            }
        }
        OwnerIdPhaseOrderPolicy::Off
        | OwnerIdPhaseOrderPolicy::DrawIndex
        | OwnerIdPhaseOrderPolicy::DrawIndexReverseNoPhase
        | OwnerIdPhaseOrderPolicy::DrawIndexNoDepth => 0.0,
    }
}

fn owner_id_phase_order_overlap_mask(
    primitives: &[BevyPrimitive],
    options: &CaptureOptions,
    policy: OwnerIdPhaseOrderPolicy,
) -> Vec<bool> {
    let view_projection = diagnostic_view_projection(options);
    let size = ScreenProjectionSize::from_pixels(options.width, options.height);
    let groups = primitives
        .iter()
        .map(|primitive| {
            let front_face = bevy_primitive_front_face(primitive).renderer_policy();
            let projections = if primitive.needs_source_order_offset() {
                primitive
                    .owner_ids
                    .iter()
                    .filter_map(|owner| {
                        owner_triangle_projection::<ReverseZeroToOneDepth>(
                            &primitive.mesh,
                            owner.triangle,
                            view_projection,
                            size,
                            front_face,
                        )
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            OwnerIdPhaseOrderProjectionGroup {
                render_order: primitive.render_order,
                material: primitive.owner_source.material,
                transparent_order_offset: primitive.transparent_order_offset,
                projections,
            }
        })
        .collect::<Vec<_>>();
    owner_id_phase_order_overlap_mask_from_groups(&groups, policy)
}

fn configure_owner_id_depth_policy(primitives: &mut [BevyPrimitive], options: &CaptureOptions) {
    if options.diagnostic_render == DiagnosticRender::OwnerId
        && !options.disable_owner_id_phase_order
        && options.owner_id_phase_order_policy == OwnerIdPhaseOrderPolicy::DrawIndexNoDepth
    {
        for primitive in primitives {
            primitive.material.set_depth_write(false);
            primitive.material.set_depth_bias(0.0);
        }
    }
}

fn owner_id_phase_order_overlap_mask_from_groups(
    groups: &[OwnerIdPhaseOrderProjectionGroup],
    policy: OwnerIdPhaseOrderPolicy,
) -> Vec<bool> {
    let mut keep_offsets = vec![false; groups.len()];
    for (left_index, left) in groups.iter().enumerate() {
        if !left.has_phase_offset_candidates() {
            continue;
        }
        for (right_index, right) in groups.iter().enumerate().skip(left_index + 1) {
            if !right.has_phase_offset_candidates()
                || left.render_order != right.render_order
                || left.material != right.material
            {
                continue;
            }
            if owner_id_phase_order_groups_overlap(left, right, policy) {
                keep_offsets[left_index] = true;
                keep_offsets[right_index] = true;
            }
        }
    }
    keep_offsets
}

fn owner_id_phase_order_groups_overlap(
    left: &OwnerIdPhaseOrderProjectionGroup,
    right: &OwnerIdPhaseOrderProjectionGroup,
    policy: OwnerIdPhaseOrderPolicy,
) -> bool {
    left.projections.iter().any(|left_projection| {
        right.projections.iter().any(|right_projection| {
            owner_id_phase_order_projections_overlap(left_projection, right_projection, policy)
                && (left_projection.webgl_depth - right_projection.webgl_depth).abs()
                    <= OWNER_ID_PHASE_ORDER_DEPTH_TOLERANCE
        })
    })
}

fn owner_id_phase_order_projections_overlap(
    left: &ScreenTriangleProjection,
    right: &ScreenTriangleProjection,
    policy: OwnerIdPhaseOrderPolicy,
) -> bool {
    match policy {
        OwnerIdPhaseOrderPolicy::Full
        | OwnerIdPhaseOrderPolicy::Off
        | OwnerIdPhaseOrderPolicy::DrawIndex
        | OwnerIdPhaseOrderPolicy::DrawIndexReverse
        | OwnerIdPhaseOrderPolicy::DrawIndexReverseNoPhase
        | OwnerIdPhaseOrderPolicy::DrawIndexNoDepth => false,
        OwnerIdPhaseOrderPolicy::OverlapArea => {
            screen_bounds_overlap_area(left.bounds, right.bounds)
                >= OWNER_ID_PHASE_ORDER_OVERLAP_AREA_THRESHOLD
        }
        OwnerIdPhaseOrderPolicy::OverlapTriangle => {
            screen_triangles_have_area_overlap(left.screen, right.screen)
        }
    }
}

#[derive(Clone, Debug)]
struct OwnerIdPhaseOrderProjectionGroup {
    render_order: i32,
    material: Option<usize>,
    transparent_order_offset: f32,
    projections: Vec<ScreenTriangleProjection>,
}

impl OwnerIdPhaseOrderProjectionGroup {
    fn has_phase_offset_candidates(&self) -> bool {
        self.transparent_order_offset.abs() > f32::EPSILON && !self.projections.is_empty()
    }
}

fn screen_bounds_overlap_area(left: ScreenProjectionBounds, right: ScreenProjectionBounds) -> f32 {
    let width = (left.max_x.min(right.max_x) - left.min_x.max(right.min_x)).max(0.0);
    let height = (left.max_y.min(right.max_y) - left.min_y.max(right.min_y)).max(0.0);
    width * height
}

fn screen_triangles_have_area_overlap(left: [[f32; 2]; 3], right: [[f32; 2]; 3]) -> bool {
    const EPSILON: f32 = 1.0e-4;
    triangle_intersection_area(left, right) > EPSILON
}

fn triangle_intersection_area(subject: [[f32; 2]; 3], clip: [[f32; 2]; 3]) -> f32 {
    const EPSILON: f32 = 1.0e-6;
    let clip_area = orient_2d(clip[0], clip[1], clip[2]);
    if clip_area.abs() <= EPSILON {
        return 0.0;
    }
    let winding = clip_area.signum();
    let mut polygon = subject.to_vec();
    for [edge_start, edge_end] in triangle_edges(clip) {
        polygon = clip_polygon_to_half_plane(&polygon, edge_start, edge_end, winding);
        if polygon.len() < 3 {
            return 0.0;
        }
    }
    polygon_area(&polygon).abs()
}

fn clip_polygon_to_half_plane(
    polygon: &[[f32; 2]],
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    winding: f32,
) -> Vec<[f32; 2]> {
    const EPSILON: f32 = 1.0e-6;
    let Some(mut previous) = polygon.last().copied() else {
        return Vec::new();
    };
    let mut previous_inside = orient_2d(edge_start, edge_end, previous) * winding >= -EPSILON;
    let mut clipped = Vec::with_capacity(polygon.len() + 1);
    for &current in polygon {
        let current_inside = orient_2d(edge_start, edge_end, current) * winding >= -EPSILON;
        if current_inside != previous_inside
            && let Some(intersection) =
                line_segment_intersection_with_line(previous, current, edge_start, edge_end)
        {
            clipped.push(intersection);
        }
        if current_inside {
            clipped.push(current);
        }
        previous = current;
        previous_inside = current_inside;
    }
    clipped
}

fn line_segment_intersection_with_line(
    start: [f32; 2],
    end: [f32; 2],
    line_start: [f32; 2],
    line_end: [f32; 2],
) -> Option<[f32; 2]> {
    const EPSILON: f32 = 1.0e-6;
    let segment = [end[0] - start[0], end[1] - start[1]];
    let line = [line_end[0] - line_start[0], line_end[1] - line_start[1]];
    let denominator = cross_2d(segment, line);
    if denominator.abs() <= EPSILON {
        return None;
    }
    let to_line = [line_start[0] - start[0], line_start[1] - start[1]];
    let t = cross_2d(to_line, line) / denominator;
    Some([start[0] + t * segment[0], start[1] + t * segment[1]])
}

fn triangle_edges(triangle: [[f32; 2]; 3]) -> impl Iterator<Item = [[f32; 2]; 2]> {
    [
        [triangle[0], triangle[1]],
        [triangle[1], triangle[2]],
        [triangle[2], triangle[0]],
    ]
    .into_iter()
}

fn polygon_area(polygon: &[[f32; 2]]) -> f32 {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(left, right)| cross_2d(*left, *right))
        .sum::<f32>()
        * 0.5
}

fn orient_2d(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    cross_2d([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]])
}

fn cross_2d(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

fn duplicate_mesh_vertices_in_index_order(
    mesh: &mut Mesh,
    indices: &[u32],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if mesh.indices().is_none() {
        return Ok(());
    }
    for (_, values) in mesh.attributes_mut() {
        duplicate_attribute_values_in_index_order(values, indices)?;
    }
    mesh.remove_indices();
    Ok(())
}

fn duplicate_attribute_values_in_index_order(
    values: &mut VertexAttributeValues,
    indices: &[u32],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    fn duplicate<T: Copy>(
        values: &[T],
        indices: &[u32],
    ) -> Result<Vec<T>, Box<dyn Error + Send + Sync>> {
        indices
            .iter()
            .map(|index| {
                values
                    .get(*index as usize)
                    .copied()
                    .ok_or_else(|| format!("mesh index {index} is out of bounds").into())
            })
            .collect()
    }

    #[expect(
        clippy::match_same_arms,
        reason = "Each VertexAttributeValues variant has distinct vertex-format semantics."
    )]
    match values {
        VertexAttributeValues::Float32(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Sint32(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Uint32(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Float32x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Sint32x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Uint32x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Float32x3(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Sint32x3(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Uint32x3(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Float32x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Sint32x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Uint32x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Sint16x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Snorm16x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Uint16x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Unorm16x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Sint16x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Snorm16x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Uint16x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Unorm16x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Sint8x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Snorm8x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Uint8x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Unorm8x2(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Sint8x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Snorm8x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Uint8x4(items) => *items = duplicate(items, indices)?,
        VertexAttributeValues::Unorm8x4(items) => *items = duplicate(items, indices)?,
    }
    Ok(())
}

fn mesh_indices_u32(mesh: &Mesh) -> Vec<u32> {
    match mesh.indices() {
        Some(Indices::U16(indices)) => indices.iter().map(|index| u32::from(*index)).collect(),
        Some(Indices::U32(indices)) => indices.clone(),
        None => (0..u32::try_from(mesh.count_vertices()).unwrap_or(0)).collect(),
    }
}

fn render_owner_surfaces(
    loaded: &LoadedVrm,
    primitives: &[BevyPrimitive],
) -> Vec<RenderOwnerSurfaceKey> {
    primitives
        .iter()
        .flat_map(|primitive| {
            let source = primitive.owner_source;
            let material_name = material_name(loaded, source.material);
            (0..mesh_indices_u32(&primitive.mesh).len() / 3).filter_map(move |triangle| {
                Some(RenderOwnerSurfaceKey::new(
                    material_name?,
                    u64::try_from(triangle).ok()?,
                ))
            })
        })
        .collect()
}

fn bind_owner_sample_override_buffers(
    loaded: &LoadedVrm,
    primitives: &mut [BevyPrimitive],
    selection: &vrm_adapter::RenderOwnerSampleSelectionPlan,
    storage_buffers: &mut Assets<ShaderStorageBuffer>,
) -> Result<(), Box<dyn Error>> {
    for primitive in primitives {
        let surfaces = owner_sample_surfaces_for_primitive(loaded, primitive);
        let draw = owner_sample_draw_key(primitive.owner_source)?;
        let plan = bevy_owner_sample_override_buffer_plan_for_surfaces_and_draw(
            selection,
            surfaces.iter(),
            &draw,
        )?;
        let handle = storage_buffers.add(ShaderStorageBuffer::new(
            plan.bytes(),
            RenderAssetUsages::default(),
        ));
        primitive.material.set_owner_sample_overrides(handle);
    }
    Ok(())
}

fn owner_sample_surfaces_for_primitive(
    loaded: &LoadedVrm,
    primitive: &BevyPrimitive,
) -> Vec<RenderOwnerSurfaceKey> {
    let source = primitive.owner_source;
    let Some(material_name) = material_name(loaded, source.material) else {
        return Vec::new();
    };
    (0..mesh_indices_u32(&primitive.mesh).len() / 3)
        .filter_map(|triangle| {
            Some(RenderOwnerSurfaceKey::new(
                material_name,
                u64::try_from(triangle).ok()?,
            ))
        })
        .collect()
}

fn owner_sample_draw_key(source: OwnerSource) -> Result<RenderOwnerSampleDrawKey, Box<dyn Error>> {
    Ok(RenderOwnerSampleDrawKey::new(
        u64::try_from(source.node_index)?,
        u64::try_from(source.mesh_index)?,
        u64::try_from(source.primitive_index)?,
        render_owner_sample_pass(source.pass),
    ))
}

fn render_owner_sample_pass(pass: OwnerPass) -> RenderOwnerSamplePass {
    match pass {
        OwnerPass::Base => RenderOwnerSamplePass::Base,
        OwnerPass::Outline => RenderOwnerSamplePass::Outline,
    }
}

fn diagnostic_owner_ids(
    loaded: &LoadedVrm,
    primitives: &[BevyPrimitive],
    options: &CaptureOptions,
) -> Vec<serde_json::Value> {
    let view_projection = diagnostic_view_projection(options);
    let reference_view_projection = diagnostic_reference_view_projection(options);
    let size = ScreenProjectionSize::from_pixels(options.width, options.height);
    primitives
        .iter()
        .enumerate()
        .flat_map(|(draw_index, primitive)| {
            let front_face = bevy_primitive_front_face(primitive).renderer_policy();
            primitive.owner_ids.iter().map(move |owner| {
                let source = primitive.owner_source;
                let projection = owner_triangle_projection::<ReverseZeroToOneDepth>(
                    &primitive.mesh,
                    owner.triangle,
                    view_projection,
                    size,
                    front_face,
                );
                let reference_projection = owner_triangle_projection::<ZeroToOneDepth>(
                    &primitive.mesh,
                    owner.triangle,
                    reference_view_projection,
                    size,
                    front_face,
                );
                json!({
                    "id": owner.id,
                    "color": owner_id_color_u8(owner.id),
                    "nodeIndex": source.node_index,
                    "nodeName": node_name(loaded, source.node_index),
                    "meshIndex": source.mesh_index,
                    "meshName": mesh_name(loaded, source.mesh_index),
                    "primitiveIndex": source.primitive_index,
                    "materialIndex": source.material,
                    "materialName": material_name(loaded, source.material),
                    "pass": source.pass.as_str(),
                    "renderOrder": source.render_order,
                    "renderPhaseOrder": source.phase_order,
                    "drawIndex": draw_index,
                    "frontFace": bevy_primitive_front_face(primitive).as_str(),
                    "cullMode": bevy_primitive_cull_mode(primitive),
                    "alphaMode": bevy_primitive_alpha_mode(primitive),
                    "alphaCutoff": bevy_primitive_alpha_cutoff(primitive),
                    "depthWrite": bevy_primitive_depth_write(primitive),
                    "depthTest": true,
                    "depthCompare": bevy_primitive_depth_compare(primitive),
                    "blend": bevy_primitive_blend(primitive),
                    "depthBias": bevy_primitive_depth_bias(primitive),
                    "ownerColorSource": options.owner_id_color_source.as_str(),
                    "bevyPhaseOrderOffset": primitive.transparent_order_offset,
                    "bevyPhaseOrderOffsetApplied": owner_id_phase_order_offset(primitive),
                    "bevySortDistanceOverride": owner_id_sort_distance_override(primitive, draw_index, options),
                    "triangle": owner.source_triangle,
                    "sourceTriangle": owner.source_triangle,
                    "bevyLocalTriangle": owner.triangle,
                    "indices": owner.indices,
                    "screen": projection.map(|projection| projection.screen),
                    "screenBounds": projection.map(|projection| json!({
                        "minX": projection.bounds.min_x,
                        "minY": projection.bounds.min_y,
                        "maxX": projection.bounds.max_x,
                        "maxY": projection.bounds.max_y,
                    })),
                    "depth": projection.map(|projection| projection.ndc_depth),
                    "webglDepth": projection.map(|projection| projection.webgl_depth),
                    "depthRange": projection.map(|_| ReverseZeroToOneDepth::DEPTH_RANGE_LABEL),
                    "referenceWebglDepth": reference_projection.map(|projection| projection.webgl_depth),
                    "referenceDepthRange": reference_projection.map(|_| ZeroToOneDepth::DEPTH_RANGE_LABEL),
                    "screenSignedArea": projection.map(|projection| projection.screen_signed_area),
                    "frontFacing": projection.map(|projection| projection.front_facing),
                    "gpuFrontFacing": projection.map(|projection| projection.gpu_front_facing),
                    "visibleByCullPolicy": projection.map(|projection| bevy_visible_by_cull_policy(
                        primitive,
                        projection.gpu_front_facing
                    )),
                })
            })
        })
        .collect()
}

fn bevy_primitive_alpha_mode(primitive: &BevyPrimitive) -> &'static str {
    match &primitive.material {
        BevyPrimitiveMaterial::Mtoon(material) => alpha_mode_name(material.shader_alpha_mode),
    }
}

fn bevy_primitive_alpha_cutoff(primitive: &BevyPrimitive) -> f32 {
    match &primitive.material {
        BevyPrimitiveMaterial::Mtoon(material) => alpha_cutoff(material.shader_alpha_mode),
    }
}

fn bevy_primitive_blend(primitive: &BevyPrimitive) -> bool {
    match &primitive.material {
        BevyPrimitiveMaterial::Mtoon(material) => material.shader_alpha_mode != AlphaMode::Opaque,
    }
}

fn bevy_primitive_cull_mode(primitive: &BevyPrimitive) -> &'static str {
    match &primitive.material {
        BevyPrimitiveMaterial::Mtoon(material) => face_name(material.cull_mode),
    }
}

fn bevy_primitive_front_face(primitive: &BevyPrimitive) -> CaptureFrontFace {
    match &primitive.material {
        BevyPrimitiveMaterial::Mtoon(material) => material.front_face,
    }
}

fn bevy_primitive_depth_write(primitive: &BevyPrimitive) -> bool {
    match &primitive.material {
        BevyPrimitiveMaterial::Mtoon(material) => material.depth_write,
    }
}

fn bevy_primitive_depth_compare(primitive: &BevyPrimitive) -> &'static str {
    match &primitive.material {
        BevyPrimitiveMaterial::Mtoon(material) => depth_compare_name(material.depth_compare),
    }
}

fn bevy_primitive_depth_bias(primitive: &BevyPrimitive) -> f32 {
    match &primitive.material {
        BevyPrimitiveMaterial::Mtoon(material) => material.depth_bias,
    }
}

fn should_apply_owner_id_depth_bias(options: &CaptureOptions) -> bool {
    options.diagnostic_render != DiagnosticRender::OwnerId || !options.disable_owner_id_depth_bias
}

fn owner_id_phase_order_offset(primitive: &BevyPrimitive) -> f32 {
    primitive.phase_order_offset_applied
}

fn owner_id_sort_distance_override(
    primitive: &BevyPrimitive,
    draw_index: usize,
    options: &CaptureOptions,
) -> Option<f32> {
    (options.diagnostic_render == DiagnosticRender::OwnerId
        && !options.disable_owner_id_phase_order
        && matches!(
            options.owner_id_phase_order_policy,
            OwnerIdPhaseOrderPolicy::DrawIndex
                | OwnerIdPhaseOrderPolicy::DrawIndexReverse
                | OwnerIdPhaseOrderPolicy::DrawIndexReverseNoPhase
                | OwnerIdPhaseOrderPolicy::DrawIndexNoDepth
        )
        && primitive.material.needs_source_order_offset())
    .then(|| match options.owner_id_phase_order_policy {
        OwnerIdPhaseOrderPolicy::DrawIndexReverse
        | OwnerIdPhaseOrderPolicy::DrawIndexReverseNoPhase => {
            owner_id_draw_index_reverse_sort_distance(primitive.render_order, draw_index)
        }
        _ => owner_id_draw_index_sort_distance(primitive.render_order, draw_index),
    })
}

fn alpha_mode_name(mode: AlphaMode) -> &'static str {
    match mode {
        AlphaMode::Opaque => "opaque",
        AlphaMode::Mask(_) => "mask",
        AlphaMode::Blend => "blend",
        AlphaMode::Premultiplied => "premultiplied",
        AlphaMode::Add => "add",
        AlphaMode::Multiply => "multiply",
        AlphaMode::AlphaToCoverage => "alpha-to-coverage",
    }
}

fn depth_compare_name(compare: CompareFunction) -> &'static str {
    match compare {
        CompareFunction::Greater => "greater",
        CompareFunction::GreaterEqual => "greater-equal",
        CompareFunction::Less => "less",
        CompareFunction::LessEqual => "less-equal",
        CompareFunction::Equal => "equal",
        CompareFunction::NotEqual => "not-equal",
        CompareFunction::Always => "always",
        CompareFunction::Never => "never",
    }
}

fn face_name(face: Option<Face>) -> &'static str {
    match face {
        Some(Face::Front) => "front",
        Some(Face::Back) => "back",
        None => "off",
    }
}

fn diagnostic_view_projection(options: &CaptureOptions) -> Mat4 {
    let projection = PerspectiveProjection {
        fov: 30.0_f32.to_radians(),
        aspect_ratio: options.width as f32 / options.height as f32,
        near: 0.1,
        far: 20.0,
        ..default()
    }
    .get_clip_from_view();
    Mat4::from_cols_array(&projection.to_cols_array()) * camera_view(options)
}

fn diagnostic_reference_view_projection(options: &CaptureOptions) -> Mat4 {
    Mat4::perspective_rh(
        30.0_f32.to_radians(),
        options.width as f32 / options.height as f32,
        0.1,
        20.0,
    ) * camera_view(options)
}

fn owner_triangle_projection<D>(
    mesh: &Mesh,
    triangle: usize,
    view_projection: Mat4,
    size: ScreenProjectionSize,
    front_face: RendererFrontFace,
) -> Option<ScreenTriangleProjection>
where
    D: ClipDepthMapping,
{
    let positions = mesh_positions(mesh)?;
    let start = triangle.checked_mul(3)?;
    project_triangle_to_screen::<D>(
        [
            *positions.get(start)?,
            *positions.get(start + 1)?,
            *positions.get(start + 2)?,
        ],
        view_projection,
        size,
        front_face,
    )
}

fn bevy_visible_by_cull_policy(primitive: &BevyPrimitive, gpu_front_facing: bool) -> bool {
    match &primitive.material {
        BevyPrimitiveMaterial::Mtoon(material) => match material.cull_mode {
            None => true,
            Some(Face::Front) => !gpu_front_facing,
            Some(Face::Back) => gpu_front_facing,
        },
    }
}

fn mesh_positions(mesh: &Mesh) -> Option<&[[f32; 3]]> {
    match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(positions) => Some(positions),
        _ => None,
    }
}

fn mesh_normals(mesh: &Mesh) -> Option<&[[f32; 3]]> {
    match mesh.attribute(Mesh::ATTRIBUTE_NORMAL)? {
        VertexAttributeValues::Float32x3(normals) => Some(normals),
        _ => None,
    }
}

fn mesh_tangents(mesh: &Mesh) -> Option<&[[f32; 4]]> {
    match mesh.attribute(Mesh::ATTRIBUTE_TANGENT)? {
        VertexAttributeValues::Float32x4(tangents) => Some(tangents),
        _ => None,
    }
}

fn mesh_uv0(mesh: &Mesh) -> Option<&[[f32; 2]]> {
    match mesh.attribute(Mesh::ATTRIBUTE_UV_0)? {
        VertexAttributeValues::Float32x2(uvs) => Some(uvs),
        _ => None,
    }
}

fn node_name(loaded: &LoadedVrm, node: usize) -> Option<&str> {
    loaded
        .scene
        .node(node)
        .and_then(|node| node.name.as_deref())
}

fn mesh_name(loaded: &LoadedVrm, mesh: usize) -> Option<&str> {
    loaded
        .meshes
        .get(mesh)
        .and_then(|mesh| mesh.name.as_deref())
}

fn material_name(loaded: &LoadedVrm, material: Option<usize>) -> Option<&str> {
    loaded.material_display_name(material)
}

fn owner_id_color(id: u32) -> [f32; 4] {
    RenderOwnerId::new(id).to_rgba_f32()
}

fn owner_id_color_u8(id: u32) -> [u8; 4] {
    RenderOwnerId::new(id).to_rgba_u8()
}

#[derive(Clone)]
struct BevyPrimitiveContext<'a> {
    expression_effects: &'a GltfExpressionRenderEffects,
    world: Mat4,
    skin_matrices: Option<&'a [Mat4]>,
    options: &'a CaptureOptions,
    image_handles: &'a BevyImageHandles,
    owner_sample_overrides: Handle<ShaderStorageBuffer>,
}

#[derive(Clone, Copy, Debug)]
struct BevyNormalMapMaterialPlan {
    scale: f32,
    derivative: bool,
    view_derivative: bool,
}

impl BevyNormalMapMaterialPlan {
    fn from_normal_plan(plan: vrm_io::GltfNormalMapPlan, has_tangents: bool) -> Self {
        Self {
            scale: plan.material_normal_scale(has_tangents),
            derivative: plan.uses_derivative_normals(),
            view_derivative: plan.uses_view_derivative_normals(),
        }
    }

    fn disabled() -> Self {
        Self {
            scale: 0.0,
            derivative: false,
            view_derivative: false,
        }
    }
}

fn bevy_outline_primitive(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    morph_weights: &[f32],
    context: &BevyPrimitiveContext<'_>,
    owner_source: OwnerSource,
    material_plan: BevyMaterialPlan,
) -> Option<BevyPrimitive> {
    let outline =
        loaded.expression_mtoon_outline_plan(primitive.material, context.expression_effects)?;
    let width_texture = loaded.material_outline_width_rgba8_image(primitive.material);
    let uv_transforms = loaded.expression_material_uv_transforms(
        primitive.material,
        context.options.mtoon_time,
        context.expression_effects,
    );
    let mesh = if context.options.diagnostic_render == DiagnosticRender::Shaded {
        bevy_outline_mesh(
            primitive,
            morph_weights,
            context.world,
            context.skin_matrices,
            BevyOutlineMeshSettings {
                width: outline.width_factor * context.options.outline_width_scale,
                width_mode: outline.width_mode,
                capture: context.options,
                width_texture: width_texture.as_ref(),
                width_transform: uv_transforms.outline_width,
            },
        )
    } else {
        bevy_mesh(
            primitive,
            morph_weights,
            context.world,
            context.skin_matrices,
            false,
        )
        .0
    };
    let mut material = bevy_mtoon_material(
        loaded,
        primitive,
        loaded.expression_material_shading_plan(
            primitive.material,
            GltfMaterialShadingOptions {
                v0_compat_shade: context.options.mtoon_v0_compat_shade,
            },
            context.expression_effects,
        ),
        material_plan,
        context,
        render_depth_bias(material_plan.render_order.saturating_add(1)),
        BevyNormalMapMaterialPlan::disabled(),
    );
    material.outline_color = BVec4::from_array(outline.color);
    material.shader_alpha_mode = AlphaMode::Opaque;
    material.cull_mode = Some(Face::Front);
    material.pipeline.w = 0.0;
    let material = BevyPrimitiveMaterial::Mtoon(material);
    let transparent_order_offset =
        bevy_source_order_offset(&material, material_plan.phase_order.saturating_add(1), 0);
    Some(BevyPrimitive {
        mesh,
        transparent_order_offset,
        phase_order_offset_applied: transparent_order_offset,
        material,
        render_order: material_plan.render_order.saturating_add(1),
        owner_source: OwnerSource {
            pass: OwnerPass::Outline,
            render_order: material_plan.render_order.saturating_add(1),
            phase_order: material_plan
                .mtoon_phase_order
                .map(|phase_order| phase_order.saturating_add(1)),
            ..owner_source
        },
        owner_ids: Vec::new(),
    })
}

fn bevy_outline_mesh(
    primitive: &GltfPrimitiveData,
    morph_weights: &[f32],
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    settings: BevyOutlineMeshSettings<'_>,
) -> Mesh {
    let outline_scale = GltfOutlineScale::new(
        settings.width_mode,
        camera_view(settings.capture),
        projection_y_scale(),
    );
    let outline_vertices = primitive
        .outline_vertices(
            morph_weights,
            GltfOutlineVertexSettings {
                base_width: settings.width,
                scale: outline_scale,
                width_texture: settings.width_texture,
                width_transform: settings.width_transform,
                width_texture_origin: Rgba8SamplingOrigin::BottomLeft,
            },
            world,
            skin_matrices,
        )
        .expect("iterating over primitive positions should keep vertex indices valid");
    let positions = outline_vertices
        .iter()
        .map(|vertex| vertex.position.to_array())
        .collect::<Vec<_>>();
    let normals = outline_vertices
        .iter()
        .map(|vertex| vertex.normal.to_array())
        .collect::<Vec<_>>();
    let tangents = (primitive.tangents.len() == primitive.positions.len()).then(|| {
        outline_vertices
            .iter()
            .map(|vertex| vertex.tangent.to_array())
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
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, primitive.tex_coords_0_or_defaults());
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

struct BevyPrimitive {
    mesh: Mesh,
    material: BevyPrimitiveMaterial,
    render_order: i32,
    transparent_order_offset: f32,
    phase_order_offset_applied: f32,
    owner_source: OwnerSource,
    owner_ids: Vec<OwnerTriangle>,
}

impl BevyPrimitive {
    fn needs_source_order_offset(&self) -> bool {
        self.material.needs_source_order_offset()
    }

    fn apply_phase_order_depth_bias(&mut self) {
        if self.needs_source_order_offset() {
            self.material
                .set_depth_bias(self.phase_order_offset_applied);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnerPass {
    Base,
    Outline,
}

impl OwnerPass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Outline => "outline",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BevyMaterialPlan {
    render_order: i32,
    phase_order: i32,
    mtoon_phase_order: Option<i32>,
    alpha_mode: AlphaMode,
    cull_mode: Option<Face>,
    depth_write: bool,
}

impl BevyMaterialPlan {
    fn new(loaded: &LoadedVrm, material: Option<usize>) -> Self {
        let plan = render_capture_scene::capture_material_plan(loaded, material);
        let mtoon_phase_order = material.and_then(|index| {
            loaded
                .model()
                .document()
                .materials
                .get(index)
                .and_then(|material| material.mtoon.is_present().then_some(plan.phase_order))
        });
        Self {
            render_order: bevy_render_order_from_plan(plan),
            phase_order: plan.phase_order,
            mtoon_phase_order,
            alpha_mode: bevy_alpha_mode_from_plan(plan.alpha_mode, plan.alpha_cutoff),
            cull_mode: bevy_cull_mode_from_plan(plan.cull_mode),
            depth_write: plan.depth_write,
        }
    }
}

fn bevy_render_order_from_plan(plan: render_capture_scene::CaptureMaterialPlan) -> i32 {
    plan.render_order
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OwnerSource {
    node_index: usize,
    mesh_index: usize,
    primitive_index: usize,
    material: Option<usize>,
    pass: OwnerPass,
    render_order: i32,
    phase_order: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnerTriangle {
    id: u32,
    triangle: usize,
    source_triangle: usize,
    indices: [u32; 3],
}

#[derive(Clone, Debug, Default, Resource)]
struct RenderOwnerMetadata {
    diagnostic_owner_ids: Vec<serde_json::Value>,
    render_surfaces: Vec<RenderOwnerSurfaceKey>,
}

#[derive(Clone)]
enum BevyPrimitiveMaterial {
    Mtoon(BevyMtoonMaterial),
}

impl BevyPrimitiveMaterial {
    fn needs_source_order_offset(&self) -> bool {
        match self {
            Self::Mtoon(material) => material.needs_source_order_offset(),
        }
    }

    fn set_depth_bias(&mut self, depth_bias: f32) {
        match self {
            Self::Mtoon(material) => {
                material.depth_bias = depth_bias;
            }
        }
    }

    fn set_depth_write(&mut self, depth_write: bool) {
        match self {
            Self::Mtoon(material) => {
                material.depth_write = depth_write;
            }
        }
    }

    fn set_owner_color(&mut self, owner_color: BVec4) {
        match self {
            Self::Mtoon(material) => {
                material.owner_color = owner_color;
            }
        }
    }

    fn set_owner_sample_overrides(&mut self, handle: Handle<ShaderStorageBuffer>) {
        match self {
            Self::Mtoon(material) => {
                material.owner_sample_overrides = handle;
            }
        }
    }
}

#[derive(Clone, Copy, Component, Debug, ExtractComponent)]
struct BevyMtoonPhaseOrder {
    offset: f32,
    distance_override: Option<f32>,
}

struct BevyImageHandles {
    color_images: Vec<Option<Handle<Image>>>,
    raw_color_images: Vec<Option<Handle<Image>>>,
    linear_images: Vec<Option<Handle<Image>>>,
    raw_base_color_filter: bool,
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
    let transformed_vertices = primitive
        .transformed_vertices(morph_weights, world, skin_matrices)
        .expect("iterating over primitive positions should keep vertex indices valid");
    let positions = transformed_vertices
        .iter()
        .map(|vertex| vertex.position.to_array())
        .collect::<Vec<_>>();
    let normals = transformed_vertices
        .iter()
        .map(|vertex| vertex.normal.to_array())
        .collect::<Vec<_>>();
    let tangents = if primitive.tangents.len() == primitive.positions.len() {
        Some(
            transformed_vertices
                .iter()
                .map(|vertex| vertex.tangent.to_array())
                .collect::<Vec<_>>(),
        )
    } else if generate_tangents {
        generate_gltf_tangents(
            &positions,
            &normals,
            &primitive.tex_coords_0_or_defaults(),
            &primitive.indices,
        )
        .map(|generated| {
            generated
                .tangents
                .into_iter()
                .zip(normals.iter())
                .map(|(tangent, normal)| {
                    tangent.unwrap_or_else(|| fallback_tangent(GVec3::from_array(*normal)))
                })
                .collect::<Vec<_>>()
        })
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
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, primitive.tex_coords_0_or_defaults());
    mesh.insert_indices(Indices::U32(primitive.indices.clone()));
    (mesh, has_tangents)
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
    owner_color: BVec4,
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
    #[storage(20, read_only)]
    owner_sample_overrides: Handle<ShaderStorageBuffer>,
    shader_alpha_mode: AlphaMode,
    render_alpha_mode: AlphaMode,
    cull_mode: Option<Face>,
    depth_write: bool,
    depth_compare: CompareFunction,
    front_face: CaptureFrontFace,
    depth_bias: f32,
}

impl BevyMtoonMaterial {
    fn needs_source_order_offset(&self) -> bool {
        matches!(
            self.render_alpha_mode,
            AlphaMode::Blend | AlphaMode::Premultiplied | AlphaMode::Add | AlphaMode::Multiply
        )
    }

    fn is_owner_id_diagnostic(&self) -> bool {
        self.material_flags2.w > 4.5 && self.material_flags2.w < 5.5
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct BevyMtoonKey {
    cull_mode: Option<Face>,
    depth_write: bool,
    depth_compare: CompareFunction,
    front_face: CaptureFrontFace,
    owner_id_no_blend: bool,
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
    owner_color: BVec4,
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
            owner_color: material.owner_color,
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
            depth_compare: material.depth_compare,
            front_face: material.front_face,
            owner_id_no_blend: material.is_owner_id_diagnostic(),
        }
    }
}

impl Material for BevyMtoonMaterial {
    fn fragment_shader() -> ShaderRef {
        MTOON_SHADER_ASSET_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.render_alpha_mode
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
        descriptor.primitive.front_face = key.bind_group_data.front_face.to_bevy();
        descriptor.primitive.cull_mode = key.bind_group_data.cull_mode;
        if layout.0.contains(Mesh::ATTRIBUTE_TANGENT) {
            descriptor.vertex.shader_defs.push("VERTEX_TANGENTS".into());
            if let Some(fragment) = &mut descriptor.fragment {
                fragment.shader_defs.push("VERTEX_TANGENTS".into());
            }
        }
        if layout.0.contains(Mesh::ATTRIBUTE_COLOR) {
            descriptor.vertex.shader_defs.push("VERTEX_COLORS".into());
            if let Some(fragment) = &mut descriptor.fragment {
                fragment.shader_defs.push("VERTEX_COLORS".into());
            }
        }
        if let Some(depth_stencil) = &mut descriptor.depth_stencil {
            depth_stencil.depth_write_enabled = key.bind_group_data.depth_write;
            depth_stencil.depth_compare = key.bind_group_data.depth_compare;
        }
        if key.bind_group_data.owner_id_no_blend
            && let Some(fragment) = &mut descriptor.fragment
        {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = None;
            }
        }
        Ok(())
    }
}

fn bevy_mtoon_material(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    shading: GltfMaterialShadingPlan,
    material_plan: BevyMaterialPlan,
    context: &BevyPrimitiveContext<'_>,
    depth_bias: f32,
    normal_map: BevyNormalMapMaterialPlan,
) -> BevyMtoonMaterial {
    let uv_transforms = loaded.expression_material_uv_transforms(
        primitive.material,
        context.options.mtoon_time,
        context.expression_effects,
    );
    let uv_plan = uv_transforms.uniform_plan();
    let image_handles = context.image_handles;
    let texture_plan = loaded
        .material_texture_slots(primitive.material)
        .binding_plan();
    let render_extra = shading
        .render_extra_plan(GltfMaterialRenderExtraOptions {
            light_accumulation: context.options.mtoon_light_accumulation.into(),
            derivative_normals: normal_map.derivative,
            view_derivative_normals: normal_map.view_derivative,
            direct_light_scale: context.options.direct_light_scale,
        })
        .uniform_plan();
    let material_flags2 = BVec4::new(
        render_extra.flags2[0],
        render_extra.flags2[1],
        if context.options.diagnostic_render == DiagnosticRender::Flat {
            1.0
        } else {
            0.0
        },
        match context.options.diagnostic_render {
            DiagnosticRender::BaseFactor => -1.0,
            DiagnosticRender::BaseColor => 1.0,
            DiagnosticRender::BaseColorFlipV => 2.0,
            DiagnosticRender::BaseColorRawSrgb => 1.25,
            DiagnosticRender::Uv => 3.0,
            DiagnosticRender::BaseUv => 4.0,
            DiagnosticRender::OwnerId => 5.0,
            DiagnosticRender::Shaded | DiagnosticRender::Flat => 0.0,
        },
    );
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
        material_flags: BVec4::from_array(render_extra.flags),
        material_flags2,
        pbr_params: BVec4::from_array(render_extra.pbr_params),
        owner_color: BVec4::ZERO,
        outline_color: BVec4::new(1.0, 1.0, 1.0, -1.0),
        pipeline: BVec4::new(
            alpha_mode_code(material_plan.alpha_mode),
            alpha_cutoff(material_plan.alpha_mode),
            normal_map.scale,
            if material_plan.cull_mode.is_none() {
                1.0
            } else {
                0.0
            },
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
        owner_sample_overrides: context.owner_sample_overrides.clone(),
        shader_alpha_mode: material_plan.alpha_mode,
        render_alpha_mode: bevy_render_alpha_mode(material_plan.alpha_mode),
        cull_mode: material_plan.cull_mode,
        depth_write: material_plan.depth_write,
        depth_compare: bevy_depth_compare(context.options),
        front_face: context.options.front_face,
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
        GltfMaterialTextureColorSpace::Srgb
            if binding.slot == GltfMaterialTextureSlot::Base && handles.raw_base_color_filter =>
        {
            &handles.raw_color_images
        }
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
    base_camera_eye(options) + camera_jitter_world(options)
}

fn camera_view(options: &CaptureOptions) -> Mat4 {
    Mat4::look_at_rh(
        camera_eye(options),
        base_camera_target(options) + camera_jitter_world(options),
        GVec3::Y,
    )
}

fn base_camera_eye(options: &CaptureOptions) -> GVec3 {
    GVec3::new(0.0, options.camera_y, -options.camera_z)
}

fn base_camera_target(options: &CaptureOptions) -> GVec3 {
    GVec3::new(0.0, options.target_y, 0.0)
}

fn bevy_camera_transform(options: &CaptureOptions) -> Transform {
    Transform::from_translation(to_bevy_vec3(
        base_camera_eye(options) + camera_jitter_world(options),
    ))
    .looking_at(
        to_bevy_vec3(base_camera_target(options) + camera_jitter_world(options)),
        Vec3::Y,
    )
}

fn camera_jitter_world(options: &CaptureOptions) -> GVec3 {
    camera_jitter_world_pixels(
        [options.screen_jitter_x, options.screen_jitter_y],
        options.height,
        options.camera_z,
    )
}

fn camera_jitter_world_pixels(screen_jitter: [f32; 2], height: u32, camera_z: f32) -> GVec3 {
    let distance = camera_z.max(0.0001);
    let half_height = (0.5 * 30.0_f32.to_radians()).tan() * distance;
    let world_per_pixel = 2.0 * half_height / height as f32;
    GVec3::new(
        -screen_jitter[0] * world_per_pixel,
        screen_jitter[1] * world_per_pixel,
        0.0,
    )
}

fn to_bevy_vec3(value: GVec3) -> Vec3 {
    Vec3::new(value.x, value.y, value.z)
}

fn projection_y_scale() -> f32 {
    1.0 / (0.5 * 30.0_f32.to_radians()).tan()
}

fn material_transparent_order_offset(phase_order: i32, draw_order: i32) -> f32 {
    (phase_order as f32 + draw_order as f32) * BEVY_PHASE_ORDER_OFFSET_SCALE
}

fn owner_id_draw_index_sort_distance(render_order: i32, draw_index: usize) -> f32 {
    render_order as f32 * BEVY_OWNER_ID_RENDER_ORDER_SORT_SCALE
        + draw_index as f32 * BEVY_OWNER_ID_DRAW_INDEX_SORT_SCALE
}

fn owner_id_draw_index_reverse_sort_distance(render_order: i32, draw_index: usize) -> f32 {
    render_order as f32 * BEVY_OWNER_ID_RENDER_ORDER_SORT_SCALE
        - draw_index as f32 * BEVY_OWNER_ID_DRAW_INDEX_SORT_SCALE
}

fn bevy_source_order_offset(
    material: &BevyPrimitiveMaterial,
    phase_order: i32,
    draw_order: i32,
) -> f32 {
    if material.needs_source_order_offset() {
        material_transparent_order_offset(phase_order, draw_order)
    } else {
        0.0
    }
}

fn render_depth_bias(_render_order: i32) -> f32 {
    0.0
}

fn bevy_cull_mode_from_plan(
    cull_mode: render_capture_scene::CaptureMaterialCullMode,
) -> Option<Face> {
    match cull_mode {
        render_capture_scene::CaptureMaterialCullMode::Off => None,
        render_capture_scene::CaptureMaterialCullMode::Front => Some(Face::Front),
        render_capture_scene::CaptureMaterialCullMode::Back => Some(Face::Back),
    }
}

fn bevy_alpha_mode_from_plan(
    alpha_mode: render_capture_scene::CaptureMaterialAlphaMode,
    alpha_cutoff: f32,
) -> AlphaMode {
    match alpha_mode {
        render_capture_scene::CaptureMaterialAlphaMode::Opaque => AlphaMode::Opaque,
        render_capture_scene::CaptureMaterialAlphaMode::Mask => AlphaMode::Mask(alpha_cutoff),
        render_capture_scene::CaptureMaterialAlphaMode::Blend => AlphaMode::Blend,
    }
}

fn bevy_render_alpha_mode(alpha_mode: AlphaMode) -> AlphaMode {
    if alpha_mode == AlphaMode::Opaque {
        // Keep shader alpha semantics opaque, but use Bevy's sorted phase so
        // equal-depth MToon overlaps preserve the same source-order behavior
        // as three-vrm/wgpu/Ash capture paths.
        AlphaMode::Blend
    } else {
        alpha_mode
    }
}

fn bevy_depth_compare(options: &CaptureOptions) -> CompareFunction {
    if options.diagnostic_render == DiagnosticRender::OwnerId
        && options.owner_id_strict_depth_compare
    {
        CompareFunction::Greater
    } else {
        CompareFunction::GreaterEqual
    }
}

fn bevy_image_with_format(
    image: &ImageData,
    format: TextureFormat,
    sampler: GltfSamplerData,
    use_mips: bool,
) -> Option<Image> {
    Some(bevy_image_from_rgba(
        image.width,
        image.height,
        image_data_to_rgba8(image).ok()?,
        format,
        sampler,
        use_mips,
    ))
}

fn effective_sampler_data(
    mut sampler: GltfSamplerData,
    force_nearest_textures: bool,
) -> GltfSamplerData {
    if force_nearest_textures {
        sampler.mag_filter = GltfMagFilter::Nearest;
        sampler.min_filter = GltfMinFilter::Nearest;
    }
    sampler
}

fn bevy_image_from_rgba(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    format: TextureFormat,
    sampler: GltfSamplerData,
    use_mips: bool,
) -> Image {
    let levels = if use_mips {
        generate_rgba_mip_chain(width, height, &rgba)
            .expect("texture upload RGBA data should match its dimensions")
    } else {
        vec![vrm_io::RgbaMipLevel {
            width,
            height,
            rgba,
        }]
    };
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
    bevy_image_from_rgba(
        1,
        1,
        rgba.to_vec(),
        format,
        GltfSamplerData::default(),
        true,
    )
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
                if let Some(distance) = order.distance_override {
                    item.distance = distance;
                } else {
                    item.distance += order.offset;
                }
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
    owner_metadata: Option<Res<RenderOwnerMetadata>>,
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

    let result = write_capture(&options, owner_metadata.as_deref(), &image_data)
        .map_err(|error| error.to_string());
    let _ = sender.0.send(result);
    app_exit_writer.write(AppExit::Success);
}

fn write_capture(
    options: &CaptureOptions,
    owner_metadata: Option<&RenderOwnerMetadata>,
    image_data: &[u8],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let row_bytes = options.width as usize * 4;
    let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes);
    let mut rgba = if row_bytes == aligned_row_bytes {
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
    let diagnostic_owner_ids = owner_metadata
        .map(|metadata| metadata.diagnostic_owner_ids.clone())
        .unwrap_or_default();
    let owner_sample_correction_plan = if let Some(path) = &options.owner_sample_correction_manifest
    {
        let plan = render_capture_correction::load_owner_sample_correction_manifest(path)?;
        if options.apply_owner_sample_readback_replacement {
            render_capture_correction::apply_owner_sample_correction_plan(
                &plan,
                options.width,
                options.height,
                &mut rgba,
            )?;
        }
        Some(
            render_capture_correction::owner_sample_correction_plan_metadata(
                path,
                &plan,
                owner_metadata
                    .map(|metadata| metadata.render_surfaces.clone())
                    .unwrap_or_default(),
            ),
        )
    } else {
        None
    };
    let artifact = json!({
        "generator": "vrm-rs examples/bevy_render_capture.rs",
        "fixture": options.fixture.to_string_lossy(),
        "width": options.width,
        "height": options.height,
        "disableOutlines": options.disable_outlines,
        "outlineWidthScale": options.outline_width_scale,
        "disableNormalMaps": options.disable_normal_maps,
        "disableTextureMips": options.disable_texture_mips,
        "forceNearestTextures": options.force_nearest_textures,
        "normalMapMode": options.normal_map_mode.as_str(),
        "normalMapScale": options.normal_map_scale,
        "diagnosticRender": options.diagnostic_render.as_str(),
        "disableOwnerIdDepthBias": options.disable_owner_id_depth_bias,
        "ownerIdStrictDepthCompare": options.owner_id_strict_depth_compare,
        "disableOwnerIdPhaseOrder": options.disable_owner_id_phase_order,
        "ownerIdPhaseOrderPolicy": options.owner_id_phase_order_policy.as_str(),
        "ownerIdColorSource": options.owner_id_color_source.as_str(),
        "frontFace": options.front_face.as_str(),
        "renderer": {
            "backend": "bevy",
            "diagnosticOwnerIds": diagnostic_owner_ids,
            "ownerSampleCorrectionPlan": owner_sample_correction_plan,
        },
        "expressions": options.expressions,
        "camera": {
            "y": options.camera_y,
            "z": options.camera_z,
            "targetY": options.target_y,
            "screenJitter": [options.screen_jitter_x, options.screen_jitter_y],
            "screenJitterMode": "camera-translation"
        },
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
    if let Some(path) = &options.imqraw_out {
        render_capture_imqraw::write_imqraw_rgba8(
            path,
            "bevy",
            ["bevy", "candidate"],
            options.width,
            options.height,
            &rgba,
        )?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_jitter_uses_same_world_scale_for_non_square_pixels() {
        let jitter = camera_jitter_world_pixels([2.0, 2.0], 180, 3.0);

        assert_close(jitter.x.abs(), jitter.y.abs());
    }

    #[test]
    fn shader_declares_owner_sample_override_storage_path() {
        assert!(MTOON_SHADER_SOURCE.contains("@binding(20)"));
        assert!(MTOON_SHADER_SOURCE.contains("var<storage, read> owner_sample_overrides"));
        assert!(MTOON_SHADER_SOURCE.contains("arrayLength(&owner_sample_overrides)"));
        assert!(MTOON_SHADER_SOURCE.contains("geometry_ids: vec4<u32>"));
        assert!(MTOON_SHADER_SOURCE.contains("geometry_uvs: vec4<f32>"));
        assert!(MTOON_SHADER_SOURCE.contains("owner_sample_override_index(input.position"));
        assert!(MTOON_SHADER_SOURCE.contains("owner_sample_has_geometry(owner_sample_index)"));
        assert!(MTOON_SHADER_SOURCE.contains("owner_sample_base_uv(owner_sample_index"));
        assert!(
            MTOON_SHADER_SOURCE.contains("use_owner_sample_geometry = owner_sample_has_geometry")
        );
        assert!(MTOON_SHADER_SOURCE.contains("textureSampleGrad("));
        assert!(!MTOON_SHADER_SOURCE.contains("textureSampleLevel("));
        assert!(!MTOON_SHADER_SOURCE.contains("apply_owner_sample_override"));
    }

    #[test]
    fn bevy_render_order_preserves_adapter_transparent_order() {
        let early = render_capture_scene::CaptureMaterialPlan {
            render_order: 3000,
            phase_order: 0,
            alpha_mode: render_capture_scene::CaptureMaterialAlphaMode::Blend,
            transparent_order_offset: Some(0),
            ..Default::default()
        };
        let late = render_capture_scene::CaptureMaterialPlan {
            render_order: 3019,
            phase_order: 19,
            alpha_mode: render_capture_scene::CaptureMaterialAlphaMode::Blend,
            transparent_order_offset: Some(19),
            ..Default::default()
        };

        assert!(bevy_render_order_from_plan(early) < bevy_render_order_from_plan(late));
        assert_eq!(bevy_render_order_from_plan(late), 3019);
    }

    #[test]
    fn bevy_transparent_phase_order_bias_keeps_early_material_first() {
        let early = material_transparent_order_offset(0, 0);
        let late = material_transparent_order_offset(19, 1);

        assert!(early < late);
    }

    #[test]
    fn owner_id_draw_index_sort_key_orders_render_order_then_draw_index() {
        let first = owner_id_draw_index_sort_distance(3000, 10);
        let second = owner_id_draw_index_sort_distance(3000, 11);
        let next_render_order = owner_id_draw_index_sort_distance(3001, 0);
        let late_same_render_order = owner_id_draw_index_sort_distance(3000, 999);

        assert!(first < second);
        assert!(second < late_same_render_order);
        assert!(second < next_render_order);
        assert!(late_same_render_order < next_render_order);
    }

    #[test]
    fn owner_id_reverse_draw_index_sort_key_keeps_render_order_primary() {
        let first = owner_id_draw_index_reverse_sort_distance(3000, 10);
        let second = owner_id_draw_index_reverse_sort_distance(3000, 11);
        let next_render_order = owner_id_draw_index_reverse_sort_distance(3001, 0);
        let late_same_render_order = owner_id_draw_index_reverse_sort_distance(3000, 999);

        assert!(second < first);
        assert!(late_same_render_order < second);
        assert!(first < next_render_order);
        assert!(late_same_render_order < next_render_order);
    }

    #[test]
    fn owner_id_strict_depth_compare_is_diagnostic_only() {
        let mut options = CaptureOptions::parse_from([
            "bevy_render_capture",
            "--fixture",
            "fixture.vrm",
            "--out",
            "out.rgba.json",
            "--diagnostic-render",
            "owner-id",
        ]);
        assert_eq!(bevy_depth_compare(&options), CompareFunction::GreaterEqual);
        options.owner_id_strict_depth_compare = true;
        assert_eq!(bevy_depth_compare(&options), CompareFunction::Greater);
        options.diagnostic_render = DiagnosticRender::Shaded;
        assert_eq!(bevy_depth_compare(&options), CompareFunction::GreaterEqual);
    }

    #[test]
    fn owner_id_reverse_draw_index_preserves_phase_offset() {
        assert_eq!(
            owner_id_phase_order_offset_for_policy(
                OwnerIdPhaseOrderPolicy::DrawIndex,
                0.000019,
                true,
            ),
            0.0
        );
        assert_eq!(
            owner_id_phase_order_offset_for_policy(
                OwnerIdPhaseOrderPolicy::DrawIndexReverse,
                0.000019,
                false,
            ),
            0.000019
        );
        assert_eq!(
            owner_id_phase_order_offset_for_policy(
                OwnerIdPhaseOrderPolicy::DrawIndexReverseNoPhase,
                0.000019,
                true,
            ),
            0.0
        );
        assert_eq!(
            owner_id_phase_order_offset_for_policy(
                OwnerIdPhaseOrderPolicy::OverlapTriangle,
                0.000019,
                false,
            ),
            0.0
        );
    }

    #[test]
    fn owner_id_overlap_area_policy_keeps_large_same_material_overlaps_only() {
        let groups = vec![
            owner_phase_group(19.0, Some(1), 3000, 10.0, 10.0, 14.0, 14.0, 0.5),
            owner_phase_group(19.0, Some(1), 3000, 11.0, 11.0, 15.0, 15.0, 0.5005),
            owner_phase_group(19.0, Some(1), 3000, 30.0, 30.0, 31.0, 31.0, 0.5),
            owner_phase_group(19.0, Some(1), 3000, 30.5, 30.5, 31.5, 31.5, 0.5005),
            owner_phase_group(19.0, Some(2), 3000, 10.0, 10.0, 14.0, 14.0, 0.5),
        ];

        assert_eq!(
            owner_id_phase_order_overlap_mask_from_groups(
                &groups,
                OwnerIdPhaseOrderPolicy::OverlapArea
            ),
            vec![true, true, false, false, false]
        );
    }

    #[test]
    fn owner_id_triangle_overlap_policy_rejects_shared_edges() {
        let groups = vec![
            owner_phase_triangle_group(
                19.0,
                Some(1),
                3000,
                [[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]],
                0.5,
            ),
            owner_phase_triangle_group(
                19.0,
                Some(1),
                3000,
                [[4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
                0.5005,
            ),
            owner_phase_triangle_group(
                19.0,
                Some(1),
                3000,
                [[8.0, 0.0], [12.0, 0.0], [8.0, 4.0]],
                0.5,
            ),
            owner_phase_triangle_group(
                19.0,
                Some(1),
                3000,
                [[9.0, 1.0], [13.0, 1.0], [9.0, 5.0]],
                0.5005,
            ),
            owner_phase_triangle_group(
                19.0,
                Some(1),
                3000,
                [[16.0, 0.0], [20.0, 0.0], [16.0, 4.0]],
                0.5,
            ),
            owner_phase_triangle_group(
                19.0,
                Some(1),
                3000,
                [[16.0, 0.0], [20.0, 0.0], [16.0, 4.0]],
                0.5005,
            ),
        ];

        assert_eq!(
            owner_id_phase_order_overlap_mask_from_groups(
                &groups,
                OwnerIdPhaseOrderPolicy::OverlapArea
            ),
            vec![true, true, true, true, true, true]
        );
        assert_eq!(
            owner_id_phase_order_overlap_mask_from_groups(
                &groups,
                OwnerIdPhaseOrderPolicy::OverlapTriangle
            ),
            vec![false, false, true, true, true, true]
        );
    }

    #[test]
    fn bevy_opaque_mtoon_uses_source_order_capable_phase() {
        assert_eq!(bevy_render_alpha_mode(AlphaMode::Opaque), AlphaMode::Blend);
        assert_eq!(
            bevy_render_alpha_mode(AlphaMode::Mask(0.5)),
            AlphaMode::Mask(0.5)
        );
    }

    #[test]
    fn owner_id_unindexing_preserves_index_buffer_order() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        );
        mesh.insert_indices(Indices::U32(vec![2, 0, 1]));

        duplicate_mesh_vertices_in_index_order(&mut mesh, &[2, 0, 1]).unwrap();

        assert!(mesh.indices().is_none());
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(VertexAttributeValues::as_float3)
            .unwrap();
        assert_eq!(
            positions,
            &[[0.0, 1.0, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]
        );
    }

    #[test]
    fn owner_id_triangle_mesh_can_use_vertex_or_uniform_color_source() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        );

        let vertex_color_mesh = owner_id_triangle_mesh(&mesh, 0, Some([1.0, 0.0, 0.0, 1.0]))
            .expect("owner triangle mesh should be generated");
        assert!(vertex_color_mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_some());

        let uniform_color_mesh = owner_id_triangle_mesh(&mesh, 0, None)
            .expect("owner triangle mesh should be generated");
        assert!(
            uniform_color_mesh
                .attribute(Mesh::ATTRIBUTE_COLOR)
                .is_none()
        );
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1.0e-6,
            "expected {expected}, got {actual}"
        );
    }

    fn owner_phase_group(
        offset_units: f32,
        material: Option<usize>,
        render_order: i32,
        min_x: f32,
        min_y: f32,
        max_x: f32,
        max_y: f32,
        webgl_depth: f32,
    ) -> OwnerIdPhaseOrderProjectionGroup {
        OwnerIdPhaseOrderProjectionGroup {
            render_order,
            material,
            transparent_order_offset: offset_units * BEVY_PHASE_ORDER_OFFSET_SCALE,
            projections: vec![ScreenTriangleProjection {
                screen: [[min_x, min_y], [max_x, min_y], [min_x, max_y]],
                bounds: ScreenProjectionBounds {
                    min_x,
                    min_y,
                    max_x,
                    max_y,
                },
                ndc_depth: webgl_depth,
                webgl_depth,
                screen_signed_area: 1.0,
                front_facing: true,
                gpu_front_facing: true,
            }],
        }
    }

    fn owner_phase_triangle_group(
        offset_units: f32,
        material: Option<usize>,
        render_order: i32,
        screen: [[f32; 2]; 3],
        webgl_depth: f32,
    ) -> OwnerIdPhaseOrderProjectionGroup {
        OwnerIdPhaseOrderProjectionGroup {
            render_order,
            material,
            transparent_order_offset: offset_units * BEVY_PHASE_ORDER_OFFSET_SCALE,
            projections: vec![ScreenTriangleProjection {
                screen,
                bounds: ScreenProjectionBounds::from_triangle(screen),
                ndc_depth: webgl_depth,
                webgl_depth,
                screen_signed_area: 1.0,
                front_facing: true,
                gpu_front_facing: true,
            }],
        }
    }
}
