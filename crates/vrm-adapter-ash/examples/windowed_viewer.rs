//! Windowed ash/Vulkan viewer for a VRM avatar plus an optional VRMA clip.
//!
//! This example deliberately keeps the crate boundary intact: `vrm-adapter-ash`
//! plans and CPU-bakes VRM geometry, while the example owns the unsafe Vulkan
//! instance/device/surface/swapchain edge. The default path materializes a full
//! `AshRendererFrame` with WGSL/Naga MToon SPIR-V and presents directly to the
//! swapchain; `--simple-preview` keeps the older CPU-projected mesh preview.

use ash::khr::{surface, swapchain};
use ash::{Entry, vk};
use bytemuck::{Pod, Zeroable};
use clap::Parser;
use glam::{Mat4, Vec3, Vec4};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::error::Error;
use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::ptr;
use std::time::Instant;
use vrm_io::{GltfMaterialTextureFallback, RgbaMipLevel, generate_rgba_mip_chain};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use vrm_adapter::ScreenProjectionSize;
use vrm_adapter_ash::{
    ASH_FALLBACK_TEXTURES, ASH_MTOON_WGSL_DEFAULT_FRAGMENT_SPIRV_PATH,
    ASH_MTOON_WGSL_DEFAULT_VERTEX_SPIRV_PATH, ASH_MTOON_WGSL_FRAGMENT_ENTRY,
    ASH_MTOON_WGSL_VERTEX_ENTRY, AshColorAttachmentFinalLayout, AshCommandPlan,
    AshDescriptorSetLayoutPlan, AshDescriptorWriteData, AshDescriptorWriteResource,
    AshDrawableFrameOptions, AshGraphicsPipelineCreateInfoPlan, AshGraphicsPipelinePlan,
    AshMtoonBufferCacheKey, AshMtoonDescriptorSetCacheKey, AshMtoonPipelineCacheKey,
    AshMtoonSamplerCacheKey, AshMtoonShaderCacheKey, AshMtoonTextureCacheKey,
    AshMtoonUniformCacheKey, AshMtoonWindowedCacheStats, AshRenderOptions,
    AshRenderPassCreationPlan, AshRenderPassDependencyPolicy, AshRendererFrame, AshSamplerPlan,
    AshSwapchainAcquireStatus, AshSwapchainPresentStatus, AshVrmFramePlanOptions,
    AshVrmFramePlanner, AshVrmPrimitive, AshVrmVertex, AshWindowedFrameAcquirePlan,
    AshWindowedFrameSyncHandles, AshWindowedFrameSyncPlan, AshWindowedResizeValidation,
    AshWindowedRunValidation, ash_2d_image_resource_plan, ash_2d_image_view_plan,
    ash_binary_semaphore_plan, ash_classify_swapchain_acquire, ash_classify_swapchain_present,
    ash_depth_attachment_plan, ash_descriptor_pool_plan, ash_descriptor_set_allocation_plan,
    ash_descriptor_set_layout_plans, ash_descriptor_write_plans,
    ash_drawable_frame_from_renderer_frame_with_options, ash_empty_pipeline_layout_plan,
    ash_fallback_texture_mip_level, ash_fallback_texture_rgba, ash_framebuffer_plan,
    ash_graphics_pipeline_create_info_plan, ash_graphics_shader_stages_plan, ash_host_buffer_plan,
    ash_host_visible_buffer_plan, ash_memory_allocation_plan, ash_memory_type_index,
    ash_mtoon_renderer_cache_keys, ash_one_time_command_buffer_begin_plan,
    ash_pipeline_layout_plans, ash_position_color_pipeline_state_plan,
    ash_primary_command_buffer_allocation_plan, ash_queue_submit_plan, ash_render_pass_begin_plan,
    ash_render_pass_begin_plan_from_clear_values, ash_render_pass_creation_plan,
    ash_renderer_frame_from_plan_with_owner_sample_selection, ash_resettable_command_pool_plan,
    ash_reusable_command_buffer_begin_plan, ash_select_depth_format, ash_shader_module_plan,
    ash_signaled_fence_plan, ash_swapchain_surface_plan, ash_texture_mip_upload_bytes,
    ash_texture_upload_command_plan, ash_unsignaled_fence_plan, ash_windowed_present_plan,
    ash_windowed_submit_plan,
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
    /// Animation playback speed multiplier.
    #[arg(long, default_value_t = 1.0)]
    speed: f32,
    /// Keep rendering the same sampled time instead of advancing playback.
    #[arg(long)]
    paused: bool,
    /// Initial window width.
    #[arg(long, default_value_t = 1280)]
    width: u32,
    /// Initial window height.
    #[arg(long, default_value_t = 720)]
    height: u32,
    /// Use the legacy CPU-projected simple mesh preview instead of full MToon swapchain rendering.
    #[arg(long)]
    simple_preview: bool,
    /// Vertex SPIR-V for the active viewer path.
    #[arg(long, default_value = ASH_MTOON_WGSL_DEFAULT_VERTEX_SPIRV_PATH)]
    vertex_spv: PathBuf,
    /// Fragment SPIR-V for the active viewer path.
    #[arg(long, default_value = ASH_MTOON_WGSL_DEFAULT_FRAGMENT_SPIRV_PATH)]
    fragment_spv: PathBuf,
    /// Entry point name for `--vertex-spv`.
    #[arg(long, default_value = ASH_MTOON_WGSL_VERTEX_ENTRY)]
    vertex_entry: String,
    /// Entry point name for `--fragment-spv`.
    #[arg(long, default_value = ASH_MTOON_WGSL_FRAGMENT_ENTRY)]
    fragment_entry: String,
    /// Exit after rendering this many frames. Useful for smoke tests.
    #[arg(long)]
    max_frames: Option<u64>,
    /// Request a window resize after this many successfully presented frames.
    #[arg(long)]
    resize_after_frames: Option<u64>,
    /// Target width used by `--resize-after-frames`.
    #[arg(long, default_value_t = 960)]
    resize_width: u32,
    /// Target height used by `--resize-after-frames`.
    #[arg(long, default_value_t = 540)]
    resize_height: u32,
    /// Treat a smoke run as failed unless a requested resize recreates the swapchain.
    #[arg(long)]
    require_resize_recreate: bool,
    /// Number of queued MToon frames allowed before waiting. Ignored by `--simple-preview`.
    #[arg(long, default_value_t = 2)]
    frames_in_flight: usize,
    /// Print renderer cache hit/rebuild counters before exiting.
    #[arg(long)]
    print_cache_stats: bool,
    /// Treat a smoke run as failed unless the MToon renderer reports steady-state cache hits.
    #[arg(long)]
    require_cache_hits: bool,
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

struct WindowedAvatar {
    planner: AshVrmFramePlanner,
    options: Options,
    started_at: Instant,
}

impl WindowedAvatar {
    fn new(options: Options) -> Result<Self, Box<dyn Error>> {
        let animation = (!options.no_animation).then_some(options.animation.clone());
        let planner = AshVrmFramePlanner::from_paths(options.avatar.clone(), animation)?;
        Ok(Self {
            planner,
            options,
            started_at: Instant::now(),
        })
    }

    fn sample_time(&self) -> f32 {
        if self.options.no_animation || self.options.paused {
            self.options.time
        } else {
            self.options.time + self.started_at.elapsed().as_secs_f32() * self.options.speed
        }
    }

    fn sample_mesh(&mut self, size: PhysicalSize<u32>) -> Result<SimpleMesh, Box<dyn Error>> {
        let time = self.sample_time();
        build_simple_mesh(
            &mut self.planner,
            &self.options,
            time,
            size.width,
            size.height,
        )
    }

    fn sample_renderer_frame(
        &mut self,
        size: PhysicalSize<u32>,
    ) -> Result<AshRendererFrame, Box<dyn Error>> {
        let time = self.sample_time();
        let aspect = size.width.max(1) as f32 / size.height.max(1) as f32;
        let mut frame_options = AshVrmFramePlanOptions::parse_from(["ash-windowed-viewer"]);
        frame_options.avatar = self.options.avatar.clone();
        frame_options.animation = self.options.animation.clone();
        frame_options.no_animation = self.options.no_animation;
        frame_options.time = time;
        let scene_options = frame_options.scene_options_with_screen_size(
            aspect,
            ScreenProjectionSize {
                width: size.width.max(1) as f32,
                height: size.height.max(1) as f32,
            },
        );
        let plan = self.planner.sample_frame_with_full_render_options(
            time,
            scene_options,
            AshRenderOptions {
                diagnostic_render: frame_options.diagnostic_render,
                disable_outlines: frame_options.disable_outlines,
                outline_width_scale: frame_options.outline_width_scale,
                disable_normal_maps: frame_options.disable_normal_maps,
                normal_map_mode: frame_options.normal_map_mode,
                normal_map_scale: frame_options.normal_map_scale,
                descriptor_binding_model: frame_options.descriptor_binding_model,
            },
        )?;
        ash_renderer_frame_from_plan_with_owner_sample_selection(&plan, None)
            .map_err(|error| format!("failed to build ash renderer frame: {error:?}").into())
    }
}

#[derive(Clone, Debug)]
struct ShaderModules {
    vertex: Vec<u32>,
    fragment: Vec<u32>,
    vertex_entry: String,
    fragment_entry: String,
}

enum ActiveRenderer {
    Simple(Box<WindowedAshRenderer>),
    Mtoon(Box<MtoonWindowedAshRenderer>),
}

struct App {
    avatar: WindowedAvatar,
    shaders: ShaderModules,
    active_renderer: Option<ActiveRenderer>,
    window: Option<Window>,
    recreate_swapchain: bool,
    rendered_frames: u64,
    resize_requested: bool,
    resize_events: u64,
    resize_events_after_request: u64,
    swapchain_recreates: u64,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title("vrm-rs ash windowed viewer")
            .with_inner_size(PhysicalSize::new(
                self.avatar.options.width,
                self.avatar.options.height,
            ));
        let window = event_loop.create_window(attributes).expect("create window");
        let active_renderer = if self.avatar.options.simple_preview {
            let initial_size =
                PhysicalSize::new(self.avatar.options.width, self.avatar.options.height);
            let initial_mesh = self
                .avatar
                .sample_mesh(initial_size)
                .expect("sample initial simple preview mesh");
            let renderer = WindowedAshRenderer::new(&window, &initial_mesh, &self.shaders)
                .expect("initialize ash simple preview renderer");
            ActiveRenderer::Simple(Box::new(renderer))
        } else {
            ActiveRenderer::Mtoon(Box::new(
                MtoonWindowedAshRenderer::new(
                    &window,
                    &self.shaders,
                    self.avatar.options.frames_in_flight,
                )
                .expect("initialize ash mtoon windowed renderer"),
            ))
        };
        self.active_renderer = Some(active_renderer);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != window_id)
        {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                self.finish_windowed_run(event_loop);
            }
            WindowEvent::Resized(size) => {
                self.resize_events = self.resize_events.saturating_add(1);
                if self.resize_requested {
                    self.resize_events_after_request =
                        self.resize_events_after_request.saturating_add(1);
                }
                self.recreate_swapchain = size.width > 0 && size.height > 0;
            }
            WindowEvent::RedrawRequested => {
                let Some(window) = self.window.as_ref() else {
                    return;
                };
                let size = window.inner_size();
                let render_status = match self.active_renderer.as_mut() {
                    Some(ActiveRenderer::Simple(renderer)) => {
                        let frame_mesh = match self.avatar.sample_mesh(size) {
                            Ok(mesh) => mesh,
                            Err(error) => {
                                eprintln!("ash windowed animation sample failed: {error}");
                                event_loop.exit();
                                return;
                            }
                        };
                        if let Err(error) = renderer.update_mesh(&frame_mesh) {
                            eprintln!("ash windowed mesh update failed: {error}");
                            event_loop.exit();
                            return;
                        }
                        if self.recreate_swapchain {
                            renderer
                                .recreate_swapchain(window)
                                .expect("recreate swapchain");
                            self.swapchain_recreates = self.swapchain_recreates.saturating_add(1);
                            self.recreate_swapchain = false;
                        }
                        renderer.render(window)
                    }
                    Some(ActiveRenderer::Mtoon(renderer)) => {
                        let frame = match self.avatar.sample_renderer_frame(size) {
                            Ok(frame) => frame,
                            Err(error) => {
                                eprintln!("ash windowed animation sample failed: {error}");
                                event_loop.exit();
                                return;
                            }
                        };
                        if self.recreate_swapchain {
                            renderer
                                .recreate_swapchain(window)
                                .expect("recreate swapchain");
                            self.swapchain_recreates = self.swapchain_recreates.saturating_add(1);
                            self.recreate_swapchain = false;
                        }
                        renderer.render(window, &frame)
                    }
                    None => return,
                };
                match render_status {
                    Ok(RenderStatus::Ok) => {
                        self.rendered_frames = self.rendered_frames.saturating_add(1);
                        if let Some(target) = self.consume_test_resize_target()
                            && let Some(window) = self.window.as_ref()
                        {
                            let _ = window.request_inner_size(target);
                        }
                        if self
                            .avatar
                            .options
                            .max_frames
                            .is_some_and(|max| self.rendered_frames >= max)
                        {
                            self.finish_windowed_run(event_loop);
                        }
                    }
                    Ok(RenderStatus::NeedsRecreate) => self.recreate_swapchain = true,
                    Err(error) => {
                        eprintln!("ash windowed render failed: {error}");
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            let size = window.inner_size();
            if size.width > 0 && size.height > 0 {
                window.request_redraw();
            }
        }
    }
}

impl App {
    fn finish_windowed_run(&mut self, event_loop: &ActiveEventLoop) {
        if self.avatar.options.print_cache_stats
            && let Some(ActiveRenderer::Mtoon(renderer)) = self.active_renderer.as_ref()
        {
            println!("ash windowed cache stats: {}", renderer.cache_stats());
            println!(
                "ash windowed sync stats: frames_in_flight={}, swapchain_images={}",
                renderer.frames_in_flight(),
                renderer.swapchain_image_count()
            );
        }
        if self.avatar.options.print_cache_stats {
            println!(
                "ash windowed resize stats: requested={}, events={}, events_after_request={}, recreates={}",
                self.resize_requested,
                self.resize_events,
                self.resize_events_after_request,
                self.swapchain_recreates
            );
        }
        if self.avatar.options.require_cache_hits
            && let Some(ActiveRenderer::Mtoon(renderer)) = self.active_renderer.as_ref()
            && let Err(error) = renderer.cache_stats().validate_steady_state_hits()
        {
            eprintln!("ash windowed cache validation failed: {error}");
            std::process::exit(1);
        }
        if self.avatar.options.require_resize_recreate
            && let Err(error) = self.validate_resize_recreate()
        {
            eprintln!("ash windowed resize validation failed: {error}");
            std::process::exit(1);
        }
        event_loop.exit();
    }

    fn consume_test_resize_target(&mut self) -> Option<PhysicalSize<u32>> {
        let resize_after_frames = self.avatar.options.resize_after_frames?;
        if self.resize_requested || self.rendered_frames < resize_after_frames {
            return None;
        }
        self.resize_requested = true;
        Some(PhysicalSize::new(
            self.avatar.options.resize_width.max(1),
            self.avatar.options.resize_height.max(1),
        ))
    }

    fn validate_resize_recreate(&self) -> Result<(), String> {
        AshWindowedResizeValidation {
            resize_after_frames: self.avatar.options.resize_after_frames,
            resize_requested: self.resize_requested,
            resize_events_after_request: self.resize_events_after_request,
            swapchain_recreates: self.swapchain_recreates,
        }
        .validate_recreate()
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse();
    AshWindowedRunValidation {
        simple_preview: options.simple_preview,
        require_cache_hits: options.require_cache_hits,
        require_resize_recreate: options.require_resize_recreate,
        resize_after_frames: options.resize_after_frames,
        frames_in_flight: options.frames_in_flight,
    }
    .validate()?;
    let avatar = WindowedAvatar::new(options)?;
    let shaders = ShaderModules {
        vertex: read_spirv_words(&avatar.options.vertex_spv, avatar.options.simple_preview)?,
        fragment: read_spirv_words(&avatar.options.fragment_spv, avatar.options.simple_preview)?,
        vertex_entry: if avatar.options.simple_preview {
            "main".to_owned()
        } else {
            avatar.options.vertex_entry.clone()
        },
        fragment_entry: if avatar.options.simple_preview {
            "main".to_owned()
        } else {
            avatar.options.fragment_entry.clone()
        },
    };
    let event_loop = EventLoop::new()?;
    let mut app = App {
        avatar,
        shaders,
        active_renderer: None,
        window: None,
        recreate_swapchain: false,
        rendered_frames: 0,
        resize_requested: false,
        resize_events: 0,
        resize_events_after_request: 0,
        swapchain_recreates: 0,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn build_simple_mesh(
    planner: &mut AshVrmFramePlanner,
    options: &Options,
    time: f32,
    width: u32,
    height: u32,
) -> Result<SimpleMesh, Box<dyn Error>> {
    let mut frame_options = AshVrmFramePlanOptions::parse_from(["ash-windowed-viewer"]);
    frame_options.avatar = options.avatar.clone();
    frame_options.animation = options.animation.clone();
    frame_options.no_animation = options.no_animation;
    frame_options.time = time;
    let aspect = width.max(1) as f32 / height.max(1) as f32;
    let scene_options = frame_options.scene_options_with_screen_size(
        aspect,
        ScreenProjectionSize {
            width: width.max(1) as f32,
            height: height.max(1) as f32,
        },
    );
    let plan = planner.sample_frame_with_full_render_options(
        time,
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

fn read_spirv_words(path: &PathBuf, simple_preview: bool) -> Result<Vec<u32>, Box<dyn Error>> {
    let hint = if simple_preview {
        "run `just ash-windowed-simple-shaders` first"
    } else {
        "run `just ash-mtoon-wgsl-base-shaders` first"
    };
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read {}: {error}; {hint}", path.display()))?;
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

fn write_host_buffer(
    device: &ash::Device,
    buffer: &VulkanBuffer,
    bytes: &[u8],
) -> Result<(), Box<dyn Error>> {
    let byte_len = vk::DeviceSize::try_from(bytes.len())?;
    if byte_len > buffer.byte_size {
        return Err(format!(
            "animated frame needs {byte_len} bytes but the existing Vulkan buffer has {} bytes",
            buffer.byte_size
        )
        .into());
    }
    if bytes.is_empty() {
        return Ok(());
    }
    unsafe {
        let mapped = device.map_memory(
            buffer.memory,
            0,
            buffer.byte_size,
            vk::MemoryMapFlags::empty(),
        )?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
        device.unmap_memory(buffer.memory);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderStatus {
    Ok,
    NeedsRecreate,
}

struct MtoonVulkanBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
}

struct MtoonVulkanImage {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

struct MtoonVulkanFallbackTextures {
    white: MtoonVulkanImage,
    black: MtoonVulkanImage,
    neutral_normal: MtoonVulkanImage,
}

impl MtoonVulkanFallbackTextures {
    fn get(&self, fallback: GltfMaterialTextureFallback) -> &MtoonVulkanImage {
        match fallback {
            GltfMaterialTextureFallback::White => &self.white,
            GltfMaterialTextureFallback::Black => &self.black,
            GltfMaterialTextureFallback::NeutralNormal => &self.neutral_normal,
        }
    }
}

impl IntoIterator for MtoonVulkanFallbackTextures {
    type IntoIter = std::array::IntoIter<MtoonVulkanImage, 3>;
    type Item = MtoonVulkanImage;

    fn into_iter(self) -> Self::IntoIter {
        [self.white, self.black, self.neutral_normal].into_iter()
    }
}

struct MtoonVulkanFallbackBuffers {
    white: MtoonVulkanBuffer,
    black: MtoonVulkanBuffer,
    neutral_normal: MtoonVulkanBuffer,
}

impl MtoonVulkanFallbackBuffers {
    fn get(&self, fallback: GltfMaterialTextureFallback) -> &MtoonVulkanBuffer {
        match fallback {
            GltfMaterialTextureFallback::White => &self.white,
            GltfMaterialTextureFallback::Black => &self.black,
            GltfMaterialTextureFallback::NeutralNormal => &self.neutral_normal,
        }
    }
}

impl IntoIterator for MtoonVulkanFallbackBuffers {
    type IntoIter = std::array::IntoIter<MtoonVulkanBuffer, 3>;
    type Item = MtoonVulkanBuffer;

    fn into_iter(self) -> Self::IntoIter {
        [self.white, self.black, self.neutral_normal].into_iter()
    }
}

struct MtoonPersistentPipelineCache {
    key: AshMtoonPipelineCacheKey,
    descriptor_set_layouts: Vec<vk::DescriptorSetLayout>,
    pipeline_layouts: Vec<vk::PipelineLayout>,
    pipelines: Vec<vk::Pipeline>,
}

struct MtoonPersistentDescriptorSetCache {
    key: AshMtoonDescriptorSetCacheKey,
    descriptor_pool: vk::DescriptorPool,
    descriptor_sets: Vec<vk::DescriptorSet>,
}

struct MtoonPersistentSamplerCache {
    key: AshMtoonSamplerCacheKey,
    samplers: Vec<vk::Sampler>,
}

struct MtoonPersistentBufferCache {
    key: AshMtoonBufferCacheKey,
    buffers: Vec<MtoonVulkanBuffer>,
}

struct MtoonPersistentUniformCache {
    key: AshMtoonUniformCacheKey,
    buffers: Vec<MtoonVulkanBuffer>,
}

struct MtoonPersistentTextureCache {
    key: AshMtoonTextureCacheKey,
    images: Vec<MtoonVulkanImage>,
}

struct MtoonPersistentFallbackTextureCache {
    textures: MtoonVulkanFallbackTextures,
}

struct MtoonFrameSync {
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    in_flight: vk::Fence,
}

struct MtoonSwapchainShell {
    swapchain: vk::SwapchainKHR,
    image_views: Vec<vk::ImageView>,
    depth: VulkanImage,
    render_pass: vk::RenderPass,
    framebuffers: Vec<vk::Framebuffer>,
    command_buffers: Vec<vk::CommandBuffer>,
    extent: vk::Extent2D,
}

impl MtoonSwapchainShell {
    fn empty() -> Self {
        Self {
            swapchain: vk::SwapchainKHR::null(),
            image_views: Vec::new(),
            depth: VulkanImage::empty(),
            render_pass: vk::RenderPass::null(),
            framebuffers: Vec::new(),
            command_buffers: Vec::new(),
            extent: vk::Extent2D {
                width: 0,
                height: 0,
            },
        }
    }
}

struct MtoonWindowedAshRenderer {
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
    swapchain: MtoonSwapchainShell,
    vertex_shader: vk::ShaderModule,
    fragment_shader: vk::ShaderModule,
    vertex_entry: CString,
    fragment_entry: CString,
    shader_cache_key: AshMtoonShaderCacheKey,
    depth_format: vk::Format,
    frame_sync: Vec<MtoonFrameSync>,
    current_frame: usize,
    images_in_flight: Vec<vk::Fence>,
    persistent_cache: Option<MtoonPersistentPipelineCache>,
    persistent_descriptors: Option<MtoonPersistentDescriptorSetCache>,
    persistent_samplers: Option<MtoonPersistentSamplerCache>,
    persistent_buffers: Option<MtoonPersistentBufferCache>,
    persistent_uniforms: Option<MtoonPersistentUniformCache>,
    persistent_textures: Option<MtoonPersistentTextureCache>,
    persistent_fallback_textures: Option<MtoonPersistentFallbackTextureCache>,
    cache_stats: AshMtoonWindowedCacheStats,
}

impl MtoonWindowedAshRenderer {
    fn new(
        window: &Window,
        shaders: &ShaderModules,
        frames_in_flight: usize,
    ) -> Result<Self, Box<dyn Error>> {
        let entry = unsafe { Entry::load()? };
        let app_name = CString::new("vrm-rs ash windowed mtoon viewer")?;
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
        let command_pool_info =
            ash_resettable_command_pool_plan(queue_family_index).command_pool_create_info();
        let command_pool = unsafe { device.create_command_pool(&command_pool_info, None)? };
        let depth_format = select_depth_format(&instance, physical_device)?;
        let vertex_shader = create_shader_module(&device, &shaders.vertex)?;
        let fragment_shader = create_shader_module(&device, &shaders.fragment)?;
        let swapchain = create_mtoon_swapchain_shell(
            MtoonSwapchainCreateContext {
                instance: &instance,
                physical_device,
                device: &device,
                surface_loader: &surface_loader,
                surface,
                swapchain_loader: &swapchain_loader,
                command_pool,
                memory_properties,
                depth_format,
                old_swapchain: vk::SwapchainKHR::null(),
            },
            window,
        )?;
        let sync_plan =
            AshWindowedFrameSyncPlan::new(frames_in_flight, swapchain.image_views.len())?;
        let frame_sync = create_mtoon_frame_sync(&device, sync_plan)?;
        let images_in_flight = vec![vk::Fence::null(); sync_plan.image_fence_slots()];
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
            swapchain,
            vertex_shader,
            fragment_shader,
            vertex_entry: CString::new(shaders.vertex_entry.as_str())?,
            fragment_entry: CString::new(shaders.fragment_entry.as_str())?,
            shader_cache_key: AshMtoonShaderCacheKey::from_spirv_words(
                shaders.vertex_entry.clone(),
                shaders.fragment_entry.clone(),
                &shaders.vertex,
                &shaders.fragment,
            ),
            depth_format,
            frame_sync,
            current_frame: 0,
            images_in_flight,
            persistent_cache: None,
            persistent_descriptors: None,
            persistent_samplers: None,
            persistent_buffers: None,
            persistent_uniforms: None,
            persistent_textures: None,
            persistent_fallback_textures: None,
            cache_stats: AshMtoonWindowedCacheStats::default(),
        })
    }

    fn cache_stats(&self) -> AshMtoonWindowedCacheStats {
        self.cache_stats
    }

    fn frames_in_flight(&self) -> usize {
        self.frame_sync.len()
    }

    fn swapchain_image_count(&self) -> usize {
        self.swapchain.image_views.len()
    }

    fn render(
        &mut self,
        window: &Window,
        frame: &AshRendererFrame,
    ) -> Result<RenderStatus, Box<dyn Error>> {
        if window.inner_size().width == 0 || window.inner_size().height == 0 {
            return Ok(RenderStatus::Ok);
        }
        let sync_plan =
            AshWindowedFrameSyncPlan::new(self.frame_sync.len(), self.swapchain.image_views.len())?;
        let current_frame = sync_plan.frame_slot(self.current_frame)?;
        let sync = self
            .frame_sync
            .get(current_frame)
            .ok_or("ash windowed mtoon renderer has no frame sync object")?;
        let sync_handles = AshWindowedFrameSyncHandles {
            image_available: sync.image_available,
            render_finished: sync.render_finished,
            in_flight: sync.in_flight,
        };
        unsafe {
            self.device
                .wait_for_fences(&[sync_handles.in_flight], true, u64::MAX)?;
        }
        let acquired = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain.swapchain,
                u64::MAX,
                sync_handles.image_available,
                vk::Fence::null(),
            )
        };
        let selection = match sync_plan.select_acquired_frame(
            self.current_frame,
            ash_classify_swapchain_acquire(acquired)?,
            &self.images_in_flight,
        )? {
            AshWindowedFrameAcquirePlan::Acquired(selection) => selection,
            AshWindowedFrameAcquirePlan::NeedsRecreate => return Ok(RenderStatus::NeedsRecreate),
        };
        if selection.previous_image_fence != vk::Fence::null() {
            unsafe {
                self.device
                    .wait_for_fences(&[selection.previous_image_fence], true, u64::MAX)?;
            }
        }
        let vertex_entry = self.vertex_entry.clone();
        let fragment_entry = self.fragment_entry.clone();
        let command_buffer = self.materialize_swapchain_frame(
            frame,
            selection.swapchain_image_slot,
            &vertex_entry,
            &fragment_entry,
        )?;
        self.images_in_flight[selection.swapchain_image_slot] = sync_handles.in_flight;
        let submit_plan = selection.submit_plan(sync_handles, command_buffer);
        let submit_info_plan = submit_plan.submit_info_plan();
        let submit = submit_info_plan.submit_infos();
        unsafe {
            self.device.reset_fences(&[submit_info_plan.fence])?;
            self.device
                .queue_submit(self.queue, &submit, submit_info_plan.fence)?;
        }
        let present_plan =
            selection.present_plan(submit_plan.signal_semaphore, self.swapchain.swapchain);
        let present_info_plan = present_plan.present_info_plan();
        let present = present_info_plan.present_info();
        let present_result = unsafe { self.swapchain_loader.queue_present(self.queue, &present) };
        let status =
            match ash_classify_swapchain_present(selection.acquired_suboptimal, present_result)? {
                AshSwapchainPresentStatus::Presented => Ok(RenderStatus::Ok),
                AshSwapchainPresentStatus::NeedsRecreate => Ok(RenderStatus::NeedsRecreate),
            };
        self.current_frame = selection.next_frame_index;
        status
    }

    fn recreate_swapchain(&mut self, window: &Window) -> Result<(), Box<dyn Error>> {
        if window.inner_size().width == 0 || window.inner_size().height == 0 {
            return Ok(());
        }
        unsafe {
            self.device.device_wait_idle()?;
        }
        if let Some(cache) = self.persistent_descriptors.take() {
            self.destroy_persistent_descriptor_set_cache(cache);
        }
        if let Some(cache) = self.persistent_cache.take() {
            self.destroy_persistent_cache(cache);
        }
        let old_swapchain = self.swapchain.swapchain;
        let old = std::mem::replace(&mut self.swapchain, MtoonSwapchainShell::empty());
        self.destroy_swapchain_shell(old, false);
        let new_swapchain = create_mtoon_swapchain_shell(
            MtoonSwapchainCreateContext {
                instance: &self.instance,
                physical_device: self.physical_device,
                device: &self.device,
                surface_loader: &self.surface_loader,
                surface: self.surface,
                swapchain_loader: &self.swapchain_loader,
                command_pool: self.command_pool,
                memory_properties: self.memory_properties,
                depth_format: self.depth_format,
                old_swapchain,
            },
            window,
        );
        destroy_swapchain_handle(&self.swapchain_loader, old_swapchain);
        self.swapchain = new_swapchain?;
        let sync_plan =
            AshWindowedFrameSyncPlan::new(self.frame_sync.len(), self.swapchain.image_views.len())?;
        self.images_in_flight = vec![vk::Fence::null(); sync_plan.image_fence_slots()];
        Ok(())
    }

    fn materialize_swapchain_frame(
        &mut self,
        frame: &AshRendererFrame,
        image_index: usize,
        vertex_entry: &CString,
        fragment_entry: &CString,
    ) -> Result<vk::CommandBuffer, Box<dyn Error>> {
        let extent = self.swapchain.extent;
        let framebuffer = self.swapchain.framebuffers[image_index];
        let render_pass = self.swapchain.render_pass;
        let command_buffer = *self
            .swapchain
            .command_buffers
            .get(image_index)
            .ok_or("swapchain image has no matching draw command buffer")?;
        let cache_keys =
            ash_mtoon_renderer_cache_keys(frame, extent, self.shader_cache_key.clone());
        self.ensure_persistent_pipeline_cache(
            frame,
            extent,
            render_pass,
            vertex_entry,
            fragment_entry,
            cache_keys.pipeline,
        )?;
        self.ensure_persistent_sampler_cache(frame, cache_keys.samplers)?;
        self.ensure_persistent_buffer_cache(frame, cache_keys.buffers)?;
        self.ensure_persistent_uniform_cache(frame, cache_keys.uniforms)?;
        self.ensure_persistent_texture_cache(frame, cache_keys.textures)?;
        self.ensure_persistent_fallback_textures()?;
        self.ensure_persistent_descriptor_set_cache(frame, cache_keys.descriptor_sets)?;
        let persistent = self
            .persistent_cache
            .as_ref()
            .ok_or("missing ash windowed persistent pipeline cache")?;
        let descriptor_sets = &self
            .persistent_descriptors
            .as_ref()
            .ok_or("missing ash windowed persistent descriptor set cache")?
            .descriptor_sets;
        let buffers = &self
            .persistent_buffers
            .as_ref()
            .ok_or("missing ash windowed persistent buffer cache")?
            .buffers;
        let uniform_buffers = &self
            .persistent_uniforms
            .as_ref()
            .ok_or("missing ash windowed persistent uniform cache")?
            .buffers;
        let texture_images = &self
            .persistent_textures
            .as_ref()
            .ok_or("missing ash windowed persistent texture cache")?
            .images;
        let fallback_textures = &self
            .persistent_fallback_textures
            .as_ref()
            .ok_or("missing ash windowed persistent fallback texture cache")?
            .textures;
        let samplers = &self
            .persistent_samplers
            .as_ref()
            .ok_or("missing ash windowed persistent sampler cache")?
            .samplers;
        self.update_descriptor_sets(
            frame,
            descriptor_sets,
            MtoonDescriptorUpdateResources {
                buffers,
                uniform_buffers,
                images: texture_images,
                fallback_textures,
                samplers,
            },
        )?;
        self.cache_stats.command_buffers.hit();
        self.record_mtoon_swapchain_draws(
            command_buffer,
            frame,
            MtoonSwapchainDrawContext {
                render_pass,
                framebuffer,
                extent,
                pipelines: &persistent.pipelines,
                pipeline_layouts: &persistent.pipeline_layouts,
                buffers,
                descriptor_sets,
            },
        )?;
        Ok(command_buffer)
    }

    fn ensure_persistent_pipeline_cache(
        &mut self,
        frame: &AshRendererFrame,
        extent: vk::Extent2D,
        render_pass: vk::RenderPass,
        vertex_entry: &CString,
        fragment_entry: &CString,
        key: AshMtoonPipelineCacheKey,
    ) -> Result<(), Box<dyn Error>> {
        if self
            .persistent_cache
            .as_ref()
            .is_some_and(|cache| cache.key == key)
        {
            self.cache_stats.pipeline.hit();
            return Ok(());
        }
        self.cache_stats.pipeline.rebuild();
        if let Some(cache) = self.persistent_descriptors.take() {
            self.destroy_persistent_descriptor_set_cache(cache);
        }
        if let Some(cache) = self.persistent_cache.take() {
            self.destroy_persistent_cache(cache);
        }
        let descriptor_set_layout_plans = ash_descriptor_set_layout_plans(frame);
        let descriptor_set_layouts = descriptor_set_layout_plans
            .iter()
            .map(|plan| self.create_descriptor_set_layout(plan))
            .collect::<Result<Vec<_>, _>>()?;
        let pipeline_layouts = ash_pipeline_layout_plans(&descriptor_set_layout_plans)
            .iter()
            .map(|plan| {
                plan.with_pipeline_layout_create_info(&descriptor_set_layouts, |info| unsafe {
                    self.device.create_pipeline_layout(&info, None)
                })
                .map_err(std::io::Error::other)?
                .map_err(Into::into)
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        let pipelines = self.create_mtoon_graphics_pipelines(
            frame,
            render_pass,
            extent,
            &pipeline_layouts,
            vertex_entry,
            fragment_entry,
        )?;
        self.persistent_cache = Some(MtoonPersistentPipelineCache {
            key,
            descriptor_set_layouts,
            pipeline_layouts,
            pipelines,
        });
        Ok(())
    }

    fn ensure_persistent_descriptor_set_cache(
        &mut self,
        frame: &AshRendererFrame,
        key: AshMtoonDescriptorSetCacheKey,
    ) -> Result<(), Box<dyn Error>> {
        if self
            .persistent_descriptors
            .as_ref()
            .is_some_and(|cache| cache.key == key)
        {
            self.cache_stats.descriptors.hit();
            return Ok(());
        }
        self.cache_stats.descriptors.rebuild();
        if let Some(cache) = self.persistent_descriptors.take() {
            self.destroy_persistent_descriptor_set_cache(cache);
        }
        let layouts = self
            .persistent_cache
            .as_ref()
            .ok_or("missing ash windowed persistent pipeline cache")?
            .descriptor_set_layouts
            .as_slice();
        let descriptor_pool = self.create_descriptor_pool(frame)?;
        let allocation_plan =
            ash_descriptor_set_allocation_plan(&ash_descriptor_set_layout_plans(frame));
        let descriptor_sets =
            self.allocate_descriptor_sets(descriptor_pool, &allocation_plan, layouts)?;
        self.persistent_descriptors = Some(MtoonPersistentDescriptorSetCache {
            key,
            descriptor_pool,
            descriptor_sets,
        });
        Ok(())
    }

    fn ensure_persistent_sampler_cache(
        &mut self,
        frame: &AshRendererFrame,
        key: AshMtoonSamplerCacheKey,
    ) -> Result<(), Box<dyn Error>> {
        if self
            .persistent_samplers
            .as_ref()
            .is_some_and(|cache| cache.key == key)
        {
            self.cache_stats.samplers.hit();
            return Ok(());
        }
        self.cache_stats.samplers.rebuild();
        if let Some(cache) = self.persistent_samplers.take() {
            self.destroy_persistent_sampler_cache(cache);
        }
        let samplers = frame
            .descriptor_sets
            .iter()
            .flat_map(|set| {
                set.bindings.iter().filter(|binding| {
                    matches!(
                        binding.descriptor_type,
                        vk::DescriptorType::COMBINED_IMAGE_SAMPLER | vk::DescriptorType::SAMPLER
                    )
                })
            })
            .map(|binding| self.create_sampler(binding.sampler.unwrap_or_default()))
            .collect::<Result<Vec<_>, _>>()?;
        self.persistent_samplers = Some(MtoonPersistentSamplerCache { key, samplers });
        Ok(())
    }

    fn ensure_persistent_buffer_cache(
        &mut self,
        frame: &AshRendererFrame,
        key: AshMtoonBufferCacheKey,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(cache) = self
            .persistent_buffers
            .as_ref()
            .filter(|cache| cache.key == key)
        {
            self.cache_stats.buffers.hit();
            cache
                .buffers
                .iter()
                .zip(&frame.buffers)
                .try_for_each(|(buffer, upload)| self.write_host_buffer(buffer, &upload.bytes))?;
            return Ok(());
        }
        self.cache_stats.buffers.rebuild();
        if let Some(cache) = self.persistent_buffers.take() {
            self.destroy_persistent_buffer_cache(cache);
        }
        let buffers = frame
            .buffers
            .iter()
            .map(|buffer| self.create_upload_buffer(buffer.usage, &buffer.bytes))
            .collect::<Result<Vec<_>, _>>()?;
        self.persistent_buffers = Some(MtoonPersistentBufferCache { key, buffers });
        Ok(())
    }

    fn ensure_persistent_uniform_cache(
        &mut self,
        frame: &AshRendererFrame,
        key: AshMtoonUniformCacheKey,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(cache) = self
            .persistent_uniforms
            .as_ref()
            .filter(|cache| cache.key == key)
        {
            self.cache_stats.uniforms.hit();
            cache
                .buffers
                .iter()
                .zip(&frame.uniforms)
                .try_for_each(|(buffer, uniform)| self.write_host_buffer(buffer, &uniform.bytes))?;
            return Ok(());
        }
        self.cache_stats.uniforms.rebuild();
        if let Some(cache) = self.persistent_uniforms.take() {
            self.destroy_persistent_uniform_cache(cache);
        }
        let buffers = frame
            .uniforms
            .iter()
            .map(|uniform| {
                self.create_host_buffer(vk::BufferUsageFlags::UNIFORM_BUFFER, &uniform.bytes)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.persistent_uniforms = Some(MtoonPersistentUniformCache { key, buffers });
        Ok(())
    }

    fn ensure_persistent_texture_cache(
        &mut self,
        frame: &AshRendererFrame,
        key: AshMtoonTextureCacheKey,
    ) -> Result<(), Box<dyn Error>> {
        if self
            .persistent_textures
            .as_ref()
            .is_some_and(|cache| cache.key == key)
        {
            self.cache_stats.textures.hit();
            return Ok(());
        }
        self.cache_stats.textures.rebuild();
        if let Some(cache) = self.persistent_textures.take() {
            self.destroy_persistent_texture_cache(cache);
        }
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
                    texture.aspect_mask,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let staging_buffers = texture_mip_levels
            .iter()
            .map(|mip_levels| {
                self.create_host_buffer(
                    vk::BufferUsageFlags::TRANSFER_SRC,
                    &ash_texture_mip_upload_bytes(mip_levels),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let upload_result =
            self.upload_texture_images_once(&images, &staging_buffers, &texture_mip_levels);
        self.destroy_buffers(staging_buffers);
        if let Err(error) = upload_result {
            self.destroy_images(images);
            return Err(error);
        }
        self.persistent_textures = Some(MtoonPersistentTextureCache { key, images });
        Ok(())
    }

    fn ensure_persistent_fallback_textures(&mut self) -> Result<(), Box<dyn Error>> {
        if self.persistent_fallback_textures.is_some() {
            self.cache_stats.fallback_textures.hit();
            return Ok(());
        }
        self.cache_stats.fallback_textures.rebuild();
        let textures = self.create_fallback_textures()?;
        let staging = self.create_fallback_staging_buffers()?;
        let upload_result = self.upload_fallback_textures_once(&textures, &staging);
        self.destroy_fallback_staging_buffers(staging);
        if let Err(error) = upload_result {
            self.destroy_fallback_textures(textures);
            return Err(error);
        }
        self.persistent_fallback_textures = Some(MtoonPersistentFallbackTextureCache { textures });
        Ok(())
    }

    fn create_host_buffer(
        &self,
        usage: vk::BufferUsageFlags,
        bytes: &[u8],
    ) -> Result<MtoonVulkanBuffer, Box<dyn Error>> {
        let plan = ash_host_buffer_plan(usage, bytes.len());
        let info = plan.buffer_create_info();
        let buffer = unsafe { self.device.create_buffer(&info, None)? };
        let requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type_index =
            self.find_memory_type(requirements.memory_type_bits, plan.memory_property_flags)?;
        let allocate_info =
            ash_memory_allocation_plan(requirements, memory_type_index).memory_allocate_info();
        let memory = unsafe { self.device.allocate_memory(&allocate_info, None)? };
        unsafe {
            self.device.bind_buffer_memory(buffer, memory, 0)?;
            if !bytes.is_empty() {
                let mapped =
                    self.device
                        .map_memory(memory, 0, plan.size, vk::MemoryMapFlags::empty())?;
                ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
                self.device.unmap_memory(memory);
            }
        }
        Ok(MtoonVulkanBuffer { buffer, memory })
    }

    fn create_upload_buffer(
        &self,
        usage: vk::BufferUsageFlags,
        bytes: &[u8],
    ) -> Result<MtoonVulkanBuffer, Box<dyn Error>> {
        self.create_host_buffer(usage | vk::BufferUsageFlags::TRANSFER_DST, bytes)
    }

    fn write_host_buffer(
        &self,
        buffer: &MtoonVulkanBuffer,
        bytes: &[u8],
    ) -> Result<(), Box<dyn Error>> {
        if bytes.is_empty() {
            return Ok(());
        }
        unsafe {
            let mapped = self.device.map_memory(
                buffer.memory,
                0,
                bytes.len() as vk::DeviceSize,
                vk::MemoryMapFlags::empty(),
            )?;
            ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
            self.device.unmap_memory(buffer.memory);
        }
        Ok(())
    }

    fn create_image(
        &self,
        format: vk::Format,
        extent: vk::Extent3D,
        mip_levels: u32,
        usage: vk::ImageUsageFlags,
        aspect_mask: vk::ImageAspectFlags,
    ) -> Result<MtoonVulkanImage, Box<dyn Error>> {
        let plan = ash_2d_image_resource_plan(format, extent, mip_levels, usage, aspect_mask);
        let image_info = plan.image_create_info();
        let image = unsafe { self.device.create_image(&image_info, None)? };
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type_index =
            self.find_memory_type(requirements.memory_type_bits, plan.memory_property_flags)?;
        let allocate_info =
            ash_memory_allocation_plan(requirements, memory_type_index).memory_allocate_info();
        let memory = unsafe { self.device.allocate_memory(&allocate_info, None)? };
        unsafe {
            self.device.bind_image_memory(image, memory, 0)?;
        }
        let view_info = plan.image_view_create_info(image);
        let view = unsafe { self.device.create_image_view(&view_info, None)? };
        Ok(MtoonVulkanImage {
            image,
            memory,
            view,
        })
    }

    fn create_fallback_textures(&self) -> Result<MtoonVulkanFallbackTextures, Box<dyn Error>> {
        Ok(MtoonVulkanFallbackTextures {
            white: self.create_fallback_texture_image()?,
            black: self.create_fallback_texture_image()?,
            neutral_normal: self.create_fallback_texture_image()?,
        })
    }

    fn create_fallback_texture_image(&self) -> Result<MtoonVulkanImage, Box<dyn Error>> {
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

    fn create_fallback_staging_buffers(
        &self,
    ) -> Result<MtoonVulkanFallbackBuffers, Box<dyn Error>> {
        Ok(MtoonVulkanFallbackBuffers {
            white: self.create_fallback_staging_buffer(GltfMaterialTextureFallback::White)?,
            black: self.create_fallback_staging_buffer(GltfMaterialTextureFallback::Black)?,
            neutral_normal: self
                .create_fallback_staging_buffer(GltfMaterialTextureFallback::NeutralNormal)?,
        })
    }

    fn create_fallback_staging_buffer(
        &self,
        fallback: GltfMaterialTextureFallback,
    ) -> Result<MtoonVulkanBuffer, Box<dyn Error>> {
        let rgba = ash_fallback_texture_rgba(fallback);
        self.create_host_buffer(vk::BufferUsageFlags::TRANSFER_SRC, &rgba)
    }

    fn upload_fallback_textures_once(
        &self,
        textures: &MtoonVulkanFallbackTextures,
        staging: &MtoonVulkanFallbackBuffers,
    ) -> Result<(), Box<dyn Error>> {
        self.submit_one_time_commands(|command_buffer| {
            record_mtoon_fallback_texture_uploads(&self.device, command_buffer, textures, staging);
        })
    }

    fn upload_texture_images_once(
        &self,
        images: &[MtoonVulkanImage],
        staging_buffers: &[MtoonVulkanBuffer],
        mip_levels: &[Vec<RgbaMipLevel>],
    ) -> Result<(), Box<dyn Error>> {
        self.submit_one_time_commands(|command_buffer| {
            for ((image, staging), mip_levels) in images.iter().zip(staging_buffers).zip(mip_levels)
            {
                record_mtoon_texture_upload(
                    &self.device,
                    command_buffer,
                    image.image,
                    staging.buffer,
                    mip_levels,
                );
            }
        })
    }

    fn submit_one_time_commands<F>(&self, record: F) -> Result<(), Box<dyn Error>>
    where
        F: FnOnce(vk::CommandBuffer),
    {
        let allocate_plan = ash_primary_command_buffer_allocation_plan(self.command_pool, 1);
        let allocate_info = allocate_plan.command_buffer_allocate_info();
        let command_buffers = unsafe { self.device.allocate_command_buffers(&allocate_info)? };
        let command_buffer = command_buffers[0];
        let begin_info = ash_one_time_command_buffer_begin_plan().command_buffer_begin_info();
        let begin_result = unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin_info)
        };
        if let Err(error) = begin_result {
            unsafe {
                self.device
                    .free_command_buffers(self.command_pool, &command_buffers);
            }
            return Err(error.into());
        }
        record(command_buffer);
        let end_result = unsafe { self.device.end_command_buffer(command_buffer) };
        if let Err(error) = end_result {
            unsafe {
                self.device
                    .free_command_buffers(self.command_pool, &command_buffers);
            }
            return Err(error.into());
        }
        let fence_info = ash_unsignaled_fence_plan().fence_create_info();
        let fence = match unsafe { self.device.create_fence(&fence_info, None) } {
            Ok(fence) => fence,
            Err(error) => {
                unsafe {
                    self.device
                        .free_command_buffers(self.command_pool, &command_buffers);
                }
                return Err(error.into());
            }
        };
        let submit_plan = ash_queue_submit_plan(command_buffers.clone(), fence);
        let submit = submit_plan.submit_infos();
        let wait_fences = submit_plan.wait_fences();
        let result = unsafe {
            self.device
                .queue_submit(self.queue, &submit, submit_plan.fence)
                .and_then(|_| self.device.wait_for_fences(&wait_fences, true, u64::MAX))
        };
        unsafe {
            self.device.destroy_fence(submit_plan.fence, None);
            self.device
                .free_command_buffers(self.command_pool, &command_buffers);
        }
        result.map_err(Into::into)
    }

    fn create_descriptor_set_layout(
        &self,
        plan: &AshDescriptorSetLayoutPlan,
    ) -> Result<vk::DescriptorSetLayout, vk::Result> {
        plan.with_descriptor_set_layout_create_info(|info| unsafe {
            self.device.create_descriptor_set_layout(&info, None)
        })
    }

    fn create_descriptor_pool(
        &self,
        frame: &AshRendererFrame,
    ) -> Result<vk::DescriptorPool, vk::Result> {
        let plan = ash_descriptor_pool_plan(frame);
        plan.with_descriptor_pool_create_info(|info| unsafe {
            self.device.create_descriptor_pool(&info, None)
        })
    }

    fn allocate_descriptor_sets(
        &self,
        pool: vk::DescriptorPool,
        allocation: &vrm_adapter_ash::AshDescriptorSetAllocationPlan,
        layouts: &[vk::DescriptorSetLayout],
    ) -> Result<Vec<vk::DescriptorSet>, Box<dyn Error>> {
        allocation
            .with_descriptor_set_allocate_info(pool, layouts, |info| unsafe {
                self.device.allocate_descriptor_sets(&info)
            })
            .map_err(std::io::Error::other)?
            .map_err(Into::into)
    }

    fn create_sampler(&self, plan: AshSamplerPlan) -> Result<vk::Sampler, vk::Result> {
        let info = plan.sampler_create_info();
        unsafe { self.device.create_sampler(&info, None) }
    }

    fn update_descriptor_sets(
        &self,
        frame: &AshRendererFrame,
        descriptor_sets: &[vk::DescriptorSet],
        resources: MtoonDescriptorUpdateResources<'_>,
    ) -> Result<(), Box<dyn Error>> {
        for plan in ash_descriptor_write_plans(frame)? {
            let write_data = match &plan.resource {
                AshDescriptorWriteResource::UniformBuffer {
                    uniform_upload_index: _,
                } => {
                    let uniform = plan
                        .resource
                        .uniform_resource(resources.uniform_buffers)
                        .map_err(std::io::Error::other)?;
                    AshDescriptorWriteData::whole_buffer(uniform.buffer)
                }
                AshDescriptorWriteResource::CombinedImageSampler {
                    sampler_index: _,
                    image: _,
                } => {
                    let sampler = plan
                        .resource
                        .sampler(resources.samplers)
                        .map_err(std::io::Error::other)?;
                    let image = plan
                        .resource
                        .image_resource()
                        .map_err(std::io::Error::other)?
                        .resolve_resource(resources.images, |fallback| {
                            resources.fallback_textures.get(fallback)
                        })
                        .map_err(std::io::Error::other)?;
                    AshDescriptorWriteData::combined_image_sampler(
                        sampler,
                        image.view,
                        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    )
                }
                AshDescriptorWriteResource::SampledImage { image: _ } => {
                    let image = plan
                        .resource
                        .image_resource()
                        .map_err(std::io::Error::other)?
                        .resolve_resource(resources.images, |fallback| {
                            resources.fallback_textures.get(fallback)
                        })
                        .map_err(std::io::Error::other)?;
                    AshDescriptorWriteData::sampled_image(
                        image.view,
                        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    )
                }
                AshDescriptorWriteResource::Sampler { sampler_index: _ } => {
                    let sampler = plan
                        .resource
                        .sampler(resources.samplers)
                        .map_err(std::io::Error::other)?;
                    AshDescriptorWriteData::sampler(sampler)
                }
                AshDescriptorWriteResource::StorageBuffer {
                    buffer_upload_index: _,
                } => {
                    let buffer = plan
                        .resource
                        .storage_buffer_resource(resources.buffers)
                        .map_err(std::io::Error::other)?;
                    AshDescriptorWriteData::whole_buffer(buffer.buffer)
                }
            };
            plan.with_write_descriptor_set(descriptor_sets, write_data, |write| unsafe {
                self.device.update_descriptor_sets(&[write], &[]);
            })
            .map_err(std::io::Error::other)?;
        }
        Ok(())
    }

    fn create_mtoon_graphics_pipelines(
        &self,
        frame: &AshRendererFrame,
        render_pass: vk::RenderPass,
        extent: vk::Extent2D,
        pipeline_layouts: &[vk::PipelineLayout],
        vertex_entry: &CString,
        fragment_entry: &CString,
    ) -> Result<Vec<vk::Pipeline>, Box<dyn Error>> {
        frame
            .pipelines
            .iter()
            .map(|pipeline| {
                self.create_mtoon_graphics_pipeline(
                    pipeline,
                    render_pass,
                    extent,
                    pipeline_layouts,
                    vertex_entry,
                    fragment_entry,
                )
            })
            .collect()
    }

    fn create_mtoon_graphics_pipeline(
        &self,
        pipeline: &AshGraphicsPipelinePlan,
        render_pass: vk::RenderPass,
        extent: vk::Extent2D,
        pipeline_layouts: &[vk::PipelineLayout],
        vertex_entry: &CString,
        fragment_entry: &CString,
    ) -> Result<vk::Pipeline, Box<dyn Error>> {
        let shader_stages =
            ash_graphics_shader_stages_plan(self.vertex_shader, self.fragment_shader)
                .shader_stage_create_infos(vertex_entry, fragment_entry);
        let pipeline_plan = ash_graphics_pipeline_create_info_plan(
            pipeline,
            extent,
            shader_stages,
            pipeline_layouts,
            render_pass,
        )
        .map_err(|error| -> Box<dyn Error> { error.into() })?;
        let pipelines = pipeline_plan
            .with_graphics_pipeline_create_info(|pipeline_info| unsafe {
                self.device.create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[pipeline_info],
                    None,
                )
            })
            .map_err(|(_, err)| Box::<dyn Error>::from(err))?;
        pipelines
            .into_iter()
            .next()
            .ok_or_else(|| "Vulkan returned no graphics pipeline".into())
    }

    fn record_mtoon_swapchain_draws(
        &self,
        command_buffer: vk::CommandBuffer,
        frame: &AshRendererFrame,
        context: MtoonSwapchainDrawContext<'_>,
    ) -> Result<(), Box<dyn Error>> {
        let drawable = ash_drawable_frame_from_renderer_frame_with_options(
            frame,
            context.extent,
            AshDrawableFrameOptions {
                color_clear: [0.0, 0.0, 0.0, 0.0],
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
        let begin_info = ash_one_time_command_buffer_begin_plan().command_buffer_begin_info();
        let render_pass_begin = ash_render_pass_begin_plan(
            &drawable.render_pass,
            context.render_pass,
            context.framebuffer,
        );
        let render_pass_info = render_pass_begin.render_pass_begin_info();
        unsafe {
            self.device
                .reset_command_buffer(command_buffer, vk::CommandBufferResetFlags::empty())?;
            self.device
                .begin_command_buffer(command_buffer, &begin_info)?;
            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );
            for command in &drawable.commands {
                match command {
                    AshCommandPlan::BindGraphicsPipeline { .. } => {
                        let bind = command
                            .bind_graphics_pipeline_command(context.pipelines)
                            .map_err(std::io::Error::other)?;
                        self.device.cmd_bind_pipeline(
                            command_buffer,
                            bind.bind_point,
                            bind.pipeline,
                        );
                    }
                    AshCommandPlan::BindDescriptorSet { .. } => {
                        let bind = command
                            .bind_descriptor_set_command(
                                &frame.pipelines,
                                context.pipeline_layouts,
                                context.descriptor_sets,
                            )
                            .map_err(std::io::Error::other)?;
                        self.device.cmd_bind_descriptor_sets(
                            command_buffer,
                            bind.bind_point,
                            bind.layout,
                            bind.first_set,
                            &bind.descriptor_sets,
                            &bind.dynamic_offsets,
                        );
                    }
                    AshCommandPlan::BindVertexBuffer { .. } => {
                        let buffer = command
                            .vertex_buffer_resource(context.buffers)
                            .map_err(std::io::Error::other)?
                            .buffer;
                        let bind = command
                            .bind_vertex_buffer_command(buffer)
                            .map_err(std::io::Error::other)?;
                        self.device.cmd_bind_vertex_buffers(
                            command_buffer,
                            bind.first_binding,
                            &bind.buffers,
                            &bind.offsets,
                        );
                    }
                    AshCommandPlan::BindIndexBuffer { .. } => {
                        let buffer = command
                            .index_buffer_resource(context.buffers)
                            .map_err(std::io::Error::other)?
                            .buffer;
                        let bind = command
                            .bind_index_buffer_command(buffer)
                            .map_err(std::io::Error::other)?;
                        self.device.cmd_bind_index_buffer(
                            command_buffer,
                            bind.buffer,
                            bind.offset,
                            bind.index_type,
                        );
                    }
                    AshCommandPlan::DrawIndexed { .. } => {
                        let draw = command.draw_indexed_args().map_err(std::io::Error::other)?;
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
            self.device.end_command_buffer(command_buffer)?;
        }
        Ok(())
    }

    fn find_memory_type(
        &self,
        type_bits: u32,
        properties: vk::MemoryPropertyFlags,
    ) -> Result<u32, Box<dyn Error>> {
        ash_memory_type_index(self.memory_properties, type_bits, properties).map_err(Into::into)
    }

    fn destroy_persistent_cache(&self, cache: MtoonPersistentPipelineCache) {
        unsafe {
            for pipeline in cache.pipelines {
                self.device.destroy_pipeline(pipeline, None);
            }
            for layout in cache.pipeline_layouts {
                self.device.destroy_pipeline_layout(layout, None);
            }
            for layout in cache.descriptor_set_layouts {
                self.device.destroy_descriptor_set_layout(layout, None);
            }
        }
    }

    fn destroy_persistent_sampler_cache(&self, cache: MtoonPersistentSamplerCache) {
        unsafe {
            for sampler in cache.samplers {
                self.device.destroy_sampler(sampler, None);
            }
        }
    }

    fn destroy_persistent_descriptor_set_cache(&self, cache: MtoonPersistentDescriptorSetCache) {
        unsafe {
            self.device
                .destroy_descriptor_pool(cache.descriptor_pool, None);
        }
    }

    fn destroy_persistent_buffer_cache(&self, cache: MtoonPersistentBufferCache) {
        self.destroy_buffers(cache.buffers);
    }

    fn destroy_persistent_uniform_cache(&self, cache: MtoonPersistentUniformCache) {
        self.destroy_buffers(cache.buffers);
    }

    fn destroy_persistent_texture_cache(&self, cache: MtoonPersistentTextureCache) {
        self.destroy_images(cache.images);
    }

    fn destroy_persistent_fallback_texture_cache(
        &self,
        cache: MtoonPersistentFallbackTextureCache,
    ) {
        self.destroy_fallback_textures(cache.textures);
    }

    fn destroy_images(&self, images: Vec<MtoonVulkanImage>) {
        unsafe {
            for image in images {
                self.device.destroy_image_view(image.view, None);
                self.device.destroy_image(image.image, None);
                self.device.free_memory(image.memory, None);
            }
        }
    }

    fn destroy_buffers(&self, buffers: Vec<MtoonVulkanBuffer>) {
        unsafe {
            for buffer in buffers {
                self.device.destroy_buffer(buffer.buffer, None);
                self.device.free_memory(buffer.memory, None);
            }
        }
    }

    fn destroy_fallback_textures(&self, textures: MtoonVulkanFallbackTextures) {
        unsafe {
            for image in textures.into_iter() {
                self.device.destroy_image_view(image.view, None);
                self.device.destroy_image(image.image, None);
                self.device.free_memory(image.memory, None);
            }
        }
    }

    fn destroy_fallback_staging_buffers(&self, staging: MtoonVulkanFallbackBuffers) {
        unsafe {
            for buffer in staging.into_iter() {
                self.device.destroy_buffer(buffer.buffer, None);
                self.device.free_memory(buffer.memory, None);
            }
        }
    }

    fn destroy_swapchain_shell(&self, resources: MtoonSwapchainShell, destroy_swapchain: bool) {
        unsafe {
            if !resources.command_buffers.is_empty() {
                self.device
                    .free_command_buffers(self.command_pool, &resources.command_buffers);
            }
            for framebuffer in resources.framebuffers {
                self.device.destroy_framebuffer(framebuffer, None);
            }
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

impl Drop for MtoonWindowedAshRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
        }
        if let Some(cache) = self.persistent_descriptors.take() {
            self.destroy_persistent_descriptor_set_cache(cache);
        }
        if let Some(cache) = self.persistent_cache.take() {
            self.destroy_persistent_cache(cache);
        }
        if let Some(cache) = self.persistent_samplers.take() {
            self.destroy_persistent_sampler_cache(cache);
        }
        if let Some(cache) = self.persistent_buffers.take() {
            self.destroy_persistent_buffer_cache(cache);
        }
        if let Some(cache) = self.persistent_uniforms.take() {
            self.destroy_persistent_uniform_cache(cache);
        }
        if let Some(cache) = self.persistent_textures.take() {
            self.destroy_persistent_texture_cache(cache);
        }
        if let Some(cache) = self.persistent_fallback_textures.take() {
            self.destroy_persistent_fallback_texture_cache(cache);
        }
        let shell = std::mem::replace(&mut self.swapchain, MtoonSwapchainShell::empty());
        self.destroy_swapchain_shell(shell, true);
        destroy_mtoon_frame_sync(&self.device, std::mem::take(&mut self.frame_sync));
        unsafe {
            self.device
                .destroy_shader_module(self.fragment_shader, None);
            self.device.destroy_shader_module(self.vertex_shader, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}

#[derive(Clone, Copy)]
struct MtoonDescriptorUpdateResources<'a> {
    buffers: &'a [MtoonVulkanBuffer],
    uniform_buffers: &'a [MtoonVulkanBuffer],
    images: &'a [MtoonVulkanImage],
    fallback_textures: &'a MtoonVulkanFallbackTextures,
    samplers: &'a [vk::Sampler],
}

struct MtoonSwapchainDrawContext<'a> {
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    extent: vk::Extent2D,
    pipelines: &'a [vk::Pipeline],
    pipeline_layouts: &'a [vk::PipelineLayout],
    buffers: &'a [MtoonVulkanBuffer],
    descriptor_sets: &'a [vk::DescriptorSet],
}

struct MtoonSwapchainCreateContext<'a> {
    instance: &'a ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &'a ash::Device,
    surface_loader: &'a surface::Instance,
    surface: vk::SurfaceKHR,
    swapchain_loader: &'a swapchain::Device,
    command_pool: vk::CommandPool,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    depth_format: vk::Format,
    old_swapchain: vk::SwapchainKHR,
}

fn create_mtoon_swapchain_shell(
    context: MtoonSwapchainCreateContext<'_>,
    window: &Window,
) -> Result<MtoonSwapchainShell, Box<dyn Error>> {
    let support = mtoon_surface_support(&context, window)?;
    let swapchain_info = support.create_info(context.surface, context.old_swapchain);
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
    let depth_plan = ash_depth_attachment_plan(context.depth_format, support.extent);
    let depth = memory.create_image(
        depth_plan.format,
        depth_plan.extent,
        depth_plan.image_usage,
        depth_plan.aspect_mask,
    )?;
    let render_pass_plan = ash_render_pass_creation_plan(
        support.format.format,
        context.depth_format,
        AshColorAttachmentFinalLayout::Present,
        AshRenderPassDependencyPolicy::ColorOnly,
    );
    let render_pass = create_render_pass(context.device, render_pass_plan)?;
    let framebuffer_plan = ash_framebuffer_plan(support.extent);
    let framebuffers = image_views
        .iter()
        .map(|view| {
            let attachments = [*view, depth.view];
            let info = framebuffer_plan.framebuffer_create_info(render_pass, &attachments);
            unsafe { context.device.create_framebuffer(&info, None) }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let command_buffers = allocate_swapchain_command_buffers(
        context.device,
        context.command_pool,
        u32::try_from(framebuffers.len())?,
    )?;
    Ok(MtoonSwapchainShell {
        swapchain,
        image_views,
        depth,
        render_pass,
        framebuffers,
        command_buffers,
        extent: support.extent,
    })
}

fn mtoon_surface_support(
    context: &MtoonSwapchainCreateContext<'_>,
    window: &Window,
) -> Result<vrm_adapter_ash::AshSwapchainSurfacePlan, Box<dyn Error>> {
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
    let size = window.inner_size();
    ash_swapchain_surface_plan(
        capabilities,
        &formats,
        &present_modes,
        vk::Extent2D {
            width: size.width,
            height: size.height,
        },
    )
    .map_err(Into::into)
}

fn select_depth_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<vk::Format, Box<dyn Error>> {
    ash_select_depth_format(|format| unsafe {
        instance.get_physical_device_format_properties(physical_device, format)
    })
    .map_err(Into::into)
}

fn record_mtoon_fallback_texture_uploads(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    textures: &MtoonVulkanFallbackTextures,
    staging: &MtoonVulkanFallbackBuffers,
) {
    for fallback in ASH_FALLBACK_TEXTURES {
        let image = textures.get(fallback);
        let staging = staging.get(fallback);
        let level = [ash_fallback_texture_mip_level(fallback)];
        record_mtoon_texture_upload(device, command_buffer, image.image, staging.buffer, &level);
    }
}

fn record_mtoon_texture_upload(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    image: vk::Image,
    staging_buffer: vk::Buffer,
    mip_levels: &[RgbaMipLevel],
) {
    let plan = ash_texture_upload_command_plan(mip_levels);
    let to_transfer = plan.transfer_dst_barrier_command(image);
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            to_transfer.src_stage_mask,
            to_transfer.dst_stage_mask,
            to_transfer.dependency_flags,
            &[],
            &[],
            &to_transfer.image_barriers,
        );
        let copy = plan.buffer_to_image_copy_command(staging_buffer, image);
        device.cmd_copy_buffer_to_image(
            command_buffer,
            copy.buffer,
            copy.image,
            copy.image_layout,
            copy.regions,
        );
        let to_shader = plan.shader_read_barrier_command(image);
        device.cmd_pipeline_barrier(
            command_buffer,
            to_shader.src_stage_mask,
            to_shader.dst_stage_mask,
            to_shader.dependency_flags,
            &[],
            &[],
            &to_shader.image_barriers,
        );
    }
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
    vertex_entry: CString,
    fragment_entry: CString,
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    in_flight: vk::Fence,
    depth_format: vk::Format,
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
        let command_pool_info =
            ash_resettable_command_pool_plan(queue_family_index).command_pool_create_info();
        let command_pool = unsafe { device.create_command_pool(&command_pool_info, None)? };
        let depth_format = select_depth_format(&instance, physical_device)?;
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
        let semaphore_info = ash_binary_semaphore_plan().semaphore_create_info();
        let image_available = unsafe { device.create_semaphore(&semaphore_info, None)? };
        let render_finished = unsafe { device.create_semaphore(&semaphore_info, None)? };
        let in_flight =
            unsafe { device.create_fence(&ash_signaled_fence_plan().fence_create_info(), None)? };
        let vertex_entry = CString::new(shaders.vertex_entry.as_str())?;
        let fragment_entry = CString::new(shaders.fragment_entry.as_str())?;
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
                depth_format,
                vertex_spv: &shaders.vertex,
                fragment_spv: &shaders.fragment,
                vertex_entry: &vertex_entry,
                fragment_entry: &fragment_entry,
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
            vertex_entry: CString::new(shaders.vertex_entry.as_str())?,
            fragment_entry: CString::new(shaders.fragment_entry.as_str())?,
            image_available,
            render_finished,
            in_flight,
            depth_format,
            swapchain,
        })
    }

    fn render(&mut self, window: &Window) -> Result<RenderStatus, Box<dyn Error>> {
        if window.inner_size().width == 0 || window.inner_size().height == 0 {
            return Ok(RenderStatus::Ok);
        }
        unsafe {
            self.device
                .wait_for_fences(&[self.in_flight], true, u64::MAX)?;
        }
        let acquired = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain.swapchain,
                u64::MAX,
                self.image_available,
                vk::Fence::null(),
            )
        };
        let (image_index, suboptimal) = match ash_classify_swapchain_acquire(acquired)? {
            AshSwapchainAcquireStatus::Acquired {
                image_index,
                suboptimal,
            } => (image_index, suboptimal),
            AshSwapchainAcquireStatus::NeedsRecreate => return Ok(RenderStatus::NeedsRecreate),
        };
        let command_buffer = self.swapchain.command_buffers[image_index as usize];
        let submit_plan = ash_windowed_submit_plan(
            self.image_available,
            self.render_finished,
            command_buffer,
            self.in_flight,
        );
        let submit_info_plan = submit_plan.submit_info_plan();
        let submit = submit_info_plan.submit_infos();
        unsafe {
            self.device.reset_fences(&[submit_info_plan.fence])?;
            self.device
                .queue_submit(self.queue, &submit, submit_info_plan.fence)?;
        }
        let present_plan = ash_windowed_present_plan(
            submit_plan.signal_semaphore,
            self.swapchain.swapchain,
            image_index,
        );
        let present_info_plan = present_plan.present_info_plan();
        let present = present_info_plan.present_info();
        let present_result = unsafe { self.swapchain_loader.queue_present(self.queue, &present) };
        match ash_classify_swapchain_present(suboptimal, present_result)? {
            AshSwapchainPresentStatus::Presented => Ok(RenderStatus::Ok),
            AshSwapchainPresentStatus::NeedsRecreate => Ok(RenderStatus::NeedsRecreate),
        }
    }

    fn update_mesh(&mut self, mesh: &SimpleMesh) -> Result<(), Box<dyn Error>> {
        if u32::try_from(mesh.indices.len())? != self.index_count {
            return Err(format!(
                "animated frame changed index count from {} to {}; restart viewer to rebuild command buffers",
                self.index_count,
                mesh.indices.len()
            )
            .into());
        }
        unsafe {
            self.device
                .wait_for_fences(&[self.in_flight], true, u64::MAX)?;
        }
        write_host_buffer(
            &self.device,
            &self.vertex_buffer,
            bytemuck::cast_slice(&mesh.vertices),
        )?;
        write_host_buffer(
            &self.device,
            &self.index_buffer,
            bytemuck::cast_slice(&mesh.indices),
        )?;
        Ok(())
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
        let new_swapchain = create_swapchain_resources(
            SwapchainCreateContext {
                instance: &self.instance,
                physical_device: self.physical_device,
                device: &self.device,
                surface_loader: &self.surface_loader,
                surface: self.surface,
                swapchain_loader: &self.swapchain_loader,
                command_pool: self.command_pool,
                memory_properties: self.memory_properties,
                depth_format: self.depth_format,
                vertex_spv: &self.vertex_spv,
                fragment_spv: &self.fragment_spv,
                vertex_entry: &self.vertex_entry,
                fragment_entry: &self.fragment_entry,
                vertex_buffer: self.vertex_buffer.buffer,
                index_buffer: self.index_buffer.buffer,
                index_count: self.index_count,
                old_swapchain,
            },
            window,
        );
        destroy_swapchain_handle(&self.swapchain_loader, old_swapchain);
        self.swapchain = new_swapchain?;
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
    depth_format: vk::Format,
    vertex_spv: &'a [u32],
    fragment_spv: &'a [u32],
    vertex_entry: &'a CString,
    fragment_entry: &'a CString,
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
    let swapchain_info = support.create_info(context.surface, context.old_swapchain);
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
    let depth_plan = ash_depth_attachment_plan(context.depth_format, support.extent);
    let depth = memory.create_image(
        depth_plan.format,
        depth_plan.extent,
        depth_plan.image_usage,
        depth_plan.aspect_mask,
    )?;
    let render_pass_plan = ash_render_pass_creation_plan(
        support.format.format,
        context.depth_format,
        AshColorAttachmentFinalLayout::Present,
        AshRenderPassDependencyPolicy::ColorOnly,
    );
    let render_pass = create_render_pass(context.device, render_pass_plan)?;
    let (pipeline_layout, pipeline) = create_simple_pipeline(
        context.device,
        render_pass,
        support.extent,
        context.vertex_spv,
        context.fragment_spv,
        context.vertex_entry,
        context.fragment_entry,
    )?;
    let framebuffer_plan = ash_framebuffer_plan(support.extent);
    let framebuffers = image_views
        .iter()
        .map(|view| {
            let attachments = [*view, depth.view];
            let info = framebuffer_plan.framebuffer_create_info(render_pass, &attachments);
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

fn surface_support(
    context: &SwapchainCreateContext<'_>,
    window: &Window,
) -> Result<vrm_adapter_ash::AshSwapchainSurfacePlan, Box<dyn Error>> {
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
    let size = window.inner_size();
    ash_swapchain_surface_plan(
        capabilities,
        &formats,
        &present_modes,
        vk::Extent2D {
            width: size.width,
            height: size.height,
        },
    )
    .map_err(Into::into)
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
    let info =
        ash_2d_image_view_plan(format, vk::ImageAspectFlags::COLOR).image_view_create_info(image);
    unsafe { device.create_image_view(&info, None) }
}

fn destroy_swapchain_handle(loader: &swapchain::Device, swapchain: vk::SwapchainKHR) {
    if swapchain != vk::SwapchainKHR::null() {
        unsafe {
            loader.destroy_swapchain(swapchain, None);
        }
    }
}

fn create_render_pass(
    device: &ash::Device,
    plan: AshRenderPassCreationPlan,
) -> Result<vk::RenderPass, vk::Result> {
    plan.with_render_pass_create_info(|info| unsafe { device.create_render_pass(&info, None) })
}

fn create_simple_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    extent: vk::Extent2D,
    vertex_spv: &[u32],
    fragment_spv: &[u32],
    vertex_entry: &CString,
    fragment_entry: &CString,
) -> Result<(vk::PipelineLayout, vk::Pipeline), Box<dyn Error>> {
    let vertex_shader = create_shader_module(device, vertex_spv)?;
    let fragment_shader = create_shader_module(device, fragment_spv)?;
    let stages = ash_graphics_shader_stages_plan(vertex_shader, fragment_shader)
        .shader_stage_create_infos(vertex_entry, fragment_entry);
    let layout_info = ash_empty_pipeline_layout_plan().pipeline_layout_create_info();
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None)? };
    let state = ash_position_color_pipeline_state_plan(
        extent,
        std::mem::size_of::<SimpleVertex>() as u32,
        0,
        12,
    );
    let plan = AshGraphicsPipelineCreateInfoPlan {
        shader_stages: stages,
        state,
        layout: pipeline_layout,
        render_pass,
        subpass: 0,
    };
    let pipeline = plan
        .with_graphics_pipeline_create_info(|info| unsafe {
            device.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
        })
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
    let info = ash_shader_module_plan(words).shader_module_create_info();
    unsafe { device.create_shader_module(&info, None) }
}

fn create_mtoon_frame_sync(
    device: &ash::Device,
    plan: AshWindowedFrameSyncPlan,
) -> Result<Vec<MtoonFrameSync>, vk::Result> {
    let mut sync_objects = Vec::with_capacity(plan.fence_count());
    let semaphore_info = ash_binary_semaphore_plan().semaphore_create_info();
    for _ in 0..plan.fence_count() {
        let image_available = match unsafe { device.create_semaphore(&semaphore_info, None) } {
            Ok(semaphore) => semaphore,
            Err(error) => {
                destroy_mtoon_frame_sync(device, sync_objects);
                return Err(error);
            }
        };
        let render_finished = match unsafe { device.create_semaphore(&semaphore_info, None) } {
            Ok(semaphore) => semaphore,
            Err(error) => {
                unsafe {
                    device.destroy_semaphore(image_available, None);
                }
                destroy_mtoon_frame_sync(device, sync_objects);
                return Err(error);
            }
        };
        let in_flight = match unsafe {
            device.create_fence(&ash_signaled_fence_plan().fence_create_info(), None)
        } {
            Ok(fence) => fence,
            Err(error) => {
                unsafe {
                    device.destroy_semaphore(render_finished, None);
                    device.destroy_semaphore(image_available, None);
                }
                destroy_mtoon_frame_sync(device, sync_objects);
                return Err(error);
            }
        };
        sync_objects.push(MtoonFrameSync {
            image_available,
            render_finished,
            in_flight,
        });
    }
    Ok(sync_objects)
}

fn destroy_mtoon_frame_sync(device: &ash::Device, sync_objects: Vec<MtoonFrameSync>) {
    unsafe {
        for sync in sync_objects {
            device.destroy_fence(sync.in_flight, None);
            device.destroy_semaphore(sync.render_finished, None);
            device.destroy_semaphore(sync.image_available, None);
        }
    }
}

fn allocate_swapchain_command_buffers(
    device: &ash::Device,
    command_pool: vk::CommandPool,
    command_buffer_count: u32,
) -> Result<Vec<vk::CommandBuffer>, vk::Result> {
    let allocate_plan =
        ash_primary_command_buffer_allocation_plan(command_pool, command_buffer_count);
    let allocate_info = allocate_plan.command_buffer_allocate_info();
    unsafe { device.allocate_command_buffers(&allocate_info) }
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
    let allocate_plan = ash_primary_command_buffer_allocation_plan(
        context.command_pool,
        u32::try_from(context.framebuffers.len())?,
    );
    let allocate_info = allocate_plan.command_buffer_allocate_info();
    let command_buffers = unsafe { context.device.allocate_command_buffers(&allocate_info)? };
    for (command_buffer, framebuffer) in command_buffers.iter().zip(context.framebuffers) {
        let begin = ash_reusable_command_buffer_begin_plan().command_buffer_begin_info();
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
        let render_pass_begin = ash_render_pass_begin_plan_from_clear_values(
            context.render_pass,
            *framebuffer,
            vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: context.extent,
            },
            clear.to_vec(),
        );
        let render_pass_info = render_pass_begin.render_pass_begin_info();
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
        let plan = ash_host_visible_buffer_plan(usage, bytes.len());
        let size = plan.size;
        let info = plan.buffer_create_info();
        let buffer = unsafe { self.device.create_buffer(&info, None)? };
        let requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type_index =
            self.find_memory_type(requirements.memory_type_bits, plan.memory_property_flags)?;
        let allocate_info =
            ash_memory_allocation_plan(requirements, memory_type_index).memory_allocate_info();
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
        Ok(VulkanBuffer {
            buffer,
            memory,
            byte_size: size,
        })
    }

    fn create_image(
        self,
        format: vk::Format,
        extent: vk::Extent3D,
        usage: vk::ImageUsageFlags,
        aspect_mask: vk::ImageAspectFlags,
    ) -> Result<VulkanImage, Box<dyn Error>> {
        let plan = ash_2d_image_resource_plan(format, extent, 1, usage, aspect_mask);
        let image_info = plan.image_create_info();
        let image = unsafe { self.device.create_image(&image_info, None)? };
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type_index =
            self.find_memory_type(requirements.memory_type_bits, plan.memory_property_flags)?;
        let allocate_info =
            ash_memory_allocation_plan(requirements, memory_type_index).memory_allocate_info();
        let memory = unsafe { self.device.allocate_memory(&allocate_info, None)? };
        unsafe {
            self.device.bind_image_memory(image, memory, 0)?;
        }
        let view_info = plan.image_view_create_info(image);
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
        ash_memory_type_index(self.memory_properties, type_bits, properties).map_err(|err| {
            let device_name = unsafe {
                self.instance
                    .get_physical_device_properties(self.physical_device)
                    .device_name_as_c_str()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "unknown".to_owned())
            };
            format!("{err} on {device_name}").into()
        })
    }
}

struct VulkanBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    byte_size: vk::DeviceSize,
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
