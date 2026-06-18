//! Ash/Vulkan-shaped frame planning for VRM assets.
//!
//! This crate intentionally does not create Vulkan instances, devices, swapchains,
//! or shader modules. It keeps the unsafe Vulkan boundary in the downstream ash
//! application while providing renderer-ready CPU vertices, indices, texture
//! uploads, and `ash::vk`-typed pipeline/descriptor plans.

use ash::vk;
use bytemuck::{Pod, Zeroable};
use clap::Parser;
use glam::Mat4;
use std::{collections::HashMap, error::Error, path::PathBuf};
use vrm_adapter::{
    HeadlessSceneState, HumanoidPoseRig, MtoonMaterializationOptions, MtoonRendererPass,
    MtoonSamplerHint, MtoonTextureBindingPlan, WorldMatrixAccess, WorldTransformUpdate,
    apply_vrma_animation_frame_with_look_at, mtoon_renderer_material_plans,
};
use vrm_core::{Feature, MaterialRef, MtoonCullMode, NodeRef, TextureRef, VrmAnimation};
use vrm_io::{
    CpuRgba8Image, GltfAlphaMode, GltfNodeRest, GltfPrimitiveData, LoadedVrm, load_vrm_from_path,
};
use vrm_runtime::sample_vrm_animation;

#[derive(Clone, Debug, Parser)]
#[command(about = "Build an ash/Vulkan-shaped VRM frame plan without opening a renderer")]
pub struct AshVrmFramePlanOptions {
    /// VRM avatar file.
    #[arg(long, default_value = ".external-fixtures/official/Seed-san.vrm")]
    pub avatar: PathBuf,
    /// Optional VRMA animation clip file.
    #[arg(long, default_value = ".external-fixtures/official/idle_loop.vrma")]
    pub animation: PathBuf,
    /// Disable VRMA sampling after loading the avatar.
    #[arg(long)]
    pub no_animation: bool,
    /// Sample time in seconds.
    #[arg(long, default_value_t = 0.0)]
    pub time: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct AshVrmVertex {
    pub position: [f32; 3],
    pub tex_coord_0: [f32; 2],
    pub color_0: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshVrmPrimitive {
    pub node: NodeRef,
    pub material: Option<MaterialRef>,
    pub vertices: Vec<AshVrmVertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshTextureUpload {
    pub texture: Option<TextureRef>,
    pub format: vk::Format,
    pub extent: vk::Extent3D,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshMaterialRecord {
    pub material: MaterialRef,
    pub base_color_factor: [f32; 4],
    pub base_color_texture_upload: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshMtoonPass {
    Base,
    Outline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshPipelineKey {
    pub pass: AshMtoonPass,
    pub render_order: i32,
    pub phase_order: i32,
    pub topology: vk::PrimitiveTopology,
    pub cull_mode: vk::CullModeFlags,
    pub front_face: vk::FrontFace,
    pub depth_test_enable: bool,
    pub depth_write_enable: bool,
    pub depth_compare_op: vk::CompareOp,
    pub blend_enable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshDescriptorBindingPlan {
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub stage_flags: vk::ShaderStageFlags,
    pub texture: Option<TextureRef>,
    pub sampler: Option<AshSamplerPlan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshSamplerPlan {
    pub mag_filter: vk::Filter,
    pub min_filter: vk::Filter,
    pub mipmap_mode: vk::SamplerMipmapMode,
    pub address_mode_u: vk::SamplerAddressMode,
    pub address_mode_v: vk::SamplerAddressMode,
    pub normal_map_decode: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshMtoonPipelinePlan {
    pub material: MaterialRef,
    pub name: Option<String>,
    pub key: AshPipelineKey,
    pub descriptor_bindings: Vec<AshDescriptorBindingPlan>,
    pub push_constant_size: u32,
    pub alpha_cutoff: f32,
    pub outline_width: f32,
    pub base_color_factor: [f32; 4],
    pub emissive_color: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AshVrmFramePlan {
    pub primitives: Vec<AshVrmPrimitive>,
    pub materials: Vec<AshMaterialRecord>,
    pub texture_uploads: Vec<AshTextureUpload>,
    pub mtoon_pipelines: Vec<AshMtoonPipelinePlan>,
}

pub struct AshVrmFramePlanner {
    loaded: LoadedVrm,
    scene: HeadlessSceneState,
    rig: HumanoidPoseRig,
    animation: Option<VrmAnimation>,
}

impl AshVrmFramePlanner {
    pub fn from_paths(
        avatar: impl Into<PathBuf>,
        animation: Option<impl Into<PathBuf>>,
    ) -> Result<Self, Box<dyn Error>> {
        let loaded = load_vrm_from_path(avatar.into())?;
        let animation = match animation {
            Some(path) => {
                let loaded_animation = load_vrm_from_path(path.into())?;
                animation_from_loaded(&loaded_animation)
            }
            None => None,
        };
        Self::new(loaded, animation)
    }

    pub fn new(loaded: LoadedVrm, animation: Option<VrmAnimation>) -> Result<Self, Box<dyn Error>> {
        let mut scene = headless_scene_from_loaded(&loaded)?;
        scene.update_world_transforms()?;
        let rig = HumanoidPoseRig::capture(&scene, loaded.model().document())?;
        Ok(Self {
            loaded,
            scene,
            rig,
            animation,
        })
    }

    pub fn sample_frame(&mut self, time_seconds: f32) -> Result<AshVrmFramePlan, Box<dyn Error>> {
        if let Some(animation) = &self.animation {
            let time = if animation.duration > f32::EPSILON {
                time_seconds.rem_euclid(animation.duration)
            } else {
                0.0
            };
            let frame = sample_vrm_animation(animation, time);
            apply_vrma_animation_frame_with_look_at(
                &mut self.scene,
                &mut self.rig,
                self.loaded.model().document(),
                &frame,
            )?;
        }
        self.scene.update_world_transforms()?;
        let texture_uploads = self.texture_uploads();
        let material_texture_uploads = material_texture_upload_indices(&texture_uploads);
        Ok(AshVrmFramePlan {
            primitives: self.bake_primitives()?,
            materials: self.material_records(&material_texture_uploads),
            texture_uploads,
            mtoon_pipelines: self.mtoon_pipeline_plans(),
        })
    }

    fn world_matrices(&self) -> Vec<Mat4> {
        self.loaded
            .scene
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                self.scene
                    .world_matrix(NodeRef(index))
                    .unwrap_or(node.world_matrix)
            })
            .collect()
    }

    fn bake_primitives(&self) -> Result<Vec<AshVrmPrimitive>, Box<dyn Error>> {
        let mut primitives = Vec::new();
        for (node_index, node) in self.loaded.scene.nodes.iter().enumerate() {
            let Some(mesh_index) = node.mesh else {
                continue;
            };
            let mesh = &self.loaded.meshes[mesh_index];
            for primitive in &mesh.primitives {
                primitives.push(self.bake_primitive(node_index, node, mesh, primitive)?);
            }
        }
        Ok(primitives)
    }

    fn bake_primitive(
        &self,
        node_index: usize,
        node: &GltfNodeRest,
        mesh: &vrm_io::GltfMeshData,
        primitive: &GltfPrimitiveData,
    ) -> Result<AshVrmPrimitive, Box<dyn Error>> {
        let morph_weights = active_morph_weights(&self.scene, node_index, node, mesh);
        let world_matrices = self.world_matrices();
        let world = world_matrices[node_index];
        let skin_matrices = node.skin.and_then(|skin| {
            self.loaded.skins.get(skin).map(|skin| {
                skin.joint_matrices(&self.loaded.scene, &world_matrices, Mat4::IDENTITY)
            })
        });
        let material = primitive
            .material
            .and_then(|index| self.loaded.gltf_materials.get(index));
        let base_color = material
            .map(|material| material.base_color_factor)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let alpha_enabled = material
            .map(|material| material.alpha_mode != GltfAlphaMode::Opaque)
            .unwrap_or(false);
        let vertices = primitive
            .transformed_vertices(&morph_weights, world, skin_matrices.as_deref())
            .ok_or("primitive geometry is inconsistent")?
            .into_iter()
            .map(|vertex| {
                let alpha = if alpha_enabled {
                    base_color[3] * vertex.color_0[3]
                } else {
                    1.0
                };
                AshVrmVertex {
                    position: vertex.position.to_array(),
                    tex_coord_0: vertex.tex_coord_0,
                    color_0: [
                        base_color[0] * vertex.color_0[0],
                        base_color[1] * vertex.color_0[1],
                        base_color[2] * vertex.color_0[2],
                        alpha,
                    ],
                }
            })
            .collect();
        Ok(AshVrmPrimitive {
            node: NodeRef(node_index),
            material: primitive.material.map(MaterialRef),
            vertices,
            indices: primitive.indices.clone(),
        })
    }

    fn material_records(&self, texture_uploads: &HashMap<usize, usize>) -> Vec<AshMaterialRecord> {
        self.loaded
            .gltf_materials
            .iter()
            .enumerate()
            .map(|(index, material)| AshMaterialRecord {
                material: MaterialRef(index),
                base_color_factor: material.base_color_factor,
                base_color_texture_upload: texture_uploads.get(&index).copied(),
            })
            .collect()
    }

    fn texture_uploads(&self) -> Vec<AshTextureUpload> {
        self.loaded
            .gltf_materials
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                self.loaded
                    .material_base_texture_rgba8_image(Some(index))
                    .map(|image| texture_upload(Some(TextureRef(index)), image))
            })
            .collect()
    }

    fn mtoon_pipeline_plans(&self) -> Vec<AshMtoonPipelinePlan> {
        mtoon_renderer_material_plans(
            self.loaded.model().document(),
            MtoonMaterializationOptions::default(),
        )
        .into_iter()
        .map(|plan| AshMtoonPipelinePlan {
            material: plan.material,
            name: plan.name,
            key: AshPipelineKey {
                pass: match plan.pass {
                    MtoonRendererPass::Base => AshMtoonPass::Base,
                    MtoonRendererPass::Outline => AshMtoonPass::Outline,
                },
                render_order: plan.pipeline.render_order,
                phase_order: plan.pipeline.phase_order,
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                cull_mode: cull_mode(plan.pipeline.cull_mode),
                front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                depth_test_enable: plan.pipeline.depth_test,
                depth_write_enable: plan.pipeline.depth_write,
                depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                blend_enable: plan.pipeline.blend,
            },
            descriptor_bindings: descriptor_bindings(&plan.texture_bindings),
            push_constant_size: 160,
            alpha_cutoff: plan.shader.cutoff_factor,
            outline_width: plan.shader.outline_width_factor,
            base_color_factor: plan.shader.base_color_factor,
            emissive_color: plan.shader.emissive_color,
        })
        .collect()
    }
}

pub fn frame_plan_from_options(
    options: &AshVrmFramePlanOptions,
) -> Result<AshVrmFramePlan, Box<dyn Error>> {
    let animation = (!options.no_animation).then_some(options.animation.clone());
    let mut planner = AshVrmFramePlanner::from_paths(options.avatar.clone(), animation)?;
    planner.sample_frame(options.time)
}

fn material_texture_upload_indices(texture_uploads: &[AshTextureUpload]) -> HashMap<usize, usize> {
    texture_uploads
        .iter()
        .enumerate()
        .filter_map(|(upload_index, upload)| {
            upload.texture.map(|texture| (texture.0, upload_index))
        })
        .collect()
}

fn cull_mode(mode: MtoonCullMode) -> vk::CullModeFlags {
    match mode {
        MtoonCullMode::Off => vk::CullModeFlags::NONE,
        MtoonCullMode::Front => vk::CullModeFlags::FRONT,
        MtoonCullMode::Back => vk::CullModeFlags::BACK,
    }
}

fn descriptor_bindings(bindings: &[MtoonTextureBindingPlan]) -> Vec<AshDescriptorBindingPlan> {
    let mut result = Vec::with_capacity(bindings.len() + 1);
    result.push(AshDescriptorBindingPlan {
        binding: 0,
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        sampler: None,
    });
    result.extend(
        bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| AshDescriptorBindingPlan {
                binding: index as u32 + 1,
                descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
                texture: Some(binding.texture),
                sampler: Some(sampler_plan(binding.sampler)),
            }),
    );
    result
}

fn sampler_plan(hint: MtoonSamplerHint) -> AshSamplerPlan {
    AshSamplerPlan {
        mag_filter: vk::Filter::LINEAR,
        min_filter: vk::Filter::LINEAR,
        mipmap_mode: vk::SamplerMipmapMode::LINEAR,
        address_mode_u: vk::SamplerAddressMode::REPEAT,
        address_mode_v: vk::SamplerAddressMode::REPEAT,
        normal_map_decode: matches!(hint, MtoonSamplerHint::NormalMapLinearRepeat),
    }
}

fn texture_upload(texture: Option<TextureRef>, image: CpuRgba8Image) -> AshTextureUpload {
    AshTextureUpload {
        texture,
        format: vk::Format::R8G8B8A8_SRGB,
        extent: vk::Extent3D {
            width: image.width,
            height: image.height,
            depth: 1,
        },
        rgba: image.rgba,
    }
}

fn animation_from_loaded(loaded: &LoadedVrm) -> Option<VrmAnimation> {
    match &loaded.model().document().animation {
        Feature::Present(animation) => Some(animation.clone()),
        Feature::Absent => loaded.model().document().animations.first().cloned(),
    }
}

fn active_morph_weights(
    scene: &HeadlessSceneState,
    node_index: usize,
    node: &GltfNodeRest,
    mesh: &vrm_io::GltfMeshData,
) -> Vec<f32> {
    let mut weights = if node.weights.is_empty() {
        mesh.weights.clone()
    } else {
        node.weights.clone()
    };
    for index in 0..mesh
        .primitives
        .iter()
        .map(|primitive| primitive.morph_targets.len())
        .max()
        .unwrap_or(0)
    {
        if let Some(weight) = scene.morph_weight(NodeRef(node_index), index) {
            if weights.len() <= index {
                weights.resize(index + 1, 0.0);
            }
            weights[index] = weight;
        }
    }
    weights
}

fn headless_scene_from_loaded(loaded: &LoadedVrm) -> Result<HeadlessSceneState, Box<dyn Error>> {
    let mut scene = HeadlessSceneState::default();
    for (index, node) in loaded.scene.nodes.iter().enumerate() {
        scene.insert_node(NodeRef(index), node.local);
    }
    for (index, node) in loaded.scene.nodes.iter().enumerate() {
        scene.set_parent(NodeRef(index), node.parent.map(NodeRef))?;
    }
    scene.update_world_transforms()?;
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ash_sampler_hint_marks_normal_decode() {
        assert!(sampler_plan(MtoonSamplerHint::NormalMapLinearRepeat).normal_map_decode);
        assert!(!sampler_plan(MtoonSamplerHint::LinearRepeat).normal_map_decode);
    }

    #[test]
    fn descriptor_bindings_start_with_uniform_buffer() {
        let bindings = descriptor_bindings(&[]);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].binding, 0);
        assert_eq!(
            bindings[0].descriptor_type,
            vk::DescriptorType::UNIFORM_BUFFER
        );
    }
}
