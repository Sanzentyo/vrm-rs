//! Renderer-neutral VRMA animation sampling example.
//!
//! This loads a VRM avatar plus a separate VRMA clip, samples several frames,
//! applies those frames to `HeadlessSceneState`, and prints the resulting
//! humanoid/expression/lookAt state. It is intentionally renderer-free so the
//! same flow can be copied into Bevy, wgpu, ash, or a custom engine adapter.
//!
//! Example with local external fixtures:
//!
//! ```text
//! cargo run --example headless_vrma_animation -- \
//!   --avatar .external-fixtures/official/Seed-san.vrm \
//!   --animation .external-fixtures/official/test.vrma \
//!   --frames 5
//! ```

use std::error::Error;
use std::path::PathBuf;

use clap::Parser;
use glam::{Quat, Vec3};
use vrm_adapter::{
    HumanoidPoseRig, TransformAccess, WorldTransformUpdate, apply_vrma_animation_frame_with_look_at,
};
use vrm_core::{HumanBoneName, Transform, VrmAnimation, VrmDocument};
use vrm_io::{LoadedVrm, load_vrm_from_path};
use vrm_runtime::{VrmAnimationFrame, sample_vrm_animation};

#[derive(Clone, Debug, Parser)]
#[command(
    about = "Sample a VRMA clip onto a VRM avatar through the renderer-neutral headless adapter"
)]
struct Options {
    /// VRM avatar file used as the runtime target.
    #[arg(long)]
    avatar: PathBuf,
    /// VRMA animation clip file.
    #[arg(long)]
    animation: PathBuf,
    /// Number of evenly spaced frames to sample.
    #[arg(long, default_value_t = 5)]
    frames: usize,
    /// First sample time in seconds.
    #[arg(long, default_value_t = 0.0)]
    start: f32,
    /// Last sample time in seconds. Defaults to the clip duration.
    #[arg(long)]
    end: Option<f32>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse();
    if options.frames == 0 {
        return Err("frames must be at least 1".into());
    }

    let avatar = load_vrm_from_path(&options.avatar)?;
    let animation_asset = load_vrm_from_path(&options.animation)?;
    let animation = animation_from_loaded(&animation_asset)
        .ok_or("animation file does not contain a VRM animation")?;
    let document = avatar.model().document();
    let mut scene = vrm_rs::headless_scene_from_loaded(&avatar)?;
    scene.update_world_transforms()?;
    let mut rig = HumanoidPoseRig::capture(&scene, document)?;
    let times = sample_times(
        options.start,
        options.end.unwrap_or(animation.duration),
        options.frames,
    );

    println!(
        "avatar={} animation={} duration={:.3}s frames={}",
        document.meta.name,
        options.animation.display(),
        animation.duration,
        times.len(),
    );
    println!(
        "time, hips_translation, hips_rotation_xyzw, head_rotation_xyzw, look_at_xyzw, expressions"
    );

    for time in times {
        let frame = sample_vrm_animation(animation, time);
        apply_vrma_animation_frame_with_look_at(&mut scene, &mut rig, document, &frame)?;
        scene.update_world_transforms()?;
        print_sample(document, &scene, time, &frame)?;
    }

    Ok(())
}

fn animation_from_loaded(loaded: &LoadedVrm) -> Option<&VrmAnimation> {
    let document = loaded.model().document();
    document
        .animation
        .as_ref()
        .or_else(|| document.animations.first())
}

fn sample_times(start: f32, end: f32, frames: usize) -> Vec<f32> {
    match frames {
        0 => Vec::new(),
        1 => vec![start],
        frames => (0..frames)
            .map(|index| start + (end - start) * index as f32 / (frames - 1) as f32)
            .collect(),
    }
}

fn print_sample(
    document: &VrmDocument,
    scene: &vrm_adapter::HeadlessSceneState,
    time: f32,
    frame: &VrmAnimationFrame,
) -> Result<(), Box<dyn Error>> {
    let hips = bone_local_transform(document, scene, HumanBoneName::Hips)?;
    let head = bone_local_transform(document, scene, HumanBoneName::Head)?;
    let look_at = scene.look_at_rotation().unwrap_or(Quat::IDENTITY);

    println!(
        "{time:.3}, {}, {}, {}, {}, {}",
        format_vec3(hips.translation),
        format_quat(hips.rotation),
        format_quat(head.rotation),
        format_quat(look_at),
        expression_summary(frame)
    );
    Ok(())
}

fn bone_local_transform(
    document: &VrmDocument,
    scene: &vrm_adapter::HeadlessSceneState,
    bone: HumanBoneName,
) -> Result<Transform, Box<dyn Error>> {
    let node = document
        .humanoid
        .bones
        .get(&bone)
        .ok_or_else(|| format!("avatar is missing required bone {bone:?}"))?
        .node;
    Ok(scene.local_transform(node)?)
}

fn expression_summary(frame: &VrmAnimationFrame) -> String {
    let preset = frame
        .preset_expressions
        .iter()
        .map(|(name, weight)| format!("{}={weight:.3}", name.as_str()));
    let custom = frame
        .custom_expressions
        .iter()
        .map(|(name, weight)| format!("{name}={weight:.3}"));
    let summary = preset.chain(custom).collect::<Vec<_>>().join(" ");
    if summary.is_empty() {
        "-".to_owned()
    } else {
        summary
    }
}

fn format_vec3(value: Vec3) -> String {
    format!("[{:.4}, {:.4}, {:.4}]", value.x, value.y, value.z)
}

fn format_quat(value: Quat) -> String {
    format!(
        "[{:.4}, {:.4}, {:.4}, {:.4}]",
        value.x, value.y, value.z, value.w
    )
}
