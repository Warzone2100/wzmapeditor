//! Terrain pipelines (Classic, Medium, High) plus the grid and border overlays.

use eframe::wgpu::util::DeviceExt;
use wz_maplib::MapData;
use wz_maplib::terrain_types::TerrainTypeData;

use super::super::ground_types::GroundData;
use super::super::pipelines;
use super::super::render_types::{GroundTextureState, TerrainGpuData};
use super::super::terrain::{TerrainMesh, TerrainVertex};
use super::util::{
    BindGroupLayoutBuilder, OVERLAY_DEPTH_BIAS, PipelineRecipe, biased_overlay_pipeline,
    pipeline_with_recipe,
};

/// Pipelines and bind group layouts produced by [`build`].
pub(super) struct TerrainPassBuild {
    pub ground_state: GroundTextureState,
    pub terrain: wgpu::RenderPipeline,
    pub terrain_medium: wgpu::RenderPipeline,
    pub terrain_high: wgpu::RenderPipeline,
    pub grid: wgpu::RenderPipeline,
    pub border: wgpu::RenderPipeline,
}

/// Inputs needed to build the terrain pipelines.
#[derive(Clone, Copy)]
pub(super) struct TerrainPassInputs<'a> {
    pub uniform_layout: &'a wgpu::BindGroupLayout,
    pub atlas_layout: &'a wgpu::BindGroupLayout,
    pub shadow_layout: &'a wgpu::BindGroupLayout,
    pub target_format: wgpu::TextureFormat,
}

/// Build the bind group layouts and pipelines for terrain rendering.
pub(super) fn build(device: &wgpu::Device, inputs: TerrainPassInputs<'_>) -> TerrainPassBuild {
    let TerrainPassInputs {
        uniform_layout,
        atlas_layout,
        shadow_layout,
        target_format,
    } = inputs;

    let ground_layout = BindGroupLayoutBuilder::new("ground_bind_group_layout")
        .texture_2d_array_filterable(0, wgpu::ShaderStages::FRAGMENT)
        .sampler_filtering(1, wgpu::ShaderStages::FRAGMENT)
        .uniform_buffer(2, wgpu::ShaderStages::FRAGMENT)
        .build(device);

    let ground_high_layout = BindGroupLayoutBuilder::new("ground_high_bind_group_layout")
        .texture_2d_array_filterable(0, wgpu::ShaderStages::FRAGMENT)
        .sampler_filtering(1, wgpu::ShaderStages::FRAGMENT)
        .uniform_buffer(2, wgpu::ShaderStages::FRAGMENT)
        .texture_2d_array_filterable(3, wgpu::ShaderStages::FRAGMENT)
        .texture_2d_array_filterable(4, wgpu::ShaderStages::FRAGMENT)
        .texture_2d_array_filterable(5, wgpu::ShaderStages::FRAGMENT)
        .texture_2d_array_filterable(6, wgpu::ShaderStages::FRAGMENT)
        .texture_2d_array_filterable(7, wgpu::ShaderStages::FRAGMENT)
        .sampler_filtering(8, wgpu::ShaderStages::FRAGMENT)
        .build(device);

    let terrain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terrain_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/terrain.wgsl").into()),
    });
    let terrain_medium_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terrain_medium_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/terrain_medium.wgsl").into()),
    });
    let terrain_high_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terrain_high_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/terrain_high.wgsl").into()),
    });
    let grid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("grid_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/grid.wgsl").into()),
    });
    let border_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("border_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/border.wgsl").into()),
    });

    let terrain_layout = pipelines::create_pipeline_layout(
        device,
        "terrain_pipeline_layout",
        &[uniform_layout, atlas_layout, shadow_layout],
    );
    let terrain_medium_layout = pipelines::create_pipeline_layout(
        device,
        "terrain_medium_pipeline_layout",
        &[uniform_layout, atlas_layout, shadow_layout, &ground_layout],
    );
    let terrain_high_layout = pipelines::create_pipeline_layout(
        device,
        "terrain_high_pipeline_layout",
        &[
            uniform_layout,
            atlas_layout,
            shadow_layout,
            &ground_high_layout,
        ],
    );
    let overlay_layout =
        pipelines::create_pipeline_layout(device, "grid_pipeline_layout", &[uniform_layout]);

    let terrain_vb = &[TerrainVertex::desc()];

    let terrain = pipeline_with_recipe(
        device,
        "terrain_pipeline",
        &terrain_layout,
        &terrain_shader,
        terrain_vb,
        target_format,
        PipelineRecipe::TERRAIN_OPAQUE,
    );
    let terrain_medium = pipeline_with_recipe(
        device,
        "terrain_medium_pipeline",
        &terrain_medium_layout,
        &terrain_medium_shader,
        terrain_vb,
        target_format,
        PipelineRecipe::TERRAIN_OPAQUE,
    );
    let terrain_high = pipeline_with_recipe(
        device,
        "terrain_high_pipeline",
        &terrain_high_layout,
        &terrain_high_shader,
        terrain_vb,
        target_format,
        PipelineRecipe::TERRAIN_OPAQUE,
    );
    let grid = pipeline_with_recipe(
        device,
        "grid_pipeline",
        &overlay_layout,
        &grid_shader,
        terrain_vb,
        target_format,
        PipelineRecipe::OVERLAY_ALPHA,
    );
    let border = biased_overlay_pipeline(
        device,
        "border_pipeline",
        &overlay_layout,
        &border_shader,
        terrain_vb,
        target_format,
        OVERLAY_DEPTH_BIAS,
    );

    TerrainPassBuild {
        ground_state: GroundTextureState {
            bind_group_layout: ground_layout,
            bind_group: None,
            has_textures: false,
            high_bind_group_layout: ground_high_layout,
            high_bind_group: None,
        },
        terrain,
        terrain_medium,
        terrain_high,
        grid,
        border,
    }
}

/// Build a fresh `TerrainGpuData` from a map plus optional ground data.
///
/// Returns `None` when the mesh is empty so callers can clear `terrain_gpu`.
pub(super) fn build_terrain_gpu(
    device: &wgpu::Device,
    map: &MapData,
    ground_data: Option<&GroundData>,
    terrain_types: Option<&TerrainTypeData>,
) -> Option<TerrainGpuData> {
    let mesh = TerrainMesh::from_map(map, ground_data, terrain_types);
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        log::warn!("upload_terrain_with_ground: empty mesh");
        return None;
    }

    // COPY_DST so partial brush updates can write into the existing buffer
    // via queue.write_buffer instead of reallocating per tile crossing.
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("terrain_vertex_buffer"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("terrain_index_buffer"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    // Cache per-vertex water depth so partial brush updates can skip the
    // global blurred-depth pass. Ground votes are recomputed locally on
    // each partial update so a brush that changed a tile texture doesn't
    // read a stale vote.
    let vw = (map.width + 1) as usize;
    let vh = (map.height + 1) as usize;
    let water_depth = if let Some(ttp) = terrain_types {
        super::super::water::build_water_vertex_depths(map, ttp)
    } else {
        vec![0.0; vw * vh]
    };

    Some(TerrainGpuData {
        vertex_buffer,
        index_buffer,
        index_count: mesh.indices.len() as u32,
        water_depth,
    })
}

/// Re-upload only the vertices for tiles in the given inclusive rect.
///
/// Reuses the cached water-depth grid from the last full upload, so shore
/// slopes near the brush edge may be momentarily stale until the next full
/// rebuild. Per-vertex ground votes are recomputed locally against the
/// current map so a brush that just changed a tile's texture sees the new
/// vote, not the cached one.
pub(super) fn update_tile_rect(
    queue: &wgpu::Queue,
    gpu: &TerrainGpuData,
    map: &MapData,
    ground_data: Option<&GroundData>,
    min_tx: u32,
    min_ty: u32,
    max_tx: u32,
    max_ty: u32,
) {
    // Expand by one tile in each direction. Adjacent tiles store their own
    // copies of the shared corner vertices; without re-emitting them, stale
    // Y/normal/ground-vote create visible seams at the brush edge.
    let map_w = map.width;
    let map_h = map.height;
    let min_tx = min_tx.saturating_sub(1);
    let min_ty = min_ty.saturating_sub(1);
    let max_tx = (max_tx + 1).min(map_w.saturating_sub(1));
    let max_ty = (max_ty + 1).min(map_h.saturating_sub(1));
    if min_tx > max_tx || min_ty > max_ty {
        return;
    }

    let rect_verts = TerrainMesh::build_tile_rect_vertices(
        map,
        ground_data,
        &gpu.water_depth,
        min_tx,
        min_ty,
        max_tx,
        max_ty,
    );

    // Buffer is row-major over tiles, 4 vertices per tile. Each row of the
    // dirty rect is contiguous; rows themselves are not, so write one
    // queue.write_buffer per row.
    let w = map.width;
    let row_tiles = (max_tx - min_tx + 1) as usize;
    let row_verts = row_tiles * 4;
    let vertex_size = size_of::<TerrainVertex>() as u64;

    for ty in min_ty..=max_ty {
        let row_idx = (ty - min_ty) as usize;
        let src_start = row_idx * row_verts;
        let src_end = src_start + row_verts;
        let dst_offset = ((ty * w + min_tx) as u64) * 4 * vertex_size;
        queue.write_buffer(
            &gpu.vertex_buffer,
            dst_offset,
            bytemuck::cast_slice(&rect_verts[src_start..src_end]),
        );
    }
}

impl super::EditorRenderer {
    /// Upload terrain mesh with ground splatting data for Medium/High quality.
    pub fn upload_terrain_with_ground(
        &mut self,
        device: &wgpu::Device,
        map: &MapData,
        ground_data: Option<&GroundData>,
        terrain_types: Option<&TerrainTypeData>,
    ) {
        self.terrain_gpu = build_terrain_gpu(device, map, ground_data, terrain_types);
        if self.terrain_gpu.is_some() {
            self.map_dims = (map.width, map.height);
        }
    }

    /// Re-upload only the vertices for tiles in the given inclusive rect.
    pub fn update_terrain_tile_rect(
        &mut self,
        queue: &wgpu::Queue,
        map: &MapData,
        ground_data: Option<&GroundData>,
        min_tx: u32,
        min_ty: u32,
        max_tx: u32,
        max_ty: u32,
    ) {
        let Some(gpu) = self.terrain_gpu.as_ref() else {
            return;
        };
        if map.width != self.map_dims.0 || map.height != self.map_dims.1 {
            return;
        }
        update_tile_rect(queue, gpu, map, ground_data, min_tx, min_ty, max_tx, max_ty);
    }

    /// Upload the tileset tile array (one 256px layer per tile) to the GPU.
    pub fn upload_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tile_rgba: &[u8],
        tile_size: u32,
        num_layers: u32,
    ) {
        self.atlas
            .upload(device, queue, tile_rgba, tile_size, num_layers);
    }

    /// Upload ground type textures as a 2D texture array for Medium quality.
    pub fn upload_ground_texture_data(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
        num_layers: u32,
        ground_scales: &[f32; 16],
    ) {
        self.ground
            .upload_texture_data(device, queue, data, num_layers, ground_scales);
    }

    /// Assemble the High-quality terrain bind group from pre-uploaded views.
    #[expect(
        clippy::too_many_arguments,
        reason = "ground_high binds eight separate texture views by design"
    )]
    pub fn create_ground_high_bind_group(
        &mut self,
        device: &wgpu::Device,
        ground_scales: &[f32; 16],
        diffuse_view: &wgpu::TextureView,
        normal_view: &wgpu::TextureView,
        specular_view: &wgpu::TextureView,
        decal_diffuse_view: &wgpu::TextureView,
        decal_normal_view: &wgpu::TextureView,
        decal_specular_view: &wgpu::TextureView,
    ) {
        self.ground.create_high_bind_group(
            device,
            ground_scales,
            diffuse_view,
            normal_view,
            specular_view,
            decal_diffuse_view,
            decal_normal_view,
            decal_specular_view,
        );
    }
}
