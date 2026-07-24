//! Tileset atlas GPU bind group state. CPU-side atlas building lives in
//! [`super::atlas`].

use eframe::wgpu::util::DeviceExt;

use super::texture_loader::downsample_2x;

/// Anisotropic filtering samples for terrain samplers.
///
/// 16 is the common hardware maximum; wgpu clamps it to the device limit, so
/// it is safe on native and WebGL2. Sharpens grazing-angle terrain that
/// trilinear filtering alone leaves blurry. Requires all filter modes Linear.
pub(crate) const MAX_ANISOTROPY: u16 = 16;

/// Tileset atlas GPU bind group and layout.
pub struct AtlasState {
    pub bind_group: wgpu::BindGroup,
    pub(crate) bind_group_layout: wgpu::BindGroupLayout,
    /// True once a real tileset atlas has been uploaded; false leaves the
    /// 1x1 white fallback.
    pub has_atlas: bool,
}

impl AtlasState {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let default_atlas = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("default_atlas"),
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
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255, 255, 255, 255],
        );
        let default_view = default_atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let default_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("default_sampler"),
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&default_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&default_sampler),
                },
            ],
        });

        Self {
            bind_group,
            bind_group_layout,
            has_atlas: false,
        }
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tile_rgba: &[u8],
        tile_size: u32,
        num_layers: u32,
    ) {
        // One layer per tile, each with its own mip chain: trilinear
        // minification stays within a layer, so distant tiles never bleed into
        // neighbours the way a packed atlas would. Reuses the ground-array
        // upload path (per-layer CPU mip generation).
        let view = upload_texture_array(
            device,
            queue,
            "tileset_atlas",
            tile_rgba,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            tile_size,
            num_layers,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: MAX_ANISOTROPY,
            ..Default::default()
        });

        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas_bind_group"),
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
            ],
        });

        self.has_atlas = true;
        log::info!("Uploaded tileset atlas: {num_layers} tiles x {tile_size}px (mipmapped array)");
    }
}

/// Upload a single texture array and return its view.
///
/// Used by the chunked ground-texture upload path: no bind group is
/// touched here so the caller can spread multiple arrays across frames.
pub fn upload_texture_array(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    data: &[u8],
    format: wgpu::TextureFormat,
    size: u32,
    layers: u32,
) -> wgpu::TextureView {
    let texture_layers = pad_d2_array_layers(layers);
    let layer_bytes = size as usize * size as usize * 4;
    let expected = layer_bytes * layers as usize;
    // Guard against an unexpectedly-sized (e.g. empty) source buffer: skip
    // mip generation rather than panic slicing per-layer, matching the prior
    // single bulk write.
    let mipmapped = data.len() == expected;
    let mip_level_count = if mipmapped { size.ilog2() + 1 } else { 1 };
    if !mipmapped {
        log::warn!(
            "Texture array {label}: buffer is {} bytes, expected {expected}; skipping mip generation",
            data.len(),
        );
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: texture_layers,
        },
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    if mipmapped {
        let srgb = matches!(format, wgpu::TextureFormat::Rgba8UnormSrgb);
        for layer in 0..layers {
            let start = layer as usize * layer_bytes;
            let layer_data = &data[start..start + layer_bytes];
            write_mipmapped_array_layer(queue, &texture, layer_data, size, layer, srgb);
        }
        // pad_d2_array_layers may append one layer to dodge the GL cube-map-array
        // path. Mirror the last real layer into it so a shader layer-index clamp
        // that lands on the pad samples a real tile, not zero-filled black.
        if texture_layers > layers {
            let start = (layers - 1) as usize * layer_bytes;
            let last = &data[start..start + layer_bytes];
            write_mipmapped_array_layer(queue, &texture, last, size, layers, srgb);
        }
    } else {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
        );
    }

    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

/// Write one mip level of a 2D-array layer via `queue.write_texture`.
///
/// `write_texture` imposes no `bytes_per_row` alignment (unlike
/// buffer-to-texture copies), so `4 * width` is valid at every level.
pub(crate) fn write_texture_level(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    data: &[u8],
    width: u32,
    height: u32,
    mip_level: u32,
    layer: u32,
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// Upload one array layer with a full CPU-generated mip chain.
///
/// `layer_data` is the layer's level-0 RGBA8 (`size * size * 4` bytes),
/// written directly, then each halved level down to 1x1. `srgb` selects
/// gamma-correct averaging for diffuse layers (see [`downsample_2x`]).
pub(crate) fn write_mipmapped_array_layer(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer_data: &[u8],
    size: u32,
    layer: u32,
    srgb: bool,
) {
    write_texture_level(queue, texture, layer_data, size, size, 0, layer);
    let mut cur = downsample_2x(layer_data, size, size, srgb);
    let mut level = 1u32;
    loop {
        write_texture_level(
            queue, texture, &cur.data, cur.width, cur.height, level, layer,
        );
        if cur.width <= 1 && cur.height <= 1 {
            break;
        }
        cur = downsample_2x(&cur.data, cur.width, cur.height, srgb);
        level += 1;
    }
}

/// Round a D2-array layer count up by one when it's a non-trivial
/// multiple of 6. wgpu's GL backend would otherwise create the texture
/// as `GL_TEXTURE_CUBE_MAP_ARRAY`, which a `sampler2DArray` reads as
/// zeros (black terrain).
pub(crate) fn pad_d2_array_layers(layers: u32) -> u32 {
    if layers > 1 && layers.is_multiple_of(6) {
        layers + 1
    } else {
        layers
    }
}
