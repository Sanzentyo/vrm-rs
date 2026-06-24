use ash::{Entry, vk};
use clap::Parser;
use imq::{
    FrameOwned, PixelFormat, RawImageBundle, RawImageRecord, decode_imqraw_bundle,
    encode_imqraw_bundle,
};
use serde_json::json;
use std::{
    error::Error,
    ffi::CString,
    fs, io,
    path::{Path, PathBuf},
    ptr,
};
use vrm_adapter::{
    MtoonLightAccumulation, MtoonLightingConfig, MtoonTextureSlot, RenderOwnerSampleCorrectionPlan,
    RenderOwnerSampleDrawKey, RenderOwnerSamplePass, RenderOwnerSampleSurfaceOverride,
    RenderOwnerSurfaceKey,
};
use vrm_adapter_ash::{
    AshColorAttachmentFinalLayout, AshCommandPlan, AshDescriptorImageResource,
    AshDescriptorSetLayoutPlan, AshDescriptorWriteResource, AshDiagnosticOwnerId,
    AshDrawableFrameOptions, AshGraphicsPipelinePlan, AshMaterialExtraUniform,
    AshMtoonLightAccumulation, AshMtoonPass, AshMtoonPipelinePlan, AshRenderPassCreationPlan,
    AshRenderPassDependencyPolicy, AshRendererFrame, AshSamplerPlan, AshVrmFramePlanOptions,
    AshVrmPrimitive, ash_depth_attachment_plan, ash_descriptor_pool_plan,
    ash_descriptor_set_allocation_plan, ash_descriptor_set_layout_plans,
    ash_descriptor_write_plans, ash_drawable_frame_from_renderer_frame_with_options,
    ash_framebuffer_plan, ash_graphics_pipeline_state_plan, ash_material_texture_binding,
    ash_mtoon_texture_binding, ash_pipeline_layout_plans, ash_reference_depth_format,
    ash_render_pass_creation_plan, ash_renderer_frame_from_plan_with_owner_sample_selection,
    frame_plan_from_options_with_viewport,
};
use vrm_io::{
    GltfAlphaMode, GltfMaterialTextureFallback, GltfMaterialTextureSlot, RgbaMipLevel,
    generate_rgba_mip_chain,
};

#[derive(Clone, Debug, Parser)]
#[command(about = "Materialize a VRM frame plan into real ash Vulkan offscreen draw resources")]
struct Options {
    #[command(flatten)]
    frame: AshVrmFramePlanOptions,
    /// Only print help/parse inputs; useful for CI smoke checks.
    #[arg(long)]
    dry_run: bool,
    /// Write and validate tiny RGBA/imqraw artifacts without opening Vulkan.
    #[arg(long)]
    artifact_self_test: bool,
    /// Offscreen framebuffer width for the drawable pipeline smoke.
    #[arg(long, default_value_t = 64)]
    width: u32,
    /// Offscreen framebuffer height for the drawable pipeline smoke.
    #[arg(long, default_value_t = 64)]
    height: u32,
    /// Clear alpha for the offscreen color target.
    #[arg(long, default_value_t = 0.0)]
    clear_alpha: f32,
    /// Submit the recorded offscreen draw and read back the color attachment.
    #[arg(long)]
    submit_readback: bool,
    /// Write the submitted/read-back offscreen color attachment as a render-parity RGBA JSON artifact.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Write the submitted/read-back offscreen color attachment as an imqraw bundle.
    #[arg(long)]
    imqraw_out: Option<PathBuf>,
    /// Load a source-like owner/sample correction manifest for renderer metadata.
    #[arg(long)]
    owner_sample_correction_manifest: Option<PathBuf>,
    /// Apply owner/sample manifest RGBA replacements to the final readback image.
    ///
    /// This is an upper-bound diagnostic, not the default renderer behavior.
    #[arg(long)]
    apply_owner_sample_readback_replacement: bool,
    /// Optional precompiled SPIR-V vertex shader for the offscreen graphics pipelines.
    ///
    /// The shader must match the example vertex input plus descriptor-set layout
    /// emitted from `AshRendererFrame`.
    #[arg(long, requires = "fragment_spv")]
    vertex_spv: Option<PathBuf>,
    /// Optional precompiled SPIR-V fragment shader for the offscreen graphics pipelines.
    ///
    /// Use together with `--vertex-spv` to replace the built-in color-only smoke
    /// shader without committing shader binaries to this repository.
    #[arg(long, requires = "vertex_spv")]
    fragment_spv: Option<PathBuf>,
    /// Entry point name for `--vertex-spv`.
    #[arg(long, default_value = "main")]
    vertex_entry: String,
    /// Entry point name for `--fragment-spv`.
    #[arg(long, default_value = "main")]
    fragment_entry: String,
}

struct VulkanFrameResources {
    buffers: Vec<VulkanBuffer>,
    images: Vec<VulkanImage>,
    texture_staging_buffers: Vec<VulkanBuffer>,
    fallback_textures: VulkanFallbackTextures,
    fallback_texture_staging: VulkanFallbackBuffers,
    uniform_buffers: Vec<VulkanBuffer>,
    samplers: Vec<vk::Sampler>,
    color_target: VulkanImage,
    depth_target: VulkanImage,
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    shader_modules: Vec<vk::ShaderModule>,
    descriptor_set_layouts: Vec<vk::DescriptorSetLayout>,
    descriptor_pool: vk::DescriptorPool,
    descriptor_sets: Vec<vk::DescriptorSet>,
    pipeline_layouts: Vec<vk::PipelineLayout>,
    pipelines: Vec<vk::Pipeline>,
    command_buffers: Vec<vk::CommandBuffer>,
    readback: VulkanBuffer,
    readback_len: usize,
    command_pool: vk::CommandPool,
    depth_format: vk::Format,
}

struct VulkanBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
}

struct VulkanImage {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

struct VulkanFallbackTextures {
    white: VulkanImage,
    black: VulkanImage,
    neutral_normal: VulkanImage,
}

impl VulkanFallbackTextures {
    fn get(&self, fallback: GltfMaterialTextureFallback) -> &VulkanImage {
        match fallback {
            GltfMaterialTextureFallback::White => &self.white,
            GltfMaterialTextureFallback::Black => &self.black,
            GltfMaterialTextureFallback::NeutralNormal => &self.neutral_normal,
        }
    }
}

impl IntoIterator for VulkanFallbackTextures {
    type IntoIter = std::array::IntoIter<VulkanImage, 3>;
    type Item = VulkanImage;

    fn into_iter(self) -> Self::IntoIter {
        [self.white, self.black, self.neutral_normal].into_iter()
    }
}

struct VulkanFallbackBuffers {
    white: VulkanBuffer,
    black: VulkanBuffer,
    neutral_normal: VulkanBuffer,
}

impl IntoIterator for VulkanFallbackBuffers {
    type IntoIter = std::array::IntoIter<VulkanBuffer, 3>;
    type Item = VulkanBuffer;

    fn into_iter(self) -> Self::IntoIter {
        [self.white, self.black, self.neutral_normal].into_iter()
    }
}

struct PipelineBuildContext<'a> {
    render_pass: vk::RenderPass,
    extent: vk::Extent2D,
    vertex_shader: vk::ShaderModule,
    fragment_shader: vk::ShaderModule,
    pipeline_layouts: &'a [vk::PipelineLayout],
    vertex_entry_point: &'a CString,
    fragment_entry_point: &'a CString,
}

struct CommandRecordContext<'a> {
    command_pool: vk::CommandPool,
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    extent: vk::Extent2D,
    pipelines: &'a [vk::Pipeline],
    pipeline_layouts: &'a [vk::PipelineLayout],
    buffers: &'a [VulkanBuffer],
    descriptor_sets: &'a [vk::DescriptorSet],
    texture_images: &'a [VulkanImage],
    texture_staging_buffers: &'a [VulkanBuffer],
    texture_mip_levels: &'a [Vec<RgbaMipLevel>],
    fallback_textures: &'a VulkanFallbackTextures,
    fallback_texture_staging: &'a VulkanFallbackBuffers,
    color_target: vk::Image,
    readback_buffer: vk::Buffer,
    clear_alpha: f32,
}

#[derive(Clone, Copy)]
struct DescriptorUpdateResources<'a> {
    buffers: &'a [VulkanBuffer],
    uniform_buffers: &'a [VulkanBuffer],
    images: &'a [VulkanImage],
    fallback_textures: &'a VulkanFallbackTextures,
    samplers: &'a [vk::Sampler],
}

fn descriptor_image_resource<'a>(
    resources: DescriptorUpdateResources<'a>,
    image: AshDescriptorImageResource,
) -> Result<&'a VulkanImage, Box<dyn Error>> {
    match image {
        AshDescriptorImageResource::TextureUpload {
            texture_upload_index,
        } => resources
            .images
            .get(texture_upload_index)
            .ok_or_else(|| "descriptor write references a missing texture image".into()),
        AshDescriptorImageResource::Fallback { fallback } => {
            Ok(resources.fallback_textures.get(fallback))
        }
    }
}

#[derive(Clone, Debug)]
struct ReadbackFrame {
    rgba: Vec<u8>,
    checksum: u64,
    nonzero_pixels: usize,
}

impl ReadbackFrame {
    fn from_rgba(rgba: Vec<u8>) -> Self {
        let checksum = fnv1a64(&rgba);
        let nonzero_pixels = rgba
            .chunks_exact(4)
            .filter(|pixel| pixel.iter().any(|channel| *channel != 0))
            .count();
        Self {
            rgba,
            checksum,
            nonzero_pixels,
        }
    }

    fn byte_len(&self) -> usize {
        self.rgba.len()
    }

    fn apply_owner_sample_correction_plan(
        &mut self,
        plan: &RenderOwnerSampleCorrectionPlan,
        width: u32,
        height: u32,
    ) -> Result<usize, Box<dyn Error>> {
        let applied = plan.apply_rgba8(u64::from(width), u64::from(height), &mut self.rgba)?;
        *self = Self::from_rgba(std::mem::take(&mut self.rgba));
        Ok(applied)
    }
}

#[derive(Clone, Debug)]
struct ShaderModuleSources {
    vertex: Vec<u32>,
    fragment: Vec<u32>,
    vertex_entry: String,
    fragment_entry: String,
    source: ShaderSourceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShaderSourceKind {
    BuiltInSmoke,
    ExternalSpirv,
}

const MINIMAL_VERTEX_SPV: &[u32] = &[
    119734787, 65536, 851979, 39, 0, 131089, 1, 393227, 1, 1280527431, 1685353262, 808793134, 0,
    196622, 0, 1, 655375, 0, 4, 1852399981, 0, 13, 18, 33, 35, 38, 196611, 2, 450, 655364,
    1197427783, 1279741775, 1885560645, 1953718128, 1600482425, 1701734764, 1919509599, 1769235301,
    25974, 524292, 1197427783, 1279741775, 1852399429, 1685417059, 1768185701, 1952671090, 6649449,
    262149, 4, 1852399981, 0, 393221, 11, 1348430951, 1700164197, 2019914866, 0, 393222, 11, 0,
    1348430951, 1953067887, 7237481, 458758, 11, 1, 1348430951, 1953393007, 1702521171, 0, 458758,
    11, 2, 1130327143, 1148217708, 1635021673, 6644590, 458758, 11, 3, 1130327143, 1147956341,
    1635021673, 6644590, 196613, 13, 0, 327685, 18, 1885302377, 1953067887, 7237481, 327685, 33,
    1601467759, 1869377379, 114, 327685, 35, 1667198569, 1919904879, 0, 262149, 38, 1969188457,
    118, 196679, 11, 2, 327752, 11, 0, 11, 0, 327752, 11, 1, 11, 1, 327752, 11, 2, 11, 3, 327752,
    11, 3, 11, 4, 262215, 18, 30, 0, 262215, 33, 30, 0, 262215, 35, 30, 2, 262215, 38, 30, 1,
    131091, 2, 196641, 3, 2, 196630, 6, 32, 262167, 7, 6, 4, 262165, 8, 32, 0, 262187, 8, 9, 1,
    262172, 10, 6, 9, 393246, 11, 7, 6, 10, 10, 262176, 12, 3, 11, 262203, 12, 13, 3, 262165, 14,
    32, 1, 262187, 14, 15, 0, 262167, 16, 6, 3, 262176, 17, 1, 16, 262203, 17, 18, 1, 262167, 19,
    6, 2, 262187, 6, 22, 1056964608, 262187, 6, 23, 3204448256, 327724, 19, 24, 22, 23, 262187, 6,
    26, 0, 262187, 6, 27, 1065353216, 262176, 31, 3, 7, 262203, 31, 33, 3, 262176, 34, 1, 7,
    262203, 34, 35, 1, 262176, 37, 1, 19, 262203, 37, 38, 1, 327734, 2, 4, 0, 3, 131320, 5, 262205,
    16, 20, 18, 458831, 19, 21, 20, 20, 0, 1, 327813, 19, 25, 21, 24, 327761, 6, 28, 25, 0, 327761,
    6, 29, 25, 1, 458832, 7, 30, 28, 29, 26, 27, 327745, 31, 32, 13, 15, 196670, 32, 30, 262205, 7,
    36, 35, 196670, 33, 36, 65789, 65592,
];

const MINIMAL_FRAGMENT_SPV: &[u32] = &[
    119734787, 65536, 851979, 13, 0, 131089, 1, 393227, 1, 1280527431, 1685353262, 808793134, 0,
    196622, 0, 1, 458767, 4, 4, 1852399981, 0, 9, 11, 196624, 4, 7, 196611, 2, 450, 655364,
    1197427783, 1279741775, 1885560645, 1953718128, 1600482425, 1701734764, 1919509599, 1769235301,
    25974, 524292, 1197427783, 1279741775, 1852399429, 1685417059, 1768185701, 1952671090, 6649449,
    262149, 4, 1852399981, 0, 327685, 9, 1601467759, 1869377379, 114, 327685, 11, 1667198569,
    1919904879, 0, 262215, 9, 30, 0, 262215, 11, 30, 0, 131091, 2, 196641, 3, 2, 196630, 6, 32,
    262167, 7, 6, 4, 262176, 8, 3, 7, 262203, 8, 9, 3, 262176, 10, 1, 7, 262203, 10, 11, 1, 327734,
    2, 4, 0, 3, 131320, 5, 262205, 7, 12, 11, 196670, 9, 12, 65789, 65592,
];

struct UnsafeAshDeviceRenderer {
    _entry: Entry,
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    queue_family_index: u32,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
}

impl UnsafeAshDeviceRenderer {
    fn new() -> Result<Self, Box<dyn Error>> {
        let app_name = CString::new("vrm-rs unsafe ash renderer example")?;
        let engine_name = CString::new("vrm-rs")?;
        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(1)
            .engine_name(&engine_name)
            .engine_version(1)
            .api_version(vk::API_VERSION_1_0);
        let instance_info = vk::InstanceCreateInfo::default().application_info(&app_info);

        let entry = unsafe { Entry::load()? };
        let instance = unsafe { entry.create_instance(&instance_info, None)? };
        let physical_device = unsafe { instance.enumerate_physical_devices()? }
            .into_iter()
            .next()
            .ok_or("no Vulkan physical device found")?;
        let queue_family_index = unsafe {
            instance
                .get_physical_device_queue_family_properties(physical_device)
                .iter()
                .enumerate()
                .find_map(|(index, family)| {
                    family
                        .queue_flags
                        .contains(vk::QueueFlags::GRAPHICS)
                        .then_some(index as u32)
                })
                .ok_or("no graphics queue family found")?
        };
        let queue_priorities = [1.0_f32];
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&queue_priorities)];
        let device_info = vk::DeviceCreateInfo::default().queue_create_infos(&queue_info);
        let device = unsafe { instance.create_device(physical_device, &device_info, None)? };
        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };
        Ok(Self {
            _entry: entry,
            instance,
            physical_device,
            device,
            queue_family_index,
            memory_properties,
        })
    }

    fn materialize_frame(
        &self,
        frame: &AshRendererFrame,
        extent: vk::Extent2D,
        shaders: &ShaderModuleSources,
        clear_alpha: f32,
    ) -> Result<VulkanFrameResources, Box<dyn Error>> {
        let command_pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = unsafe { self.device.create_command_pool(&command_pool_info, None)? };

        let buffers = frame
            .buffers
            .iter()
            .map(|buffer| self.create_upload_buffer(buffer.usage, &buffer.bytes))
            .collect::<Result<Vec<_>, _>>()?;
        let texture_mip_levels = frame
            .textures
            .iter()
            .map(|texture| {
                generate_rgba_mip_chain(
                    texture.upload.extent.width,
                    texture.upload.extent.height,
                    &texture.upload.rgba,
                )
                .map_err(|err| format!("failed to build ash texture mip chain: {err}").into())
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        let images = frame
            .textures
            .iter()
            .zip(&texture_mip_levels)
            .map(|(texture, mip_levels)| {
                self.create_image(
                    texture.upload.format,
                    texture.upload.extent,
                    u32::try_from(mip_levels.len()).unwrap_or(1),
                    texture.image_usage,
                    vk::ImageAspectFlags::COLOR,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let texture_staging_buffers = frame
            .textures
            .iter()
            .zip(&texture_mip_levels)
            .map(|(_, mip_levels)| {
                self.create_host_buffer(
                    vk::BufferUsageFlags::TRANSFER_SRC,
                    &flatten_mip_level_rgba(mip_levels),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fallback_textures = self.create_fallback_textures()?;
        let fallback_texture_staging = self.create_fallback_staging_buffers()?;
        let uniform_buffers = frame
            .uniforms
            .iter()
            .map(|uniform| {
                self.create_host_buffer(vk::BufferUsageFlags::UNIFORM_BUFFER, &uniform.bytes)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let color_format = vk::Format::R8G8B8A8_UNORM;
        let depth_format = self.select_depth_format()?;
        let depth_plan = ash_depth_attachment_plan(depth_format, extent);
        let color_target = self.create_image(
            color_format,
            vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            },
            1,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
            vk::ImageAspectFlags::COLOR,
        )?;
        let depth_target = self.create_image(
            depth_plan.format,
            depth_plan.extent,
            1,
            depth_plan.image_usage,
            depth_plan.aspect_mask,
        )?;
        let readback_len = extent.width as usize * extent.height as usize * 4;
        let readback = self.create_host_buffer(
            vk::BufferUsageFlags::TRANSFER_DST,
            &vec![0_u8; readback_len],
        )?;
        let descriptor_set_layout_plans = ash_descriptor_set_layout_plans(frame);
        let descriptor_set_layouts = descriptor_set_layout_plans
            .iter()
            .map(|plan| self.create_descriptor_set_layout(plan))
            .collect::<Result<Vec<_>, _>>()?;
        let descriptor_pool = self.create_descriptor_pool(frame)?;
        let descriptor_sets = self.allocate_descriptor_sets(
            descriptor_pool,
            &ash_descriptor_set_allocation_plan(&descriptor_set_layout_plans)
                .vk_set_layouts(&descriptor_set_layouts)
                .map_err(io::Error::other)?,
        )?;
        let samplers = self.create_samplers(frame)?;
        self.update_descriptor_sets(
            frame,
            &descriptor_sets,
            DescriptorUpdateResources {
                buffers: &buffers,
                uniform_buffers: &uniform_buffers,
                images: &images,
                fallback_textures: &fallback_textures,
                samplers: &samplers,
            },
        )?;
        let pipeline_layouts = ash_pipeline_layout_plans(&descriptor_set_layout_plans)
            .iter()
            .map(|plan| {
                let layouts = plan
                    .vk_set_layouts(&descriptor_set_layouts)
                    .map_err(io::Error::other)?;
                let info = vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts);
                unsafe { self.device.create_pipeline_layout(&info, None) }.map_err(Into::into)
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        let render_pass = self.create_render_pass(ash_render_pass_creation_plan(
            color_format,
            depth_format,
            AshColorAttachmentFinalLayout::ColorAttachment,
            AshRenderPassDependencyPolicy::ColorAndDepth,
        ))?;
        let framebuffer =
            self.create_framebuffer(render_pass, color_target.view, depth_target.view, extent)?;
        let vertex_shader = self.create_shader_module(&shaders.vertex)?;
        let fragment_shader = self.create_shader_module(&shaders.fragment)?;
        let shader_modules = vec![vertex_shader, fragment_shader];
        let vertex_entry_point = CString::new(shaders.vertex_entry.as_str())?;
        let fragment_entry_point = CString::new(shaders.fragment_entry.as_str())?;
        let pipeline_context = PipelineBuildContext {
            render_pass,
            extent,
            vertex_shader,
            fragment_shader,
            pipeline_layouts: &pipeline_layouts,
            vertex_entry_point: &vertex_entry_point,
            fragment_entry_point: &fragment_entry_point,
        };
        let pipelines = self.create_graphics_pipelines(frame, &pipeline_context)?;
        let command_context = CommandRecordContext {
            command_pool,
            render_pass,
            framebuffer,
            extent,
            pipelines: &pipelines,
            pipeline_layouts: &pipeline_layouts,
            buffers: &buffers,
            descriptor_sets: &descriptor_sets,
            texture_images: &images,
            texture_staging_buffers: &texture_staging_buffers,
            texture_mip_levels: &texture_mip_levels,
            fallback_textures: &fallback_textures,
            fallback_texture_staging: &fallback_texture_staging,
            color_target: color_target.image,
            readback_buffer: readback.buffer,
            clear_alpha,
        };
        let command_buffers = self.record_command_buffers(frame, &command_context)?;
        Ok(VulkanFrameResources {
            buffers,
            images,
            texture_staging_buffers,
            fallback_textures,
            fallback_texture_staging,
            uniform_buffers,
            samplers,
            color_target,
            depth_target,
            render_pass,
            framebuffer,
            shader_modules,
            descriptor_set_layouts,
            descriptor_pool,
            descriptor_sets,
            pipeline_layouts,
            pipelines,
            command_buffers,
            readback,
            readback_len,
            command_pool,
            depth_format,
        })
    }

    fn create_host_buffer(
        &self,
        usage: vk::BufferUsageFlags,
        bytes: &[u8],
    ) -> Result<VulkanBuffer, Box<dyn Error>> {
        let size = bytes.len().max(1) as vk::DeviceSize;
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage | vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe { self.device.create_buffer(&info, None)? };
        let requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type_index = self.find_memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let allocate_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        let memory = unsafe { self.device.allocate_memory(&allocate_info, None)? };
        unsafe {
            self.device.bind_buffer_memory(buffer, memory, 0)?;
            if !bytes.is_empty() {
                let mapped =
                    self.device
                        .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())?;
                ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
                self.device.unmap_memory(memory);
            }
        }
        Ok(VulkanBuffer { buffer, memory })
    }

    fn create_upload_buffer(
        &self,
        usage: vk::BufferUsageFlags,
        bytes: &[u8],
    ) -> Result<VulkanBuffer, Box<dyn Error>> {
        self.create_host_buffer(usage | vk::BufferUsageFlags::TRANSFER_DST, bytes)
    }

    fn create_image(
        &self,
        format: vk::Format,
        extent: vk::Extent3D,
        mip_levels: u32,
        usage: vk::ImageUsageFlags,
        aspect_mask: vk::ImageAspectFlags,
    ) -> Result<VulkanImage, Box<dyn Error>> {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(extent)
            .mip_levels(mip_levels.max(1))
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { self.device.create_image(&image_info, None)? };
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type_index = self.find_memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let allocate_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        let memory = unsafe { self.device.allocate_memory(&allocate_info, None)? };
        unsafe {
            self.device.bind_image_memory(image, memory, 0)?;
        }
        let subresource_range = vk::ImageSubresourceRange::default()
            .aspect_mask(aspect_mask)
            .level_count(mip_levels.max(1))
            .layer_count(1);
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(subresource_range);
        let view = unsafe { self.device.create_image_view(&view_info, None)? };
        Ok(VulkanImage {
            image,
            memory,
            view,
        })
    }

    fn select_depth_format(&self) -> Result<vk::Format, Box<dyn Error>> {
        [
            ash_reference_depth_format(),
            vk::Format::X8_D24_UNORM_PACK32,
            vk::Format::D32_SFLOAT,
        ]
        .into_iter()
        .find(|format| {
            let properties = unsafe {
                self.instance
                    .get_physical_device_format_properties(self.physical_device, *format)
            };
            properties
                .optimal_tiling_features
                .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT)
        })
        .ok_or_else(|| "no supported Vulkan depth attachment format found".into())
    }

    fn create_fallback_textures(&self) -> Result<VulkanFallbackTextures, Box<dyn Error>> {
        Ok(VulkanFallbackTextures {
            white: self.create_fallback_texture_image()?,
            black: self.create_fallback_texture_image()?,
            neutral_normal: self.create_fallback_texture_image()?,
        })
    }

    fn create_fallback_texture_image(&self) -> Result<VulkanImage, Box<dyn Error>> {
        self.create_image(
            vk::Format::R8G8B8A8_UNORM,
            vk::Extent3D {
                width: 1,
                height: 1,
                depth: 1,
            },
            1,
            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
            vk::ImageAspectFlags::COLOR,
        )
    }

    fn create_fallback_staging_buffers(&self) -> Result<VulkanFallbackBuffers, Box<dyn Error>> {
        Ok(VulkanFallbackBuffers {
            white: self.create_fallback_staging_buffer(GltfMaterialTextureFallback::White)?,
            black: self.create_fallback_staging_buffer(GltfMaterialTextureFallback::Black)?,
            neutral_normal: self
                .create_fallback_staging_buffer(GltfMaterialTextureFallback::NeutralNormal)?,
        })
    }

    fn create_fallback_staging_buffer(
        &self,
        fallback: GltfMaterialTextureFallback,
    ) -> Result<VulkanBuffer, Box<dyn Error>> {
        self.create_host_buffer(vk::BufferUsageFlags::TRANSFER_SRC, fallback_rgba(fallback))
    }

    fn create_descriptor_set_layout(
        &self,
        plan: &AshDescriptorSetLayoutPlan,
    ) -> Result<vk::DescriptorSetLayout, vk::Result> {
        let bindings = plan.vk_bindings();
        let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        unsafe { self.device.create_descriptor_set_layout(&info, None) }
    }

    fn create_descriptor_pool(
        &self,
        frame: &AshRendererFrame,
    ) -> Result<vk::DescriptorPool, vk::Result> {
        let plan = ash_descriptor_pool_plan(frame);
        let sizes = plan.vk_pool_sizes();
        let info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(plan.max_sets)
            .pool_sizes(&sizes);
        unsafe { self.device.create_descriptor_pool(&info, None) }
    }

    fn allocate_descriptor_sets(
        &self,
        pool: vk::DescriptorPool,
        layouts: &[vk::DescriptorSetLayout],
    ) -> Result<Vec<vk::DescriptorSet>, vk::Result> {
        let info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(layouts);
        unsafe { self.device.allocate_descriptor_sets(&info) }
    }

    fn create_samplers(&self, frame: &AshRendererFrame) -> Result<Vec<vk::Sampler>, vk::Result> {
        frame
            .descriptor_sets
            .iter()
            .flat_map(|set| &set.bindings)
            .filter(|binding| {
                matches!(
                    binding.descriptor_type,
                    vk::DescriptorType::COMBINED_IMAGE_SAMPLER | vk::DescriptorType::SAMPLER
                )
            })
            .map(|binding| self.create_sampler(binding.sampler.unwrap_or(default_sampler_plan())))
            .collect()
    }

    fn create_sampler(&self, plan: AshSamplerPlan) -> Result<vk::Sampler, vk::Result> {
        let info = vk::SamplerCreateInfo::default()
            .mag_filter(plan.mag_filter)
            .min_filter(plan.min_filter)
            .mipmap_mode(plan.mipmap_mode)
            .address_mode_u(plan.address_mode_u)
            .address_mode_v(plan.address_mode_v)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .min_lod(plan.min_lod)
            .max_lod(plan.max_lod);
        unsafe { self.device.create_sampler(&info, None) }
    }

    fn update_descriptor_sets(
        &self,
        frame: &AshRendererFrame,
        descriptor_sets: &[vk::DescriptorSet],
        resources: DescriptorUpdateResources<'_>,
    ) -> Result<(), Box<dyn Error>> {
        for plan in ash_descriptor_write_plans(frame)? {
            let descriptor_set = plan
                .vk_descriptor_set(descriptor_sets)
                .map_err(io::Error::other)?;
            match &plan.resource {
                AshDescriptorWriteResource::UniformBuffer {
                    uniform_upload_index: _,
                } => {
                    let uniform = plan
                        .resource
                        .uniform_resource(resources.uniform_buffers)
                        .map_err(io::Error::other)?;
                    let buffer_info = [vk::DescriptorBufferInfo::default()
                        .buffer(uniform.buffer)
                        .offset(0)
                        .range(vk::WHOLE_SIZE)];
                    let write = [vk::WriteDescriptorSet::default()
                        .dst_set(descriptor_set)
                        .dst_binding(plan.binding)
                        .descriptor_type(plan.descriptor_type)
                        .buffer_info(&buffer_info)];
                    unsafe {
                        self.device.update_descriptor_sets(&write, &[]);
                    }
                }
                AshDescriptorWriteResource::CombinedImageSampler {
                    sampler_index: _,
                    image: _,
                } => {
                    let sampler = plan
                        .resource
                        .sampler(resources.samplers)
                        .map_err(io::Error::other)?;
                    let image = descriptor_image_resource(
                        resources,
                        plan.resource.image_resource().map_err(io::Error::other)?,
                    )?;
                    let image_info = [vk::DescriptorImageInfo::default()
                        .sampler(sampler)
                        .image_view(image.view)
                        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                    let write = [vk::WriteDescriptorSet::default()
                        .dst_set(descriptor_set)
                        .dst_binding(plan.binding)
                        .descriptor_type(plan.descriptor_type)
                        .image_info(&image_info)];
                    unsafe {
                        self.device.update_descriptor_sets(&write, &[]);
                    }
                }
                AshDescriptorWriteResource::SampledImage { image: _ } => {
                    let image = descriptor_image_resource(
                        resources,
                        plan.resource.image_resource().map_err(io::Error::other)?,
                    )?;
                    let image_info = [vk::DescriptorImageInfo::default()
                        .image_view(image.view)
                        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                    let write = [vk::WriteDescriptorSet::default()
                        .dst_set(descriptor_set)
                        .dst_binding(plan.binding)
                        .descriptor_type(plan.descriptor_type)
                        .image_info(&image_info)];
                    unsafe {
                        self.device.update_descriptor_sets(&write, &[]);
                    }
                }
                AshDescriptorWriteResource::Sampler { sampler_index: _ } => {
                    let sampler = plan
                        .resource
                        .sampler(resources.samplers)
                        .map_err(io::Error::other)?;
                    let image_info = [vk::DescriptorImageInfo::default().sampler(sampler)];
                    let write = [vk::WriteDescriptorSet::default()
                        .dst_set(descriptor_set)
                        .dst_binding(plan.binding)
                        .descriptor_type(plan.descriptor_type)
                        .image_info(&image_info)];
                    unsafe {
                        self.device.update_descriptor_sets(&write, &[]);
                    }
                }
                AshDescriptorWriteResource::StorageBuffer {
                    buffer_upload_index: _,
                } => {
                    let buffer = plan
                        .resource
                        .storage_buffer_resource(resources.buffers)
                        .map_err(io::Error::other)?;
                    let buffer_info = [vk::DescriptorBufferInfo::default()
                        .buffer(buffer.buffer)
                        .offset(0)
                        .range(vk::WHOLE_SIZE)];
                    let write = [vk::WriteDescriptorSet::default()
                        .dst_set(descriptor_set)
                        .dst_binding(plan.binding)
                        .descriptor_type(plan.descriptor_type)
                        .buffer_info(&buffer_info)];
                    unsafe {
                        self.device.update_descriptor_sets(&write, &[]);
                    }
                }
            }
        }
        Ok(())
    }

    fn create_render_pass(
        &self,
        plan: AshRenderPassCreationPlan,
    ) -> Result<vk::RenderPass, vk::Result> {
        let attachments = plan.attachment_descriptions();
        let color_attachment = plan.color_attachment_references();
        let depth_attachment = plan.depth_attachment_reference();
        let subpass = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment)
            .depth_stencil_attachment(&depth_attachment)];
        let dependency = [plan.subpass_dependency()];
        let info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpass)
            .dependencies(&dependency);
        unsafe { self.device.create_render_pass(&info, None) }
    }

    fn create_framebuffer(
        &self,
        render_pass: vk::RenderPass,
        color_view: vk::ImageView,
        depth_view: vk::ImageView,
        extent: vk::Extent2D,
    ) -> Result<vk::Framebuffer, vk::Result> {
        let attachments = [color_view, depth_view];
        let plan = ash_framebuffer_plan(extent);
        let info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(&attachments)
            .width(plan.width())
            .height(plan.height())
            .layers(plan.layers);
        unsafe { self.device.create_framebuffer(&info, None) }
    }

    fn create_shader_module(&self, code: &[u32]) -> Result<vk::ShaderModule, vk::Result> {
        let info = vk::ShaderModuleCreateInfo::default().code(code);
        unsafe { self.device.create_shader_module(&info, None) }
    }

    fn create_graphics_pipelines(
        &self,
        frame: &AshRendererFrame,
        context: &PipelineBuildContext<'_>,
    ) -> Result<Vec<vk::Pipeline>, Box<dyn Error>> {
        frame
            .pipelines
            .iter()
            .map(|pipeline| self.create_graphics_pipeline(pipeline, context))
            .collect()
    }

    fn create_graphics_pipeline(
        &self,
        pipeline: &AshGraphicsPipelinePlan,
        context: &PipelineBuildContext<'_>,
    ) -> Result<vk::Pipeline, Box<dyn Error>> {
        let shader_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(context.vertex_shader)
                .name(context.vertex_entry_point),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(context.fragment_shader)
                .name(context.fragment_entry_point),
        ];
        let state = ash_graphics_pipeline_state_plan(pipeline, context.extent);
        let layout = context.pipeline_layouts[state.descriptor_set_index];
        let vertex_binding = [state.vertex_binding];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_binding)
            .vertex_attribute_descriptions(&state.vertex_attributes);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(state.topology)
            .primitive_restart_enable(state.primitive_restart_enable);
        let viewport = [state.viewport];
        let scissor = [state.scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewport)
            .scissors(&scissor);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(state.polygon_mode)
            .cull_mode(state.cull_mode)
            .front_face(state.front_face)
            .line_width(state.line_width);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(state.rasterization_samples);
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(state.depth_test_enable)
            .depth_write_enable(state.depth_write_enable)
            .depth_compare_op(state.depth_compare_op);
        let color_attachment = [state.color_blend_attachment];
        let color_blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&color_attachment);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend)
            .layout(layout)
            .render_pass(context.render_pass)
            .subpass(0);
        let pipelines = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
        }
        .map_err(|(_, err)| Box::<dyn Error>::from(err))?;
        pipelines
            .into_iter()
            .next()
            .ok_or_else(|| "Vulkan returned no graphics pipeline".into())
    }

    fn record_command_buffers(
        &self,
        frame: &AshRendererFrame,
        context: &CommandRecordContext<'_>,
    ) -> Result<Vec<vk::CommandBuffer>, Box<dyn Error>> {
        let drawable = ash_drawable_frame_from_renderer_frame_with_options(
            frame,
            context.extent,
            AshDrawableFrameOptions {
                color_clear: [0.0, 0.0, 0.0, context.clear_alpha.clamp(0.0, 1.0)],
                ..Default::default()
            },
        );
        if !drawable.skipped_draws.is_empty() {
            return Err(format!(
                "drawable frame has skipped draws: {:?}",
                drawable.skipped_draws
            )
            .into());
        }
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(context.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffers = unsafe { self.device.allocate_command_buffers(&allocate_info)? };
        let command_buffer = command_buffers[0];
        let begin_info = vk::CommandBufferBeginInfo::default();
        let depth_clear = drawable.render_pass.depth_stencil_clear.unwrap_or_default();
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: drawable.render_pass.color_clear,
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: depth_clear.depth,
                    stencil: depth_clear.stencil,
                },
            },
        ];
        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(context.render_pass)
            .framebuffer(context.framebuffer)
            .render_area(drawable.render_pass.render_area)
            .clear_values(&clear_values);

        unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin_info)?;
            self.record_texture_uploads(command_buffer, frame, context);
            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );
            for command in &drawable.commands {
                match command {
                    AshCommandPlan::BindGraphicsPipeline { .. } => {
                        let pipeline = command
                            .vk_graphics_pipeline(context.pipelines)
                            .map_err(io::Error::other)?;
                        self.device.cmd_bind_pipeline(
                            command_buffer,
                            vk::PipelineBindPoint::GRAPHICS,
                            pipeline,
                        );
                    }
                    AshCommandPlan::BindDescriptorSet { .. } => {
                        let layout = command
                            .vk_pipeline_layout(&frame.pipelines, context.pipeline_layouts)
                            .map_err(io::Error::other)?;
                        let descriptor_set = command
                            .vk_descriptor_set(context.descriptor_sets)
                            .map_err(io::Error::other)?;
                        self.device.cmd_bind_descriptor_sets(
                            command_buffer,
                            vk::PipelineBindPoint::GRAPHICS,
                            layout,
                            0,
                            &[descriptor_set],
                            &[],
                        );
                    }
                    AshCommandPlan::BindVertexBuffer {
                        binding, offset, ..
                    } => {
                        let buffer = command
                            .vertex_buffer_resource(context.buffers)
                            .map_err(io::Error::other)?
                            .buffer;
                        self.device.cmd_bind_vertex_buffers(
                            command_buffer,
                            *binding,
                            &[buffer],
                            &[*offset],
                        );
                    }
                    AshCommandPlan::BindIndexBuffer {
                        offset, index_type, ..
                    } => {
                        let buffer = command
                            .index_buffer_resource(context.buffers)
                            .map_err(io::Error::other)?
                            .buffer;
                        self.device.cmd_bind_index_buffer(
                            command_buffer,
                            buffer,
                            *offset,
                            *index_type,
                        );
                    }
                    AshCommandPlan::DrawIndexed { .. } => {
                        let draw = command.draw_indexed_args().map_err(io::Error::other)?;
                        self.device.cmd_draw_indexed(
                            command_buffer,
                            draw.index_count,
                            draw.instance_count,
                            draw.first_index,
                            draw.vertex_offset,
                            draw.first_instance,
                        );
                    }
                }
            }
            self.device.cmd_end_render_pass(command_buffer);
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .image(context.color_target)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(1)
                            .layer_count(1),
                    )],
            );
            self.device.cmd_copy_image_to_buffer(
                command_buffer,
                context.color_target,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                context.readback_buffer,
                &[vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width: context.extent.width,
                        height: context.extent.height,
                        depth: 1,
                    })],
            );
            self.device.end_command_buffer(command_buffer)?;
        }
        Ok(command_buffers)
    }

    fn record_texture_uploads(
        &self,
        command_buffer: vk::CommandBuffer,
        _frame: &AshRendererFrame,
        context: &CommandRecordContext<'_>,
    ) {
        for ((image, staging), mip_levels) in context
            .texture_images
            .iter()
            .zip(context.texture_staging_buffers)
            .zip(context.texture_mip_levels)
        {
            self.record_texture_upload(command_buffer, image.image, staging.buffer, mip_levels);
        }
        for (fallback, image, staging) in [
            (
                GltfMaterialTextureFallback::White,
                &context.fallback_textures.white,
                &context.fallback_texture_staging.white,
            ),
            (
                GltfMaterialTextureFallback::Black,
                &context.fallback_textures.black,
                &context.fallback_texture_staging.black,
            ),
            (
                GltfMaterialTextureFallback::NeutralNormal,
                &context.fallback_textures.neutral_normal,
                &context.fallback_texture_staging.neutral_normal,
            ),
        ] {
            let level = [RgbaMipLevel {
                width: 1,
                height: 1,
                rgba: fallback_rgba(fallback).to_vec(),
            }];
            self.record_texture_upload(command_buffer, image.image, staging.buffer, &level);
        }
    }

    fn record_texture_upload(
        &self,
        command_buffer: vk::CommandBuffer,
        image: vk::Image,
        staging_buffer: vk::Buffer,
        mip_levels: &[RgbaMipLevel],
    ) {
        let subresource_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(u32::try_from(mip_levels.len()).unwrap_or(1).max(1))
            .layer_count(1);
        let to_transfer = [vk::ImageMemoryBarrier::default()
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .image(image)
            .subresource_range(subresource_range)];
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_transfer,
            );
            let regions = mip_copy_regions(mip_levels);
            self.device.cmd_copy_buffer_to_image(
                command_buffer,
                staging_buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &regions,
            );
            let to_shader = [vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image(image)
                .subresource_range(subresource_range)];
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_shader,
            );
        }
    }

    fn submit_and_readback(
        &self,
        resources: &VulkanFrameResources,
    ) -> Result<ReadbackFrame, Box<dyn Error>> {
        let queue = unsafe { self.device.get_device_queue(self.queue_family_index, 0) };
        let submit_info = [vk::SubmitInfo::default().command_buffers(&resources.command_buffers)];
        let fence_info = vk::FenceCreateInfo::default();
        let fence = unsafe { self.device.create_fence(&fence_info, None)? };
        unsafe {
            self.device.queue_submit(queue, &submit_info, fence)?;
            self.device.wait_for_fences(&[fence], true, u64::MAX)?;
            self.device.destroy_fence(fence, None);
        }
        self.readback_summary(resources)
    }

    fn readback_summary(
        &self,
        resources: &VulkanFrameResources,
    ) -> Result<ReadbackFrame, Box<dyn Error>> {
        let size = resources.readback_len as vk::DeviceSize;
        let bytes = unsafe {
            let mapped = self.device.map_memory(
                resources.readback.memory,
                0,
                size,
                vk::MemoryMapFlags::empty(),
            )?;
            let slice = std::slice::from_raw_parts(mapped.cast::<u8>(), resources.readback_len);
            let bytes = slice.to_vec();
            self.device.unmap_memory(resources.readback.memory);
            bytes
        };
        Ok(ReadbackFrame::from_rgba(bytes))
    }

    fn find_memory_type(
        &self,
        type_bits: u32,
        properties: vk::MemoryPropertyFlags,
    ) -> Result<u32, Box<dyn Error>> {
        (0..self.memory_properties.memory_type_count)
            .find(|index| {
                let type_supported = (type_bits & (1 << index)) != 0;
                let memory_type = self.memory_properties.memory_types[*index as usize];
                type_supported && memory_type.property_flags.contains(properties)
            })
            .ok_or_else(|| format!("no Vulkan memory type supports {properties:?}").into())
    }

    fn destroy_frame_resources(&self, resources: VulkanFrameResources) {
        unsafe {
            self.device
                .destroy_command_pool(resources.command_pool, None);
            for pipeline in resources.pipelines {
                self.device.destroy_pipeline(pipeline, None);
            }
            for module in resources.shader_modules {
                self.device.destroy_shader_module(module, None);
            }
            self.device.destroy_framebuffer(resources.framebuffer, None);
            self.device.destroy_render_pass(resources.render_pass, None);
            for layout in resources.pipeline_layouts {
                self.device.destroy_pipeline_layout(layout, None);
            }
            self.device
                .destroy_descriptor_pool(resources.descriptor_pool, None);
            for layout in resources.descriptor_set_layouts {
                self.device.destroy_descriptor_set_layout(layout, None);
            }
            for sampler in resources.samplers {
                self.device.destroy_sampler(sampler, None);
            }
            for buffer in resources.uniform_buffers {
                self.device.destroy_buffer(buffer.buffer, None);
                self.device.free_memory(buffer.memory, None);
            }
            for buffer in resources.texture_staging_buffers {
                self.device.destroy_buffer(buffer.buffer, None);
                self.device.free_memory(buffer.memory, None);
            }
            for image in resources.images {
                self.device.destroy_image_view(image.view, None);
                self.device.destroy_image(image.image, None);
                self.device.free_memory(image.memory, None);
            }
            for image in resources.fallback_textures.into_iter() {
                self.device.destroy_image_view(image.view, None);
                self.device.destroy_image(image.image, None);
                self.device.free_memory(image.memory, None);
            }
            for buffer in resources.fallback_texture_staging.into_iter() {
                self.device.destroy_buffer(buffer.buffer, None);
                self.device.free_memory(buffer.memory, None);
            }
            self.device
                .destroy_image_view(resources.depth_target.view, None);
            self.device
                .destroy_image(resources.depth_target.image, None);
            self.device.free_memory(resources.depth_target.memory, None);
            self.device
                .destroy_image_view(resources.color_target.view, None);
            self.device
                .destroy_image(resources.color_target.image, None);
            self.device.free_memory(resources.color_target.memory, None);
            self.device.destroy_buffer(resources.readback.buffer, None);
            self.device.free_memory(resources.readback.memory, None);
            for buffer in resources.buffers {
                self.device.destroy_buffer(buffer.buffer, None);
                self.device.free_memory(buffer.memory, None);
            }
        }
    }
}

fn flatten_mip_level_rgba(mip_levels: &[RgbaMipLevel]) -> Vec<u8> {
    let byte_len = mip_levels.iter().map(|level| level.rgba.len()).sum();
    let mut bytes = Vec::with_capacity(byte_len);
    for level in mip_levels {
        bytes.extend_from_slice(&level.rgba);
    }
    bytes
}

fn mip_copy_regions(mip_levels: &[RgbaMipLevel]) -> Vec<vk::BufferImageCopy> {
    let mut offset = 0_u64;
    mip_levels
        .iter()
        .enumerate()
        .map(|(mip_level, level)| {
            let region = vk::BufferImageCopy::default()
                .buffer_offset(offset)
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(u32::try_from(mip_level).unwrap_or(0))
                        .layer_count(1),
                )
                .image_extent(vk::Extent3D {
                    width: level.width,
                    height: level.height,
                    depth: 1,
                });
            offset += u64::try_from(level.rgba.len()).unwrap_or(0);
            region
        })
        .collect()
}

fn default_sampler_plan() -> AshSamplerPlan {
    AshSamplerPlan {
        mag_filter: vk::Filter::LINEAR,
        min_filter: vk::Filter::LINEAR,
        mipmap_mode: vk::SamplerMipmapMode::LINEAR,
        address_mode_u: vk::SamplerAddressMode::REPEAT,
        address_mode_v: vk::SamplerAddressMode::REPEAT,
        min_lod: 0.0,
        max_lod: 32.0,
        normal_map_decode: false,
    }
}

fn fallback_rgba(fallback: GltfMaterialTextureFallback) -> &'static [u8; 4] {
    match fallback {
        GltfMaterialTextureFallback::White => &[255, 255, 255, 255],
        GltfMaterialTextureFallback::Black => &[0, 0, 0, 255],
        GltfMaterialTextureFallback::NeutralNormal => &[128, 128, 255, 255],
    }
}

fn shader_sources_from_options(options: &Options) -> Result<ShaderModuleSources, Box<dyn Error>> {
    match (&options.vertex_spv, &options.fragment_spv) {
        (Some(vertex), Some(fragment)) => Ok(ShaderModuleSources {
            vertex: read_spirv_words(vertex)?,
            fragment: read_spirv_words(fragment)?,
            vertex_entry: options.vertex_entry.clone(),
            fragment_entry: options.fragment_entry.clone(),
            source: ShaderSourceKind::ExternalSpirv,
        }),
        (None, None) => Ok(ShaderModuleSources {
            vertex: MINIMAL_VERTEX_SPV.to_vec(),
            fragment: MINIMAL_FRAGMENT_SPV.to_vec(),
            vertex_entry: "main".to_owned(),
            fragment_entry: "main".to_owned(),
            source: ShaderSourceKind::BuiltInSmoke,
        }),
        _ => Err("--vertex-spv and --fragment-spv must be provided together".into()),
    }
}

fn read_spirv_words(path: &Path) -> Result<Vec<u32>, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    if bytes.len() % std::mem::size_of::<u32>() != 0 {
        return Err(format!(
            "{} is not valid SPIR-V: byte length is not a multiple of 4",
            path.display()
        )
        .into());
    }
    let words = bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>();
    if words.first().copied() != Some(0x0723_0203) {
        return Err(format!("{} is not valid SPIR-V: missing magic", path.display()).into());
    }
    Ok(words)
}

fn shader_source_label(source: ShaderSourceKind) -> &'static str {
    match source {
        ShaderSourceKind::BuiltInSmoke => "built-in-color-smoke",
        ShaderSourceKind::ExternalSpirv => "external-spirv",
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

struct RgbaJsonArtifact<'a> {
    options: &'a Options,
    frame: &'a AshRendererFrame,
    primitives: &'a [AshVrmPrimitive],
    pipelines: &'a [AshMtoonPipelinePlan],
    diagnostic_owner_ids: &'a [AshDiagnosticOwnerId],
    shaders: ShaderSourceKind,
    readback: &'a ReadbackFrame,
    width: u32,
    height: u32,
    depth_format: Option<vk::Format>,
    render_surfaces: &'a [RenderOwnerSurfaceKey],
    render_draws: &'a [RenderOwnerSampleDrawKey],
    owner_sample_correction_plan: Option<(&'a Path, &'a RenderOwnerSampleCorrectionPlan)>,
}

fn write_rgba_json(
    path: &Path,
    artifact_input: RgbaJsonArtifact<'_>,
) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let owner_sample_correction_plan =
        artifact_input
            .owner_sample_correction_plan
            .map(|(path, plan)| {
                owner_sample_correction_plan_json(
                    path,
                    plan,
                    artifact_input.render_surfaces,
                    artifact_input.render_draws,
                )
            });
    let artifact = json!({
        "generator": "vrm-rs crates/vrm-adapter-ash/examples/unsafe_device_renderer.rs",
        "fixture": artifact_input.options.frame.avatar.to_string_lossy(),
        "animation": (!artifact_input.options.frame.no_animation).then(|| artifact_input.options.frame.animation.to_string_lossy().to_string()),
        "time": artifact_input.options.frame.time,
        "width": artifact_input.width,
        "height": artifact_input.height,
        "renderer": {
            "backend": "ash",
            "physicalDevice": "local-vulkan-device",
            "shaderSource": shader_source_label(artifact_input.shaders),
            "graphicsPipelines": artifact_input.frame.pipelines.len(),
            "drawCalls": artifact_input.frame.draw_calls.len(),
            "depthFormat": artifact_input.depth_format.map(ash_format_label),
            "diagnosticOwnerIds": ash_diagnostic_owner_ids_json(artifact_input.diagnostic_owner_ids),
            "materialDraws": ash_material_draw_metadata(
                artifact_input.frame,
                artifact_input.primitives,
                artifact_input.pipelines,
            ),
            "ownerSampleCorrectionPlan": owner_sample_correction_plan,
        },
        "readback": {
            "checksum": format!("{:016x}", artifact_input.readback.checksum),
            "nonzeroPixels": artifact_input.readback.nonzero_pixels,
        },
        "mtoonLighting": ash_mtoon_lighting_metadata(&artifact_input.options.frame),
        "format": "rgba8",
        "rgba": &artifact_input.readback.rgba,
    });
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&artifact)?),
    )?;
    Ok(())
}

fn ash_mtoon_lighting_metadata(options: &AshVrmFramePlanOptions) -> serde_json::Value {
    let config = MtoonLightingConfig {
        accumulation: MtoonLightAccumulation::from(options.mtoon_light_accumulation),
        exposure: options.mtoon_exposure,
        ambient_base: options.mtoon_ambient_base,
        ambient_gi_scale: options.mtoon_ambient_gi_scale,
        pbr_ambient: options.pbr_ambient,
    };
    let effective = config.effective_values().to_array();
    json!({
        "exposure": options.mtoon_exposure,
        "ambientBase": options.mtoon_ambient_base,
        "ambientGiScale": options.mtoon_ambient_gi_scale,
        "pbrAmbient": options.pbr_ambient,
        "directLightScale": options.direct_light_scale,
        "directionalColor": [
            options.directional_r,
            options.directional_g,
            options.directional_b
        ],
        "lightAccumulation": ash_mtoon_light_accumulation_label(options.mtoon_light_accumulation),
        "effective": {
            "exposure": effective[0],
            "ambientBase": effective[1],
            "ambientGiScale": effective[2],
            "pbrAmbient": effective[3]
        },
        "time": options.time
    })
}

fn ash_mtoon_light_accumulation_label(value: AshMtoonLightAccumulation) -> &'static str {
    match value {
        AshMtoonLightAccumulation::Tuned => "tuned",
        AshMtoonLightAccumulation::ThreeVrm => "three-vrm",
    }
}

fn owner_sample_correction_plan_json(
    path: &Path,
    plan: &RenderOwnerSampleCorrectionPlan,
    surfaces: &[RenderOwnerSurfaceKey],
    draws: &[RenderOwnerSampleDrawKey],
) -> serde_json::Value {
    let selection = plan.surface_selection_plan(surfaces.iter());
    let coverage = plan.surface_coverage(surfaces.iter());
    json!({
        "manifest": path.to_string_lossy(),
        "entryCount": selection.entry_count(),
        "surfaceCount": coverage.surface_count,
        "matchedEntryCount": selection.matched_entry_count(),
        "unmatchedEntryCount": selection.unmatched_entry_count(),
        "matchedSurfaceCount": coverage.matched_surface_count,
        "allEntriesResolved": selection.all_entries_resolved(),
        "surfaceSelections": selection.surfaces.iter().map(|surface| {
            json!({
                "surface": owner_surface_json(&surface.surface),
                "entryCount": surface.entries.len(),
                "entries": surface.overrides().map(|entry| owner_sample_entry_json(&surface.surface, entry)).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "drawSelections": draws.iter().map(|draw| {
            let entries = plan.entries().iter()
                .filter(|entry| entry.sample_geometry.is_some())
                .filter(|entry| entry.matches_draw(draw))
                .collect::<Vec<_>>();
            json!({
                "draw": owner_sample_draw_json(draw),
                "entryCount": entries.len(),
                "entries": entries.iter().map(|entry| {
                    owner_sample_entry_json(entry.sample.surface(), RenderOwnerSampleSurfaceOverride::from(*entry))
                }).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "unmatchedEntries": selection.unmatched_entries.iter().map(|entry| {
            owner_sample_entry_json(entry.sample.surface(), RenderOwnerSampleSurfaceOverride::from(entry))
        }).collect::<Vec<_>>(),
        "unmatchedSurfaces": coverage.unmatched_surfaces.into_iter().map(|surface| {
            owner_surface_json(&surface)
        }).collect::<Vec<_>>(),
    })
}

fn ash_owner_sample_draws(primitives: &[AshVrmPrimitive]) -> Vec<RenderOwnerSampleDrawKey> {
    primitives
        .iter()
        .map(|primitive| {
            RenderOwnerSampleDrawKey::new(
                primitive.node.0 as u64,
                primitive.mesh_index as u64,
                primitive.primitive_index as u64,
                ash_owner_sample_pass(primitive.pass),
            )
        })
        .collect()
}

fn ash_material_draw_metadata(
    frame: &AshRendererFrame,
    primitives: &[AshVrmPrimitive],
    pipelines: &[AshMtoonPipelinePlan],
) -> Vec<serde_json::Value> {
    frame
        .draw_calls
        .iter()
        .filter_map(|draw| {
            let primitive = primitives.get(draw.primitive_index)?;
            let graphics_pipeline = draw.pipeline_plan_index.and_then(|index| {
                frame
                    .pipelines
                    .iter()
                    .find(|pipeline| pipeline.pipeline_plan_index == index)
            });
            let source_pipeline_index = draw
                .descriptor_set_index
                .and_then(|index| frame.descriptor_sets.get(index))
                .map(|descriptor_set| descriptor_set.pipeline_plan_index)
                .or(draw.pipeline_plan_index);
            let pipeline = source_pipeline_index.and_then(|index| pipelines.get(index))?;
            let policy = graphics_pipeline
                .map(|pipeline| pipeline.key)
                .unwrap_or(pipeline.key);
            let role = if draw.pipeline_plan_index == source_pipeline_index {
                "source"
            } else {
                "owner-sample-resolve"
            };
            let material = primitive.material.or(Some(pipeline.material));
            Some(json!({
                "draw": {
                    "node": primitive.node.0,
                    "mesh": primitive.mesh_index,
                    "primitive": primitive.primitive_index,
                    "pass": ash_mtoon_pass_label(pipeline.key.pass),
                    "key": ash_owner_source_key(primitive, pipeline.key.pass),
                    "role": role,
                },
                "material": {
                    "index": material.map(|material| material.0),
                    "name": primitive.material_name.as_deref().or(pipeline.name.as_deref()).unwrap_or("unnamed"),
                },
                "policy": {
                    "renderOrder": policy.render_order,
                    "phaseOrder": policy.phase_order,
                    "cullMode": ash_cull_mode_label(policy.cull_mode),
                    "frontFace": ash_front_face_label(policy.front_face),
                    "alphaMode": ash_uniform_alpha_mode_label(pipeline.uniform.flags[3]),
                    "alphaCutoff": pipeline.alpha_cutoff,
                    "depthWrite": policy.depth_write_enable,
                    "depthTest": policy.depth_test_enable,
                    "depthCompare": ash_compare_op_label(policy.depth_compare_op),
                    "blend": policy.blend_enable,
                },
                "vertexMaterial": ash_vertex_material_metadata(primitive, pipeline),
                "materialExtra": ash_material_extra_metadata(pipeline.render_extra_uniform),
                "textureSlots": ash_texture_slot_metadata(pipeline),
            }))
        })
        .collect()
}

fn ash_owner_source_key(primitive: &AshVrmPrimitive, pass: AshMtoonPass) -> String {
    format!(
        "node{}/mesh{}/prim{}/{}",
        primitive.node.0,
        primitive.mesh_index,
        primitive.primitive_index,
        ash_mtoon_pass_label(pass)
    )
}

fn ash_vertex_material_metadata(
    primitive: &AshVrmPrimitive,
    pipeline: &AshMtoonPipelinePlan,
) -> serde_json::Value {
    let uniform = pipeline.uniform;
    let vertex = primitive.vertices.first();
    json!({
        "baseColor": uniform.base_color_factor,
        "shadeColor": [
            uniform.shade_color_factor_cutoff[0],
            uniform.shade_color_factor_cutoff[1],
            uniform.shade_color_factor_cutoff[2],
            1.0,
        ],
        "shading": {
            "shift": uniform.shading[2],
            "toony": uniform.shading[3],
            "giEqualization": uniform.lighting[2],
            "shiftTextureScale": uniform.lighting[0],
        },
        "emissive": [
            uniform.emissive_color_outline_width[0],
            uniform.emissive_color_outline_width[1],
            uniform.emissive_color_outline_width[2],
            0.0,
        ],
        "matcapFactor": uniform.matcap_factor_debug,
        "rimColor": [
            uniform.rim_color_lighting_mix[0],
            uniform.rim_color_lighting_mix[1],
            uniform.rim_color_lighting_mix[2],
            0.0,
        ],
        "rimParams": {
            "lightingMix": uniform.rim_color_lighting_mix[3],
            "fresnelPower": uniform.rim_params[0],
            "lift": uniform.rim_params[1],
            "alphaCutoff": uniform.shade_color_factor_cutoff[3],
        },
        "normalScale": vertex.map(|vertex| vertex.normal_scale),
        "doubleSided": vertex.is_some_and(|vertex| vertex.double_sided != 0.0),
    })
}

fn ash_material_extra_metadata(extra: AshMaterialExtraUniform) -> serde_json::Value {
    json!({
        "shaderBranch": ash_shader_branch(extra),
        "flags": {
            "v0CompatShade": extra.flags[0] != 0.0,
            "pbrFallback": extra.flags[1] != 0.0,
            "gltfPbr": extra.flags[1] != 0.0,
            "threeVrmLightAccumulation": extra.flags[2] != 0.0,
            "derivativeNormals": extra.flags[3] != 0.0,
        },
        "pbr": {
            "metallic": extra.pbr_params[0],
            "roughness": extra.pbr_params[1],
            "occlusionStrength": extra.pbr_params[2],
            "directLightScale": extra.pbr_params[3],
        },
        "renderFlags": {
            "unlit": extra.flags2[0] != 0.0,
            "viewDerivativeNormals": extra.flags2[1] != 0.0,
            "flatDiagnostic": extra.flags2[2] != 0.0,
            "diagnosticCode": extra.flags2[3],
        },
        "ownerColor": extra.owner_color,
    })
}

fn ash_shader_branch(extra: AshMaterialExtraUniform) -> &'static str {
    if extra.flags2[0] != 0.0 {
        "unlit"
    } else if extra.flags[1] != 0.0 {
        "gltf_pbr"
    } else {
        "mtoon"
    }
}

fn ash_texture_slot_metadata(pipeline: &AshMtoonPipelinePlan) -> serde_json::Value {
    json!({
        "base": ash_texture_binding_metadata(pipeline, ash_mtoon_texture_binding(MtoonTextureSlot::Main)),
        "shade": ash_texture_binding_metadata(pipeline, ash_mtoon_texture_binding(MtoonTextureSlot::ShadeMultiply)),
        "shadingShift": ash_texture_binding_metadata(pipeline, ash_mtoon_texture_binding(MtoonTextureSlot::ShadingShift)),
        "normal": ash_texture_binding_metadata(pipeline, ash_mtoon_texture_binding(MtoonTextureSlot::Normal)),
        "matcap": ash_texture_binding_metadata(pipeline, ash_mtoon_texture_binding(MtoonTextureSlot::Matcap)),
        "rim": ash_texture_binding_metadata(pipeline, ash_mtoon_texture_binding(MtoonTextureSlot::RimMultiply)),
        "emissive": ash_texture_binding_metadata(pipeline, ash_material_texture_binding(GltfMaterialTextureSlot::Emissive)),
        "occlusion": ash_texture_binding_metadata(pipeline, ash_material_texture_binding(GltfMaterialTextureSlot::Occlusion)),
        "uvAnimationMask": ash_texture_binding_metadata(pipeline, ash_mtoon_texture_binding(MtoonTextureSlot::UvAnimationMask)),
    })
}

fn ash_texture_binding_metadata(
    pipeline: &AshMtoonPipelinePlan,
    binding: u32,
) -> serde_json::Value {
    pipeline
        .descriptor_bindings
        .iter()
        .find(|descriptor| descriptor.binding == binding)
        .and_then(|descriptor| descriptor.texture)
        .map_or(serde_json::Value::Null, |texture| json!(texture.0))
}

fn ash_uniform_alpha_mode_label(code: u32) -> &'static str {
    match code {
        0 => "opaque",
        1 => "mask",
        2 => "blend",
        _ => "unknown",
    }
}

fn ash_owner_sample_pass(pass: AshMtoonPass) -> RenderOwnerSamplePass {
    match pass {
        AshMtoonPass::Base => RenderOwnerSamplePass::Base,
        AshMtoonPass::Outline => RenderOwnerSamplePass::Outline,
    }
}

fn owner_sample_draw_json(draw: &RenderOwnerSampleDrawKey) -> serde_json::Value {
    json!({
        "node": draw.node,
        "mesh": draw.mesh,
        "primitive": draw.primitive,
        "pass": draw.pass.as_str(),
        "key": format!(
            "node{}/mesh{}/prim{}/{}",
            draw.node,
            draw.mesh,
            draw.primitive,
            draw.pass.as_str(),
        ),
    })
}

fn owner_surface_json(surface: &RenderOwnerSurfaceKey) -> serde_json::Value {
    json!({
        "materialName": surface.material_name(),
        "triangle": surface.triangle(),
    })
}

fn owner_sample_entry_json(
    surface: &RenderOwnerSurfaceKey,
    entry: RenderOwnerSampleSurfaceOverride,
) -> serde_json::Value {
    json!({
        "pixel": entry.pixel.to_pair(),
        "sample": entry.sample.to_pair(),
        "rgba": entry.replacement_rgba,
        "relationToExpected": entry.relation_to_expected.map(|relation| relation.as_str()),
        "surface": owner_surface_json(surface),
    })
}

fn ash_diagnostic_owner_ids_json(owners: &[AshDiagnosticOwnerId]) -> Vec<serde_json::Value> {
    owners
        .iter()
        .map(|owner| {
            let projection = owner.projection;
            json!({
                "id": owner.id,
                "color": owner.color,
                "nodeIndex": owner.source.node.0,
                "nodeName": owner.source.node_name.as_deref(),
                "meshIndex": owner.source.mesh_index,
                "meshName": owner.source.mesh_name.as_deref(),
                "primitiveIndex": owner.source.primitive_index,
                "materialIndex": owner.source.material.map(|material| material.0),
                "materialName": owner.source.material_name.as_deref(),
                "materialType": "ash-owner-id",
                "side": ash_material_side_code(owner.source.double_sided),
                "pass": ash_mtoon_pass_label(owner.source.pass),
                "renderOrder": owner.source.render_order,
                "renderPhaseOrder": owner.source.phase_order,
                "drawIndex": owner.source.draw_index,
                "frontFace": ash_front_face_label(owner.source.front_face),
                "cullMode": ash_cull_mode_label(owner.source.cull_mode),
                "alphaMode": ash_alpha_mode_label(owner.source.alpha_mode),
                "alphaTest": ash_alpha_test(owner.source.alpha_mode, owner.source.alpha_cutoff),
                "alphaCutoff": owner.source.alpha_cutoff,
                "transparent": owner.source.alpha_mode == GltfAlphaMode::Blend,
                "opacity": owner.source.opacity,
                "depthWrite": owner.source.depth_write,
                "depthTest": owner.source.depth_test,
                "depthCompare": ash_compare_op_label(owner.source.depth_compare),
                "blend": owner.source.blend,
                "triangle": owner.triangle,
                "indices": owner.indices,
                "screen": projection.map(|projection| projection.screen),
                "screenBounds": projection.map(|projection| json!({
                    "minX": projection.bounds.min_x,
                    "minY": projection.bounds.min_y,
                    "maxX": projection.bounds.max_x,
                    "maxY": projection.bounds.max_y,
                })),
                "depth": projection.map(|projection| projection.ndc_depth),
                "webglDepth": projection.map(|projection| projection.webgl_depth),
                "depthRange": projection.map(|_| "zero-to-one-ndc"),
                "screenSignedArea": projection.map(|projection| projection.screen_signed_area),
                "frontFacing": projection.map(|projection| projection.front_facing),
                "gpuFrontFacing": projection.map(|projection| projection.gpu_front_facing),
                "visibleByCullPolicy": projection.map(|projection| projection.visible_by_cull_policy),
            })
        })
        .collect()
}

fn ash_mtoon_pass_label(pass: AshMtoonPass) -> &'static str {
    match pass {
        AshMtoonPass::Base => "base",
        AshMtoonPass::Outline => "outline",
    }
}

fn ash_alpha_mode_label(mode: GltfAlphaMode) -> &'static str {
    match mode {
        GltfAlphaMode::Opaque => "opaque",
        GltfAlphaMode::Mask => "mask",
        GltfAlphaMode::Blend => "blend",
    }
}

fn ash_alpha_test(mode: GltfAlphaMode, cutoff: Option<f32>) -> f32 {
    match mode {
        GltfAlphaMode::Mask => cutoff.unwrap_or(0.5),
        GltfAlphaMode::Opaque | GltfAlphaMode::Blend => 0.0,
    }
}

fn ash_material_side_code(double_sided: bool) -> u32 {
    if double_sided { 2 } else { 0 }
}

fn ash_cull_mode_label(mode: vk::CullModeFlags) -> &'static str {
    if mode.is_empty() {
        "off"
    } else if mode == vk::CullModeFlags::BACK {
        "back"
    } else if mode == vk::CullModeFlags::FRONT {
        "front"
    } else {
        "front-and-back"
    }
}

fn ash_front_face_label(front_face: vk::FrontFace) -> &'static str {
    if front_face == vk::FrontFace::COUNTER_CLOCKWISE {
        "ccw"
    } else {
        "cw"
    }
}

fn ash_compare_op_label(compare: vk::CompareOp) -> &'static str {
    match compare {
        vk::CompareOp::LESS_OR_EQUAL => "less-equal",
        vk::CompareOp::LESS => "less",
        vk::CompareOp::GREATER_OR_EQUAL => "greater-equal",
        vk::CompareOp::GREATER => "greater",
        vk::CompareOp::ALWAYS => "always",
        vk::CompareOp::NEVER => "never",
        vk::CompareOp::EQUAL => "equal",
        vk::CompareOp::NOT_EQUAL => "not-equal",
        _ => "unknown",
    }
}

fn ash_format_label(format: vk::Format) -> &'static str {
    match format {
        vk::Format::D24_UNORM_S8_UINT => "D24_UNORM_S8_UINT",
        vk::Format::X8_D24_UNORM_PACK32 => "X8_D24_UNORM_PACK32",
        vk::Format::D32_SFLOAT => "D32_SFLOAT",
        vk::Format::R8G8B8A8_UNORM => "R8G8B8A8_UNORM",
        vk::Format::R8G8B8A8_SRGB => "R8G8B8A8_SRGB",
        _ => "UNKNOWN",
    }
}

fn write_imqraw_rgba8(path: &Path, width: u32, height: u32, rgba: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let frame = FrameOwned::packed_tight(rgba.to_vec(), width, height, PixelFormat::Rgba8)
        .map_err(|err| io::Error::other(format!("failed to create imqraw RGBA frame: {err}")))?;
    let record = RawImageRecord::new(
        Some("ash".to_string()),
        vec!["ash".to_string(), "candidate".to_string()],
        frame,
    );
    let bytes = encode_imqraw_bundle(&RawImageBundle::new(vec![record]))
        .map_err(|err| io::Error::other(format!("failed to encode imqraw bundle: {err}")))?;
    fs::write(path, bytes)?;
    Ok(())
}

fn run_artifact_self_test(options: &Options) -> Result<(), Box<dyn Error>> {
    let width = 2;
    let height = 1;
    let rgba = vec![255, 0, 0, 255, 0, 0, 255, 128];
    let readback = ReadbackFrame::from_rgba(rgba.clone());
    let json_path = options.out.clone().unwrap_or_else(|| {
        PathBuf::from("target/ash-artifact-self-test/ash-self-test.frame000.rgba.json")
    });
    let imqraw_path = options.imqraw_out.clone().unwrap_or_else(|| {
        PathBuf::from("target/ash-artifact-self-test/ash-self-test.frame000.imqraw")
    });
    write_rgba_json(
        &json_path,
        RgbaJsonArtifact {
            options,
            frame: &AshRendererFrame::default(),
            primitives: &[],
            pipelines: &[],
            diagnostic_owner_ids: &[],
            shaders: ShaderSourceKind::BuiltInSmoke,
            readback: &readback,
            width,
            height,
            depth_format: None,
            render_surfaces: &[],
            render_draws: &[],
            owner_sample_correction_plan: None,
        },
    )?;
    write_imqraw_rgba8(&imqraw_path, width, height, &rgba)?;
    validate_rgba_json(&json_path, width, height, &rgba)?;
    validate_imqraw(&imqraw_path, width, height, &rgba)?;
    println!(
        "ash artifact self-test: wrote {} and {}",
        json_path.display(),
        imqraw_path.display()
    );
    Ok(())
}

fn validate_rgba_json(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Box<dyn Error>> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    if value.get("width").and_then(serde_json::Value::as_u64) != Some(u64::from(width)) {
        return Err(format!("{} has unexpected width", path.display()).into());
    }
    if value.get("height").and_then(serde_json::Value::as_u64) != Some(u64::from(height)) {
        return Err(format!("{} has unexpected height", path.display()).into());
    }
    if value
        .pointer("/mtoonLighting/effective/pbrAmbient")
        .and_then(serde_json::Value::as_f64)
        .is_none()
    {
        return Err(format!(
            "{} does not contain effective PBR ambient metadata",
            path.display()
        )
        .into());
    }
    if value
        .pointer("/mtoonLighting/lightAccumulation")
        .and_then(serde_json::Value::as_str)
        .is_none()
    {
        return Err(format!(
            "{} does not contain light accumulation metadata",
            path.display()
        )
        .into());
    }
    let actual = value
        .get("rgba")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{} does not contain an rgba array", path.display()))?
        .iter()
        .map(|entry| {
            entry
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| format!("{} contains a non-u8 rgba value", path.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actual != rgba {
        return Err(format!("{} rgba payload did not round-trip", path.display()).into());
    }
    Ok(())
}

fn validate_imqraw(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Box<dyn Error>> {
    let bundle = decode_imqraw_bundle(&fs::read(path)?)?;
    let record = bundle
        .records
        .first()
        .ok_or_else(|| format!("{} contains no imqraw records", path.display()))?;
    let dimensions = record.frame.dimensions();
    if dimensions.width != width || dimensions.height != height {
        return Err(format!("{} has unexpected imqraw dimensions", path.display()).into());
    }
    if record.frame.format().pixel_format != PixelFormat::Rgba8 {
        return Err(format!("{} is not RGBA8 imqraw", path.display()).into());
    }
    let plane = record
        .frame
        .owned_planes()
        .first()
        .ok_or_else(|| format!("{} contains no imqraw plane", path.display()))?;
    if plane.data != rgba {
        return Err(format!("{} imqraw payload did not round-trip", path.display()).into());
    }
    Ok(())
}

impl Drop for UnsafeAshDeviceRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_upload_bytes_are_tightly_packed() {
        let levels = vec![
            RgbaMipLevel {
                width: 2,
                height: 1,
                rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
            },
            RgbaMipLevel {
                width: 1,
                height: 1,
                rgba: vec![9, 10, 11, 12],
            },
        ];

        assert_eq!(
            flatten_mip_level_rgba(&levels),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
    }

    #[test]
    fn mip_copy_regions_advance_offsets_and_levels() {
        let levels = vec![
            RgbaMipLevel {
                width: 4,
                height: 2,
                rgba: vec![0; 32],
            },
            RgbaMipLevel {
                width: 2,
                height: 1,
                rgba: vec![0; 8],
            },
            RgbaMipLevel {
                width: 1,
                height: 1,
                rgba: vec![0; 4],
            },
        ];

        let regions = mip_copy_regions(&levels);

        assert_eq!(regions.len(), 3);
        assert_eq!(regions[0].buffer_offset, 0);
        assert_eq!(regions[0].image_subresource.mip_level, 0);
        assert_eq!(regions[0].image_extent.width, 4);
        assert_eq!(regions[1].buffer_offset, 32);
        assert_eq!(regions[1].image_subresource.mip_level, 1);
        assert_eq!(regions[1].image_extent.width, 2);
        assert_eq!(regions[2].buffer_offset, 40);
        assert_eq!(regions[2].image_subresource.mip_level, 2);
        assert_eq!(regions[2].image_extent.width, 1);
    }

    #[test]
    fn owner_sample_correction_manifest_updates_readback_metadata() {
        let value = serde_json::json!({
            "corrections": [{
                "x": 1,
                "y": 0,
                "rgba": [0, 0, 0, 0],
                "surface": {"materialName": "body", "triangle": 7},
                "sample": [0.5, 0.5],
            }]
        });
        let plan = RenderOwnerSampleCorrectionPlan::from_manifest_value(&value).unwrap();
        let mut readback = ReadbackFrame::from_rgba(vec![255, 0, 0, 255, 0, 0, 255, 255]);
        let original_checksum = readback.checksum;

        let applied = readback
            .apply_owner_sample_correction_plan(&plan, 2, 1)
            .unwrap();

        assert_eq!(applied, 1);
        assert_eq!(readback.rgba, vec![255, 0, 0, 255, 0, 0, 0, 0]);
        assert_ne!(readback.checksum, original_checksum);
        assert_eq!(readback.nonzero_pixels, 1);
    }

    #[test]
    fn material_draw_metadata_exposes_ash_pipeline_policy_and_textures() {
        use bytemuck::Zeroable;
        use vrm_adapter::MtoonGpuUniform;
        use vrm_adapter_ash::{
            AshDescriptorBindingPlan, AshDescriptorSetPlan, AshDrawCallPlan,
            AshGraphicsPipelinePlan, AshMaterialUvUniform, AshPipelineKey,
        };
        use vrm_core::{MaterialRef, NodeRef, TextureRef};

        let primitive = AshVrmPrimitive {
            node: NodeRef(145),
            mesh_index: 4,
            primitive_index: 9,
            material_name: Some("backpack_nm".to_owned()),
            material: Some(MaterialRef(14)),
            pass: AshMtoonPass::Base,
            vertices: vec![vrm_adapter_ash::AshVrmVertex {
                normal_scale: 0.75,
                double_sided: 0.0,
                ..vrm_adapter_ash::AshVrmVertex::zeroed()
            }],
            indices: vec![0, 1, 2],
        };
        let mut uniform = MtoonGpuUniform::zeroed();
        uniform.base_color_factor = [0.1, 0.2, 0.3, 1.0];
        uniform.shade_color_factor_cutoff = [0.4, 0.5, 0.6, 0.5];
        uniform.flags[3] = 1;
        let pipeline = AshMtoonPipelinePlan {
            material: MaterialRef(14),
            name: Some("backpack_nm".to_owned()),
            key: AshPipelineKey {
                pass: AshMtoonPass::Base,
                render_order: 2000,
                phase_order: 19,
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
                    binding: ash_mtoon_texture_binding(MtoonTextureSlot::Main),
                    descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                    stage_flags: vk::ShaderStageFlags::FRAGMENT,
                    texture: Some(TextureRef(10)),
                    color_space: vrm_io::GltfMaterialTextureColorSpace::Srgb,
                    sampler: None,
                },
                AshDescriptorBindingPlan {
                    binding: ash_mtoon_texture_binding(MtoonTextureSlot::Normal),
                    descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                    stage_flags: vk::ShaderStageFlags::FRAGMENT,
                    texture: Some(TextureRef(13)),
                    color_space: vrm_io::GltfMaterialTextureColorSpace::Linear,
                    sampler: None,
                },
            ],
            uniform,
            uv_uniform: AshMaterialUvUniform::default(),
            render_extra_uniform: AshMaterialExtraUniform {
                flags: [0.0, 1.0, 1.0, 1.0],
                pbr_params: [0.0, 0.657, 1.0, 1.0],
                flags2: [1.0, 1.0, 0.0, 1.25],
                ..Default::default()
            },
            uniform_buffer_size: 0,
            alpha_cutoff: 0.5,
            outline_width: 0.0,
            base_color_factor: [1.0; 4],
            emissive_color: [0.0; 3],
        };
        let frame = AshRendererFrame {
            pipelines: vec![AshGraphicsPipelinePlan {
                material: MaterialRef(14),
                pipeline_plan_index: 1,
                descriptor_set_index: 0,
                key: AshPipelineKey {
                    cull_mode: vk::CullModeFlags::empty(),
                    depth_write_enable: false,
                    depth_compare_op: vk::CompareOp::ALWAYS,
                    render_order: 12_000,
                    phase_order: 10_019,
                    ..pipeline.key
                },
                vertex_stride: 0,
                vertex_attributes: Vec::new(),
                color_format: vk::Format::R8G8B8A8_UNORM,
                depth_format: None,
            }],
            descriptor_sets: vec![AshDescriptorSetPlan {
                material: MaterialRef(14),
                pipeline_plan_index: 0,
                bindings: Vec::new(),
            }],
            draw_calls: vec![
                AshDrawCallPlan {
                    primitive_index: 0,
                    material: Some(MaterialRef(14)),
                    pipeline_plan_index: Some(0),
                    descriptor_set_index: Some(0),
                    vertex_buffer_index: 0,
                    index_buffer_index: 1,
                    index_count: 3,
                    render_order: 2000,
                    phase_order: 19,
                },
                AshDrawCallPlan {
                    primitive_index: 0,
                    material: Some(MaterialRef(14)),
                    pipeline_plan_index: Some(1),
                    descriptor_set_index: Some(0),
                    vertex_buffer_index: 2,
                    index_buffer_index: 3,
                    index_count: 3,
                    render_order: 12_000,
                    phase_order: 10_019,
                },
            ],
            ..Default::default()
        };

        let metadata = ash_material_draw_metadata(&frame, &[primitive], &[pipeline]);

        assert_eq!(
            metadata[0]
                .pointer("/draw/key")
                .and_then(serde_json::Value::as_str),
            Some("node145/mesh4/prim9/base")
        );
        assert_eq!(
            metadata[0]
                .pointer("/draw/role")
                .and_then(serde_json::Value::as_str),
            Some("source")
        );
        assert_eq!(
            metadata[1]
                .pointer("/draw/role")
                .and_then(serde_json::Value::as_str),
            Some("owner-sample-resolve")
        );
        assert_eq!(
            metadata[1]
                .pointer("/policy/cullMode")
                .and_then(serde_json::Value::as_str),
            Some("off")
        );
        assert_eq!(
            metadata[0]
                .pointer("/materialExtra/shaderBranch")
                .and_then(serde_json::Value::as_str),
            Some("unlit")
        );
        assert_eq!(
            metadata[0]
                .pointer("/materialExtra/flags/pbrFallback")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            metadata[0]
                .pointer("/materialExtra/flags/gltfPbr")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            metadata[0]
                .pointer("/materialExtra/flags/derivativeNormals")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            metadata[0]
                .pointer("/materialExtra/renderFlags/viewDerivativeNormals")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            metadata[0]
                .pointer("/materialExtra/pbr/directLightScale")
                .and_then(serde_json::Value::as_f64),
            Some(1.0)
        );
        assert_eq!(
            metadata[0]
                .pointer("/policy/alphaMode")
                .and_then(serde_json::Value::as_str),
            Some("mask")
        );
        assert_eq!(
            metadata[0]
                .pointer("/textureSlots/base")
                .and_then(serde_json::Value::as_u64),
            Some(10)
        );
        assert_eq!(
            metadata[0]
                .pointer("/textureSlots/normal")
                .and_then(serde_json::Value::as_u64),
            Some(13)
        );
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse();
    if options.artifact_self_test {
        return run_artifact_self_test(&options);
    }
    if options.dry_run {
        println!("dry run: parsed ash unsafe device renderer options");
        return Ok(());
    }
    let correction_plan = options
        .owner_sample_correction_manifest
        .as_deref()
        .map(
            |path| -> Result<RenderOwnerSampleCorrectionPlan, Box<dyn Error>> {
                let value = serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path)?)?;
                Ok(RenderOwnerSampleCorrectionPlan::from_manifest_value(
                    &value,
                )?)
            },
        )
        .transpose()?;
    let frame_plan =
        frame_plan_from_options_with_viewport(&options.frame, options.width, options.height)?;
    let owner_sample_selection = correction_plan
        .as_ref()
        .map(|plan| plan.surface_selection_plan(frame_plan.render_surfaces.iter()));
    let renderer_frame = ash_renderer_frame_from_plan_with_owner_sample_selection(
        &frame_plan,
        owner_sample_selection.as_ref(),
    )
    .map_err(|error| format!("failed to build ash owner/sample renderer frame: {error:?}"))?;
    let shaders = shader_sources_from_options(&options)?;
    let renderer = UnsafeAshDeviceRenderer::new()?;
    let resources = renderer.materialize_frame(
        &renderer_frame,
        vk::Extent2D {
            width: options.width.max(1),
            height: options.height.max(1),
        },
        &shaders,
        options.clear_alpha,
    )?;
    println!(
        "unsafe ash device renderer: {} buffers, {} images, {} descriptor sets, {} graphics pipelines, {} recorded command buffers, {} draw plans, {} shaders, depth {}, on physical device {:?}",
        resources.buffers.len(),
        resources.images.len(),
        resources.descriptor_sets.len(),
        resources.pipelines.len(),
        resources.command_buffers.len(),
        renderer_frame.draw_calls.len(),
        shader_source_label(shaders.source),
        ash_format_label(resources.depth_format),
        renderer.physical_device
    );
    if options.submit_readback || options.out.is_some() || options.imqraw_out.is_some() {
        let mut summary = renderer.submit_and_readback(&resources)?;
        if options.apply_owner_sample_readback_replacement
            && let Some((path, plan)) = options
                .owner_sample_correction_manifest
                .as_deref()
                .zip(correction_plan.as_ref())
        {
            let applied = summary.apply_owner_sample_correction_plan(
                plan,
                options.width.max(1),
                options.height.max(1),
            )?;
            println!(
                "ash owner/sample correction manifest: applied {} corrections from {}",
                applied,
                path.display()
            );
        }
        println!(
            "ash offscreen readback: {} bytes, {} nonzero pixels, checksum {:016x}",
            summary.byte_len(),
            summary.nonzero_pixels,
            summary.checksum
        );
        if let Some(path) = &options.out {
            let render_draws = ash_owner_sample_draws(&frame_plan.primitives);
            write_rgba_json(
                path,
                RgbaJsonArtifact {
                    options: &options,
                    frame: &renderer_frame,
                    primitives: &frame_plan.primitives,
                    pipelines: &frame_plan.mtoon_pipelines,
                    diagnostic_owner_ids: &frame_plan.diagnostic_owner_ids,
                    shaders: shaders.source,
                    readback: &summary,
                    width: options.width.max(1),
                    height: options.height.max(1),
                    depth_format: Some(resources.depth_format),
                    render_surfaces: &frame_plan.render_surfaces,
                    render_draws: &render_draws,
                    owner_sample_correction_plan: options
                        .owner_sample_correction_manifest
                        .as_deref()
                        .zip(correction_plan.as_ref()),
                },
            )?;
        }
        if let Some(path) = &options.imqraw_out {
            write_imqraw_rgba8(
                path,
                options.width.max(1),
                options.height.max(1),
                &summary.rgba,
            )?;
        }
    }
    renderer.destroy_frame_resources(resources);
    Ok(())
}
