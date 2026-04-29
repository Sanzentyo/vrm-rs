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
pub use vrm_io::{LoadedVrm, VrmIoError, load_vrm_from_path, load_vrm_from_slice};
pub use vrm_sans_io::{BuildError, ValidatedAssetBuilder};

/// Parse, validate, and resolve a VRM/VRMA payload from a `.gltf`, `.glb`,
/// `.vrm`, or `.vrma` byte slice.
pub fn load(bytes: &[u8]) -> Result<VrmModel<Resolved>, VrmIoError> {
    load_vrm_from_slice(bytes).map(LoadedVrm::into_model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_load_reports_invalid_payload() {
        let err = load(b"not gltf").unwrap_err();
        assert!(matches!(err, VrmIoError::Gltf(_)));
    }

    #[test]
    fn facade_reexports_state_types() {
        let asset = VrmAsset::<Parsed>::new_parsed(core::VrmDocument::default());
        let model: VrmModel<Resolved> = asset.mark_validated().resolve();
        assert_eq!(model.document().kind, core::VrmKind::Vrm1);
    }
}
