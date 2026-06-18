use ash::{Entry, vk};
use clap::Parser;
use std::{error::Error, ffi::CString, ptr};
use vrm_adapter_ash::{
    AshRendererFrame, AshVrmFramePlanOptions, ash_renderer_frame_from_plan, frame_plan_from_options,
};

#[derive(Clone, Debug, Parser)]
#[command(about = "Materialize a VRM frame plan into real ash Vulkan buffers and descriptors")]
struct Options {
    #[command(flatten)]
    frame: AshVrmFramePlanOptions,
    /// Only print help/parse inputs; useful for CI smoke checks.
    #[arg(long)]
    dry_run: bool,
}

struct VulkanFrameResources {
    buffers: Vec<VulkanBuffer>,
    images: Vec<VulkanImage>,
    descriptor_set_layouts: Vec<vk::DescriptorSetLayout>,
    descriptor_pool: vk::DescriptorPool,
    descriptor_sets: Vec<vk::DescriptorSet>,
    pipeline_layouts: Vec<vk::PipelineLayout>,
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
                self.create_sampled_image(
                    texture.upload.format,
                    texture.upload.extent,
                    texture.image_usage,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
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
        let pipeline_layouts = descriptor_set_layouts
            .iter()
            .map(|layout| {
                let layouts = [*layout];
                let info = vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts);
                unsafe { self.device.create_pipeline_layout(&info, None) }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(VulkanFrameResources {
            buffers,
            images,
            descriptor_set_layouts,
            descriptor_pool,
            descriptor_sets,
            pipeline_layouts,
            command_pool,
        })
    }

    fn create_upload_buffer(
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
            let mapped = self
                .device
                .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())?;
            ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
            self.device.unmap_memory(memory);
        }
        Ok(VulkanBuffer { buffer, memory })
    }

    fn create_sampled_image(
        &self,
        format: vk::Format,
        extent: vk::Extent3D,
        usage: vk::ImageUsageFlags,
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
            .aspect_mask(vk::ImageAspectFlags::COLOR)
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
            for layout in resources.pipeline_layouts {
                self.device.destroy_pipeline_layout(layout, None);
            }
            self.device
                .destroy_descriptor_pool(resources.descriptor_pool, None);
            for layout in resources.descriptor_set_layouts {
                self.device.destroy_descriptor_set_layout(layout, None);
            }
            for image in resources.images {
                self.device.destroy_image_view(image.view, None);
                self.device.destroy_image(image.image, None);
                self.device.free_memory(image.memory, None);
            }
            for buffer in resources.buffers {
                self.device.destroy_buffer(buffer.buffer, None);
                self.device.free_memory(buffer.memory, None);
            }
        }
    }
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
    if options.dry_run {
        println!("dry run: parsed ash unsafe device renderer options");
        return Ok(());
    }
    let frame_plan = frame_plan_from_options(&options.frame)?;
    let renderer_frame = ash_renderer_frame_from_plan(&frame_plan);
    let renderer = UnsafeAshDeviceRenderer::new()?;
    let resources = renderer.materialize_frame(&renderer_frame)?;
    println!(
        "unsafe ash device renderer: {} buffers, {} images, {} descriptor sets, {} pipeline layouts, {} draw plans on physical device {:?}",
        resources.buffers.len(),
        resources.images.len(),
        resources.descriptor_sets.len(),
        resources.pipeline_layouts.len(),
        renderer_frame.draw_calls.len(),
        renderer.physical_device
    );
    renderer.destroy_frame_resources(resources);
    Ok(())
}
