//! Heatmap and viewshed overlay pipelines.
//!
//! Heatmap colours terrain by traversal cost using a terrain-type LUT plus
//! a per-propulsion speed buffer. Viewshed rings are drawn as a line list;
//! wgpu 29 forbids depth bias on non-triangle topology, so the ring
//! pipeline cannot reuse the shared overlay helper.

use super::super::pipelines;
use super::super::render_types::{HeatmapState, RingVertex, ViewshedState};
use super::super::terrain::TerrainVertex;
use super::util::{BindGroupLayoutBuilder, OVERLAY_DEPTH_BIAS, biased_overlay_pipeline};

/// Outputs from [`build`].
pub(super) struct HeatmapViewshedBuild {
    pub heatmap: HeatmapState,
    pub viewshed: ViewshedState,
}

/// Build heatmap and viewshed pipelines, layouts, and the heatmap GPU buffers.
pub(super) fn build(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> HeatmapViewshedBuild {
    let heatmap_bind_group_layout = BindGroupLayoutBuilder::new("heatmap_bind_group_layout")
        .uniform_buffer(0, wgpu::ShaderStages::FRAGMENT)
        .uniform_buffer(1, wgpu::ShaderStages::FRAGMENT)
        .build(device);

    let heatmap_pipeline_layout = pipelines::create_pipeline_layout(
        device,
        "heatmap_pipeline_layout",
        &[uniform_layout, &heatmap_bind_group_layout],
    );

    let heatmap_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("heatmap_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/heatmap.wgsl").into()),
    });

    let heatmap_pipeline = biased_overlay_pipeline(
        device,
        "heatmap_pipeline",
        &heatmap_pipeline_layout,
        &heatmap_shader,
        &[TerrainVertex::desc()],
        target_format,
        OVERLAY_DEPTH_BIAS,
    );

    // 512 u32 = 2048 bytes terrain-type LUT, written via queue.write_buffer.
    let lut_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("heatmap_lut_buffer"),
        size: 512 * 4,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let speed_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("heatmap_speed_buffer"),
        size: 48,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let viewshed_ring_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("viewshed_ring_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/viewshed_ring.wgsl").into()),
    });
    let viewshed_ring_pipeline_layout = pipelines::create_pipeline_layout(
        device,
        "viewshed_ring_pipeline_layout",
        &[uniform_layout],
    );
    let viewshed_ring_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("viewshed_ring_pipeline"),
        layout: Some(&viewshed_ring_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &viewshed_ring_shader,
            entry_point: Some("vs_main"),
            buffers: &[RingVertex::desc()],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &viewshed_ring_shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            // wgpu 29 forbids depth bias on non-triangle topology.
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    HeatmapViewshedBuild {
        heatmap: HeatmapState {
            pipeline: heatmap_pipeline,
            bind_group_layout: heatmap_bind_group_layout,
            bind_group: None,
            lut_buffer,
            speed_buffer,
        },
        viewshed: ViewshedState {
            ring_pipeline: viewshed_ring_pipeline,
            ring_vertex_buffer: None,
            ring_vertex_count: 0,
        },
    }
}

impl super::EditorRenderer {
    /// Upload heatmap data: terrain type LUT and speed factors for the selected propulsion.
    pub fn upload_heatmap_data(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain_type_lut: &[u32; 512],
        speed_factors: &[f32; 12],
    ) {
        self.heatmap
            .upload_data(device, queue, terrain_type_lut, speed_factors);
    }

    /// Push a fresh viewshed frame to the GPU.
    pub fn upload_viewshed_frame(
        &mut self,
        device: &wgpu::Device,
        frame: &crate::viewshed::ViewshedFrame,
    ) {
        self.viewshed.upload(device, frame);
    }
}
