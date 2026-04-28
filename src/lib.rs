//! Ergonomic facade for the `vrm-rs` workspace.
//!
//! The lower crates are intentionally split into protocol, sans-IO conversion,
//! IO, runtime, and adapter layers. This crate re-exports the stable entry
//! points most applications need first.

pub use vrm_adapter as adapter;
pub use vrm_core as core;
pub use vrm_io as io;
pub use vrm_protocol as protocol;
pub use vrm_runtime as runtime;
pub use vrm_sans_io as sans_io;

pub use vrm_core::{Parsed, Raw, Resolved, Validated, VrmAsset, VrmModel};
pub use vrm_io::{LoadedVrm, VrmIoError, load_vrm_from_slice};
pub use vrm_sans_io::{BuildError, ValidatedAssetBuilder};

/// Parse, validate, and resolve a VRM/VRMA payload from a `.gltf`, `.glb`,
/// `.vrm`, or `.vrma` byte slice.
pub fn load(bytes: &[u8]) -> Result<VrmModel<Resolved>, VrmIoError> {
    load_vrm_from_slice(bytes).map(LoadedVrm::into_model)
}
