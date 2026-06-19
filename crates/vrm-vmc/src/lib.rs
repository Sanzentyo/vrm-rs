//! Typed VMC message layer over `vrm-osc`.
//!
//! The crate keeps sockets at the edge: callers decode UDP/TCP with
//! `vrm-osc`, then use [`VmcMessage::from_osc_message`] or [`apply_packet`].
//! Applications that want reusable sender/rate/time policy without giving
//! this crate socket ownership can put packets through [`VmcTransportGate`]
//! before applying them to a runtime sink.

use glam::{Quat, Vec3};
use thiserror::Error;
use vrm_osc::{OscMessage, OscPacket, OscType};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VmcTransform {
    pub translation: Vec3,
    pub rotation: Quat,
}

impl Default for VmcTransform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmcDeviceKind {
    Hmd,
    Controller,
    Tracker,
}

impl VmcDeviceKind {
    fn address_prefix(self) -> &'static str {
        match self {
            Self::Hmd => "/VMC/Ext/Hmd/Pos",
            Self::Controller => "/VMC/Ext/Con/Pos",
            Self::Tracker => "/VMC/Ext/Tra/Pos",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmcInputState {
    Release,
    Press,
    Axis,
    Other(i32),
}

impl VmcInputState {
    fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Release,
            1 => Self::Press,
            2 => Self::Axis,
            other => Self::Other(other),
        }
    }

    pub fn raw(self) -> i32 {
        match self {
            Self::Release => 0,
            Self::Press => 1,
            Self::Axis => 2,
            Self::Other(value) => value,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VmcParsePolicy {
    #[default]
    Strict,
    IgnoreInvalidKnownMessages,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VmcSenderId(String);

impl VmcSenderId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for VmcSenderId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for VmcSenderId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VmcPacketContext {
    pub sender: Option<VmcSenderId>,
    pub received_at_seconds: f64,
}

impl VmcPacketContext {
    pub fn anonymous(received_at_seconds: f64) -> Self {
        Self {
            sender: None,
            received_at_seconds,
        }
    }

    pub fn from_sender(sender: impl Into<VmcSenderId>, received_at_seconds: f64) -> Self {
        Self {
            sender: Some(sender.into()),
            received_at_seconds,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VmcTransportPolicy {
    pub parse_policy: VmcParsePolicy,
    pub allowed_senders: Vec<VmcSenderId>,
    pub max_messages_per_packet: Option<usize>,
    pub min_packet_interval_seconds: Option<f64>,
    pub reject_relative_time_rewind: bool,
    pub max_relative_time_step_seconds: Option<f32>,
}

impl Default for VmcTransportPolicy {
    fn default() -> Self {
        Self {
            parse_policy: VmcParsePolicy::Strict,
            allowed_senders: Vec::new(),
            max_messages_per_packet: None,
            min_packet_interval_seconds: None,
            reject_relative_time_rewind: true,
            max_relative_time_step_seconds: None,
        }
    }
}

impl VmcTransportPolicy {
    pub fn open() -> Self {
        Self::default()
    }

    pub fn parse_policy(mut self, policy: VmcParsePolicy) -> Self {
        self.parse_policy = policy;
        self
    }

    pub fn allowed_senders(mut self, senders: impl IntoIterator<Item = VmcSenderId>) -> Self {
        self.allowed_senders = senders.into_iter().collect();
        self
    }

    pub fn allow_sender(mut self, sender: impl Into<VmcSenderId>) -> Self {
        self.allowed_senders.push(sender.into());
        self
    }

    pub fn max_messages_per_packet(mut self, max: usize) -> Self {
        self.max_messages_per_packet = Some(max);
        self
    }

    pub fn min_packet_interval_seconds(mut self, seconds: f64) -> Self {
        self.min_packet_interval_seconds = Some(seconds);
        self
    }

    pub fn reject_relative_time_rewind(mut self, reject: bool) -> Self {
        self.reject_relative_time_rewind = reject;
        self
    }

    pub fn max_relative_time_step_seconds(mut self, seconds: f32) -> Self {
        self.max_relative_time_step_seconds = Some(seconds);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VmcTransportReport {
    pub sender: Option<VmcSenderId>,
    pub received_at_seconds: f64,
    pub message_count: usize,
    pub relative_time: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VmcAcceptedPacket {
    pub messages: Vec<VmcMessage>,
    pub report: VmcTransportReport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VmcTransportGate {
    policy: VmcTransportPolicy,
    sender_states: Vec<VmcSenderTransportState>,
}

impl Default for VmcTransportGate {
    fn default() -> Self {
        Self::new(VmcTransportPolicy::default())
    }
}

impl VmcTransportGate {
    pub fn new(policy: VmcTransportPolicy) -> Self {
        Self {
            policy,
            sender_states: Vec::new(),
        }
    }

    pub fn policy(&self) -> &VmcTransportPolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut VmcTransportPolicy {
        &mut self.policy
    }

    pub fn accept_packet(
        &mut self,
        context: VmcPacketContext,
        packet: &OscPacket,
    ) -> Result<VmcAcceptedPacket, VmcTransportError> {
        self.ensure_context_allowed(&context)?;
        let messages = collect_packet_messages_with_policy(packet, self.policy.parse_policy)?;
        self.ensure_message_count(messages.len())?;
        let relative_time = self.ensure_sender_timing(&context, &messages)?;
        let report = VmcTransportReport {
            sender: context.sender.clone(),
            received_at_seconds: context.received_at_seconds,
            message_count: messages.len(),
            relative_time,
        };
        Ok(VmcAcceptedPacket { messages, report })
    }

    pub fn apply_packet<S>(
        &mut self,
        sink: &mut S,
        context: VmcPacketContext,
        packet: &OscPacket,
    ) -> Result<VmcTransportReport, VmcTransportApplyError<S::Error>>
    where
        S: VmcRuntimeSink,
    {
        let accepted = self.accept_packet(context, packet)?;
        apply_messages(sink, &accepted.messages).map_err(VmcTransportApplyError::from_apply)?;
        Ok(accepted.report)
    }

    fn ensure_context_allowed(&self, context: &VmcPacketContext) -> Result<(), VmcTransportError> {
        if !context.received_at_seconds.is_finite() {
            return Err(VmcTransportError::InvalidReceiveTime {
                received_at_seconds: context.received_at_seconds,
            });
        }
        if self.policy.allowed_senders.is_empty() {
            return Ok(());
        }
        let Some(sender) = &context.sender else {
            return Err(VmcTransportError::MissingSender);
        };
        if self.policy.allowed_senders.contains(sender) {
            Ok(())
        } else {
            Err(VmcTransportError::UnauthorizedSender {
                sender: sender.clone(),
            })
        }
    }

    fn ensure_message_count(&self, message_count: usize) -> Result<(), VmcTransportError> {
        if let Some(max) = self.policy.max_messages_per_packet
            && message_count > max
        {
            return Err(VmcTransportError::TooManyMessages {
                max,
                actual: message_count,
            });
        }
        Ok(())
    }

    fn ensure_sender_timing(
        &mut self,
        context: &VmcPacketContext,
        messages: &[VmcMessage],
    ) -> Result<Option<f32>, VmcTransportError> {
        let policy = self.policy.clone();
        let state = self.sender_state_mut(context.sender.clone());
        if let Some(previous) = state.last_received_at_seconds {
            if context.received_at_seconds < previous {
                return Err(VmcTransportError::ReceiveTimeRewind {
                    previous,
                    actual: context.received_at_seconds,
                });
            }
            if let Some(min_interval) = policy.min_packet_interval_seconds {
                let elapsed = context.received_at_seconds - previous;
                if elapsed + f64::EPSILON < min_interval {
                    return Err(VmcTransportError::RateLimited {
                        min_interval_seconds: min_interval,
                        elapsed_seconds: elapsed,
                    });
                }
            }
        }

        let relative_time = latest_relative_time(messages)?;
        if let Some(actual) = relative_time {
            if let Some(previous) = state.last_relative_time {
                if policy.reject_relative_time_rewind && actual + f32::EPSILON < previous {
                    return Err(VmcTransportError::RelativeTimeRewind { previous, actual });
                }
                if let Some(max_step) = policy.max_relative_time_step_seconds {
                    let step = actual - previous;
                    if step > max_step {
                        return Err(VmcTransportError::RelativeTimeStepTooLarge { max_step, step });
                    }
                }
            }
            state.last_relative_time = Some(actual);
        }
        state.last_received_at_seconds = Some(context.received_at_seconds);
        Ok(relative_time)
    }

    fn sender_state_mut(&mut self, sender: Option<VmcSenderId>) -> &mut VmcSenderTransportState {
        if let Some(index) = self
            .sender_states
            .iter()
            .position(|state| state.sender == sender)
        {
            &mut self.sender_states[index]
        } else {
            self.sender_states.push(VmcSenderTransportState {
                sender,
                last_received_at_seconds: None,
                last_relative_time: None,
            });
            self.sender_states
                .last_mut()
                .expect("sender state was just pushed")
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct VmcSenderTransportState {
    sender: Option<VmcSenderId>,
    last_received_at_seconds: Option<f64>,
    last_relative_time: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VmcMessage {
    Available {
        available: bool,
        calibration_state: Option<i32>,
        calibration_mode: Option<i32>,
        tracking_status: Option<i32>,
    },
    RelativeTime(f32),
    RootPose {
        name: String,
        transform: VmcTransform,
        scale: Option<Vec3>,
        offset: Option<Vec3>,
    },
    BonePose {
        bone: String,
        transform: VmcTransform,
    },
    BlendValue {
        name: String,
        value: f32,
    },
    BlendApply,
    CameraPose {
        name: String,
        transform: VmcTransform,
        fov_y_degrees: Option<f32>,
    },
    DirectionalLight {
        name: String,
        transform: VmcTransform,
        color: Option<[f32; 4]>,
    },
    ControllerInput {
        state: VmcInputState,
        name: String,
        is_left: bool,
        is_touch: bool,
        is_axis: bool,
        axis: Vec3,
    },
    KeyboardInput {
        state: VmcInputState,
        name: String,
        keycode: i32,
    },
    MidiNote {
        state: VmcInputState,
        channel: i32,
        note: i32,
        velocity: f32,
    },
    MidiCcValue {
        knob: i32,
        value: f32,
    },
    MidiCcButton {
        knob: i32,
        state: VmcInputState,
    },
    DevicePose {
        kind: VmcDeviceKind,
        local: bool,
        serial: String,
        transform: VmcTransform,
    },
    ReceiveEnabled {
        enabled: bool,
        port: i32,
        ip_address: Option<String>,
    },
    LocalVrmInfo {
        path: String,
        title: String,
        hash: Option<String>,
    },
    RemoteVrmInfo {
        service: String,
        json: String,
    },
    OptionString(String),
    SettingColor([f32; 4]),
    WindowAttribute {
        is_top_most: bool,
        is_transparent: bool,
        window_click_through: bool,
        hide_border: bool,
    },
    ConfigPath(String),
    SetPeriod {
        status: i32,
        root: i32,
        bone: i32,
        blend_shape: i32,
        camera: i32,
        devices: i32,
    },
    EyeTrackingTarget {
        enabled: bool,
        position: Vec3,
    },
    InformationRequest,
    ResponseString(String),
    CalibrationReady,
    CalibrationExecute {
        mode: i32,
    },
    RequestLoadConfig {
        path: String,
    },
    Shortcut {
        shortcut: String,
    },
    Thru(OscMessage),
    Unknown(OscMessage),
}

impl VmcMessage {
    pub fn from_osc_message(message: &OscMessage) -> Result<Self, VmcError> {
        match message.addr.as_str() {
            "/VMC/Ext/OK" => parse_available(message),
            "/VMC/Ext/T" => parse_relative_time(message),
            "/VMC/Ext/Root/Pos" => parse_root_pose(message),
            "/VMC/Ext/Bone/Pos" => parse_bone_pose(message),
            "/VMC/Ext/Blend/Val" => parse_blend_value(message),
            "/VMC/Ext/Blend/Apply" => parse_no_args(message, Self::BlendApply),
            "/VMC/Ext/Cam" | "/VMC/Ext/Camera/Pos" => parse_camera_pose(message),
            "/VMC/Ext/Light" | "/VMC/Ext/Light/Dir" => parse_light(message),
            "/VMC/Ext/Con" => parse_controller_input(message),
            "/VMC/Ext/Key" => parse_keyboard_input(message),
            "/VMC/Ext/Midi/Note" => parse_midi_note(message),
            "/VMC/Ext/Midi/CC/Val" => parse_midi_cc_value(message),
            "/VMC/Ext/Midi/CC/Bit" => parse_midi_cc_button(message),
            "/VMC/Ext/Hmd/Pos" => parse_device_pose(message, VmcDeviceKind::Hmd, false),
            "/VMC/Ext/Con/Pos" => parse_device_pose(message, VmcDeviceKind::Controller, false),
            "/VMC/Ext/Tra/Pos" => parse_device_pose(message, VmcDeviceKind::Tracker, false),
            "/VMC/Ext/Hmd/Pos/Local" => parse_device_pose(message, VmcDeviceKind::Hmd, true),
            "/VMC/Ext/Con/Pos/Local" => parse_device_pose(message, VmcDeviceKind::Controller, true),
            "/VMC/Ext/Tra/Pos/Local" => parse_device_pose(message, VmcDeviceKind::Tracker, true),
            "/VMC/Ext/Rcv" => parse_receive_enabled(message),
            "/VMC/Ext/VRM" => parse_local_vrm_info(message),
            "/VMC/Ext/Remote" => parse_remote_vrm_info(message),
            "/VMC/Ext/Opt" => parse_option_string(message),
            "/VMC/Ext/Setting/Color" => parse_setting_color(message),
            "/VMC/Ext/Setting/Win" => parse_window_attribute(message),
            "/VMC/Ext/Config" => parse_config_path(message),
            "/VMC/Ext/Set/Period" => parse_set_period(message),
            "/VMC/Ext/Set/Eye" => parse_eye_tracking_target(message),
            "/VMC/Ext/Set/Req" => parse_no_args(message, Self::InformationRequest),
            "/VMC/Ext/Set/Res" => parse_response_string(message),
            "/VMC/Ext/Set/Calib/Ready" => parse_no_args(message, Self::CalibrationReady),
            "/VMC/Ext/Set/Calib/Exec" => parse_calibration_execute(message),
            "/VMC/Ext/Set/Config" => parse_request_load_config(message),
            "/VMC/Ext/Set/Shortcut" => parse_shortcut(message),
            address if address.starts_with("/VMC/Thru/") => Ok(Self::Thru(message.clone())),
            _ => Ok(Self::Unknown(message.clone())),
        }
    }

    pub fn to_osc_message(&self) -> OscMessage {
        match self {
            Self::Available {
                available,
                calibration_state,
                calibration_mode,
                tracking_status,
            } => {
                let mut args = vec![OscType::Int(i32::from(*available))];
                args.extend(calibration_state.map(OscType::Int));
                args.extend(calibration_mode.map(OscType::Int));
                args.extend(tracking_status.map(OscType::Int));
                osc_message("/VMC/Ext/OK", args)
            }
            Self::RelativeTime(time) => osc_message("/VMC/Ext/T", vec![OscType::Float(*time)]),
            Self::RootPose {
                name,
                transform,
                scale,
                offset,
            } => {
                let mut args = transform_args(name, *transform);
                if let Some(scale) = scale {
                    args.extend(vec3_args(*scale));
                }
                if let Some(offset) = offset {
                    args.extend(vec3_args(*offset));
                }
                osc_message("/VMC/Ext/Root/Pos", args)
            }
            Self::BonePose { bone, transform } => {
                osc_message("/VMC/Ext/Bone/Pos", transform_args(bone, *transform))
            }
            Self::BlendValue { name, value } => osc_message(
                "/VMC/Ext/Blend/Val",
                vec![OscType::String(name.clone()), OscType::Float(*value)],
            ),
            Self::BlendApply => osc_message("/VMC/Ext/Blend/Apply", Vec::new()),
            Self::CameraPose {
                name,
                transform,
                fov_y_degrees,
            } => {
                let mut args = transform_args(name, *transform);
                args.extend(fov_y_degrees.map(OscType::Float));
                osc_message("/VMC/Ext/Cam", args)
            }
            Self::DirectionalLight {
                name,
                transform,
                color,
            } => {
                let mut args = transform_args(name, *transform);
                if let Some(color) = color {
                    args.extend(color.iter().copied().map(OscType::Float));
                }
                osc_message("/VMC/Ext/Light", args)
            }
            Self::ControllerInput {
                state,
                name,
                is_left,
                is_touch,
                is_axis,
                axis,
            } => osc_message(
                "/VMC/Ext/Con",
                vec![
                    OscType::Int(state.raw()),
                    OscType::String(name.clone()),
                    OscType::Int(bool_int(*is_left)),
                    OscType::Int(bool_int(*is_touch)),
                    OscType::Int(bool_int(*is_axis)),
                    OscType::Float(axis.x),
                    OscType::Float(axis.y),
                    OscType::Float(axis.z),
                ],
            ),
            Self::KeyboardInput {
                state,
                name,
                keycode,
            } => osc_message(
                "/VMC/Ext/Key",
                vec![
                    OscType::Int(state.raw()),
                    OscType::String(name.clone()),
                    OscType::Int(*keycode),
                ],
            ),
            Self::MidiNote {
                state,
                channel,
                note,
                velocity,
            } => osc_message(
                "/VMC/Ext/Midi/Note",
                vec![
                    OscType::Int(state.raw()),
                    OscType::Int(*channel),
                    OscType::Int(*note),
                    OscType::Float(*velocity),
                ],
            ),
            Self::MidiCcValue { knob, value } => osc_message(
                "/VMC/Ext/Midi/CC/Val",
                vec![OscType::Int(*knob), OscType::Float(*value)],
            ),
            Self::MidiCcButton { knob, state } => osc_message(
                "/VMC/Ext/Midi/CC/Bit",
                vec![OscType::Int(*knob), OscType::Int(state.raw())],
            ),
            Self::DevicePose {
                kind,
                local,
                serial,
                transform,
            } => {
                let mut address = kind.address_prefix().to_owned();
                if *local {
                    address.push_str("/Local");
                }
                osc_message(&address, transform_args(serial, *transform))
            }
            Self::ReceiveEnabled {
                enabled,
                port,
                ip_address,
            } => {
                let mut args = vec![OscType::Int(bool_int(*enabled)), OscType::Int(*port)];
                args.extend(ip_address.iter().cloned().map(OscType::String));
                osc_message("/VMC/Ext/Rcv", args)
            }
            Self::LocalVrmInfo { path, title, hash } => {
                let mut args = vec![
                    OscType::String(path.clone()),
                    OscType::String(title.clone()),
                ];
                args.extend(hash.iter().cloned().map(OscType::String));
                osc_message("/VMC/Ext/VRM", args)
            }
            Self::RemoteVrmInfo { service, json } => osc_message(
                "/VMC/Ext/Remote",
                vec![
                    OscType::String(service.clone()),
                    OscType::String(json.clone()),
                ],
            ),
            Self::OptionString(option) => {
                osc_message("/VMC/Ext/Opt", vec![OscType::String(option.clone())])
            }
            Self::SettingColor(color) => osc_message(
                "/VMC/Ext/Setting/Color",
                color.iter().copied().map(OscType::Float).collect(),
            ),
            Self::WindowAttribute {
                is_top_most,
                is_transparent,
                window_click_through,
                hide_border,
            } => osc_message(
                "/VMC/Ext/Setting/Win",
                vec![
                    OscType::Int(bool_int(*is_top_most)),
                    OscType::Int(bool_int(*is_transparent)),
                    OscType::Int(bool_int(*window_click_through)),
                    OscType::Int(bool_int(*hide_border)),
                ],
            ),
            Self::ConfigPath(path) => {
                osc_message("/VMC/Ext/Config", vec![OscType::String(path.clone())])
            }
            Self::SetPeriod {
                status,
                root,
                bone,
                blend_shape,
                camera,
                devices,
            } => osc_message(
                "/VMC/Ext/Set/Period",
                vec![
                    OscType::Int(*status),
                    OscType::Int(*root),
                    OscType::Int(*bone),
                    OscType::Int(*blend_shape),
                    OscType::Int(*camera),
                    OscType::Int(*devices),
                ],
            ),
            Self::EyeTrackingTarget { enabled, position } => osc_message(
                "/VMC/Ext/Set/Eye",
                vec![
                    OscType::Int(bool_int(*enabled)),
                    OscType::Float(position.x),
                    OscType::Float(position.y),
                    OscType::Float(position.z),
                ],
            ),
            Self::InformationRequest => osc_message("/VMC/Ext/Set/Req", Vec::new()),
            Self::ResponseString(response) => {
                osc_message("/VMC/Ext/Set/Res", vec![OscType::String(response.clone())])
            }
            Self::CalibrationReady => osc_message("/VMC/Ext/Set/Calib/Ready", Vec::new()),
            Self::CalibrationExecute { mode } => {
                osc_message("/VMC/Ext/Set/Calib/Exec", vec![OscType::Int(*mode)])
            }
            Self::RequestLoadConfig { path } => {
                osc_message("/VMC/Ext/Set/Config", vec![OscType::String(path.clone())])
            }
            Self::Shortcut { shortcut } => osc_message(
                "/VMC/Ext/Set/Shortcut",
                vec![OscType::String(shortcut.clone())],
            ),
            Self::Thru(message) => message.clone(),
            Self::Unknown(message) => message.clone(),
        }
    }
}

pub trait VmcRuntimeSink {
    type Error;

    fn begin_vmc_transaction(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn commit_vmc_transaction(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn rollback_vmc_transaction(&mut self) {}

    fn set_available(
        &mut self,
        _available: bool,
        _calibration_state: Option<i32>,
        _calibration_mode: Option<i32>,
        _tracking_status: Option<i32>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_relative_time(&mut self, _time: f32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_root_pose(
        &mut self,
        _name: &str,
        _transform: VmcTransform,
        _scale: Option<Vec3>,
        _offset: Option<Vec3>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_bone_pose(&mut self, _bone: &str, _transform: VmcTransform) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_expression_value(&mut self, _name: &str, _value: f32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn apply_expressions(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_camera_pose(
        &mut self,
        _name: &str,
        _transform: VmcTransform,
        _fov_y_degrees: Option<f32>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_directional_light(
        &mut self,
        _name: &str,
        _transform: VmcTransform,
        _color: Option<[f32; 4]>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_controller_input(
        &mut self,
        _state: VmcInputState,
        _name: &str,
        _is_left: bool,
        _is_touch: bool,
        _is_axis: bool,
        _axis: Vec3,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_keyboard_input(
        &mut self,
        _state: VmcInputState,
        _name: &str,
        _keycode: i32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_midi_note(
        &mut self,
        _state: VmcInputState,
        _channel: i32,
        _note: i32,
        _velocity: f32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_midi_cc_value(&mut self, _knob: i32, _value: f32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_midi_cc_button(&mut self, _knob: i32, _state: VmcInputState) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_device_pose(
        &mut self,
        _kind: VmcDeviceKind,
        _local: bool,
        _serial: &str,
        _transform: VmcTransform,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_receive_enabled(
        &mut self,
        _enabled: bool,
        _port: i32,
        _ip_address: Option<&str>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_local_vrm_info(
        &mut self,
        _path: &str,
        _title: &str,
        _hash: Option<&str>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_remote_vrm_info(&mut self, _service: &str, _json: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_option_string(&mut self, _option: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_setting_color(&mut self, _color: [f32; 4]) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_window_attribute(
        &mut self,
        _is_top_most: bool,
        _is_transparent: bool,
        _window_click_through: bool,
        _hide_border: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_config_path(&mut self, _path: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_period(
        &mut self,
        _status: i32,
        _root: i32,
        _bone: i32,
        _blend_shape: i32,
        _camera: i32,
        _devices: i32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_eye_tracking_target(
        &mut self,
        _enabled: bool,
        _position: Vec3,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn request_information(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_response_string(&mut self, _response: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn request_calibration_ready(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn request_calibration_execute(&mut self, _mode: i32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn request_load_config(&mut self, _path: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn call_shortcut(&mut self, _shortcut: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn thru_vmc_message(&mut self, _message: &OscMessage) -> Result<(), Self::Error> {
        Ok(())
    }

    fn unknown_vmc_message(&mut self, _message: &OscMessage) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub fn collect_packet_messages(packet: &OscPacket) -> Result<Vec<VmcMessage>, VmcError> {
    collect_packet_messages_with_policy(packet, VmcParsePolicy::Strict)
}

pub fn collect_packet_messages_with_policy(
    packet: &OscPacket,
    policy: VmcParsePolicy,
) -> Result<Vec<VmcMessage>, VmcError> {
    let mut messages = Vec::new();
    collect_packet_messages_into(packet, &mut messages, policy)?;
    Ok(messages)
}

pub fn apply_packet<S>(sink: &mut S, packet: &OscPacket) -> Result<(), VmcApplyError<S::Error>>
where
    S: VmcRuntimeSink,
{
    apply_packet_with_policy(sink, packet, VmcParsePolicy::Strict)
}

pub fn apply_packet_with_policy<S>(
    sink: &mut S,
    packet: &OscPacket,
    policy: VmcParsePolicy,
) -> Result<(), VmcApplyError<S::Error>>
where
    S: VmcRuntimeSink,
{
    let messages = collect_packet_messages_with_policy(packet, policy)?;
    apply_messages(sink, &messages)
}

pub fn apply_messages<S>(
    sink: &mut S,
    messages: &[VmcMessage],
) -> Result<(), VmcApplyError<S::Error>>
where
    S: VmcRuntimeSink,
{
    sink.begin_vmc_transaction().map_err(VmcApplyError::Sink)?;
    for message in messages {
        if let Err(error) = apply_message(sink, message) {
            sink.rollback_vmc_transaction();
            return Err(error);
        }
    }
    sink.commit_vmc_transaction().map_err(VmcApplyError::Sink)
}

pub fn apply_message<S>(sink: &mut S, message: &VmcMessage) -> Result<(), VmcApplyError<S::Error>>
where
    S: VmcRuntimeSink,
{
    match message {
        VmcMessage::Available {
            available,
            calibration_state,
            calibration_mode,
            tracking_status,
        } => sink
            .set_available(
                *available,
                *calibration_state,
                *calibration_mode,
                *tracking_status,
            )
            .map_err(VmcApplyError::Sink),
        VmcMessage::RelativeTime(time) => {
            sink.set_relative_time(*time).map_err(VmcApplyError::Sink)
        }
        VmcMessage::RootPose {
            name,
            transform,
            scale,
            offset,
        } => sink
            .set_root_pose(name, *transform, *scale, *offset)
            .map_err(VmcApplyError::Sink),
        VmcMessage::BonePose { bone, transform } => sink
            .set_bone_pose(bone, *transform)
            .map_err(VmcApplyError::Sink),
        VmcMessage::BlendValue { name, value } => sink
            .set_expression_value(name, *value)
            .map_err(VmcApplyError::Sink),
        VmcMessage::BlendApply => sink.apply_expressions().map_err(VmcApplyError::Sink),
        VmcMessage::CameraPose {
            name,
            transform,
            fov_y_degrees,
        } => sink
            .set_camera_pose(name, *transform, *fov_y_degrees)
            .map_err(VmcApplyError::Sink),
        VmcMessage::DirectionalLight {
            name,
            transform,
            color,
        } => sink
            .set_directional_light(name, *transform, *color)
            .map_err(VmcApplyError::Sink),
        VmcMessage::ControllerInput {
            state,
            name,
            is_left,
            is_touch,
            is_axis,
            axis,
        } => sink
            .set_controller_input(*state, name, *is_left, *is_touch, *is_axis, *axis)
            .map_err(VmcApplyError::Sink),
        VmcMessage::KeyboardInput {
            state,
            name,
            keycode,
        } => sink
            .set_keyboard_input(*state, name, *keycode)
            .map_err(VmcApplyError::Sink),
        VmcMessage::MidiNote {
            state,
            channel,
            note,
            velocity,
        } => sink
            .set_midi_note(*state, *channel, *note, *velocity)
            .map_err(VmcApplyError::Sink),
        VmcMessage::MidiCcValue { knob, value } => sink
            .set_midi_cc_value(*knob, *value)
            .map_err(VmcApplyError::Sink),
        VmcMessage::MidiCcButton { knob, state } => sink
            .set_midi_cc_button(*knob, *state)
            .map_err(VmcApplyError::Sink),
        VmcMessage::DevicePose {
            kind,
            local,
            serial,
            transform,
        } => sink
            .set_device_pose(*kind, *local, serial, *transform)
            .map_err(VmcApplyError::Sink),
        VmcMessage::ReceiveEnabled {
            enabled,
            port,
            ip_address,
        } => sink
            .set_receive_enabled(*enabled, *port, ip_address.as_deref())
            .map_err(VmcApplyError::Sink),
        VmcMessage::LocalVrmInfo { path, title, hash } => sink
            .set_local_vrm_info(path, title, hash.as_deref())
            .map_err(VmcApplyError::Sink),
        VmcMessage::RemoteVrmInfo { service, json } => sink
            .set_remote_vrm_info(service, json)
            .map_err(VmcApplyError::Sink),
        VmcMessage::OptionString(option) => {
            sink.set_option_string(option).map_err(VmcApplyError::Sink)
        }
        VmcMessage::SettingColor(color) => {
            sink.set_setting_color(*color).map_err(VmcApplyError::Sink)
        }
        VmcMessage::WindowAttribute {
            is_top_most,
            is_transparent,
            window_click_through,
            hide_border,
        } => sink
            .set_window_attribute(
                *is_top_most,
                *is_transparent,
                *window_click_through,
                *hide_border,
            )
            .map_err(VmcApplyError::Sink),
        VmcMessage::ConfigPath(path) => sink.set_config_path(path).map_err(VmcApplyError::Sink),
        VmcMessage::SetPeriod {
            status,
            root,
            bone,
            blend_shape,
            camera,
            devices,
        } => sink
            .set_period(*status, *root, *bone, *blend_shape, *camera, *devices)
            .map_err(VmcApplyError::Sink),
        VmcMessage::EyeTrackingTarget { enabled, position } => sink
            .set_eye_tracking_target(*enabled, *position)
            .map_err(VmcApplyError::Sink),
        VmcMessage::InformationRequest => sink.request_information().map_err(VmcApplyError::Sink),
        VmcMessage::ResponseString(response) => sink
            .set_response_string(response)
            .map_err(VmcApplyError::Sink),
        VmcMessage::CalibrationReady => sink
            .request_calibration_ready()
            .map_err(VmcApplyError::Sink),
        VmcMessage::CalibrationExecute { mode } => sink
            .request_calibration_execute(*mode)
            .map_err(VmcApplyError::Sink),
        VmcMessage::RequestLoadConfig { path } => {
            sink.request_load_config(path).map_err(VmcApplyError::Sink)
        }
        VmcMessage::Shortcut { shortcut } => {
            sink.call_shortcut(shortcut).map_err(VmcApplyError::Sink)
        }
        VmcMessage::Thru(message) => sink.thru_vmc_message(message).map_err(VmcApplyError::Sink),
        VmcMessage::Unknown(message) => sink
            .unknown_vmc_message(message)
            .map_err(VmcApplyError::Sink),
    }
}

fn collect_packet_messages_into(
    packet: &OscPacket,
    messages: &mut Vec<VmcMessage>,
    policy: VmcParsePolicy,
) -> Result<(), VmcError> {
    match packet {
        OscPacket::Message(message) => match VmcMessage::from_osc_message(message) {
            Ok(message) => messages.push(message),
            Err(error) => {
                if policy == VmcParsePolicy::Strict {
                    return Err(error);
                }
            }
        },
        OscPacket::Bundle(bundle) => {
            for packet in &bundle.content {
                collect_packet_messages_into(packet, messages, policy)?;
            }
        }
    }
    Ok(())
}

fn latest_relative_time(messages: &[VmcMessage]) -> Result<Option<f32>, VmcTransportError> {
    messages
        .iter()
        .filter_map(|message| match message {
            VmcMessage::RelativeTime(time) => Some(*time),
            _ => None,
        })
        .try_fold(None, |_, time| {
            if time.is_finite() {
                Ok(Some(time))
            } else {
                Err(VmcTransportError::InvalidRelativeTime { time })
            }
        })
}

fn parse_available(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    let available = arg_i32(message, 0)? != 0;
    Ok(VmcMessage::Available {
        available,
        calibration_state: optional_i32(message, 1)?,
        calibration_mode: optional_i32(message, 2)?,
        tracking_status: optional_i32(message, 3)?,
    })
}

fn parse_relative_time(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    Ok(VmcMessage::RelativeTime(arg_f32(message, 0)?))
}

fn parse_root_pose(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    Ok(VmcMessage::RootPose {
        name: arg_string(message, 0)?,
        transform: transform_at(message, 1)?,
        scale: optional_vec3(message, 8)?,
        offset: optional_vec3(message, 11)?,
    })
}

fn parse_bone_pose(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    Ok(VmcMessage::BonePose {
        bone: arg_string(message, 0)?,
        transform: transform_at(message, 1)?,
    })
}

fn parse_blend_value(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 2)?;
    Ok(VmcMessage::BlendValue {
        name: arg_string(message, 0)?,
        value: arg_f32(message, 1)?,
    })
}

fn parse_camera_pose(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    Ok(VmcMessage::CameraPose {
        name: arg_string(message, 0)?,
        transform: transform_at(message, 1)?,
        fov_y_degrees: optional_f32(message, 8)?,
    })
}

fn parse_light(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    Ok(VmcMessage::DirectionalLight {
        name: arg_string(message, 0)?,
        transform: transform_at(message, 1)?,
        color: optional_color(message, 8)?,
    })
}

fn parse_controller_input(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 8)?;
    Ok(VmcMessage::ControllerInput {
        state: arg_input_state(message, 0)?,
        name: arg_string(message, 1)?,
        is_left: arg_bool_int(message, 2)?,
        is_touch: arg_bool_int(message, 3)?,
        is_axis: arg_bool_int(message, 4)?,
        axis: vec3_at(message, 5)?,
    })
}

fn parse_keyboard_input(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 3)?;
    Ok(VmcMessage::KeyboardInput {
        state: arg_input_state(message, 0)?,
        name: arg_string(message, 1)?,
        keycode: arg_i32(message, 2)?,
    })
}

fn parse_midi_note(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 4)?;
    Ok(VmcMessage::MidiNote {
        state: arg_input_state(message, 0)?,
        channel: arg_i32(message, 1)?,
        note: arg_i32(message, 2)?,
        velocity: arg_f32(message, 3)?,
    })
}

fn parse_midi_cc_value(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 2)?;
    Ok(VmcMessage::MidiCcValue {
        knob: arg_i32(message, 0)?,
        value: arg_f32(message, 1)?,
    })
}

fn parse_midi_cc_button(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 2)?;
    Ok(VmcMessage::MidiCcButton {
        knob: arg_i32(message, 0)?,
        state: arg_input_state(message, 1)?,
    })
}

fn parse_device_pose(
    message: &OscMessage,
    kind: VmcDeviceKind,
    local: bool,
) -> Result<VmcMessage, VmcError> {
    Ok(VmcMessage::DevicePose {
        kind,
        local,
        serial: arg_string(message, 0)?,
        transform: transform_at(message, 1)?,
    })
}

fn parse_receive_enabled(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    Ok(VmcMessage::ReceiveEnabled {
        enabled: arg_bool_int(message, 0)?,
        port: arg_i32(message, 1)?,
        ip_address: optional_string(message, 2)?,
    })
}

fn parse_local_vrm_info(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    Ok(VmcMessage::LocalVrmInfo {
        path: arg_string(message, 0)?,
        title: arg_string(message, 1)?,
        hash: optional_string(message, 2)?,
    })
}

fn parse_remote_vrm_info(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 2)?;
    Ok(VmcMessage::RemoteVrmInfo {
        service: arg_string(message, 0)?,
        json: arg_string(message, 1)?,
    })
}

fn parse_option_string(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 1)?;
    Ok(VmcMessage::OptionString(arg_string(message, 0)?))
}

fn parse_setting_color(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 4)?;
    Ok(VmcMessage::SettingColor([
        arg_f32(message, 0)?,
        arg_f32(message, 1)?,
        arg_f32(message, 2)?,
        arg_f32(message, 3)?,
    ]))
}

fn parse_window_attribute(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 4)?;
    Ok(VmcMessage::WindowAttribute {
        is_top_most: arg_bool_int(message, 0)?,
        is_transparent: arg_bool_int(message, 1)?,
        window_click_through: arg_bool_int(message, 2)?,
        hide_border: arg_bool_int(message, 3)?,
    })
}

fn parse_config_path(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 1)?;
    Ok(VmcMessage::ConfigPath(arg_string(message, 0)?))
}

fn parse_set_period(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 6)?;
    Ok(VmcMessage::SetPeriod {
        status: arg_i32(message, 0)?,
        root: arg_i32(message, 1)?,
        bone: arg_i32(message, 2)?,
        blend_shape: arg_i32(message, 3)?,
        camera: arg_i32(message, 4)?,
        devices: arg_i32(message, 5)?,
    })
}

fn parse_eye_tracking_target(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 4)?;
    Ok(VmcMessage::EyeTrackingTarget {
        enabled: arg_bool_int(message, 0)?,
        position: vec3_at(message, 1)?,
    })
}

fn parse_response_string(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 1)?;
    Ok(VmcMessage::ResponseString(arg_string(message, 0)?))
}

fn parse_calibration_execute(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 1)?;
    Ok(VmcMessage::CalibrationExecute {
        mode: arg_i32(message, 0)?,
    })
}

fn parse_request_load_config(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 1)?;
    Ok(VmcMessage::RequestLoadConfig {
        path: arg_string(message, 0)?,
    })
}

fn parse_shortcut(message: &OscMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 1)?;
    Ok(VmcMessage::Shortcut {
        shortcut: arg_string(message, 0)?,
    })
}

fn parse_no_args(message: &OscMessage, parsed: VmcMessage) -> Result<VmcMessage, VmcError> {
    expect_arg_len(message, 0)?;
    Ok(parsed)
}

fn transform_at(message: &OscMessage, start: usize) -> Result<VmcTransform, VmcError> {
    Ok(VmcTransform {
        translation: vec3_at(message, start)?,
        rotation: quat_at(message, start + 3)?,
    })
}

fn vec3_at(message: &OscMessage, start: usize) -> Result<Vec3, VmcError> {
    Ok(Vec3::new(
        arg_f32(message, start)?,
        arg_f32(message, start + 1)?,
        arg_f32(message, start + 2)?,
    ))
}

fn quat_at(message: &OscMessage, start: usize) -> Result<Quat, VmcError> {
    let rotation = Quat::from_xyzw(
        arg_f32(message, start)?,
        arg_f32(message, start + 1)?,
        arg_f32(message, start + 2)?,
        arg_f32(message, start + 3)?,
    );
    if !rotation.x.is_finite()
        || !rotation.y.is_finite()
        || !rotation.z.is_finite()
        || !rotation.w.is_finite()
        || rotation.length_squared() <= f32::EPSILON
    {
        return Err(VmcError::InvalidQuaternion {
            address: message.addr.clone(),
            index: start,
        });
    }
    Ok(rotation.normalize())
}

fn optional_vec3(message: &OscMessage, start: usize) -> Result<Option<Vec3>, VmcError> {
    if message.args.len() <= start {
        return Ok(None);
    }
    Ok(Some(vec3_at(message, start)?))
}

fn optional_color(message: &OscMessage, start: usize) -> Result<Option<[f32; 4]>, VmcError> {
    if message.args.len() <= start {
        return Ok(None);
    }
    Ok(Some([
        arg_f32(message, start)?,
        arg_f32(message, start + 1)?,
        arg_f32(message, start + 2)?,
        arg_f32(message, start + 3)?,
    ]))
}

fn optional_i32(message: &OscMessage, index: usize) -> Result<Option<i32>, VmcError> {
    if message.args.len() <= index {
        return Ok(None);
    }
    Ok(Some(arg_i32(message, index)?))
}

fn optional_string(message: &OscMessage, index: usize) -> Result<Option<String>, VmcError> {
    if message.args.len() <= index {
        return Ok(None);
    }
    Ok(Some(arg_string(message, index)?))
}

fn optional_f32(message: &OscMessage, index: usize) -> Result<Option<f32>, VmcError> {
    if message.args.len() <= index {
        return Ok(None);
    }
    Ok(Some(arg_f32(message, index)?))
}

fn arg_string(message: &OscMessage, index: usize) -> Result<String, VmcError> {
    match arg(message, index)? {
        OscType::String(value) => Ok(value.clone()),
        actual => Err(VmcError::WrongArgType {
            address: message.addr.clone(),
            index,
            expected: "string",
            actual: arg_kind(actual),
        }),
    }
}

fn arg_i32(message: &OscMessage, index: usize) -> Result<i32, VmcError> {
    match arg(message, index)? {
        OscType::Int(value) => Ok(*value),
        actual => Err(VmcError::WrongArgType {
            address: message.addr.clone(),
            index,
            expected: "int",
            actual: arg_kind(actual),
        }),
    }
}

fn arg_bool_int(message: &OscMessage, index: usize) -> Result<bool, VmcError> {
    Ok(arg_i32(message, index)? != 0)
}

fn arg_input_state(message: &OscMessage, index: usize) -> Result<VmcInputState, VmcError> {
    Ok(VmcInputState::from_raw(arg_i32(message, index)?))
}

fn arg_f32(message: &OscMessage, index: usize) -> Result<f32, VmcError> {
    match arg(message, index)? {
        OscType::Float(value) => Ok(*value),
        OscType::Double(value) => Ok(*value as f32),
        actual => Err(VmcError::WrongArgType {
            address: message.addr.clone(),
            index,
            expected: "float",
            actual: arg_kind(actual),
        }),
    }
}

fn arg(message: &OscMessage, index: usize) -> Result<&OscType, VmcError> {
    message.args.get(index).ok_or_else(|| VmcError::MissingArg {
        address: message.addr.clone(),
        index,
    })
}

fn expect_arg_len(message: &OscMessage, expected: usize) -> Result<(), VmcError> {
    if message.args.len() == expected {
        Ok(())
    } else {
        Err(VmcError::ArgCount {
            address: message.addr.clone(),
            expected,
            actual: message.args.len(),
        })
    }
}

fn transform_args(name: &str, transform: VmcTransform) -> Vec<OscType> {
    let mut args = vec![OscType::String(name.to_owned())];
    args.extend(vec3_args(transform.translation));
    args.extend(quat_args(transform.rotation));
    args
}

fn vec3_args(value: Vec3) -> Vec<OscType> {
    vec![
        OscType::Float(value.x),
        OscType::Float(value.y),
        OscType::Float(value.z),
    ]
}

fn quat_args(value: Quat) -> Vec<OscType> {
    vec![
        OscType::Float(value.x),
        OscType::Float(value.y),
        OscType::Float(value.z),
        OscType::Float(value.w),
    ]
}

fn osc_message(addr: &str, args: Vec<OscType>) -> OscMessage {
    OscMessage {
        addr: addr.to_owned(),
        args,
    }
}

fn bool_int(value: bool) -> i32 {
    i32::from(value)
}

fn arg_kind(arg: &OscType) -> &'static str {
    match arg {
        OscType::Int(_) => "int",
        OscType::Float(_) => "float",
        OscType::String(_) => "string",
        OscType::Blob(_) => "blob",
        OscType::Time(_) => "time",
        OscType::Long(_) => "long",
        OscType::Double(_) => "double",
        OscType::Char(_) => "char",
        OscType::Color(_) => "color",
        OscType::Midi(_) => "midi",
        OscType::Bool(_) => "bool",
        OscType::Array(_) => "array",
        OscType::Nil => "nil",
        OscType::Inf => "infinitum",
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VmcError {
    #[error("{address} missing argument {index}")]
    MissingArg { address: String, index: usize },
    #[error("{address} expected {expected} arguments, got {actual}")]
    ArgCount {
        address: String,
        expected: usize,
        actual: usize,
    },
    #[error("{address} argument {index} expected {expected}, got {actual}")]
    WrongArgType {
        address: String,
        index: usize,
        expected: &'static str,
        actual: &'static str,
    },
    #[error("{address} quaternion at argument {index} is not finite or has zero length")]
    InvalidQuaternion { address: String, index: usize },
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VmcApplyError<E> {
    #[error(transparent)]
    Parse(#[from] VmcError),
    #[error("VMC sink error: {0}")]
    Sink(E),
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum VmcTransportError {
    #[error(transparent)]
    Parse(#[from] VmcError),
    #[error("VMC packet context receive time is not finite: {received_at_seconds}")]
    InvalidReceiveTime { received_at_seconds: f64 },
    #[error("VMC transport policy requires a sender id")]
    MissingSender,
    #[error("VMC sender is not allowed: {sender:?}")]
    UnauthorizedSender { sender: VmcSenderId },
    #[error("VMC packet has too many messages: max {max}, actual {actual}")]
    TooManyMessages { max: usize, actual: usize },
    #[error("VMC receive time moved backwards: previous {previous}, actual {actual}")]
    ReceiveTimeRewind { previous: f64, actual: f64 },
    #[error(
        "VMC sender is rate limited: minimum interval {min_interval_seconds}, elapsed {elapsed_seconds}"
    )]
    RateLimited {
        min_interval_seconds: f64,
        elapsed_seconds: f64,
    },
    #[error("VMC relative time is not finite: {time}")]
    InvalidRelativeTime { time: f32 },
    #[error("VMC relative time moved backwards: previous {previous}, actual {actual}")]
    RelativeTimeRewind { previous: f32, actual: f32 },
    #[error("VMC relative time step is too large: max {max_step}, actual {step}")]
    RelativeTimeStepTooLarge { max_step: f32, step: f32 },
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum VmcTransportApplyError<E> {
    #[error(transparent)]
    Transport(#[from] VmcTransportError),
    #[error("VMC sink error: {0}")]
    Sink(E),
}

impl<E> VmcTransportApplyError<E> {
    fn from_apply(error: VmcApplyError<E>) -> Self {
        match error {
            VmcApplyError::Parse(error) => Self::Transport(VmcTransportError::Parse(error)),
            VmcApplyError::Sink(error) => Self::Sink(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use vrm_osc::{OscBundle, OscTime};

    #[test]
    fn parses_and_roundtrips_bone_pose_message() {
        let message = OscMessage {
            addr: "/VMC/Ext/Bone/Pos".to_owned(),
            args: vec![
                OscType::String("Head".to_owned()),
                OscType::Float(1.0),
                OscType::Float(2.0),
                OscType::Float(3.0),
                OscType::Float(0.0),
                OscType::Float(0.0),
                OscType::Float(0.0),
                OscType::Float(1.0),
            ],
        };

        let parsed = VmcMessage::from_osc_message(&message).unwrap();

        assert_eq!(
            parsed,
            VmcMessage::BonePose {
                bone: "Head".to_owned(),
                transform: VmcTransform {
                    translation: Vec3::new(1.0, 2.0, 3.0),
                    rotation: Quat::IDENTITY,
                },
            }
        );
        assert_eq!(parsed.to_osc_message(), message);
    }

    #[test]
    fn roundtrips_vmc31_marionette_and_performer_message_families() {
        let transform = VmcTransform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::IDENTITY,
        };
        let messages = vec![
            VmcMessage::Available {
                available: true,
                calibration_state: Some(3),
                calibration_mode: Some(1),
                tracking_status: Some(1),
            },
            VmcMessage::RelativeTime(1.25),
            VmcMessage::RootPose {
                name: "root".to_owned(),
                transform,
                scale: Some(Vec3::splat(1.2)),
                offset: Some(Vec3::new(0.0, 1.0, 0.0)),
            },
            VmcMessage::BonePose {
                bone: "Head".to_owned(),
                transform,
            },
            VmcMessage::BlendValue {
                name: "Joy".to_owned(),
                value: 0.8,
            },
            VmcMessage::BlendApply,
            VmcMessage::CameraPose {
                name: "Camera".to_owned(),
                transform,
                fov_y_degrees: Some(45.0),
            },
            VmcMessage::DirectionalLight {
                name: "Light".to_owned(),
                transform,
                color: Some([1.0, 0.8, 0.6, 1.0]),
            },
            VmcMessage::ControllerInput {
                state: VmcInputState::Axis,
                name: "trigger".to_owned(),
                is_left: true,
                is_touch: false,
                is_axis: true,
                axis: Vec3::new(0.1, 0.2, 0.3),
            },
            VmcMessage::KeyboardInput {
                state: VmcInputState::Press,
                name: "Space".to_owned(),
                keycode: 32,
            },
            VmcMessage::MidiNote {
                state: VmcInputState::Press,
                channel: 1,
                note: 64,
                velocity: 0.7,
            },
            VmcMessage::MidiCcValue {
                knob: 7,
                value: 0.5,
            },
            VmcMessage::MidiCcButton {
                knob: 8,
                state: VmcInputState::Release,
            },
            VmcMessage::DevicePose {
                kind: VmcDeviceKind::Hmd,
                local: false,
                serial: "hmd-1".to_owned(),
                transform,
            },
            VmcMessage::DevicePose {
                kind: VmcDeviceKind::Tracker,
                local: true,
                serial: "tracker-1".to_owned(),
                transform,
            },
            VmcMessage::ReceiveEnabled {
                enabled: true,
                port: 39540,
                ip_address: Some("127.0.0.1".to_owned()),
            },
            VmcMessage::LocalVrmInfo {
                path: "avatar.vrm".to_owned(),
                title: "Avatar".to_owned(),
                hash: Some("abc123".to_owned()),
            },
            VmcMessage::RemoteVrmInfo {
                service: "vroidhub".to_owned(),
                json: "{\"characterModelId\":\"1\"}".to_owned(),
            },
            VmcMessage::OptionString("arbitrary".to_owned()),
            VmcMessage::SettingColor([0.1, 0.2, 0.3, 1.0]),
            VmcMessage::WindowAttribute {
                is_top_most: true,
                is_transparent: true,
                window_click_through: false,
                hide_border: true,
            },
            VmcMessage::ConfigPath("profile.json".to_owned()),
            VmcMessage::SetPeriod {
                status: 1,
                root: 2,
                bone: 3,
                blend_shape: 4,
                camera: 5,
                devices: 6,
            },
            VmcMessage::EyeTrackingTarget {
                enabled: true,
                position: Vec3::new(0.0, 0.1, 1.0),
            },
            VmcMessage::InformationRequest,
            VmcMessage::ResponseString("ok".to_owned()),
            VmcMessage::CalibrationReady,
            VmcMessage::CalibrationExecute { mode: 2 },
            VmcMessage::RequestLoadConfig {
                path: "next-profile.json".to_owned(),
            },
            VmcMessage::Shortcut {
                shortcut: "Functions.FreeCamera".to_owned(),
            },
            VmcMessage::Thru(OscMessage {
                addr: "/VMC/Thru/vendor/topic".to_owned(),
                args: vec![OscType::String("payload".to_owned())],
            }),
        ];

        for message in messages {
            let osc = message.to_osc_message();
            assert_eq!(VmcMessage::from_osc_message(&osc).unwrap(), message);
        }
    }

    #[test]
    fn parses_legacy_camera_and_light_aliases_but_emits_official_addresses() {
        let camera = OscMessage {
            addr: "/VMC/Ext/Camera/Pos".to_owned(),
            args: transform_args("Camera", VmcTransform::default()),
        };
        let light = OscMessage {
            addr: "/VMC/Ext/Light/Dir".to_owned(),
            args: transform_args("Light", VmcTransform::default()),
        };

        assert_eq!(
            VmcMessage::from_osc_message(&camera)
                .unwrap()
                .to_osc_message()
                .addr,
            "/VMC/Ext/Cam"
        );
        assert_eq!(
            VmcMessage::from_osc_message(&light)
                .unwrap()
                .to_osc_message()
                .addr,
            "/VMC/Ext/Light"
        );
    }

    #[test]
    fn bundle_application_preserves_wire_order_and_applies_expressions() {
        let packet = OscPacket::Bundle(OscBundle {
            timetag: OscTime::IMMEDIATE,
            content: vec![
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/Blend/Val".to_owned(),
                    args: vec![OscType::String("blink".to_owned()), OscType::Float(0.75)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/Blend/Apply".to_owned(),
                    args: Vec::new(),
                }),
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/T".to_owned(),
                    args: vec![OscType::Float(1.25)],
                }),
            ],
        });
        let mut sink = RecordingSink::default();

        apply_packet(&mut sink, &packet).unwrap();

        assert_eq!(
            sink.events,
            vec!["begin", "expr:blink=0.75", "apply", "time:1.25", "commit"]
        );
    }

    #[test]
    fn parses_all_messages_before_starting_sink_transaction() {
        let packet = OscPacket::Bundle(OscBundle {
            timetag: OscTime::IMMEDIATE,
            content: vec![
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/T".to_owned(),
                    args: vec![OscType::Float(1.0)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/Blend/Apply".to_owned(),
                    args: vec![OscType::Int(9)],
                }),
            ],
        });
        let mut sink = RecordingSink::default();

        assert!(matches!(
            apply_packet(&mut sink, &packet),
            Err(VmcApplyError::Parse(VmcError::ArgCount { .. }))
        ));
        assert!(sink.events.is_empty());
    }

    #[test]
    fn lenient_parse_policy_skips_invalid_known_messages_before_transaction() {
        let packet = OscPacket::Bundle(OscBundle {
            timetag: OscTime::IMMEDIATE,
            content: vec![
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/T".to_owned(),
                    args: vec![OscType::Float(1.0)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/Blend/Apply".to_owned(),
                    args: vec![OscType::Int(9)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Thru/vendor/topic".to_owned(),
                    args: vec![OscType::String("kept".to_owned())],
                }),
            ],
        });
        let mut sink = RecordingSink::default();

        apply_packet_with_policy(
            &mut sink,
            &packet,
            VmcParsePolicy::IgnoreInvalidKnownMessages,
        )
        .unwrap();

        assert_eq!(
            sink.events,
            vec!["begin", "time:1", "thru:/VMC/Thru/vendor/topic", "commit"]
        );
    }

    #[test]
    fn zero_length_quaternion_is_invalid_and_lenient_policy_can_skip_it() {
        let invalid_bone = OscMessage {
            addr: "/VMC/Ext/Bone/Pos".to_owned(),
            args: vec![
                OscType::String("Head".to_owned()),
                OscType::Float(0.0),
                OscType::Float(0.0),
                OscType::Float(0.0),
                OscType::Float(0.0),
                OscType::Float(0.0),
                OscType::Float(0.0),
                OscType::Float(0.0),
            ],
        };
        assert!(matches!(
            VmcMessage::from_osc_message(&invalid_bone),
            Err(VmcError::InvalidQuaternion { .. })
        ));

        let packet = OscPacket::Bundle(OscBundle {
            timetag: OscTime::IMMEDIATE,
            content: vec![
                OscPacket::Message(invalid_bone),
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/T".to_owned(),
                    args: vec![OscType::Float(2.0)],
                }),
            ],
        });
        let messages = collect_packet_messages_with_policy(
            &packet,
            VmcParsePolicy::IgnoreInvalidKnownMessages,
        )
        .unwrap();

        assert_eq!(messages, vec![VmcMessage::RelativeTime(2.0)]);
    }

    #[test]
    fn sink_error_rolls_back_transaction() {
        let messages = vec![VmcMessage::RelativeTime(1.0)];
        let mut sink = FailingSink::default();

        assert!(matches!(
            apply_messages(&mut sink, &messages),
            Err(VmcApplyError::Sink("time failed"))
        ));
        assert_eq!(sink.events, vec!["begin", "time", "rollback"]);
    }

    #[test]
    fn extended_messages_apply_to_runtime_sink_in_transaction_order() {
        let transform = VmcTransform::default();
        let messages = vec![
            VmcMessage::DevicePose {
                kind: VmcDeviceKind::Controller,
                local: true,
                serial: "left".to_owned(),
                transform,
            },
            VmcMessage::ControllerInput {
                state: VmcInputState::Axis,
                name: "stick".to_owned(),
                is_left: true,
                is_touch: true,
                is_axis: true,
                axis: Vec3::new(1.0, 0.0, 0.0),
            },
            VmcMessage::MidiCcButton {
                knob: 10,
                state: VmcInputState::Press,
            },
            VmcMessage::EyeTrackingTarget {
                enabled: true,
                position: Vec3::new(0.0, 0.0, 1.0),
            },
            VmcMessage::Shortcut {
                shortcut: "Functions.ColorGreen".to_owned(),
            },
            VmcMessage::Thru(OscMessage {
                addr: "/VMC/Thru/vendor/topic".to_owned(),
                args: vec![OscType::Int(7)],
            }),
        ];
        let mut sink = RecordingSink::default();

        apply_messages(&mut sink, &messages).unwrap();

        assert_eq!(
            sink.events,
            vec![
                "begin",
                "device:Controller:true:left",
                "controller:stick:2:true:true:true:1",
                "midi-bit:10:1",
                "eye:true:1",
                "shortcut:Functions.ColorGreen",
                "thru:/VMC/Thru/vendor/topic",
                "commit",
            ]
        );
    }

    #[test]
    fn transport_gate_enforces_sender_allow_list_and_message_limit_before_apply() {
        let packet = OscPacket::Bundle(OscBundle {
            timetag: OscTime::IMMEDIATE,
            content: vec![
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/T".to_owned(),
                    args: vec![OscType::Float(1.0)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/Blend/Apply".to_owned(),
                    args: Vec::new(),
                }),
            ],
        });
        let policy = VmcTransportPolicy::open()
            .allow_sender("trusted")
            .max_messages_per_packet(1);
        let mut gate = VmcTransportGate::new(policy);

        assert!(matches!(
            gate.accept_packet(VmcPacketContext::anonymous(0.0), &packet),
            Err(VmcTransportError::MissingSender)
        ));
        assert!(matches!(
            gate.accept_packet(VmcPacketContext::from_sender("other", 0.0), &packet),
            Err(VmcTransportError::UnauthorizedSender { sender })
                if sender.as_str() == "other"
        ));
        assert!(matches!(
            gate.accept_packet(VmcPacketContext::from_sender("trusted", 0.0), &packet),
            Err(VmcTransportError::TooManyMessages { max: 1, actual: 2 })
        ));
    }

    #[test]
    fn transport_gate_rate_limits_per_sender_and_tracks_relative_time() {
        let packet = OscPacket::Message(OscMessage {
            addr: "/VMC/Ext/T".to_owned(),
            args: vec![OscType::Float(1.0)],
        });
        let policy = VmcTransportPolicy::open()
            .min_packet_interval_seconds(0.1)
            .max_relative_time_step_seconds(0.5);
        let mut gate = VmcTransportGate::new(policy);

        let accepted = gate
            .accept_packet(VmcPacketContext::from_sender("sender-a", 1.0), &packet)
            .unwrap();
        assert_eq!(accepted.report.message_count, 1);
        assert_eq!(accepted.report.relative_time, Some(1.0));

        assert!(matches!(
            gate.accept_packet(VmcPacketContext::from_sender("sender-a", 1.05), &packet),
            Err(VmcTransportError::RateLimited { .. })
        ));

        let other_sender = gate
            .accept_packet(VmcPacketContext::from_sender("sender-b", 1.05), &packet)
            .unwrap();
        assert_eq!(
            other_sender.report.sender.as_ref().map(VmcSenderId::as_str),
            Some("sender-b")
        );

        let jump = OscPacket::Message(OscMessage {
            addr: "/VMC/Ext/T".to_owned(),
            args: vec![OscType::Float(2.0)],
        });
        assert!(matches!(
            gate.accept_packet(VmcPacketContext::from_sender("sender-a", 1.2), &jump),
            Err(VmcTransportError::RelativeTimeStepTooLarge {
                max_step,
                step
            }) if (max_step - 0.5).abs() < 0.0001 && (step - 1.0).abs() < 0.0001
        ));
    }

    #[test]
    fn transport_gate_rejects_relative_time_rewind_before_sink_transaction() {
        let mut gate = VmcTransportGate::default();
        let first = OscPacket::Message(OscMessage {
            addr: "/VMC/Ext/T".to_owned(),
            args: vec![OscType::Float(5.0)],
        });
        let rewind = OscPacket::Message(OscMessage {
            addr: "/VMC/Ext/T".to_owned(),
            args: vec![OscType::Float(4.0)],
        });
        let mut sink = RecordingSink::default();

        gate.apply_packet(&mut sink, VmcPacketContext::anonymous(10.0), &first)
            .unwrap();
        assert!(matches!(
            gate.apply_packet(&mut sink, VmcPacketContext::anonymous(10.1), &rewind),
            Err(VmcTransportApplyError::Transport(
                VmcTransportError::RelativeTimeRewind {
                    previous: 5.0,
                    actual: 4.0
                }
            ))
        ));
        assert_eq!(sink.events, vec!["begin", "time:5", "commit"]);
    }

    #[test]
    fn transport_gate_lenient_parse_policy_still_applies_all_or_nothing() {
        let packet = OscPacket::Bundle(OscBundle {
            timetag: OscTime::IMMEDIATE,
            content: vec![
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/Blend/Apply".to_owned(),
                    args: vec![OscType::Int(99)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/VMC/Ext/T".to_owned(),
                    args: vec![OscType::Float(3.0)],
                }),
            ],
        });
        let policy =
            VmcTransportPolicy::open().parse_policy(VmcParsePolicy::IgnoreInvalidKnownMessages);
        let mut gate = VmcTransportGate::new(policy);
        let mut sink = RecordingSink::default();

        let report = gate
            .apply_packet(&mut sink, VmcPacketContext::anonymous(0.0), &packet)
            .unwrap();

        assert_eq!(report.message_count, 1);
        assert_eq!(report.relative_time, Some(3.0));
        assert_eq!(sink.events, vec!["begin", "time:3", "commit"]);
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Vec<String>,
    }

    impl VmcRuntimeSink for RecordingSink {
        type Error = Infallible;

        fn begin_vmc_transaction(&mut self) -> Result<(), Self::Error> {
            self.events.push("begin".to_owned());
            Ok(())
        }

        fn commit_vmc_transaction(&mut self) -> Result<(), Self::Error> {
            self.events.push("commit".to_owned());
            Ok(())
        }

        fn set_relative_time(&mut self, time: f32) -> Result<(), Self::Error> {
            self.events.push(format!("time:{time}"));
            Ok(())
        }

        fn set_expression_value(&mut self, name: &str, value: f32) -> Result<(), Self::Error> {
            self.events.push(format!("expr:{name}={value}"));
            Ok(())
        }

        fn apply_expressions(&mut self) -> Result<(), Self::Error> {
            self.events.push("apply".to_owned());
            Ok(())
        }

        fn set_controller_input(
            &mut self,
            state: VmcInputState,
            name: &str,
            is_left: bool,
            is_touch: bool,
            is_axis: bool,
            axis: Vec3,
        ) -> Result<(), Self::Error> {
            self.events.push(format!(
                "controller:{name}:{}:{is_left}:{is_touch}:{is_axis}:{}",
                state.raw(),
                axis.x
            ));
            Ok(())
        }

        fn set_midi_cc_button(
            &mut self,
            knob: i32,
            state: VmcInputState,
        ) -> Result<(), Self::Error> {
            self.events.push(format!("midi-bit:{knob}:{}", state.raw()));
            Ok(())
        }

        fn set_device_pose(
            &mut self,
            kind: VmcDeviceKind,
            local: bool,
            serial: &str,
            _transform: VmcTransform,
        ) -> Result<(), Self::Error> {
            self.events
                .push(format!("device:{kind:?}:{local}:{serial}"));
            Ok(())
        }

        fn set_eye_tracking_target(
            &mut self,
            enabled: bool,
            position: Vec3,
        ) -> Result<(), Self::Error> {
            self.events.push(format!("eye:{enabled}:{}", position.z));
            Ok(())
        }

        fn call_shortcut(&mut self, shortcut: &str) -> Result<(), Self::Error> {
            self.events.push(format!("shortcut:{shortcut}"));
            Ok(())
        }

        fn thru_vmc_message(&mut self, message: &OscMessage) -> Result<(), Self::Error> {
            self.events.push(format!("thru:{}", message.addr));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FailingSink {
        events: Vec<&'static str>,
    }

    impl VmcRuntimeSink for FailingSink {
        type Error = &'static str;

        fn begin_vmc_transaction(&mut self) -> Result<(), Self::Error> {
            self.events.push("begin");
            Ok(())
        }

        fn rollback_vmc_transaction(&mut self) {
            self.events.push("rollback");
        }

        fn set_relative_time(&mut self, _time: f32) -> Result<(), Self::Error> {
            self.events.push("time");
            Err("time failed")
        }
    }
}
