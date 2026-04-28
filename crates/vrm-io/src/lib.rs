//! glTF/GLB IO for VRM and VRMA assets.

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use vrm_core::{Resolved, VrmModel};
use vrm_protocol::{
    ExtensionBundle, ExtensionMap, NodeConstraintExtension, ProtocolError, parse_root_extensions,
};
use vrm_sans_io::{BuildError, ValidatedAssetBuilder};

#[derive(Clone, Debug)]
pub struct LoadedVrm {
    model: VrmModel<Resolved>,
    pub buffers: Vec<Vec<u8>>,
    pub images: Vec<ImageData>,
}

impl LoadedVrm {
    pub fn model(&self) -> &VrmModel<Resolved> {
        &self.model
    }

    pub fn into_model(self) -> VrmModel<Resolved> {
        self.model
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageData {
    pub mime_type: Option<String>,
    pub bytes: Vec<u8>,
}

pub fn load_vrm_from_slice(bytes: &[u8]) -> Result<LoadedVrm, VrmIoError> {
    let (document, buffers, images) = gltf::import_slice(bytes)?;
    let mut bundle = parse_root_extensions(&extension_map(document.as_json().extensions.as_ref()))?;
    extract_node_constraints(&document, &mut bundle)?;
    extract_mtoon_materials(&document, &mut bundle)?;

    let image_data = images
        .into_iter()
        .map(|image| ImageData {
            mime_type: image.format.to_mime_type().map(str::to_owned),
            bytes: image.pixels,
        })
        .collect();

    let node_count = document.nodes().count();
    let material_count = document.materials().count();
    let model = ValidatedAssetBuilder::new()
        .with_node_count(node_count)
        .with_material_count(material_count)
        .build(bundle)?
        .resolve();

    Ok(LoadedVrm {
        model,
        buffers: buffers.into_iter().map(|buffer| buffer.0).collect(),
        images: image_data,
    })
}

fn extension_map<T>(source: Option<&T>) -> ExtensionMap
where
    T: Serialize,
{
    source
        .and_then(|extensions| serde_json::to_value(extensions).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map.into_iter().collect()),
            _ => None,
        })
        .unwrap_or_default()
}

fn extract_node_constraints(
    document: &gltf::Document,
    bundle: &mut ExtensionBundle,
) -> Result<(), VrmIoError> {
    for node in document.nodes() {
        let extensions = extension_map(document.as_json().nodes[node.index()].extensions.as_ref());
        if let Some(value) = extensions.get("VRMC_node_constraint") {
            let constraint = serde_json::from_value(value.clone()).map_err(|err| {
                VrmIoError::InvalidExtension {
                    extension: "VRMC_node_constraint".to_owned(),
                    message: err.to_string(),
                }
            })?;
            bundle.node_constraints.push(NodeConstraintExtension {
                node: node.index(),
                constraint,
            });
        }
    }
    Ok(())
}

fn extract_mtoon_materials(
    document: &gltf::Document,
    bundle: &mut ExtensionBundle,
) -> Result<(), VrmIoError> {
    for material in document.materials() {
        let Some(material_index) = material.index() else {
            continue;
        };
        let extensions = extension_map(
            document.as_json().materials[material_index]
                .extensions
                .as_ref(),
        );
        if let Some(value) = extensions.get("VRMC_materials_mtoon") {
            let mtoon = serde_json::from_value(value.clone()).map_err(|err| {
                VrmIoError::InvalidExtension {
                    extension: "VRMC_materials_mtoon".to_owned(),
                    message: err.to_string(),
                }
            })?;
            bundle.mtoon_materials.insert(material_index, mtoon);
        }
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum VrmIoError {
    #[error(transparent)]
    Gltf(#[from] gltf::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error("invalid extension {extension}: {message}")]
    InvalidExtension { extension: String, message: String },
}

trait ImageFormatExt {
    fn to_mime_type(&self) -> Option<&'static str>;
}

impl ImageFormatExt for gltf::image::Format {
    fn to_mime_type(&self) -> Option<&'static str> {
        match self {
            gltf::image::Format::R8 => None,
            gltf::image::Format::R8G8 => None,
            gltf::image::Format::R8G8B8 => None,
            gltf::image::Format::R8G8B8A8 => None,
            gltf::image::Format::R16 => None,
            gltf::image::Format::R16G16 => None,
            gltf::image::Format::R16G16B16 => None,
            gltf::image::Format::R16G16B16A16 => None,
            gltf::image::Format::R32G32B32FLOAT => None,
            gltf::image::Format::R32G32B32A32FLOAT => None,
        }
    }
}

#[allow(dead_code)]
fn _preserve_indexmap_dependency(_: IndexMap<String, Value>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use vrm_core::{Feature, HumanBoneName, LookAtKind, OutlineWidthMode, VrmKind};

    #[test]
    fn loads_generated_vrm1_gltf_without_repo_fixture_asset() {
        let bytes = generated_vrm1_gltf().to_string().into_bytes();
        let loaded = load_vrm_from_slice(&bytes).unwrap();
        let document = loaded.model().document();

        assert_eq!(document.kind, VrmKind::Vrm1);
        assert_eq!(document.meta.name, "Generated Test Avatar");
        assert!(document.humanoid.bones.contains_key(&HumanBoneName::Hips));
        assert!(matches!(
            document.look_at,
            Feature::Present(ref look_at) if look_at.kind == LookAtKind::Expression
        ));
        assert!(matches!(
            document.materials.first().map(|material| &material.mtoon),
            Some(Feature::Present(mtoon)) if mtoon.outline_width_mode == OutlineWidthMode::WorldCoordinates
        ));
        assert_eq!(document.node_constraints.len(), 1);
        assert!(document.spring_bone.is_present());
    }

    #[test]
    fn generated_sample_reports_invalid_node_references() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VRMC_vrm"]["humanoid"]["humanBones"]["hips"]["node"] = json!(999);
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("node(999)"));
    }

    fn generated_vrm1_gltf() -> Value {
        let required_bones = [
            ("hips", 0),
            ("spine", 1),
            ("head", 2),
            ("leftUpperLeg", 3),
            ("leftLowerLeg", 4),
            ("leftFoot", 5),
            ("rightUpperLeg", 6),
            ("rightLowerLeg", 7),
            ("rightFoot", 8),
            ("leftUpperArm", 9),
            ("leftLowerArm", 10),
            ("leftHand", 11),
            ("rightUpperArm", 12),
            ("rightLowerArm", 13),
            ("rightHand", 14),
        ];
        let human_bones = required_bones
            .into_iter()
            .map(|(name, node)| (name.to_owned(), json!({ "node": node })))
            .collect::<serde_json::Map<_, _>>();

        let mut nodes = (0..15)
            .map(|index| json!({ "name": format!("node_{index}") }))
            .collect::<Vec<_>>();
        nodes[14]["extensions"] = json!({
            "VRMC_node_constraint": {
                "specVersion": "1.0",
                "constraint": {
                    "rotation": { "source": 13, "weight": 0.75 }
                }
            }
        });

        json!({
            "asset": { "version": "2.0", "generator": "vrm-rs generated test data" },
            "extensionsUsed": [
                "VRMC_vrm",
                "VRMC_springBone",
                "VRMC_node_constraint",
                "VRMC_materials_mtoon"
            ],
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": nodes,
            "materials": [{
                "name": "mtoon",
                "extensions": {
                    "VRMC_materials_mtoon": {
                        "specVersion": "1.0",
                        "transparentWithZWrite": true,
                        "renderQueueOffsetNumber": 2,
                        "shadeColorFactor": [0.8, 0.7, 0.6],
                        "outlineWidthMode": "worldCoordinates",
                        "outlineWidthFactor": 0.01,
                        "outlineColorFactor": [0.1, 0.1, 0.1]
                    }
                }
            }],
            "extensions": {
                "VRMC_vrm": {
                    "specVersion": "1.0",
                    "meta": {
                        "name": "Generated Test Avatar",
                        "authors": ["vrm-rs"]
                    },
                    "humanoid": { "humanBones": human_bones },
                    "firstPerson": {
                        "meshAnnotations": [{ "node": 0, "type": "auto" }]
                    },
                    "lookAt": {
                        "type": "expression",
                        "offsetFromHeadBone": [0.0, 0.06, 0.0],
                        "rangeMapHorizontalInner": {
                            "inputMaxValue": 45.0,
                            "outputScale": 10.0
                        }
                    },
                    "expressions": {
                        "preset": {
                            "blink": {
                                "morphTargetBinds": [{
                                    "node": 2,
                                    "index": 0,
                                    "weight": 100.0
                                }],
                                "overrideLookAt": "block"
                            }
                        }
                    }
                },
                "VRMC_springBone": {
                    "specVersion": "1.0",
                    "colliders": [{
                        "node": 2,
                        "shape": {
                            "sphere": {
                                "offset": [0.0, 0.0, 0.0],
                                "radius": 0.1
                            }
                        }
                    }],
                    "colliderGroups": [{
                        "name": "head",
                        "colliders": [0]
                    }],
                    "springs": [{
                        "name": "hair",
                        "joints": [{
                            "node": 2,
                            "hitRadius": 0.02,
                            "stiffness": 0.8,
                            "gravityPower": 0.1,
                            "gravityDir": [0.0, -1.0, 0.0],
                            "dragForce": 0.4
                        }],
                        "colliderGroups": [0]
                    }]
                }
            }
        })
    }
}
