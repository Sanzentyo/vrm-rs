//! Dependency-free Open Sound Control (OSC) 1.0 codec for `vrm-rs`.
//!
//! This crate intentionally mirrors the packet and argument model exposed by
//! `rosc` while avoiding `rosc` as a runtime dependency. It supports encoding
//! and decoding all OSC value variants that `rosc` 0.11 exposes: int, float,
//! string, blob, time tag, long, double, char, RGBA color, MIDI, bool, array,
//! nil, and infinitum.
//!
//! The public API is intentionally small:
//!
//! - [`decoder::decode_udp`] decodes one UDP packet.
//! - [`decoder::decode_tcp`] decodes one length-prefixed TCP packet.
//! - [`decoder::decode_tcp_vec`] decodes every complete length-prefixed TCP
//!   packet in a byte slice.
//! - [`encoder::encode`] and [`encoder::encode_tcp`] allocate a fresh `Vec<u8>`.
//! - [`encoder::encode_into`] and [`encoder::encode_into_tcp`] write into a
//!   reusable output such as `Vec<u8>`.
//!
//! ```
//! use vrm_osc::{decoder, encoder, OscMessage, OscPacket, OscType};
//!
//! let packet = OscPacket::Message(OscMessage {
//!     addr: "/avatar/head".to_owned(),
//!     args: vec![OscType::Float(1.0), OscType::String("ok".to_owned())],
//! });
//!
//! let bytes = encoder::encode(&packet)?;
//! let (remainder, decoded) = decoder::decode_udp(&bytes)?;
//! assert!(remainder.is_empty());
//! assert_eq!(decoded, packet);
//! # Ok::<(), vrm_osc::OscError>(())
//! ```

pub mod decoder;
pub mod encoder;
mod types;

pub use types::{
    OscArray, OscBundle, OscColor, OscError, OscMessage, OscMidiMessage, OscPacket, OscTime,
    OscTimeError, OscType, Result,
};
