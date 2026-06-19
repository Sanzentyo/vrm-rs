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
    HeadlessSceneState, HumanoidPoseRig, MTOON_GPU_UNIFORM_SIZE, MtoonGpuMaterial, MtoonGpuUniform,
    MtoonLightingConfig, MtoonMaterializationOptions, MtoonRendererPass, MtoonRendererTextureRefs,
    MtoonSamplerHint, MtoonTextureSlot, WorldMatrixAccess, WorldTransformUpdate,
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
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
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
    pub uniform: MtoonGpuUniform,
    pub uniform_buffer_size: u32,
    pub alpha_cutoff: f32,
    pub outline_width: f32,
    pub base_color_factor: [f32; 4],
    pub emissive_color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct AshSceneUniform {
    pub view_projection: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub world_from_view: [[f32; 4]; 4],
    pub light_dir: [f32; 4],
    pub light_color: [f32; 4],
    pub camera_pos: [f32; 4],
    pub mtoon_lighting: [f32; 4],
}

impl AshSceneUniform {
    pub fn parity_camera(aspect_ratio: f32) -> Self {
        let aspect_ratio = if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
            aspect_ratio
        } else {
            1.0
        };
        let eye = glam::Vec3::new(0.0, 1.4, -4.0);
        let view = Mat4::look_at_rh(eye, glam::Vec3::new(0.0, 1.0, 0.0), glam::Vec3::Y);
        let projection = Mat4::perspective_rh(30.0_f32.to_radians(), aspect_ratio, 0.1, 20.0);
        let light_dir = glam::Vec3::new(-1.0, 1.0, -1.0).normalize();
        Self {
            view_projection: (projection * view).to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            world_from_view: view.inverse().to_cols_array_2d(),
            light_dir: [light_dir.x, light_dir.y, light_dir.z, 1.0],
            light_color: [1.0, 1.0, 1.0, 0.0],
            camera_pos: [eye.x, eye.y, eye.z, 1.0],
            mtoon_lighting: MtoonLightingConfig::default().effective_values().to_array(),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

impl Default for AshSceneUniform {
    fn default() -> Self {
        Self::parity_camera(1.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AshVrmFramePlan {
    pub primitives: Vec<AshVrmPrimitive>,
    pub materials: Vec<AshMaterialRecord>,
    pub texture_uploads: Vec<AshTextureUpload>,
    pub mtoon_pipelines: Vec<AshMtoonPipelinePlan>,
    pub scene_uniform: AshSceneUniform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshBufferRole {
    Vertex,
    Index,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshBufferUpload {
    pub role: AshBufferRole,
    pub usage: vk::BufferUsageFlags,
    pub stride: u32,
    pub count: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshTextureResourcePlan {
    pub upload: AshTextureUpload,
    pub image_usage: vk::ImageUsageFlags,
    pub image_layout_after_upload: vk::ImageLayout,
    pub aspect_mask: vk::ImageAspectFlags,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshDescriptorSetPlan {
    pub material: MaterialRef,
    pub pipeline_plan_index: usize,
    pub bindings: Vec<AshResolvedDescriptorBinding>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshResolvedDescriptorBinding {
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub stage_flags: vk::ShaderStageFlags,
    pub uniform_upload_index: Option<usize>,
    pub texture_upload_index: Option<usize>,
    pub sampler: Option<AshSamplerPlan>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshDrawCallPlan {
    pub primitive_index: usize,
    pub material: Option<MaterialRef>,
    pub pipeline_plan_index: Option<usize>,
    pub descriptor_set_index: Option<usize>,
    pub vertex_buffer_index: usize,
    pub index_buffer_index: usize,
    pub index_count: u32,
    pub render_order: i32,
    pub phase_order: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AshRendererFrame {
    pub buffers: Vec<AshBufferUpload>,
    pub textures: Vec<AshTextureResourcePlan>,
    pub uniforms: Vec<AshUniformUpload>,
    pub pipelines: Vec<AshGraphicsPipelinePlan>,
    pub descriptor_sets: Vec<AshDescriptorSetPlan>,
    pub draw_calls: Vec<AshDrawCallPlan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AshUniformScope {
    Material {
        material: MaterialRef,
        pipeline_plan_index: usize,
    },
    Scene,
}

impl AshUniformScope {
    pub const fn material(material: MaterialRef, pipeline_plan_index: usize) -> Self {
        Self::Material {
            material,
            pipeline_plan_index,
        }
    }

    pub const fn binding(self) -> u32 {
        match self {
            Self::Material { .. } => ash_mtoon_uniform_binding(),
            Self::Scene => ash_mtoon_scene_binding(),
        }
    }

    pub const fn material_ref(self) -> Option<MaterialRef> {
        match self {
            Self::Material { material, .. } => Some(material),
            Self::Scene => None,
        }
    }

    pub const fn pipeline_plan_index(self) -> Option<usize> {
        match self {
            Self::Material {
                pipeline_plan_index,
                ..
            } => Some(pipeline_plan_index),
            Self::Scene => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshUniformUpload {
    pub scope: AshUniformScope,
    pub binding: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AshGraphicsPipelinePlan {
    pub material: MaterialRef,
    pub pipeline_plan_index: usize,
    pub descriptor_set_index: usize,
    pub key: AshPipelineKey,
    pub vertex_stride: u32,
    pub vertex_attributes: Vec<AshVertexAttributePlan>,
    pub color_format: vk::Format,
    pub depth_format: Option<vk::Format>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AshVertexAttributePlan {
    pub location: u32,
    pub binding: u32,
    pub format: vk::Format,
    pub offset: u32,
}

pub fn ash_renderer_frame_from_plan(plan: &AshVrmFramePlan) -> AshRendererFrame {
    let texture_indices = texture_ref_upload_indices(&plan.texture_uploads);
    let material_uniform_count = plan.mtoon_pipelines.len();
    let scene_uniform_upload_index = material_uniform_count;
    let descriptor_sets = plan
        .mtoon_pipelines
        .iter()
        .enumerate()
        .map(|(pipeline_plan_index, pipeline)| AshDescriptorSetPlan {
            material: pipeline.material,
            pipeline_plan_index,
            bindings: pipeline
                .descriptor_bindings
                .iter()
                .map(|binding| AshResolvedDescriptorBinding {
                    binding: binding.binding,
                    descriptor_type: binding.descriptor_type,
                    stage_flags: binding.stage_flags,
                    uniform_upload_index: (binding.descriptor_type
                        == vk::DescriptorType::UNIFORM_BUFFER)
                        .then(|| {
                            if binding.binding == ash_mtoon_scene_binding() {
                                scene_uniform_upload_index
                            } else {
                                pipeline_plan_index
                            }
                        }),
                    texture_upload_index: binding
                        .texture
                        .and_then(|texture| texture_indices.get(&texture).copied()),
                    sampler: binding.sampler,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let pipeline_indices = mtoon_base_pipeline_indices(&plan.mtoon_pipelines);
    let descriptor_indices = descriptor_set_indices(&descriptor_sets);
    let pipelines = plan
        .mtoon_pipelines
        .iter()
        .enumerate()
        .filter_map(|(pipeline_plan_index, pipeline)| {
            descriptor_indices
                .get(&(pipeline.material, pipeline_plan_index))
                .copied()
                .map(|descriptor_set_index| AshGraphicsPipelinePlan {
                    material: pipeline.material,
                    pipeline_plan_index,
                    descriptor_set_index,
                    key: pipeline.key,
                    vertex_stride: std::mem::size_of::<AshVrmVertex>() as u32,
                    vertex_attributes: ash_vrm_vertex_attributes(),
                    color_format: vk::Format::R8G8B8A8_UNORM,
                    depth_format: Some(vk::Format::D32_SFLOAT),
                })
        })
        .collect::<Vec<_>>();
    let mut buffers = Vec::with_capacity(plan.primitives.len() * 2);
    let mut draw_calls = Vec::with_capacity(plan.primitives.len());
    for (primitive_index, primitive) in plan.primitives.iter().enumerate() {
        let vertex_buffer_index = buffers.len();
        buffers.push(AshBufferUpload {
            role: AshBufferRole::Vertex,
            usage: vk::BufferUsageFlags::VERTEX_BUFFER,
            stride: std::mem::size_of::<AshVrmVertex>() as u32,
            count: primitive.vertices.len() as u32,
            bytes: bytemuck::cast_slice(&primitive.vertices).to_vec(),
        });
        let index_buffer_index = buffers.len();
        buffers.push(AshBufferUpload {
            role: AshBufferRole::Index,
            usage: vk::BufferUsageFlags::INDEX_BUFFER,
            stride: std::mem::size_of::<u32>() as u32,
            count: primitive.indices.len() as u32,
            bytes: bytemuck::cast_slice(&primitive.indices).to_vec(),
        });
        let pipeline_plan_index = primitive
            .material
            .and_then(|material| pipeline_indices.get(&material).copied());
        let descriptor_set_index = pipeline_plan_index.and_then(|index| {
            primitive
                .material
                .and_then(|material| descriptor_indices.get(&(material, index)).copied())
        });
        let (render_order, phase_order) = pipeline_plan_index
            .and_then(|index| plan.mtoon_pipelines.get(index))
            .map(|pipeline| (pipeline.key.render_order, pipeline.key.phase_order))
            .unwrap_or((2000, 2000));
        draw_calls.push(AshDrawCallPlan {
            primitive_index,
            material: primitive.material,
            pipeline_plan_index,
            descriptor_set_index,
            vertex_buffer_index,
            index_buffer_index,
            index_count: primitive.indices.len() as u32,
            render_order,
            phase_order,
        });
    }
    draw_calls.sort_by_key(|draw| (draw.render_order, draw.phase_order, draw.primitive_index));
    AshRendererFrame {
        buffers,
        textures: plan
            .texture_uploads
            .iter()
            .cloned()
            .map(|upload| AshTextureResourcePlan {
                upload,
                image_usage: vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
                image_layout_after_upload: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                aspect_mask: vk::ImageAspectFlags::COLOR,
            })
            .collect(),
        uniforms: plan
            .mtoon_pipelines
            .iter()
            .enumerate()
            .map(|(pipeline_plan_index, pipeline)| AshUniformUpload {
                scope: AshUniformScope::material(pipeline.material, pipeline_plan_index),
                binding: ash_mtoon_uniform_binding(),
                bytes: pipeline.uniform.bytes().to_vec(),
            })
            .chain(std::iter::once(AshUniformUpload {
                scope: AshUniformScope::Scene,
                binding: ash_mtoon_scene_binding(),
                bytes: plan.scene_uniform.bytes().to_vec(),
            }))
            .collect(),
        pipelines,
        descriptor_sets,
        draw_calls,
    }
}

pub fn ash_vrm_vertex_attributes() -> Vec<AshVertexAttributePlan> {
    vec![
        AshVertexAttributePlan {
            location: 0,
            binding: 0,
            format: vk::Format::R32G32B32_SFLOAT,
            offset: 0,
        },
        AshVertexAttributePlan {
            location: 1,
            binding: 0,
            format: vk::Format::R32G32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, tex_coord_0) as u32,
        },
        AshVertexAttributePlan {
            location: 2,
            binding: 0,
            format: vk::Format::R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, color_0) as u32,
        },
        AshVertexAttributePlan {
            location: 3,
            binding: 0,
            format: vk::Format::R32G32B32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, normal) as u32,
        },
        AshVertexAttributePlan {
            location: 4,
            binding: 0,
            format: vk::Format::R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(AshVrmVertex, tangent) as u32,
        },
    ]
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
        let mtoon_pipelines = self.mtoon_pipeline_plans();
        let texture_uploads = self.texture_uploads(&mtoon_pipelines);
        let texture_upload_indices = texture_ref_upload_indices(&texture_uploads);
        Ok(AshVrmFramePlan {
            primitives: self.bake_primitives()?,
            materials: self.material_records(&texture_upload_indices),
            texture_uploads,
            mtoon_pipelines,
            scene_uniform: AshSceneUniform::default(),
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
                    normal: vertex.normal.to_array(),
                    tangent: vertex.tangent.to_array(),
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

    fn material_records(
        &self,
        texture_uploads: &HashMap<TextureRef, usize>,
    ) -> Vec<AshMaterialRecord> {
        self.loaded
            .gltf_materials
            .iter()
            .enumerate()
            .map(|(index, material)| AshMaterialRecord {
                material: MaterialRef(index),
                base_color_factor: material.base_color_factor,
                base_color_texture_upload: self
                    .loaded
                    .material_texture_slots(Some(index))
                    .base
                    .and_then(|texture| texture_uploads.get(&TextureRef(texture)).copied()),
            })
            .collect()
    }

    fn texture_uploads(&self, pipelines: &[AshMtoonPipelinePlan]) -> Vec<AshTextureUpload> {
        required_texture_refs(&self.loaded, pipelines)
            .into_iter()
            .filter_map(|texture| {
                self.loaded
                    .texture_rgba8_image(texture.0)
                    .map(|image| texture_upload(Some(texture), image))
            })
            .collect()
    }

    fn mtoon_pipeline_plans(&self) -> Vec<AshMtoonPipelinePlan> {
        mtoon_renderer_material_plans(
            self.loaded.model().document(),
            MtoonMaterializationOptions::default(),
        )
        .into_iter()
        .map(|plan| {
            let gpu = MtoonGpuMaterial::from_renderer_plan(&plan);
            AshMtoonPipelinePlan {
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
                descriptor_bindings: descriptor_bindings(&gpu.textures),
                uniform: gpu.uniform,
                uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
                alpha_cutoff: plan.shader.cutoff_factor,
                outline_width: plan.shader.outline_width_factor,
                base_color_factor: plan.shader.base_color_factor,
                emissive_color: plan.shader.emissive_color,
            }
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

fn required_texture_refs(
    loaded: &LoadedVrm,
    pipelines: &[AshMtoonPipelinePlan],
) -> Vec<TextureRef> {
    let mut textures = Vec::new();
    for material in 0..loaded.gltf_materials.len() {
        if let Some(texture) = loaded.material_texture_slots(Some(material)).base {
            push_unique_texture(&mut textures, TextureRef(texture));
        }
    }
    for pipeline in pipelines {
        for texture in pipeline
            .descriptor_bindings
            .iter()
            .filter_map(|binding| binding.texture)
        {
            push_unique_texture(&mut textures, texture);
        }
    }
    textures
}

fn push_unique_texture(textures: &mut Vec<TextureRef>, texture: TextureRef) {
    if !textures.contains(&texture) {
        textures.push(texture);
    }
}

fn texture_ref_upload_indices(texture_uploads: &[AshTextureUpload]) -> HashMap<TextureRef, usize> {
    texture_uploads
        .iter()
        .enumerate()
        .filter_map(|(upload_index, upload)| upload.texture.map(|texture| (texture, upload_index)))
        .collect()
}

fn mtoon_base_pipeline_indices(pipelines: &[AshMtoonPipelinePlan]) -> HashMap<MaterialRef, usize> {
    pipelines
        .iter()
        .enumerate()
        .filter(|(_, pipeline)| pipeline.key.pass == AshMtoonPass::Base)
        .map(|(index, pipeline)| (pipeline.material, index))
        .collect()
}

fn descriptor_set_indices(
    descriptor_sets: &[AshDescriptorSetPlan],
) -> HashMap<(MaterialRef, usize), usize> {
    descriptor_sets
        .iter()
        .enumerate()
        .map(|(index, set)| ((set.material, set.pipeline_plan_index), index))
        .collect()
}

fn cull_mode(mode: MtoonCullMode) -> vk::CullModeFlags {
    match mode {
        MtoonCullMode::Off => vk::CullModeFlags::NONE,
        MtoonCullMode::Front => vk::CullModeFlags::FRONT,
        MtoonCullMode::Back => vk::CullModeFlags::BACK,
    }
}

pub const fn ash_mtoon_uniform_binding() -> u32 {
    0
}

pub const fn ash_mtoon_scene_binding() -> u32 {
    9
}

pub const fn ash_mtoon_texture_binding(slot: MtoonTextureSlot) -> u32 {
    match slot {
        MtoonTextureSlot::Main => 1,
        MtoonTextureSlot::ShadeMultiply => 2,
        MtoonTextureSlot::ShadingShift => 3,
        MtoonTextureSlot::Normal => 4,
        MtoonTextureSlot::Matcap => 5,
        MtoonTextureSlot::RimMultiply => 6,
        MtoonTextureSlot::OutlineWidth => 7,
        MtoonTextureSlot::UvAnimationMask => 8,
    }
}

fn descriptor_bindings(textures: &MtoonRendererTextureRefs) -> Vec<AshDescriptorBindingPlan> {
    let mut result = Vec::with_capacity(ASH_MTOON_TEXTURE_SLOTS.len() + 2);
    result.push(AshDescriptorBindingPlan {
        binding: ash_mtoon_uniform_binding(),
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        sampler: None,
    });
    result.extend(
        ASH_MTOON_TEXTURE_SLOTS
            .iter()
            .copied()
            .map(|slot| AshDescriptorBindingPlan {
                binding: ash_mtoon_texture_binding(slot),
                descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                stage_flags: vk::ShaderStageFlags::FRAGMENT,
                texture: texture_for_slot(textures, slot),
                sampler: Some(sampler_plan(sampler_hint_for_slot(slot))),
            }),
    );
    result.push(AshDescriptorBindingPlan {
        binding: ash_mtoon_scene_binding(),
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        texture: None,
        sampler: None,
    });
    result
}

const ASH_MTOON_TEXTURE_SLOTS: [MtoonTextureSlot; 8] = [
    MtoonTextureSlot::Main,
    MtoonTextureSlot::ShadeMultiply,
    MtoonTextureSlot::ShadingShift,
    MtoonTextureSlot::Normal,
    MtoonTextureSlot::Matcap,
    MtoonTextureSlot::RimMultiply,
    MtoonTextureSlot::OutlineWidth,
    MtoonTextureSlot::UvAnimationMask,
];

fn texture_for_slot(
    textures: &MtoonRendererTextureRefs,
    slot: MtoonTextureSlot,
) -> Option<TextureRef> {
    match slot {
        MtoonTextureSlot::Main => textures.main,
        MtoonTextureSlot::ShadeMultiply => textures.shade_multiply,
        MtoonTextureSlot::ShadingShift => textures.shading_shift,
        MtoonTextureSlot::Normal => textures.normal,
        MtoonTextureSlot::Matcap => textures.matcap,
        MtoonTextureSlot::RimMultiply => textures.rim_multiply,
        MtoonTextureSlot::OutlineWidth => textures.outline_width,
        MtoonTextureSlot::UvAnimationMask => textures.uv_animation_mask,
    }
}

fn sampler_hint_for_slot(slot: MtoonTextureSlot) -> MtoonSamplerHint {
    match slot {
        MtoonTextureSlot::Normal => MtoonSamplerHint::NormalMapLinearRepeat,
        MtoonTextureSlot::Main
        | MtoonTextureSlot::ShadeMultiply
        | MtoonTextureSlot::ShadingShift
        | MtoonTextureSlot::Matcap
        | MtoonTextureSlot::RimMultiply
        | MtoonTextureSlot::OutlineWidth
        | MtoonTextureSlot::UvAnimationMask => MtoonSamplerHint::LinearRepeat,
    }
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
        let bindings = descriptor_bindings(&MtoonRendererTextureRefs::default());
        assert_eq!(bindings.len(), 10);
        assert_eq!(bindings[0].binding, ash_mtoon_uniform_binding());
        assert_eq!(
            bindings[0].descriptor_type,
            vk::DescriptorType::UNIFORM_BUFFER
        );
        assert_eq!(
            bindings[1].binding,
            ash_mtoon_texture_binding(MtoonTextureSlot::Main)
        );
        assert_eq!(bindings[1].texture, None);
        assert_eq!(
            bindings[4].binding,
            ash_mtoon_texture_binding(MtoonTextureSlot::Normal)
        );
        assert!(bindings[4].sampler.unwrap().normal_map_decode);
        assert_eq!(bindings[9].binding, ash_mtoon_scene_binding());
        assert_eq!(
            bindings[9].descriptor_type,
            vk::DescriptorType::UNIFORM_BUFFER
        );
    }

    #[test]
    fn scene_uniform_exposes_stable_camera_light_abi() {
        let uniform = AshSceneUniform::parity_camera(1.0);
        assert_eq!(std::mem::size_of::<AshSceneUniform>(), 256);
        assert_eq!(uniform.bytes().len(), 256);
        assert_eq!(uniform.camera_pos, [0.0, 1.4, -4.0, 1.0]);
        assert_eq!(uniform.light_color, [1.0, 1.0, 1.0, 0.0]);
        assert_ne!(uniform.view_projection, Mat4::IDENTITY.to_cols_array_2d());
    }

    #[test]
    fn source_mtoon_shader_matches_rust_binding_contract() {
        let vertex_shader = include_str!("../shaders/mtoon_base.vert.glsl");
        let fragment_shader = include_str!("../shaders/mtoon_base.frag.glsl");

        for (slot, expected_name) in [
            (MtoonTextureSlot::Main, "main_texture"),
            (MtoonTextureSlot::ShadeMultiply, "shade_multiply_texture"),
            (MtoonTextureSlot::ShadingShift, "shading_shift_texture"),
            (MtoonTextureSlot::Normal, "normal_texture"),
            (MtoonTextureSlot::Matcap, "matcap_texture"),
            (MtoonTextureSlot::RimMultiply, "rim_multiply_texture"),
            (MtoonTextureSlot::OutlineWidth, "outline_width_texture"),
            (
                MtoonTextureSlot::UvAnimationMask,
                "uv_animation_mask_texture",
            ),
        ] {
            let declaration = format!(
                "layout(set = 0, binding = {}) uniform sampler2D {expected_name};",
                ash_mtoon_texture_binding(slot)
            );
            assert!(fragment_shader.contains(&declaration));
        }

        assert!(fragment_shader.contains("layout(set = 0, binding = 0, std140)"));
        assert!(fragment_shader.contains("layout(set = 0, binding = 9, std140)"));
        assert!(fragment_shader.contains("mtoon.flags.z == 1u"));
        assert!(vertex_shader.contains("layout(set = 0, binding = 9, std140)"));
        assert!(vertex_shader.contains("layout(location = 3) in vec3 in_normal;"));
        assert!(vertex_shader.contains("layout(location = 4) in vec4 in_tangent;"));
    }

    #[test]
    fn renderer_frame_builds_buffers_and_sorted_draw_calls() {
        let plan = AshVrmFramePlan {
            primitives: vec![AshVrmPrimitive {
                node: NodeRef(0),
                material: Some(MaterialRef(0)),
                vertices: vec![AshVrmVertex {
                    position: [0.0, 0.0, 0.0],
                    tex_coord_0: [0.0, 0.0],
                    color_0: [1.0, 1.0, 1.0, 1.0],
                    normal: [0.0, 0.0, 1.0],
                    tangent: [1.0, 0.0, 0.0, 1.0],
                }],
                indices: vec![0],
            }],
            materials: vec![AshMaterialRecord {
                material: MaterialRef(0),
                base_color_factor: [1.0, 1.0, 1.0, 1.0],
                base_color_texture_upload: None,
            }],
            texture_uploads: Vec::new(),
            mtoon_pipelines: vec![AshMtoonPipelinePlan {
                material: MaterialRef(0),
                name: Some("mat".to_owned()),
                key: AshPipelineKey {
                    pass: AshMtoonPass::Base,
                    render_order: 2000,
                    phase_order: 2000,
                    topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                    cull_mode: vk::CullModeFlags::BACK,
                    front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                    depth_test_enable: true,
                    depth_write_enable: true,
                    depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                    blend_enable: false,
                },
                descriptor_bindings: descriptor_bindings(&MtoonRendererTextureRefs::default()),
                uniform: MtoonGpuUniform::zeroed(),
                uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
                alpha_cutoff: 0.5,
                outline_width: 0.0,
                base_color_factor: [1.0, 1.0, 1.0, 1.0],
                emissive_color: [0.0, 0.0, 0.0],
            }],
            scene_uniform: AshSceneUniform::default(),
        };
        let renderer_frame = ash_renderer_frame_from_plan(&plan);
        assert_eq!(renderer_frame.buffers.len(), 2);
        assert_eq!(renderer_frame.uniforms.len(), 2);
        assert_eq!(renderer_frame.pipelines.len(), 1);
        assert_eq!(
            renderer_frame.uniforms[0].bytes.len(),
            MTOON_GPU_UNIFORM_SIZE
        );
        assert_eq!(
            renderer_frame.uniforms[0].scope,
            AshUniformScope::material(MaterialRef(0), 0)
        );
        assert_eq!(
            renderer_frame.uniforms[0].scope.material_ref(),
            Some(MaterialRef(0))
        );
        assert_eq!(
            renderer_frame.uniforms[0].scope.pipeline_plan_index(),
            Some(0)
        );
        assert_eq!(
            renderer_frame.uniforms[0].binding,
            ash_mtoon_uniform_binding()
        );
        assert_eq!(
            renderer_frame.uniforms[1].bytes.len(),
            std::mem::size_of::<AshSceneUniform>()
        );
        assert_eq!(renderer_frame.uniforms[1].scope, AshUniformScope::Scene);
        assert_eq!(renderer_frame.uniforms[1].scope.material_ref(), None);
        assert_eq!(renderer_frame.uniforms[1].scope.pipeline_plan_index(), None);
        assert_eq!(
            renderer_frame.uniforms[1].binding,
            ash_mtoon_scene_binding()
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[0].uniform_upload_index,
            Some(0)
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[9].uniform_upload_index,
            Some(1)
        );
        assert_eq!(
            renderer_frame.pipelines[0].vertex_attributes,
            ash_vrm_vertex_attributes()
        );
        assert_eq!(renderer_frame.pipelines[0].vertex_attributes.len(), 5);
        assert_eq!(renderer_frame.buffers[0].stride, 64);
        assert_eq!(
            renderer_frame.buffers[0].usage,
            vk::BufferUsageFlags::VERTEX_BUFFER
        );
        assert_eq!(renderer_frame.draw_calls[0].pipeline_plan_index, Some(0));
        assert_eq!(renderer_frame.draw_calls[0].descriptor_set_index, Some(0));
    }

    #[test]
    fn renderer_frame_resolves_texture_descriptor_uploads() {
        let plan = AshVrmFramePlan {
            primitives: Vec::new(),
            materials: Vec::new(),
            texture_uploads: vec![AshTextureUpload {
                texture: Some(TextureRef(7)),
                format: vk::Format::R8G8B8A8_SRGB,
                extent: vk::Extent3D {
                    width: 1,
                    height: 1,
                    depth: 1,
                },
                rgba: vec![255, 255, 255, 255],
            }],
            mtoon_pipelines: vec![AshMtoonPipelinePlan {
                material: MaterialRef(0),
                name: Some("textured".to_owned()),
                key: AshPipelineKey {
                    pass: AshMtoonPass::Base,
                    render_order: 2000,
                    phase_order: 2000,
                    topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                    cull_mode: vk::CullModeFlags::BACK,
                    front_face: vk::FrontFace::COUNTER_CLOCKWISE,
                    depth_test_enable: true,
                    depth_write_enable: true,
                    depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
                    blend_enable: false,
                },
                descriptor_bindings: vec![
                    AshDescriptorBindingPlan {
                        binding: 0,
                        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
                        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                        texture: None,
                        sampler: None,
                    },
                    AshDescriptorBindingPlan {
                        binding: 1,
                        descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                        stage_flags: vk::ShaderStageFlags::FRAGMENT,
                        texture: Some(TextureRef(7)),
                        sampler: Some(AshSamplerPlan {
                            mag_filter: vk::Filter::LINEAR,
                            min_filter: vk::Filter::LINEAR,
                            mipmap_mode: vk::SamplerMipmapMode::LINEAR,
                            address_mode_u: vk::SamplerAddressMode::REPEAT,
                            address_mode_v: vk::SamplerAddressMode::REPEAT,
                            normal_map_decode: false,
                        }),
                    },
                ],
                uniform: MtoonGpuUniform::zeroed(),
                uniform_buffer_size: MTOON_GPU_UNIFORM_SIZE as u32,
                alpha_cutoff: 0.5,
                outline_width: 0.0,
                base_color_factor: [1.0, 1.0, 1.0, 1.0],
                emissive_color: [0.0, 0.0, 0.0],
            }],
            scene_uniform: AshSceneUniform::default(),
        };
        let renderer_frame = ash_renderer_frame_from_plan(&plan);
        assert_eq!(renderer_frame.textures.len(), 1);
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[1].texture_upload_index,
            Some(0)
        );
        assert_eq!(
            renderer_frame.descriptor_sets[0].bindings[1].descriptor_type,
            vk::DescriptorType::COMBINED_IMAGE_SAMPLER
        );
    }
}
