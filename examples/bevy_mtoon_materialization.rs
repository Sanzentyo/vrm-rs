//! Bevy-facing MToon materialization example.
//!
//! This keeps shader policy out of `vrm-rs`: the example material is an
//! engine-owned Bevy asset that consumes `BevyMtoonMaterialPlan` plus runtime
//! `BevyVrmMaterialState`.

use bevy::prelude::{Asset, Assets, TypePath};
use vrm_adapter::MTOON_GPU_UNIFORM_SIZE;
use vrm_adapter_bevy::{
    BevyMtoonMaterialPlan, BevyMtoonPass, BevyMtoonTextureRefs, BevyVrmMaterialState,
    VrmBevyMaterialAsset, bevy_mtoon_material_plans,
};
use vrm_core::{
    EmissiveStrength, Feature, Material, MaterialRef, MtoonAlphaMode, MtoonCullMode, MtoonMaterial,
    MtoonPipelinePass, MtoonRenderQueue, MtoonTextureSet, OutlineWidthMode, TextureRef,
    VrmDocument,
};

#[derive(Clone, Debug, PartialEq)]
struct ExamplePassState {
    pass: BevyMtoonPass,
    render_order: i32,
    alpha_mode: MtoonAlphaMode,
    cull_mode: MtoonCullMode,
    depth_write: bool,
    outline_width: Option<f32>,
}

#[derive(Asset, Clone, Debug, Default, PartialEq, TypePath)]
struct ExampleBevyMtoonMaterial {
    material: Option<MaterialRef>,
    base_pass: Option<ExamplePassState>,
    outline_pass: Option<ExamplePassState>,
    base_color: [f32; 4],
    shade_color: [f32; 3],
    emissive_color: [f32; 3],
    emissive_strength: f32,
    cutoff: f32,
    uniform_size: usize,
    textures: BevyMtoonTextureRefs,
    runtime_colors: Vec<(String, Vec<f32>)>,
}

impl ExampleBevyMtoonMaterial {
    fn from_plans(material: MaterialRef, plans: &[BevyMtoonMaterialPlan]) -> Self {
        let mut output = Self {
            material: Some(material),
            emissive_strength: 1.0,
            ..Self::default()
        };
        for plan in plans.iter().filter(|plan| plan.material == material) {
            let pass = ExamplePassState {
                pass: plan.pass,
                render_order: plan.render_order,
                alpha_mode: plan.alpha_mode,
                cull_mode: plan.cull_mode,
                depth_write: plan.depth_write,
                outline_width: plan.outline_width,
            };
            match plan.pass {
                BevyMtoonPass::Base => {
                    output.base_pass = Some(pass);
                    output.base_color = plan.base_color;
                    output.shade_color = plan.shade_color;
                    output.emissive_color = plan.emissive_color;
                    output.cutoff = plan.cutoff;
                    output.uniform_size = plan.gpu_uniform.bytes().len();
                    output.textures = plan.textures.clone();
                }
                BevyMtoonPass::Outline => {
                    output.outline_pass = Some(pass);
                }
            }
        }
        output
    }
}

impl VrmBevyMaterialAsset for ExampleBevyMtoonMaterial {
    fn apply_vrm_material_state(&mut self, material: MaterialRef, state: &BevyVrmMaterialState) {
        self.material = Some(material);
        if let Some(strength) = state.emissive_intensity {
            self.emissive_strength = strength;
        }
        self.runtime_colors = state
            .colors
            .iter()
            .map(|(property, color)| (property.clone(), color.clone()))
            .collect();
    }
}

fn sample_document() -> VrmDocument {
    VrmDocument {
        materials: vec![Material {
            name: Some("example-mtoon".to_owned()),
            khr_emissive_strength: Feature::Present(EmissiveStrength(3.0)),
            mtoon: Feature::Present(MtoonMaterial {
                render_queue: MtoonRenderQueue::Transparent,
                cull_mode: MtoonCullMode::Off,
                transparent_with_z_write: false,
                base_color_factor: [0.8, 0.7, 0.6, 0.5],
                shade_color_factor: [0.3, 0.25, 0.2],
                emissive_factor: [0.1, 0.2, 0.3],
                cutoff_factor: 0.37,
                outline_width_mode: OutlineWidthMode::WorldCoordinates,
                outline_width_factor: 0.015,
                textures: MtoonTextureSet {
                    main_texture: Some(TextureRef(1)),
                    shade_multiply_texture: Some(TextureRef(2)),
                    shading_shift_texture: Some(TextureRef(8)),
                    normal_texture: Some(TextureRef(3)),
                    matcap_texture: Some(TextureRef(4)),
                    rim_multiply_texture: Some(TextureRef(5)),
                    outline_width_multiply_texture: Some(TextureRef(6)),
                    uv_animation_mask_texture: Some(TextureRef(7)),
                },
                ..MtoonMaterial::default()
            }),
            ..Material::default()
        }],
        ..VrmDocument::default()
    }
}

fn main() {
    let document = sample_document();
    let plans = bevy_mtoon_material_plans(&document, Default::default());
    let material_ref = MaterialRef(0);
    let material = ExampleBevyMtoonMaterial::from_plans(material_ref, &plans);

    let mut assets = Assets::<ExampleBevyMtoonMaterial>::default();
    let handle = assets.add(material);
    {
        let mut asset = assets.get_mut(&handle).unwrap();
        asset.apply_vrm_material_state(
            material_ref,
            &BevyVrmMaterialState {
                emissive_intensity: Some(3.0),
                mtoon_pipeline_passes: document.materials[0]
                    .mtoon
                    .as_ref()
                    .map_or_else(Vec::new, MtoonMaterial::pipeline_passes),
                colors: [("_Color".to_owned(), vec![0.8, 0.7, 0.6, 0.5])]
                    .into_iter()
                    .collect(),
                texture_transform: None,
            },
        );
    }

    let asset = assets.get(&handle).unwrap();
    assert_eq!(asset.base_pass.as_ref().unwrap().pass, BevyMtoonPass::Base);
    assert_eq!(
        asset.outline_pass.as_ref().unwrap().pass,
        BevyMtoonPass::Outline
    );
    assert_eq!(
        asset.base_pass.as_ref().unwrap().alpha_mode,
        MtoonAlphaMode::Blend
    );
    assert_eq!(
        asset.base_pass.as_ref().unwrap().cull_mode,
        MtoonCullMode::Off
    );
    assert!(!asset.base_pass.as_ref().unwrap().depth_write);
    assert_eq!(asset.base_pass.as_ref().unwrap().render_order, 3000);
    assert_eq!(
        asset.outline_pass.as_ref().unwrap().outline_width,
        Some(0.015)
    );
    assert_eq!(asset.emissive_color, [0.3, 0.6, 0.90000004]);
    assert_eq!(asset.emissive_strength, 3.0);
    assert_eq!(asset.uniform_size, MTOON_GPU_UNIFORM_SIZE);
    assert_eq!(asset.textures.base_color, Some(TextureRef(1)));
    assert_eq!(asset.textures.uv_animation_mask, Some(TextureRef(7)));
    assert_eq!(asset.textures.shading_shift, Some(TextureRef(8)));
    assert_eq!(asset.runtime_colors[0].0, "_Color");

    let pass_count = document.materials[0]
        .mtoon
        .as_ref()
        .map(MtoonMaterial::pipeline_passes)
        .map_or(0, |passes| passes.len());
    assert!(matches!(
        document.materials[0]
            .mtoon
            .as_ref()
            .unwrap()
            .pipeline_passes()[0],
        MtoonPipelinePass::Base(_)
    ));
    assert_eq!(pass_count, 2);
}
