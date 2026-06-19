//! Typed VMC message layer over `vrm-osc`.
//!
//! The crate keeps transport at the edge: callers decode UDP/TCP with
//! `vrm-osc`, then use [`VmcMessage::from_osc_message`] or [`apply_packet`].

use glam::{Quat, Vec3};
use thiserror::Error;
use vrm_osc::{OscMessage, OscPacket, OscType};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VmcTransform {
    pub translation: Vec3,
    pub rotation: Quat,
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
            "/VMC/Ext/Camera/Pos" => parse_camera_pose(message),
            "/VMC/Ext/Light/Dir" => parse_light(message),
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
                osc_message("/VMC/Ext/Camera/Pos", args)
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
                osc_message("/VMC/Ext/Light/Dir", args)
            }
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

    fn unknown_vmc_message(&mut self, _message: &OscMessage) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub fn collect_packet_messages(packet: &OscPacket) -> Result<Vec<VmcMessage>, VmcError> {
    let mut messages = Vec::new();
    collect_packet_messages_into(packet, &mut messages)?;
    Ok(messages)
}

pub fn apply_packet<S>(sink: &mut S, packet: &OscPacket) -> Result<(), VmcApplyError<S::Error>>
where
    S: VmcRuntimeSink,
{
    let messages = collect_packet_messages(packet)?;
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
        VmcMessage::Unknown(message) => sink
            .unknown_vmc_message(message)
            .map_err(VmcApplyError::Sink),
    }
}

fn collect_packet_messages_into(
    packet: &OscPacket,
    messages: &mut Vec<VmcMessage>,
) -> Result<(), VmcError> {
    match packet {
        OscPacket::Message(message) => {
            messages.push(VmcMessage::from_osc_message(message)?);
        }
        OscPacket::Bundle(bundle) => {
            for packet in &bundle.content {
                collect_packet_messages_into(packet, messages)?;
            }
        }
    }
    Ok(())
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
    Ok(Quat::from_xyzw(
        arg_f32(message, start)?,
        arg_f32(message, start + 1)?,
        arg_f32(message, start + 2)?,
        arg_f32(message, start + 3)?,
    )
    .normalize())
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
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VmcApplyError<E> {
    #[error(transparent)]
    Parse(#[from] VmcError),
    #[error("VMC sink error: {0}")]
    Sink(E),
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
    fn sink_error_rolls_back_transaction() {
        let messages = vec![VmcMessage::RelativeTime(1.0)];
        let mut sink = FailingSink::default();

        assert!(matches!(
            apply_messages(&mut sink, &messages),
            Err(VmcApplyError::Sink("time failed"))
        ));
        assert_eq!(sink.events, vec!["begin", "time", "rollback"]);
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
