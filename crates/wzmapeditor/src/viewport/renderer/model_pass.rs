//! Model rendering: pipelines, texture bind group layout, and offscreen
//! thumbnail encoding.

use std::collections::HashMap;

use eframe::wgpu::util::DeviceExt;
use glam::Mat4;

use super::super::model_gpu::ModelResources;
use super::super::pie_mesh::{ModelInstance, ModelVertex};
use super::super::pipelines;
use super::super::render_types::SUN_DIRECTION;
use super::super::shadow::ShadowResources;
use super::super::thumbnail::{THUMB_FORMAT, ThumbnailEntry, ThumbnailResources};
use super::super::uniforms::Uniforms;
use super::util::{BindGroupLayoutBuilder, PipelineRecipe, pipeline_with_recipe};

/// Outputs produced by [`build`].
pub(super) struct ModelPassBuild {
    pub texture_layout: wgpu::BindGroupLayout,
    pub model_pipeline: wgpu::RenderPipeline,
    pub thumb_pipeline: wgpu::RenderPipeline,
}

/// Build the model texture bind group layout and the model + thumbnail pipelines.
pub(super) fn build(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    shadow_layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> ModelPassBuild {
    // Sampler lives in the per-frame group(0), not duplicated per model:
    // DX12 caps the sampler descriptor heap at 2048 entries and a session
    // caches thousands of model bind groups (one per PIE, multiplied across
    // tilesets during splash preload).
    let texture_layout = BindGroupLayoutBuilder::new("model_texture_bind_group_layout")
        .texture_2d_array_filterable(0, wgpu::ShaderStages::FRAGMENT)
        .build(device);

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("model_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/model.wgsl").into()),
    });

    let layout = pipelines::create_pipeline_layout(
        device,
        "model_pipeline_layout",
        &[uniform_layout, &texture_layout, shadow_layout],
    );

    let model_vb = &[ModelVertex::desc(), ModelInstance::desc()];

    let model_pipeline = pipeline_with_recipe(
        device,
        "model_pipeline",
        &layout,
        &shader,
        model_vb,
        target_format,
        PipelineRecipe::MODEL,
    );
    let thumb_pipeline = pipeline_with_recipe(
        device,
        "thumb_pipeline",
        &layout,
        &shader,
        model_vb,
        THUMB_FORMAT,
        PipelineRecipe::MODEL,
    );

    ModelPassBuild {
        texture_layout,
        model_pipeline,
        thumb_pipeline,
    }
}

/// Construct the empty `ModelResources` carrier given the prebuilt pipeline,
/// texture bind group layout, default atlas view, and offscreen thumb pipeline.
pub(super) fn make_model_resources(
    pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    default_atlas_view: wgpu::TextureView,
    thumb_pipeline: wgpu::RenderPipeline,
) -> ModelResources {
    ModelResources {
        pipeline,
        texture_layout,
        cache: HashMap::new(),
        draw_calls: Vec::new(),
        default_atlas_view,
        page_atlas_cache: HashMap::new(),
        thumb_pipeline,
        instance_buffers: HashMap::new(),
    }
}

/// Encode a thumbnail render pass for the given model entries into `target`.
///
/// Returns the encoder unsubmitted so callers can either submit directly
/// (preview path) or append a staging-buffer copy first (browser path).
/// Returns `None` if there is nothing to draw.
pub(super) fn encode_thumbnail_pass(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    models_state: &ModelResources,
    shadow: &ShadowResources,
    entries: &[ThumbnailEntry<'_>],
    y_rotation: f32,
    target: &ThumbnailResources,
) -> Option<wgpu::CommandEncoder> {
    if entries.is_empty() {
        return None;
    }

    let mut aabb_min = glam::Vec3::splat(f32::MAX);
    let mut aabb_max = glam::Vec3::splat(f32::MIN);
    for entry in entries {
        if let Some(gpu) = models_state.cache.get(entry.model_key) {
            let lo = glam::Vec3::from(gpu.aabb_min) + entry.offset;
            let hi = glam::Vec3::from(gpu.aabb_max) + entry.offset;
            aabb_min = aabb_min.min(lo);
            aabb_max = aabb_max.max(hi);
        }
    }
    if (aabb_min.x - f32::MAX).abs() < f32::EPSILON {
        return None;
    }

    let center = (aabb_min + aabb_max) * 0.5;
    let extent = (aabb_max - aabb_min).max_element().max(1.0);
    let cam_dir = glam::Vec3::new(0.6, 0.8, 1.0).normalize();
    let rot = glam::Quat::from_rotation_y(y_rotation);
    // 1.2 padding keeps the model off the viewport edge.
    let eye = center + rot * cam_dir * extent * 1.2;
    let view = Mat4::look_at_rh(eye, center, glam::Vec3::Y);
    let half = extent * 0.7;
    let proj = Mat4::orthographic_rh(-half, half, -half, half, -extent * 3.0, extent * 3.0);
    let mvp = proj * view;

    let uniforms = Uniforms {
        mvp: mvp.to_cols_array_2d(),
        sun_direction: [SUN_DIRECTION[0], SUN_DIRECTION[1], SUN_DIRECTION[2], 0.0],
        brush_highlight: [0.0; 4],
        brush_highlight_extra: [[0.0; 4]; 3],
        camera_pos: [eye.x, eye.y, eye.z, 0.0],
        // a=0 disables fog in the shader.
        fog_color: [0.0, 0.0, 0.0, 0.0],
        fog_params: [0.0; 4],
        shadow_mvp: Mat4::IDENTITY.to_cols_array_2d(),
        map_world_size: [1.0, 1.0, 0.0, 0.0],
    };
    queue.write_buffer(&target.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

    let mut instances = Vec::new();
    let mut draw_ranges: Vec<(&str, u32, u32)> = Vec::new();
    for entry in entries {
        if models_state.cache.contains_key(entry.model_key) {
            let model_matrix = Mat4::from_translation(entry.offset);
            let start = instances.len() as u32;
            instances.push(ModelInstance {
                model_matrix: model_matrix.to_cols_array_2d(),
                team_color: entry.team_color,
            });
            draw_ranges.push((entry.model_key, start, 1));
        }
    }
    if instances.is_empty() {
        return None;
    }

    let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("thumb_instance"),
        contents: bytemuck::cast_slice(&instances),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("thumb_encoder"),
    });

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("thumb_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    // #283445, matching the egui panel bg.
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.157,
                        g: 0.173,
                        b: 0.204,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_pipeline(&models_state.thumb_pipeline);
        pass.set_bind_group(0, &target.uniform_bind_group, &[]);
        pass.set_bind_group(2, &shadow.bind_group, &[]);

        for &(key, start, count) in &draw_ranges {
            if let Some(gpu_model) = models_state.cache.get(key) {
                pass.set_bind_group(1, &gpu_model.texture_bind_group, &[]);
                pass.set_vertex_buffer(0, gpu_model.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, instance_buffer.slice(..));
                pass.set_index_buffer(gpu_model.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..gpu_model.index_count, 0, start..start + count);
            }
        }
    }

    Some(encoder)
}

/// Create the 1x1x4 default model atlas (white diffuse, black tcmask, flat-up
/// normal, black specular). Normal/specular layers carry alpha=0 so the
/// shader's `has_normalmap` / `has_specularmap` checks see them as absent (real
/// maps are uploaded with alpha=255).
pub(super) fn make_default_atlas_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> wgpu::TextureView {
    let pixels: [u8; 16] = [
        255, 255, 255, 255, // diffuse white (sRGB-encoded)
        0, 0, 0, 0, // tcmask: no mask
        128, 128, 255, 0, // flat-up normal, alpha=0 marker for "absent"
        0, 0, 0, 0, // zero specular, alpha=0 marker for "absent"
    ];
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("default_model_atlas"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 4,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &pixels,
    );
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

impl super::EditorRenderer {
    /// Upload a model to the GPU with shared texture page caching.
    pub fn upload_model(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: &str,
        mesh: &super::super::pie_mesh::ModelMesh,
        diffuse: Option<super::super::model_gpu::TexturePageRef<'_>>,
        tcmask: Option<super::super::model_gpu::TexturePageRef<'_>>,
        normal: Option<super::super::model_gpu::TexturePageRef<'_>>,
        specular: Option<super::super::model_gpu::TexturePageRef<'_>>,
    ) {
        self.models
            .upload_model(device, queue, key, mesh, diffuse, tcmask, normal, specular);
    }

    /// Begin an offscreen thumbnail render and GPU-to-CPU readback.
    ///
    /// Renders into the small [`super::THUMB_SIZE`] target and returns a
    /// handle to poll for completion, or `None` if there is nothing to
    /// render (none of the requested models are uploaded). Used by the asset
    /// browser and the splash-time preload pass.
    pub fn begin_thumbnail_readback(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        models: &[ThumbnailEntry<'_>],
        y_rotation: f32,
    ) -> Option<super::super::thumbnail::ThumbnailReadback> {
        let slot = self.thumb.claim_slot()?;
        let Some(encoded) = encode_thumbnail_pass(
            device,
            queue,
            &self.models,
            &self.shadow,
            models,
            y_rotation,
            &self.thumb,
        ) else {
            self.thumb.free_slot(slot);
            return None;
        };
        Some(super::super::thumbnail::begin_read_back(
            queue,
            encoded,
            &self.thumb,
            slot,
            super::THUMB_SIZE,
        ))
    }

    /// Decode a completed thumbnail readback into CPU pixels.
    ///
    /// Call only after the handle from [`Self::begin_thumbnail_readback`]
    /// reports [`ReadbackStatus::Ready`], passing that handle's
    /// [`slot`](super::super::thumbnail::ThumbnailReadback::slot).
    ///
    /// [`ReadbackStatus::Ready`]: super::super::thumbnail::ReadbackStatus::Ready
    pub fn finish_thumbnail_readback(&self, slot: usize) -> egui::ColorImage {
        super::super::thumbnail::finish_read_back(&self.thumb, slot, super::THUMB_SIZE)
    }

    /// Render a model to the larger [`super::PREVIEW_THUMB_SIZE`] target.
    ///
    /// Used by the droid designer's live 3D preview. Unlike
    /// [`Self::begin_thumbnail_readback`], this path does not copy the
    /// rendered pixels back to the CPU: the preview's color texture is
    /// registered directly with `egui_wgpu` and sampled from the GPU.
    pub fn render_preview_thumbnail(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        models: &[ThumbnailEntry<'_>],
        y_rotation: f32,
    ) -> bool {
        let Some(encoder) = encode_thumbnail_pass(
            device,
            queue,
            &self.models,
            &self.shadow,
            models,
            y_rotation,
            &self.preview_thumb,
        ) else {
            return false;
        };
        queue.submit(std::iter::once(encoder.finish()));
        true
    }

    /// Create a fresh texture view over the preview target.
    pub fn preview_color_view_fresh(&self) -> wgpu::TextureView {
        self.preview_thumb
            .color_texture
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Prepare draw calls for the current frame's object instances.
    pub fn prepare_object_draw_calls(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances_by_model: &rustc_hash::FxHashMap<
            std::sync::Arc<str>,
            Vec<super::super::pie_mesh::ModelInstance>,
        >,
    ) {
        self.models
            .prepare_draw_calls(device, queue, instances_by_model);
    }
}
