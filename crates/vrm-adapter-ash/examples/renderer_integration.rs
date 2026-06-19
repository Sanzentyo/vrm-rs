use ash::vk;
use clap::Parser;
use std::error::Error;
use vrm_adapter_ash::{
    AshBufferRole, AshRendererFrame, AshVrmFramePlanOptions, ash_renderer_frame_from_plan,
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
    pipelines: Vec<(MockPipelineHandle, vk::PrimitiveTopology, vk::CullModeFlags)>,
    descriptor_sets: Vec<(MockDescriptorSetHandle, usize)>,
    draws: Vec<MockRecordedDraw>,
}

impl MockAshRenderer {
    fn upload_frame(&mut self, frame: &AshRendererFrame) {
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
        self.pipelines.clear();
        for pipeline in &frame.pipelines {
            let handle = self.alloc_pipeline();
            self.pipelines
                .push((handle, pipeline.key.topology, pipeline.key.cull_mode));
        }
        self.descriptor_sets.clear();
        for set in &frame.descriptor_sets {
            let handle = self.alloc_descriptor_set();
            self.descriptor_sets.push((handle, set.bindings.len()));
        }
        self.draws = frame
            .draw_calls
            .iter()
            .map(|draw| MockRecordedDraw {
                pipeline: draw
                    .pipeline_plan_index
                    .and_then(|index| self.pipelines.get(index).map(|pipeline| pipeline.0)),
                descriptor_set: draw
                    .descriptor_set_index
                    .and_then(|index| self.descriptor_sets.get(index).map(|set| set.0)),
                vertex_buffer: self.buffers[draw.vertex_buffer_index].0,
                index_buffer: self.buffers[draw.index_buffer_index].0,
                index_count: draw.index_count,
                render_order: draw.render_order,
                phase_order: draw.phase_order,
            })
            .collect();
    }

    fn alloc_buffer(&mut self, role: AshBufferRole) -> MockBufferHandle {
        let base = match role {
            AshBufferRole::Vertex => 10_000,
            AshBufferRole::Index => 20_000,
        };
        self.next_handle += 1;
        MockBufferHandle(base + self.next_handle)
    }

    fn alloc_image(&mut self) -> MockImageHandle {
        self.next_handle += 1;
        MockImageHandle(30_000 + self.next_handle)
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
    let mut renderer = MockAshRenderer::default();
    renderer.upload_frame(&renderer_frame);
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
    println!(
        "ash renderer example: {} buffers, {} images, {} descriptor sets, {} draws, {} indices, checksum {}",
        renderer.buffers.len(),
        renderer.images.len(),
        renderer.descriptor_sets.len(),
        renderer.draws.len(),
        total_indices,
        command_checksum
    );
    Ok(())
}
