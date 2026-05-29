//! Headless Bevy render capture for render-parity experiments.
//!
//! This example renders real `vrm-io` mesh primitives through Bevy's renderer
//! into an offscreen image and writes the shared RGBA JSON artifact consumed by
//! `tools/render-parity/compare-psnr.mjs`.

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::system::SystemParam;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::Extract;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderSystems;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_graph::{
    self, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel,
};
use bevy::render::render_resource::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode, PollType,
    PrimitiveTopology, TexelCopyBufferInfo, TexelCopyBufferLayout, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use crossbeam_channel::{Receiver, Sender};
use glam::Mat4;
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use vrm_io::{GltfPrimitiveData, ImageData, ImageFormat, LoadedVrm, load_vrm_from_path};

fn main() -> Result<(), Box<dyn Error>> {
    let options = CaptureOptions::parse()?;
    let loaded = load_vrm_from_path(&options.fixture)?;
    let (tx, rx) = crossbeam_channel::bounded(1);

    App::new()
        .insert_resource(options.clone())
        .insert_resource(LoadedResource(loaded))
        .insert_resource(CaptureSender(tx))
        .insert_resource(SceneController::new(options.width, options.height, 40))
        .insert_resource(ClearColor(Color::NONE))
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
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .add_systems(Startup, setup)
        .run();

    rx.recv()?.map_err(|message| message.into())
}

#[derive(Clone, Debug, Resource)]
struct CaptureOptions {
    fixture: PathBuf,
    out: PathBuf,
    png_out: Option<PathBuf>,
    width: u32,
    height: u32,
    camera_y: f32,
    camera_z: f32,
    target_y: f32,
}

impl CaptureOptions {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        let mut values = HashMap::new();
        let mut index = 0;
        while index < args.len() {
            let key = &args[index];
            if !key.starts_with("--") {
                return Err(format!("unexpected positional argument: {key}").into());
            }
            let Some(value) = args.get(index + 1) else {
                return Err(format!("missing value for {key}").into());
            };
            values.insert(key.trim_start_matches("--").to_string(), value.clone());
            index += 2;
        }

        Ok(Self {
            fixture: required_path(&values, "fixture")?,
            out: required_path(&values, "out")?,
            png_out: values.get("png-out").map(PathBuf::from),
            width: parse_u32(&values, "width", 512)?,
            height: parse_u32(&values, "height", 512)?,
            camera_y: parse_f32(&values, "camera-y", 1.0)?,
            camera_z: parse_f32(&values, "camera-z", 5.0)?,
            target_y: parse_f32(&values, "target-y", 1.0)?,
        })
    }
}

fn required_path(values: &HashMap<String, String>, name: &str) -> Result<PathBuf, Box<dyn Error>> {
    values
        .get(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing --{name}").into())
}

fn parse_u32(
    values: &HashMap<String, String>,
    name: &str,
    default: u32,
) -> Result<u32, Box<dyn Error>> {
    values
        .get(name)
        .map_or(Ok(default), |value| value.parse().map_err(Into::into))
}

fn parse_f32(
    values: &HashMap<String, String>,
    name: &str,
    default: f32,
) -> Result<f32, Box<dyn Error>> {
    values
        .get(name)
        .map_or(Ok(default), |value| value.parse().map_err(Into::into))
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
    render_device: Res<RenderDevice>,
    mut scene_controller: ResMut<SceneController>,
) {
    let render_target = setup_render_target(
        &mut commands,
        &mut assets.images,
        &render_device,
        &mut scene_controller,
    );

    spawn_vrm_meshes(
        &mut commands,
        &loaded.0,
        &mut assets.meshes,
        &mut assets.materials,
        &mut assets.images,
    );

    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            ..default()
        },
        Transform::from_xyz(1.0, 1.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Camera3d::default(),
        render_target,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        Tonemapping::None,
        Transform::from_xyz(0.0, options.camera_y, -options.camera_z)
            .looking_at(Vec3::new(0.0, options.target_y, 0.0), Vec3::Y),
    ));
}

#[derive(SystemParam)]
struct CaptureAssets<'w> {
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    images: ResMut<'w, Assets<Image>>,
}

fn spawn_vrm_meshes(
    commands: &mut Commands,
    loaded: &LoadedVrm,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    let image_handles = loaded
        .images
        .iter()
        .map(|image| bevy_image(image).map(|image| images.add(image)))
        .collect::<Vec<_>>();
    let orientation = Mat4::from_rotation_y(std::f32::consts::PI);

    for node in &loaded.scene.nodes {
        let Some(mesh_index) = node.mesh else {
            continue;
        };
        let Some(mesh) = loaded.meshes.get(mesh_index) else {
            continue;
        };
        let transform = bevy_transform(orientation * node.world_matrix);
        for primitive in &mesh.primitives {
            let material = materials.add(bevy_material(loaded, primitive, &image_handles));
            commands.spawn((
                Mesh3d(meshes.add(bevy_mesh(primitive))),
                MeshMaterial3d(material),
                transform,
            ));
        }
    }
}

fn bevy_mesh(primitive: &GltfPrimitiveData) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, primitive.positions.clone());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        normals_or_default(primitive.positions.len(), &primitive.normals),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        tex_coords_or_default(primitive.positions.len(), &primitive.tex_coords_0),
    );
    mesh.insert_indices(Indices::U32(primitive.indices.clone()));
    mesh
}

fn normals_or_default(vertex_count: usize, normals: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if normals.len() == vertex_count {
        normals.to_vec()
    } else {
        vec![[0.0, 0.0, 1.0]; vertex_count]
    }
}

fn tex_coords_or_default(vertex_count: usize, tex_coords: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if tex_coords.len() == vertex_count {
        tex_coords.to_vec()
    } else {
        vec![[0.0, 0.0]; vertex_count]
    }
}

fn bevy_material(
    loaded: &LoadedVrm,
    primitive: &GltfPrimitiveData,
    image_handles: &[Option<Handle<Image>>],
) -> StandardMaterial {
    let base_color = primitive
        .material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref())
        .map(|mtoon| {
            Color::srgba(
                mtoon.base_color_factor[0],
                mtoon.base_color_factor[1],
                mtoon.base_color_factor[2],
                mtoon.base_color_factor[3],
            )
        })
        .or_else(|| {
            primitive
                .material
                .and_then(|index| loaded.gltf_materials.get(index))
                .map(|material| {
                    Color::srgba(
                        material.base_color_factor[0],
                        material.base_color_factor[1],
                        material.base_color_factor[2],
                        material.base_color_factor[3],
                    )
                })
        })
        .unwrap_or(Color::srgb(0.78, 0.78, 0.78));

    StandardMaterial {
        base_color,
        base_color_texture: material_main_image(loaded, primitive.material)
            .and_then(|image| image_handles.get(image))
            .and_then(Clone::clone),
        unlit: true,
        perceptual_roughness: 1.0,
        reflectance: 0.0,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
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

fn bevy_image(image: &ImageData) -> Option<Image> {
    Some(Image::new(
        Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        image_rgba8(image)?,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ))
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

fn bevy_transform(matrix: Mat4) -> Transform {
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    Transform {
        translation: Vec3::from_array(translation.to_array()),
        rotation: Quat::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w),
        scale: Vec3::from_array(scale.to_array()),
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
                receive_image_from_buffer.after(RenderSystems::Render),
            );
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
        Image::new_target_texture(size.width, size.height, TextureFormat::bevy_default(), None);
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
    let artifact = json!({
        "generator": "vrm-rs examples/bevy_render_capture.rs",
        "fixture": options.fixture.to_string_lossy(),
        "width": options.width,
        "height": options.height,
        "camera": { "y": options.camera_y, "z": options.camera_z, "targetY": options.target_y },
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
