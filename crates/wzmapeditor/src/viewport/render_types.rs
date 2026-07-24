//! Standalone GPU resource types and rendering constants.

use eframe::wgpu::util::DeviceExt;

use super::particles::ParticleVertex;

/// Per-tileset fog colors from WZ2100's palette.txt.
pub const FOG_ARIZONA: [f32; 3] = [0.69, 0.56, 0.37]; // RGB(176,143,95)
pub const FOG_URBAN: [f32; 3] = [0.063, 0.063, 0.25]; // RGB(16,16,64)
pub const FOG_ROCKIES: [f32; 3] = [0.71, 0.88, 0.93]; // RGB(182,225,236)

/// Default fog distances in world units, matching WZ2100.
pub const FOG_START_DEFAULT: f32 = 4000.0;
pub const FOG_END_DEFAULT: f32 = 8000.0;

/// Terrain rendering quality mode.
///
/// Matches WZ2100's `TerrainShaderQuality` enum from `terrain_defs.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum TerrainQuality {
    /// Tile-based texturing from the classic tileset atlas.
    Classic,
    /// Ground type texture splatting with decal overlay.
    Medium,
    /// Normal-mapped splatting with specular highlights.
    #[default]
    High,
}

impl TerrainQuality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Medium => "Normal",
            Self::High => "Remastered (HQ)",
        }
    }

    pub const ALL: [Self; 3] = [Self::Classic, Self::Medium, Self::High];
}

/// User-facing rendering settings synced from the UI each frame.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct RenderSettings {
    pub fog_enabled: bool,
    pub fog_color: [f32; 3],
    pub fog_start: f32,
    pub fog_end: f32,
    pub shadows_enabled: bool,
    pub water_enabled: bool,
    pub sky_enabled: bool,
    pub sun_direction: [f32; 3],
    /// Terrain rendering quality mode.
    #[serde(default)]
    pub terrain_quality: TerrainQuality,
    /// Vertical field of view in degrees.
    #[serde(default = "default_fov_degrees")]
    pub fov_degrees: f32,
}

fn default_fov_degrees() -> f32 {
    45.0
}

/// Default sun direction matching WZ2100's engine default (225, -600, 450).
/// Y is negated (WZ2100 uses -Y = down) and the vector normalised so
/// `max(dot(N, L), 0)` Lambert shading works directly.
pub(crate) const SUN_DIRECTION: [f32; 4] = [0.286, 0.763, 0.572, 0.0];

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            fog_enabled: false,
            fog_color: FOG_ARIZONA,
            fog_start: FOG_START_DEFAULT,
            fog_end: FOG_END_DEFAULT,
            shadows_enabled: true,
            water_enabled: true,
            sky_enabled: true,
            sun_direction: [SUN_DIRECTION[0], SUN_DIRECTION[1], SUN_DIRECTION[2]],
            terrain_quality: TerrainQuality::default(),
            fov_degrees: default_fov_degrees(),
        }
    }
}

/// GPU resources for rendering terrain.
pub struct TerrainGpuData {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    /// Per-vertex water-lowering depths keyed `(vy * (w+1)) + vx`. Cached
    /// so brush-stroke partial updates skip the full blurred depth field
    /// rebuild.
    pub water_depth: Vec<f32>,
}

/// GPU buffers for the water surface mesh.
#[derive(Debug)]
pub struct WaterGpuData {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
}

/// Ground-type texture splatting state (Medium and High terrain quality).
pub struct GroundTextureState {
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Group 3 bind group for the Medium terrain pipeline.
    pub bind_group: Option<wgpu::BindGroup>,
    pub has_textures: bool,
    pub high_bind_group_layout: wgpu::BindGroupLayout,
    /// Group 3 bind group for the High terrain pipeline (diffuse + normal +
    /// specular arrays, plus decal variants).
    pub high_bind_group: Option<wgpu::BindGroup>,
}

/// Lightmap resources (sun illumination + AO baked into a texture).
pub struct LightmapState {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    /// True once a real lightmap is uploaded; false leaves the 1x1 white fallback.
    pub has_lightmap: bool,
}

/// Water texture resources (page-80, page-81 bind group).
pub struct WaterState {
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Group 1 bind group in the water pipeline.
    pub bind_group: wgpu::BindGroup,
    /// Water PNGs are shared across tilesets and never change at runtime, so
    /// retrying on every terrain edit just floods the log with duplicate
    /// "file not found" warnings.
    pub load_attempted: bool,
}

/// Weather particle rendering resources.
pub struct ParticleState {
    pub pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: Option<wgpu::Buffer>,
    pub index_buffer: Option<wgpu::Buffer>,
    pub index_count: u32,
}

/// Propulsion speed heatmap overlay resources.
pub struct HeatmapState {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: Option<wgpu::BindGroup>,
    /// Terrain type lookup: `lut[texture_id] = terrain_type_id (0-11)`.
    pub lut_buffer: wgpu::Buffer,
    /// Speed factors for the selected propulsion class (12 floats packed as 3 vec4s).
    pub speed_buffer: wgpu::Buffer,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RingVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

impl RingVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Float32x4,
        ];
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRS,
        }
    }
}

/// Range-ring overlay resources for the viewshed feature.
#[expect(
    clippy::struct_field_names,
    reason = "ring_* fields name the GPU resources for one logical overlay"
)]
pub struct ViewshedState {
    pub ring_pipeline: wgpu::RenderPipeline,
    pub ring_vertex_buffer: Option<wgpu::Buffer>,
    pub ring_vertex_count: u32,
}

impl WaterState {
    /// Upload water textures (page-80-water-1.png, page-81-water-2.png).
    ///
    /// The shader checks `textureDimensions > 1` and falls back to
    /// procedural noise when the default 1x1 is bound.
    pub fn upload_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tex1_rgba: &[u8],
        tex1_width: u32,
        tex1_height: u32,
        tex2_rgba: &[u8],
        tex2_width: u32,
        tex2_height: u32,
    ) {
        let create_tex =
            |label: &str, rgba: &[u8], w: u32, h: u32| -> (wgpu::Texture, wgpu::TextureView) {
                let size = wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    rgba,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * w),
                        rows_per_image: Some(h),
                    },
                    size,
                );
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (texture, view)
            };

        let (_t1, view1) = create_tex("water_tex1", tex1_rgba, tex1_width, tex1_height);
        let (_t2, view2) = create_tex("water_tex2", tex2_rgba, tex2_width, tex2_height);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("water_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view1),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view2),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        log::info!(
            "Uploaded water textures: {tex1_width}x{tex1_height} + {tex2_width}x{tex2_height}"
        );
    }

    /// Load `page-80-water-1.png` and `page-81-water-2.png` from
    /// `texpages_dir` and upload to the GPU. Missing files leave the
    /// shader on its 1x1 procedural-noise fallback.
    pub fn load_and_upload_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &dyn crate::assets::AssetSource,
        texpages_rel: &std::path::Path,
    ) {
        if self.load_attempted {
            return;
        }
        self.load_attempted = true;
        let load_png = |name: &str| -> Option<(Vec<u8>, u32, u32)> {
            let rel = texpages_rel.join(name);
            let Some(img) = assets
                .bytes(&rel)
                .and_then(|b| image::load_from_memory(&b).ok())
            else {
                log::warn!("Failed to load water texture {}", rel.display());
                return None;
            };
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            Some((rgba.into_raw(), w, h))
        };

        let tex1 = load_png("page-80-water-1.png");
        let tex2 = load_png("page-81-water-2.png");

        if let (Some((d1, w1, h1)), Some((d2, w2, h2))) = (tex1, tex2) {
            self.upload_textures(device, queue, &d1, w1, h1, &d2, w2, h2);
        } else {
            log::info!("Water textures not found; using procedural noise fallback");
        }
    }
}

impl ParticleState {
    pub fn upload(&mut self, device: &wgpu::Device, vertices: &[ParticleVertex], indices: &[u32]) {
        if vertices.is_empty() || indices.is_empty() {
            self.vertex_buffer = None;
            self.index_buffer = None;
            self.index_count = 0;
            return;
        }

        self.vertex_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("particle_vertex_buffer"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );

        self.index_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("particle_index_buffer"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
        );

        self.index_count = indices.len() as u32;
    }
}

impl GroundTextureState {
    /// Upload ground type textures as a 2D texture array for Medium quality.
    ///
    /// `data` is a flat buffer of `num_layers` x 1024x1024 RGBA images.
    pub fn upload_texture_data(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
        num_layers: u32,
        ground_scales: &[f32; 16],
    ) {
        let tex_size = 1024u32;
        let view = super::atlas_gpu::upload_texture_array(
            device,
            queue,
            "ground_texture_array",
            data,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            tex_size,
            num_layers,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ground_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: super::atlas_gpu::MAX_ANISOTROPY,
            ..Default::default()
        });

        let scales_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ground_scales"),
            contents: bytemuck::cast_slice(ground_scales),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ground_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: scales_buffer.as_entire_binding(),
                },
            ],
        }));

        self.has_textures = true;
        log::info!(
            "Uploaded ground texture array: {num_layers} layers, {tex_size}x{tex_size} per layer"
        );
    }

    /// Assemble the High-quality terrain bind group from pre-uploaded views.
    /// Final step of the chunked upload sequence, after all six texture
    /// arrays (ground + decal each as diffuse / normal / specular) are
    /// uploaded on prior frames.
    pub fn create_high_bind_group(
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
        let ground_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ground_high_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: super::atlas_gpu::MAX_ANISOTROPY,
            ..Default::default()
        });

        // Decals are ClampToEdge and non-tiling, so they take trilinear
        // filtering but not anisotropy (which only helps repeated tiling).
        let decal_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("decal_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let scales_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ground_high_scales"),
            contents: bytemuck::cast_slice(ground_scales),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        self.high_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ground_high_bind_group"),
            layout: &self.high_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(diffuse_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&ground_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: scales_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(specular_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(decal_diffuse_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(decal_normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(decal_specular_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::Sampler(&decal_sampler),
                },
            ],
        }));

        log::info!("Created High quality terrain bind group");
    }
}

impl ViewshedState {
    pub fn upload(&mut self, device: &wgpu::Device, frame: &crate::viewshed::ViewshedFrame) {
        let ring_verts = build_ring_vertices(frame);
        if ring_verts.is_empty() {
            self.ring_vertex_buffer = None;
            self.ring_vertex_count = 0;
        } else {
            self.ring_vertex_buffer = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("viewshed_ring_vertex_buffer"),
                    contents: bytemuck::cast_slice(&ring_verts),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
            self.ring_vertex_count = ring_verts.len() as u32;
        }
    }
}

/// Each ring contributes 2 * `RING_SEGMENTS` vertices, paired endpoints
/// for `LineList` topology.
fn build_ring_vertices(frame: &crate::viewshed::ViewshedFrame) -> Vec<RingVertex> {
    const RING_SEGMENTS: u32 = 96;
    const RING_LIFT: f32 = 4.0;
    let mut out = Vec::new();
    for ring in &frame.rings {
        push_circle(
            &mut out,
            ring.center,
            ring.max_range,
            RING_LIFT,
            RING_SEGMENTS,
            [1.0, 1.0, 1.0, 0.85],
        );
    }
    out
}

fn push_circle(
    out: &mut Vec<RingVertex>,
    center: glam::Vec3,
    radius: f32,
    lift: f32,
    segments: u32,
    color: [f32; 4],
) {
    // Flat ring at source eye height. Per-vertex ground sampling hides
    // the ring on ridges; a flat outline reads more clearly as a
    // max-range disc.
    let y = center.y + lift;
    for i in 0..segments {
        let t0 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let t1 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
        let p0 = [
            center.x + radius * t0.cos(),
            y,
            center.z + radius * t0.sin(),
        ];
        let p1 = [
            center.x + radius * t1.cos(),
            y,
            center.z + radius * t1.sin(),
        ];
        out.push(RingVertex {
            position: p0,
            color,
        });
        out.push(RingVertex {
            position: p1,
            color,
        });
    }
}

impl HeatmapState {
    /// Upload terrain type LUT and speed factors for the selected propulsion,
    /// then (re)create the bind group.
    pub fn upload_data(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain_type_lut: &[u32; 512],
        speed_factors: &[f32; 12],
    ) {
        queue.write_buffer(&self.lut_buffer, 0, bytemuck::cast_slice(terrain_type_lut));
        queue.write_buffer(&self.speed_buffer, 0, bytemuck::cast_slice(speed_factors));
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("heatmap_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.lut_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.speed_buffer.as_entire_binding(),
                },
            ],
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_quality_all_contains_three_variants() {
        assert_eq!(TerrainQuality::ALL.len(), 3);
        assert_eq!(TerrainQuality::ALL[0], TerrainQuality::Classic);
        assert_eq!(TerrainQuality::ALL[1], TerrainQuality::Medium);
        assert_eq!(TerrainQuality::ALL[2], TerrainQuality::High);
    }

    #[test]
    fn terrain_quality_labels() {
        assert_eq!(TerrainQuality::Classic.label(), "Classic");
        assert_eq!(TerrainQuality::Medium.label(), "Normal");
        assert_eq!(TerrainQuality::High.label(), "Remastered (HQ)");
    }
}
