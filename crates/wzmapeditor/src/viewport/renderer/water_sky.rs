//! Water and sky pipelines plus the default water bind group.

use eframe::wgpu::util::DeviceExt;

use super::super::pipelines;
use super::super::render_types::WaterState;
use super::super::water::WaterVertex;
use super::util::{
    BindGroupLayoutBuilder, PipelineRecipe, biased_overlay_pipeline, pipeline_with_recipe,
};

/// Outputs from [`build`].
pub(super) struct WaterSkyBuild {
    pub water_state: WaterState,
    pub water_pipeline: wgpu::RenderPipeline,
    pub sky_pipeline: wgpu::RenderPipeline,
}

/// Build the water bind group layout, default water bind group, and the
/// water + sky render pipelines.
pub(super) fn build(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    uniform_layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> WaterSkyBuild {
    let water_bind_group_layout = BindGroupLayoutBuilder::new("water_bind_group_layout")
        .texture_2d_filterable(0, wgpu::ShaderStages::FRAGMENT)
        .texture_2d_filterable(1, wgpu::ShaderStages::FRAGMENT)
        .sampler_filtering(2, wgpu::ShaderStages::FRAGMENT)
        .build(device);

    // 1x1 fallback. Shader checks dimensions > 1 to detect real textures.
    let default_water_tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("default_water_tex"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::default(),
        &[128, 128, 128, 255],
    );
    let default_water_view = default_water_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let water_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("water_sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let water_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("water_bind_group"),
        layout: &water_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&default_water_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&default_water_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&water_sampler),
            },
        ],
    });

    let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sky_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/sky.wgsl").into()),
    });
    let water_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("water_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/water.wgsl").into()),
    });

    let sky_layout =
        pipelines::create_pipeline_layout(device, "sky_pipeline_layout", &[uniform_layout]);
    let water_pipeline_layout = pipelines::create_pipeline_layout(
        device,
        "water_pipeline_layout",
        &[uniform_layout, &water_bind_group_layout],
    );

    let sky_pipeline = pipeline_with_recipe(
        device,
        "sky_pipeline",
        &sky_layout,
        &sky_shader,
        &[],
        target_format,
        PipelineRecipe::SKY,
    );
    // Pull the water plane toward the camera so it wins the depth test
    // against the lowered basin terrain. A flat plane has ~zero depth slope,
    // so a slope-scaled bias is a near-no-op on a correct backend, but it is
    // implementation-defined: some Win11/Vulkan drivers compute it so water
    // loses the depth test and the basin shows through ("water not loading").
    // A constant-only bias is portable and keeps water visible everywhere.
    let water_bias = wgpu::DepthBiasState {
        constant: -4,
        slope_scale: 0.0,
        clamp: 0.0,
    };
    let water_pipeline = biased_overlay_pipeline(
        device,
        "water_pipeline",
        &water_pipeline_layout,
        &water_shader,
        &[WaterVertex::desc()],
        target_format,
        water_bias,
    );

    WaterSkyBuild {
        water_state: WaterState {
            bind_group_layout: water_bind_group_layout,
            bind_group: water_bind_group,
            load_attempted: false,
        },
        water_pipeline,
        sky_pipeline,
    }
}

/// Build a `WaterGpuData` from a map and terrain-types table.
///
/// Returns `None` when there are no water tiles so callers can clear the
/// `water_gpu` state.
pub(super) fn build_water_gpu(
    device: &wgpu::Device,
    map: &wz_maplib::MapData,
    terrain_types: &wz_maplib::terrain_types::TerrainTypeData,
) -> Option<super::super::render_types::WaterGpuData> {
    let mesh = super::super::water::WaterMesh::from_map(map, terrain_types);
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        log::debug!("No water tiles found on this map");
        return None;
    }
    log::debug!(
        "Uploading water mesh: {} vertices, {} indices",
        mesh.vertices.len(),
        mesh.indices.len()
    );

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("water_vertex_buffer"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("water_index_buffer"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    Some(super::super::render_types::WaterGpuData {
        vertex_buffer,
        index_buffer,
        index_count: mesh.indices.len() as u32,
    })
}

impl super::EditorRenderer {
    /// Upload water mesh data to the GPU.
    pub fn upload_water(
        &mut self,
        device: &wgpu::Device,
        map: &wz_maplib::MapData,
        terrain_types: &wz_maplib::terrain_types::TerrainTypeData,
    ) {
        self.water_gpu = build_water_gpu(device, map, terrain_types);
    }

    /// Load water textures from the texpages directory and upload to GPU.
    pub fn load_and_upload_water_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &dyn crate::assets::AssetSource,
        texpages_rel: &std::path::Path,
    ) {
        self.water
            .load_and_upload_textures(device, queue, assets, texpages_rel);
    }
}
