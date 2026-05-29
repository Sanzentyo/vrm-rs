use glam::Mat4;
use std::error::Error;
use vrm_adapter::{
    HeadlessSceneState, SpringRestMap, VrmRuntimeDriver, WorldMatrixAccess, WorldTransformUpdate,
};
use vrm_core::{Feature, NodeRef};
use vrm_io::LoadedVrm;
use vrm_runtime::{DeltaTime, Runtime};

pub fn runtime_world_matrices(loaded: &LoadedVrm) -> Result<Vec<Mat4>, Box<dyn Error>> {
    let mut scene = HeadlessSceneState::default();

    for (index, node) in loaded.scene.nodes.iter().enumerate() {
        scene.insert_node(NodeRef(index), node.local);
    }
    for (index, node) in loaded.scene.nodes.iter().enumerate() {
        scene.set_parent(NodeRef(index), node.parent.map(NodeRef))?;
    }
    scene.update_world_transforms()?;

    let document = loaded.model().document();
    let mut runtime = Runtime::from_document(document);
    let events = runtime.update(DeltaTime(0.0))?;
    let root = loaded
        .scene
        .nodes
        .iter()
        .position(|node| node.parent.is_none())
        .map(NodeRef);
    let mut driver = VrmRuntimeDriver::new(document).with_runtime_events(&events);
    if let Some(root) = root {
        driver = driver.with_root(root);
    }

    match &document.spring_bone {
        Feature::Present(system) => {
            let rest = SpringRestMap::capture(&scene, system)?;
            let mut state = rest.runtime_state(system);
            driver.tick_with_spring_parity(&mut scene, Some((&rest, &mut state)))?;
        }
        Feature::Absent => {
            driver.tick_with_spring_parity(&mut scene, None)?;
        }
    }
    scene.update_world_transforms()?;

    loaded
        .scene
        .nodes
        .iter()
        .enumerate()
        .map(|(index, _)| scene.world_matrix(NodeRef(index)).map_err(Into::into))
        .collect()
}
