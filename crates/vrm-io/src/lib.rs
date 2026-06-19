//! glTF/GLB IO for VRM and VRMA assets.

pub mod optimize;
pub mod resource;

use glam::{Mat4, Quat, Vec3, Vec4};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use vrm_core::{
    ExpressionBind, ExpressionName, Feature, HumanBoneName, MtoonMaterial, OutlineWidthMode,
    Resolved, RotationTrack, ScalarTrack, TextureRef, TextureTransform2d, Transform,
    TranslationTrack, VrmAnimation, VrmKind, VrmModel,
};
pub use vrm_diagnostics::{
    Diagnostic, DiagnosticPolicy, DiagnosticReport, DiagnosticSeverity, JsonPath,
};
use vrm_protocol::{
    ExtensionBundle, ExtensionMap, NodeConstraintExtension, ProtocolError, VrmExtension,
    parse_root_extensions,
};
use vrm_sans_io::{BuildError, ValidatedAssetBuilder};

pub use optimize::{
    JointPaletteCompaction, OptimizeError, OptimizeOptions, OptimizeReport, VertexRemap,
    apply_joint_compaction_to_skin, optimize_primitive,
};
pub use resource::{
    CodecRegistry, CompressedMeshPayload, CompressedTexturePayload, DecodedMeshPayload,
    DecodedTexturePayload, FileResourceReader, MeshCodec, MeshCodecProvider, ResourceData,
    ResourceError, ResourceLimits, ResourceReader, ResourceSource, TextureCodec,
    TextureCodecProvider, TextureColorSpace, TextureDecodeOptions, TextureFormatCapabilities,
    TextureOutputFormat,
};

#[derive(Clone, Debug)]
pub struct LoadedVrm {
    model: VrmModel<Resolved>,
    pub source: GltfSource,
    pub scene: GltfSceneRest,
    pub meshes: Vec<GltfMeshData>,
    pub skins: Vec<GltfSkinData>,
    pub gltf_materials: Vec<GltfMaterialData>,
    pub textures: Vec<GltfTextureData>,
    pub buffers: Vec<Vec<u8>>,
    pub images: Vec<ImageData>,
    pub warnings: Vec<VrmIoWarning>,
}

#[derive(Clone, Debug)]
pub struct LoadedVrmWithDiagnostics {
    pub loaded: LoadedVrm,
    pub diagnostics: DiagnosticReport,
}

impl LoadedVrm {
    pub fn model(&self) -> &VrmModel<Resolved> {
        &self.model
    }

    pub fn into_model(self) -> VrmModel<Resolved> {
        self.model
    }

    pub fn warnings(&self) -> &[VrmIoWarning] {
        &self.warnings
    }

    pub fn source(&self) -> &GltfSource {
        &self.source
    }

    pub fn scene(&self) -> &GltfSceneRest {
        &self.scene
    }

    pub fn texture_image(&self, texture: usize) -> Option<&ImageData> {
        let image = self.textures.get(texture)?.image;
        self.images.get(image)
    }

    pub fn texture_rgba8_image(&self, texture: usize) -> Option<CpuRgba8Image> {
        CpuRgba8Image::from_image_data(self.texture_image(texture)?).ok()
    }

    pub fn material_display_name(&self, material: Option<usize>) -> Option<&str> {
        let index = material?;
        self.gltf_materials
            .get(index)
            .and_then(|material| material.name.as_deref())
            .or_else(|| {
                self.model
                    .document()
                    .materials
                    .get(index)
                    .and_then(|material| material.name.as_deref())
            })
    }

    pub fn material_base_texture_rgba8_image(
        &self,
        material: Option<usize>,
    ) -> Option<CpuRgba8Image> {
        self.material_texture_slots(material)
            .base
            .and_then(|texture| self.texture_rgba8_image(texture))
    }

    pub fn material_outline_width_rgba8_image(
        &self,
        material: Option<usize>,
    ) -> Option<CpuRgba8Image> {
        self.material_texture_slots(material)
            .outline_width
            .and_then(|texture| self.texture_rgba8_image(texture))
    }

    pub fn material_texture_slots(&self, material: Option<usize>) -> GltfMaterialTextureSlots {
        let mtoon = material
            .and_then(|index| self.model.document().materials.get(index))
            .and_then(|material| material.mtoon.as_ref());
        let gltf = material.and_then(|index| self.gltf_materials.get(index));
        let valid_texture = |texture: Option<usize>| {
            texture.and_then(|texture| self.textures.get(texture).map(|_| texture))
        };
        let mtoon_texture =
            |texture: Option<TextureRef>| valid_texture(texture.map(|texture| texture.0));

        GltfMaterialTextureSlots {
            base: mtoon_texture(mtoon.and_then(|mtoon| mtoon.textures.main_texture))
                .or_else(|| valid_texture(gltf.and_then(|material| material.base_color_texture))),
            shade: mtoon_texture(mtoon.and_then(|mtoon| mtoon.textures.shade_multiply_texture)),
            shading_shift: mtoon_texture(
                mtoon.and_then(|mtoon| mtoon.textures.shading_shift_texture),
            ),
            normal: mtoon_texture(mtoon.and_then(|mtoon| mtoon.textures.normal_texture))
                .or_else(|| valid_texture(gltf.and_then(|material| material.normal_texture))),
            matcap: mtoon_texture(mtoon.and_then(|mtoon| mtoon.textures.matcap_texture)),
            rim: mtoon_texture(mtoon.and_then(|mtoon| mtoon.textures.rim_multiply_texture)),
            outline_width: mtoon_texture(
                mtoon.and_then(|mtoon| mtoon.textures.outline_width_multiply_texture),
            ),
            emissive: valid_texture(gltf.and_then(|material| material.emissive_texture)),
            occlusion: valid_texture(gltf.and_then(|material| material.occlusion_texture)),
            uv_animation_mask: mtoon_texture(
                mtoon.and_then(|mtoon| mtoon.textures.uv_animation_mask_texture),
            ),
        }
    }

    pub fn material_uv_transforms(
        &self,
        material: Option<usize>,
        mtoon_time: f32,
    ) -> GltfMaterialUvTransforms {
        let mtoon = material
            .and_then(|index| self.model.document().materials.get(index))
            .and_then(|material| material.mtoon.as_ref());
        let gltf = material.and_then(|index| self.gltf_materials.get(index));
        let base = mtoon
            .and_then(|mtoon| mtoon.texture_transforms.main_texture)
            .or_else(|| gltf.and_then(|material| material.base_color_texture_transform));
        let shade = mtoon
            .and_then(|mtoon| mtoon.texture_transforms.shade_multiply_texture)
            .or(base);

        GltfMaterialUvTransforms {
            base,
            shade,
            shading_shift: mtoon.and_then(|mtoon| mtoon.texture_transforms.shading_shift_texture),
            normal: mtoon
                .and_then(|mtoon| mtoon.texture_transforms.normal_texture)
                .or_else(|| gltf.and_then(|material| material.normal_texture_transform)),
            matcap: mtoon.and_then(|mtoon| mtoon.texture_transforms.matcap_texture),
            rim: mtoon.and_then(|mtoon| mtoon.texture_transforms.rim_multiply_texture),
            outline_width: mtoon
                .and_then(|mtoon| mtoon.texture_transforms.outline_width_multiply_texture),
            emissive: gltf.and_then(|material| material.emissive_texture_transform),
            occlusion: gltf.and_then(|material| material.occlusion_texture_transform),
            uv_animation_mask: mtoon
                .and_then(|mtoon| mtoon.texture_transforms.uv_animation_mask_texture),
            uv_animation_scroll: mtoon.map_or([0.0, 0.0], |mtoon| {
                [
                    mtoon.uv_animation.scroll_x_speed * mtoon_time,
                    mtoon.uv_animation.scroll_y_speed * mtoon_time,
                ]
            }),
            uv_animation_rotation: mtoon
                .map_or(0.0, |mtoon| mtoon.uv_animation.rotation_speed * mtoon_time),
        }
    }

    pub fn expression_material_uv_transforms(
        &self,
        material: Option<usize>,
        mtoon_time: f32,
        expression_effects: &GltfExpressionRenderEffects,
    ) -> GltfMaterialUvTransforms {
        let transforms = self.material_uv_transforms(material, mtoon_time);
        expression_effects.apply_uv_transforms(transforms, material)
    }

    pub fn material_shading_plan(
        &self,
        material: Option<usize>,
        options: GltfMaterialShadingOptions,
    ) -> GltfMaterialShadingPlan {
        if let Some(shading) = material
            .and_then(|index| self.model.document().materials.get(index))
            .and_then(|core_material| {
                let mtoon = core_material.mtoon.as_ref()?;
                let (emissive_strength, _) = core_material.effective_emissive_strength();
                Some(GltfMaterialShadingPlan {
                    base_color: mtoon.base_color_factor,
                    shade_color: [
                        mtoon.shade_color_factor[0],
                        mtoon.shade_color_factor[1],
                        mtoon.shade_color_factor[2],
                        1.0,
                    ],
                    shading_shift: mtoon.shading_shift_factor,
                    shading_toony: mtoon.shading_toony_factor,
                    shading_shift_texture_scale: mtoon.shading_shift_texture_scale,
                    gi_equalization: mtoon.gi_equalization_factor,
                    emissive: mtoon
                        .emissive_factor
                        .map(|channel| channel * emissive_strength.0),
                    matcap_factor: mtoon.matcap_factor,
                    parametric_rim_color: mtoon.parametric_rim_color_factor,
                    rim_lighting_mix: mtoon.rim_lighting_mix_factor,
                    parametric_rim_fresnel_power: mtoon.parametric_rim_fresnel_power_factor,
                    parametric_rim_lift: mtoon.parametric_rim_lift_factor,
                    normal_scale: self.material_normal_scale(material),
                    metallic: 0.0,
                    roughness: 1.0,
                    occlusion_strength: 0.0,
                    pbr_fallback: false,
                    unlit: false,
                    v0_compat_shade: options.v0_compat_shade,
                })
            })
        {
            return shading;
        }

        let gltf = material.and_then(|index| self.gltf_materials.get(index));
        let base_color = gltf
            .map(|material| material.base_color_factor)
            .unwrap_or([0.78, 0.78, 0.78, 1.0]);
        let emissive = gltf
            .map(|material| {
                material
                    .emissive_factor
                    .map(|channel| channel * material.emissive_strength)
            })
            .unwrap_or([0.0, 0.0, 0.0]);

        GltfMaterialShadingPlan {
            base_color,
            shade_color: base_color,
            shading_shift: 0.0,
            shading_toony: 0.0,
            shading_shift_texture_scale: 1.0,
            gi_equalization: 0.0,
            emissive,
            matcap_factor: [0.0, 0.0, 0.0],
            parametric_rim_color: [0.0, 0.0, 0.0],
            rim_lighting_mix: 1.0,
            parametric_rim_fresnel_power: 5.0,
            parametric_rim_lift: 0.0,
            normal_scale: self.material_normal_scale(material),
            metallic: gltf.map_or(0.0, |material| material.metallic_factor),
            roughness: gltf.map_or(1.0, |material| material.roughness_factor),
            occlusion_strength: gltf.map_or(1.0, |material| material.occlusion_strength),
            pbr_fallback: true,
            unlit: gltf.is_some_and(|material| material.unlit),
            v0_compat_shade: false,
        }
    }

    pub fn expression_material_shading_plan(
        &self,
        material: Option<usize>,
        options: GltfMaterialShadingOptions,
        expression_effects: &GltfExpressionRenderEffects,
    ) -> GltfMaterialShadingPlan {
        let mut shading = self.material_shading_plan(material, options);
        shading.base_color = expression_effects.apply_color4(shading.base_color, material, "color");
        if !shading.pbr_fallback {
            shading.shade_color =
                expression_effects.apply_color4(shading.shade_color, material, "shadeColor");
            shading.matcap_factor =
                expression_effects.apply_color3(shading.matcap_factor, material, "matcapColor");
            shading.parametric_rim_color =
                expression_effects.apply_color3(shading.parametric_rim_color, material, "rimColor");
        }
        shading.emissive =
            expression_effects.apply_color3(shading.emissive, material, "emissionColor");
        shading
    }

    pub fn expression_mtoon_outline_plan(
        &self,
        material: Option<usize>,
        expression_effects: &GltfExpressionRenderEffects,
    ) -> Option<GltfMtoonOutlinePlan> {
        let mtoon = material
            .and_then(|index| self.model.document().materials.get(index))
            .and_then(|material| material.mtoon.as_ref())?;
        if !mtoon.outline_enabled() {
            return None;
        }
        let color = expression_effects.apply_color4(
            [
                mtoon.outline_color_factor[0],
                mtoon.outline_color_factor[1],
                mtoon.outline_color_factor[2],
                mtoon.outline_lighting_mix_factor,
            ],
            material,
            "outlineColor",
        );
        Some(GltfMtoonOutlinePlan {
            width_factor: mtoon.outline_width_factor,
            width_mode: mtoon.outline_width_mode,
            color,
        })
    }

    fn material_normal_scale(&self, material: Option<usize>) -> f32 {
        self.material_texture_slots(material)
            .normal
            .map_or(0.0, |_| {
                material
                    .and_then(|index| self.gltf_materials.get(index))
                    .map_or(1.0, |material| material.normal_scale)
            })
    }

    pub fn expression_render_effects<I, N>(
        &self,
        expression_weights: I,
    ) -> Result<GltfExpressionRenderEffects, VrmIoError>
    where
        I: IntoIterator<Item = (N, f32)>,
        N: AsRef<str>,
    {
        let requested = expression_weights
            .into_iter()
            .map(|(name, weight)| (name.as_ref().to_owned(), weight))
            .collect::<Vec<_>>();
        let mut result = GltfExpressionRenderEffects::default();
        let Feature::Present(expressions) = &self.model.document().expressions else {
            if requested.is_empty() {
                return Ok(result);
            }
            return Err(VrmIoError::MissingExpressions);
        };

        for expression in expressions
            .preset
            .values()
            .chain(expressions.custom.values())
        {
            for bind in &expression.binds {
                match bind {
                    ExpressionBind::MorphTarget { node, index, .. } => {
                        result.cleared.entry(node.0).or_default().insert(*index);
                    }
                    ExpressionBind::MaterialColor { .. }
                    | ExpressionBind::TextureTransform { .. } => {}
                }
            }
        }

        for (name, weight) in requested {
            let expression = if let Some(expression) =
                expressions.preset.get(&ExpressionName::from(name.as_str()))
            {
                expression
            } else if let Some(expression) = expressions.custom.get(&name) {
                expression
            } else {
                return Err(VrmIoError::UnknownExpression { name });
            };
            let effective_weight = expression.output_weight(weight);
            for bind in &expression.binds {
                match bind {
                    ExpressionBind::MorphTarget {
                        node,
                        index,
                        weight,
                    } => {
                        *result.weights.entry((node.0, *index)).or_default() +=
                            effective_weight * *weight;
                    }
                    ExpressionBind::MaterialColor {
                        material,
                        kind,
                        target_value,
                    } => {
                        result.material_colors.push(GltfMaterialColorEffect {
                            material: material.0,
                            kind: kind.clone(),
                            target_value: target_value.clone(),
                            weight: effective_weight,
                        });
                    }
                    ExpressionBind::TextureTransform {
                        material,
                        scale,
                        offset,
                    } => {
                        result.texture_transforms.push(GltfTextureTransformEffect {
                            material: material.0,
                            scale: *scale,
                            offset: *offset,
                            weight: effective_weight,
                        });
                    }
                }
            }
        }

        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GltfSource {
    pub format: GltfSourceFormat,
    pub original_bytes: Vec<u8>,
    pub json_bytes: Vec<u8>,
    pub json: Value,
    pub glb_chunks: Vec<GlbChunk>,
}

impl GltfSource {
    pub fn from_slice(bytes: &[u8]) -> Result<Self, VrmIoError> {
        if is_glb(bytes) {
            Self::from_glb_slice(bytes)
        } else {
            Self::from_json_slice(bytes)
        }
    }

    pub fn root_extensions(&self) -> Option<&serde_json::Map<String, Value>> {
        self.json.get("extensions")?.as_object()
    }

    pub fn root_extension(&self, name: &str) -> Option<&Value> {
        self.root_extensions()?.get(name)
    }

    pub fn root_extras(&self) -> Option<&Value> {
        self.json.get("extras")
    }

    pub fn glb_json_chunk(&self) -> Option<&GlbChunk> {
        self.glb_chunks
            .iter()
            .find(|chunk| chunk.kind == GlbChunkKind::Json)
    }

    pub fn glb_bin_chunk(&self) -> Option<&GlbChunk> {
        self.glb_chunks
            .iter()
            .find(|chunk| chunk.kind == GlbChunkKind::Bin)
    }

    pub fn edited_vrm_metadata(&self, patch: &VrmMetadataPatch) -> Result<Value, VrmIoError> {
        let mut json = self.json.clone();
        let Some(extensions) = json.get_mut("extensions").and_then(Value::as_object_mut) else {
            return Err(VrmIoError::SourceWrite {
                message: "missing glTF root extensions object".to_owned(),
            });
        };

        if let Some(vrm1) = extensions.get_mut("VRMC_vrm") {
            let meta = vrm1
                .get_mut("meta")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| VrmIoError::SourceWrite {
                    message: "missing VRMC_vrm.meta object".to_owned(),
                })?;
            patch.apply_vrm1(meta);
            return Ok(json);
        }

        if let Some(vrm0) = extensions.get_mut("VRM") {
            let meta = vrm0
                .get_mut("meta")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| VrmIoError::SourceWrite {
                    message: "missing VRM.meta object".to_owned(),
                })?;
            patch.apply_vrm0(meta);
            return Ok(json);
        }

        Err(VrmIoError::SourceWrite {
            message: "missing VRM metadata extension".to_owned(),
        })
    }

    pub fn to_bytes_with_json(&self, json: &Value) -> Result<Vec<u8>, VrmIoError> {
        self.to_bytes_with_json_options(json, GltfWriteOptions::default())
    }

    pub fn to_bytes_with_json_options(
        &self,
        json: &Value,
        options: GltfWriteOptions,
    ) -> Result<Vec<u8>, VrmIoError> {
        match self.format {
            GltfSourceFormat::Json => serialize_json(json, options.json_format),
            GltfSourceFormat::Glb { .. } => self.to_glb_bytes_with_json_options(json, options),
        }
    }

    pub fn to_bytes_with_metadata_patch(
        &self,
        patch: &VrmMetadataPatch,
    ) -> Result<Vec<u8>, VrmIoError> {
        self.to_bytes_with_metadata_patch_options(patch, GltfWriteOptions::default())
    }

    pub fn to_bytes_with_metadata_patch_options(
        &self,
        patch: &VrmMetadataPatch,
        options: GltfWriteOptions,
    ) -> Result<Vec<u8>, VrmIoError> {
        self.to_bytes_with_json_options(&self.edited_vrm_metadata(patch)?, options)
    }

    pub fn save_with_json_atomic(
        &self,
        path: impl AsRef<Path>,
        json: &Value,
    ) -> Result<(), VrmIoError> {
        write_atomic(path.as_ref(), &self.to_bytes_with_json(json)?)
    }

    pub fn save_with_json_options_atomic(
        &self,
        path: impl AsRef<Path>,
        json: &Value,
        options: GltfWriteOptions,
    ) -> Result<(), VrmIoError> {
        write_atomic(
            path.as_ref(),
            &self.to_bytes_with_json_options(json, options)?,
        )
    }

    pub fn save_with_metadata_patch_atomic(
        &self,
        path: impl AsRef<Path>,
        patch: &VrmMetadataPatch,
    ) -> Result<(), VrmIoError> {
        write_atomic(path.as_ref(), &self.to_bytes_with_metadata_patch(patch)?)
    }

    pub fn save_with_metadata_patch_options_atomic(
        &self,
        path: impl AsRef<Path>,
        patch: &VrmMetadataPatch,
        options: GltfWriteOptions,
    ) -> Result<(), VrmIoError> {
        write_atomic(
            path.as_ref(),
            &self.to_bytes_with_metadata_patch_options(patch, options)?,
        )
    }

    pub fn save_original_atomic(&self, path: impl AsRef<Path>) -> Result<(), VrmIoError> {
        write_atomic(path.as_ref(), &self.original_bytes)
    }

    fn from_json_slice(bytes: &[u8]) -> Result<Self, VrmIoError> {
        Ok(Self {
            format: GltfSourceFormat::Json,
            original_bytes: bytes.to_vec(),
            json_bytes: bytes.to_vec(),
            json: serde_json::from_slice(bytes).map_err(|source| {
                VrmIoError::SourcePreservation {
                    message: format!("invalid glTF JSON source: {source}"),
                }
            })?,
            glb_chunks: Vec::new(),
        })
    }

    fn from_glb_slice(bytes: &[u8]) -> Result<Self, VrmIoError> {
        let chunks = parse_glb_chunks(bytes)?;
        let json_chunk = chunks
            .iter()
            .find(|chunk| chunk.kind == GlbChunkKind::Json)
            .ok_or_else(|| VrmIoError::SourcePreservation {
                message: "missing GLB JSON chunk".to_owned(),
            })?;
        Ok(Self {
            format: GltfSourceFormat::Glb {
                version: u32::from_le_bytes(bytes[4..8].try_into().expect("slice length checked")),
                declared_length: u32::from_le_bytes(
                    bytes[8..12].try_into().expect("slice length checked"),
                ),
            },
            original_bytes: bytes.to_vec(),
            json_bytes: json_chunk.bytes.clone(),
            json: serde_json::from_slice(&json_chunk.bytes).map_err(|source| {
                VrmIoError::SourcePreservation {
                    message: format!("invalid GLB JSON chunk: {source}"),
                }
            })?,
            glb_chunks: chunks,
        })
    }

    fn to_glb_bytes_with_json_options(
        &self,
        json: &Value,
        options: GltfWriteOptions,
    ) -> Result<Vec<u8>, VrmIoError> {
        let mut json_bytes = serialize_json(json, options.json_format)?;
        pad_json_chunk(&mut json_bytes);

        let chunks = self
            .glb_chunks
            .iter()
            .map(|chunk| {
                let bytes = if chunk.kind == GlbChunkKind::Json {
                    json_bytes.clone()
                } else {
                    chunk.bytes.clone()
                };
                (chunk.raw_type, bytes)
            })
            .collect::<Vec<_>>();

        let total_len = 12
            + chunks
                .iter()
                .map(|(_, bytes)| 8 + bytes.len())
                .sum::<usize>();
        if total_len > u32::MAX as usize {
            return Err(VrmIoError::SourceWrite {
                message: "GLB output is too large".to_owned(),
            });
        }

        let mut output = Vec::with_capacity(total_len);
        output.extend_from_slice(GLB_MAGIC);
        output.extend_from_slice(&2u32.to_le_bytes());
        output.extend_from_slice(&(total_len as u32).to_le_bytes());
        for (raw_type, bytes) in chunks {
            output.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            output.extend_from_slice(&raw_type.to_le_bytes());
            output.extend_from_slice(&bytes);
        }
        Ok(output)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GltfWriteOptions {
    pub json_format: GltfJsonFormat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GltfJsonFormat {
    #[default]
    Compact,
    Pretty,
}

fn serialize_json(json: &Value, format: GltfJsonFormat) -> Result<Vec<u8>, VrmIoError> {
    match format {
        GltfJsonFormat::Compact => serde_json::to_vec(json),
        GltfJsonFormat::Pretty => serde_json::to_vec_pretty(json),
    }
    .map_err(|source| VrmIoError::SourceWrite {
        message: format!("could not serialize glTF JSON: {source}"),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VrmMetadataPatch {
    pub name: MetadataFieldEdit<String>,
    pub authors: MetadataFieldEdit<Vec<String>>,
    pub version: MetadataFieldEdit<String>,
    pub license_url: MetadataFieldEdit<String>,
    pub copyright_information: MetadataFieldEdit<String>,
    pub contact_information: MetadataFieldEdit<String>,
}

impl VrmMetadataPatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = MetadataFieldEdit::Set(name.into());
        self
    }

    pub fn with_authors(mut self, authors: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.authors = MetadataFieldEdit::Set(authors.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = MetadataFieldEdit::Set(version.into());
        self
    }

    pub fn with_license_url(mut self, license_url: impl Into<String>) -> Self {
        self.license_url = MetadataFieldEdit::Set(license_url.into());
        self
    }

    pub fn clear_license_url(mut self) -> Self {
        self.license_url = MetadataFieldEdit::Clear;
        self
    }

    pub fn with_copyright_information(mut self, value: impl Into<String>) -> Self {
        self.copyright_information = MetadataFieldEdit::Set(value.into());
        self
    }

    pub fn clear_copyright_information(mut self) -> Self {
        self.copyright_information = MetadataFieldEdit::Clear;
        self
    }

    pub fn with_contact_information(mut self, value: impl Into<String>) -> Self {
        self.contact_information = MetadataFieldEdit::Set(value.into());
        self
    }

    pub fn clear_contact_information(mut self) -> Self {
        self.contact_information = MetadataFieldEdit::Clear;
        self
    }

    fn apply_vrm1(&self, meta: &mut serde_json::Map<String, Value>) {
        apply_string_edit(meta, "name", &self.name);
        apply_string_array_edit(meta, "authors", &self.authors);
        apply_string_edit(meta, "version", &self.version);
        apply_string_edit(meta, "licenseUrl", &self.license_url);
        apply_string_edit(meta, "copyrightInformation", &self.copyright_information);
        apply_string_edit(meta, "contactInformation", &self.contact_information);
    }

    fn apply_vrm0(&self, meta: &mut serde_json::Map<String, Value>) {
        apply_string_edit(meta, "title", &self.name);
        apply_legacy_author_edit(meta, &self.authors);
        apply_string_edit(meta, "version", &self.version);
        apply_string_edit(meta, "otherLicenseUrl", &self.license_url);
        apply_string_edit(meta, "contactInformation", &self.contact_information);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MetadataFieldEdit<T> {
    #[default]
    Leave,
    Set(T),
    Clear,
}

fn apply_string_edit(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    edit: &MetadataFieldEdit<String>,
) {
    match edit {
        MetadataFieldEdit::Leave => {}
        MetadataFieldEdit::Set(value) => {
            object.insert(key.to_owned(), Value::String(value.clone()));
        }
        MetadataFieldEdit::Clear => {
            object.remove(key);
        }
    }
}

fn apply_string_array_edit(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    edit: &MetadataFieldEdit<Vec<String>>,
) {
    match edit {
        MetadataFieldEdit::Leave => {}
        MetadataFieldEdit::Set(values) => {
            object.insert(
                key.to_owned(),
                Value::Array(values.iter().cloned().map(Value::String).collect()),
            );
        }
        MetadataFieldEdit::Clear => {
            object.remove(key);
        }
    }
}

fn apply_legacy_author_edit(
    object: &mut serde_json::Map<String, Value>,
    edit: &MetadataFieldEdit<Vec<String>>,
) {
    match edit {
        MetadataFieldEdit::Leave => {}
        MetadataFieldEdit::Set(values) => {
            object.insert("author".to_owned(), Value::String(values.join(", ")));
        }
        MetadataFieldEdit::Clear => {
            object.remove("author");
        }
    }
}

fn pad_json_chunk(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(b' ');
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), VrmIoError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)?;
    }

    let file_name = path.file_name().ok_or_else(|| VrmIoError::SourceWrite {
        message: format!("atomic save path has no file name: {}", path.display()),
    })?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temp_name = format!(
        ".{}.tmp.{}.{}",
        file_name.to_string_lossy(),
        std::process::id(),
        unique
    );
    let temp_path = parent.map_or_else(
        || Path::new(&temp_name).to_path_buf(),
        |parent| parent.join(&temp_name),
    );

    let write_result = (|| {
        let mut file = std::fs::File::create(&temp_path)?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp_path, path)?;
        Ok::<(), std::io::Error>(())
    })();

    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error.into());
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GltfSourceFormat {
    Json,
    Glb { version: u32, declared_length: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlbChunk {
    pub kind: GlbChunkKind,
    pub raw_type: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlbChunkKind {
    Json,
    Bin,
    Unknown(u32),
}

const GLB_MAGIC: &[u8; 4] = b"glTF";
const GLB_JSON_CHUNK_TYPE: u32 = 0x4E4F_534A;
const GLB_BIN_CHUNK_TYPE: u32 = 0x004E_4942;

fn is_glb(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[..4] == GLB_MAGIC
}

fn parse_glb_chunks(bytes: &[u8]) -> Result<Vec<GlbChunk>, VrmIoError> {
    if bytes.len() < 12 {
        return Err(VrmIoError::SourcePreservation {
            message: "GLB source is shorter than the 12-byte header".to_owned(),
        });
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("slice length checked"));
    if version != 2 {
        return Err(VrmIoError::SourcePreservation {
            message: format!("unsupported GLB version for source preservation: {version}"),
        });
    }
    let declared_length =
        u32::from_le_bytes(bytes[8..12].try_into().expect("slice length checked")) as usize;
    if declared_length != bytes.len() {
        return Err(VrmIoError::SourcePreservation {
            message: format!(
                "GLB declared length {declared_length} does not match input length {}",
                bytes.len()
            ),
        });
    }
    let mut offset = 12;
    let mut chunks = Vec::new();
    while offset < declared_length {
        if declared_length - offset < 8 {
            return Err(VrmIoError::SourcePreservation {
                message: "truncated GLB chunk header".to_owned(),
            });
        }
        let chunk_len = u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("slice length checked"),
        ) as usize;
        let raw_type = u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .expect("slice length checked"),
        );
        if !chunk_len.is_multiple_of(4) {
            return Err(VrmIoError::SourcePreservation {
                message: format!("GLB chunk length {chunk_len} is not 4-byte aligned"),
            });
        }
        let data_start = offset + 8;
        let data_end =
            data_start
                .checked_add(chunk_len)
                .ok_or_else(|| VrmIoError::SourcePreservation {
                    message: "GLB chunk length overflow".to_owned(),
                })?;
        if data_end > declared_length {
            return Err(VrmIoError::SourcePreservation {
                message: "GLB chunk extends beyond declared length".to_owned(),
            });
        }
        chunks.push(GlbChunk {
            kind: match raw_type {
                GLB_JSON_CHUNK_TYPE => GlbChunkKind::Json,
                GLB_BIN_CHUNK_TYPE => GlbChunkKind::Bin,
                other => GlbChunkKind::Unknown(other),
            },
            raw_type,
            bytes: bytes[data_start..data_end].to_vec(),
        });
        offset = data_end;
    }
    Ok(chunks)
}

impl GltfSceneRest {
    fn from_document(document: &gltf::Document) -> Self {
        NodeRestGraph::from_document(document).into_scene_rest(document.nodes().count())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageData {
    pub mime_type: Option<String>,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    R8,
    R8G8,
    R8G8B8,
    R8G8B8A8,
    R16,
    R16G16,
    R16G16B16,
    R16G16B16A16,
    R32G32B32Float,
    R32G32B32A32Float,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum Rgba8ImageError {
    #[error("image dimensions must be non-zero")]
    InvalidDimensions,
    #[error(
        "image byte length mismatch for {format:?}: expected {expected} bytes, got {actual} bytes"
    )]
    InvalidByteLength {
        format: ImageFormat,
        expected: usize,
        actual: usize,
    },
    #[error("unsupported image format for RGBA8 conversion: {0:?}")]
    UnsupportedFormat(ImageFormat),
}

pub fn image_data_to_rgba8(image: &ImageData) -> Result<Vec<u8>, Rgba8ImageError> {
    image_bytes_to_rgba8(image.width, image.height, image.format, &image.bytes)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rgba8SamplingOrigin {
    #[default]
    TopLeft,
    BottomLeft,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuRgba8Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl CpuRgba8Image {
    pub fn from_image_data(image: &ImageData) -> Result<Self, Rgba8ImageError> {
        Self::from_rgba8(image.width, image.height, image_data_to_rgba8(image)?)
    }

    pub fn from_rgba8(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, Rgba8ImageError> {
        let expected = rgba_len(width, height).map_err(|_| Rgba8ImageError::InvalidDimensions)?;
        if rgba.len() != expected {
            return Err(Rgba8ImageError::InvalidByteLength {
                format: ImageFormat::R8G8B8A8,
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn sample_green_repeat_linear(
        &self,
        tex_coord: [f32; 2],
        origin: Rgba8SamplingOrigin,
    ) -> f32 {
        self.sample_channel_repeat_linear(tex_coord, 1, 255, origin)
    }

    pub fn sample_rgba_repeat_linear(
        &self,
        tex_coord: [f32; 2],
        origin: Rgba8SamplingOrigin,
    ) -> [f32; 4] {
        [
            self.sample_channel_repeat_linear(tex_coord, 0, 0, origin),
            self.sample_channel_repeat_linear(tex_coord, 1, 0, origin),
            self.sample_channel_repeat_linear(tex_coord, 2, 0, origin),
            self.sample_channel_repeat_linear(tex_coord, 3, 255, origin),
        ]
    }

    pub fn sample_rgba8_repeat_linear(
        &self,
        tex_coord: [f32; 2],
        origin: Rgba8SamplingOrigin,
    ) -> [u8; 4] {
        self.sample_rgba_repeat_linear(tex_coord, origin)
            .map(quantize_unorm8)
    }

    pub fn sample_channel_repeat_linear(
        &self,
        tex_coord: [f32; 2],
        channel: usize,
        fallback: u8,
        origin: Rgba8SamplingOrigin,
    ) -> f32 {
        if channel >= 4 {
            return f32::from(fallback) / 255.0;
        }
        let u = tex_coord[0].rem_euclid(1.0);
        let v = tex_coord[1].rem_euclid(1.0);
        let x = u * self.width as f32 - 0.5;
        let y = match origin {
            Rgba8SamplingOrigin::TopLeft => v * self.height as f32 - 0.5,
            Rgba8SamplingOrigin::BottomLeft => (1.0 - v) * self.height as f32 - 0.5,
        };
        let x0 = x.floor();
        let y0 = y.floor();
        let tx = x - x0;
        let ty = y - y0;
        let x0 = x0 as i32;
        let y0 = y0 as i32;
        let top = lerp(
            self.channel_at_repeat(x0, y0, channel, fallback),
            self.channel_at_repeat(x0 + 1, y0, channel, fallback),
            tx,
        );
        let bottom = lerp(
            self.channel_at_repeat(x0, y0 + 1, channel, fallback),
            self.channel_at_repeat(x0 + 1, y0 + 1, channel, fallback),
            tx,
        );
        lerp(top, bottom, ty)
    }

    fn channel_at_repeat(&self, x: i32, y: i32, channel: usize, fallback: u8) -> f32 {
        let width = self.width as i32;
        let height = self.height as i32;
        let x = x.rem_euclid(width) as u32;
        let y = y.rem_euclid(height) as u32;
        let index = ((y * self.width + x) * 4) as usize + channel;
        f32::from(self.rgba.get(index).copied().unwrap_or(fallback)) / 255.0
    }
}

pub fn image_bytes_to_rgba8(
    width: u32,
    height: u32,
    format: ImageFormat,
    bytes: &[u8],
) -> Result<Vec<u8>, Rgba8ImageError> {
    let pixels = checked_pixel_count(width, height).ok_or(Rgba8ImageError::InvalidDimensions)?;
    let Some(bytes_per_pixel) = rgba8_source_bytes_per_pixel(format) else {
        return Err(Rgba8ImageError::UnsupportedFormat(format));
    };
    let expected = pixels
        .checked_mul(bytes_per_pixel)
        .ok_or(Rgba8ImageError::InvalidDimensions)?;
    if bytes.len() != expected {
        return Err(Rgba8ImageError::InvalidByteLength {
            format,
            expected,
            actual: bytes.len(),
        });
    }

    match format {
        ImageFormat::R8 => Ok(bytes
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect()),
        ImageFormat::R8G8 => Ok(bytes
            .chunks_exact(2)
            .flat_map(|chunk| [chunk[0], chunk[0], chunk[0], chunk[1]])
            .collect()),
        ImageFormat::R8G8B8 => Ok(bytes
            .chunks_exact(3)
            .flat_map(|chunk| [chunk[0], chunk[1], chunk[2], 255])
            .collect()),
        ImageFormat::R8G8B8A8 => Ok(bytes.to_vec()),
        ImageFormat::R16
        | ImageFormat::R16G16
        | ImageFormat::R16G16B16
        | ImageFormat::R16G16B16A16
        | ImageFormat::R32G32B32Float
        | ImageFormat::R32G32B32A32Float => Err(Rgba8ImageError::UnsupportedFormat(format)),
    }
}

fn rgba8_source_bytes_per_pixel(format: ImageFormat) -> Option<usize> {
    match format {
        ImageFormat::R8 => Some(1),
        ImageFormat::R8G8 => Some(2),
        ImageFormat::R8G8B8 => Some(3),
        ImageFormat::R8G8B8A8 => Some(4),
        ImageFormat::R16
        | ImageFormat::R16G16
        | ImageFormat::R16G16B16
        | ImageFormat::R16G16B16A16
        | ImageFormat::R32G32B32Float
        | ImageFormat::R32G32B32A32Float => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaMipLevel {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TextureMipError {
    #[error("texture dimensions must be non-zero")]
    InvalidDimensions,
    #[error("RGBA data length mismatch: expected {expected} bytes, got {actual} bytes")]
    InvalidRgbaLength { expected: usize, actual: usize },
}

pub fn generate_rgba_mip_chain(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<Vec<RgbaMipLevel>, TextureMipError> {
    if width == 0 || height == 0 {
        return Err(TextureMipError::InvalidDimensions);
    }
    let expected = rgba_len(width, height)?;
    if rgba.len() != expected {
        return Err(TextureMipError::InvalidRgbaLength {
            expected,
            actual: rgba.len(),
        });
    }

    let mut levels = vec![RgbaMipLevel {
        width,
        height,
        rgba: rgba.to_vec(),
    }];
    let mut current_width = width;
    let mut current_height = height;
    let mut current_rgba = rgba.to_vec();
    while current_width > 1 || current_height > 1 {
        let next_width = (current_width / 2).max(1);
        let next_height = (current_height / 2).max(1);
        let next = downsample_rgba_box(
            current_width,
            current_height,
            &current_rgba,
            next_width,
            next_height,
        );
        current_width = next_width;
        current_height = next_height;
        current_rgba = next;
        levels.push(RgbaMipLevel {
            width: current_width,
            height: current_height,
            rgba: current_rgba.clone(),
        });
    }
    Ok(levels)
}

fn downsample_rgba_box(
    width: u32,
    height: u32,
    rgba: &[u8],
    next_width: u32,
    next_height: u32,
) -> Vec<u8> {
    let mut next = vec![0; (next_width as usize) * (next_height as usize) * 4];
    for y in 0..next_height {
        let source_y0 = (u64::from(y) * u64::from(height) / u64::from(next_height)) as u32;
        let source_y1 = (u64::from(y + 1) * u64::from(height) / u64::from(next_height)) as u32;
        for x in 0..next_width {
            let source_x0 = (u64::from(x) * u64::from(width) / u64::from(next_width)) as u32;
            let source_x1 = (u64::from(x + 1) * u64::from(width) / u64::from(next_width)) as u32;
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for source_y in source_y0..source_y1.max(source_y0 + 1).min(height) {
                for source_x in source_x0..source_x1.max(source_x0 + 1).min(width) {
                    let source = ((source_y * width + source_x) * 4) as usize;
                    for channel in 0..4 {
                        sum[channel] += u32::from(rgba[source + channel]);
                    }
                    count += 1;
                }
            }
            let destination = ((y * next_width + x) * 4) as usize;
            for channel in 0..4 {
                next[destination + channel] = ((sum[channel] + count / 2) / count) as u8;
            }
        }
    }
    next
}

fn rgba_len(width: u32, height: u32) -> Result<usize, TextureMipError> {
    checked_pixel_count(width, height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(TextureMipError::InvalidDimensions)
}

fn checked_pixel_count(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }
    usize::try_from(width).ok().and_then(|width| {
        usize::try_from(height)
            .ok()
            .and_then(|height| width.checked_mul(height))
    })
}

fn lerp(left: f32, right: f32, t: f32) -> f32 {
    left + (right - left) * t
}

fn quantize_unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl From<gltf::image::Format> for ImageFormat {
    fn from(value: gltf::image::Format) -> Self {
        match value {
            gltf::image::Format::R8 => Self::R8,
            gltf::image::Format::R8G8 => Self::R8G8,
            gltf::image::Format::R8G8B8 => Self::R8G8B8,
            gltf::image::Format::R8G8B8A8 => Self::R8G8B8A8,
            gltf::image::Format::R16 => Self::R16,
            gltf::image::Format::R16G16 => Self::R16G16,
            gltf::image::Format::R16G16B16 => Self::R16G16B16,
            gltf::image::Format::R16G16B16A16 => Self::R16G16B16A16,
            gltf::image::Format::R32G32B32FLOAT => Self::R32G32B32Float,
            gltf::image::Format::R32G32B32A32FLOAT => Self::R32G32B32A32Float,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GltfTextureData {
    pub image: usize,
    pub sampler: GltfSamplerData,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfExpressionRenderEffects {
    pub cleared: HashMap<usize, HashSet<usize>>,
    pub weights: HashMap<(usize, usize), f32>,
    pub material_colors: Vec<GltfMaterialColorEffect>,
    pub texture_transforms: Vec<GltfTextureTransformEffect>,
}

impl GltfExpressionRenderEffects {
    pub fn active_morph_weights(
        &self,
        node_index: usize,
        node: &GltfNodeRest,
        mesh: &GltfMeshData,
    ) -> Vec<f32> {
        let mut weights = if node.weights.is_empty() {
            mesh.weights.clone()
        } else {
            node.weights.clone()
        };

        if let Some(cleared) = self.cleared.get(&node_index) {
            for index in cleared {
                if weights.len() <= *index {
                    weights.resize(index + 1, 0.0);
                }
                weights[*index] = 0.0;
            }
        }
        for ((node, index), weight) in &self.weights {
            if *node != node_index {
                continue;
            }
            if weights.len() <= *index {
                weights.resize(index + 1, 0.0);
            }
            weights[*index] += *weight;
        }
        weights
    }

    pub fn apply_color4(&self, initial: [f32; 4], material: Option<usize>, kind: &str) -> [f32; 4] {
        let Some(material) = material else {
            return initial;
        };
        self.material_colors
            .iter()
            .filter(|effect| effect.material == material && effect.kind == kind)
            .fold(initial, |mut color, effect| {
                let target = [
                    effect.target_value.first().copied().unwrap_or(initial[0]),
                    effect.target_value.get(1).copied().unwrap_or(initial[1]),
                    effect.target_value.get(2).copied().unwrap_or(initial[2]),
                    effect.target_value.get(3).copied().unwrap_or(1.0),
                ];
                for index in 0..4 {
                    color[index] += (target[index] - initial[index]) * effect.weight;
                }
                color
            })
    }

    pub fn apply_color3(&self, initial: [f32; 3], material: Option<usize>, kind: &str) -> [f32; 3] {
        let Some(material) = material else {
            return initial;
        };
        self.material_colors
            .iter()
            .filter(|effect| effect.material == material && effect.kind == kind)
            .fold(initial, |mut color, effect| {
                let target = [
                    effect.target_value.first().copied().unwrap_or(initial[0]),
                    effect.target_value.get(1).copied().unwrap_or(initial[1]),
                    effect.target_value.get(2).copied().unwrap_or(initial[2]),
                ];
                for index in 0..3 {
                    color[index] += (target[index] - initial[index]) * effect.weight;
                }
                color
            })
    }

    pub fn apply_uv_transforms(
        &self,
        mut transforms: GltfMaterialUvTransforms,
        material: Option<usize>,
    ) -> GltfMaterialUvTransforms {
        let Some(material) = material else {
            return transforms;
        };
        for effect in self
            .texture_transforms
            .iter()
            .filter(|effect| effect.material == material)
        {
            transforms.base = Some(apply_expression_texture_transform(transforms.base, effect));
            transforms.shade = Some(apply_expression_texture_transform(transforms.shade, effect));
            transforms.shading_shift = Some(apply_expression_texture_transform(
                transforms.shading_shift,
                effect,
            ));
            transforms.normal = Some(apply_expression_texture_transform(
                transforms.normal,
                effect,
            ));
            transforms.matcap = Some(apply_expression_texture_transform(
                transforms.matcap,
                effect,
            ));
            transforms.rim = Some(apply_expression_texture_transform(transforms.rim, effect));
            transforms.outline_width = Some(apply_expression_texture_transform(
                transforms.outline_width,
                effect,
            ));
            transforms.emissive = Some(apply_expression_texture_transform(
                transforms.emissive,
                effect,
            ));
            transforms.occlusion = Some(apply_expression_texture_transform(
                transforms.occlusion,
                effect,
            ));
            transforms.uv_animation_mask = Some(apply_expression_texture_transform(
                transforms.uv_animation_mask,
                effect,
            ));
        }
        transforms
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GltfMaterialColorEffect {
    pub material: usize,
    pub kind: String,
    pub target_value: Vec<f32>,
    pub weight: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfTextureTransformEffect {
    pub material: usize,
    pub scale: Option<[f32; 2]>,
    pub offset: Option<[f32; 2]>,
    pub weight: f32,
}

fn apply_expression_texture_transform(
    initial: Option<TextureTransform2d>,
    effect: &GltfTextureTransformEffect,
) -> TextureTransform2d {
    let initial = initial.unwrap_or_default();
    let target_scale = effect.scale.unwrap_or(initial.scale);
    let target_offset = effect.offset.unwrap_or(initial.offset);
    TextureTransform2d {
        offset: [
            initial.offset[0] + (target_offset[0] - initial.offset[0]) * effect.weight,
            initial.offset[1] + (target_offset[1] - initial.offset[1]) * effect.weight,
        ],
        scale: [
            initial.scale[0] + (target_scale[0] - initial.scale[0]) * effect.weight,
            initial.scale[1] + (target_scale[1] - initial.scale[1]) * effect.weight,
        ],
        rotation: initial.rotation,
        tex_coord: initial.tex_coord,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GltfMaterialTextureSlots {
    pub base: Option<usize>,
    pub shade: Option<usize>,
    pub shading_shift: Option<usize>,
    pub normal: Option<usize>,
    pub matcap: Option<usize>,
    pub rim: Option<usize>,
    pub outline_width: Option<usize>,
    pub emissive: Option<usize>,
    pub occlusion: Option<usize>,
    pub uv_animation_mask: Option<usize>,
}

impl GltfMaterialTextureSlots {
    pub fn binding_plan(self) -> GltfMaterialTextureBindingPlan {
        GltfMaterialTextureBindingPlan {
            bindings: [
                GltfMaterialTextureBinding {
                    slot: GltfMaterialTextureSlot::Base,
                    texture: self.base,
                    color_space: GltfMaterialTextureColorSpace::Srgb,
                    fallback: GltfMaterialTextureFallback::White,
                },
                GltfMaterialTextureBinding {
                    slot: GltfMaterialTextureSlot::Shade,
                    texture: self.shade,
                    color_space: GltfMaterialTextureColorSpace::Srgb,
                    fallback: GltfMaterialTextureFallback::White,
                },
                GltfMaterialTextureBinding {
                    slot: GltfMaterialTextureSlot::ShadingShift,
                    texture: self.shading_shift,
                    color_space: GltfMaterialTextureColorSpace::Srgb,
                    fallback: GltfMaterialTextureFallback::Black,
                },
                GltfMaterialTextureBinding {
                    slot: GltfMaterialTextureSlot::Normal,
                    texture: self.normal,
                    color_space: GltfMaterialTextureColorSpace::Linear,
                    fallback: GltfMaterialTextureFallback::NeutralNormal,
                },
                GltfMaterialTextureBinding {
                    slot: GltfMaterialTextureSlot::Matcap,
                    texture: self.matcap,
                    color_space: GltfMaterialTextureColorSpace::Srgb,
                    fallback: GltfMaterialTextureFallback::Black,
                },
                GltfMaterialTextureBinding {
                    slot: GltfMaterialTextureSlot::Rim,
                    texture: self.rim,
                    color_space: GltfMaterialTextureColorSpace::Srgb,
                    fallback: GltfMaterialTextureFallback::White,
                },
                GltfMaterialTextureBinding {
                    slot: GltfMaterialTextureSlot::Emissive,
                    texture: self.emissive,
                    color_space: GltfMaterialTextureColorSpace::Srgb,
                    fallback: GltfMaterialTextureFallback::White,
                },
                GltfMaterialTextureBinding {
                    slot: GltfMaterialTextureSlot::Occlusion,
                    texture: self.occlusion,
                    color_space: GltfMaterialTextureColorSpace::Linear,
                    fallback: GltfMaterialTextureFallback::White,
                },
                GltfMaterialTextureBinding {
                    slot: GltfMaterialTextureSlot::UvAnimationMask,
                    texture: self.uv_animation_mask,
                    color_space: GltfMaterialTextureColorSpace::Srgb,
                    fallback: GltfMaterialTextureFallback::White,
                },
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GltfMaterialTextureSlot {
    Base,
    Shade,
    ShadingShift,
    Normal,
    Matcap,
    Rim,
    Emissive,
    Occlusion,
    UvAnimationMask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GltfMaterialTextureColorSpace {
    Srgb,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GltfMaterialTextureFallback {
    White,
    Black,
    NeutralNormal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GltfMaterialTextureBinding {
    pub slot: GltfMaterialTextureSlot,
    pub texture: Option<usize>,
    pub color_space: GltfMaterialTextureColorSpace,
    pub fallback: GltfMaterialTextureFallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GltfMaterialTextureBindingPlan {
    pub bindings: [GltfMaterialTextureBinding; 9],
}

impl GltfMaterialTextureBindingPlan {
    pub fn iter(&self) -> impl Iterator<Item = GltfMaterialTextureBinding> + '_ {
        self.bindings.iter().copied()
    }

    pub fn binding(&self, slot: GltfMaterialTextureSlot) -> Option<GltfMaterialTextureBinding> {
        self.iter().find(|binding| binding.slot == slot)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GltfMaterialUvTransforms {
    pub base: Option<TextureTransform2d>,
    pub shade: Option<TextureTransform2d>,
    pub shading_shift: Option<TextureTransform2d>,
    pub normal: Option<TextureTransform2d>,
    pub matcap: Option<TextureTransform2d>,
    pub rim: Option<TextureTransform2d>,
    pub outline_width: Option<TextureTransform2d>,
    pub emissive: Option<TextureTransform2d>,
    pub occlusion: Option<TextureTransform2d>,
    pub uv_animation_mask: Option<TextureTransform2d>,
    pub uv_animation_scroll: [f32; 2],
    pub uv_animation_rotation: f32,
}

impl GltfMaterialUvTransforms {
    pub fn uniform_plan(self) -> GltfMaterialUvUniformPlan {
        GltfMaterialUvUniformPlan {
            base_transform: texture_transform_uniform(self.base),
            shade_transform: texture_transform_uniform(self.shade),
            shading_shift_transform: texture_transform_uniform(self.shading_shift),
            normal_transform: texture_transform_uniform(self.normal),
            matcap_transform: texture_transform_uniform(self.matcap),
            rim_transform: texture_transform_uniform(self.rim),
            emissive_transform: texture_transform_uniform(self.emissive),
            occlusion_transform: texture_transform_uniform(self.occlusion),
            uv_animation_mask_transform: texture_transform_uniform(self.uv_animation_mask),
            rotation_a: [
                texture_transform_rotation(self.base),
                texture_transform_rotation(self.shade),
                texture_transform_rotation(self.shading_shift),
                texture_transform_rotation(self.normal),
            ],
            rotation_b: [
                texture_transform_rotation(self.rim),
                texture_transform_rotation(self.emissive),
                texture_transform_rotation(self.uv_animation_mask),
                texture_transform_rotation(self.matcap),
            ],
            uv_animation: [
                self.uv_animation_scroll[0],
                self.uv_animation_scroll[1],
                self.uv_animation_rotation,
                texture_transform_rotation(self.occlusion),
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfMaterialUvUniformPlan {
    pub base_transform: [f32; 4],
    pub shade_transform: [f32; 4],
    pub shading_shift_transform: [f32; 4],
    pub normal_transform: [f32; 4],
    pub matcap_transform: [f32; 4],
    pub rim_transform: [f32; 4],
    pub emissive_transform: [f32; 4],
    pub occlusion_transform: [f32; 4],
    pub uv_animation_mask_transform: [f32; 4],
    pub rotation_a: [f32; 4],
    pub rotation_b: [f32; 4],
    pub uv_animation: [f32; 4],
}

fn texture_transform_uniform(transform: Option<TextureTransform2d>) -> [f32; 4] {
    let Some(transform) =
        transform.filter(|transform| transform.tex_coord.is_none_or(|tex_coord| tex_coord == 0))
    else {
        return [0.0, 0.0, 1.0, 1.0];
    };
    [
        transform.offset[0],
        transform.offset[1],
        transform.scale[0],
        transform.scale[1],
    ]
}

fn texture_transform_rotation(transform: Option<TextureTransform2d>) -> f32 {
    transform
        .filter(|transform| transform.tex_coord.is_none_or(|tex_coord| tex_coord == 0))
        .map_or(0.0, |transform| transform.rotation)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GltfMaterialShadingOptions {
    pub v0_compat_shade: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfMaterialShadingPlan {
    pub base_color: [f32; 4],
    pub shade_color: [f32; 4],
    pub shading_shift: f32,
    pub shading_toony: f32,
    pub shading_shift_texture_scale: f32,
    pub gi_equalization: f32,
    pub emissive: [f32; 3],
    pub matcap_factor: [f32; 3],
    pub parametric_rim_color: [f32; 3],
    pub rim_lighting_mix: f32,
    pub parametric_rim_fresnel_power: f32,
    pub parametric_rim_lift: f32,
    pub normal_scale: f32,
    pub metallic: f32,
    pub roughness: f32,
    pub occlusion_strength: f32,
    pub pbr_fallback: bool,
    pub unlit: bool,
    pub v0_compat_shade: bool,
}

impl GltfMaterialShadingPlan {
    pub fn render_extra_plan(
        self,
        options: GltfMaterialRenderExtraOptions,
    ) -> GltfMaterialRenderExtraPlan {
        GltfMaterialRenderExtraPlan {
            flags: GltfMaterialRenderFlags {
                v0_compat_shade: self.v0_compat_shade,
                pbr_fallback: self.pbr_fallback,
                three_vrm_light_accumulation: options.light_accumulation.is_three_vrm(),
                derivative_normals: options.derivative_normals,
                unlit: self.unlit,
                view_derivative_normals: options.view_derivative_normals,
            },
            metallic: self.metallic,
            roughness: self.roughness,
            occlusion_strength: self.occlusion_strength,
            direct_light_scale: options.direct_light_scale,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GltfMtoonLightAccumulation {
    Tuned,
    #[default]
    ThreeVrm,
}

impl GltfMtoonLightAccumulation {
    pub fn is_three_vrm(self) -> bool {
        self == Self::ThreeVrm
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfMaterialRenderExtraOptions {
    pub light_accumulation: GltfMtoonLightAccumulation,
    pub derivative_normals: bool,
    pub view_derivative_normals: bool,
    pub direct_light_scale: f32,
}

impl Default for GltfMaterialRenderExtraOptions {
    fn default() -> Self {
        Self {
            light_accumulation: GltfMtoonLightAccumulation::ThreeVrm,
            derivative_normals: false,
            view_derivative_normals: false,
            direct_light_scale: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GltfMaterialRenderFlags {
    pub v0_compat_shade: bool,
    pub pbr_fallback: bool,
    pub three_vrm_light_accumulation: bool,
    pub derivative_normals: bool,
    pub unlit: bool,
    pub view_derivative_normals: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfMaterialRenderExtraPlan {
    pub flags: GltfMaterialRenderFlags,
    pub metallic: f32,
    pub roughness: f32,
    pub occlusion_strength: f32,
    pub direct_light_scale: f32,
}

impl GltfMaterialRenderExtraPlan {
    pub fn uniform_plan(self) -> GltfMaterialRenderExtraUniformPlan {
        GltfMaterialRenderExtraUniformPlan {
            flags: [
                self.flags.v0_compat_shade as u8 as f32,
                self.flags.pbr_fallback as u8 as f32,
                self.flags.three_vrm_light_accumulation as u8 as f32,
                self.flags.derivative_normals as u8 as f32,
            ],
            pbr_params: [
                self.metallic,
                self.roughness,
                self.occlusion_strength,
                self.direct_light_scale,
            ],
            flags2: [
                self.flags.unlit as u8 as f32,
                self.flags.view_derivative_normals as u8 as f32,
                0.0,
                0.0,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfMaterialRenderExtraUniformPlan {
    pub flags: [f32; 4],
    pub pbr_params: [f32; 4],
    pub flags2: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfMtoonOutlinePlan {
    pub width_factor: f32,
    pub width_mode: OutlineWidthMode,
    pub color: [f32; 4],
}

pub fn transform_tex_coord_0(
    tex_coord: [f32; 2],
    transform: Option<TextureTransform2d>,
) -> [f32; 2] {
    let Some(transform) = transform else {
        return tex_coord;
    };
    if transform.tex_coord.is_some_and(|tex_coord| tex_coord != 0) {
        return tex_coord;
    }
    let (sin, cos) = transform.rotation.sin_cos();
    let scaled = [
        tex_coord[0] * transform.scale[0],
        tex_coord[1] * transform.scale[1],
    ];
    [
        cos * scaled[0] - sin * scaled[1] + transform.offset[0],
        sin * scaled[0] + cos * scaled[1] + transform.offset[1],
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GltfSamplerData {
    pub mag_filter: GltfMagFilter,
    pub min_filter: GltfMinFilter,
    pub wrap_s: GltfWrapMode,
    pub wrap_t: GltfWrapMode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GltfMagFilter {
    Nearest,
    #[default]
    Linear,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GltfMinFilter {
    Nearest,
    Linear,
    NearestMipmapNearest,
    LinearMipmapNearest,
    NearestMipmapLinear,
    #[default]
    LinearMipmapLinear,
}

impl GltfMinFilter {
    pub fn uses_mipmaps(self) -> bool {
        matches!(
            self,
            Self::NearestMipmapNearest
                | Self::LinearMipmapNearest
                | Self::NearestMipmapLinear
                | Self::LinearMipmapLinear
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GltfWrapMode {
    ClampToEdge,
    MirroredRepeat,
    #[default]
    Repeat,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GltfMaterialData {
    pub name: Option<String>,
    pub base_color_factor: [f32; 4],
    pub base_color_texture: Option<usize>,
    pub base_color_texture_transform: Option<TextureTransform2d>,
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub normal_texture: Option<usize>,
    pub normal_texture_transform: Option<TextureTransform2d>,
    pub normal_scale: f32,
    pub occlusion_texture: Option<usize>,
    pub occlusion_texture_transform: Option<TextureTransform2d>,
    pub occlusion_strength: f32,
    pub emissive_factor: [f32; 3],
    pub emissive_texture: Option<usize>,
    pub emissive_texture_transform: Option<TextureTransform2d>,
    pub emissive_strength: f32,
    pub unlit: bool,
    pub alpha_mode: GltfAlphaMode,
    pub alpha_cutoff: Option<f32>,
    pub double_sided: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GltfAlphaMode {
    #[default]
    Opaque,
    Mask,
    Blend,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfSceneRest {
    pub nodes: Vec<GltfNodeRest>,
}

impl GltfSceneRest {
    pub fn node(&self, index: usize) -> Option<&GltfNodeRest> {
        self.nodes.get(index)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfNodeRest {
    pub name: Option<String>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
    pub weights: Vec<f32>,
    pub local: Transform,
    pub world: Transform,
    pub world_matrix: Mat4,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfSkinData {
    pub joints: Vec<usize>,
    pub inverse_bind_matrices: Vec<Mat4>,
}

impl GltfSkinData {
    pub fn joint_matrices(
        &self,
        scene: &GltfSceneRest,
        world_matrices: &[Mat4],
        orientation: Mat4,
    ) -> Vec<Mat4> {
        self.joints
            .iter()
            .enumerate()
            .map(|(index, joint)| {
                let joint_world = world_matrices
                    .get(*joint)
                    .copied()
                    .or_else(|| scene.node(*joint).map(|node| node.world_matrix))
                    .unwrap_or(Mat4::IDENTITY);
                let inverse_bind = self
                    .inverse_bind_matrices
                    .get(index)
                    .copied()
                    .unwrap_or(Mat4::IDENTITY);
                orientation * joint_world * inverse_bind
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfMeshData {
    pub name: Option<String>,
    pub weights: Vec<f32>,
    pub primitives: Vec<GltfPrimitiveData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfMorphTargetData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GltfNormalMapMode {
    #[default]
    GeneratedTangents,
    Derivative,
    ViewDerivative,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfNormalMapPlan {
    pub normal_scale: f32,
    pub mode: GltfNormalMapMode,
    pub authored_tangents: bool,
}

impl GltfNormalMapPlan {
    pub fn new(normal_scale: f32, authored_tangents: bool, mode: GltfNormalMapMode) -> Self {
        Self {
            normal_scale: normal_scale.max(0.0),
            mode,
            authored_tangents,
        }
    }

    pub fn disabled() -> Self {
        Self::new(0.0, false, GltfNormalMapMode::GeneratedTangents)
    }

    pub fn is_enabled(self) -> bool {
        self.normal_scale > 0.0
    }

    pub fn should_generate_tangents(self) -> bool {
        self.is_enabled()
            && !self.authored_tangents
            && self.mode == GltfNormalMapMode::GeneratedTangents
    }

    pub fn uses_derivative_normals(self) -> bool {
        self.is_enabled()
            && !self.authored_tangents
            && matches!(
                self.mode,
                GltfNormalMapMode::Derivative | GltfNormalMapMode::ViewDerivative
            )
    }

    pub fn uses_view_derivative_normals(self) -> bool {
        self.is_enabled()
            && !self.authored_tangents
            && self.mode == GltfNormalMapMode::ViewDerivative
    }

    pub fn material_normal_scale(self, has_runtime_tangents: bool) -> f32 {
        if self.uses_derivative_normals() || has_runtime_tangents {
            self.normal_scale
        } else {
            0.0
        }
    }

    pub fn vertex_normal_scale(self, has_vertex_tangent: bool) -> f32 {
        if self.uses_derivative_normals() {
            -self.normal_scale
        } else if has_vertex_tangent {
            self.normal_scale
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfPrimitiveData {
    pub material: Option<usize>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub tex_coords_0: Vec<[f32; 2]>,
    pub colors_0: Vec<[f32; 4]>,
    pub joints_0: Vec<[u16; 4]>,
    pub weights_0: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub morph_targets: Vec<GltfMorphTargetData>,
}

impl GltfPrimitiveData {
    pub fn has_authored_tangents(&self) -> bool {
        self.tangents.len() == self.positions.len()
    }

    pub fn normal_map_plan(&self, normal_scale: f32, mode: GltfNormalMapMode) -> GltfNormalMapPlan {
        GltfNormalMapPlan::new(normal_scale, self.has_authored_tangents(), mode)
    }

    pub fn tex_coord_0_or_default(&self, index: usize) -> [f32; 2] {
        self.tex_coords_0.get(index).copied().unwrap_or([0.0, 0.0])
    }

    pub fn tex_coords_0_or_defaults(&self) -> Vec<[f32; 2]> {
        if self.tex_coords_0.len() == self.positions.len() {
            self.tex_coords_0.clone()
        } else {
            vec![[0.0, 0.0]; self.positions.len()]
        }
    }

    pub fn morphed_vertex(&self, index: usize, morph_weights: &[f32]) -> Option<GltfMorphedVertex> {
        let mut position = Vec3::from_array(*self.positions.get(index)?);
        let mut normal = self
            .normals
            .get(index)
            .copied()
            .map(Vec3::from_array)
            .unwrap_or(Vec3::Z);
        let base_tangent = self
            .tangents
            .get(index)
            .copied()
            .unwrap_or([1.0, 0.0, 0.0, 1.0]);
        let mut tangent = Vec3::new(base_tangent[0], base_tangent[1], base_tangent[2]);

        for (target, weight) in self
            .morph_targets
            .iter()
            .zip(morph_weights.iter().copied())
            .filter(|(_, weight)| weight.abs() > f32::EPSILON)
        {
            if let Some(delta) = target.positions.get(index).copied() {
                position += Vec3::from_array(delta) * weight;
            }
            if let Some(delta) = target.normals.get(index).copied() {
                normal += Vec3::from_array(delta) * weight;
            }
            if let Some(delta) = target.tangents.get(index).copied() {
                tangent += Vec3::from_array(delta) * weight;
            }
        }

        Some(GltfMorphedVertex {
            position,
            normal,
            tangent: tangent.extend(base_tangent[3]),
        })
    }

    pub fn transformed_vertex(
        &self,
        index: usize,
        morph_weights: &[f32],
        world: Mat4,
        skin_matrices: Option<&[Mat4]>,
    ) -> Option<GltfTransformedVertex> {
        let morphed = self.morphed_vertex(index, morph_weights)?;
        let joints = self.joints_0.get(index).copied();
        let weights = self.weights_0.get(index).copied();
        let skinned = skin_vertex(
            morphed.position,
            morphed.normal,
            world,
            skin_matrices,
            joints,
            weights,
        );
        let tangent = skin_direction(
            morphed.tangent.truncate(),
            world,
            skin_matrices,
            joints,
            weights,
        )
        .extend(morphed.tangent.w);

        Some(GltfTransformedVertex {
            position: skinned.position,
            normal: skinned.normal,
            tangent,
            tex_coord_0: self.tex_coord_0_or_default(index),
            color_0: self
                .colors_0
                .get(index)
                .copied()
                .unwrap_or([1.0, 1.0, 1.0, 1.0]),
        })
    }

    pub fn transformed_vertices(
        &self,
        morph_weights: &[f32],
        world: Mat4,
        skin_matrices: Option<&[Mat4]>,
    ) -> Option<Vec<GltfTransformedVertex>> {
        (0..self.positions.len())
            .map(|index| self.transformed_vertex(index, morph_weights, world, skin_matrices))
            .collect()
    }

    pub fn outline_position(
        &self,
        index: usize,
        morph_weights: &[f32],
        settings: GltfOutlineSettings,
        world: Mat4,
        skin_matrices: Option<&[Mat4]>,
    ) -> Option<Vec3> {
        let morphed = self.morphed_vertex(index, morph_weights)?;
        let normal = morphed.normal.normalize_or_zero();
        let transform = blended_vertex_transform(
            world,
            skin_matrices,
            self.joints_0.get(index).copied(),
            self.weights_0.get(index).copied(),
        );
        let world_position = transform.transform_point3(morphed.position);
        let normal_scale = normal_matrix_length(transform, normal);
        let offset_scale = settings.width * normal_scale * settings.scale.at(world_position);
        let offset = normal * offset_scale;
        Some(transform.transform_point3(morphed.position + offset))
    }

    pub fn outline_vertices(
        &self,
        morph_weights: &[f32],
        settings: GltfOutlineVertexSettings<'_>,
        world: Mat4,
        skin_matrices: Option<&[Mat4]>,
    ) -> Option<Vec<GltfTransformedVertex>> {
        (0..self.positions.len())
            .map(|index| {
                let mut vertex =
                    self.transformed_vertex(index, morph_weights, world, skin_matrices)?;
                let width = settings.base_width
                    * settings
                        .width_texture
                        .map(|image| {
                            image.sample_green_repeat_linear(
                                transform_tex_coord_0(vertex.tex_coord_0, settings.width_transform),
                                settings.width_texture_origin,
                            )
                        })
                        .unwrap_or(1.0);
                vertex.position = self.outline_position(
                    index,
                    morph_weights,
                    GltfOutlineSettings {
                        width,
                        scale: settings.scale,
                    },
                    world,
                    skin_matrices,
                )?;
                Some(vertex)
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfMorphedVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub tangent: Vec4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfTransformedVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub tangent: Vec4,
    pub tex_coord_0: [f32; 2],
    pub color_0: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfSkinnedVertex {
    pub position: Vec3,
    pub normal: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfOutlineSettings {
    pub width: f32,
    pub scale: GltfOutlineScale,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfOutlineVertexSettings<'a> {
    pub base_width: f32,
    pub scale: GltfOutlineScale,
    pub width_texture: Option<&'a CpuRgba8Image>,
    pub width_transform: Option<TextureTransform2d>,
    pub width_texture_origin: Rgba8SamplingOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfOutlineScale {
    pub mode: OutlineWidthMode,
    pub view: Mat4,
    pub projection_y: f32,
}

impl GltfOutlineScale {
    pub fn new(mode: OutlineWidthMode, view: Mat4, projection_y: f32) -> Self {
        Self {
            mode,
            view,
            projection_y,
        }
    }

    pub fn at(self, world_position: Vec3) -> f32 {
        match self.mode {
            OutlineWidthMode::ScreenCoordinates => {
                let view_z = self.view.transform_point3(world_position).z;
                (-view_z / self.projection_y).max(0.0)
            }
            OutlineWidthMode::None
            | OutlineWidthMode::WorldCoordinates
            | OutlineWidthMode::Unknown => 1.0,
        }
    }
}

pub fn skin_vertex(
    position: Vec3,
    normal: Vec3,
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    joints: Option<[u16; 4]>,
    weights: Option<[f32; 4]>,
) -> GltfSkinnedVertex {
    let fallback = || GltfSkinnedVertex {
        position: world.transform_point3(position),
        normal: world.transform_vector3(normal).normalize_or_zero(),
    };
    let (Some(skin_matrices), Some(joints), Some(weights)) = (skin_matrices, joints, weights)
    else {
        return fallback();
    };

    let (skinned_position, skinned_normal, total_weight) = joints
        .into_iter()
        .zip(weights)
        .filter(|(_, weight)| *weight > 0.0)
        .filter_map(|(joint, weight)| {
            skin_matrices
                .get(usize::from(joint))
                .map(|matrix| (matrix, weight))
        })
        .fold(
            (Vec3::ZERO, Vec3::ZERO, 0.0),
            |(skinned_position, skinned_normal, total_weight), (matrix, weight)| {
                (
                    skinned_position + matrix.transform_point3(position) * weight,
                    skinned_normal + matrix.transform_vector3(normal) * weight,
                    total_weight + weight,
                )
            },
        );

    if total_weight > 0.0 {
        GltfSkinnedVertex {
            position: skinned_position,
            normal: skinned_normal.normalize_or_zero(),
        }
    } else {
        fallback()
    }
}

pub fn skin_direction(
    direction: Vec3,
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    joints: Option<[u16; 4]>,
    weights: Option<[f32; 4]>,
) -> Vec3 {
    let fallback = || world.transform_vector3(direction).normalize_or_zero();
    let (Some(skin_matrices), Some(joints), Some(weights)) = (skin_matrices, joints, weights)
    else {
        return fallback();
    };

    let (transformed, total_weight) = joints
        .into_iter()
        .zip(weights)
        .filter(|(_, weight)| *weight > 0.0)
        .filter_map(|(joint, weight)| {
            skin_matrices
                .get(usize::from(joint))
                .map(|matrix| (matrix, weight))
        })
        .fold(
            (Vec3::ZERO, 0.0),
            |(transformed, total_weight), (matrix, weight)| {
                (
                    transformed + matrix.transform_vector3(direction) * weight,
                    total_weight + weight,
                )
            },
        );

    if total_weight > 0.0 {
        transformed.normalize_or_zero()
    } else {
        fallback()
    }
}

pub fn blended_vertex_transform(
    world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    joints: Option<[u16; 4]>,
    weights: Option<[f32; 4]>,
) -> Mat4 {
    let (Some(skin_matrices), Some(joints), Some(weights)) = (skin_matrices, joints, weights)
    else {
        return world;
    };

    let (transform, total_weight) = joints
        .into_iter()
        .zip(weights)
        .filter(|(_, weight)| *weight > 0.0)
        .filter_map(|(joint, weight)| {
            skin_matrices
                .get(usize::from(joint))
                .map(|matrix| (matrix, weight))
        })
        .fold(
            (Mat4::ZERO, 0.0),
            |(transform, total_weight), (matrix, weight)| {
                (transform + *matrix * weight, total_weight + weight)
            },
        );

    if total_weight > 0.0 { transform } else { world }
}

pub fn normal_matrix_length(transform: Mat4, normal: Vec3) -> f32 {
    if normal.length_squared() <= f32::EPSILON || transform.determinant().abs() <= 0.000001 {
        return 1.0;
    }
    let length = transform
        .inverse()
        .transpose()
        .transform_vector3(normal)
        .length();
    if length.is_finite() && length > 0.0 {
        length
    } else {
        1.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GltfGeneratedTangents {
    pub tangents: Vec<Option<[f32; 4]>>,
}

impl GltfGeneratedTangents {
    pub fn all_tangents(&self) -> Option<Vec<[f32; 4]>> {
        self.tangents.iter().copied().collect()
    }
}

pub fn generate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    tex_coords: &[[f32; 2]],
    indices: &[u32],
) -> Option<GltfGeneratedTangents> {
    if normals.len() != positions.len() || tex_coords.len() != positions.len() {
        return None;
    }

    let mut tangents = vec![Vec3::ZERO; positions.len()];
    let mut bitangents = vec![Vec3::ZERO; positions.len()];
    let mut referenced = vec![false; positions.len()];

    for triangle in indices.chunks_exact(3) {
        let [i0, i1, i2] = [
            usize::try_from(triangle[0]).ok()?,
            usize::try_from(triangle[1]).ok()?,
            usize::try_from(triangle[2]).ok()?,
        ];
        for index in [i0, i1, i2] {
            *referenced.get_mut(index)? = true;
        }

        let [p0, p1, p2] = [
            Vec3::from_array(*positions.get(i0)?),
            Vec3::from_array(*positions.get(i1)?),
            Vec3::from_array(*positions.get(i2)?),
        ];
        let [uv0, uv1, uv2] = [
            *tex_coords.get(i0)?,
            *tex_coords.get(i1)?,
            *tex_coords.get(i2)?,
        ];
        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let delta_uv1 = [uv1[0] - uv0[0], uv1[1] - uv0[1]];
        let delta_uv2 = [uv2[0] - uv0[0], uv2[1] - uv0[1]];
        let determinant = delta_uv1[0] * delta_uv2[1] - delta_uv1[1] * delta_uv2[0];
        if determinant.abs() < 0.000001 {
            continue;
        }

        let scale = determinant.recip();
        let tangent = (edge1 * delta_uv2[1] - edge2 * delta_uv1[1]) * scale;
        let bitangent = (edge2 * delta_uv1[0] - edge1 * delta_uv2[0]) * scale;
        for index in [i0, i1, i2] {
            tangents[index] += tangent;
            bitangents[index] += bitangent;
        }
    }

    let tangents = tangents
        .into_iter()
        .zip(bitangents)
        .zip(referenced)
        .zip(normals)
        .map(|(((tangent, bitangent), referenced), normal)| {
            let normal = Vec3::from_array(*normal).normalize_or_zero();
            let tangent = tangent - normal * normal.dot(tangent);
            if tangent.length_squared() < 0.000001 || bitangent.length_squared() < 0.000001 {
                return (!referenced).then(|| fallback_tangent(normal));
            }
            let tangent = tangent.normalize();
            let handedness = if normal.cross(tangent).dot(bitangent) < 0.0 {
                -1.0
            } else {
                1.0
            };
            Some([tangent.x, tangent.y, tangent.z, handedness])
        })
        .collect();

    Some(GltfGeneratedTangents { tangents })
}

pub fn fallback_tangent(normal: Vec3) -> [f32; 4] {
    let seed = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let tangent = (seed - normal * normal.dot(seed)).normalize_or_zero();
    [tangent.x, tangent.y, tangent.z, 1.0]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VrmIoWarning {
    MissingSpecVersion { extension: String, assumed: String },
    DraftSpecVersion { extension: String, version: String },
    UnknownSpecVersion { extension: String, version: String },
    IgnoredAnimationChannel { node: usize, message: String },
}

impl VrmIoWarning {
    #[must_use]
    pub fn to_diagnostic(&self) -> Diagnostic {
        match self {
            Self::MissingSpecVersion { extension, assumed } => Diagnostic::warning(
                "vrm.extension.missing_spec_version",
                extension_spec_version_path(extension),
                format!("{extension} is missing specVersion; assuming {assumed}"),
            ),
            Self::DraftSpecVersion { extension, version } => Diagnostic::warning(
                "vrm.extension.draft_spec_version",
                extension_spec_version_path(extension),
                format!("{extension} uses draft specVersion {version}"),
            ),
            Self::UnknownSpecVersion { extension, version } => Diagnostic::warning(
                "vrm.extension.unknown_spec_version",
                extension_spec_version_path(extension),
                format!("{extension} uses unknown specVersion {version}"),
            ),
            Self::IgnoredAnimationChannel { node, message } => Diagnostic::warning(
                "vrm.animation.ignored_channel",
                JsonPath::root().child("animations"),
                format!("ignored animation channel targeting node {node}: {message}"),
            ),
        }
    }
}

fn extension_path(extension: &str) -> JsonPath {
    JsonPath::root().child("extensions").child(extension)
}

fn extension_spec_version_path(extension: &str) -> JsonPath {
    extension_path(extension).child("specVersion")
}

fn append_warning_diagnostics(warnings: &[VrmIoWarning], diagnostics: &mut DiagnosticReport) {
    for warning in warnings {
        diagnostics.push(warning.to_diagnostic());
    }
}

fn append_unknown_extension_diagnostics(
    bundle: &ExtensionBundle,
    diagnostics: &mut DiagnosticReport,
) {
    for name in bundle.unknown.keys() {
        diagnostics.warning(
            "vrm.extension.unknown",
            extension_path(name),
            format!("unknown root extension {name} was preserved in the source sidecar"),
        );
    }
}

fn extract_meshes(document: &gltf::Document, buffers: &[gltf::buffer::Data]) -> Vec<GltfMeshData> {
    document
        .meshes()
        .map(|mesh| GltfMeshData {
            name: mesh.name().map(ToOwned::to_owned),
            weights: mesh.weights().unwrap_or_default().to_vec(),
            primitives: mesh
                .primitives()
                .map(|primitive| {
                    let reader = primitive
                        .reader(|buffer| buffers.get(buffer.index()).map(|data| data.0.as_slice()));
                    let positions: Vec<[f32; 3]> = reader
                        .read_positions()
                        .map(Iterator::collect)
                        .unwrap_or_default();
                    let normals: Vec<[f32; 3]> = reader
                        .read_normals()
                        .map(Iterator::collect)
                        .unwrap_or_default();
                    let tangents: Vec<[f32; 4]> = reader
                        .read_tangents()
                        .map(Iterator::collect)
                        .unwrap_or_default();
                    let tex_coords_0: Vec<[f32; 2]> = reader
                        .read_tex_coords(0)
                        .map(|coords| coords.into_f32().collect())
                        .unwrap_or_default();
                    let colors_0: Vec<[f32; 4]> = reader
                        .read_colors(0)
                        .map(|colors| colors.into_rgba_f32().collect())
                        .unwrap_or_default();
                    let joints_0: Vec<[u16; 4]> = reader
                        .read_joints(0)
                        .map(|joints| joints.into_u16().collect())
                        .unwrap_or_default();
                    let weights_0: Vec<[f32; 4]> = reader
                        .read_weights(0)
                        .map(|weights| weights.into_f32().collect())
                        .unwrap_or_default();
                    let indices = reader
                        .read_indices()
                        .map(|indices| indices.into_u32().collect())
                        .unwrap_or_else(|| (0..positions.len() as u32).collect());
                    let morph_targets = reader
                        .read_morph_targets()
                        .map(|(positions, normals, tangents)| GltfMorphTargetData {
                            positions: positions.map(Iterator::collect).unwrap_or_default(),
                            normals: normals.map(Iterator::collect).unwrap_or_default(),
                            tangents: tangents.map(Iterator::collect).unwrap_or_default(),
                        })
                        .collect();
                    GltfPrimitiveData {
                        material: primitive.material().index(),
                        positions,
                        normals,
                        tangents,
                        tex_coords_0,
                        colors_0,
                        joints_0,
                        weights_0,
                        indices,
                        morph_targets,
                    }
                })
                .collect(),
        })
        .collect()
}

fn extract_skins(document: &gltf::Document, buffers: &[gltf::buffer::Data]) -> Vec<GltfSkinData> {
    document
        .skins()
        .map(|skin| {
            let joints = skin.joints().map(|joint| joint.index()).collect::<Vec<_>>();
            let inverse_bind_matrices = skin
                .reader(|buffer| buffers.get(buffer.index()).map(|data| data.0.as_slice()))
                .read_inverse_bind_matrices()
                .map(|matrices| {
                    matrices
                        .map(|matrix| Mat4::from_cols_array_2d(&matrix))
                        .collect()
                })
                .unwrap_or_else(|| vec![Mat4::IDENTITY; joints.len()]);
            GltfSkinData {
                joints,
                inverse_bind_matrices,
            }
        })
        .collect()
}

fn extract_textures(document: &gltf::Document) -> Vec<GltfTextureData> {
    document
        .textures()
        .map(|texture| {
            let sampler = texture.sampler();
            GltfTextureData {
                image: texture.source().index(),
                sampler: GltfSamplerData {
                    mag_filter: sampler.mag_filter().map(Into::into).unwrap_or_default(),
                    min_filter: sampler.min_filter().map(Into::into).unwrap_or_default(),
                    wrap_s: sampler.wrap_s().into(),
                    wrap_t: sampler.wrap_t().into(),
                },
            }
        })
        .collect()
}

impl From<gltf::texture::MagFilter> for GltfMagFilter {
    fn from(value: gltf::texture::MagFilter) -> Self {
        match value {
            gltf::texture::MagFilter::Nearest => Self::Nearest,
            gltf::texture::MagFilter::Linear => Self::Linear,
        }
    }
}

impl From<gltf::texture::MinFilter> for GltfMinFilter {
    fn from(value: gltf::texture::MinFilter) -> Self {
        match value {
            gltf::texture::MinFilter::Nearest => Self::Nearest,
            gltf::texture::MinFilter::Linear => Self::Linear,
            gltf::texture::MinFilter::NearestMipmapNearest => Self::NearestMipmapNearest,
            gltf::texture::MinFilter::LinearMipmapNearest => Self::LinearMipmapNearest,
            gltf::texture::MinFilter::NearestMipmapLinear => Self::NearestMipmapLinear,
            gltf::texture::MinFilter::LinearMipmapLinear => Self::LinearMipmapLinear,
        }
    }
}

impl From<gltf::texture::WrappingMode> for GltfWrapMode {
    fn from(value: gltf::texture::WrappingMode) -> Self {
        match value {
            gltf::texture::WrappingMode::ClampToEdge => Self::ClampToEdge,
            gltf::texture::WrappingMode::MirroredRepeat => Self::MirroredRepeat,
            gltf::texture::WrappingMode::Repeat => Self::Repeat,
        }
    }
}

fn extract_gltf_materials(document: &gltf::Document) -> Vec<GltfMaterialData> {
    document
        .materials()
        .map(|material| {
            let pbr = material.pbr_metallic_roughness();
            let base_color_texture = pbr.base_color_texture();
            let normal_texture = material.normal_texture();
            let occlusion_texture = material.occlusion_texture();
            let emissive_texture = material.emissive_texture();
            GltfMaterialData {
                name: material.name().map(str::to_owned),
                base_color_factor: pbr.base_color_factor(),
                base_color_texture: base_color_texture
                    .as_ref()
                    .map(|texture| texture.texture().index()),
                base_color_texture_transform: base_color_texture
                    .as_ref()
                    .and_then(texture_transform),
                metallic_factor: pbr.metallic_factor(),
                roughness_factor: pbr.roughness_factor(),
                normal_texture: normal_texture
                    .as_ref()
                    .map(|texture| texture.texture().index()),
                normal_texture_transform: normal_texture.as_ref().and_then(|texture| {
                    texture_transform_value(
                        texture.extension_value("KHR_texture_transform"),
                        Some(texture.tex_coord()),
                    )
                }),
                normal_scale: normal_texture
                    .as_ref()
                    .map_or(1.0, |texture| texture.scale()),
                occlusion_texture: occlusion_texture
                    .as_ref()
                    .map(|texture| texture.texture().index()),
                occlusion_texture_transform: occlusion_texture.as_ref().and_then(|texture| {
                    texture_transform_value(
                        texture.extension_value("KHR_texture_transform"),
                        Some(texture.tex_coord()),
                    )
                }),
                occlusion_strength: occlusion_texture
                    .as_ref()
                    .map_or(1.0, |texture| texture.strength()),
                emissive_factor: material.emissive_factor(),
                emissive_texture: emissive_texture
                    .as_ref()
                    .map(|texture| texture.texture().index()),
                emissive_texture_transform: emissive_texture.as_ref().and_then(texture_transform),
                emissive_strength: khr_emissive_strength(
                    material.extension_value("KHR_materials_emissive_strength"),
                ),
                unlit: material.unlit(),
                alpha_mode: match material.alpha_mode() {
                    gltf::material::AlphaMode::Opaque => GltfAlphaMode::Opaque,
                    gltf::material::AlphaMode::Mask => GltfAlphaMode::Mask,
                    gltf::material::AlphaMode::Blend => GltfAlphaMode::Blend,
                },
                alpha_cutoff: material.alpha_cutoff(),
                double_sided: material.double_sided(),
            }
        })
        .collect()
}

fn texture_transform(texture: &gltf::texture::Info<'_>) -> Option<TextureTransform2d> {
    let transform = texture.texture_transform()?;
    Some(TextureTransform2d {
        offset: transform.offset(),
        scale: transform.scale(),
        rotation: transform.rotation(),
        tex_coord: transform.tex_coord().or(Some(texture.tex_coord())),
    })
}

fn texture_transform_value(
    value: Option<&Value>,
    texture_tex_coord: Option<u32>,
) -> Option<TextureTransform2d> {
    let value = value?;
    let offset = vec2_value(value.get("offset")).unwrap_or([0.0, 0.0]);
    let scale = vec2_value(value.get("scale")).unwrap_or([1.0, 1.0]);
    let rotation = value
        .get("rotation")
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or(0.0);
    let tex_coord = value
        .get("texCoord")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .or(texture_tex_coord);
    Some(TextureTransform2d {
        offset,
        scale,
        rotation,
        tex_coord,
    })
}

fn vec2_value(value: Option<&Value>) -> Option<[f32; 2]> {
    let array = value?.as_array()?;
    let [x, y] = array.as_slice() else {
        return None;
    };
    Some([x.as_f64()? as f32, y.as_f64()? as f32])
}

fn khr_emissive_strength(extension: Option<&Value>) -> f32 {
    extension
        .and_then(|value| value.get("emissiveStrength"))
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or(1.0)
}

fn vrma_extension_warnings(extensions: &ExtensionMap) -> Vec<VrmIoWarning> {
    let Some(value) = extensions.get("VRMC_vrm_animation") else {
        return Vec::new();
    };
    match value.get("specVersion").and_then(|value| value.as_str()) {
        None => vec![VrmIoWarning::MissingSpecVersion {
            extension: "VRMC_vrm_animation".to_owned(),
            assumed: "1.0".to_owned(),
        }],
        Some("1.0") => Vec::new(),
        Some("1.0-draft") => vec![VrmIoWarning::DraftSpecVersion {
            extension: "VRMC_vrm_animation".to_owned(),
            version: "1.0-draft".to_owned(),
        }],
        Some(version) => vec![VrmIoWarning::UnknownSpecVersion {
            extension: "VRMC_vrm_animation".to_owned(),
            version: version.to_owned(),
        }],
    }
}

pub fn load_vrm_from_slice(bytes: &[u8]) -> Result<LoadedVrm, VrmIoError> {
    load_vrm_from_slice_with_policy(bytes, DiagnosticPolicy::Strict).map(|load| load.loaded)
}

pub fn load_vrm_from_slice_with_policy(
    bytes: &[u8],
    diagnostic_policy: DiagnosticPolicy,
) -> Result<LoadedVrmWithDiagnostics, VrmIoError> {
    let (document, buffers, images) = gltf::import_slice(bytes)?;
    let source = GltfSource::from_slice(bytes)?;
    let scene = GltfSceneRest::from_document(&document);
    let meshes = extract_meshes(&document, &buffers);
    let skins = extract_skins(&document, &buffers);
    let gltf_materials = extract_gltf_materials(&document);
    let textures = extract_textures(&document);
    let root_extensions = extension_map(document.as_json().extensions.as_ref());
    let mut warnings = vrma_extension_warnings(&root_extensions);
    let mut diagnostics = DiagnosticReport::new();
    let mut bundle = parse_root_extensions(&root_extensions)?;
    extract_node_constraints(&document, &mut bundle)?;
    extract_mtoon_materials(&document, &mut bundle)?;
    extract_hdr_emissive_multipliers(&document, &mut bundle)?;
    extract_khr_emissive_strengths(&document, &mut bundle)?;
    validate_vrmc_extension_versions(&bundle)?;
    let vrma_animations = extract_vrma_animations(&document, &buffers, &bundle, &mut warnings)?;
    append_unknown_extension_diagnostics(&bundle, &mut diagnostics);
    append_warning_diagnostics(&warnings, &mut diagnostics);

    let image_data = images
        .into_iter()
        .map(|image| ImageData {
            mime_type: image.format.to_mime_type().map(str::to_owned),
            bytes: image.pixels,
            width: image.width,
            height: image.height,
            format: image.format.into(),
        })
        .collect();

    let node_count = document.nodes().count();
    let material_count = document.materials().count();
    let build = ValidatedAssetBuilder::new()
        .with_node_count(node_count)
        .with_material_count(material_count)
        .with_diagnostic_policy(diagnostic_policy)
        .build_with_diagnostics(bundle)?;
    diagnostics.merge(build.diagnostics);
    let mut asset = build.value;
    merge_gltf_material_params_into_mtoon(&mut asset.document.materials, &gltf_materials);
    expand_vrm0_spring_roots(&mut asset.document, &scene);
    if let Some(animations) = vrma_animations {
        asset.document.animation = animations
            .first()
            .cloned()
            .map_or(Feature::Absent, Feature::Present);
        asset.document.animations = animations;
    }
    let model = asset.resolve();

    Ok(LoadedVrmWithDiagnostics {
        loaded: LoadedVrm {
            model,
            source,
            scene,
            meshes,
            skins,
            gltf_materials,
            textures,
            buffers: buffers.into_iter().map(|buffer| buffer.0).collect(),
            images: image_data,
            warnings,
        },
        diagnostics,
    })
}

fn merge_gltf_material_params_into_mtoon(
    materials: &mut [vrm_core::Material],
    gltf_materials: &[GltfMaterialData],
) {
    for (material, gltf) in materials.iter_mut().zip(gltf_materials) {
        let Feature::Present(mtoon) = &mut material.mtoon else {
            continue;
        };
        if mtoon.base_color_factor == MtoonMaterial::default().base_color_factor {
            mtoon.base_color_factor = gltf.base_color_factor;
        }
        if mtoon.emissive_factor == MtoonMaterial::default().emissive_factor {
            mtoon.emissive_factor = gltf.emissive_factor;
        }
        if mtoon.textures.main_texture.is_none() {
            mtoon.textures.main_texture = gltf.base_color_texture.map(TextureRef);
        }
        if mtoon.texture_transforms.main_texture.is_none() {
            mtoon.texture_transforms.main_texture = gltf.base_color_texture_transform;
        }
        if mtoon.textures.normal_texture.is_none() {
            mtoon.textures.normal_texture = gltf.normal_texture.map(TextureRef);
        }
        if mtoon.texture_transforms.normal_texture.is_none() {
            mtoon.texture_transforms.normal_texture = gltf.normal_texture_transform;
        }
    }
}

fn extract_vrma_animations(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    bundle: &ExtensionBundle,
    warnings: &mut Vec<VrmIoWarning>,
) -> Result<Option<Vec<VrmAnimation>>, VrmIoError> {
    let Some(VrmExtension::Vrma(vrma)) = &bundle.vrm else {
        return Ok(None);
    };

    let node_map = VrmaNodeMap::from_extension(vrma);
    let rest_pose = VrmaRestPose::from_document(document, &node_map);
    let animations = document
        .animations()
        .map(|animation| {
            let mut result = VrmAnimation {
                rest_hips_position: rest_pose.hips_world_position,
                ..VrmAnimation::default()
            };

            for channel in animation.channels() {
                let node_index = channel.target().node().index();
                let reader = channel
                    .reader(|buffer| buffers.get(buffer.index()).map(|data| data.0.as_slice()));
                let times = reader
                    .read_inputs()
                    .ok_or(VrmIoError::InvalidAnimationChannel {
                        message: "missing animation input accessor".to_owned(),
                    })?
                    .collect::<Vec<_>>();

                if let Some(bone_name) = node_map.humanoid.get(&node_index) {
                    match reader.read_outputs() {
                        Some(gltf::animation::util::ReadOutputs::Translations(values))
                            if *bone_name == HumanBoneName::Hips =>
                        {
                            result.hips_translation = Some(TranslationTrack {
                                times: times.clone(),
                                values: values
                                    .map(Vec3::from_array)
                                    .map(|translation| {
                                        rest_pose
                                            .hips_parent_world_matrix
                                            .transform_point3(translation)
                                    })
                                    .collect(),
                            });
                        }
                        Some(gltf::animation::util::ReadOutputs::Translations(_)) => {
                            warnings.push(VrmIoWarning::IgnoredAnimationChannel {
                                node: node_index,
                                message: "ignored non-hips humanoid translation track".to_owned(),
                            });
                        }
                        Some(gltf::animation::util::ReadOutputs::Rotations(values)) => {
                            let bone_rest = rest_pose
                                .bone_world_rotations
                                .get(bone_name)
                                .copied()
                                .unwrap_or(Quat::IDENTITY);
                            let parent_rest = rest_pose.parent_world_rotation(bone_name);
                            result.humanoid_rotation_tracks.insert(
                                bone_name.clone(),
                                RotationTrack {
                                    times: times.clone(),
                                    values: values
                                        .into_f32()
                                        .map(|[x, y, z, w]| {
                                            parent_rest
                                                * Quat::from_xyzw(x, y, z, w)
                                                * bone_rest.inverse()
                                        })
                                        .collect(),
                                },
                            );
                        }
                        Some(_) => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: format!(
                                    "invalid humanoid animation path for node {node_index}"
                                ),
                            });
                        }
                        None => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: "missing animation output accessor".to_owned(),
                            });
                        }
                    }
                    continue;
                }

                if let Some(expression) = node_map.expressions.get(&node_index) {
                    match reader.read_outputs() {
                        Some(gltf::animation::util::ReadOutputs::Translations(values)) => {
                            let track = ScalarTrack {
                                times: times.clone(),
                                values: values.map(|value| value[0]).collect(),
                            };
                            match expression {
                                VrmaExpressionTarget::Preset(name) => {
                                    result.preset_expression_tracks.insert(name.clone(), track);
                                }
                                VrmaExpressionTarget::Custom(name) => {
                                    result.custom_expression_tracks.insert(name.clone(), track);
                                }
                            }
                        }
                        Some(_) => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: format!(
                                    "invalid expression animation path for node {node_index}"
                                ),
                            });
                        }
                        None => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: "missing animation output accessor".to_owned(),
                            });
                        }
                    }
                    continue;
                }

                if Some(node_index) == node_map.look_at {
                    match reader.read_outputs() {
                        Some(gltf::animation::util::ReadOutputs::Rotations(values)) => {
                            result.look_at_track = Some(RotationTrack {
                                times: times.clone(),
                                values: values
                                    .into_f32()
                                    .map(|[x, y, z, w]| Quat::from_xyzw(x, y, z, w))
                                    .collect(),
                            });
                        }
                        Some(_) => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: format!(
                                    "invalid lookAt animation path for node {node_index}"
                                ),
                            });
                        }
                        None => {
                            return Err(VrmIoError::InvalidAnimationChannel {
                                message: "missing animation output accessor".to_owned(),
                            });
                        }
                    }
                }
            }

            result.duration = result
                .hips_translation
                .as_ref()
                .and_then(|track| track.times.last().copied())
                .into_iter()
                .chain(
                    result
                        .humanoid_rotation_tracks
                        .values()
                        .filter_map(|track| track.times.last().copied()),
                )
                .chain(
                    result
                        .preset_expression_tracks
                        .values()
                        .filter_map(|track| track.times.last().copied()),
                )
                .chain(
                    result
                        .custom_expression_tracks
                        .values()
                        .filter_map(|track| track.times.last().copied()),
                )
                .chain(
                    result
                        .look_at_track
                        .as_ref()
                        .and_then(|track| track.times.last().copied()),
                )
                .fold(0.0, f32::max);

            Ok(result)
        })
        .collect::<Result<Vec<_>, VrmIoError>>()?;

    Ok(Some(animations))
}

#[derive(Clone, Debug, Default)]
struct VrmaNodeMap {
    humanoid: HashMap<usize, HumanBoneName>,
    expressions: HashMap<usize, VrmaExpressionTarget>,
    look_at: Option<usize>,
}

impl VrmaNodeMap {
    fn from_extension(vrma: &vrm_protocol::vrma::VrmcVrmAnimation) -> Self {
        let mut map = Self::default();

        if let Some(humanoid) = &vrma.humanoid {
            for (name, value) in &humanoid.human_bones {
                if let Some(node) = node_from_value(value) {
                    map.humanoid
                        .insert(node, HumanBoneName::from(name.as_str()));
                }
            }
        }

        if let Some(expressions) = &vrma.expressions {
            for (name, value) in expressions.preset.as_ref().into_iter().flatten() {
                if let Some(node) = node_from_value(value) {
                    map.expressions.insert(
                        node,
                        VrmaExpressionTarget::Preset(ExpressionName::from(name.as_str())),
                    );
                }
            }
            for (name, value) in expressions.custom.as_ref().into_iter().flatten() {
                if let Some(node) = node_from_value(value) {
                    map.expressions
                        .insert(node, VrmaExpressionTarget::Custom(name.clone()));
                }
            }
        }

        map.look_at = vrma.look_at.map(|look_at| look_at.node);
        map
    }
}

#[derive(Clone, Debug, Default)]
struct VrmaRestPose {
    bone_world_rotations: HashMap<HumanBoneName, Quat>,
    hips_parent_world_rotation: Quat,
    hips_parent_world_matrix: Mat4,
    hips_world_position: Vec3,
}

impl VrmaRestPose {
    fn from_document(document: &gltf::Document, node_map: &VrmaNodeMap) -> Self {
        let graph = NodeRestGraph::from_document(document);
        let bone_world_rotations = node_map
            .humanoid
            .iter()
            .filter_map(|(node, bone)| {
                graph
                    .world_rotations
                    .get(*node)
                    .copied()
                    .map(|rotation| (bone.clone(), rotation))
            })
            .collect::<HashMap<_, _>>();
        let hips_parent_world_rotation = node_map
            .humanoid
            .iter()
            .find_map(|(node, bone)| {
                (*bone == HumanBoneName::Hips)
                    .then(|| graph.parents.get(*node).and_then(|parent| *parent))
                    .flatten()
            })
            .and_then(|parent| graph.world_rotations.get(parent).copied())
            .unwrap_or(Quat::IDENTITY);
        let hips_node = node_map
            .humanoid
            .iter()
            .find_map(|(node, bone)| (*bone == HumanBoneName::Hips).then_some(*node));
        let hips_parent_world_matrix = hips_node
            .and_then(|node| graph.parents.get(node).and_then(|parent| *parent))
            .and_then(|parent| graph.world_matrices.get(parent).copied())
            .unwrap_or(Mat4::IDENTITY);
        let hips_world_position = hips_node
            .and_then(|node| graph.world_matrices.get(node).copied())
            .map(|matrix| matrix.transform_point3(Vec3::ZERO))
            .unwrap_or(Vec3::ZERO);

        Self {
            bone_world_rotations,
            hips_parent_world_rotation,
            hips_parent_world_matrix,
            hips_world_position,
        }
    }

    fn parent_world_rotation(&self, bone: &HumanBoneName) -> Quat {
        let mut parent = human_bone_parent(bone);
        while let Some(parent_bone) = parent.as_ref() {
            if let Some(rotation) = self.bone_world_rotations.get(parent_bone) {
                return *rotation;
            }
            parent = human_bone_parent(parent_bone);
        }
        self.hips_parent_world_rotation
    }
}

#[derive(Clone, Debug, Default)]
struct NodeRestGraph {
    parents: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    names: Vec<Option<String>>,
    meshes: Vec<Option<usize>>,
    skins: Vec<Option<usize>>,
    weights: Vec<Vec<f32>>,
    local_transforms: Vec<Transform>,
    world_transforms: Vec<Transform>,
    world_rotations: Vec<Quat>,
    world_matrices: Vec<Mat4>,
}

impl NodeRestGraph {
    fn from_document(document: &gltf::Document) -> Self {
        let node_count = document.nodes().count();
        let mut graph = Self {
            parents: vec![None; node_count],
            children: vec![Vec::new(); node_count],
            names: vec![None; node_count],
            meshes: vec![None; node_count],
            skins: vec![None; node_count],
            weights: vec![Vec::new(); node_count],
            local_transforms: vec![Transform::default(); node_count],
            world_transforms: vec![Transform::default(); node_count],
            world_rotations: vec![Quat::IDENTITY; node_count],
            world_matrices: vec![Mat4::IDENTITY; node_count],
        };

        for scene in document.scenes() {
            for node in scene.nodes() {
                graph.visit_node(node, None, Mat4::IDENTITY, Quat::IDENTITY);
            }
        }

        graph
    }

    fn into_scene_rest(self, node_count: usize) -> GltfSceneRest {
        GltfSceneRest {
            nodes: (0..node_count)
                .map(|index| GltfNodeRest {
                    name: self.names[index].clone(),
                    parent: self.parents[index],
                    children: self.children[index].clone(),
                    mesh: self.meshes[index],
                    skin: self.skins[index],
                    weights: self.weights[index].clone(),
                    local: self.local_transforms[index],
                    world: self.world_transforms[index],
                    world_matrix: self.world_matrices[index],
                })
                .collect(),
        }
    }

    fn visit_node(
        &mut self,
        node: gltf::Node<'_>,
        parent: Option<usize>,
        parent_matrix: Mat4,
        parent_rotation: Quat,
    ) {
        let index = node.index();
        let (translation, rotation, scale) = node.transform().decomposed();
        let local_rotation = Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]);
        let local_transform = Transform {
            translation: Vec3::from_array(translation),
            rotation: local_rotation,
            scale: Vec3::from_array(scale),
        };
        let local_matrix = Mat4::from_scale_rotation_translation(
            local_transform.scale,
            local_transform.rotation,
            local_transform.translation,
        );
        let world_matrix = parent_matrix * local_matrix;
        let world_rotation = parent_rotation * local_rotation;
        let (world_scale, world_rotation_decomposed, world_translation) =
            world_matrix.to_scale_rotation_translation();
        self.parents[index] = parent;
        self.names[index] = node.name().map(ToOwned::to_owned);
        self.meshes[index] = node.mesh().map(|mesh| mesh.index());
        self.skins[index] = node.skin().map(|skin| skin.index());
        self.weights[index] = node.weights().unwrap_or_default().to_vec();
        if let Some(parent) = parent {
            self.children[parent].push(index);
        }
        self.local_transforms[index] = local_transform;
        self.world_transforms[index] = Transform {
            translation: world_translation,
            rotation: world_rotation_decomposed,
            scale: world_scale,
        };
        self.world_rotations[index] = world_rotation;
        self.world_matrices[index] = world_matrix;

        for child in node.children() {
            self.visit_node(child, Some(index), world_matrix, world_rotation);
        }
    }
}

fn expand_vrm0_spring_roots(document: &mut vrm_core::VrmDocument, scene: &GltfSceneRest) {
    if document.kind != VrmKind::Vrm0Compat {
        return;
    }
    let Feature::Present(system) = &mut document.spring_bone else {
        return;
    };
    for spring in &mut system.springs {
        spring.joints = spring
            .joints
            .iter()
            .flat_map(|joint| {
                let nodes = scene
                    .node(joint.node.0)
                    .map(|_| scene_descendants_preorder(scene, joint.node))
                    .unwrap_or_else(|| vec![joint.node]);
                nodes.into_iter().map(|node| {
                    let mut joint = joint.clone();
                    joint.node = node;
                    joint
                })
            })
            .collect();
    }
}

fn scene_descendants_preorder(
    scene: &GltfSceneRest,
    root: vrm_core::NodeRef,
) -> Vec<vrm_core::NodeRef> {
    let mut nodes = Vec::new();
    push_scene_descendants_preorder(scene, root, &mut nodes);
    nodes
}

fn push_scene_descendants_preorder(
    scene: &GltfSceneRest,
    node: vrm_core::NodeRef,
    nodes: &mut Vec<vrm_core::NodeRef>,
) {
    nodes.push(node);
    if let Some(rest) = scene.node(node.0) {
        for child in &rest.children {
            push_scene_descendants_preorder(scene, vrm_core::NodeRef(*child), nodes);
        }
    }
}

fn human_bone_parent(bone: &HumanBoneName) -> Option<HumanBoneName> {
    use HumanBoneName::*;
    match bone {
        Hips => None,
        Spine => Some(Hips),
        Chest => Some(Spine),
        UpperChest => Some(Chest),
        Neck => Some(UpperChest),
        Head => Some(Neck),
        LeftEye | RightEye | Jaw => Some(Head),
        LeftUpperLeg => Some(Hips),
        LeftLowerLeg => Some(LeftUpperLeg),
        LeftFoot => Some(LeftLowerLeg),
        LeftToes => Some(LeftFoot),
        RightUpperLeg => Some(Hips),
        RightLowerLeg => Some(RightUpperLeg),
        RightFoot => Some(RightLowerLeg),
        RightToes => Some(RightFoot),
        LeftShoulder => Some(UpperChest),
        LeftUpperArm => Some(LeftShoulder),
        LeftLowerArm => Some(LeftUpperArm),
        LeftHand => Some(LeftLowerArm),
        RightShoulder => Some(UpperChest),
        RightUpperArm => Some(RightShoulder),
        RightLowerArm => Some(RightUpperArm),
        RightHand => Some(RightLowerArm),
        LeftThumbMetacarpal => Some(LeftHand),
        LeftThumbProximal => Some(LeftThumbMetacarpal),
        LeftThumbDistal => Some(LeftThumbProximal),
        LeftIndexProximal => Some(LeftHand),
        LeftIndexIntermediate => Some(LeftIndexProximal),
        LeftIndexDistal => Some(LeftIndexIntermediate),
        LeftMiddleProximal => Some(LeftHand),
        LeftMiddleIntermediate => Some(LeftMiddleProximal),
        LeftMiddleDistal => Some(LeftMiddleIntermediate),
        LeftRingProximal => Some(LeftHand),
        LeftRingIntermediate => Some(LeftRingProximal),
        LeftRingDistal => Some(LeftRingIntermediate),
        LeftLittleProximal => Some(LeftHand),
        LeftLittleIntermediate => Some(LeftLittleProximal),
        LeftLittleDistal => Some(LeftLittleIntermediate),
        RightThumbMetacarpal => Some(RightHand),
        RightThumbProximal => Some(RightThumbMetacarpal),
        RightThumbDistal => Some(RightThumbProximal),
        RightIndexProximal => Some(RightHand),
        RightIndexIntermediate => Some(RightIndexProximal),
        RightIndexDistal => Some(RightIndexIntermediate),
        RightMiddleProximal => Some(RightHand),
        RightMiddleIntermediate => Some(RightMiddleProximal),
        RightMiddleDistal => Some(RightMiddleIntermediate),
        RightRingProximal => Some(RightHand),
        RightRingIntermediate => Some(RightRingProximal),
        RightRingDistal => Some(RightRingIntermediate),
        RightLittleProximal => Some(RightHand),
        RightLittleIntermediate => Some(RightLittleProximal),
        RightLittleDistal => Some(RightLittleIntermediate),
        Custom(_) => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum VrmaExpressionTarget {
    Preset(ExpressionName),
    Custom(String),
}

fn node_from_value(value: &Value) -> Option<usize> {
    value
        .get("node")
        .and_then(Value::as_u64)
        .and_then(|node| usize::try_from(node).ok())
}

pub fn load_vrm_from_path(path: impl AsRef<Path>) -> Result<LoadedVrm, VrmIoError> {
    let bytes = std::fs::read(path)?;
    load_vrm_from_slice(&bytes)
}

pub fn load_vrm_from_path_with_policy(
    path: impl AsRef<Path>,
    diagnostic_policy: DiagnosticPolicy,
) -> Result<LoadedVrmWithDiagnostics, VrmIoError> {
    let bytes = std::fs::read(path)?;
    load_vrm_from_slice_with_policy(&bytes, diagnostic_policy)
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

fn extract_hdr_emissive_multipliers(
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
        if let Some(value) = extensions.get("VRMC_materials_hdr_emissiveMultiplier") {
            let multiplier = serde_json::from_value(value.clone()).map_err(|err| {
                VrmIoError::InvalidExtension {
                    extension: "VRMC_materials_hdr_emissiveMultiplier".to_owned(),
                    message: err.to_string(),
                }
            })?;
            bundle
                .hdr_emissive_multipliers
                .insert(material_index, multiplier);
        }
    }
    Ok(())
}

fn extract_khr_emissive_strengths(
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
        if let Some(value) = extensions.get("KHR_materials_emissive_strength") {
            let strength = serde_json::from_value(value.clone()).map_err(|err| {
                VrmIoError::InvalidExtension {
                    extension: "KHR_materials_emissive_strength".to_owned(),
                    message: err.to_string(),
                }
            })?;
            bundle
                .khr_emissive_strengths
                .insert(material_index, strength);
        }
    }
    Ok(())
}

fn validate_vrmc_extension_versions(bundle: &ExtensionBundle) -> Result<(), VrmIoError> {
    if let Some(spring_bone) = &bundle.spring_bone {
        ensure_vrmc_spec_version("VRMC_springBone", &spring_bone.spec_version)?;
    }
    for constraint in &bundle.node_constraints {
        ensure_vrmc_spec_version("VRMC_node_constraint", &constraint.constraint.spec_version)?;
    }
    for mtoon in bundle.mtoon_materials.values() {
        ensure_vrmc_spec_version("VRMC_materials_mtoon", &mtoon.spec_version)?;
    }
    Ok(())
}

fn ensure_vrmc_spec_version(extension: &'static str, spec_version: &str) -> Result<(), VrmIoError> {
    if matches!(spec_version, "1.0" | "1.0-beta") {
        Ok(())
    } else {
        Err(VrmIoError::UnsupportedExtensionSpecVersion {
            extension: extension.to_owned(),
            spec_version: spec_version.to_owned(),
        })
    }
}

#[derive(Debug, Error)]
pub enum VrmIoError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Gltf(#[from] gltf::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error("invalid extension {extension}: {message}")]
    InvalidExtension { extension: String, message: String },
    #[error("unsupported {extension} specVersion: {spec_version}")]
    UnsupportedExtensionSpecVersion {
        extension: String,
        spec_version: String,
    },
    #[error("invalid animation channel: {message}")]
    InvalidAnimationChannel { message: String },
    #[error("render expression was requested, but the VRM has no expressions")]
    MissingExpressions,
    #[error("unknown render expression: {name}")]
    UnknownExpression { name: String },
    #[error("could not preserve source data: {message}")]
    SourcePreservation { message: String },
    #[error("could not write source data: {message}")]
    SourceWrite { message: String },
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
    use std::{env, fs, path::PathBuf};
    use vrm_core::{
        ExpressionName, Feature, FirstPersonAnnotation, HumanBoneName, LookAtKind,
        MtoonRenderQueue, OutlineWidthMode, VrmKind,
    };

    fn assert_f32_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 0.00001,
            "actual {actual} did not match expected {expected}"
        );
    }

    fn assert_vec3_close(actual: [f32; 3], expected: [f32; 3]) {
        actual
            .into_iter()
            .zip(expected)
            .for_each(|(actual, expected)| assert_f32_close(actual, expected));
    }

    fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
        actual
            .into_iter()
            .zip(expected)
            .for_each(|(actual, expected)| assert_f32_close(actual, expected));
    }

    fn assert_vec2_close(actual: [f32; 2], expected: [f32; 2]) {
        actual
            .into_iter()
            .zip(expected)
            .for_each(|(actual, expected)| assert_f32_close(actual, expected));
    }

    fn gamma_eotf(value: f32) -> f32 {
        value.powf(2.2)
    }

    #[test]
    fn generated_rgba_mip_chain_is_renderer_neutral() {
        let rgba = (0..32).collect::<Vec<u8>>();
        let levels = generate_rgba_mip_chain(4, 2, &rgba).unwrap();

        assert_eq!(levels.len(), 3);
        assert_eq!((levels[0].width, levels[0].height), (4, 2));
        assert_eq!(levels[0].rgba, rgba);
        assert_eq!((levels[1].width, levels[1].height), (2, 1));
        assert_eq!(levels[1].rgba, vec![10, 11, 12, 13, 18, 19, 20, 21]);
        assert_eq!((levels[2].width, levels[2].height), (1, 1));
        assert_eq!(levels[2].rgba, vec![14, 15, 16, 17]);
    }

    #[test]
    fn generated_rgba_mip_chain_averages_odd_sized_box_regions() {
        let rgba = vec![0, 10, 20, 30, 30, 40, 50, 60, 60, 70, 80, 90];
        let levels = generate_rgba_mip_chain(3, 1, &rgba).unwrap();

        assert_eq!(levels.len(), 2);
        assert_eq!((levels[1].width, levels[1].height), (1, 1));
        assert_eq!(levels[1].rgba, vec![30, 40, 50, 60]);
    }

    #[test]
    fn generated_rgba_mip_chain_partitions_odd_sized_regions_without_overlap() {
        let rgba = [
            0u8, 10, 20, 30, 10, 20, 30, 40, 20, 30, 40, 50, 30, 40, 50, 60, 40, 50, 60, 70,
        ];
        let levels = generate_rgba_mip_chain(5, 1, &rgba).unwrap();

        assert_eq!((levels[1].width, levels[1].height), (2, 1));
        assert_eq!(levels[1].rgba, vec![5, 15, 25, 35, 30, 40, 50, 60]);
    }

    #[test]
    fn generated_rgba_mip_chain_rejects_invalid_input() {
        assert_eq!(
            generate_rgba_mip_chain(0, 1, &[]),
            Err(TextureMipError::InvalidDimensions)
        );
        assert_eq!(
            generate_rgba_mip_chain(2, 2, &[255, 0, 0, 255]),
            Err(TextureMipError::InvalidRgbaLength {
                expected: 16,
                actual: 4,
            })
        );
    }

    #[test]
    fn image_bytes_to_rgba8_converts_supported_formats() {
        assert_eq!(
            image_bytes_to_rgba8(2, 1, ImageFormat::R8, &[8, 16]).unwrap(),
            vec![8, 8, 8, 255, 16, 16, 16, 255]
        );
        assert_eq!(
            image_bytes_to_rgba8(2, 1, ImageFormat::R8G8, &[8, 80, 16, 160]).unwrap(),
            vec![8, 8, 8, 80, 16, 16, 16, 160]
        );
        assert_eq!(
            image_bytes_to_rgba8(2, 1, ImageFormat::R8G8B8, &[1, 2, 3, 4, 5, 6]).unwrap(),
            vec![1, 2, 3, 255, 4, 5, 6, 255]
        );
        assert_eq!(
            image_bytes_to_rgba8(2, 1, ImageFormat::R8G8B8A8, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn image_bytes_to_rgba8_rejects_invalid_lengths_and_unsupported_formats() {
        assert_eq!(
            image_bytes_to_rgba8(0, 1, ImageFormat::R8, &[]),
            Err(Rgba8ImageError::InvalidDimensions)
        );
        assert_eq!(
            image_bytes_to_rgba8(2, 1, ImageFormat::R8G8B8, &[1, 2, 3]),
            Err(Rgba8ImageError::InvalidByteLength {
                format: ImageFormat::R8G8B8,
                expected: 6,
                actual: 3,
            })
        );
        assert_eq!(
            image_bytes_to_rgba8(1, 1, ImageFormat::R16, &[0, 0]),
            Err(Rgba8ImageError::UnsupportedFormat(ImageFormat::R16))
        );
    }

    #[test]
    fn cpu_rgba8_image_samples_repeat_linear_with_explicit_origin() {
        let image = CpuRgba8Image::from_rgba8(
            2,
            2,
            vec![0, 10, 0, 255, 0, 20, 0, 255, 0, 30, 0, 255, 0, 40, 0, 255],
        )
        .unwrap();

        assert_f32_close(
            image.sample_green_repeat_linear([0.25, 0.25], Rgba8SamplingOrigin::TopLeft),
            10.0 / 255.0,
        );
        assert_f32_close(
            image.sample_green_repeat_linear([0.25, 0.25], Rgba8SamplingOrigin::BottomLeft),
            30.0 / 255.0,
        );
        assert_f32_close(
            image.sample_green_repeat_linear([1.25, 1.25], Rgba8SamplingOrigin::TopLeft),
            10.0 / 255.0,
        );
        assert_f32_close(
            image.sample_channel_repeat_linear([0.25, 0.25], 7, 128, Rgba8SamplingOrigin::TopLeft),
            128.0 / 255.0,
        );
        assert_eq!(
            image.sample_rgba8_repeat_linear([0.25, 0.25], Rgba8SamplingOrigin::TopLeft),
            [0, 10, 0, 255]
        );
        assert_eq!(
            image.sample_rgba8_repeat_linear([0.25, 0.25], Rgba8SamplingOrigin::BottomLeft),
            [0, 30, 0, 255]
        );
    }

    #[test]
    fn cpu_rgba8_image_samples_all_channels_repeat_linear() {
        let image = CpuRgba8Image::from_rgba8(
            2,
            2,
            vec![
                0, 0, 0, 0, 100, 0, 0, 100, 0, 100, 0, 200, 100, 100, 100, 255,
            ],
        )
        .unwrap();

        assert_eq!(
            image.sample_rgba8_repeat_linear([0.25, 0.25], Rgba8SamplingOrigin::TopLeft),
            [0, 0, 0, 0]
        );
        assert_eq!(
            image.sample_rgba8_repeat_linear([0.75, 0.75], Rgba8SamplingOrigin::TopLeft),
            [100, 100, 100, 255]
        );
        assert_eq!(
            image.sample_rgba8_repeat_linear([0.5, 0.5], Rgba8SamplingOrigin::TopLeft),
            [50, 50, 25, 139]
        );
    }

    #[test]
    fn cpu_rgba8_image_validates_rgba_length() {
        assert_eq!(
            CpuRgba8Image::from_rgba8(2, 2, vec![255; 4]),
            Err(Rgba8ImageError::InvalidByteLength {
                format: ImageFormat::R8G8B8A8,
                expected: 16,
                actual: 4,
            })
        );
    }

    #[test]
    fn transform_tex_coord_0_applies_scale_rotation_and_offset() {
        let actual = transform_tex_coord_0(
            [0.25, 0.5],
            Some(TextureTransform2d {
                offset: [0.1, -0.2],
                scale: [2.0, 0.5],
                rotation: std::f32::consts::FRAC_PI_2,
                tex_coord: Some(0),
            }),
        );

        assert_vec2_close(actual, [-0.15, 0.3]);
    }

    #[test]
    fn transform_tex_coord_0_ignores_none_and_nonzero_tex_coord_sets() {
        assert_eq!(transform_tex_coord_0([0.25, 0.5], None), [0.25, 0.5]);
        assert_eq!(
            transform_tex_coord_0(
                [0.25, 0.5],
                Some(TextureTransform2d {
                    offset: [1.0, 2.0],
                    scale: [3.0, 4.0],
                    rotation: 1.0,
                    tex_coord: Some(1),
                }),
            ),
            [0.25, 0.5]
        );
    }

    #[test]
    fn loads_generated_vrm1_gltf_without_repo_fixture_asset() {
        let bytes = generated_vrm1_gltf().to_string().into_bytes();
        let loaded = load_vrm_from_slice(&bytes).unwrap();
        let document = loaded.model().document();

        assert!(loaded.buffers.is_empty());
        assert!(loaded.images.is_empty());
        assert!(loaded.scene().node(0).is_some());
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
        assert!(matches!(
            document
                .materials
                .first()
                .map(|material| material.hdr_emissive_multiplier.as_ref()),
            Some(Some(multiplier)) if multiplier.emissive_intensity() == 2.5
        ));
        let (emissive_strength, _) = document.materials[0].effective_emissive_strength();
        assert_eq!(emissive_strength.0, 5.0);
        assert_eq!(document.node_constraints.len(), 1);
        assert!(document.spring_bone.is_present());

        let effects = loaded
            .expression_render_effects([("blink", 0.5)])
            .expect("generated expression should resolve render effects");
        let mesh = GltfMeshData {
            name: None,
            weights: vec![0.2],
            primitives: Vec::new(),
        };
        assert_eq!(
            effects.active_morph_weights(2, &loaded.scene.nodes[2], &mesh),
            vec![50.0]
        );
        assert_vec4_close(
            effects.apply_color4([0.25, 0.5, 0.75, 1.0], Some(0), "color"),
            [0.575, 0.65, 0.725, 0.8],
        );
        assert_vec3_close(
            effects.apply_color3([0.0, 0.0, 0.0], Some(0), "emissionColor"),
            [0.25, 0.2, 0.15],
        );
        let expression_shading = loaded.expression_material_shading_plan(
            Some(0),
            GltfMaterialShadingOptions {
                v0_compat_shade: false,
            },
            &effects,
        );
        assert_vec4_close(expression_shading.base_color, [0.95, 0.9, 0.85, 0.8]);
        assert_vec3_close(expression_shading.emissive, [0.25, 0.2, 0.15]);
        let outline = loaded
            .expression_mtoon_outline_plan(Some(0), &effects)
            .unwrap();
        let mtoon = document.materials[0].mtoon.as_ref().unwrap();
        assert_f32_close(outline.width_factor, mtoon.outline_width_factor);
        assert_eq!(outline.width_mode, mtoon.outline_width_mode);
        assert_vec4_close(
            outline.color,
            [
                mtoon.outline_color_factor[0],
                mtoon.outline_color_factor[1],
                mtoon.outline_color_factor[2],
                mtoon.outline_lighting_mix_factor,
            ],
        );
        let transforms = effects.apply_uv_transforms(
            GltfMaterialUvTransforms {
                base: Some(TextureTransform2d {
                    offset: [0.25, 0.5],
                    scale: [1.0, 1.0],
                    rotation: 0.25,
                    tex_coord: Some(0),
                }),
                ..Default::default()
            },
            Some(0),
        );
        assert_eq!(
            transforms.base,
            Some(TextureTransform2d {
                offset: [0.5, 0.375],
                scale: [0.625, 0.75],
                rotation: 0.25,
                tex_coord: Some(0),
            })
        );
        let expression_transforms =
            loaded.expression_material_uv_transforms(Some(0), 0.0, &effects);
        assert_eq!(
            expression_transforms.base,
            Some(TextureTransform2d {
                offset: [0.375, 0.125],
                scale: [0.625, 0.75],
                rotation: 0.0,
                tex_coord: None,
            })
        );
        assert!(matches!(
            loaded.expression_render_effects([("missing", 1.0)]),
            Err(VrmIoError::UnknownExpression { name }) if name == "missing"
        ));
    }

    #[test]
    fn generated_gltf_preserves_source_json_extensions_and_extras() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VENDOR_lossless"] = json!({
            "nested": { "kept": true },
            "array": [1, 2, 3]
        });
        sample["extras"] = json!({
            "authoringTool": "unit-test",
            "note": { "preserve": "root extras" }
        });
        let bytes = sample.to_string().into_bytes();

        let loaded = load_vrm_from_slice(&bytes).unwrap();
        let source = loaded.source();

        assert_eq!(source.format, GltfSourceFormat::Json);
        assert_eq!(source.original_bytes, bytes);
        assert_eq!(source.json_bytes, source.original_bytes);
        assert_eq!(
            source.root_extension("VENDOR_lossless").unwrap()["nested"]["kept"],
            true
        );
        assert_eq!(source.root_extras().unwrap()["authoringTool"], "unit-test");
        assert!(source.glb_chunks.is_empty());
    }

    #[test]
    fn generated_glb_preserves_json_and_bin_chunks() {
        let mut sample = generated_vrm1_gltf();
        sample["buffers"] = json!([{ "byteLength": 4 }]);
        sample["extensions"]["VENDOR_glb"] = json!({ "raw": "kept" });
        sample["extras"] = json!({ "glb": true });
        let glb = generated_glb(sample, &[9, 8, 7, 6]);

        let loaded = load_vrm_from_slice(&glb).unwrap();
        let source = loaded.source();

        assert_eq!(
            source.format,
            GltfSourceFormat::Glb {
                version: 2,
                declared_length: glb.len() as u32
            }
        );
        assert_eq!(source.original_bytes, glb);
        assert_eq!(source.root_extension("VENDOR_glb").unwrap()["raw"], "kept");
        assert_eq!(source.root_extras().unwrap()["glb"], true);
        assert_eq!(source.glb_chunks.len(), 2);
        assert_eq!(source.glb_json_chunk().unwrap().kind, GlbChunkKind::Json);
        assert_eq!(
            source.glb_json_chunk().unwrap().raw_type,
            GLB_JSON_CHUNK_TYPE
        );
        assert_eq!(source.glb_bin_chunk().unwrap().bytes, vec![9, 8, 7, 6]);
        assert_eq!(loaded.buffers, vec![vec![9, 8, 7, 6]]);
    }

    #[test]
    fn metadata_patch_rewrites_gltf_and_atomic_save_preserves_unknown_data() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VENDOR_editor"] = json!({ "kept": true });
        sample["extensions"]["VRMC_vrm"]["meta"]["licenseUrl"] = json!("https://old.example");
        sample["extras"] = json!({ "root": "kept" });
        let loaded = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap();

        let edited = loaded
            .source()
            .edited_vrm_metadata(
                &VrmMetadataPatch::new()
                    .with_name("Edited Avatar")
                    .with_authors(["alice", "bob"])
                    .with_version("2.0")
                    .clear_license_url(),
            )
            .unwrap();
        assert_eq!(
            edited["extensions"]["VRMC_vrm"]["meta"]["name"],
            "Edited Avatar"
        );
        assert_eq!(
            edited["extensions"]["VRMC_vrm"]["meta"]["authors"],
            json!(["alice", "bob"])
        );
        assert!(edited["extensions"]["VRMC_vrm"]["meta"]["licenseUrl"].is_null());
        assert_eq!(edited["extensions"]["VENDOR_editor"]["kept"], true);
        assert_eq!(edited["extras"]["root"], "kept");

        let bytes = loaded.source().to_bytes_with_json(&edited).unwrap();
        let reloaded = load_vrm_from_slice(&bytes).unwrap();
        assert_eq!(reloaded.model().document().meta.name, "Edited Avatar");
        assert_eq!(
            reloaded.model().document().meta.authors,
            vec!["alice".to_owned(), "bob".to_owned()]
        );

        let path = temp_output_path("edited-source", "gltf");
        loaded
            .source()
            .save_with_json_atomic(&path, &edited)
            .unwrap();
        let saved = load_vrm_from_path(&path).unwrap();
        assert_eq!(saved.model().document().meta.name, "Edited Avatar");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn writer_options_pretty_json_round_trip_normalized_document() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VENDOR_lossless"] = json!({ "kept": true });
        let loaded = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap();

        let bytes = loaded
            .source()
            .to_bytes_with_json_options(
                &loaded.source().json,
                GltfWriteOptions {
                    json_format: GltfJsonFormat::Pretty,
                },
            )
            .unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            text.contains("\n  "),
            "pretty writer should include indentation"
        );
        let reloaded = load_vrm_from_slice(&bytes).unwrap();

        assert_eq!(
            reloaded.model().document().meta,
            loaded.model().document().meta
        );
        assert_eq!(
            reloaded.source().root_extension("VENDOR_lossless").unwrap()["kept"],
            true
        );
    }

    #[test]
    fn metadata_patch_rewrites_glb_json_while_preserving_bin_chunk() {
        let mut sample = generated_vrm1_gltf();
        sample["buffers"] = json!([{ "byteLength": 4 }]);
        let loaded = load_vrm_from_slice(&generated_glb(sample, &[1, 3, 5, 7])).unwrap();

        let edited = loaded
            .source()
            .edited_vrm_metadata(&VrmMetadataPatch::new().with_name("GLB Edited"))
            .unwrap();
        let rewritten = loaded.source().to_bytes_with_json(&edited).unwrap();
        let reloaded = load_vrm_from_slice(&rewritten).unwrap();

        assert!(matches!(
            reloaded.source().format,
            GltfSourceFormat::Glb { .. }
        ));
        assert_eq!(reloaded.model().document().meta.name, "GLB Edited");
        assert_eq!(
            reloaded.source().glb_bin_chunk().unwrap().bytes,
            vec![1, 3, 5, 7]
        );
        assert_eq!(reloaded.buffers, vec![vec![1, 3, 5, 7]]);
    }

    #[test]
    fn metadata_patch_options_preserve_unknown_glb_chunks_and_padding() {
        const VENDOR_CHUNK: u32 = 0x5858_5858;
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VENDOR_glb"] = json!({ "raw": "kept" });
        let source = GltfSource::from_slice(&generated_glb_with_chunks(
            sample,
            &[
                (GLB_BIN_CHUNK_TYPE, vec![1, 2, 3]),
                (VENDOR_CHUNK, vec![9, 8]),
            ],
        ))
        .unwrap();

        let rewritten = source
            .to_bytes_with_metadata_patch_options(
                &VrmMetadataPatch::new().with_name("Pretty GLB"),
                GltfWriteOptions {
                    json_format: GltfJsonFormat::Pretty,
                },
            )
            .unwrap();
        let rewritten_source = GltfSource::from_slice(&rewritten).unwrap();

        assert_eq!(
            rewritten_source.format,
            GltfSourceFormat::Glb {
                version: 2,
                declared_length: rewritten.len() as u32
            }
        );
        assert_eq!(
            rewritten_source
                .glb_chunks
                .iter()
                .find(|chunk| chunk.raw_type == VENDOR_CHUNK)
                .unwrap()
                .bytes,
            vec![9, 8, 0, 0]
        );
        assert_eq!(rewritten.len() % 4, 0);
        assert_eq!(
            rewritten_source.json["extensions"]["VRMC_vrm"]["meta"]["name"],
            "Pretty GLB"
        );
        assert!(
            std::str::from_utf8(&rewritten_source.json_bytes)
                .unwrap()
                .contains("\n  ")
        );
    }

    #[test]
    fn metadata_patch_atomic_helper_preserves_original_on_error() {
        let source = GltfSource::from_slice(br#"{ "asset": { "version": "2.0" } }"#).unwrap();
        let path = temp_output_path("metadata-patch-atomic", "gltf");
        fs::write(&path, b"original").unwrap();

        let err = source
            .save_with_metadata_patch_atomic(
                &path,
                &VrmMetadataPatch::new().with_name("Atomic Edited"),
            )
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("missing glTF root extensions object")
        );
        assert_eq!(fs::read(&path).unwrap(), b"original");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn glb_source_validation_rejects_bad_header_length_and_chunk_alignment() {
        let sample = generated_vrm1_gltf();
        let mut bad_declared_len = generated_glb(sample.clone(), &[]);
        let wrong_len = (bad_declared_len.len() as u32 + 4).to_le_bytes();
        bad_declared_len[8..12].copy_from_slice(&wrong_len);
        let err = GltfSource::from_slice(&bad_declared_len).unwrap_err();
        assert!(err.to_string().contains("does not match input length"));

        let mut unaligned_chunk = Vec::new();
        unaligned_chunk.extend_from_slice(GLB_MAGIC);
        unaligned_chunk.extend_from_slice(&2u32.to_le_bytes());
        unaligned_chunk.extend_from_slice(&23u32.to_le_bytes());
        unaligned_chunk.extend_from_slice(&3u32.to_le_bytes());
        unaligned_chunk.extend_from_slice(&GLB_JSON_CHUNK_TYPE.to_le_bytes());
        unaligned_chunk.extend_from_slice(b"{} ");
        let err = GltfSource::from_slice(&unaligned_chunk).unwrap_err();
        assert!(err.to_string().contains("not 4-byte aligned"));
    }

    #[test]
    fn metadata_patch_supports_vrm0_legacy_meta_shape() {
        let source = GltfSource::from_slice(
            json!({
                "asset": { "version": "2.0" },
                "extensions": {
                    "VRM": {
                        "exporterVersion": "UniVRM-test",
                        "specVersion": "0.0",
                        "meta": {
                            "title": "Old",
                            "author": "old author",
                            "otherLicenseUrl": "https://old.example"
                        }
                    }
                }
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();

        let edited = source
            .edited_vrm_metadata(
                &VrmMetadataPatch::new()
                    .with_name("Legacy Edited")
                    .with_authors(["alice", "bob"])
                    .clear_license_url(),
            )
            .unwrap();

        let meta = &edited["extensions"]["VRM"]["meta"];
        assert_eq!(meta["title"], "Legacy Edited");
        assert_eq!(meta["author"], "alice, bob");
        assert!(meta["otherLicenseUrl"].is_null());
    }

    #[test]
    fn generated_binary_expression_render_effects_use_vrm_threshold() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VRMC_vrm"]["expressions"]["preset"]["blink"]["isBinary"] =
            json!(true);
        let bytes = sample.to_string().into_bytes();
        let loaded = load_vrm_from_slice(&bytes).unwrap();
        let mesh = GltfMeshData {
            name: None,
            weights: vec![0.2],
            primitives: Vec::new(),
        };

        let at_boundary = loaded
            .expression_render_effects([("blink", 0.5)])
            .expect("binary expression should resolve");
        assert_eq!(
            at_boundary.active_morph_weights(2, &loaded.scene.nodes[2], &mesh),
            vec![0.0]
        );

        let above_boundary = loaded
            .expression_render_effects([("blink", 0.5001)])
            .expect("binary expression should resolve");
        assert_eq!(
            above_boundary.active_morph_weights(2, &loaded.scene.nodes[2], &mesh),
            vec![100.0]
        );

        let non_finite = loaded
            .expression_render_effects([("blink", f32::NAN)])
            .expect("binary expression should resolve");
        assert_eq!(
            non_finite.active_morph_weights(2, &loaded.scene.nodes[2], &mesh),
            vec![0.0]
        );
    }

    #[test]
    fn generated_sample_reports_invalid_expression_shape() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VRMC_vrm"]["expressions"]["preset"]["blink"] = json!(false);
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("invalid preset expression"));
        assert!(message.contains("blink"));
    }

    #[test]
    fn lenient_load_reports_invalid_expression_diagnostic_and_keeps_loading() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VRMC_vrm"]["expressions"]["preset"]["blink"] = json!(false);
        let bytes = sample.to_string().into_bytes();

        let loaded = load_vrm_from_slice_with_policy(&bytes, DiagnosticPolicy::Lenient).unwrap();

        assert!(loaded.diagnostics.has_errors());
        let diagnostic = loaded
            .diagnostics
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code == "vrm.expression.invalid_shape")
            .unwrap();
        assert_eq!(
            diagnostic.path.as_str(),
            "$.extensions.VRMC_vrm.expressions.preset.blink"
        );
        assert_eq!(
            loaded.loaded.model().document().meta.name,
            "Generated Test Avatar"
        );
        let expressions = loaded
            .loaded
            .model()
            .document()
            .expressions
            .as_ref()
            .unwrap();
        assert!(!expressions.preset.contains_key(&ExpressionName::Blink));
    }

    #[test]
    fn load_with_policy_reports_unknown_extension_without_losing_source() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VENDOR_lossless"] = json!({ "nested": { "kept": true } });
        let bytes = sample.to_string().into_bytes();

        let loaded = load_vrm_from_slice_with_policy(&bytes, DiagnosticPolicy::Strict).unwrap();

        assert_eq!(
            loaded
                .loaded
                .source()
                .root_extension("VENDOR_lossless")
                .unwrap()["nested"]["kept"],
            true
        );
        let diagnostic = loaded
            .diagnostics
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code == "vrm.extension.unknown")
            .unwrap();
        assert_eq!(diagnostic.path.as_str(), "$.extensions.VENDOR_lossless");
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn load_with_policy_mirrors_io_warnings_as_structured_diagnostics() {
        let mut sample = generated_vrma_gltf();
        sample["extensions"]["VRMC_vrm_animation"]
            .as_object_mut()
            .unwrap()
            .remove("specVersion");

        let loaded = load_vrm_from_slice_with_policy(
            sample.to_string().as_bytes(),
            DiagnosticPolicy::Strict,
        )
        .unwrap();

        assert!(matches!(
            loaded.loaded.warnings(),
            [VrmIoWarning::MissingSpecVersion { extension, assumed }]
                if extension == "VRMC_vrm_animation" && assumed == "1.0"
        ));
        let diagnostic = loaded
            .diagnostics
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code == "vrm.extension.missing_spec_version")
            .unwrap();
        assert_eq!(
            diagnostic.path.as_str(),
            "$.extensions.VRMC_vrm_animation.specVersion"
        );
    }

    #[test]
    fn load_from_slice_reports_invalid_gltf_payload() {
        let err = load_vrm_from_slice(b"not a gltf document").unwrap_err();

        assert!(matches!(err, VrmIoError::Gltf(_)));
    }

    #[test]
    fn load_from_slice_reports_invalid_glb_payload() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"glTF");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&12u32.to_le_bytes());

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(err, VrmIoError::Gltf(_)));
    }

    #[test]
    fn load_from_path_reports_filesystem_errors() {
        let missing = env::temp_dir().join("vrm-rs-missing-fixture.vrm");
        let err = load_vrm_from_path(&missing).unwrap_err();

        assert!(matches!(err, VrmIoError::Io(_)));
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

    #[test]
    fn generated_sample_reports_invalid_node_constraint_extension() {
        let mut sample = generated_vrm1_gltf();
        sample["nodes"][14]["extensions"]["VRMC_node_constraint"]["constraint"]["rotation"]["source"] =
            json!("bad");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidExtension { extension, .. }
                if extension == "VRMC_node_constraint"
        ));
    }

    #[test]
    fn generated_sample_reports_invalid_mtoon_extension() {
        let mut sample = generated_vrm1_gltf();
        sample["materials"][0]["extensions"]["VRMC_materials_mtoon"]["specVersion"] = json!(1);
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidExtension { extension, .. }
                if extension == "VRMC_materials_mtoon"
        ));
    }

    #[test]
    fn generated_sample_rejects_unsupported_spring_bone_spec_version() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VRMC_springBone"]["specVersion"] = json!("2.0");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::UnsupportedExtensionSpecVersion {
                extension,
                spec_version
            } if extension == "VRMC_springBone" && spec_version == "2.0"
        ));
    }

    #[test]
    fn generated_sample_rejects_unsupported_node_constraint_spec_version() {
        let mut sample = generated_vrm1_gltf();
        sample["nodes"][14]["extensions"]["VRMC_node_constraint"]["specVersion"] = json!("2.0");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::UnsupportedExtensionSpecVersion {
                extension,
                spec_version
            } if extension == "VRMC_node_constraint" && spec_version == "2.0"
        ));
    }

    #[test]
    fn generated_sample_rejects_unsupported_mtoon_spec_version() {
        let mut sample = generated_vrm1_gltf();
        sample["materials"][0]["extensions"]["VRMC_materials_mtoon"]["specVersion"] = json!("2.0");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::UnsupportedExtensionSpecVersion {
                extension,
                spec_version
            } if extension == "VRMC_materials_mtoon" && spec_version == "2.0"
        ));
    }

    #[test]
    fn generated_sample_accepts_beta_secondary_extension_spec_versions() {
        let mut sample = generated_vrm1_gltf();
        sample["extensions"]["VRMC_springBone"]["specVersion"] = json!("1.0-beta");
        sample["nodes"][14]["extensions"]["VRMC_node_constraint"]["specVersion"] =
            json!("1.0-beta");
        sample["materials"][0]["extensions"]["VRMC_materials_mtoon"]["specVersion"] =
            json!("1.0-beta");
        let bytes = sample.to_string().into_bytes();

        let loaded = load_vrm_from_slice(&bytes).unwrap();

        assert!(loaded.model().document().spring_bone.is_present());
        assert_eq!(loaded.model().document().node_constraints.len(), 1);
        assert!(loaded.model().document().materials[0].mtoon.is_present());
    }

    #[test]
    fn generated_sample_extracts_embedded_png_images() {
        let mut sample = generated_vrm1_gltf();
        sample["buffers"] = json!([{
            "uri": "data:application/octet-stream;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGNgAAIAAAUAAXpeqz8AAAAASUVORK5CYII=",
            "byteLength": 68
        }]);
        sample["bufferViews"] = json!([{ "buffer": 0, "byteOffset": 0, "byteLength": 68 }]);
        sample["images"] = json!([{
            "mimeType": "image/png",
            "bufferView": 0
        }]);
        sample["samplers"] = json!([{
            "magFilter": 9728,
            "minFilter": 9729,
            "wrapS": 33071,
            "wrapT": 33648
        }, {
            "magFilter": 9729,
            "minFilter": 9985,
            "wrapS": 10497,
            "wrapT": 33071
        }]);
        sample["textures"] = json!([
            { "source": 0, "sampler": 0 },
            { "source": 0, "sampler": 1 }
        ]);
        sample["materials"][0]["pbrMetallicRoughness"] = json!({
            "baseColorTexture": {
                "index": 0,
                "extensions": {
                    "KHR_texture_transform": {
                        "offset": [0.25, 0.5],
                        "scale": [2.0, 3.0],
                        "rotation": 0.125,
                        "texCoord": 1
                    }
                }
            },
            "baseColorFactor": [0.25, 0.5, 0.75, 1.0],
            "metallicFactor": 0.75,
            "roughnessFactor": 0.25
        });
        sample["materials"][0]["normalTexture"] = json!({
            "index": 1,
            "scale": 0.25,
            "extensions": {
                "KHR_texture_transform": { "offset": [0.1, 0.2], "scale": [0.5, 0.75] }
            }
        });
        sample["materials"][0]["extensions"]["VRMC_materials_mtoon"]["outlineWidthMultiplyTexture"] =
            json!({ "index": 0 });
        sample["materials"][0]["occlusionTexture"] = json!({
            "index": 1,
            "strength": 0.5,
            "extensions": {
                "KHR_texture_transform": { "offset": [0.3, 0.4], "scale": [0.6, 0.7], "texCoord": 0 }
            }
        });
        sample["materials"][0]["emissiveFactor"] = json!([0.1, 0.2, 0.3]);
        sample["materials"][0]["emissiveTexture"] = json!({ "index": 0 });
        sample["materials"][0]["extensions"]["KHR_materials_emissive_strength"] =
            json!({ "emissiveStrength": 2.0 });
        sample["materials"][0]["extensions"]["KHR_materials_unlit"] = json!({});

        let loaded = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap();

        assert_eq!(loaded.buffers.len(), 1);
        assert_eq!(loaded.images.len(), 1);
        assert_eq!(loaded.images[0].width, 1);
        assert_eq!(loaded.images[0].height, 1);
        assert_eq!(loaded.images[0].format, ImageFormat::R8G8B8A8);
        assert!(!loaded.images[0].bytes.is_empty());
        assert_eq!(loaded.texture_image(0).unwrap().width, 1);
        assert_eq!(loaded.texture_image(99), None);
        assert_eq!(loaded.texture_rgba8_image(0).unwrap().rgba.len(), 4);
        assert_eq!(loaded.texture_rgba8_image(99), None);
        assert_eq!(loaded.material_display_name(Some(0)), Some("mtoon"));
        assert_eq!(loaded.material_display_name(Some(99)), None);
        assert_eq!(
            loaded
                .material_outline_width_rgba8_image(Some(0))
                .unwrap()
                .rgba
                .len(),
            4
        );
        assert_eq!(loaded.material_outline_width_rgba8_image(Some(99)), None);
        assert_eq!(
            loaded.textures,
            vec![
                GltfTextureData {
                    image: 0,
                    sampler: GltfSamplerData {
                        mag_filter: GltfMagFilter::Nearest,
                        min_filter: GltfMinFilter::Linear,
                        wrap_s: GltfWrapMode::ClampToEdge,
                        wrap_t: GltfWrapMode::MirroredRepeat,
                    },
                },
                GltfTextureData {
                    image: 0,
                    sampler: GltfSamplerData {
                        mag_filter: GltfMagFilter::Linear,
                        min_filter: GltfMinFilter::LinearMipmapNearest,
                        wrap_s: GltfWrapMode::Repeat,
                        wrap_t: GltfWrapMode::ClampToEdge,
                    },
                },
            ]
        );
        assert_eq!(
            loaded.gltf_materials[0],
            GltfMaterialData {
                name: Some("mtoon".to_owned()),
                base_color_factor: [0.25, 0.5, 0.75, 1.0],
                base_color_texture: Some(0),
                base_color_texture_transform: Some(TextureTransform2d {
                    offset: [0.25, 0.5],
                    scale: [2.0, 3.0],
                    rotation: 0.125,
                    tex_coord: Some(1),
                }),
                metallic_factor: 0.75,
                roughness_factor: 0.25,
                normal_texture: Some(1),
                normal_texture_transform: Some(TextureTransform2d {
                    offset: [0.1, 0.2],
                    scale: [0.5, 0.75],
                    rotation: 0.0,
                    tex_coord: Some(0),
                }),
                normal_scale: 0.25,
                occlusion_texture: Some(1),
                occlusion_texture_transform: Some(TextureTransform2d {
                    offset: [0.3, 0.4],
                    scale: [0.6, 0.7],
                    rotation: 0.0,
                    tex_coord: Some(0),
                }),
                occlusion_strength: 0.5,
                emissive_factor: [0.1, 0.2, 0.3],
                emissive_texture: Some(0),
                emissive_texture_transform: None,
                emissive_strength: 2.0,
                unlit: true,
                alpha_mode: GltfAlphaMode::Opaque,
                alpha_cutoff: None,
                double_sided: false,
            }
        );
        let mtoon = loaded.model().document().materials[0]
            .mtoon
            .as_ref()
            .unwrap();
        assert_eq!(mtoon.base_color_factor, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(mtoon.emissive_factor, [0.1, 0.2, 0.3]);
        assert_eq!(mtoon.textures.main_texture, Some(TextureRef(0)));
        assert_eq!(
            mtoon.texture_transforms.main_texture,
            Some(TextureTransform2d {
                offset: [0.25, 0.5],
                scale: [2.0, 3.0],
                rotation: 0.125,
                tex_coord: Some(1),
            })
        );
        assert_eq!(mtoon.textures.normal_texture, Some(TextureRef(1)));
        assert_eq!(
            mtoon.texture_transforms.normal_texture,
            Some(TextureTransform2d {
                offset: [0.1, 0.2],
                scale: [0.5, 0.75],
                rotation: 0.0,
                tex_coord: Some(0),
            })
        );
        assert_eq!(
            loaded.material_texture_slots(Some(0)),
            GltfMaterialTextureSlots {
                base: Some(0),
                normal: Some(1),
                outline_width: Some(0),
                emissive: Some(0),
                occlusion: Some(1),
                ..Default::default()
            }
        );
        let texture_plan = loaded.material_texture_slots(Some(0)).binding_plan();
        assert_eq!(
            texture_plan.binding(GltfMaterialTextureSlot::Base),
            Some(GltfMaterialTextureBinding {
                slot: GltfMaterialTextureSlot::Base,
                texture: Some(0),
                color_space: GltfMaterialTextureColorSpace::Srgb,
                fallback: GltfMaterialTextureFallback::White,
            })
        );
        assert_eq!(
            texture_plan.binding(GltfMaterialTextureSlot::Normal),
            Some(GltfMaterialTextureBinding {
                slot: GltfMaterialTextureSlot::Normal,
                texture: Some(1),
                color_space: GltfMaterialTextureColorSpace::Linear,
                fallback: GltfMaterialTextureFallback::NeutralNormal,
            })
        );
        assert_eq!(
            texture_plan.binding(GltfMaterialTextureSlot::Occlusion),
            Some(GltfMaterialTextureBinding {
                slot: GltfMaterialTextureSlot::Occlusion,
                texture: Some(1),
                color_space: GltfMaterialTextureColorSpace::Linear,
                fallback: GltfMaterialTextureFallback::White,
            })
        );
        assert_eq!(
            texture_plan.binding(GltfMaterialTextureSlot::ShadingShift),
            Some(GltfMaterialTextureBinding {
                slot: GltfMaterialTextureSlot::ShadingShift,
                texture: None,
                color_space: GltfMaterialTextureColorSpace::Srgb,
                fallback: GltfMaterialTextureFallback::Black,
            })
        );
        assert_eq!(texture_plan.iter().count(), 9);
        assert_eq!(
            loaded.material_uv_transforms(Some(0), 1.0),
            GltfMaterialUvTransforms {
                base: Some(TextureTransform2d {
                    offset: [0.25, 0.5],
                    scale: [2.0, 3.0],
                    rotation: 0.125,
                    tex_coord: Some(1),
                }),
                shade: Some(TextureTransform2d {
                    offset: [0.25, 0.5],
                    scale: [2.0, 3.0],
                    rotation: 0.125,
                    tex_coord: Some(1),
                }),
                normal: Some(TextureTransform2d {
                    offset: [0.1, 0.2],
                    scale: [0.5, 0.75],
                    rotation: 0.0,
                    tex_coord: Some(0),
                }),
                occlusion: Some(TextureTransform2d {
                    offset: [0.3, 0.4],
                    scale: [0.6, 0.7],
                    rotation: 0.0,
                    tex_coord: Some(0),
                }),
                ..Default::default()
            }
        );
        let uv_plan = loaded.material_uv_transforms(Some(0), 1.0).uniform_plan();
        assert_vec4_close(uv_plan.base_transform, [0.0, 0.0, 1.0, 1.0]);
        assert_vec4_close(uv_plan.shade_transform, [0.0, 0.0, 1.0, 1.0]);
        assert_vec4_close(uv_plan.normal_transform, [0.1, 0.2, 0.5, 0.75]);
        assert_vec4_close(uv_plan.occlusion_transform, [0.3, 0.4, 0.6, 0.7]);
        assert_vec4_close(uv_plan.rotation_a, [0.0, 0.0, 0.0, 0.0]);
        assert_vec4_close(uv_plan.uv_animation, [0.0, 0.0, 0.0, 0.0]);

        let shading = loaded.material_shading_plan(
            Some(0),
            GltfMaterialShadingOptions {
                v0_compat_shade: true,
            },
        );
        assert_vec4_close(shading.base_color, [0.25, 0.5, 0.75, 1.0]);
        assert_vec3_close(shading.emissive, [0.2, 0.4, 0.6]);
        assert_f32_close(shading.normal_scale, 0.25);
        assert_f32_close(shading.metallic, 0.0);
        assert_f32_close(shading.roughness, 1.0);
        assert_f32_close(shading.occlusion_strength, 0.0);
        assert!(!shading.pbr_fallback);
        assert!(!shading.unlit);
        assert!(shading.v0_compat_shade);
        let extra = shading
            .render_extra_plan(GltfMaterialRenderExtraOptions {
                light_accumulation: GltfMtoonLightAccumulation::ThreeVrm,
                derivative_normals: true,
                view_derivative_normals: true,
                direct_light_scale: 0.75,
            })
            .uniform_plan();
        assert_vec4_close(extra.flags, [1.0, 0.0, 1.0, 1.0]);
        assert_vec4_close(extra.pbr_params, [0.0, 1.0, 0.0, 0.75]);
        assert_vec4_close(extra.flags2, [0.0, 1.0, 0.0, 0.0]);

        let fallback = loaded.material_shading_plan(None, GltfMaterialShadingOptions::default());
        assert_vec4_close(fallback.base_color, [0.78, 0.78, 0.78, 1.0]);
        assert_vec4_close(fallback.shade_color, fallback.base_color);
        assert_vec3_close(fallback.emissive, [0.0, 0.0, 0.0]);
        assert!(fallback.pbr_fallback);
        assert!(!fallback.v0_compat_shade);
        let fallback_extra = fallback
            .render_extra_plan(GltfMaterialRenderExtraOptions {
                light_accumulation: GltfMtoonLightAccumulation::Tuned,
                derivative_normals: false,
                view_derivative_normals: false,
                direct_light_scale: 1.0,
            })
            .uniform_plan();
        assert_vec4_close(fallback_extra.flags, [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn generated_sample_extracts_mesh_primitives_for_renderers() {
        let mut sample = generated_vrm1_gltf();
        sample["nodes"][0]["mesh"] = json!(0);
        sample["nodes"][0]["skin"] = json!(0);
        sample["nodes"][0]["weights"] = json!([0.75]);
        sample["buffers"] = json!([{
            "uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAIA/AAAAAAAAAAAAAIA/AAAAAAAAAAAAAIA/AAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAACAPwAAgD8AAAAAAAAAAAAAgD8AAIA/AAAAAAAAAAAAAIA/AAABAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD8=",
            "byteLength": 260
        }]);
        sample["bufferViews"] = json!([
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 72, "byteLength": 24 },
            { "buffer": 0, "byteOffset": 96, "byteLength": 24 },
            { "buffer": 0, "byteOffset": 120, "byteLength": 48 },
            { "buffer": 0, "byteOffset": 168, "byteLength": 48 },
            { "buffer": 0, "byteOffset": 216, "byteLength": 6 },
            { "buffer": 0, "byteOffset": 224, "byteLength": 36 }
        ]);
        sample["accessors"] = json!([
            {
                "bufferView": 0,
                "componentType": 5126,
                "count": 3,
                "type": "VEC3",
                "min": [0.0, 0.0, 0.0],
                "max": [1.0, 1.0, 0.0]
            },
            {
                "bufferView": 1,
                "componentType": 5126,
                "count": 3,
                "type": "VEC3"
            },
            {
                "bufferView": 2,
                "componentType": 5126,
                "count": 3,
                "type": "VEC2"
            },
            {
                "bufferView": 3,
                "componentType": 5123,
                "count": 3,
                "type": "VEC4"
            },
            {
                "bufferView": 4,
                "componentType": 5126,
                "count": 3,
                "type": "VEC4"
            },
            {
                "bufferView": 5,
                "componentType": 5126,
                "count": 3,
                "type": "VEC4"
            },
            {
                "bufferView": 6,
                "componentType": 5123,
                "count": 3,
                "type": "SCALAR"
            },
            {
                "bufferView": 7,
                "componentType": 5126,
                "count": 3,
                "type": "VEC3"
            }
        ]);
        sample["meshes"] = json!([{
            "weights": [0.25],
            "primitives": [{
                "attributes": {
                    "POSITION": 0,
                    "NORMAL": 1,
                    "TEXCOORD_0": 2,
                    "COLOR_0": 4,
                    "JOINTS_0": 3,
                    "WEIGHTS_0": 4,
                    "TANGENT": 5
                },
                "indices": 6,
                "material": 0,
                "targets": [
                    {
                        "POSITION": 7
                    }
                ]
            }]
        }]);
        sample["skins"] = json!([{
            "joints": [0]
        }]);

        let loaded = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap();

        assert_eq!(loaded.scene.node(0).unwrap().mesh, Some(0));
        assert_eq!(loaded.scene.node(0).unwrap().skin, Some(0));
        assert_eq!(loaded.scene.node(0).unwrap().weights, vec![0.75]);
        assert_eq!(loaded.skins.len(), 1);
        assert_eq!(loaded.skins[0].joints, vec![0]);
        assert_eq!(loaded.skins[0].inverse_bind_matrices, vec![Mat4::IDENTITY]);
        assert_eq!(loaded.meshes.len(), 1);
        assert_eq!(loaded.meshes[0].weights, vec![0.25]);
        let primitive = &loaded.meshes[0].primitives[0];
        assert_eq!(primitive.material, Some(0));
        assert_eq!(loaded.gltf_materials[0].base_color_factor, [1.0; 4]);
        assert_eq!(loaded.gltf_materials[0].base_color_texture, None);
        assert_eq!(loaded.gltf_materials[0].metallic_factor, 1.0);
        assert_eq!(loaded.gltf_materials[0].roughness_factor, 1.0);
        assert_eq!(loaded.gltf_materials[0].normal_texture, None);
        assert_eq!(loaded.gltf_materials[0].normal_scale, 1.0);
        assert_eq!(primitive.positions.len(), 3);
        assert_eq!(primitive.positions[1], [1.0, 0.0, 0.0]);
        assert_eq!(primitive.normals, vec![[0.0, 0.0, 1.0]; 3]);
        assert_eq!(primitive.tangents, vec![[1.0, 0.0, 0.0, 1.0]; 3]);
        assert_eq!(
            primitive.tex_coords_0,
            vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
        );
        assert_eq!(primitive.tex_coord_0_or_default(1), [1.0, 0.0]);
        assert_eq!(primitive.tex_coord_0_or_default(99), [0.0, 0.0]);
        assert_eq!(
            primitive.tex_coords_0_or_defaults(),
            vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
        );
        assert_eq!(primitive.colors_0, vec![[1.0, 0.0, 0.0, 0.0]; 3]);
        assert_eq!(
            primitive.joints_0,
            vec![[0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]]
        );
        assert_eq!(primitive.weights_0, vec![[1.0, 0.0, 0.0, 0.0]; 3]);
        assert_eq!(primitive.indices, vec![0, 1, 2]);
        assert_eq!(primitive.morph_targets.len(), 1);
        assert_eq!(primitive.morph_targets[0].positions[2], [0.0, 0.0, 0.5]);
        let morphed = primitive.morphed_vertex(2, &[0.5]).unwrap();
        assert_vec3_close(morphed.position.to_array(), [0.0, 1.0, 0.25]);
        assert_vec3_close(morphed.normal.to_array(), [0.0, 0.0, 1.0]);
        assert_vec4_close(morphed.tangent.to_array(), [1.0, 0.0, 0.0, 1.0]);
        let skin_matrices =
            loaded.skins[0].joint_matrices(&loaded.scene, &[Mat4::IDENTITY], Mat4::IDENTITY);
        assert_eq!(skin_matrices, vec![Mat4::IDENTITY]);
        let skinned = skin_vertex(
            morphed.position,
            morphed.normal,
            Mat4::IDENTITY,
            Some(&skin_matrices),
            primitive.joints_0.get(2).copied(),
            primitive.weights_0.get(2).copied(),
        );
        assert_vec3_close(skinned.position.to_array(), [0.0, 1.0, 0.25]);
        assert_vec3_close(skinned.normal.to_array(), [0.0, 0.0, 1.0]);
        let transformed = primitive
            .transformed_vertex(2, &[0.5], Mat4::IDENTITY, Some(&skin_matrices))
            .unwrap();
        assert_vec3_close(transformed.position.to_array(), [0.0, 1.0, 0.25]);
        assert_vec3_close(transformed.normal.to_array(), [0.0, 0.0, 1.0]);
        assert_vec4_close(transformed.tangent.to_array(), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(transformed.tex_coord_0, [0.0, 1.0]);
        assert_eq!(transformed.color_0, [1.0, 0.0, 0.0, 0.0]);
        let transformed_vertices = primitive
            .transformed_vertices(&[0.5], Mat4::IDENTITY, Some(&skin_matrices))
            .unwrap();
        assert_eq!(transformed_vertices.len(), 3);
        assert_vec3_close(
            transformed_vertices[2].position.to_array(),
            [0.0, 1.0, 0.25],
        );
        assert_vec4_close(
            transformed_vertices[2].tangent.to_array(),
            [1.0, 0.0, 0.0, 1.0],
        );
        assert_vec3_close(
            skin_direction(
                morphed.tangent.truncate(),
                Mat4::IDENTITY,
                Some(&skin_matrices),
                primitive.joints_0.get(2).copied(),
                primitive.weights_0.get(2).copied(),
            )
            .to_array(),
            [1.0, 0.0, 0.0],
        );
        let world_outline_scale =
            GltfOutlineScale::new(OutlineWidthMode::WorldCoordinates, Mat4::IDENTITY, 1.0);
        let outline = primitive
            .outline_position(
                2,
                &[0.5],
                GltfOutlineSettings {
                    width: 0.2,
                    scale: world_outline_scale,
                },
                Mat4::IDENTITY,
                Some(&skin_matrices),
            )
            .unwrap();
        assert_vec3_close(outline.to_array(), [0.0, 1.0, 0.45]);
        let outline_width_texture = CpuRgba8Image::from_rgba8(1, 1, vec![0, 255, 0, 255]).unwrap();
        let outline_vertices = primitive
            .outline_vertices(
                &[0.5],
                GltfOutlineVertexSettings {
                    base_width: 0.2,
                    scale: world_outline_scale,
                    width_texture: Some(&outline_width_texture),
                    width_transform: None,
                    width_texture_origin: Rgba8SamplingOrigin::TopLeft,
                },
                Mat4::IDENTITY,
                Some(&skin_matrices),
            )
            .unwrap();
        assert_eq!(outline_vertices.len(), 3);
        assert_vec3_close(outline_vertices[2].position.to_array(), [0.0, 1.0, 0.45]);
        assert_vec3_close(outline_vertices[2].normal.to_array(), [0.0, 0.0, 1.0]);
        let screen_scale =
            GltfOutlineScale::new(OutlineWidthMode::ScreenCoordinates, Mat4::IDENTITY, 2.0);
        assert_f32_close(screen_scale.at(Vec3::new(0.0, 0.0, -4.0)), 2.0);
        let generated_tangents = generate_tangents(
            &primitive.positions,
            &primitive.normals,
            &primitive.tex_coords_0,
            &primitive.indices,
        )
        .unwrap();
        assert_eq!(
            generated_tangents.all_tangents().unwrap(),
            vec![[1.0, 0.0, 0.0, 1.0]; 3]
        );
    }

    #[test]
    fn skin_vertex_applies_weighted_joint_matrices_to_positions_and_normals() {
        let joint_a = Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0));
        let joint_b = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 1.0, 1.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(0.0, 2.0, 0.0),
        );
        let skinned = skin_vertex(
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::X,
            Mat4::IDENTITY,
            Some(&[joint_a, joint_b]),
            Some([0, 1, 0, 0]),
            Some([0.25, 0.75, 0.0, 0.0]),
        );

        assert_vec3_close(skinned.position.to_array(), [0.5, 3.0, 0.0]);
        assert_vec3_close(skinned.normal.to_array(), [0.16439898, 0.9863939, 0.0]);

        let fallback = skin_vertex(
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::X,
            Mat4::from_translation(Vec3::new(0.0, 0.0, 3.0)),
            Some(&[joint_a]),
            Some([0, 0, 0, 0]),
            Some([0.0, 0.0, 0.0, 0.0]),
        );
        assert_vec3_close(fallback.position.to_array(), [1.0, 0.0, 3.0]);
        assert_vec3_close(fallback.normal.to_array(), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn generated_tangents_preserve_unreferenced_fallbacks_and_degenerate_failures() {
        let positions = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let normals = vec![[0.0, 0.0, 1.0]; 4];
        let tex_coords = vec![[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.5, 0.5]];
        let tangents = generate_tangents(&positions, &normals, &tex_coords, &[0, 1, 2]).unwrap();

        assert_eq!(tangents.tangents[0], None);
        assert_eq!(tangents.tangents[1], None);
        assert_eq!(tangents.tangents[2], None);
        assert_eq!(tangents.tangents[3], Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(tangents.all_tangents(), None);
    }

    #[test]
    fn normal_map_plan_selects_authored_generated_and_derivative_paths() {
        let mut primitive = GltfPrimitiveData {
            positions: vec![[0.0, 0.0, 0.0]; 3],
            ..Default::default()
        };

        let generated = primitive.normal_map_plan(0.8, GltfNormalMapMode::GeneratedTangents);
        assert!(!generated.authored_tangents);
        assert!(generated.should_generate_tangents());
        assert!(!generated.uses_derivative_normals());
        assert_eq!(generated.material_normal_scale(false), 0.0);
        assert_f32_close(generated.material_normal_scale(true), 0.8);
        assert_eq!(generated.vertex_normal_scale(false), 0.0);

        let derivative = primitive.normal_map_plan(0.8, GltfNormalMapMode::Derivative);
        assert!(!derivative.should_generate_tangents());
        assert!(derivative.uses_derivative_normals());
        assert!(!derivative.uses_view_derivative_normals());
        assert_f32_close(derivative.material_normal_scale(false), 0.8);
        assert_f32_close(derivative.vertex_normal_scale(false), -0.8);

        let view_derivative = primitive.normal_map_plan(0.8, GltfNormalMapMode::ViewDerivative);
        assert!(!view_derivative.should_generate_tangents());
        assert!(view_derivative.uses_derivative_normals());
        assert!(view_derivative.uses_view_derivative_normals());
        assert_f32_close(view_derivative.material_normal_scale(false), 0.8);
        assert_f32_close(view_derivative.vertex_normal_scale(false), -0.8);

        primitive.tangents = vec![[1.0, 0.0, 0.0, 1.0]; 3];
        let authored = primitive.normal_map_plan(0.8, GltfNormalMapMode::ViewDerivative);
        assert!(authored.authored_tangents);
        assert!(!authored.should_generate_tangents());
        assert!(!authored.uses_derivative_normals());
        assert!(!authored.uses_view_derivative_normals());
        assert_f32_close(authored.material_normal_scale(true), 0.8);
        assert_f32_close(authored.vertex_normal_scale(true), 0.8);

        let disabled = primitive.normal_map_plan(0.0, GltfNormalMapMode::GeneratedTangents);
        assert!(!disabled.is_enabled());
        assert_eq!(disabled.material_normal_scale(true), 0.0);
        assert_eq!(disabled.vertex_normal_scale(true), 0.0);
    }

    #[test]
    fn generated_sample_reports_invalid_hdr_emissive_extension() {
        let mut sample = generated_vrm1_gltf();
        sample["materials"][0]["extensions"]["VRMC_materials_hdr_emissiveMultiplier"]["emissiveMultiplier"] =
            json!("bright");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidExtension { extension, .. }
                if extension == "VRMC_materials_hdr_emissiveMultiplier"
        ));
    }

    #[test]
    fn generated_sample_reports_invalid_khr_emissive_extension() {
        let mut sample = generated_vrm1_gltf();
        sample["materials"][0]["extensions"]["KHR_materials_emissive_strength"]["emissiveStrength"] =
            json!("bright");
        let bytes = sample.to_string().into_bytes();

        let err = load_vrm_from_slice(&bytes).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidExtension { extension, .. }
                if extension == "KHR_materials_emissive_strength"
        ));
    }

    #[test]
    fn vrma_node_map_extracts_humanoid_expression_and_look_at_nodes() {
        let vrma = vrm_protocol::vrma::VrmcVrmAnimation {
            spec_version: "1.0".to_owned(),
            humanoid: Some(vrm_protocol::vrma::Humanoid {
                human_bones: [("hips".to_owned(), json!({ "node": 1 }))]
                    .into_iter()
                    .collect(),
            }),
            expressions: Some(vrm_protocol::vrma::Expressions {
                preset: Some(
                    [("blink".to_owned(), json!({ "node": 2 }))]
                        .into_iter()
                        .collect(),
                ),
                custom: Some(
                    [("custom".to_owned(), json!({ "node": 3 }))]
                        .into_iter()
                        .collect(),
                ),
            }),
            look_at: Some(vrm_protocol::vrma::LookAt { node: 4 }),
            extensions: None,
            extras: None,
        };

        let map = VrmaNodeMap::from_extension(&vrma);

        assert_eq!(map.humanoid.get(&1), Some(&HumanBoneName::Hips));
        assert_eq!(
            map.expressions.get(&2),
            Some(&VrmaExpressionTarget::Preset(ExpressionName::Blink))
        );
        assert_eq!(
            map.expressions.get(&3),
            Some(&VrmaExpressionTarget::Custom("custom".to_owned()))
        );
        assert_eq!(map.look_at, Some(4));
    }

    #[test]
    fn vrma_extension_warnings_follow_three_vrm_fallback_policy() {
        let mut missing = ExtensionMap::new();
        missing.insert(
            "VRMC_vrm_animation".to_owned(),
            json!({ "humanoid": { "humanBones": {} } }),
        );
        assert_eq!(
            vrma_extension_warnings(&missing),
            vec![VrmIoWarning::MissingSpecVersion {
                extension: "VRMC_vrm_animation".to_owned(),
                assumed: "1.0".to_owned(),
            }]
        );

        let mut draft = ExtensionMap::new();
        draft.insert(
            "VRMC_vrm_animation".to_owned(),
            json!({ "specVersion": "1.0-draft" }),
        );
        assert!(matches!(
            vrma_extension_warnings(&draft).as_slice(),
            [VrmIoWarning::DraftSpecVersion { .. }]
        ));

        let mut unknown = ExtensionMap::new();
        unknown.insert(
            "VRMC_vrm_animation".to_owned(),
            json!({ "specVersion": "2.0" }),
        );
        assert!(matches!(
            vrma_extension_warnings(&unknown).as_slice(),
            [VrmIoWarning::UnknownSpecVersion { version, .. }] if version == "2.0"
        ));
    }

    #[test]
    fn vrma_non_hips_humanoid_translation_warns_with_stable_message() {
        let mut sample = generated_vrma_gltf();
        sample["animations"][0]["channels"][0]["target"] =
            json!({ "node": 1, "path": "translation" });

        let loaded = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap();

        assert_eq!(
            loaded.warnings(),
            &[VrmIoWarning::IgnoredAnimationChannel {
                node: 1,
                message: "ignored non-hips humanoid translation track".to_owned()
            }]
        );
        assert!(
            loaded.model().document().animations[0]
                .humanoid_rotation_tracks
                .is_empty()
        );
    }

    #[test]
    fn vrma_invalid_expression_path_has_stable_error_message() {
        let mut sample = generated_vrma_gltf();
        sample["animations"][0]["samplers"][0]["output"] = json!(2);
        sample["animations"][0]["channels"][0]["target"] =
            json!({ "node": 15, "path": "rotation" });

        let err = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidAnimationChannel { ref message }
                if message == "invalid expression animation path for node 15"
        ));
    }

    #[test]
    fn vrma_invalid_humanoid_path_has_stable_error_message() {
        let mut sample = generated_vrma_gltf();
        sample["animations"][0]["channels"][0]["target"] = json!({ "node": 0, "path": "scale" });

        let err = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidAnimationChannel { ref message }
                if message == "invalid humanoid animation path for node 0"
        ));
    }

    #[test]
    fn vrma_invalid_look_at_path_has_stable_error_message() {
        let mut sample = generated_vrma_gltf();
        sample["animations"][0]["channels"][0]["target"] =
            json!({ "node": 16, "path": "translation" });

        let err = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap_err();

        assert!(matches!(
            err,
            VrmIoError::InvalidAnimationChannel { ref message }
                if message == "invalid lookAt animation path for node 16"
        ));
    }

    #[test]
    fn vrma_extracts_humanoid_rotation_tracks() {
        let mut sample = generated_vrma_gltf();
        sample["animations"][0]["samplers"][0]["output"] = json!(2);
        sample["animations"][0]["channels"][0]["target"] = json!({ "node": 1, "path": "rotation" });

        let loaded = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap();
        let animation = &loaded.model().document().animations[0];

        let track = animation
            .humanoid_rotation_tracks
            .get(&HumanBoneName::Head)
            .expect("head rotation track should be extracted");
        assert_eq!(track.times, vec![0.0, 1.0]);
        assert_eq!(track.values.len(), 2);
        assert_eq!(animation.duration, 1.0);
    }

    #[test]
    fn vrma_extracts_expression_and_look_at_tracks() {
        let mut sample = generated_vrma_gltf();
        sample["animations"][0]["samplers"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "input": 0,
                "output": 2,
                "interpolation": "LINEAR"
            }));
        sample["animations"][0]["channels"] = json!([
            {
                "sampler": 0,
                "target": { "node": 15, "path": "translation" }
            },
            {
                "sampler": 1,
                "target": { "node": 16, "path": "rotation" }
            }
        ]);

        let loaded = load_vrm_from_slice(sample.to_string().as_bytes()).unwrap();
        let animation = &loaded.model().document().animations[0];

        let blink = animation
            .preset_expression_tracks
            .get(&ExpressionName::Blink)
            .expect("blink expression track should be extracted");
        assert_eq!(blink.times, vec![0.0, 1.0]);
        assert_eq!(blink.values, vec![0.0, 1.0]);
        let look_at = animation
            .look_at_track
            .as_ref()
            .expect("lookAt track should be extracted");
        assert_eq!(look_at.times, vec![0.0, 1.0]);
        assert_eq!(look_at.values.len(), 2);
        assert_eq!(animation.duration, 1.0);
    }

    #[test]
    fn generated_vrma_retains_embedded_buffers_and_empty_image_list() {
        let loaded = load_vrm_from_slice(generated_vrma_gltf().to_string().as_bytes()).unwrap();

        assert_eq!(loaded.buffers.len(), 1);
        assert_eq!(loaded.buffers[0].len(), 64);
        assert!(loaded.images.is_empty());
    }

    #[test]
    fn node_rest_graph_tracks_parent_and_world_matrices() {
        let sample = generated_transform_hierarchy_gltf();
        let (document, _, _) = gltf::import_slice(sample.to_string().as_bytes()).unwrap();
        let graph = NodeRestGraph::from_document(&document);

        assert_eq!(graph.parents[1], Some(0));
        assert_eq!(graph.children[0], vec![1]);
        assert!(
            graph.local_transforms[1]
                .translation
                .abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 0.0001)
        );
        assert!(
            graph.world_transforms[1]
                .translation
                .abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 0.0001)
        );
        assert!(
            graph.world_matrices[1]
                .transform_point3(Vec3::ZERO)
                .abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 0.0001)
        );
    }

    #[test]
    fn vrma_rest_pose_captures_hips_parent_and_position() {
        let sample = generated_transform_hierarchy_gltf();
        let (document, _, _) = gltf::import_slice(sample.to_string().as_bytes()).unwrap();
        let node_map = VrmaNodeMap {
            humanoid: [(1, HumanBoneName::Hips)].into_iter().collect(),
            ..VrmaNodeMap::default()
        };

        let rest_pose = VrmaRestPose::from_document(&document, &node_map);

        assert!(
            rest_pose
                .hips_world_position
                .abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 0.0001)
        );
        assert!(
            rest_pose
                .hips_parent_world_matrix
                .transform_point3(Vec3::X)
                .abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 0.0001)
        );
    }

    #[test]
    fn human_bone_parent_handles_humanoid_chain_and_custom_bones() {
        assert_eq!(
            human_bone_parent(&HumanBoneName::Head),
            Some(HumanBoneName::Neck)
        );
        assert_eq!(
            human_bone_parent(&HumanBoneName::LeftIndexDistal),
            Some(HumanBoneName::LeftIndexIntermediate)
        );
        assert_eq!(
            human_bone_parent(&HumanBoneName::LeftThumbProximal),
            Some(HumanBoneName::LeftThumbMetacarpal)
        );
        assert_eq!(
            human_bone_parent(&HumanBoneName::Custom("x".to_owned())),
            None
        );
    }

    #[test]
    fn node_from_value_rejects_missing_or_overflowing_nodes() {
        assert_eq!(node_from_value(&json!({ "node": 7 })), Some(7));
        assert_eq!(node_from_value(&json!({ "notNode": 7 })), None);
        assert_eq!(node_from_value(&json!({ "node": "7" })), None);
    }

    #[test]
    fn supported_fixture_filter_accepts_only_known_extensions() {
        assert!(is_supported_fixture(std::path::Path::new("avatar.vrm")));
        assert!(is_supported_fixture(std::path::Path::new("clip.VRMA")));
        assert!(!is_supported_fixture(std::path::Path::new("texture.png")));
        assert!(!is_supported_fixture(std::path::Path::new("README")));
    }

    #[test]
    fn supported_fixture_discovery_recurses_into_subdirectories() {
        let root =
            std::env::temp_dir().join(format!("vrm-rs-fixture-discovery-{}", std::process::id()));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("top.vrm"), b"").unwrap();
        fs::write(nested.join("clip.vrma"), b"").unwrap();
        fs::write(nested.join("note.txt"), b"").unwrap();

        let mut fixtures = supported_fixtures_under(&root)
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        fixtures.sort();

        assert_eq!(fixtures, vec!["clip.vrma", "top.vrm"]);
        fs::remove_dir_all(root).unwrap();
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
                "VRMC_materials_mtoon",
                "VRMC_materials_hdr_emissiveMultiplier",
                "KHR_materials_emissive_strength",
                "KHR_texture_transform"
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
                    },
                    "VRMC_materials_hdr_emissiveMultiplier": {
                        "emissiveMultiplier": 2.5
                    },
                    "KHR_materials_emissive_strength": {
                        "emissiveStrength": 5.0
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
                                "materialColorBinds": [{
                                    "material": 0,
                                    "type": "color",
                                    "targetValue": [0.9, 0.8, 0.7, 0.6]
                                }, {
                                    "material": 0,
                                    "type": "emissionColor",
                                    "targetValue": [0.5, 0.4, 0.3]
                                }],
                                "textureTransformBinds": [{
                                    "material": 0,
                                    "scale": [0.25, 0.5],
                                    "offset": [0.75, 0.25]
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

    fn generated_transform_hierarchy_gltf() -> Value {
        json!({
            "asset": { "version": "2.0", "generator": "vrm-rs transform graph test" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [
                { "translation": [1.0, 0.0, 0.0], "children": [1] },
                { "translation": [0.0, 2.0, 0.0] }
            ]
        })
    }

    fn generated_vrma_gltf() -> Value {
        json!({
            "asset": { "version": "2.0", "generator": "vrm-rs generated VRMA test data" },
            "extensionsUsed": ["VRMC_vrm_animation"],
            "scene": 0,
            "scenes": [{ "nodes": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16] }],
            "nodes": (0..17)
                .map(|index| json!({ "name": format!("node_{index}") }))
                .collect::<Vec<_>>(),
            "buffers": [{
                "uri": "data:application/octet-stream;base64,AAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAQAAAQEAAAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAACAPw==",
                "byteLength": 64
            }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 8 },
                { "buffer": 0, "byteOffset": 8, "byteLength": 24 },
                { "buffer": 0, "byteOffset": 32, "byteLength": 32 }
            ],
            "accessors": [
                {
                    "bufferView": 0,
                    "componentType": 5126,
                    "count": 2,
                    "type": "SCALAR",
                    "min": [0.0],
                    "max": [1.0]
                },
                {
                    "bufferView": 1,
                    "componentType": 5126,
                    "count": 2,
                    "type": "VEC3",
                    "min": [0.0, 0.0, 0.0],
                    "max": [1.0, 2.0, 3.0]
                },
                {
                    "bufferView": 2,
                    "componentType": 5126,
                    "count": 2,
                    "type": "VEC4",
                    "min": [0.0, 0.0, 0.0, 1.0],
                    "max": [0.0, 0.0, 0.0, 1.0]
                }
            ],
            "animations": [{
                "samplers": [{
                    "input": 0,
                    "output": 1,
                    "interpolation": "LINEAR"
                }],
                "channels": [{
                    "sampler": 0,
                    "target": {
                        "node": 0,
                        "path": "translation"
                    }
                }]
            }],
            "extensions": {
                "VRMC_vrm_animation": {
                    "specVersion": "1.0",
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
                    },
                    "expressions": {
                        "preset": {
                            "blink": { "node": 15 }
                        }
                    },
                    "lookAt": { "node": 16 }
                }
            }
        })
    }

    fn generated_glb(json: Value, bin: &[u8]) -> Vec<u8> {
        generated_glb_with_chunks(json, &[(GLB_BIN_CHUNK_TYPE, bin.to_vec())])
    }

    fn generated_glb_with_chunks(json: Value, chunks: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut json_bytes = json.to_string().into_bytes();
        pad_json_chunk(&mut json_bytes);
        let padded_chunks = chunks
            .iter()
            .map(|(raw_type, bytes)| {
                let mut bytes = bytes.clone();
                while !bytes.len().is_multiple_of(4) {
                    bytes.push(0);
                }
                (*raw_type, bytes)
            })
            .collect::<Vec<_>>();

        let total_len = 12
            + 8
            + json_bytes.len()
            + padded_chunks
                .iter()
                .map(|(_, bytes)| 8 + bytes.len())
                .sum::<usize>();
        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(GLB_MAGIC);
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&(total_len as u32).to_le_bytes());
        bytes.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&GLB_JSON_CHUNK_TYPE.to_le_bytes());
        bytes.extend_from_slice(&json_bytes);
        for (raw_type, chunk_bytes) in padded_chunks {
            bytes.extend_from_slice(&(chunk_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&raw_type.to_le_bytes());
            bytes.extend_from_slice(&chunk_bytes);
        }
        bytes
    }

    fn temp_output_path(label: &str, extension: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "vrm-rs-{label}-{}-{unique}.{extension}",
            std::process::id()
        ))
    }

    #[test]
    #[ignore = "requires local external fixtures; set VRM_RS_FIXTURE_DIR"]
    fn loads_external_fixture_directory() {
        let fixture_dir = env::var_os("VRM_RS_FIXTURE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".external-fixtures/official"));
        let mut loaded = Vec::new();
        for path in supported_fixtures_under(&fixture_dir) {
            if !is_supported_fixture(&path) {
                continue;
            }
            let result = load_vrm_from_path(&path);
            assert!(
                result.is_ok(),
                "failed to load external fixture {}: {:?}",
                path.display(),
                result.err()
            );
            let result = result.unwrap();
            assert_external_fixture_semantics(&path, &result);
            loaded.push(path);
        }

        assert!(
            !loaded.is_empty(),
            "no .vrm/.vrma/.glb/.gltf fixtures found in {}",
            fixture_dir.display()
        );
    }

    fn assert_external_fixture_semantics(path: &std::path::Path, loaded: &LoadedVrm) {
        let document = loaded.model().document();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();

        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("vrma"))
        {
            assert!(
                !document.animations.is_empty(),
                "VRMA fixture did not produce animations: {}",
                path.display()
            );
            let animation = &document.animations[0];
            assert!(
                animation.duration > 0.0,
                "VRMA fixture has zero duration: {}",
                path.display()
            );
            assert!(
                animation.hips_translation.is_some()
                    || !animation.humanoid_rotation_tracks.is_empty()
                    || !animation.preset_expression_tracks.is_empty()
                    || !animation.custom_expression_tracks.is_empty()
                    || animation.look_at_track.is_some(),
                "VRMA fixture has no extracted tracks: {}",
                path.display()
            );
            if file_name.eq_ignore_ascii_case("test.vrma") {
                let track_classes = [
                    !animation.humanoid_rotation_tracks.is_empty(),
                    animation.hips_translation.is_some(),
                    !animation.preset_expression_tracks.is_empty()
                        || !animation.custom_expression_tracks.is_empty(),
                    animation.look_at_track.is_some(),
                ]
                .into_iter()
                .filter(|present| *present)
                .count();
                assert!(
                    !animation.humanoid_rotation_tracks.is_empty(),
                    "test.vrma should expose humanoid rotation tracks"
                );
                assert!(
                    track_classes >= 2,
                    "test.vrma should expose multiple VRMA track classes"
                );
            }
            return;
        }

        assert!(
            !document.meta.name.is_empty(),
            "VRM fixture has empty meta name: {}",
            path.display()
        );
        assert!(
            !document.humanoid.bones.is_empty(),
            "VRM fixture has no humanoid bones: {}",
            path.display()
        );
        if file_name.eq_ignore_ascii_case("Seed-san.vrm") {
            assert!(
                !document.materials.is_empty(),
                "Seed-san should expose material data"
            );
            assert!(
                document.spring_bone.is_present(),
                "Seed-san should expose spring bone data"
            );
        }
        if file_name.eq_ignore_ascii_case("VRM1_Constraint_Twist_Sample.vrm") {
            assert!(
                !document.node_constraints.is_empty(),
                "constraint sample should expose node constraints"
            );
            assert!(
                document.spring_bone.is_present(),
                "constraint sample should expose spring bone data"
            );
        }
        if file_name.eq_ignore_ascii_case("VRMC_materials_mtoon_UV_Animation_Test.vrm") {
            let animated_mtoon = document
                .materials
                .iter()
                .filter_map(|material| material.mtoon.as_ref())
                .find(|mtoon| {
                    mtoon.uv_animation.scroll_x_speed != 0.0
                        || mtoon.uv_animation.scroll_y_speed != 0.0
                        || mtoon.uv_animation.rotation_speed != 0.0
                        || mtoon.textures.uv_animation_mask_texture.is_some()
                });
            assert!(
                animated_mtoon.is_some(),
                "MToon UV animation sample should expose UV animation parameters"
            );
        }
        if file_name.eq_ignore_ascii_case("VRMC_vrm_expressions_isBinary_Overrides.vrm")
            || file_name.eq_ignore_ascii_case("VRMC_vrm_expressions_isBinary_Overridden.vrm")
        {
            let expressions = document.expressions.as_ref().unwrap_or_else(|| {
                panic!(
                    "expression override sample should expose expressions: {}",
                    path.display()
                )
            });
            let preset_count = expressions.preset.len();
            let has_binary = expressions
                .preset
                .values()
                .chain(expressions.custom.values())
                .any(|expression| expression.is_binary);
            let has_override = expressions
                .preset
                .values()
                .chain(expressions.custom.values())
                .any(|expression| {
                    expression.override_blink != vrm_core::OverrideMode::None
                        || expression.override_look_at != vrm_core::OverrideMode::None
                        || expression.override_mouth != vrm_core::OverrideMode::None
                });
            assert!(
                preset_count > 0,
                "expression override sample should expose preset expressions"
            );
            assert!(
                has_binary || has_override,
                "expression override sample should expose binary or override metadata"
            );
        }
        if file_name.eq_ignore_ascii_case("VRM0_AliciaSolid.vrm")
            || file_name.eq_ignore_ascii_case("AliciaSolid_vrm-0.51.vrm")
        {
            assert_eq!(document.kind, vrm_core::VrmKind::Vrm0Compat);
            assert!(
                document.compatibility.vrm0.is_some(),
                "VRM0 fixture should expose compatibility metadata"
            );
            assert!(
                document.first_person.is_present() || document.expressions.is_present(),
                "VRM0 fixture should expose first-person or expression compatibility data"
            );
            assert_eq!(document.meta.name, "Alicia Solid");
            assert_eq!(document.humanoid.bones.len(), 55);
            assert!(
                document.humanoid.bones.contains_key(&HumanBoneName::Head)
                    && document
                        .humanoid
                        .bones
                        .contains_key(&HumanBoneName::LeftThumbMetacarpal)
                    && document
                        .humanoid
                        .bones
                        .contains_key(&HumanBoneName::LeftThumbProximal)
                    && document
                        .humanoid
                        .bones
                        .contains_key(&HumanBoneName::RightThumbMetacarpal)
                    && document
                        .humanoid
                        .bones
                        .contains_key(&HumanBoneName::RightThumbProximal),
                "Alicia VRM0 humanoid bones should include normalized head and thumb aliases"
            );

            let first_person = document.first_person.as_ref().unwrap_or_else(|| {
                panic!("Alicia VRM0 fixture should expose first-person annotations")
            });
            assert_eq!(first_person.mesh_annotations.len(), 12);
            assert!(
                first_person
                    .mesh_annotations
                    .iter()
                    .all(|annotation| annotation.kind == FirstPersonAnnotation::Auto),
                "Alicia VRM0 mesh annotations should preserve Auto flags"
            );

            let look_at = document
                .look_at
                .as_ref()
                .unwrap_or_else(|| panic!("Alicia VRM0 fixture should expose lookAt data"));
            assert_eq!(look_at.kind, LookAtKind::Bone);
            assert!((look_at.offset_from_head.y - 0.059999943).abs() < 0.000001);
            assert_eq!(look_at.horizontal_inner.input_max_value, 20.0);
            assert_eq!(look_at.horizontal_inner.output_scale, 5.0);

            let expressions = document
                .expressions
                .as_ref()
                .unwrap_or_else(|| panic!("Alicia VRM0 fixture should expose expressions"));
            assert_eq!(expressions.preset.len(), 17);
            for preset in [
                ExpressionName::Aa,
                ExpressionName::Ih,
                ExpressionName::Ou,
                ExpressionName::Ee,
                ExpressionName::Oh,
                ExpressionName::Happy,
                ExpressionName::Sad,
                ExpressionName::Relaxed,
                ExpressionName::BlinkLeft,
                ExpressionName::BlinkRight,
            ] {
                assert!(
                    expressions.preset.contains_key(&preset),
                    "Alicia VRM0 preset {} should map to canonical expression",
                    preset.as_str()
                );
            }

            assert_eq!(document.materials.len(), 12);
            assert!(
                document
                    .materials
                    .iter()
                    .all(|material| material.mtoon.is_present()),
                "Alicia VRM0 materials should map legacy VRM/MToon properties"
            );
            let body_mtoon = document.materials[0].mtoon.as_ref().unwrap();
            assert_eq!(body_mtoon.render_queue, MtoonRenderQueue::Opaque);
            assert!(body_mtoon.outline_enabled());
            assert_eq!(body_mtoon.base_color_factor, [1.0, 1.0, 1.0, 1.0]);
            assert_eq!(body_mtoon.emissive_factor, [0.0, 0.0, 0.0]);
            assert_eq!(body_mtoon.cutoff_factor, 0.5);
            assert_vec3_close(
                body_mtoon.shade_color_factor,
                [
                    gamma_eotf(1.0),
                    gamma_eotf(0.8666667),
                    gamma_eotf(0.84000003),
                ],
            );
            assert_eq!(body_mtoon.receive_shadow_rate_factor, 1.0);
            assert_eq!(body_mtoon.shading_grade_rate_factor, 1.0);
            assert_f32_close(body_mtoon.shading_shift_factor, -0.05);
            assert_f32_close(body_mtoon.shading_toony_factor, 0.95);
            assert_eq!(body_mtoon.light_color_attenuation_factor, 0.0);
            assert_eq!(body_mtoon.gi_equalization_factor, 0.9);
            assert_eq!(
                body_mtoon.outline_width_mode,
                OutlineWidthMode::WorldCoordinates
            );
            assert_f32_close(body_mtoon.outline_width_factor, 0.0005);
            assert_vec3_close(
                body_mtoon.outline_color_factor,
                [
                    gamma_eotf(0.671),
                    gamma_eotf(0.55702585),
                    gamma_eotf(0.53478694),
                ],
            );
            assert_eq!(body_mtoon.outline_lighting_mix_factor, 1.0);
            assert_eq!(
                body_mtoon.textures.main_texture,
                Some(vrm_core::TextureRef(0))
            );
            assert_eq!(
                body_mtoon.textures.shade_multiply_texture,
                Some(vrm_core::TextureRef(0))
            );
            assert_eq!(
                body_mtoon.textures.matcap_texture,
                Some(vrm_core::TextureRef(1))
            );
            assert_eq!(body_mtoon.textures.normal_texture, None);
            assert_eq!(body_mtoon.textures.outline_width_multiply_texture, None);
            assert_eq!(body_mtoon.uv_animation.scroll_x_speed, 0.0);
            assert_eq!(body_mtoon.uv_animation.scroll_y_speed, 0.0);
            assert_eq!(body_mtoon.uv_animation.rotation_speed, 0.0);

            let spring_bone = document.spring_bone.as_ref().unwrap_or_else(|| {
                panic!("Alicia VRM0 fixture should expose secondary animation as spring bone")
            });
            assert_eq!(spring_bone.springs.len(), 3);
            assert_eq!(spring_bone.collider_groups.len(), 6);
            assert_eq!(
                spring_bone
                    .springs
                    .iter()
                    .map(|spring| spring.joints.len())
                    .sum::<usize>(),
                48
            );
            assert!(
                spring_bone
                    .springs
                    .iter()
                    .all(|spring| spring.center.is_none())
            );
            assert!(
                spring_bone
                    .springs
                    .iter()
                    .any(|spring| spring.joints.len() >= 5),
                "Alicia VRM0 spring groups should retain multi-joint chains"
            );
        }
    }

    fn is_supported_fixture(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "vrm" | "vrma" | "glb" | "gltf"
                )
            })
    }

    fn supported_fixtures_under(root: &std::path::Path) -> Vec<PathBuf> {
        let mut result = Vec::new();
        collect_supported_fixtures(root, &mut result);
        result
    }

    fn collect_supported_fixtures(path: &std::path::Path, result: &mut Vec<PathBuf>) {
        if path.is_file() {
            if is_supported_fixture(path) {
                result.push(path.to_owned());
            }
            return;
        }

        let entries = fs::read_dir(path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for entry in entries {
            collect_supported_fixtures(&entry.unwrap().path(), result);
        }
    }
}
