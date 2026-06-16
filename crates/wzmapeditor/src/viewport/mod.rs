//! 3D viewport rendering and GPU resource management.

pub mod atlas;
pub mod atlas_gpu;
#[cfg(target_arch = "wasm32")]
pub mod basis;
pub mod camera;
pub mod ground_types;
pub mod lightmap;
pub mod model_gpu;
pub mod model_loader;
pub mod particles;
pub mod picking;
pub mod pie_mesh;
pub mod pipeline_set;
pub mod pipelines;
pub mod render_types;
pub mod renderer;
pub mod shadow;
pub mod terrain;
pub mod texture_loader;
pub mod thumbnail;
pub mod uniforms;
pub mod water;

use eframe::egui_wgpu;
use egui::PaintCallbackInfo;

use camera::Camera;
use particles::ParticleSystem;
use renderer::EditorRenderer;

/// Shared state stored in the `egui_wgpu` `CallbackResources`.
pub struct ViewportResources {
    pub renderer: EditorRenderer,
    pub camera: Camera,
    pub particle_system: ParticleSystem,
}

impl std::fmt::Debug for ViewportResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewportResources").finish_non_exhaustive()
    }
}

/// A paint callback that renders the 3D terrain viewport.
pub struct TerrainPaintCallback {
    pub show_grid: bool,
    pub show_border: bool,
    pub show_heatmap: bool,
    pub show_viewshed: bool,
    /// Snapshot from the UI phase so `prepare()` can write the latest
    /// time/camera values before the render pass begins.
    pub camera: Camera,
    pub brush_highlight: [f32; 4],
    pub brush_highlight_extra: [[f32; 4]; 3],
    /// When false the shadow pass is skipped and the previous frame's
    /// depth texture is reused (wgpu textures retain contents until
    /// overwritten, so the main pass still samples a valid shadow).
    pub run_shadow: bool,
}

impl egui_wgpu::CallbackTrait for TerrainPaintCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(resources) = callback_resources.get_mut::<ViewportResources>() {
            resources.renderer.show_grid = self.show_grid;
            resources.renderer.show_border = self.show_border;
            resources.renderer.show_heatmap = self.show_heatmap;
            resources.renderer.show_viewshed = self.show_viewshed;

            // Encoder copy keeps the update in the same command stream
            // as the render pass.
            resources.renderer.update_uniforms(
                queue,
                egui_encoder,
                &self.camera,
                self.brush_highlight,
                self.brush_highlight_extra,
            );

            // Renderer forces the pass on first invocation regardless of
            // the dirty flag, then skips when clear. Saves ~5-10ms/frame
            // at idle on populated maps.
            resources
                .renderer
                .run_shadow_pass(egui_encoder, self.run_shadow);
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<ViewportResources>() else {
            return;
        };
        let renderer = &resources.renderer;

        let Some(ref terrain_gpu) = renderer.terrain_gpu else {
            return;
        };

        let viewport = info.viewport_in_pixels();
        render_pass.set_viewport(
            viewport.left_px as f32,
            viewport.top_px as f32,
            viewport.width_px as f32,
            viewport.height_px as f32,
            0.0,
            1.0,
        );

        let clip = info.clip_rect_in_pixels();
        render_pass.set_scissor_rect(
            clip.left_px as u32,
            clip.top_px as u32,
            clip.width_px.max(1) as u32,
            clip.height_px.max(1) as u32,
        );

        if renderer.settings.sky_enabled {
            render_pass.set_pipeline(&renderer.pipelines.sky);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.draw(0..3, 0..1); // Fullscreen triangle, no vertex buffer.
        }

        let use_high = renderer.settings.terrain_quality == renderer::TerrainQuality::High
            && renderer.ground.high_bind_group.is_some();
        let use_medium = !use_high
            && renderer.settings.terrain_quality != renderer::TerrainQuality::Classic
            && renderer.ground.has_textures
            && renderer.ground.bind_group.is_some();

        // Log pipeline switches once, not every frame.
        {
            use std::sync::atomic::{AtomicU8, Ordering};
            // 255 sentinel; valid pipeline IDs are 0-2.
            static LAST_PIPELINE: AtomicU8 = AtomicU8::new(255);
            let pipeline_id: u8 = if use_high { 2 } else { u8::from(use_medium) };
            if LAST_PIPELINE.swap(pipeline_id, Ordering::Relaxed) != pipeline_id {
                let name = ["Classic", "Medium", "High"][pipeline_id as usize];
                log::info!(
                    "Terrain pipeline: {name} (requested={:?}, ground_tex={}, ground_bind={}, high_bind={})",
                    renderer.settings.terrain_quality,
                    renderer.ground.has_textures,
                    renderer.ground.bind_group.is_some(),
                    renderer.ground.high_bind_group.is_some(),
                );
            }
        }
        if use_high {
            render_pass.set_pipeline(&renderer.pipelines.terrain_high);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_bind_group(1, &renderer.atlas.bind_group, &[]);
            render_pass.set_bind_group(2, &renderer.shadow.bind_group, &[]);
            render_pass.set_bind_group(3, renderer.ground.high_bind_group.as_ref().unwrap(), &[]);
        } else if use_medium {
            render_pass.set_pipeline(&renderer.pipelines.terrain_medium);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_bind_group(1, &renderer.atlas.bind_group, &[]);
            render_pass.set_bind_group(2, &renderer.shadow.bind_group, &[]);
            render_pass.set_bind_group(3, renderer.ground.bind_group.as_ref().unwrap(), &[]);
        } else {
            render_pass.set_pipeline(&renderer.pipelines.terrain);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_bind_group(1, &renderer.atlas.bind_group, &[]);
            render_pass.set_bind_group(2, &renderer.shadow.bind_group, &[]);
        }
        render_pass.set_vertex_buffer(0, terrain_gpu.vertex_buffer.slice(..));
        render_pass.set_index_buffer(
            terrain_gpu.index_buffer.slice(..),
            wgpu::IndexFormat::Uint32,
        );
        render_pass.draw_indexed(0..terrain_gpu.index_count, 0, 0..1);

        if !renderer.models.draw_calls.is_empty() {
            render_pass.set_pipeline(&renderer.models.pipeline);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_bind_group(2, &renderer.shadow.bind_group, &[]);

            for draw_call in &renderer.models.draw_calls {
                let key: &str = draw_call.model_key.as_ref();
                let gpu_model = renderer.models.cache.get(key);
                let inst_buf = renderer.models.instance_buffer(key);
                if let (Some(gpu_model), Some(inst_buf)) = (gpu_model, inst_buf) {
                    render_pass.set_bind_group(1, &gpu_model.texture_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, gpu_model.vertex_buffer.slice(..));
                    render_pass.set_vertex_buffer(1, inst_buf.slice(..));
                    render_pass.set_index_buffer(
                        gpu_model.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    render_pass.draw_indexed(
                        0..gpu_model.index_count,
                        0,
                        0..draw_call.instance_count,
                    );
                }
            }
        }

        if renderer.settings.water_enabled
            && let Some(ref water_gpu) = renderer.water_gpu
        {
            render_pass.set_pipeline(&renderer.pipelines.water);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_bind_group(1, &renderer.water.bind_group, &[]);
            render_pass.set_vertex_buffer(0, water_gpu.vertex_buffer.slice(..));
            render_pass
                .set_index_buffer(water_gpu.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..water_gpu.index_count, 0, 0..1);
        }

        if renderer.particles.index_count > 0
            && let (Some(vb), Some(ib)) = (
                &renderer.particles.vertex_buffer,
                &renderer.particles.index_buffer,
            )
        {
            render_pass.set_pipeline(&renderer.particles.pipeline);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_vertex_buffer(0, vb.slice(..));
            render_pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..renderer.particles.index_count, 0, 0..1);
        }

        if renderer.show_grid {
            render_pass.set_pipeline(&renderer.pipelines.grid);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_vertex_buffer(0, terrain_gpu.vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                terrain_gpu.index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..terrain_gpu.index_count, 0, 0..1);
        }

        if renderer.show_border {
            render_pass.set_pipeline(&renderer.pipelines.border);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_vertex_buffer(0, terrain_gpu.vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                terrain_gpu.index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..terrain_gpu.index_count, 0, 0..1);
        }

        if renderer.show_heatmap
            && let Some(ref heatmap_bg) = renderer.heatmap.bind_group
        {
            render_pass.set_pipeline(&renderer.heatmap.pipeline);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_bind_group(1, heatmap_bg, &[]);
            render_pass.set_vertex_buffer(0, terrain_gpu.vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                terrain_gpu.index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..terrain_gpu.index_count, 0, 0..1);
        }

        if renderer.show_viewshed
            && renderer.viewshed.ring_vertex_count > 0
            && let Some(ref vb) = renderer.viewshed.ring_vertex_buffer
        {
            render_pass.set_pipeline(&renderer.viewshed.ring_pipeline);
            render_pass.set_bind_group(0, &renderer.uniforms.bind_group, &[]);
            render_pass.set_vertex_buffer(0, vb.slice(..));
            render_pass.draw(0..renderer.viewshed.ring_vertex_count, 0..1);
        }
    }
}

/// Initialize the viewport resources in the `egui_wgpu` renderer's callback resources.
pub fn init_viewport_resources(render_state: &egui_wgpu::RenderState) {
    let device = &render_state.device;
    let queue = &render_state.queue;
    let target_format = render_state.target_format;

    let renderer = EditorRenderer::new(device, queue, target_format);
    // Replaced when a map loads.
    let camera = Camera::for_map(64, 64);

    let resources = ViewportResources {
        renderer,
        camera,
        particle_system: ParticleSystem::new(),
    };

    render_state
        .renderer
        .write()
        .callback_resources
        .insert(resources);
}
