//! Windowed ash/Vulkan viewer for a VRM avatar plus an optional VRMA clip.
//!
//! This example deliberately keeps the crate boundary intact: `vrm-adapter-ash`
//! plans and CPU-bakes VRM geometry, while the example owns the unsafe Vulkan
//! instance/device/surface/swapchain edge.

use ash::khr::{surface, swapchain};
use ash::{Entry, vk};
use bytemuck::{Pod, Zeroable};
use clap::Parser;
use glam::{Mat4, Vec3, Vec4};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::error::Error;
use std::ffi::CString;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use vrm_adapter::ScreenProjectionSize;
use vrm_adapter_ash::{
    AshRenderOptions, AshVrmFramePlanOptions, AshVrmFramePlanner, AshVrmPrimitive, AshVrmVertex,
};

#[derive(Clone, Debug, Parser)]
#[command(about = "Open a real ash/Vulkan window and draw a VRM avatar")]
struct Options {
    /// VRM avatar file.
    #[arg(long, default_value = ".external-fixtures/official/Seed-san.vrm")]
    avatar: PathBuf,
    /// Optional VRMA animation clip file.
    #[arg(long, default_value = ".external-fixtures/official/idle_loop.vrma")]
    animation: PathBuf,
    /// Disable VRMA sampling after loading the avatar.
    #[arg(long)]
    no_animation: bool,
    /// Sample time in seconds.
    #[arg(long, default_value_t = 0.0)]
    time: f32,
    /// Initial window width.
    #[arg(long, default_value_t = 1280)]
    width: u32,
    /// Initial window height.
    #[arg(long, default_value_t = 720)]
    height: u32,
    /// Vertex SPIR-V compiled from `shaders/windowed_simple.vert.glsl`.
    #[arg(
        long,
        default_value = "target/ash-windowed-simple-shaders/mtoon_base.vert.spv"
    )]
    vertex_spv: PathBuf,
    /// Fragment SPIR-V compiled from `shaders/windowed_simple.frag.glsl`.
    #[arg(
        long,
        default_value = "target/ash-windowed-simple-shaders/mtoon_base.frag.spv"
    )]
    fragment_spv: PathBuf,
    /// Exit after rendering this many frames. Useful for smoke tests.
    #[arg(long)]
    max_frames: Option<u64>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SimpleVertex {
    position: [f32; 3],
    color: [f32; 4],
}

#[derive(Clone, Debug)]
struct SimpleMesh {
    vertices: Vec<SimpleVertex>,
    indices: Vec<u32>,
}

#[derive(Clone, Debug)]
struct ShaderModules {
    vertex: Vec<u32>,
    fragment: Vec<u32>,
}

struct App {
    options: Options,
    mesh: SimpleMesh,
    shaders: ShaderModules,
    window: Option<Window>,
    renderer: Option<WindowedAshRenderer>,
    recreate_swapchain: bool,
    rendered_frames: u64,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title("vrm-rs ash windowed viewer")
            .with_inner_size(PhysicalSize::new(self.options.width, self.options.height));
        let window = event_loop.create_window(attributes).expect("create window");
        let renderer = WindowedAshRenderer::new(&window, &self.mesh, &self.shaders)
            .expect("initialize ash windowed renderer");
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = &self.window else {
            return;
        };
        if window.id() != window_id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.recreate_swapchain = size.width > 0 && size.height > 0;
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = &mut self.renderer {
                    if self.recreate_swapchain {
                        renderer
                            .recreate_swapchain(window)
                            .expect("recreate swapchain");
                        self.recreate_swapchain = false;
                    }
                    match renderer.render(window) {
                        Ok(RenderStatus::Ok) => {
                            self.rendered_frames = self.rendered_frames.saturating_add(1);
                            if self
                                .options
                                .max_frames
                                .is_some_and(|max| self.rendered_frames >= max)
                            {
                                event_loop.exit();
                            }
                        }
                        Ok(RenderStatus::NeedsRecreate) => self.recreate_swapchain = true,
                        Err(error) => eprintln!("ash windowed render failed: {error}"),
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse();
    let mesh = build_simple_mesh(&options)?;
    let shaders = ShaderModules {
        vertex: read_spirv_words(&options.vertex_spv)?,
        fragment: read_spirv_words(&options.fragment_spv)?,
    };
    let event_loop = EventLoop::new()?;
    let mut app = App {
        options,
        mesh,
        shaders,
        window: None,
        renderer: None,
        recreate_swapchain: false,
        rendered_frames: 0,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn build_simple_mesh(options: &Options) -> Result<SimpleMesh, Box<dyn Error>> {
    let animation = (!options.no_animation).then_some(options.animation.clone());
    let mut planner = AshVrmFramePlanner::from_paths(options.avatar.clone(), animation)?;
    let aspect = options.width.max(1) as f32 / options.height.max(1) as f32;
    let mut frame_options = AshVrmFramePlanOptions::parse_from(["ash-windowed-viewer"]);
    frame_options.avatar = options.avatar.clone();
    frame_options.animation = options.animation.clone();
    frame_options.no_animation = options.no_animation;
    frame_options.time = options.time;
    let scene_options = frame_options.scene_options_with_screen_size(
        aspect,
        ScreenProjectionSize {
            width: options.width.max(1) as f32,
            height: options.height.max(1) as f32,
        },
    );
    let plan = planner.sample_frame_with_full_render_options(
        options.time,
        scene_options,
        AshRenderOptions {
            disable_outlines: true,
            ..AshRenderOptions::default()
        },
    )?;
    let view_projection = Mat4::from_cols_array_2d(&plan.scene_uniform.view_projection);
    let light = Vec3::new(
        plan.scene_uniform.light_dir[0],
        plan.scene_uniform.light_dir[1],
        plan.scene_uniform.light_dir[2],
    )
    .normalize_or_zero();

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for primitive in &plan.primitives {
        append_primitive(
            primitive,
            view_projection,
            light,
            &mut vertices,
            &mut indices,
        )?;
    }
    if vertices.is_empty() || indices.is_empty() {
        return Err("VRM frame produced no drawable vertices".into());
    }
    Ok(SimpleMesh { vertices, indices })
}

fn append_primitive(
    primitive: &AshVrmPrimitive,
    view_projection: Mat4,
    light: Vec3,
    vertices: &mut Vec<SimpleVertex>,
    indices: &mut Vec<u32>,
) -> Result<(), Box<dyn Error>> {
    let base = u32::try_from(vertices.len())?;
    vertices.extend(
        primitive
            .vertices
            .iter()
            .map(|vertex| simple_vertex(vertex, view_projection, light)),
    );
    indices.extend(primitive.indices.iter().map(|index| base + *index));
    Ok(())
}

fn simple_vertex(vertex: &AshVrmVertex, view_projection: Mat4, light: Vec3) -> SimpleVertex {
    let world = Vec4::new(
        vertex.position[0],
        vertex.position[1],
        vertex.position[2],
        1.0,
    );
    let clip = view_projection * world;
    let ndc = if clip.w.abs() > f32::EPSILON {
        clip.truncate() / clip.w
    } else {
        clip.truncate()
    };
    let normal = Vec3::from_array(vertex.normal).normalize_or_zero();
    let lambert = normal.dot(-light).max(0.0);
    let shade = 0.35 + 0.65 * lambert;
    SimpleVertex {
        position: [ndc.x, ndc.y, ndc.z],
        color: [
            vertex.color_0[0] * shade,
            vertex.color_0[1] * shade,
            vertex.color_0[2] * shade,
            vertex.color_0[3],
        ],
    }
}

fn read_spirv_words(path: &PathBuf) -> Result<Vec<u32>, Box<dyn Error>> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "failed to read {}: {error}; run `just ash-windowed-simple-shaders` first",
            path.display()
        )
    })?;
    let words = ash::util::read_spv(&mut Cursor::new(bytes))?;
    Ok(words)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderStatus {
    Ok,
    NeedsRecreate,
}

struct WindowedAshRenderer {
    _entry: Entry,
    instance: ash::Instance,
    surface_loader: surface::Instance,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    swapchain_loader: swapchain::Device,
    queue: vk::Queue,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    command_pool: vk::CommandPool,
    vertex_buffer: VulkanBuffer,
    index_buffer: VulkanBuffer,
    index_count: u32,
    vertex_spv: Vec<u32>,
    fragment_spv: Vec<u32>,
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    in_flight: vk::Fence,
    swapchain: SwapchainResources,
}

impl WindowedAshRenderer {
    fn new(
        window: &Window,
        mesh: &SimpleMesh,
        shaders: &ShaderModules,
    ) -> Result<Self, Box<dyn Error>> {
        let entry = unsafe { Entry::load()? };
        let app_name = CString::new("vrm-rs ash windowed viewer")?;
        let engine_name = CString::new("vrm-rs")?;
        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(1)
            .engine_name(&engine_name)
            .engine_version(1)
            .api_version(vk::API_VERSION_1_0);
        let display_handle = window.display_handle()?.as_raw();
        let window_handle = window.window_handle()?.as_raw();
        let instance_extensions = ash_window::enumerate_required_extensions(display_handle)?;
        let instance_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(instance_extensions);
        let instance = unsafe { entry.create_instance(&instance_info, None)? };
        let surface = unsafe {
            ash_window::create_surface(&entry, &instance, display_handle, window_handle, None)?
        };
        let surface_loader = surface::Instance::new(&entry, &instance);
        let (physical_device, queue_family_index) =
            select_physical_device(&instance, &surface_loader, surface)?;
        let queue_priorities = [1.0_f32];
        let queue_infos = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&queue_priorities)];
        let device_extensions = [swapchain::NAME.as_ptr()];
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&device_extensions);
        let device = unsafe { instance.create_device(physical_device, &device_info, None)? };
        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
        let swapchain_loader = swapchain::Device::new(&instance, &device);
        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };
        let command_pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = unsafe { device.create_command_pool(&command_pool_info, None)? };
        let memory = MemoryContext {
            instance: &instance,
            physical_device,
            device: &device,
            memory_properties,
        };
        let vertex_buffer = memory.create_host_buffer(
            vk::BufferUsageFlags::VERTEX_BUFFER,
            bytemuck::cast_slice(&mesh.vertices),
        )?;
        let index_buffer = memory.create_host_buffer(
            vk::BufferUsageFlags::INDEX_BUFFER,
            bytemuck::cast_slice(&mesh.indices),
        )?;
        let image_available =
            unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)? };
        let render_finished =
            unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)? };
        let in_flight = unsafe {
            device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )?
        };
        let swapchain = create_swapchain_resources(
            SwapchainCreateContext {
                instance: &instance,
                physical_device,
                device: &device,
                surface_loader: &surface_loader,
                surface,
                swapchain_loader: &swapchain_loader,
                command_pool,
                memory_properties,
                vertex_spv: &shaders.vertex,
                fragment_spv: &shaders.fragment,
                vertex_buffer: vertex_buffer.buffer,
                index_buffer: index_buffer.buffer,
                index_count: u32::try_from(mesh.indices.len())?,
                old_swapchain: vk::SwapchainKHR::null(),
            },
            window,
        )?;
        Ok(Self {
            _entry: entry,
            instance,
            surface_loader,
            surface,
            physical_device,
            device,
            swapchain_loader,
            queue,
            memory_properties,
            command_pool,
            vertex_buffer,
            index_buffer,
            index_count: u32::try_from(mesh.indices.len())?,
            vertex_spv: shaders.vertex.clone(),
            fragment_spv: shaders.fragment.clone(),
            image_available,
            render_finished,
            in_flight,
            swapchain,
        })
    }

    fn render(&mut self, window: &Window) -> Result<RenderStatus, Box<dyn Error>> {
        unsafe {
            self.device
                .wait_for_fences(&[self.in_flight], true, u64::MAX)?;
            self.device.reset_fences(&[self.in_flight])?;
        }
        let acquired = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain.swapchain,
                u64::MAX,
                self.image_available,
                vk::Fence::null(),
            )
        };
        let (image_index, suboptimal) = match acquired {
            Ok(value) => value,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(RenderStatus::NeedsRecreate),
            Err(error) => return Err(error.into()),
        };
        if window.inner_size().width == 0 || window.inner_size().height == 0 {
            return Ok(RenderStatus::Ok);
        }
        let wait_semaphores = [self.image_available];
        let signal_semaphores = [self.render_finished];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let command_buffers = [self.swapchain.command_buffers[image_index as usize]];
        let submit = [vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores)];
        unsafe {
            self.device
                .queue_submit(self.queue, &submit, self.in_flight)?;
        }
        let swapchains = [self.swapchain.swapchain];
        let image_indices = [image_index];
        let present = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);
        let present_result = unsafe { self.swapchain_loader.queue_present(self.queue, &present) };
        match present_result {
            Ok(present_suboptimal) if suboptimal || present_suboptimal => {
                Ok(RenderStatus::NeedsRecreate)
            }
            Ok(_) => Ok(RenderStatus::Ok),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(RenderStatus::NeedsRecreate),
            Err(error) => Err(error.into()),
        }
    }

    fn recreate_swapchain(&mut self, window: &Window) -> Result<(), Box<dyn Error>> {
        if window.inner_size().width == 0 || window.inner_size().height == 0 {
            return Ok(());
        }
        unsafe {
            self.device.device_wait_idle()?;
        }
        let old_swapchain = self.swapchain.swapchain;
        let old = std::mem::replace(&mut self.swapchain, SwapchainResources::empty());
        self.destroy_swapchain_resources(old, false);
        self.swapchain = create_swapchain_resources(
            SwapchainCreateContext {
                instance: &self.instance,
                physical_device: self.physical_device,
                device: &self.device,
                surface_loader: &self.surface_loader,
                surface: self.surface,
                swapchain_loader: &self.swapchain_loader,
                command_pool: self.command_pool,
                memory_properties: self.memory_properties,
                vertex_spv: &self.vertex_spv,
                fragment_spv: &self.fragment_spv,
                vertex_buffer: self.vertex_buffer.buffer,
                index_buffer: self.index_buffer.buffer,
                index_count: self.index_count,
                old_swapchain,
            },
            window,
        )?;
        unsafe {
            self.swapchain_loader.destroy_swapchain(old_swapchain, None);
        }
        Ok(())
    }

    fn destroy_swapchain_resources(&self, resources: SwapchainResources, destroy_swapchain: bool) {
        unsafe {
            if !resources.command_buffers.is_empty() {
                self.device
                    .free_command_buffers(self.command_pool, &resources.command_buffers);
            }
            for framebuffer in resources.framebuffers {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            self.device.destroy_pipeline(resources.pipeline, None);
            self.device
                .destroy_pipeline_layout(resources.pipeline_layout, None);
            self.device.destroy_render_pass(resources.render_pass, None);
            self.device.destroy_image_view(resources.depth.view, None);
            self.device.destroy_image(resources.depth.image, None);
            self.device.free_memory(resources.depth.memory, None);
            for view in resources.image_views {
                self.device.destroy_image_view(view, None);
            }
            if destroy_swapchain {
                self.swapchain_loader
                    .destroy_swapchain(resources.swapchain, None);
            }
        }
    }
}

impl Drop for WindowedAshRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
        }
        let resources = std::mem::replace(&mut self.swapchain, SwapchainResources::empty());
        self.destroy_swapchain_resources(resources, true);
        unsafe {
            self.device.destroy_fence(self.in_flight, None);
            self.device.destroy_semaphore(self.render_finished, None);
            self.device.destroy_semaphore(self.image_available, None);
            self.device.destroy_buffer(self.index_buffer.buffer, None);
            self.device.free_memory(self.index_buffer.memory, None);
            self.device.destroy_buffer(self.vertex_buffer.buffer, None);
            self.device.free_memory(self.vertex_buffer.memory, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}

struct SwapchainCreateContext<'a> {
    instance: &'a ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &'a ash::Device,
    surface_loader: &'a surface::Instance,
    surface: vk::SurfaceKHR,
    swapchain_loader: &'a swapchain::Device,
    command_pool: vk::CommandPool,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    vertex_spv: &'a [u32],
    fragment_spv: &'a [u32],
    vertex_buffer: vk::Buffer,
    index_buffer: vk::Buffer,
    index_count: u32,
    old_swapchain: vk::SwapchainKHR,
}

struct SwapchainResources {
    swapchain: vk::SwapchainKHR,
    image_views: Vec<vk::ImageView>,
    depth: VulkanImage,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    framebuffers: Vec<vk::Framebuffer>,
    command_buffers: Vec<vk::CommandBuffer>,
}

impl SwapchainResources {
    fn empty() -> Self {
        Self {
            swapchain: vk::SwapchainKHR::null(),
            image_views: Vec::new(),
            depth: VulkanImage::empty(),
            render_pass: vk::RenderPass::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            framebuffers: Vec::new(),
            command_buffers: Vec::new(),
        }
    }
}

fn create_swapchain_resources(
    context: SwapchainCreateContext<'_>,
    window: &Window,
) -> Result<SwapchainResources, Box<dyn Error>> {
    let support = surface_support(&context, window)?;
    let swapchain_info = vk::SwapchainCreateInfoKHR::default()
        .surface(context.surface)
        .min_image_count(support.image_count)
        .image_format(support.format.format)
        .image_color_space(support.format.color_space)
        .image_extent(support.extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .pre_transform(support.capabilities.current_transform)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
        .present_mode(support.present_mode)
        .clipped(true)
        .old_swapchain(context.old_swapchain);
    let swapchain = unsafe {
        context
            .swapchain_loader
            .create_swapchain(&swapchain_info, None)?
    };
    let images = unsafe { context.swapchain_loader.get_swapchain_images(swapchain)? };
    let image_views = images
        .iter()
        .map(|image| create_image_view(context.device, *image, support.format.format))
        .collect::<Result<Vec<_>, _>>()?;
    let memory = MemoryContext {
        instance: context.instance,
        physical_device: context.physical_device,
        device: context.device,
        memory_properties: context.memory_properties,
    };
    let depth = memory.create_image(
        vk::Format::D32_SFLOAT,
        vk::Extent3D {
            width: support.extent.width,
            height: support.extent.height,
            depth: 1,
        },
        vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
        vk::ImageAspectFlags::DEPTH,
    )?;
    let render_pass = create_render_pass(
        context.device,
        support.format.format,
        vk::Format::D32_SFLOAT,
    )?;
    let (pipeline_layout, pipeline) = create_pipeline(
        context.device,
        render_pass,
        support.extent,
        context.vertex_spv,
        context.fragment_spv,
    )?;
    let framebuffers = image_views
        .iter()
        .map(|view| {
            let attachments = [*view, depth.view];
            let info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(&attachments)
                .width(support.extent.width)
                .height(support.extent.height)
                .layers(1);
            unsafe { context.device.create_framebuffer(&info, None) }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let command_buffers = record_command_buffers(CommandRecordContext {
        device: context.device,
        command_pool: context.command_pool,
        render_pass,
        extent: support.extent,
        framebuffers: &framebuffers,
        pipeline,
        vertex_buffer: context.vertex_buffer,
        index_buffer: context.index_buffer,
        index_count: context.index_count,
    })?;
    Ok(SwapchainResources {
        swapchain,
        image_views,
        depth,
        render_pass,
        pipeline_layout,
        pipeline,
        framebuffers,
        command_buffers,
    })
}

struct SurfaceSupport {
    capabilities: vk::SurfaceCapabilitiesKHR,
    format: vk::SurfaceFormatKHR,
    present_mode: vk::PresentModeKHR,
    extent: vk::Extent2D,
    image_count: u32,
}

fn surface_support(
    context: &SwapchainCreateContext<'_>,
    window: &Window,
) -> Result<SurfaceSupport, Box<dyn Error>> {
    let capabilities = unsafe {
        context
            .surface_loader
            .get_physical_device_surface_capabilities(context.physical_device, context.surface)?
    };
    let formats = unsafe {
        context
            .surface_loader
            .get_physical_device_surface_formats(context.physical_device, context.surface)?
    };
    let present_modes = unsafe {
        context
            .surface_loader
            .get_physical_device_surface_present_modes(context.physical_device, context.surface)?
    };
    let format = formats
        .iter()
        .copied()
        .find(|format| {
            matches!(
                format.format,
                vk::Format::B8G8R8A8_UNORM | vk::Format::R8G8B8A8_UNORM
            )
        })
        .or_else(|| formats.first().copied())
        .ok_or("surface reports no formats")?;
    let present_mode = present_modes
        .iter()
        .copied()
        .find(|mode| *mode == vk::PresentModeKHR::MAILBOX)
        .unwrap_or(vk::PresentModeKHR::FIFO);
    let extent = if capabilities.current_extent.width != u32::MAX {
        capabilities.current_extent
    } else {
        let size = window.inner_size();
        vk::Extent2D {
            width: size.width.clamp(
                capabilities.min_image_extent.width,
                capabilities.max_image_extent.width,
            ),
            height: size.height.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            ),
        }
    };
    let desired = capabilities.min_image_count.saturating_add(1);
    let image_count = if capabilities.max_image_count == 0 {
        desired
    } else {
        desired.min(capabilities.max_image_count)
    };
    Ok(SurfaceSupport {
        capabilities,
        format,
        present_mode,
        extent,
        image_count,
    })
}

fn select_physical_device(
    instance: &ash::Instance,
    surface_loader: &surface::Instance,
    surface: vk::SurfaceKHR,
) -> Result<(vk::PhysicalDevice, u32), Box<dyn Error>> {
    let devices = unsafe { instance.enumerate_physical_devices()? };
    devices
        .into_iter()
        .find_map(|device| {
            let queue_index = unsafe {
                instance
                    .get_physical_device_queue_family_properties(device)
                    .iter()
                    .enumerate()
                    .find_map(|(index, family)| {
                        let supports_graphics =
                            family.queue_flags.contains(vk::QueueFlags::GRAPHICS);
                        let supports_present = surface_loader
                            .get_physical_device_surface_support(device, index as u32, surface)
                            .ok()?;
                        (supports_graphics && supports_present).then_some(index as u32)
                    })
            }?;
            Some((device, queue_index))
        })
        .ok_or_else(|| "no Vulkan physical device supports graphics+present".into())
}

fn create_image_view(
    device: &ash::Device,
    image: vk::Image,
    format: vk::Format,
) -> Result<vk::ImageView, vk::Result> {
    let subresource_range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1);
    let info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(subresource_range);
    unsafe { device.create_image_view(&info, None) }
}

fn create_render_pass(
    device: &ash::Device,
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
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR),
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
    let color_ref = [vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    }];
    let depth_ref = vk::AttachmentReference {
        attachment: 1,
        layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
    };
    let subpass = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_ref)
        .depth_stencil_attachment(&depth_ref)];
    let dependency = [vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];
    let info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpass)
        .dependencies(&dependency);
    unsafe { device.create_render_pass(&info, None) }
}

fn create_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    extent: vk::Extent2D,
    vertex_spv: &[u32],
    fragment_spv: &[u32],
) -> Result<(vk::PipelineLayout, vk::Pipeline), Box<dyn Error>> {
    let vertex_shader = create_shader_module(device, vertex_spv)?;
    let fragment_shader = create_shader_module(device, fragment_spv)?;
    let entry_point = CString::new("main")?;
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex_shader)
            .name(&entry_point),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment_shader)
            .name(&entry_point),
    ];
    let binding = [vk::VertexInputBindingDescription {
        binding: 0,
        stride: std::mem::size_of::<SimpleVertex>() as u32,
        input_rate: vk::VertexInputRate::VERTEX,
    }];
    let attributes = [
        vk::VertexInputAttributeDescription {
            location: 0,
            binding: 0,
            format: vk::Format::R32G32B32_SFLOAT,
            offset: 0,
        },
        vk::VertexInputAttributeDescription {
            location: 1,
            binding: 0,
            format: vk::Format::R32G32B32A32_SFLOAT,
            offset: 12,
        },
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&binding)
        .vertex_attribute_descriptions(&attributes);
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport = [vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    }];
    let scissor = [vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent,
    }];
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewports(&viewport)
        .scissors(&scissor);
    let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let depth = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);
    let color_attachment = [vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
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
    let layout_info = vk::PipelineLayoutCreateInfo::default();
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None)? };
    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterization)
        .multisample_state(&multisample)
        .depth_stencil_state(&depth)
        .color_blend_state(&color_blend)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);
    let pipeline =
        unsafe { device.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None) }
            .map_err(|(_, error)| Box::<dyn Error>::from(error))?
            .into_iter()
            .next()
            .ok_or("Vulkan returned no graphics pipeline")?;
    unsafe {
        device.destroy_shader_module(fragment_shader, None);
        device.destroy_shader_module(vertex_shader, None);
    }
    Ok((pipeline_layout, pipeline))
}

fn create_shader_module(
    device: &ash::Device,
    words: &[u32],
) -> Result<vk::ShaderModule, vk::Result> {
    let info = vk::ShaderModuleCreateInfo::default().code(words);
    unsafe { device.create_shader_module(&info, None) }
}

struct CommandRecordContext<'a> {
    device: &'a ash::Device,
    command_pool: vk::CommandPool,
    render_pass: vk::RenderPass,
    extent: vk::Extent2D,
    framebuffers: &'a [vk::Framebuffer],
    pipeline: vk::Pipeline,
    vertex_buffer: vk::Buffer,
    index_buffer: vk::Buffer,
    index_count: u32,
}

fn record_command_buffers(
    context: CommandRecordContext<'_>,
) -> Result<Vec<vk::CommandBuffer>, Box<dyn Error>> {
    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(context.command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(u32::try_from(context.framebuffers.len())?);
    let command_buffers = unsafe { context.device.allocate_command_buffers(&allocate_info)? };
    for (command_buffer, framebuffer) in command_buffers.iter().zip(context.framebuffers) {
        let begin = vk::CommandBufferBeginInfo::default();
        let clear = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.02, 0.025, 0.03, 1.0],
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];
        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(context.render_pass)
            .framebuffer(*framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: context.extent,
            })
            .clear_values(&clear);
        unsafe {
            context
                .device
                .begin_command_buffer(*command_buffer, &begin)?;
            context.device.cmd_begin_render_pass(
                *command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );
            context.device.cmd_bind_pipeline(
                *command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                context.pipeline,
            );
            context.device.cmd_bind_vertex_buffers(
                *command_buffer,
                0,
                &[context.vertex_buffer],
                &[0],
            );
            context.device.cmd_bind_index_buffer(
                *command_buffer,
                context.index_buffer,
                0,
                vk::IndexType::UINT32,
            );
            context
                .device
                .cmd_draw_indexed(*command_buffer, context.index_count, 1, 0, 0, 0);
            context.device.cmd_end_render_pass(*command_buffer);
            context.device.end_command_buffer(*command_buffer)?;
        }
    }
    Ok(command_buffers)
}

#[derive(Clone, Copy)]
struct MemoryContext<'a> {
    instance: &'a ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &'a ash::Device,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
}

impl MemoryContext<'_> {
    fn create_host_buffer(
        self,
        usage: vk::BufferUsageFlags,
        bytes: &[u8],
    ) -> Result<VulkanBuffer, Box<dyn Error>> {
        let size = bytes.len().max(1) as vk::DeviceSize;
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
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
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
                self.device.unmap_memory(memory);
            }
        }
        Ok(VulkanBuffer { buffer, memory })
    }

    fn create_image(
        self,
        format: vk::Format,
        extent: vk::Extent3D,
        usage: vk::ImageUsageFlags,
        aspect_mask: vk::ImageAspectFlags,
    ) -> Result<VulkanImage, Box<dyn Error>> {
        let info = vk::ImageCreateInfo::default()
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
        let image = unsafe { self.device.create_image(&info, None)? };
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
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(aspect_mask)
                    .level_count(1)
                    .layer_count(1),
            );
        let view = unsafe { self.device.create_image_view(&view_info, None)? };
        Ok(VulkanImage {
            image,
            memory,
            view,
        })
    }

    fn find_memory_type(
        self,
        type_bits: u32,
        properties: vk::MemoryPropertyFlags,
    ) -> Result<u32, Box<dyn Error>> {
        (0..self.memory_properties.memory_type_count)
            .find(|index| {
                let type_supported = (type_bits & (1 << index)) != 0;
                let memory_type = self.memory_properties.memory_types[*index as usize];
                type_supported && memory_type.property_flags.contains(properties)
            })
            .ok_or_else(|| {
                let device_name = unsafe {
                    self.instance
                        .get_physical_device_properties(self.physical_device)
                        .device_name_as_c_str()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| "unknown".to_owned())
                };
                format!("no Vulkan memory type supports {properties:?} on {device_name}").into()
            })
    }
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

impl VulkanImage {
    fn empty() -> Self {
        Self {
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
        }
    }
}
