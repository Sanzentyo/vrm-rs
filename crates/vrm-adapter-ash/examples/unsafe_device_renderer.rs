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
use vrm_adapter_ash::{
    AshGraphicsPipelinePlan, AshRendererFrame, AshSamplerPlan, AshVertexAttributePlan,
    AshVrmFramePlanOptions, ash_renderer_frame_from_plan, frame_plan_from_options,
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
    /// Submit the recorded offscreen draw and read back the color attachment.
    #[arg(long)]
    submit_readback: bool,
    /// Write the submitted/read-back offscreen color attachment as a render-parity RGBA JSON artifact.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Write the submitted/read-back offscreen color attachment as an imqraw bundle.
    #[arg(long)]
    imqraw_out: Option<PathBuf>,
    /// Optional precompiled SPIR-V vertex shader for the offscreen graphics pipelines.
    ///
    /// The shader must use entry point `main` and match the example vertex input
    /// plus descriptor-set layout emitted from `AshRendererFrame`.
    #[arg(long, requires = "fragment_spv")]
    vertex_spv: Option<PathBuf>,
    /// Optional precompiled SPIR-V fragment shader for the offscreen graphics pipelines.
    ///
    /// Use together with `--vertex-spv` to replace the built-in color-only smoke
    /// shader without committing shader binaries to this repository.
    #[arg(long, requires = "vertex_spv")]
    fragment_spv: Option<PathBuf>,
}

struct VulkanFrameResources {
    buffers: Vec<VulkanBuffer>,
    images: Vec<VulkanImage>,
    texture_staging_buffers: Vec<VulkanBuffer>,
    fallback_texture: VulkanImage,
    fallback_texture_staging: VulkanBuffer,
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

struct PipelineBuildContext<'a> {
    render_pass: vk::RenderPass,
    extent: vk::Extent2D,
    vertex_shader: vk::ShaderModule,
    fragment_shader: vk::ShaderModule,
    pipeline_layouts: &'a [vk::PipelineLayout],
    entry_point: &'a CString,
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
    fallback_texture: &'a VulkanImage,
    fallback_texture_staging: &'a VulkanBuffer,
    color_target: vk::Image,
    readback_buffer: vk::Buffer,
}

#[derive(Clone, Debug)]
struct ReadbackFrame {
    rgba: Vec<u8>,
    checksum: u64,
    nonzero_pixels: usize,
}

impl ReadbackFrame {
    fn byte_len(&self) -> usize {
        self.rgba.len()
    }
}

#[derive(Clone, Debug)]
struct ShaderModuleSources {
    vertex: Vec<u32>,
    fragment: Vec<u32>,
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
        let images = frame
            .textures
            .iter()
            .map(|texture| {
                self.create_image(
                    texture.upload.format,
                    texture.upload.extent,
                    texture.image_usage,
                    vk::ImageAspectFlags::COLOR,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let texture_staging_buffers = frame
            .textures
            .iter()
            .map(|texture| {
                self.create_host_buffer(vk::BufferUsageFlags::TRANSFER_SRC, &texture.upload.rgba)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fallback_texture = self.create_image(
            vk::Format::R8G8B8A8_SRGB,
            vk::Extent3D {
                width: 1,
                height: 1,
                depth: 1,
            },
            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
            vk::ImageAspectFlags::COLOR,
        )?;
        let fallback_texture_staging =
            self.create_host_buffer(vk::BufferUsageFlags::TRANSFER_SRC, &[255, 255, 255, 255])?;
        let uniform_buffers = frame
            .uniforms
            .iter()
            .map(|uniform| {
                self.create_host_buffer(vk::BufferUsageFlags::UNIFORM_BUFFER, &uniform.bytes)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let color_format = vk::Format::R8G8B8A8_UNORM;
        let depth_format = vk::Format::D32_SFLOAT;
        let color_target = self.create_image(
            color_format,
            vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            },
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
            vk::ImageAspectFlags::COLOR,
        )?;
        let depth_target = self.create_image(
            depth_format,
            vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            },
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            vk::ImageAspectFlags::DEPTH,
        )?;
        let readback_len = extent.width as usize * extent.height as usize * 4;
        let readback = self.create_host_buffer(
            vk::BufferUsageFlags::TRANSFER_DST,
            &vec![0_u8; readback_len],
        )?;
        let descriptor_set_layouts = frame
            .descriptor_sets
            .iter()
            .map(|set| {
                self.create_descriptor_set_layout(set.bindings.iter().map(|binding| {
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(binding.binding)
                        .descriptor_type(binding.descriptor_type)
                        .descriptor_count(1)
                        .stage_flags(binding.stage_flags)
                }))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let descriptor_pool = self.create_descriptor_pool(frame)?;
        let descriptor_sets =
            self.allocate_descriptor_sets(descriptor_pool, &descriptor_set_layouts)?;
        let samplers = self.create_samplers(frame)?;
        self.update_descriptor_sets(
            frame,
            &descriptor_sets,
            &uniform_buffers,
            &images,
            &fallback_texture,
            &samplers,
        )?;
        let pipeline_layouts = descriptor_set_layouts
            .iter()
            .map(|layout| {
                let layouts = [*layout];
                let info = vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts);
                unsafe { self.device.create_pipeline_layout(&info, None) }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let render_pass = self.create_render_pass(color_format, depth_format)?;
        let framebuffer =
            self.create_framebuffer(render_pass, color_target.view, depth_target.view, extent)?;
        let vertex_shader = self.create_shader_module(&shaders.vertex)?;
        let fragment_shader = self.create_shader_module(&shaders.fragment)?;
        let shader_modules = vec![vertex_shader, fragment_shader];
        let entry_point = CString::new("main")?;
        let pipeline_context = PipelineBuildContext {
            render_pass,
            extent,
            vertex_shader,
            fragment_shader,
            pipeline_layouts: &pipeline_layouts,
            entry_point: &entry_point,
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
            fallback_texture: &fallback_texture,
            fallback_texture_staging: &fallback_texture_staging,
            color_target: color_target.image,
            readback_buffer: readback.buffer,
        };
        let command_buffers = self.record_command_buffers(frame, &command_context)?;
        Ok(VulkanFrameResources {
            buffers,
            images,
            texture_staging_buffers,
            fallback_texture,
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
        usage: vk::ImageUsageFlags,
        aspect_mask: vk::ImageAspectFlags,
    ) -> Result<VulkanImage, Box<dyn Error>> {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(extent)
            .mip_levels(1)
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
            .level_count(1)
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

    fn create_descriptor_set_layout<I>(
        &self,
        bindings: I,
    ) -> Result<vk::DescriptorSetLayout, vk::Result>
    where
        I: IntoIterator<Item = vk::DescriptorSetLayoutBinding<'static>>,
    {
        let bindings = bindings.into_iter().collect::<Vec<_>>();
        let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        unsafe { self.device.create_descriptor_set_layout(&info, None) }
    }

    fn create_descriptor_pool(
        &self,
        frame: &AshRendererFrame,
    ) -> Result<vk::DescriptorPool, vk::Result> {
        let uniform_count = frame
            .descriptor_sets
            .iter()
            .flat_map(|set| &set.bindings)
            .filter(|binding| binding.descriptor_type == vk::DescriptorType::UNIFORM_BUFFER)
            .count()
            .max(1) as u32;
        let sampler_count = frame
            .descriptor_sets
            .iter()
            .flat_map(|set| &set.bindings)
            .filter(|binding| binding.descriptor_type == vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .count()
            .max(1) as u32;
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: uniform_count,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: sampler_count,
            },
        ];
        let info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(frame.descriptor_sets.len().max(1) as u32)
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
            .filter(|binding| binding.descriptor_type == vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
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
            .min_lod(0.0)
            .max_lod(0.0);
        unsafe { self.device.create_sampler(&info, None) }
    }

    fn update_descriptor_sets(
        &self,
        frame: &AshRendererFrame,
        descriptor_sets: &[vk::DescriptorSet],
        uniform_buffers: &[VulkanBuffer],
        images: &[VulkanImage],
        fallback_texture: &VulkanImage,
        samplers: &[vk::Sampler],
    ) -> Result<(), Box<dyn Error>> {
        let mut sampler_index = 0;
        for (set_index, set) in frame.descriptor_sets.iter().enumerate() {
            let descriptor_set = descriptor_sets[set_index];
            for binding in &set.bindings {
                match binding.descriptor_type {
                    vk::DescriptorType::UNIFORM_BUFFER => {
                        let uniform = uniform_buffers
                            .get(binding.uniform_upload_index.ok_or(
                                "uniform descriptor binding is missing a uniform upload index",
                            )?)
                            .ok_or("descriptor set references a missing uniform buffer")?;
                        let buffer_info = [vk::DescriptorBufferInfo::default()
                            .buffer(uniform.buffer)
                            .offset(0)
                            .range(vk::WHOLE_SIZE)];
                        let write = [vk::WriteDescriptorSet::default()
                            .dst_set(descriptor_set)
                            .dst_binding(binding.binding)
                            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                            .buffer_info(&buffer_info)];
                        unsafe {
                            self.device.update_descriptor_sets(&write, &[]);
                        }
                    }
                    vk::DescriptorType::COMBINED_IMAGE_SAMPLER => {
                        let sampler = *samplers
                            .get(sampler_index)
                            .ok_or("descriptor set references a missing sampler")?;
                        sampler_index += 1;
                        let image = binding
                            .texture_upload_index
                            .and_then(|index| images.get(index))
                            .unwrap_or(fallback_texture);
                        let image_info = [vk::DescriptorImageInfo::default()
                            .sampler(sampler)
                            .image_view(image.view)
                            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                        let write = [vk::WriteDescriptorSet::default()
                            .dst_set(descriptor_set)
                            .dst_binding(binding.binding)
                            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                            .image_info(&image_info)];
                        unsafe {
                            self.device.update_descriptor_sets(&write, &[]);
                        }
                    }
                    other => {
                        return Err(format!(
                            "unsupported ash descriptor type in example renderer: {other:?}"
                        )
                        .into());
                    }
                }
            }
        }
        Ok(())
    }

    fn create_render_pass(
        &self,
        color_format: vk::Format,
        depth_format: vk::Format,
    ) -> Result<vk::RenderPass, vk::Result> {
        let attachments = [
            vk::AttachmentDescription::default()
                .format(color_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            vk::AttachmentDescription::default()
                .format(depth_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
        ];
        let color_attachment = [vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        }];
        let depth_attachment = vk::AttachmentReference {
            attachment: 1,
            layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
        };
        let subpass = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment)
            .depth_stencil_attachment(&depth_attachment)];
        let dependency = [vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            )];
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
        let info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(&attachments)
            .width(extent.width)
            .height(extent.height)
            .layers(1);
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
                .name(context.entry_point),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(context.fragment_shader)
                .name(context.entry_point),
        ];
        let layout = context.pipeline_layouts[pipeline.descriptor_set_index];
        let vertex_binding = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: pipeline.vertex_stride,
            input_rate: vk::VertexInputRate::VERTEX,
        }];
        let vertex_attributes = pipeline
            .vertex_attributes
            .iter()
            .map(vertex_attribute_description)
            .collect::<Vec<_>>();
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_binding)
            .vertex_attribute_descriptions(&vertex_attributes);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(pipeline.key.topology)
            .primitive_restart_enable(false);
        let viewport = [vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: context.extent.width as f32,
            height: context.extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }];
        let scissor = [vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: context.extent,
        }];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewport)
            .scissors(&scissor);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(pipeline.key.cull_mode)
            .front_face(pipeline.key.front_face)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(pipeline.key.depth_test_enable)
            .depth_write_enable(pipeline.key.depth_write_enable)
            .depth_compare_op(pipeline.key.depth_compare_op);
        let color_attachment = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(pipeline.key.blend_enable)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            )];
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
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(context.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffers = unsafe { self.device.allocate_command_buffers(&allocate_info)? };
        let command_buffer = command_buffers[0];
        let begin_info = vk::CommandBufferBeginInfo::default();
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 0.0],
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];
        let render_area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: context.extent,
        };
        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(context.render_pass)
            .framebuffer(context.framebuffer)
            .render_area(render_area)
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
            for draw in &frame.draw_calls {
                let Some(pipeline_plan_index) = draw.pipeline_plan_index else {
                    continue;
                };
                let Some((pipeline_index, pipeline_plan)) = frame
                    .pipelines
                    .iter()
                    .enumerate()
                    .find(|(_, pipeline)| pipeline.pipeline_plan_index == pipeline_plan_index)
                else {
                    continue;
                };
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    context.pipelines[pipeline_index],
                );
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &[context.buffers[draw.vertex_buffer_index].buffer],
                    &[0],
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    context.buffers[draw.index_buffer_index].buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                if let Some(descriptor_set_index) = draw.descriptor_set_index {
                    self.device.cmd_bind_descriptor_sets(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        context.pipeline_layouts[pipeline_plan.descriptor_set_index],
                        0,
                        &[context.descriptor_sets[descriptor_set_index]],
                        &[],
                    );
                }
                self.device
                    .cmd_draw_indexed(command_buffer, draw.index_count, 1, 0, 0, 0);
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
        frame: &AshRendererFrame,
        context: &CommandRecordContext<'_>,
    ) {
        for ((texture, image), staging) in frame
            .textures
            .iter()
            .zip(context.texture_images)
            .zip(context.texture_staging_buffers)
        {
            self.record_texture_upload(
                command_buffer,
                image.image,
                staging.buffer,
                texture.upload.extent,
            );
        }
        self.record_texture_upload(
            command_buffer,
            context.fallback_texture.image,
            context.fallback_texture_staging.buffer,
            vk::Extent3D {
                width: 1,
                height: 1,
                depth: 1,
            },
        );
    }

    fn record_texture_upload(
        &self,
        command_buffer: vk::CommandBuffer,
        image: vk::Image,
        staging_buffer: vk::Buffer,
        extent: vk::Extent3D,
    ) {
        let subresource_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
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
            self.device.cmd_copy_buffer_to_image(
                command_buffer,
                staging_buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(extent)],
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
        Ok(ReadbackFrame {
            checksum: fnv1a64(&bytes),
            nonzero_pixels: bytes
                .chunks_exact(4)
                .filter(|pixel| pixel.iter().any(|channel| *channel != 0))
                .count(),
            rgba: bytes,
        })
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
            self.device
                .destroy_image_view(resources.fallback_texture.view, None);
            self.device
                .destroy_image(resources.fallback_texture.image, None);
            self.device
                .free_memory(resources.fallback_texture.memory, None);
            self.device
                .destroy_buffer(resources.fallback_texture_staging.buffer, None);
            self.device
                .free_memory(resources.fallback_texture_staging.memory, None);
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

fn vertex_attribute_description(
    attribute: &AshVertexAttributePlan,
) -> vk::VertexInputAttributeDescription {
    vk::VertexInputAttributeDescription {
        location: attribute.location,
        binding: attribute.binding,
        format: attribute.format,
        offset: attribute.offset,
    }
}

fn default_sampler_plan() -> AshSamplerPlan {
    AshSamplerPlan {
        mag_filter: vk::Filter::LINEAR,
        min_filter: vk::Filter::LINEAR,
        mipmap_mode: vk::SamplerMipmapMode::LINEAR,
        address_mode_u: vk::SamplerAddressMode::REPEAT,
        address_mode_v: vk::SamplerAddressMode::REPEAT,
        normal_map_decode: false,
    }
}

fn shader_sources_from_options(options: &Options) -> Result<ShaderModuleSources, Box<dyn Error>> {
    match (&options.vertex_spv, &options.fragment_spv) {
        (Some(vertex), Some(fragment)) => Ok(ShaderModuleSources {
            vertex: read_spirv_words(vertex)?,
            fragment: read_spirv_words(fragment)?,
            source: ShaderSourceKind::ExternalSpirv,
        }),
        (None, None) => Ok(ShaderModuleSources {
            vertex: MINIMAL_VERTEX_SPV.to_vec(),
            fragment: MINIMAL_FRAGMENT_SPV.to_vec(),
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

fn write_rgba_json(
    path: &Path,
    options: &Options,
    frame: &AshRendererFrame,
    shaders: ShaderSourceKind,
    readback: &ReadbackFrame,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let artifact = json!({
        "generator": "vrm-rs crates/vrm-adapter-ash/examples/unsafe_device_renderer.rs",
        "fixture": options.frame.avatar.to_string_lossy(),
        "animation": (!options.frame.no_animation).then(|| options.frame.animation.to_string_lossy().to_string()),
        "time": options.frame.time,
        "width": width,
        "height": height,
        "renderer": {
            "backend": "ash",
            "physicalDevice": "local-vulkan-device",
            "shaderSource": shader_source_label(shaders),
            "graphicsPipelines": frame.pipelines.len(),
            "drawCalls": frame.draw_calls.len(),
        },
        "readback": {
            "checksum": format!("{:016x}", readback.checksum),
            "nonzeroPixels": readback.nonzero_pixels,
        },
        "format": "rgba8",
        "rgba": &readback.rgba,
    });
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&artifact)?),
    )?;
    Ok(())
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
    let readback = ReadbackFrame {
        checksum: fnv1a64(&rgba),
        nonzero_pixels: 2,
        rgba: rgba.clone(),
    };
    let json_path = options.out.clone().unwrap_or_else(|| {
        PathBuf::from("target/ash-artifact-self-test/ash-self-test.frame000.rgba.json")
    });
    let imqraw_path = options.imqraw_out.clone().unwrap_or_else(|| {
        PathBuf::from("target/ash-artifact-self-test/ash-self-test.frame000.imqraw")
    });
    write_rgba_json(
        &json_path,
        options,
        &AshRendererFrame::default(),
        ShaderSourceKind::BuiltInSmoke,
        &readback,
        width,
        height,
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

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse();
    if options.artifact_self_test {
        return run_artifact_self_test(&options);
    }
    if options.dry_run {
        println!("dry run: parsed ash unsafe device renderer options");
        return Ok(());
    }
    let frame_plan = frame_plan_from_options(&options.frame)?;
    let renderer_frame = ash_renderer_frame_from_plan(&frame_plan);
    let shaders = shader_sources_from_options(&options)?;
    let renderer = UnsafeAshDeviceRenderer::new()?;
    let resources = renderer.materialize_frame(
        &renderer_frame,
        vk::Extent2D {
            width: options.width.max(1),
            height: options.height.max(1),
        },
        &shaders,
    )?;
    println!(
        "unsafe ash device renderer: {} buffers, {} images, {} descriptor sets, {} graphics pipelines, {} recorded command buffers, {} draw plans, {} shaders on physical device {:?}",
        resources.buffers.len(),
        resources.images.len(),
        resources.descriptor_sets.len(),
        resources.pipelines.len(),
        resources.command_buffers.len(),
        renderer_frame.draw_calls.len(),
        shader_source_label(shaders.source),
        renderer.physical_device
    );
    if options.submit_readback || options.out.is_some() || options.imqraw_out.is_some() {
        let summary = renderer.submit_and_readback(&resources)?;
        println!(
            "ash offscreen readback: {} bytes, {} nonzero pixels, checksum {:016x}",
            summary.byte_len(),
            summary.nonzero_pixels,
            summary.checksum
        );
        if let Some(path) = &options.out {
            write_rgba_json(
                path,
                &options,
                &renderer_frame,
                shaders.source,
                &summary,
                options.width.max(1),
                options.height.max(1),
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
