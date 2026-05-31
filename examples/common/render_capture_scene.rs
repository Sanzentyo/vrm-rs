use glam::Mat4;
use std::error::Error;
use vrm_adapter::{
    HeadlessSceneState, SpringRestMap, VrmRuntimeDriver, WorldMatrixAccess, WorldTransformUpdate,
};
use vrm_core::{Feature, MtoonAlphaMode, MtoonCullMode, NodeRef};
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureMaterialPlan {
    pub render_order: i32,
    pub phase_order: i32,
    pub cull_mode: CaptureMaterialCullMode,
    pub alpha_mode: CaptureMaterialAlphaMode,
    pub depth_write: bool,
    pub blend: bool,
    pub alpha_cutoff: f32,
    pub transparent_order_offset: Option<i32>,
}

impl Default for CaptureMaterialPlan {
    fn default() -> Self {
        Self {
            render_order: 2000,
            phase_order: 2000,
            cull_mode: CaptureMaterialCullMode::Back,
            alpha_mode: CaptureMaterialAlphaMode::Opaque,
            depth_write: true,
            blend: false,
            alpha_cutoff: 0.5,
            transparent_order_offset: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMaterialCullMode {
    Off,
    Front,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMaterialAlphaMode {
    Opaque,
    Mask,
    Blend,
}

pub fn capture_material_plan(loaded: &LoadedVrm, material: Option<usize>) -> CaptureMaterialPlan {
    let mtoon = material
        .and_then(|index| loaded.model().document().materials.get(index))
        .and_then(|material| material.mtoon.as_ref());
    let transparent_order_offset = mtoon.map(mtoon_transparent_order_offset);
    let mut plan = mtoon
        .map(|mtoon| {
            let hints = mtoon.pipeline_hints();
            CaptureMaterialPlan {
                render_order: hints.render_order,
                phase_order: transparent_order_offset.unwrap_or(2000),
                cull_mode: capture_material_cull_mode(hints.cull_mode),
                alpha_mode: capture_material_alpha_mode(hints.alpha_mode),
                depth_write: hints.depth_write,
                blend: hints.blend,
                alpha_cutoff: mtoon.cutoff_factor,
                transparent_order_offset,
            }
        })
        .unwrap_or_default();

    if let Some(gltf) = material.and_then(|index| loaded.gltf_materials.get(index)) {
        match gltf.alpha_mode {
            GltfAlphaMode::Opaque => {}
            GltfAlphaMode::Mask => {
                plan.alpha_mode = CaptureMaterialAlphaMode::Mask;
                plan.depth_write = true;
                plan.blend = false;
                plan.alpha_cutoff = gltf.alpha_cutoff.unwrap_or(0.5);
            }
            GltfAlphaMode::Blend => {
                plan.alpha_mode = CaptureMaterialAlphaMode::Blend;
                plan.depth_write = mtoon.is_some_and(|mtoon| mtoon.transparent_with_z_write);
                plan.blend = true;
                plan.render_order = transparent_order_offset
                    .map_or(plan.render_order.max(3000), |offset| 3000 + offset);
            }
        }
        if gltf.double_sided {
            plan.cull_mode = CaptureMaterialCullMode::Off;
        }
    }

    plan
}

pub fn mtoon_transparent_order_offset(mtoon: &vrm_core::MtoonMaterial) -> i32 {
    let queue_offset = if mtoon.transparent_with_z_write {
        0
    } else {
        19
    };
    queue_offset + mtoon.render_queue_offset_number
}

fn capture_material_cull_mode(mode: MtoonCullMode) -> CaptureMaterialCullMode {
    match mode {
        MtoonCullMode::Off => CaptureMaterialCullMode::Off,
        MtoonCullMode::Front => CaptureMaterialCullMode::Front,
        MtoonCullMode::Back => CaptureMaterialCullMode::Back,
    }
}

fn capture_material_alpha_mode(mode: MtoonAlphaMode) -> CaptureMaterialAlphaMode {
    match mode {
        MtoonAlphaMode::Opaque => CaptureMaterialAlphaMode::Opaque,
        MtoonAlphaMode::Mask => CaptureMaterialAlphaMode::Mask,
        MtoonAlphaMode::Blend => CaptureMaterialAlphaMode::Blend,
    }
}
