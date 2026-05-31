use glam::Mat4;
use std::error::Error;
use vrm_adapter::{
    GltfMaterialAlphaMode, GltfMaterialPipelineOverride, HeadlessSceneState,
    MtoonMaterializationOptions, MtoonRendererPass, RendererMaterialAlphaMode,
    RendererMaterialCullMode, RendererMaterialPipelinePlan, SpringRestMap, VrmRuntimeDriver,
    WorldMatrixAccess, WorldTransformUpdate, mtoon_renderer_material_plans,
};
use vrm_core::{Feature, MaterialRef, NodeRef};
use vrm_io::{GltfAlphaMode, LoadedVrm};
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

pub type CaptureMaterialPlan = RendererMaterialPipelinePlan;
pub type CaptureMaterialCullMode = RendererMaterialCullMode;
pub type CaptureMaterialAlphaMode = RendererMaterialAlphaMode;

pub fn capture_material_plan(loaded: &LoadedVrm, material: Option<usize>) -> CaptureMaterialPlan {
    let material_ref = material.map(MaterialRef);
    let plan = material_ref
        .and_then(|material| {
            mtoon_renderer_material_plans(
                loaded.model().document(),
                MtoonMaterializationOptions::default(),
            )
            .into_iter()
            .find(|plan| plan.material == material && plan.pass == MtoonRendererPass::Base)
        })
        .as_ref()
        .map(RendererMaterialPipelinePlan::from_mtoon_plan)
        .unwrap_or_default();

    if let Some(gltf) = material.and_then(|index| loaded.gltf_materials.get(index)) {
        plan.with_gltf_override(GltfMaterialPipelineOverride {
            alpha_mode: gltf_alpha_mode(gltf.alpha_mode),
            alpha_cutoff: gltf.alpha_cutoff,
            double_sided: gltf.double_sided,
        })
    } else {
        plan
    }
}

fn gltf_alpha_mode(mode: GltfAlphaMode) -> GltfMaterialAlphaMode {
    match mode {
        GltfAlphaMode::Opaque => GltfMaterialAlphaMode::Opaque,
        GltfAlphaMode::Mask => GltfMaterialAlphaMode::Mask,
        GltfAlphaMode::Blend => GltfMaterialAlphaMode::Blend,
    }
}
