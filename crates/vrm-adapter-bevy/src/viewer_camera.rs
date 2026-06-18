//! Orbit camera controls for Bevy VRM viewers.

use bevy::ecs::message::MessageReader;
use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::{
    App, ButtonInput, Component, KeyCode, MouseButton, Plugin, Query, Res, Transform, Update, Vec2,
    Vec3,
};

const MIN_RADIUS: f32 = 0.05;
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

/// Adds mouse and keyboard driven orbit-camera controls.
#[derive(Clone, Copy, Debug, Default)]
pub struct BevyVrmOrbitCameraPlugin;

impl Plugin for BevyVrmOrbitCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, update_vrm_orbit_cameras);
    }
}

/// Home pose used by `VrmOrbitCamera::reset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VrmOrbitCameraHome {
    pub target: Vec3,
    pub radius: f32,
    pub yaw: f32,
    pub pitch: f32,
}

/// Mouse/keyboard orbit camera suitable for VRM preview windows.
///
/// Default controls:
/// - Left mouse drag: orbit.
/// - Right or middle mouse drag: pan.
/// - Mouse wheel: zoom.
/// - `F`: focus the configured focus target.
/// - `R`: reset to the initial home pose.
#[derive(Clone, Copy, Debug, Component, PartialEq)]
pub struct VrmOrbitCamera {
    pub target: Vec3,
    pub focus_target: Vec3,
    pub radius: f32,
    pub min_radius: f32,
    pub max_radius: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub min_pitch: f32,
    pub max_pitch: f32,
    pub orbit_sensitivity: Vec2,
    pub pan_sensitivity: f32,
    pub zoom_sensitivity: f32,
    pub home: VrmOrbitCameraHome,
}

impl VrmOrbitCamera {
    pub fn new(target: Vec3, radius: f32) -> Self {
        Self::from_angles(target, radius, 0.0, 0.0)
    }

    pub fn from_position(target: Vec3, position: Vec3) -> Self {
        let offset = position - target;
        let radius = offset.length().max(MIN_RADIUS);
        let pitch = (offset.y / radius).clamp(-1.0, 1.0).asin();
        let yaw = offset.x.atan2(offset.z);
        Self::from_angles(target, radius, yaw, pitch)
    }

    pub fn from_angles(target: Vec3, radius: f32, yaw: f32, pitch: f32) -> Self {
        let radius = radius.max(MIN_RADIUS);
        let pitch = pitch.clamp(-MAX_PITCH, MAX_PITCH);
        let home = VrmOrbitCameraHome {
            target,
            radius,
            yaw,
            pitch,
        };
        Self {
            target,
            focus_target: target,
            radius,
            min_radius: MIN_RADIUS,
            max_radius: 100.0,
            yaw,
            pitch,
            min_pitch: -MAX_PITCH,
            max_pitch: MAX_PITCH,
            orbit_sensitivity: Vec2::splat(0.006),
            pan_sensitivity: 0.0015,
            zoom_sensitivity: 0.12,
            home,
        }
    }

    pub fn with_radius_limits(mut self, min_radius: f32, max_radius: f32) -> Self {
        self.min_radius = min_radius.max(MIN_RADIUS);
        self.max_radius = max_radius.max(self.min_radius);
        self.radius = self.radius.clamp(self.min_radius, self.max_radius);
        self.home.radius = self.home.radius.clamp(self.min_radius, self.max_radius);
        self
    }

    pub fn with_focus_target(mut self, focus_target: Vec3) -> Self {
        self.focus_target = focus_target;
        self
    }

    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * self.orbit_sensitivity.x;
        self.pitch =
            (self.pitch - delta.y * self.orbit_sensitivity.y).clamp(self.min_pitch, self.max_pitch);
    }

    pub fn pan(&mut self, delta: Vec2) {
        let transform = self.transform();
        let right = transform.rotation * Vec3::X;
        let up = transform.rotation * Vec3::Y;
        let scale = self.radius * self.pan_sensitivity;
        self.target += (-right * delta.x + up * delta.y) * scale;
    }

    pub fn zoom(&mut self, scroll_lines: f32) {
        let factor = (-scroll_lines * self.zoom_sensitivity).exp();
        self.radius = (self.radius * factor).clamp(self.min_radius, self.max_radius);
    }

    pub fn focus(&mut self) {
        self.target = self.focus_target;
    }

    pub fn reset(&mut self) {
        self.target = self.home.target;
        self.radius = self.home.radius.clamp(self.min_radius, self.max_radius);
        self.yaw = self.home.yaw;
        self.pitch = self.home.pitch.clamp(self.min_pitch, self.max_pitch);
    }

    pub fn transform(&self) -> Transform {
        let cos_pitch = self.pitch.cos();
        let offset = Vec3::new(
            self.radius * cos_pitch * self.yaw.sin(),
            self.radius * self.pitch.sin(),
            self.radius * cos_pitch * self.yaw.cos(),
        );
        Transform::from_translation(self.target + offset).looking_at(self.target, Vec3::Y)
    }

    pub fn write_transform(&self, transform: &mut Transform) {
        *transform = self.transform();
    }
}

pub fn update_vrm_orbit_cameras(
    mut motion_reader: MessageReader<'_, '_, MouseMotion>,
    mut wheel_reader: MessageReader<'_, '_, MouseWheel>,
    mouse_buttons: Res<'_, ButtonInput<MouseButton>>,
    keys: Res<'_, ButtonInput<KeyCode>>,
    mut cameras: Query<'_, '_, (&mut VrmOrbitCamera, &mut Transform)>,
) {
    let motion_delta = motion_reader
        .read()
        .map(|event| event.delta)
        .fold(Vec2::ZERO, |sum, delta| sum + delta);
    let scroll_lines = wheel_reader
        .read()
        .map(|event| match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y / MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR,
        })
        .sum::<f32>();

    for (mut camera, mut transform) in &mut cameras {
        if mouse_buttons.pressed(MouseButton::Left) && motion_delta != Vec2::ZERO {
            camera.orbit(motion_delta);
        }
        if (mouse_buttons.pressed(MouseButton::Right) || mouse_buttons.pressed(MouseButton::Middle))
            && motion_delta != Vec2::ZERO
        {
            camera.pan(motion_delta);
        }
        if scroll_lines.abs() > f32::EPSILON {
            camera.zoom(scroll_lines);
        }
        if keys.just_pressed(KeyCode::KeyF) {
            camera.focus();
        }
        if keys.just_pressed(KeyCode::KeyR) {
            camera.reset();
        }
        camera.write_transform(&mut transform);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_camera_round_trips_position_to_angles() {
        let target = Vec3::new(0.0, 1.0, 0.0);
        let position = Vec3::new(0.0, 1.0, 3.0);
        let camera = VrmOrbitCamera::from_position(target, position);
        assert!((camera.radius - 3.0).abs() < 0.0001);
        assert!((camera.transform().translation - position).length() < 0.0001);
    }

    #[test]
    fn orbit_camera_zoom_respects_limits() {
        let mut camera = VrmOrbitCamera::new(Vec3::ZERO, 3.0).with_radius_limits(1.0, 4.0);
        camera.zoom(-100.0);
        assert_eq!(camera.radius, 4.0);
        camera.zoom(100.0);
        assert_eq!(camera.radius, 1.0);
    }

    #[test]
    fn orbit_camera_focus_and_reset_are_distinct() {
        let mut camera =
            VrmOrbitCamera::new(Vec3::ZERO, 3.0).with_focus_target(Vec3::new(0.0, 1.2, 0.0));
        camera.pan(Vec2::new(10.0, -5.0));
        camera.focus();
        assert_eq!(camera.target, Vec3::new(0.0, 1.2, 0.0));
        camera.reset();
        assert_eq!(camera.target, Vec3::ZERO);
    }
}
