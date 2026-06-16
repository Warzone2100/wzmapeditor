//! Offscreen thumbnail rendering resources and GPU readback.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use super::uniforms::Uniforms;

/// 256 keeps grid thumbnails crisp after egui upscales for high-DPI displays.
pub const THUMB_SIZE: u32 = 256;

/// Designer live-preview size. 512 stays crisp under DPI scaling without
/// blowing up preload memory; the asset browser still uses [`THUMB_SIZE`].
pub const PREVIEW_THUMB_SIZE: u32 = 512;

/// `egui_wgpu::Renderer::register_native_texture` requires `Rgba8UnormSrgb`,
/// so the designer can sample the preview target directly without a
/// GPU-to-CPU-to-GPU round-trip.
pub const THUMB_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// A model to include in a GPU thumbnail render.
pub struct ThumbnailEntry<'a> {
    /// Key in the model cache (`imd_name`).
    pub model_key: &'a str,
    /// Local-space offset (e.g. connector position for weapons).
    pub offset: glam::Vec3,
    /// Team color (RGBA, alpha controls tint strength).
    pub team_color: [f32; 4],
}

/// Number of staging buffers in the readback pool. The single shared color
/// target is rendered then copied per kickoff, all in submit order, so one
/// staging buffer per in-flight readback is all the parallelism needs; this
/// lets the preload loop dispatch several thumbnails per frame instead of
/// serializing on one buffer. The readback-free preview target uses one slot.
pub(crate) const READBACK_POOL_SIZE: usize = 4;

const SLOT_FREE: u8 = 0;
const SLOT_IN_FLIGHT: u8 = 1;
const SLOT_MAPPED: u8 = 2;
const SLOT_FAILED: u8 = 3;

/// One pooled staging buffer and the state shared with its `map_async`
/// callback. A slot is `FREE` until claimed, `IN_FLIGHT` until the copy maps,
/// then `MAPPED` (decode + unmap) or `FAILED` (discard); either resolution
/// returns it to `FREE`.
struct StagingSlot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

/// Offscreen resources for GPU-based model thumbnail rendering.
pub(crate) struct ThumbnailResources {
    pub color_texture: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    /// Owns the GPU allocation; sampled via `depth_view`.
    #[expect(dead_code, reason = "must be kept alive to back the depth_view")]
    depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    staging: Vec<StagingSlot>,
    pub uniform_buffer: wgpu::Buffer,
    pub uniform_bind_group: wgpu::BindGroup,
}

impl ThumbnailResources {
    /// Atomically claim a free staging slot, or `None` if the pool is full.
    pub(crate) fn claim_slot(&self) -> Option<usize> {
        self.staging.iter().position(|slot| {
            slot.state
                .compare_exchange(
                    SLOT_FREE,
                    SLOT_IN_FLIGHT,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        })
    }

    /// Release a claimed slot whose render was abandoned before submission.
    pub(crate) fn free_slot(&self, slot: usize) {
        self.staging[slot].state.store(SLOT_FREE, Ordering::Release);
    }
}

/// `sampleable` adds `TEXTURE_BINDING` for targets that egui samples
/// directly (e.g. the designer preview); the asset-browser path reads
/// back via `COPY_SRC`.
pub(crate) fn create_thumbnail_resources(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    lightmap_view: &wgpu::TextureView,
    lightmap_sampler: &wgpu::Sampler,
    model_sampler: &wgpu::Sampler,
    size: u32,
    sampleable: bool,
    create_uniform_bind_group: impl FnOnce(
        &wgpu::Device,
        &wgpu::BindGroupLayout,
        &wgpu::Buffer,
        &wgpu::TextureView,
        &wgpu::Sampler,
        &wgpu::Sampler,
    ) -> wgpu::BindGroup,
) -> ThumbnailResources {
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
    if sampleable {
        usage |= wgpu::TextureUsages::TEXTURE_BINDING;
    }
    let color_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("thumb_color"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: THUMB_FORMAT,
        usage,
        view_formats: &[],
    });
    let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("thumb_depth"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

    // wgpu's COPY_BYTES_PER_ROW_ALIGNMENT is 256. THUMB_SIZE=256 (the
    // minimum here) gives 1024 bytes/row, already aligned, so no padding.
    let bytes_per_row = size * 4;
    // The preview target samples its color directly and never reads back, so it
    // needs only one (unused) slot; the asset-browser target gets the full pool.
    let slots = if sampleable { 1 } else { READBACK_POOL_SIZE };
    let staging: Vec<StagingSlot> = (0..slots)
        .map(|_| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("thumb_staging"),
                size: (bytes_per_row * size) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            StagingSlot {
                buffer,
                state: Arc::new(AtomicU8::new(SLOT_FREE)),
            }
        })
        .collect();

    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("thumb_uniform"),
        size: size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let uniform_bind_group = create_uniform_bind_group(
        device,
        uniform_layout,
        &uniform_buffer,
        lightmap_view,
        lightmap_sampler,
        model_sampler,
    );

    ThumbnailResources {
        color_texture,
        color_view,
        depth_texture,
        depth_view,
        staging,
        uniform_buffer,
        uniform_bind_group,
    }
}

/// Completion state of a [`ThumbnailReadback`], polled each frame.
pub(crate) enum ReadbackStatus {
    /// The GPU copy and buffer mapping have not finished yet.
    Pending,
    /// The staging buffer is mapped; decode it via [`finish_read_back`].
    Ready,
    /// Mapping failed; the buffer was never mapped, so it must not be decoded.
    Failed,
}

/// Handle to an in-flight thumbnail GPU-to-CPU readback.
///
/// Each readback owns one pooled staging slot for its lifetime. `state` is the
/// same atomic the slot holds, so [`release`](Self::release) frees the slot
/// directly without touching GPU resources.
pub(crate) struct ThumbnailReadback {
    slot: usize,
    state: Arc<AtomicU8>,
}

impl ThumbnailReadback {
    /// Non-blocking check of the readback's completion state.
    pub(crate) fn status(&self) -> ReadbackStatus {
        match self.state.load(Ordering::Acquire) {
            SLOT_MAPPED => ReadbackStatus::Ready,
            SLOT_FAILED => ReadbackStatus::Failed,
            _ => ReadbackStatus::Pending,
        }
    }

    /// The pooled staging slot this readback occupies.
    pub(crate) fn slot(&self) -> usize {
        self.slot
    }

    /// Return the slot to the pool after a failed map (the buffer was never
    /// mapped, so no unmap is needed).
    pub(crate) fn release(&self) {
        self.state.store(SLOT_FREE, Ordering::Release);
    }
}

/// Append a copy into staging slot `slot`, submit `encoder`, and issue a
/// non-blocking `map_async`. Poll the returned handle each frame and decode
/// with [`finish_read_back`] once it reports [`ReadbackStatus::Ready`].
///
/// `slot` must have been claimed via [`ThumbnailResources::claim_slot`]. The
/// `map_async` callback only flips a shared atomic, so this works on a
/// single-threaded wasm target where blocking on the buffer mapping would
/// deadlock the browser event loop.
pub(crate) fn begin_read_back(
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    target: &ThumbnailResources,
    slot: usize,
    size: u32,
) -> ThumbnailReadback {
    let bytes_per_row = size * 4;
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.color_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &target.staging[slot].buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(std::iter::once(encoder.finish()));

    let state = Arc::clone(&target.staging[slot].state);
    let cb_state = Arc::clone(&state);
    target.staging[slot]
        .buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let next = if result.is_ok() {
                SLOT_MAPPED
            } else {
                SLOT_FAILED
            };
            cb_state.store(next, Ordering::Release);
        });

    ThumbnailReadback { slot, state }
}

/// Decode mapped staging slot `slot` into an RGB [`egui::ColorImage`], unmap
/// it, and return the slot to the pool. Only call when the readback reported
/// [`ReadbackStatus::Ready`].
pub(crate) fn finish_read_back(
    target: &ThumbnailResources,
    slot: usize,
    size: u32,
) -> egui::ColorImage {
    let row_stride = (size * 4) as usize;
    let size_usize = size as usize;
    let buffer = &target.staging[slot].buffer;
    let data = buffer.slice(..).get_mapped_range();
    let mut pixels = Vec::with_capacity(size_usize * size_usize);
    for row in data.chunks_exact(row_stride).take(size_usize) {
        for pixel in row[..size_usize * 4].chunks_exact(4) {
            pixels.push(egui::Color32::from_rgb(pixel[0], pixel[1], pixel[2]));
        }
    }
    drop(data);
    buffer.unmap();
    target.staging[slot]
        .state
        .store(SLOT_FREE, Ordering::Release);
    egui::ColorImage::new([size_usize, size_usize], pixels)
}
