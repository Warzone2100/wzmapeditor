//! GPU render pipeline orchestrator for terrain, grid, 3D models, sky, water, and shadows.
//!
//! Each per-pass submodule owns its pipelines, layouts, and shader inits.
//! `EditorRenderer` bundles those resources with cross-cutting state
//! (uniform buffers, lightmap, particles, thumbnail targets, render settings).

mod heatmap_viewshed;
mod model_pass;
mod shadow_pass;
mod terrain_pass;
mod util;
mod water_sky;

use web_time::Instant;

use eframe::wgpu::util::DeviceExt as _;

use super::atlas_gpu::AtlasState;
use super::lightmap::Lightmap;
use super::particles::ParticleVertex;
use super::pipeline_set::PipelineSet;
use super::pipelines;
use super::uniforms::UniformState;
use util::{PipelineRecipe, pipeline_with_recipe};

pub use super::render_types::{
    FOG_ARIZONA, FOG_ROCKIES, FOG_URBAN, GroundTextureState, HeatmapState, LightmapState,
    ParticleState, RenderSettings, TerrainGpuData, TerrainQuality, ViewshedState, WaterGpuData,
    WaterState,
};

pub use super::model_gpu::{GpuModel, ModelResources, TexturePageRef};
use super::shadow::CachedShadowMvp;
pub use super::shadow::ShadowResources;

use super::thumbnail::{self, ThumbnailResources};
pub use super::thumbnail::{PREVIEW_THUMB_SIZE, THUMB_SIZE, ThumbnailEntry};
pub(crate) use super::thumbnail::{READBACK_POOL_SIZE, ReadbackStatus, ThumbnailReadback};

pub use super::texture_loader::{linear_to_srgb, load_ktx2_as_rgba_bytes};
// Ground/decal texture-array decode is native-only: the web build runs at
// Classic quality, which textures from the tile atlas rather than these arrays.
#[cfg(not(target_arch = "wasm32"))]
pub use super::texture_loader::{
    load_decal_normal_specular_data, load_decal_texture_data_from_wz,
    load_ground_normal_specular_data, load_ground_texture_data,
};
// KTX2 path decoding (basis-universal FFI) is native-only; the web build
// uploads PNG ground textures via the in-memory byte path instead.
#[cfg(not(target_arch = "wasm32"))]
pub use super::texture_loader::load_ktx2_as_rgba;

/// Holds all wgpu rendering state for the 3D viewport.
pub struct EditorRenderer {
    pub pipelines: PipelineSet,
    pub uniforms: UniformState,
    pub atlas: AtlasState,
    pub terrain_gpu: Option<TerrainGpuData>,
    pub water_gpu: Option<WaterGpuData>,
    pub show_grid: bool,
    pub show_border: bool,
    pub show_heatmap: bool,
    pub show_viewshed: bool,
    pub settings: RenderSettings,
    /// Wall-clock start used for water animation, independent of frame timing.
    pub start_time: Instant,
    /// Map dimensions for shadow projection (width, height in tiles).
    pub map_dims: (u32, u32),
    pub shadow: ShadowResources,
    /// Cached shadow MVP to avoid recomputation when sun/map dims are unchanged.
    shadow_mvp_cache: CachedShadowMvp,
    /// First-load gate. While false, force a shadow pass even if dirty is
    /// clear so the cached depth gets populated.
    shadow_initialized: bool,
    pub models: ModelResources,
    /// Shared sampler bound through `@group(0)`. Owned here so the
    /// lightmap-rebind path can re-include it when recreating the per-frame
    /// bind group.
    pub model_sampler: wgpu::Sampler,
    pub ground: GroundTextureState,
    pub lightmap: LightmapState,
    pub water: WaterState,
    pub particles: ParticleState,
    pub heatmap: HeatmapState,
    pub viewshed: ViewshedState,
    thumb: ThumbnailResources,
    /// Larger offscreen target used by the droid designer's live preview.
    preview_thumb: ThumbnailResources,
}

impl std::fmt::Debug for EditorRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorRenderer")
            .field("has_terrain", &self.terrain_gpu.is_some())
            .field("has_water", &self.water_gpu.is_some())
            .field("has_atlas", &self.atlas.has_atlas)
            .field("show_grid", &self.show_grid)
            .field("show_border", &self.show_border)
            .field("show_heatmap", &self.show_heatmap)
            .field("model_cache_count", &self.models.cache.len())
            .field("draw_call_count", &self.models.draw_calls.len())
            .finish_non_exhaustive()
    }
}

impl EditorRenderer {
    /// Create the renderer, building pipelines and allocating GPU resources.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let default_lightmap = make_default_lightmap(device, queue);
        // Model sampler lives in the per-frame group(0) so model bind groups
        // never reserve a sampler descriptor. DX12 caps the sampler heap at
        // 2048 entries.
        let model_sampler = repeat_linear_sampler(device, "model_sampler");
        let uniforms = UniformState::new(
            device,
            &default_lightmap.view,
            &default_lightmap.sampler,
            &model_sampler,
        );
        let atlas = AtlasState::new(device, queue);
        let default_model_atlas_view = model_pass::make_default_atlas_view(device, queue);

        let shadow = shadow_pass::build(device);
        let terrain = terrain_pass::build(
            device,
            terrain_pass::TerrainPassInputs {
                uniform_layout: &uniforms.bind_group_layout,
                atlas_layout: &atlas.bind_group_layout,
                shadow_layout: &shadow.bind_group_layout,
                target_format,
            },
        );
        let model_build = model_pass::build(
            device,
            &uniforms.bind_group_layout,
            &shadow.bind_group_layout,
            target_format,
        );
        let water_sky = water_sky::build(device, queue, &uniforms.bind_group_layout, target_format);
        let heatmap_viewshed =
            heatmap_viewshed::build(device, &uniforms.bind_group_layout, target_format);

        let particle_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/particles.wgsl").into()),
        });
        let particle_pipeline_layout = pipelines::create_pipeline_layout(
            device,
            "particle_pipeline_layout",
            &[&uniforms.bind_group_layout],
        );
        let particle_pipeline = pipeline_with_recipe(
            device,
            "particle_pipeline",
            &particle_pipeline_layout,
            &particle_shader,
            &[ParticleVertex::desc()],
            target_format,
            PipelineRecipe::OVERLAY_ALPHA,
        );

        let thumb = make_thumbnail(
            device,
            &uniforms.bind_group_layout,
            &default_lightmap.view,
            &default_lightmap.sampler,
            &model_sampler,
            THUMB_SIZE,
            false,
        );
        let preview_thumb = make_thumbnail(
            device,
            &uniforms.bind_group_layout,
            &default_lightmap.view,
            &default_lightmap.sampler,
            &model_sampler,
            PREVIEW_THUMB_SIZE,
            true,
        );

        let models = model_pass::make_model_resources(
            model_build.model_pipeline,
            model_build.texture_layout,
            default_model_atlas_view,
            model_build.thumb_pipeline,
        );

        Self {
            pipelines: PipelineSet {
                terrain: terrain.terrain,
                terrain_medium: terrain.terrain_medium,
                terrain_high: terrain.terrain_high,
                grid: terrain.grid,
                border: terrain.border,
                sky: water_sky.sky_pipeline,
                water: water_sky.water_pipeline,
                shadow_terrain: shadow.terrain_pipeline,
                shadow_model: shadow.model_pipeline,
            },
            uniforms,
            atlas,
            terrain_gpu: None,
            water_gpu: None,
            show_grid: false,
            show_border: false,
            show_heatmap: false,
            show_viewshed: false,
            settings: RenderSettings::default(),
            start_time: Instant::now(),
            map_dims: (64, 64),
            shadow: shadow.resources,
            shadow_mvp_cache: CachedShadowMvp::new(),
            shadow_initialized: false,
            models,
            model_sampler,
            ground: terrain.ground_state,
            lightmap: LightmapState {
                texture: default_lightmap.texture,
                view: default_lightmap.view,
                sampler: default_lightmap.sampler,
                has_lightmap: false,
            },
            water: water_sky.water_state,
            particles: ParticleState {
                pipeline: particle_pipeline,
                vertex_buffer: None,
                index_buffer: None,
                index_count: 0,
            },
            heatmap: heatmap_viewshed.heatmap,
            viewshed: heatmap_viewshed.viewshed,
            thumb,
            preview_thumb,
        }
    }

    /// Upload weather particle billboard vertices to the GPU.
    pub fn upload_particles(
        &mut self,
        device: &wgpu::Device,
        vertices: &[ParticleVertex],
        indices: &[u32],
    ) {
        self.particles.upload(device, vertices, indices);
    }

    /// Upload a computed lightmap to the GPU.
    pub fn upload_lightmap(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lightmap: &Lightmap,
    ) {
        upload_lightmap(
            device,
            queue,
            &mut self.lightmap,
            &mut self.uniforms,
            &self.model_sampler,
            lightmap,
        );
    }
}

/// Construct one of the offscreen thumbnail targets at the given pixel size.
fn make_thumbnail(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    lightmap_view: &wgpu::TextureView,
    lightmap_sampler: &wgpu::Sampler,
    model_sampler: &wgpu::Sampler,
    size: u32,
    sampleable: bool,
) -> ThumbnailResources {
    thumbnail::create_thumbnail_resources(
        device,
        uniform_layout,
        lightmap_view,
        lightmap_sampler,
        model_sampler,
        size,
        sampleable,
        UniformState::create_bind_group,
    )
}

/// Initial lightmap GPU resources.
struct DefaultLightmap {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

/// Create a 1x1 R8=255 default lightmap (full brightness, no AO darkening).
fn make_default_lightmap(device: &wgpu::Device, queue: &wgpu::Queue) -> DefaultLightmap {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("default_lightmap"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[255],
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("lightmap_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    DefaultLightmap {
        texture,
        view,
        sampler,
    }
}

/// Create a `Linear/Linear/Repeat` sampler with the given label.
fn repeat_linear_sampler(device: &wgpu::Device, label: &'static str) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

/// Upload a computed lightmap to the GPU.
///
/// Reuses the existing texture when dimensions match (common during
/// interactive editing); only recreates on map-size change.
fn upload_lightmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    lightmap_state: &mut LightmapState,
    uniforms_state: &mut UniformState,
    model_sampler: &wgpu::Sampler,
    lightmap: &Lightmap,
) {
    let existing_size = lightmap_state.texture.size();
    let dims_match =
        existing_size.width == lightmap.width && existing_size.height == lightmap.height;

    if dims_match && lightmap_state.has_lightmap {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &lightmap_state.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &lightmap.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                // R8Unorm: 1 byte per pixel, rows are `width` bytes.
                bytes_per_row: Some(lightmap.width),
                rows_per_image: Some(lightmap.height),
            },
            wgpu::Extent3d {
                width: lightmap.width,
                height: lightmap.height,
                depth_or_array_layers: 1,
            },
        );
    } else {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("terrain_lightmap"),
                size: wgpu::Extent3d {
                    width: lightmap.width,
                    height: lightmap.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &lightmap.data,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        uniforms_state.bind_group = UniformState::create_bind_group(
            device,
            &uniforms_state.bind_group_layout,
            &uniforms_state.buffer,
            &view,
            &lightmap_state.sampler,
            model_sampler,
        );

        lightmap_state.texture = texture;
        lightmap_state.view = view;
        log::info!(
            "Uploaded lightmap: {}x{} tiles (new texture)",
            lightmap.width,
            lightmap.height
        );
    }
    lightmap_state.has_lightmap = true;
}
