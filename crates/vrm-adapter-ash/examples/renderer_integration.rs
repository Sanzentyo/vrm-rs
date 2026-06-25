use ash::vk;
use ash::vk::Handle;
use clap::Parser;
use std::error::Error;
use vrm_adapter_ash::{
    AshBufferRole, AshCommandRecordHandleAccess, AshCommandRecordResources,
    AshMtoonMaterializationPlan, AshRendererFrame, AshResolvedCommand, AshSamplerPlan,
    AshVrmFramePlanOptions, ash_mtoon_materialization_plan, ash_renderer_frame_from_plan,
    frame_plan_from_options,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct MockBufferHandle(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct MockImageHandle(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct MockPipelineHandle(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct MockDescriptorSetHandle(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct MockSamplerHandle(u64);

#[derive(Clone, Debug)]
struct MockRecordedDraw {
    pipeline: Option<MockPipelineHandle>,
    descriptor_set: Option<MockDescriptorSetHandle>,
    vertex_buffer: MockBufferHandle,
    index_buffer: MockBufferHandle,
    index_count: u32,
    render_order: i32,
    phase_order: i32,
}

#[derive(Default)]
struct MockAshRenderer {
    next_handle: u64,
    buffers: Vec<(MockBufferHandle, vk::BufferUsageFlags, usize)>,
    images: Vec<(MockImageHandle, vk::Format, vk::Extent3D)>,
    samplers: Vec<(MockSamplerHandle, AshSamplerPlan)>,
    pipelines: Vec<(MockPipelineHandle, vk::PrimitiveTopology, vk::CullModeFlags)>,
    descriptor_sets: Vec<(MockDescriptorSetHandle, usize)>,
    draws: Vec<MockRecordedDraw>,
}

impl MockAshRenderer {
    fn upload_frame(&mut self, frame: &AshRendererFrame, plan: &AshMtoonMaterializationPlan) {
        self.buffers.clear();
        for buffer in &frame.buffers {
            let handle = self.alloc_buffer(buffer.role);
            self.buffers
                .push((handle, buffer.usage, buffer.bytes.len()));
        }
        self.images.clear();
        for texture in &frame.textures {
            let handle = self.alloc_image();
            self.images
                .push((handle, texture.upload.format, texture.upload.extent));
        }
        self.samplers.clear();
        for sampler_resource in &plan.sampler_resources {
            let handle = self.alloc_sampler();
            self.samplers.push((handle, sampler_resource.sampler));
        }
        self.pipelines.clear();
        for pipeline in &frame.pipelines {
            let handle = self.alloc_pipeline();
            self.pipelines
                .push((handle, pipeline.key.topology, pipeline.key.cull_mode));
        }
        self.descriptor_sets.clear();
        for layout in &plan.descriptor_set_layouts {
            let handle = self.alloc_descriptor_set();
            self.descriptor_sets.push((handle, layout.bindings.len()));
        }
        let pipeline_handles = self
            .pipelines
            .iter()
            .map(|(handle, _, _)| vk::Pipeline::from_raw(handle.0))
            .collect::<Vec<_>>();
        let pipeline_layout_handles = vec![vk::PipelineLayout::null(); plan.pipeline_layouts.len()];
        let descriptor_set_handles = self
            .descriptor_sets
            .iter()
            .map(|(handle, _)| vk::DescriptorSet::from_raw(handle.0))
            .collect::<Vec<_>>();
        self.draws.clear();
        let mut pipeline = None;
        let mut descriptor_set = None;
        let mut vertex_buffer = None;
        let mut index_buffer = None;
        let mut planned_draws = frame.draw_calls.iter().filter(|draw| {
            draw.pipeline_plan_index.is_some()
                && draw.descriptor_set_index.is_some()
                && draw.index_count > 0
        });
        for command in plan
            .resolve_commands(
                AshCommandRecordResources::new(
                    &frame.pipelines,
                    &pipeline_handles,
                    &pipeline_layout_handles,
                    &descriptor_set_handles,
                    &self.buffers,
                ),
                AshCommandRecordHandleAccess {
                    buffer: |buffer: &(MockBufferHandle, vk::BufferUsageFlags, usize)| {
                        vk::Buffer::from_raw(buffer.0.0)
                    },
                },
            )
            .expect("mock renderer command resolution")
        {
            match command {
                AshResolvedCommand::BindGraphicsPipeline(bind) => {
                    pipeline = Some(MockPipelineHandle(bind.pipeline.as_raw()));
                }
                AshResolvedCommand::BindDescriptorSet(bind) => {
                    descriptor_set =
                        Some(MockDescriptorSetHandle(bind.descriptor_sets[0].as_raw()));
                }
                AshResolvedCommand::BindVertexBuffer(bind) => {
                    vertex_buffer = Some(MockBufferHandle(bind.buffers[0].as_raw()));
                }
                AshResolvedCommand::BindIndexBuffer(bind) => {
                    index_buffer = Some(MockBufferHandle(bind.buffer.as_raw()));
                }
                AshResolvedCommand::DrawIndexed(draw_command) => {
                    let draw = planned_draws
                        .next()
                        .expect("drawable command references a planned draw");
                    self.draws.push(MockRecordedDraw {
                        pipeline,
                        descriptor_set,
                        vertex_buffer: vertex_buffer.expect("vertex buffer bound before draw"),
                        index_buffer: index_buffer.expect("index buffer bound before draw"),
                        index_count: draw_command.index_count,
                        render_order: draw.render_order,
                        phase_order: draw.phase_order,
                    });
                }
            }
        }
    }

    fn alloc_buffer(&mut self, role: AshBufferRole) -> MockBufferHandle {
        let base = match role {
            AshBufferRole::Vertex => 10_000,
            AshBufferRole::Index => 20_000,
            AshBufferRole::OwnerSampleOverride => 25_000,
        };
        self.next_handle += 1;
        MockBufferHandle(base + self.next_handle)
    }

    fn alloc_image(&mut self) -> MockImageHandle {
        self.next_handle += 1;
        MockImageHandle(30_000 + self.next_handle)
    }

    fn alloc_sampler(&mut self) -> MockSamplerHandle {
        self.next_handle += 1;
        MockSamplerHandle(35_000 + self.next_handle)
    }

    fn alloc_pipeline(&mut self) -> MockPipelineHandle {
        self.next_handle += 1;
        MockPipelineHandle(40_000 + self.next_handle)
    }

    fn alloc_descriptor_set(&mut self) -> MockDescriptorSetHandle {
        self.next_handle += 1;
        MockDescriptorSetHandle(50_000 + self.next_handle)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = AshVrmFramePlanOptions::parse();
    let frame_plan = frame_plan_from_options(&options)?;
    let renderer_frame = ash_renderer_frame_from_plan(&frame_plan);
    let materialization = ash_mtoon_materialization_plan(
        &renderer_frame,
        vk::Extent2D {
            width: 512,
            height: 512,
        },
        Default::default(),
    )?;
    let mut renderer = MockAshRenderer::default();
    renderer.upload_frame(&renderer_frame, &materialization);
    let total_indices = renderer
        .draws
        .iter()
        .map(|draw| draw.index_count)
        .sum::<u32>();
    let command_checksum = renderer.draws.iter().fold(0_u64, |acc, draw| {
        acc ^ draw.pipeline.map(|handle| handle.0).unwrap_or_default()
            ^ draw
                .descriptor_set
                .map(|handle| handle.0)
                .unwrap_or_default()
            ^ draw.vertex_buffer.0
            ^ draw.index_buffer.0
            ^ draw.render_order as u64
            ^ draw.phase_order as u64
    });
    let sampler_policy_checksum = renderer.samplers.iter().fold(0_u64, |acc, (handle, plan)| {
        acc ^ handle.0
            ^ sampler_filter_code(plan.mag_filter)
            ^ (sampler_filter_code(plan.min_filter) << 4)
            ^ (sampler_mipmap_code(plan.mipmap_mode) << 8)
            ^ (sampler_address_code(plan.address_mode_u) << 12)
            ^ (sampler_address_code(plan.address_mode_v) << 16)
            ^ ((if plan.normal_map_decode { 1 } else { 0 }) << 20)
            ^ (plan.max_lod.to_bits() as u64)
    });
    println!(
        "ash renderer example: {} buffers, {} images, {} samplers, {} descriptor sets, {} commands, {} skipped, {} draws, {} indices, {} persistent resources, {} dynamic resources, checksum {}, sampler checksum {}",
        renderer.buffers.len(),
        renderer.images.len(),
        renderer.samplers.len(),
        renderer.descriptor_sets.len(),
        materialization.draw_command_count(),
        materialization.drawable.skipped_draws.len(),
        renderer.draws.len(),
        total_indices,
        materialization
            .resource_manifest
            .persistent_resource_count(),
        materialization.frame_dynamic_resource_count(),
        command_checksum,
        sampler_policy_checksum
    );
    Ok(())
}

fn sampler_filter_code(filter: vk::Filter) -> u64 {
    match filter {
        vk::Filter::NEAREST => 1,
        vk::Filter::LINEAR => 2,
        _ => 0,
    }
}

fn sampler_mipmap_code(mode: vk::SamplerMipmapMode) -> u64 {
    match mode {
        vk::SamplerMipmapMode::NEAREST => 1,
        vk::SamplerMipmapMode::LINEAR => 2,
        _ => 0,
    }
}

fn sampler_address_code(mode: vk::SamplerAddressMode) -> u64 {
    match mode {
        vk::SamplerAddressMode::CLAMP_TO_EDGE => 1,
        vk::SamplerAddressMode::MIRRORED_REPEAT => 2,
        vk::SamplerAddressMode::REPEAT => 3,
        _ => 0,
    }
}
