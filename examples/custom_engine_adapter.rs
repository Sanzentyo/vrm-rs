//! Minimal custom-engine adapter flow without Bevy, wgpu, or ash.
//!
//! A renderer can keep `vrm-rs` at the runtime/material planning boundary:
//! use `HeadlessSceneState` as a sans-renderer staging scene, run
//! `VrmRuntimeDriver`, then copy the resulting transforms, morph weights,
//! material writes, and MToon pipeline passes into engine-owned tables.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use vrm_adapter::{
    HeadlessSceneState, MtoonMaterializationOptions, MtoonRendererPass, RendererMaterialAlphaMode,
    RendererMaterialPipelinePlan, TransformAccess, ViewMode, VrmRuntimeDriver,
    mtoon_renderer_material_plans,
};
use vrm_core::{
    ConstraintKind, EmissiveStrength, Expression, ExpressionBind, ExpressionName, ExpressionSet,
    Feature, FirstPerson, FirstPersonAnnotation, FirstPersonMeshAnnotation, HumanBone,
    HumanBoneName, Humanoid, Material, MaterialRef, MtoonAlphaMode, MtoonMaterial,
    MtoonPipelinePass, MtoonRenderQueue, NodeConstraint, NodeRef, TextureRef, Transform,
    VrmDocument,
};
use vrm_runtime::{AppliedExpression, DeltaTime, RuntimeEvents};

#[derive(Clone, Debug, Default, PartialEq)]
struct EngineNode {
    local: Transform,
    visible: bool,
    morph_weights: HashMap<usize, f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnginePass {
    Base,
    Outline,
}

#[derive(Clone, Debug, PartialEq)]
struct EnginePipeline {
    pass: EnginePass,
    alpha: RendererMaterialAlphaMode,
    depth_write: bool,
    render_order: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct EngineMaterial {
    color_properties: HashMap<String, Vec<f32>>,
    emissive_intensity: f32,
    base_texture: Option<TextureRef>,
    pipeline: Vec<EnginePipeline>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CustomEngine {
    staging: HeadlessSceneState,
    nodes: HashMap<NodeRef, EngineNode>,
    materials: HashMap<MaterialRef, EngineMaterial>,
}

impl CustomEngine {
    fn insert_node(&mut self, node: NodeRef, local: Transform) {
        self.staging.insert_node(node, local);
        self.nodes.insert(
            node,
            EngineNode {
                local,
                visible: true,
                morph_weights: HashMap::new(),
            },
        );
    }

    fn materialize_mtoon(&mut self, document: &VrmDocument) {
        for plan in mtoon_renderer_material_plans(document, MtoonMaterializationOptions::default())
        {
            let material = self.materials.entry(plan.material).or_default();
            if plan.pass == MtoonRendererPass::Base {
                material.base_texture = plan.textures.main;
            }
            let pipeline = RendererMaterialPipelinePlan::from_mtoon_plan(&plan);
            material.pipeline.push(EnginePipeline {
                pass: match plan.pass {
                    MtoonRendererPass::Base => EnginePass::Base,
                    MtoonRendererPass::Outline => EnginePass::Outline,
                },
                alpha: pipeline.alpha_mode,
                depth_write: pipeline.depth_write,
                render_order: pipeline.render_order,
            });
        }
    }

    fn sync_from_staging(&mut self, document: &VrmDocument) {
        for node in self.nodes.keys().copied().collect::<Vec<_>>() {
            if let Some(engine_node) = self.nodes.get_mut(&node) {
                engine_node.local = self.staging.local_transform(node).unwrap();
                engine_node.visible = self.staging.node(node).unwrap().visible;
                if let Some(weight) = self.staging.morph_weight(node, 0) {
                    engine_node.morph_weights.insert(0, weight);
                }
            }
        }
        for material_index in 0..document.materials.len() {
            let material_ref = MaterialRef(material_index);
            let material = self.materials.entry(material_ref).or_default();
            if let Some(color) = self.staging.material_color(material_ref, "_Color") {
                let color_property = material
                    .color_properties
                    .entry("_Color".to_owned())
                    .or_default();
                color_property.clear();
                color_property.extend_from_slice(color);
            }
            if let Some(intensity) = self.staging.emissive_intensity(material_ref) {
                material.emissive_intensity = intensity;
            }
            if let Some(passes) = self.staging.mtoon_pipeline_passes(material_ref) {
                material.pipeline.clear();
                material
                    .pipeline
                    .extend(passes.iter().map(engine_pipeline_from_pass));
            }
        }
    }
}

fn engine_pipeline_from_pass(pass: &MtoonPipelinePass) -> EnginePipeline {
    match pass {
        MtoonPipelinePass::Base(hints) => EnginePipeline {
            pass: EnginePass::Base,
            alpha: match hints.alpha_mode {
                MtoonAlphaMode::Opaque => RendererMaterialAlphaMode::Opaque,
                MtoonAlphaMode::Mask => RendererMaterialAlphaMode::Mask,
                MtoonAlphaMode::Blend => RendererMaterialAlphaMode::Blend,
            },
            depth_write: hints.depth_write,
            render_order: hints.render_order,
        },
        MtoonPipelinePass::Outline(hints) => EnginePipeline {
            pass: EnginePass::Outline,
            alpha: RendererMaterialAlphaMode::Opaque,
            depth_write: true,
            render_order: hints.render_order,
        },
    }
}

fn sample_document() -> VrmDocument {
    VrmDocument {
        humanoid: Humanoid {
            bones: [(
                HumanBoneName::Hips,
                HumanBone {
                    node: NodeRef(1),
                    rest: Transform::default(),
                },
            )]
            .into_iter()
            .collect(),
        },
        first_person: Feature::Present(FirstPerson {
            mesh_annotations: vec![FirstPersonMeshAnnotation {
                node: NodeRef(8),
                kind: FirstPersonAnnotation::FirstPersonOnly,
            }],
        }),
        expressions: Feature::Present(ExpressionSet {
            preset: [(
                ExpressionName::Blink,
                Expression {
                    binds: vec![
                        ExpressionBind::MorphTarget {
                            node: NodeRef(8),
                            index: 0,
                            weight: 100.0,
                        },
                        ExpressionBind::MaterialColor {
                            material: MaterialRef(0),
                            kind: "_Color".to_owned(),
                            target_value: vec![0.8, 0.6, 0.4, 1.0],
                        },
                    ],
                    ..Expression::default()
                },
            )]
            .into_iter()
            .collect(),
            custom: Default::default(),
        }),
        node_constraints: vec![NodeConstraint {
            destination: NodeRef(2),
            source: NodeRef(4),
            kind: ConstraintKind::Rotation,
            weight: 1.0,
        }],
        materials: vec![Material {
            khr_emissive_strength: Feature::Present(EmissiveStrength(2.0)),
            mtoon: Feature::Present(MtoonMaterial {
                render_queue: MtoonRenderQueue::Transparent,
                transparent_with_z_write: true,
                base_color_factor: [0.8, 0.6, 0.4, 0.5],
                emissive_factor: [0.1, 0.2, 0.3],
                textures: vrm_core::MtoonTextureSet {
                    main_texture: Some(TextureRef(3)),
                    ..Default::default()
                },
                ..MtoonMaterial::default()
            }),
            ..Material::default()
        }],
        ..VrmDocument::default()
    }
}

fn sample_engine() -> CustomEngine {
    let mut engine = CustomEngine::default();
    engine.insert_node(NodeRef(0), Transform::default());
    engine.insert_node(NodeRef(1), Transform::default());
    engine.insert_node(NodeRef(2), Transform::default());
    engine.insert_node(
        NodeRef(4),
        Transform {
            rotation: Quat::IDENTITY,
            ..Transform::default()
        },
    );
    engine.insert_node(NodeRef(8), Transform::default());
    engine
        .staging
        .set_parent(NodeRef(1), Some(NodeRef(0)))
        .unwrap();
    engine
        .staging
        .capture_constraint_rest_state(NodeRef(2), NodeRef(4))
        .unwrap();
    engine
}

fn main() {
    let document = sample_document();
    let mut engine = sample_engine();
    engine.materialize_mtoon(&document);

    engine
        .staging
        .set_local_rotation(NodeRef(4), Quat::from_rotation_y(0.5))
        .unwrap();
    let events = RuntimeEvents {
        delta: DeltaTime(0.0),
        expressions: vec![AppliedExpression {
            name: "blink".to_owned(),
            effective_weight: 0.75,
            binds: document
                .expressions
                .as_ref()
                .unwrap()
                .preset
                .get(&ExpressionName::Blink)
                .unwrap()
                .binds
                .clone(),
        }],
        constraints: document.node_constraints.clone(),
        springs: Vec::new(),
    };
    let mut driver = VrmRuntimeDriver::new(&document)
        .with_runtime_events(&events)
        .with_view_mode(ViewMode::ThirdPerson);
    driver.tick(&mut engine.staging, None).unwrap();
    engine.sync_from_staging(&document);

    assert_eq!(engine.nodes[&NodeRef(8)].morph_weights[&0], 75.0);
    assert!(!engine.nodes[&NodeRef(8)].visible);
    assert!(
        engine.nodes[&NodeRef(2)]
            .local
            .rotation
            .abs_diff_eq(Quat::from_rotation_y(0.5), 0.0001)
    );
    assert_eq!(
        engine.materials[&MaterialRef(0)].color_properties["_Color"],
        vec![0.8, 0.6, 0.4, 1.0]
    );
    assert_eq!(engine.materials[&MaterialRef(0)].emissive_intensity, 2.0);
    assert_eq!(
        engine.materials[&MaterialRef(0)].base_texture,
        Some(TextureRef(3))
    );
    assert_eq!(engine.materials[&MaterialRef(0)].pipeline.len(), 1);
    assert_eq!(
        engine.materials[&MaterialRef(0)].pipeline[0].alpha,
        RendererMaterialAlphaMode::Blend
    );
    assert!(engine.materials[&MaterialRef(0)].pipeline[0].depth_write);
    assert_eq!(
        engine.materials[&MaterialRef(0)].pipeline[0].render_order,
        3000
    );
    assert_eq!(engine.nodes[&NodeRef(1)].local.translation, Vec3::ZERO);
}
