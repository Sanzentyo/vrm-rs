//! Ergonomic facade for the `vrm-rs` workspace.
//!
//! The lower crates are intentionally split into protocol, sans-IO conversion,
//! IO, runtime, and adapter layers. This crate re-exports the stable entry
//! points most applications need first.
//!
//! ```no_run
//! # fn main() -> Result<(), vrm_rs::VrmIoError> {
//! let mut vrm = vrm_rs::load_runtime_path("avatar.vrm")?;
//! let events = vrm.update(vrm_rs::DeltaTime(1.0 / 60.0)).unwrap();
//! let driver = vrm.driver_with_events(&events);
//!
//! assert_eq!(driver.document.meta.name, vrm.document().meta.name);
//! # Ok(())
//! # }
//! ```

pub use vrm_adapter as adapter;
pub use vrm_core as core;
pub use vrm_io as io;
pub use vrm_protocol as protocol;
pub use vrm_runtime as runtime;
pub use vrm_sans_io as sans_io;

use std::path::Path;

pub use vrm_adapter::VrmRuntimeDriver;
pub use vrm_core::{Parsed, Raw, Resolved, Validated, VrmAsset, VrmDocument, VrmModel};
pub use vrm_io::{LoadedVrm, VrmIoError, load_vrm_from_path, load_vrm_from_slice};
pub use vrm_runtime::{DeltaTime, Runtime, RuntimeEvents};
pub use vrm_sans_io::{BuildError, ValidatedAssetBuilder};

/// Concrete resolved model type returned by the facade loaders.
pub type ResolvedVrmModel = VrmModel<Resolved>;

/// Loaded VRM/VRMA data plus renderer-agnostic runtime state.
///
/// This type is the root crate's ergonomic path for applications that want to
/// load once, inspect the resolved model, update runtime managers, and then
/// build an adapter `VrmRuntimeDriver` for their engine scene.
#[derive(Clone, Debug)]
pub struct Vrm {
    loaded: LoadedVrm,
    runtime: Runtime,
}

impl Vrm {
    /// Build a facade session from an already loaded IO result.
    pub fn from_loaded(loaded: LoadedVrm) -> Self {
        let runtime = Runtime::from_document(loaded.model().document());
        Self { loaded, runtime }
    }

    /// Parse, validate, resolve, and initialize runtime state from bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, VrmIoError> {
        load_full(bytes).map(Self::from_loaded)
    }

    /// Parse, validate, resolve, and initialize runtime state from a path.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, VrmIoError> {
        load_full_path(path).map(Self::from_loaded)
    }

    /// Access the complete IO result, including rest scene, buffers, images,
    /// warnings, and resolved model.
    pub fn loaded(&self) -> &LoadedVrm {
        &self.loaded
    }

    /// Consume the session and return the complete IO result.
    pub fn into_loaded(self) -> LoadedVrm {
        self.loaded
    }

    /// Access the resolved model.
    pub fn model(&self) -> &VrmModel<Resolved> {
        self.loaded.model()
    }

    /// Access the resolved document.
    pub fn document(&self) -> &VrmDocument {
        self.model().document()
    }

    /// Access renderer-independent rest scene data extracted from glTF.
    pub fn scene(&self) -> &vrm_io::GltfSceneRest {
        self.loaded.scene()
    }

    /// Access runtime managers.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Mutably access runtime managers for expression weight updates.
    pub fn runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    /// Update runtime managers and return renderer-agnostic events.
    pub fn update(&mut self, delta: DeltaTime) -> Result<RuntimeEvents, runtime::RuntimeError> {
        self.runtime.update(delta)
    }

    /// Create a high-level adapter driver bound to this document.
    pub fn driver(&self) -> VrmRuntimeDriver<'_> {
        driver_for(self.model())
    }

    /// Create a high-level adapter driver with a precomputed runtime event set.
    pub fn driver_with_events<'a>(&'a self, events: &'a RuntimeEvents) -> VrmRuntimeDriver<'a> {
        self.driver().with_runtime_events(events)
    }
}

/// Parse, validate, and resolve a VRM/VRMA payload from a `.gltf`, `.glb`,
/// `.vrm`, or `.vrma` byte slice.
pub fn load(bytes: &[u8]) -> Result<VrmModel<Resolved>, VrmIoError> {
    load_vrm_from_slice(bytes).map(LoadedVrm::into_model)
}

/// Parse, validate, and resolve a VRM/VRMA payload while keeping IO details.
pub fn load_full(bytes: &[u8]) -> Result<LoadedVrm, VrmIoError> {
    load_vrm_from_slice(bytes)
}

/// Parse, validate, and resolve a VRM/VRMA file while keeping IO details.
pub fn load_full_path(path: impl AsRef<Path>) -> Result<LoadedVrm, VrmIoError> {
    load_vrm_from_path(path)
}

/// Parse, validate, resolve, and initialize runtime state from bytes.
pub fn load_runtime(bytes: &[u8]) -> Result<Vrm, VrmIoError> {
    Vrm::from_slice(bytes)
}

/// Parse, validate, resolve, and initialize runtime state from a path.
pub fn load_runtime_path(path: impl AsRef<Path>) -> Result<Vrm, VrmIoError> {
    Vrm::from_path(path)
}

/// Create renderer-agnostic runtime state for a resolved model.
pub fn runtime_for(model: &VrmModel<Resolved>) -> Runtime {
    Runtime::from_document(model.document())
}

/// Create a high-level adapter driver for a resolved model.
pub fn driver_for(model: &VrmModel<Resolved>) -> VrmRuntimeDriver<'_> {
    VrmRuntimeDriver::new(model.document())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs};

    #[test]
    fn facade_load_reports_invalid_payload() {
        let err = load(b"not gltf").unwrap_err();
        assert!(matches!(err, VrmIoError::Gltf(_)));
    }

    #[test]
    fn facade_full_and_runtime_loaders_report_invalid_payload() {
        assert!(matches!(load_full(b"not gltf"), Err(VrmIoError::Gltf(_))));
        assert!(matches!(
            load_runtime(b"not gltf"),
            Err(VrmIoError::Gltf(_))
        ));
    }

    #[test]
    fn facade_reexports_state_types() {
        let asset = VrmAsset::<Parsed>::new_parsed(core::VrmDocument::default());
        let model: ResolvedVrmModel = asset.mark_validated().resolve();
        assert_eq!(model.document().kind, core::VrmKind::Vrm1);
    }

    #[test]
    fn facade_builds_runtime_and_driver_for_resolved_models() {
        let asset = VrmAsset::<Parsed>::new_parsed(core::VrmDocument::default());
        let model = asset.mark_validated().resolve();

        let mut runtime = runtime_for(&model);
        let events = runtime.update(DeltaTime(1.0 / 60.0)).unwrap();
        let driver = driver_for(&model).with_runtime_events(&events);

        assert_eq!(events.delta, DeltaTime(1.0 / 60.0));
        assert_eq!(driver.document.kind, core::VrmKind::Vrm1);
        assert!(driver.runtime_events.is_some());
    }

    #[test]
    fn facade_session_exposes_loaded_model_runtime_and_driver() {
        let fixture = generated_vrm1_gltf();
        let bytes = fixture.as_bytes();
        let loaded = load_full(bytes).unwrap();
        let mut session = Vrm::from_loaded(loaded.clone());

        assert_eq!(session.loaded().warnings(), &[]);
        assert_eq!(session.model().document().meta.name, "Facade Fixture");
        assert_eq!(session.document().kind, core::VrmKind::Vrm1);
        assert!(session.scene().node(0).is_some());
        assert_eq!(
            session.runtime().expression_manager.value("blink"),
            None,
            "the fixture has no expressions"
        );
        session
            .runtime_mut()
            .expression_manager
            .set_value("blink", 0.5);

        let events = session.update(DeltaTime(0.016)).unwrap();
        let driver = session.driver_with_events(&events);
        assert!(driver.runtime_events.is_some());
        assert_eq!(driver.document.meta.name, "Facade Fixture");

        let loaded_again = session.into_loaded();
        assert_eq!(loaded_again.model().document().meta.name, "Facade Fixture");
        assert_eq!(loaded_again.scene().nodes.len(), loaded.scene().nodes.len());
    }

    #[test]
    fn facade_path_loaders_initialize_runtime_sessions() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "vrm-rs-facade-fixture-{}-{unique}.gltf",
            std::process::id()
        ));
        fs::write(&path, generated_vrm1_gltf()).unwrap();

        let full = load_full_path(&path).unwrap();
        assert_eq!(full.model().document().meta.name, "Facade Fixture");
        let model = load(fs::read(&path).unwrap().as_slice()).unwrap();
        assert_eq!(model.document().meta.name, "Facade Fixture");
        let runtime = load_runtime_path(&path).unwrap();
        assert_eq!(runtime.document().meta.name, "Facade Fixture");
        let from_path = Vrm::from_path(&path).unwrap();
        assert_eq!(from_path.model().document().kind, core::VrmKind::Vrm1);

        let _ = fs::remove_file(path);
    }

    fn generated_vrm1_gltf() -> String {
        r#"{
            "asset": { "version": "2.0", "generator": "vrm-rs facade test data" },
            "extensionsUsed": ["VRMC_vrm"],
            "scene": 0,
            "scenes": [{ "nodes": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14] }],
            "nodes": [
                { "name": "hips" },
                { "name": "head" },
                { "name": "spine" },
                { "name": "leftUpperLeg" },
                { "name": "leftLowerLeg" },
                { "name": "leftFoot" },
                { "name": "rightUpperLeg" },
                { "name": "rightLowerLeg" },
                { "name": "rightFoot" },
                { "name": "leftUpperArm" },
                { "name": "leftLowerArm" },
                { "name": "leftHand" },
                { "name": "rightUpperArm" },
                { "name": "rightLowerArm" },
                { "name": "rightHand" }
            ],
            "extensions": {
                "VRMC_vrm": {
                    "specVersion": "1.0",
                    "meta": {
                        "name": "Facade Fixture",
                        "authors": ["vrm-rs"]
                    },
                    "humanoid": {
                        "humanBones": {
                            "hips": { "node": 0 },
                            "head": { "node": 1 },
                            "spine": { "node": 2 },
                            "leftUpperLeg": { "node": 3 },
                            "leftLowerLeg": { "node": 4 },
                            "leftFoot": { "node": 5 },
                            "rightUpperLeg": { "node": 6 },
                            "rightLowerLeg": { "node": 7 },
                            "rightFoot": { "node": 8 },
                            "leftUpperArm": { "node": 9 },
                            "leftLowerArm": { "node": 10 },
                            "leftHand": { "node": 11 },
                            "rightUpperArm": { "node": 12 },
                            "rightLowerArm": { "node": 13 },
                            "rightHand": { "node": 14 }
                        }
                    }
                }
            }
        }"#
        .to_owned()
    }
}
