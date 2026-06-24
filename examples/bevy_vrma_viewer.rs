//! Interactive Bevy viewer for a VRM avatar plus an optional VRMA animation.
//!
//! The rendering and animation bridge lives in `vrm-adapter-bevy`; this example
//! only wires command-line IO, camera, lights, and the Bevy app.

use bevy::prelude::*;
use bevy::window::WindowResolution;
use clap::{Parser, ValueEnum};
use std::error::Error;
use std::path::PathBuf;
use vrm_adapter_bevy::{
    BevyVrmAnimationClip, BevyVrmMaterialMode, BevyVrmOrbitCameraPlugin, BevyVrmOrientation,
    BevyVrmScenePlugin, BevyVrmSpawnConfig, VrmOrbitCamera, animation_from_loaded, spawn_vrm_scene,
};
use vrm_io::load_vrm_from_path;

#[derive(Clone, Debug, Parser, Resource)]
#[command(about = "Display a VRM avatar in Bevy and optionally play a VRMA clip")]
struct Options {
    /// VRM avatar file.
    #[arg(long, default_value = ".external-fixtures/official/Seed-san.vrm")]
    avatar: PathBuf,
    /// Optional VRMA animation clip file.
    #[arg(long, default_value = ".external-fixtures/official/idle_loop.vrma")]
    animation: PathBuf,
    /// Disable VRMA playback after loading the avatar.
    #[arg(long)]
    no_animation: bool,
    /// Playback speed multiplier.
    #[arg(long, default_value_t = 1.0)]
    speed: f32,
    /// Use Bevy PBR lighting instead of unlit base-color material preview.
    #[arg(long, value_enum, default_value_t = MaterialMode::Unlit)]
    material: MaterialMode,
    /// Use glTF/VRM matrices as-is instead of rotating the baked mesh toward Bevy's camera.
    #[arg(long)]
    identity_orientation: bool,
    /// Uniform scene scale applied to the spawned root.
    #[arg(long, default_value_t = 1.0)]
    scale: f32,
    /// Camera Z distance.
    #[arg(long, default_value_t = 3.0)]
    camera_z: f32,
    /// Minimum orbit camera radius.
    #[arg(long, default_value_t = 0.4)]
    min_camera_radius: f32,
    /// Maximum orbit camera radius.
    #[arg(long, default_value_t = 20.0)]
    max_camera_radius: f32,
    /// Camera target height.
    #[arg(long, default_value_t = 1.1)]
    look_y: f32,
    /// Initial window width.
    #[arg(long, default_value_t = 1280)]
    width: u32,
    /// Initial window height.
    #[arg(long, default_value_t = 720)]
    height: u32,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum MaterialMode {
    Unlit,
    Pbr,
}

impl From<MaterialMode> for BevyVrmMaterialMode {
    fn from(value: MaterialMode) -> Self {
        match value {
            MaterialMode::Unlit => Self::UnlitBaseColor,
            MaterialMode::Pbr => Self::StandardPbr,
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse();
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.02, 0.025, 0.03)))
        .insert_resource(options.clone())
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "vrm-rs Bevy VRMA Viewer".to_owned(),
                resolution: WindowResolution::new(options.width, options.height),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((BevyVrmScenePlugin, BevyVrmOrbitCameraPlugin))
        .add_systems(Startup, setup)
        .run();
    Ok(())
}

fn setup(
    mut commands: Commands<'_, '_>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut materials: ResMut<'_, Assets<StandardMaterial>>,
    mut images: ResMut<'_, Assets<Image>>,
    options: Res<'_, Options>,
) {
    let avatar = load_vrm_from_path(&options.avatar).unwrap_or_else(|error| {
        panic!(
            "failed to load avatar {}: {error}",
            options.avatar.display()
        )
    });
    let animation = if options.no_animation {
        None
    } else {
        let loaded = load_vrm_from_path(&options.animation).unwrap_or_else(|error| {
            panic!(
                "failed to load animation {}: {error}",
                options.animation.display()
            )
        });
        animation_from_loaded(&loaded).map(|animation| {
            let mut clip = BevyVrmAnimationClip::new(animation);
            clip.playback.speed = options.speed;
            clip
        })
    };

    let orientation = if options.identity_orientation {
        BevyVrmOrientation::Identity
    } else {
        BevyVrmOrientation::FrontFacingBevy
    };
    let root_transform = Transform {
        scale: Vec3::splat(options.scale),
        ..default()
    };

    let root = spawn_vrm_scene(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        avatar,
        BevyVrmSpawnConfig {
            orientation,
            material_mode: options.material.into(),
            root_transform,
            animation,
        },
    )
    .unwrap_or_else(|error| panic!("failed to spawn VRM scene: {error}"));

    commands.entity(root).insert(Name::new("VRM avatar"));
    let camera_target = Vec3::new(0.0, options.look_y, 0.0);
    let camera_position = Vec3::new(0.0, options.look_y, options.camera_z);
    let orbit_camera = VrmOrbitCamera::from_position(camera_target, camera_position)
        .with_radius_limits(options.min_camera_radius, options.max_camera_radius)
        .with_focus_target(camera_target);
    commands.spawn((Camera3d::default(), orbit_camera.transform(), orbit_camera));
    commands.spawn((
        DirectionalLight {
            illuminance: 4_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(1.0, 2.0, 1.0).looking_at(Vec3::new(0.0, options.look_y, 0.0), Vec3::Y),
    ));
}
