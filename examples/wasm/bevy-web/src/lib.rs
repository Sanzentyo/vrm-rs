use bevy::prelude::*;
use bevy::window::WindowResolution;
use vrm_adapter_bevy::{
    BevyVrmAnimationClip, BevyVrmMaterialMode, BevyVrmOrientation, BevyVrmScenePlugin,
    BevyVrmSpawnConfig, animation_from_loaded, spawn_vrm_scene,
};
use vrm_io::{LoadedVrm, load_vrm_from_slice};
use wasm_bindgen::prelude::*;

#[derive(Resource)]
struct WebVrmAssets {
    avatar: Option<LoadedVrm>,
    animation: Option<BevyVrmAnimationClip>,
}

#[wasm_bindgen]
pub fn start_bevy_vrm_viewer(
    canvas_selector: String,
    avatar_bytes: Vec<u8>,
    animation_bytes: Vec<u8>,
) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let avatar = load_vrm_from_slice(&avatar_bytes).map_err(js_error)?;
    let animation = if animation_bytes.is_empty() {
        None
    } else {
        let loaded = load_vrm_from_slice(&animation_bytes).map_err(js_error)?;
        animation_from_loaded(&loaded).map(BevyVrmAnimationClip::new)
    };

    App::new()
        .insert_resource(ClearColor(Color::srgb(0.02, 0.025, 0.03)))
        .insert_resource(WebVrmAssets {
            avatar: Some(avatar),
            animation,
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "vrm-rs Bevy web".to_owned(),
                resolution: WindowResolution::new(1280, 720),
                canvas: Some(canvas_selector),
                fit_canvas_to_parent: true,
                prevent_default_event_handling: false,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(BevyVrmScenePlugin)
        .add_systems(Startup, setup)
        .run();
    Ok(())
}

fn setup(
    mut commands: Commands<'_, '_>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut materials: ResMut<'_, Assets<StandardMaterial>>,
    mut images: ResMut<'_, Assets<Image>>,
    mut assets: ResMut<'_, WebVrmAssets>,
) {
    let avatar = assets
        .avatar
        .take()
        .expect("avatar is loaded before startup");
    let root = spawn_vrm_scene(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        avatar,
        BevyVrmSpawnConfig {
            orientation: BevyVrmOrientation::FrontFacingBevy,
            material_mode: BevyVrmMaterialMode::UnlitBaseColor,
            root_transform: Transform::default(),
            animation: assets.animation.take(),
        },
    )
    .unwrap_or_else(|error| panic!("failed to spawn VRM scene: {error}"));

    commands.entity(root).insert(Name::new("VRM avatar"));
    let target = Vec3::new(0.0, 1.1, 0.0);
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.1, 3.0).looking_at(target, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 4_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(1.0, 2.0, 1.0).looking_at(target, Vec3::Y),
    ));
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
