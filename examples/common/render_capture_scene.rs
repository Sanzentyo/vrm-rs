use std::error::Error;
use vrm_adapter::{
    GltfMaterialAlphaMode, GltfMaterialPipelineOverride, MtoonMaterializationOptions,
    RendererMaterialAlphaMode, RendererMaterialCullMode, RendererMaterialPipelinePlan,
    renderer_material_pipeline_plan,
};
use vrm_core::MaterialRef;
use vrm_io::{GltfAlphaMode, LoadedVrm};

pub fn runtime_world_matrices(loaded: &LoadedVrm) -> Result<Vec<vrm_rs::Mat4>, Box<dyn Error>> {
    Ok(vrm_rs::evaluated_world_matrices(loaded)?)
}

pub type CaptureMaterialPlan = RendererMaterialPipelinePlan;
pub type CaptureMaterialCullMode = RendererMaterialCullMode;
pub type CaptureMaterialAlphaMode = RendererMaterialAlphaMode;

pub fn capture_material_plan(loaded: &LoadedVrm, material: Option<usize>) -> CaptureMaterialPlan {
    let material_ref = material.map(MaterialRef);
    let gltf_override = material
        .and_then(|index| loaded.gltf_materials.get(index))
        .map(|gltf| GltfMaterialPipelineOverride {
            alpha_mode: gltf_alpha_mode(gltf.alpha_mode),
            alpha_cutoff: gltf.alpha_cutoff,
            double_sided: gltf.double_sided,
        });
    renderer_material_pipeline_plan(
        loaded.model().document(),
        material_ref,
        MtoonMaterializationOptions::default(),
        gltf_override,
    )
}

fn gltf_alpha_mode(mode: GltfAlphaMode) -> GltfMaterialAlphaMode {
    match mode {
        GltfAlphaMode::Opaque => GltfMaterialAlphaMode::Opaque,
        GltfAlphaMode::Mask => GltfMaterialAlphaMode::Mask,
        GltfAlphaMode::Blend => GltfMaterialAlphaMode::Blend,
    }
}
