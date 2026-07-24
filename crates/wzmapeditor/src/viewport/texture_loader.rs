//! Texture loading utilities for ground types, decals, and KTX2 decoding.
//!
//! All public functions in this module are safe to call from background threads.
//! They perform file I/O and image decoding but hold no GPU state.

#[cfg(not(target_arch = "wasm32"))]
use super::ground_types::GroundTexture;

/// Decal texture resolution (per-tile). Native 256px from high.wz KTX2.
#[cfg(not(target_arch = "wasm32"))]
pub const DECAL_TEX_SIZE: u32 = 256;

/// Alpha channel threshold for classifying a tile as a decal (has transparency).
///
/// 250 (not 255) tolerates near-opaque pixels (alpha 251-254) caused by
/// compression and antialiasing in the source PNGs. Those should not count
/// as genuine transparency.
#[cfg(not(target_arch = "wasm32"))]
const DECAL_ALPHA_THRESHOLD: u8 = 250;

/// Load all ground type textures into a flat RGBA buffer (background-thread safe).
///
/// Returns a flat buffer of `num_layers` x 1024x1024x4 bytes, ready for GPU upload
/// via [`super::renderer::EditorRenderer::upload_ground_texture_data`]. Textures are
/// loaded in parallel using scoped threads for maximum I/O throughput.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_ground_texture_data(
    assets: &dyn crate::assets::AssetSource,
    dir_rel: &std::path::Path,
    ground_types: &[GroundTexture],
) -> Vec<u8> {
    let tex_size = 1024u32;
    let layer_bytes = (tex_size * tex_size * 4) as usize;
    let num_layers = ground_types.len();

    let tasks: Vec<_> = ground_types
        .iter()
        .map(|gt| {
            let filename = &gt.filename;
            move || load_ground_texture(assets, dir_rel, filename, tex_size)
        })
        .collect();
    let results = load_layers(tasks);

    // Assemble into flat buffer.
    let mut data = vec![128u8; layer_bytes * num_layers]; // Gray fallback
    for (i, result) in results.into_iter().enumerate() {
        match result {
            Some(rgba) => {
                let offset = i * layer_bytes;
                data[offset..offset + layer_bytes].copy_from_slice(&rgba);
                log::info!("Loaded ground texture [{i}] {}", ground_types[i].filename);
            }
            None => {
                log::warn!("Failed to load ground texture {}", ground_types[i].filename);
            }
        }
    }

    data
}

/// Load normal or specular map data for all ground types (background-thread safe).
///
/// `suffix` is `"_nm"` for normal maps or `"_sm"` for specular maps.
/// Returns a flat buffer of `num_layers` x 1024x1024x4 bytes, with a neutral
/// default for missing maps (flat normal `(128,128,255)` or black specular).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_ground_normal_specular_data(
    assets: &dyn crate::assets::AssetSource,
    dir_rel: &std::path::Path,
    ground_types: &[GroundTexture],
    suffix: &str,
) -> Vec<u8> {
    let tex_size = 1024u32;

    let tasks: Vec<_> = ground_types
        .iter()
        .map(|gt| {
            let variant_filename = if suffix == "_nm" {
                gt.normal_filename.as_deref()
            } else {
                gt.specular_filename.as_deref()
            };
            move || {
                variant_filename
                    .and_then(|fname| load_ground_texture(assets, dir_rel, fname, tex_size))
            }
        })
        .collect();
    let results = load_layers(tasks);

    assemble_texture_array_with_default(results, tex_size, suffix)
}

/// Load decal normal or specular maps from `tile-XX_nm.png`/`.ktx2` files.
///
/// `suffix` is `"_nm"` for normal maps or `"_sm"` for specular maps.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_decal_normal_specular_data(
    assets: &dyn crate::assets::AssetSource,
    tileset_256_rel: &std::path::Path,
    num_tiles: u32,
    suffix: &str,
) -> Vec<u8> {
    let tex_size = DECAL_TEX_SIZE;

    let tasks: Vec<_> = (0..num_tiles)
        .map(|i| {
            let filename = format!("tile-{i:02}{suffix}.png");
            move || load_ground_texture(assets, tileset_256_rel, &filename, tex_size)
        })
        .collect();
    let results = load_layers(tasks);

    assemble_texture_array_with_default(results, tex_size, suffix)
}

/// Assemble per-layer texture results into a flat RGBA buffer.
///
/// Missing layers are filled with a default pixel: flat normal `(128,128,255,255)`
/// for `_nm` suffix, black `(0,0,0,255)` for `_sm`.
#[cfg(not(target_arch = "wasm32"))]
fn assemble_texture_array_with_default(
    results: Vec<Option<Vec<u8>>>,
    tex_size: u32,
    suffix: &str,
) -> Vec<u8> {
    let layer_bytes = (tex_size * tex_size * 4) as usize;
    let default_pixel: [u8; 4] = if suffix == "_nm" {
        [128, 128, 255, 255]
    } else {
        [0, 0, 0, 255]
    };

    let mut data = vec![0u8; layer_bytes * results.len()];
    for (i, result) in results.into_iter().enumerate() {
        let offset = i * layer_bytes;
        if let Some(rgba) = result {
            data[offset..offset + layer_bytes].copy_from_slice(&rgba);
        } else {
            for pixel in data[offset..offset + layer_bytes].chunks_exact_mut(4) {
                pixel.copy_from_slice(&default_pixel);
            }
        }
    }
    data
}

/// Load decal tiles from the original `base.wz` archive (before classic.wz overlay).
///
/// Returns `(rgba_data, has_alpha)` where `rgba_data` is a flat RGBA buffer for
/// `num_tiles` layers, and `has_alpha[i]` is `true` if tile `i` had any
/// transparent pixels (marking it as a genuine decal tile).
///
/// Falls back to the extracted directory if base.wz is unavailable.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_decal_texture_data_from_wz<R: std::io::Read + std::io::Seek>(
    mut base_archive: Option<wz_maplib::io_wz::WzArchiveReader<R>>,
    tileset_subpath: &str,
    assets: &dyn crate::assets::AssetSource,
    tileset_128_rel: &std::path::Path,
    tileset_256_rel: &std::path::Path,
    num_tiles: u32,
) -> (Vec<u8>, Vec<bool>) {
    let tex_size = DECAL_TEX_SIZE;
    let layer_bytes = (tex_size * tex_size * 4) as usize;
    let mut data = vec![0u8; layer_bytes * num_tiles as usize];
    let mut has_alpha = vec![false; num_tiles as usize];

    for i in 0..num_tiles {
        let filename = format!("tile-{i:02}.png");
        let ktx2_filename = format!("tile-{i:02}.ktx2");

        // Priority 1: HQ 256px KTX2 tiles from high.wz (extracted to hw-256 dir).
        // high.wz KTX2 tiles are encoded in linear color space (unlike base.wz
        // ground KTX2 which is sRGB). Convert to sRGB so the Rgba8UnormSrgb GPU
        // decode produces correct colors.
        let rgba_opt = assets
            .bytes(&tileset_256_rel.join(&ktx2_filename))
            .and_then(|bytes| match load_ktx2_as_rgba_bytes(&bytes) {
                Ok(mut rgba) => {
                    linear_to_srgb(&mut rgba);
                    Some(resize_rgba(rgba, tex_size))
                }
                Err(e) => {
                    log::warn!("Failed to decode decal KTX2 tile-{i:02}: {e}");
                    None
                }
            })
            // Priority 2: HQ 256px PNG tiles from high.wz (extracted to hw-256 dir).
            .or_else(|| {
                assets
                    .bytes(&tileset_256_rel.join(&filename))
                    .and_then(|bytes| image::load_from_memory(&bytes).ok())
                    .map(|img| resize_rgba(img.to_rgba8(), tex_size))
            })
            // Priority 3: Original tiles from base.wz archive (preserves alpha).
            .or_else(|| {
                let entry_name = format!("{tileset_subpath}/{filename}");
                base_archive
                    .as_mut()
                    .and_then(|ar| ar.read_entry(&entry_name))
                    .and_then(|bytes| image::load_from_memory(&bytes).ok())
                    .map(|img| resize_rgba(img.to_rgba8(), tex_size))
            })
            .or_else(|| {
                assets
                    .bytes(&tileset_128_rel.join(&filename))
                    .and_then(|bytes| image::load_from_memory(&bytes).ok())
                    .map(|img| resize_rgba(img.to_rgba8(), tex_size))
            });

        if let Some(rgba) = rgba_opt {
            // Detect alpha BEFORE storing.
            let tile_has_alpha = rgba.chunks_exact(4).any(|px| px[3] < DECAL_ALPHA_THRESHOLD);
            has_alpha[i as usize] = tile_has_alpha;

            let offset = (i as usize) * layer_bytes;
            data[offset..offset + layer_bytes].copy_from_slice(&rgba);
        }
    }

    let alpha_count = has_alpha.iter().filter(|&&a| a).count();
    log::info!(
        "Loaded {num_tiles} decal tiles ({alpha_count} with alpha) at {tex_size}x{tex_size}",
    );

    (data, has_alpha)
}

/// Convert an RGBA image from linear to sRGB color space (RGB channels only).
///
/// high.wz KTX2 decal tiles are encoded in linear space. Since the GPU
/// texture uses `Rgba8UnormSrgb` (hardware applies sRGB-to-linear on
/// sample), we encode to sRGB here so the round-trip preserves the
/// original linear values. Note: base.wz ground KTX2 textures are already
/// sRGB and do NOT need this conversion.
pub fn linear_to_srgb(img: &mut image::RgbaImage) {
    for pixel in img.pixels_mut() {
        for c in 0..3 {
            let linear = pixel[c] as f32 / 255.0;
            let srgb = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            pixel[c] = (srgb * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
}

/// Decode one sRGB-encoded byte to its linear-light value in `[0, 1]`.
///
/// Inverse of [`linear_to_srgb_u8`], sharing the piecewise curve and
/// breakpoint used by [`linear_to_srgb`] so the pair round-trips. Mip
/// downsampling averages in linear light through this decode.
pub(crate) fn srgb_to_linear(v: u8) -> f32 {
    let s = f32::from(v) / 255.0;
    if s <= 0.040_45 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// Encode a linear-light value in `[0, 1]` back to an sRGB byte.
///
/// Inverse of [`srgb_to_linear`]; shares the encode branch with
/// [`linear_to_srgb`].
pub(crate) fn linear_to_srgb_u8(linear: f32) -> u8 {
    let srgb = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// One generated mip level: tightly packed RGBA8 at `width` x `height`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MipLevel {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Box-filter an RGBA8 image to half size (rounded down, floored at 1).
///
/// When `srgb`, RGB is decoded to linear light before averaging and
/// re-encoded, so minification stays gamma-correct: a naive byte average
/// darkens tiled ground textures at distance. Alpha carries no gamma and is
/// always the raw byte mean. Source coordinates are clamped so odd dimensions
/// degrade gracefully -- inputs are power-of-two in practice.
pub(crate) fn downsample_2x(src: &[u8], w: u32, h: u32, srgb: bool) -> MipLevel {
    let dst_w = (w / 2).max(1);
    let dst_h = (h / 2).max(1);
    let mut data = vec![0u8; dst_w as usize * dst_h as usize * 4];
    // Decode sRGB bytes through a 256-entry table: srgb_to_linear's per-sample
    // powf otherwise dominates the whole CPU-side mip upload (~12 decodes/pixel).
    let lut: Option<[f32; 256]> = srgb.then(|| std::array::from_fn(|v| srgb_to_linear(v as u8)));

    for dy in 0..dst_h {
        let sy0 = (dy * 2).min(h - 1);
        let sy1 = (sy0 + 1).min(h - 1);
        for dx in 0..dst_w {
            let sx0 = (dx * 2).min(w - 1);
            let sx1 = (sx0 + 1).min(w - 1);

            let px = |x: u32, y: u32| ((y * w + x) * 4) as usize;
            let corners = [px(sx0, sy0), px(sx1, sy0), px(sx0, sy1), px(sx1, sy1)];
            let out = (dy * dst_w + dx) as usize * 4;

            for c in 0..3 {
                data[out + c] = if let Some(lut) = &lut {
                    let sum: f32 = corners.iter().map(|&p| lut[usize::from(src[p + c])]).sum();
                    linear_to_srgb_u8(sum / 4.0)
                } else {
                    let sum: u32 = corners.iter().map(|&p| u32::from(src[p + c])).sum();
                    ((sum + 2) / 4) as u8
                };
            }
            let alpha: u32 = corners.iter().map(|&p| u32::from(src[p + 3])).sum();
            data[out + 3] = ((alpha + 2) / 4) as u8;
        }
    }

    MipLevel {
        data,
        width: dst_w,
        height: dst_h,
    }
}

/// Build a full mip chain from a base RGBA8 image.
///
/// Level 0 is `base`; each subsequent level is [`downsample_2x`] of the last,
/// stopping at 1x1 or after `max_levels` (when `Some`). For a power-of-two
/// square input the full chain is `size.ilog2() + 1` levels.
#[cfg(test)]
pub(crate) fn generate_mip_chain(
    base: &[u8],
    w: u32,
    h: u32,
    srgb: bool,
    max_levels: Option<u32>,
) -> Vec<MipLevel> {
    let mut levels = vec![MipLevel {
        data: base.to_vec(),
        width: w,
        height: h,
    }];
    loop {
        if max_levels.is_some_and(|max| levels.len() as u32 >= max) {
            break;
        }
        let last = levels.last().expect("chain always has level 0");
        if last.width <= 1 && last.height <= 1 {
            break;
        }
        let next = downsample_2x(&last.data, last.width, last.height, srgb);
        levels.push(next);
    }
    levels
}

/// Run per-layer texture loads in parallel across scoped threads, collecting
/// results in source order, for I/O throughput.
///
/// Native only: the web build has no usable OS threads (GitHub Pages cannot
/// send the COOP/COEP headers `SharedArrayBuffer` needs) and decodes through
/// its own frame-budgeted path in [`crate::app::web_ground`] instead.
#[cfg(not(target_arch = "wasm32"))]
fn load_layers<F>(tasks: Vec<F>) -> Vec<Option<Vec<u8>>>
where
    F: FnOnce() -> Option<Vec<u8>> + Send,
{
    std::thread::scope(|s| {
        let handles: Vec<_> = tasks.into_iter().map(|t| s.spawn(t)).collect();
        handles
            .into_iter()
            .map(|h| h.join().ok().flatten())
            .collect()
    })
}

/// Load a ground type texture as RGBA pixels.
///
/// Tries the disk cache first (native only), then KTX2 (high.wz), then PNG
/// (source/extracted). The web build has no disk cache and decodes on demand.
pub(crate) fn load_ground_texture(
    assets: &dyn crate::assets::AssetSource,
    dir_rel: &std::path::Path,
    filename: &str,
    target_size: u32,
) -> Option<Vec<u8>> {
    // Check disk cache first for previously decoded raw RGBA bytes. The
    // cache lives under the config dir, not the data root, so it is read
    // directly rather than through the asset source (native-only).
    #[cfg(not(target_arch = "wasm32"))]
    {
        let expected_bytes = (target_size * target_size * 4) as usize;
        let cache_name = filename.replace(".png", ".bin");
        let cache_path = crate::config::ground_cache_dir().join(&cache_name);
        if let Ok(data) = std::fs::read(&cache_path) {
            if data.len() == expected_bytes {
                log::debug!("Loaded cached ground texture: {cache_name}");
                return Some(data);
            }
            log::warn!("Cache file {cache_name} has wrong size, re-decoding");
        }
    }

    // Try KTX2 first - high.wz ships HQ BasisU+Zstd compressed textures
    // that are higher quality than the base.wz PNGs.
    // high.wz KTX2 textures are encoded in linear color space. Diffuse
    // textures need `linear_to_srgb` before uploading as Rgba8UnormSrgb.
    // Normal/specular maps (_nm/_sm) stay linear (uploaded as Rgba8Unorm).
    let ktx2_name = filename.replace(".png", ".ktx2");
    let ktx2_rel = dir_rel.join(&ktx2_name);
    let is_diffuse = !filename.contains("_nm") && !filename.contains("_sm");
    if let Some(bytes) = assets.bytes(&ktx2_rel) {
        match load_ktx2_as_rgba_bytes(&bytes) {
            Ok(mut rgba) => {
                log::info!(
                    "Decoded KTX2 {ktx2_name}: {}x{} (linear_to_srgb={is_diffuse})",
                    rgba.width(),
                    rgba.height(),
                );
                if is_diffuse {
                    linear_to_srgb(&mut rgba);
                }
                let resized_data = resize_rgba(rgba, target_size);
                // Cache raw RGBA bytes at target size for instant loading.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let cache_name = filename.replace(".png", ".bin");
                    if let Err(e) = cache_ground_texture_raw(
                        &crate::config::ground_cache_dir(),
                        &cache_name,
                        &resized_data,
                    ) {
                        log::warn!("Failed to cache ground texture {cache_name}: {e}");
                    }
                }
                return Some(resized_data);
            }
            Err(e) => {
                log::warn!("Failed to decode KTX2 {ktx2_name}: {e}");
            }
        }
    }

    // Fall back to PNG (source checkouts or base.wz extracted assets).
    let png_rel = dir_rel.join(filename);
    if let Some(bytes) = assets.bytes(&png_rel)
        && let Ok(img) = image::load_from_memory(&bytes)
    {
        let rgba = img.to_rgba8();
        return Some(resize_rgba(rgba, target_size));
    }

    log::debug!("Ground texture not found: {filename} (tried cache, .ktx2, and .png)");
    None
}

/// Decode a single decal tile to RGBA at `target_size` (web build).
///
/// Mirrors the priority chain of [`load_decal_texture_data_from_wz`] minus the
/// `base.wz` archive step (the web VFS overlays high.wz onto the base tree and
/// has no separate pre-overlay archive): HQ 256px KTX2, then HQ 256px PNG, then
/// the extracted 128px PNG. high.wz KTX2 tiles are linear, so the diffuse path
/// converts to sRGB for the `Rgba8UnormSrgb` upload.
#[cfg(target_arch = "wasm32")]
pub(crate) fn load_decal_tile(
    assets: &dyn crate::assets::AssetSource,
    tileset_128_rel: &std::path::Path,
    tileset_256_rel: &std::path::Path,
    tile_index: u32,
    target_size: u32,
) -> Option<Vec<u8>> {
    let filename = format!("tile-{tile_index:02}.png");
    let ktx2_filename = format!("tile-{tile_index:02}.ktx2");
    assets
        .bytes(&tileset_256_rel.join(&ktx2_filename))
        .and_then(|bytes| match load_ktx2_as_rgba_bytes(&bytes) {
            Ok(mut rgba) => {
                linear_to_srgb(&mut rgba);
                Some(resize_rgba(rgba, target_size))
            }
            Err(e) => {
                log::warn!("Failed to decode decal KTX2 tile-{tile_index:02}: {e}");
                None
            }
        })
        .or_else(|| {
            assets
                .bytes(&tileset_256_rel.join(&filename))
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|img| resize_rgba(img.to_rgba8(), target_size))
        })
        .or_else(|| {
            assets
                .bytes(&tileset_128_rel.join(&filename))
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|img| resize_rgba(img.to_rgba8(), target_size))
        })
}

/// Save raw RGBA bytes to the disk cache.
#[cfg(not(target_arch = "wasm32"))]
fn cache_ground_texture_raw(
    cache_dir: &std::path::Path,
    cache_name: &str,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(cache_dir)?;
    let path = cache_dir.join(cache_name);
    std::fs::write(&path, data)?;
    log::info!("Cached ground texture: {}", path.display());
    Ok(())
}

/// Resize RGBA image to `target_size` x `target_size` if needed.
///
/// Uses `CatmullRom` filter which is visually equivalent to Lanczos3
/// for game textures but significantly faster.
pub fn resize_rgba(img: image::RgbaImage, target_size: u32) -> Vec<u8> {
    if img.width() == target_size && img.height() == target_size {
        img.into_raw()
    } else {
        image::imageops::resize(
            &img,
            target_size,
            target_size,
            image::imageops::FilterType::CatmullRom,
        )
        .into_raw()
    }
}

/// Build a flat `layers` × `tex_size`² RGBA buffer with every pixel set to
/// `pixel`.
///
/// Seeds a ground/decal texture array with a neutral default (mid-gray diffuse,
/// flat normal, black specular) so layers with no source asset still upload a
/// sensible value when decoded layers overwrite only the slots that exist.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn solid_color_array(pixel: [u8; 4], tex_size: u32, layers: u32) -> Vec<u8> {
    let pixels = (tex_size * tex_size) as usize * layers as usize;
    // Build in one pass: reserving then writing the pattern avoids the leading
    // zero-fill a `vec![0; n]` + overwrite would do over a multi-MiB buffer.
    let mut data = Vec::with_capacity(pixels * 4);
    for _ in 0..pixels {
        data.extend_from_slice(&pixel);
    }
    data
}

/// Copy one decoded RGBA layer into `buffer` at `offset`.
///
/// Returns `false` and leaves `buffer` untouched if the layer would overrun, so
/// a malformed decode (wrong dimensions) cannot corrupt adjacent layers.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn write_array_layer(buffer: &mut [u8], offset: usize, rgba: &[u8]) -> bool {
    let Some(end) = offset.checked_add(rgba.len()) else {
        return false;
    };
    if end <= buffer.len() {
        buffer[offset..end].copy_from_slice(rgba);
        true
    } else {
        false
    }
}

/// Decode a KTX2 file to RGBA8.
///
/// WZ2100 v4.x ships ground textures as KTX2 with UASTC blocks
/// and Zstandard supercompression. Decoding pipeline:
/// 1. Parse KTX2 header for dimensions, level offsets, supercompression
/// 2. Zstd-decompress level 0 (highest resolution)
/// 3. Transcode UASTC blocks to RGBA32 via basis-universal
#[cfg(not(target_arch = "wasm32"))]
pub fn load_ktx2_as_rgba(
    path: &std::path::Path,
) -> Result<image::RgbaImage, Box<dyn std::error::Error>> {
    load_ktx2_as_rgba_bytes(&std::fs::read(path)?)
}

/// Decode KTX2 bytes (already read into memory) to RGBA8. See
/// [`load_ktx2_as_rgba`] for the decoding pipeline.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_ktx2_as_rgba_bytes(
    file_data: &[u8],
) -> Result<image::RgbaImage, Box<dyn std::error::Error>> {
    use basis_universal::transcoding::LowLevelUastcTranscoder;

    if file_data.len() < 104 {
        return Err("File too small for KTX2".into());
    }

    // Verify KTX2 magic.
    if &file_data[..12] != b"\xABKTX 20\xBB\r\n\x1A\n" {
        return Err("Not a KTX2 file".into());
    }

    let pixel_width = u32::from_le_bytes(file_data[20..24].try_into()?);
    let pixel_height = u32::from_le_bytes(file_data[24..28].try_into()?);
    let level_count = u32::from_le_bytes(file_data[40..44].try_into()?);
    let supercompression = u32::from_le_bytes(file_data[44..48].try_into()?);

    if level_count == 0 {
        return Err("KTX2 has no mip levels".into());
    }

    // Level index starts at byte 80: each entry is 24 bytes.
    let level0_offset = u64::from_le_bytes(file_data[80..88].try_into()?) as usize;
    let level0_length = u64::from_le_bytes(file_data[88..96].try_into()?) as usize;

    if level0_offset + level0_length > file_data.len() {
        return Err("KTX2 level 0 data out of bounds".into());
    }

    let level0_raw = &file_data[level0_offset..level0_offset + level0_length];

    // Decompress if Zstd supercompressed (scheme 2).
    let block_data = match supercompression {
        0 => level0_raw.to_vec(),
        2 => {
            let mut output = Vec::new();
            let mut decoder = std::io::Cursor::new(level0_raw);
            zstd::stream::copy_decode(&mut decoder, &mut output)?;
            output
        }
        other => return Err(format!("Unsupported supercompression: {other}").into()),
    };

    // Transcode UASTC blocks to RGBA32 using basis-universal. Calls the C
    // FFI directly instead of `LowLevelUastcTranscoder::transcode_slice()`
    // because the Rust wrapper has a bug: it computes `output_row_pitch`
    // by dividing `original_width` by `block_width()` (4 for RGBA32), but
    // the C++ code expects pixels for uncompressed formats. This causes a
    // buffer overflow and crash for any texture larger than 4x4.
    let num_blocks_x = pixel_width.div_ceil(4);
    let num_blocks_y = pixel_height.div_ceil(4);
    let output_size = (pixel_width * pixel_height * 4) as usize;
    let mut rgba_bytes = vec![0u8; output_size];

    let transcoder = LowLevelUastcTranscoder::new();
    let success = unsafe {
        // block_format::cRGBA32 = 22
        const BLOCK_FORMAT_RGBA32: i32 = 22;
        basis_universal_sys::low_level_uastc_transcoder_transcode_slice(
            // LowLevelUastcTranscoder is a newtype around *mut sys-type, with
            // the raw pointer as its only field, so cast through the address.
            *(std::ptr::from_ref(&transcoder)
                .cast::<*mut basis_universal_sys::LowLevelUastcTranscoder>()),
            rgba_bytes.as_mut_ptr().cast(),
            num_blocks_x,
            num_blocks_y,
            block_data.as_ptr(),
            block_data.len() as u32,
            BLOCK_FORMAT_RGBA32,
            4,     // output_block_or_pixel_stride_in_bytes (4 for RGBA32)
            false, // bc1_allow_threecolor_blocks
            true,  // has_alpha
            pixel_width,
            pixel_height,
            pixel_width,          // output_row_pitch in PIXELS (not blocks!)
            std::ptr::null_mut(), // transcoder_state
            pixel_height,         // output_rows_in_pixels
            0,                    // channel0
            3,                    // channel1
            0,                    // decode_flags
        )
    };
    // Keep transcoder alive until after the FFI call.
    drop(transcoder);

    if !success {
        return Err("UASTC transcode to RGBA32 failed".into());
    }

    image::RgbaImage::from_raw(pixel_width, pixel_height, rgba_bytes)
        .ok_or_else(|| "Failed to create image from transcoded UASTC".into())
}

/// Decode KTX2 bytes to RGBA8 via the browser-side Basis Universal transcoder.
///
/// `KTX2File` handles the Zstandard supercompression and UASTC transcode
/// internally, so the whole file is passed through unchanged. Returns `Err`
/// when the transcoder is not yet loaded or the file cannot be transcoded;
/// callers then fall back to the PNG path, exactly as on a native decode error.
#[cfg(target_arch = "wasm32")]
pub fn load_ktx2_as_rgba_bytes(
    file_data: &[u8],
) -> Result<image::RgbaImage, Box<dyn std::error::Error>> {
    let (width, height, rgba) = crate::viewport::basis::transcode_ktx2_to_rgba(file_data)
        .ok_or("KTX2 transcode unavailable or failed")?;
    image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "Failed to build image from transcoded KTX2".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a filesystem asset source rooted at `dir` so the loaders read
    /// `dir/<filename>` for an empty relative directory.
    fn fs(dir: &std::path::Path) -> crate::assets::FsAssetSource {
        crate::assets::FsAssetSource::new(dir.to_path_buf())
    }

    const ROOT: &str = "";

    #[test]
    fn resize_rgba_noop_when_already_target_size() {
        let img = image::RgbaImage::from_pixel(256, 256, image::Rgba([1, 2, 3, 255]));
        let result = resize_rgba(img, 256);
        assert_eq!(result.len(), 256 * 256 * 4);
        // First pixel should be unchanged.
        assert_eq!(&result[0..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn resize_rgba_downscales_correctly() {
        let img = image::RgbaImage::from_pixel(512, 512, image::Rgba([100, 150, 200, 255]));
        let result = resize_rgba(img, 256);
        assert_eq!(result.len(), 256 * 256 * 4);
        // Uniform color should survive downscale.
        assert_eq!(result[0], 100);
        assert_eq!(result[1], 150);
        assert_eq!(result[2], 200);
        assert_eq!(result[3], 255);
    }

    #[test]
    fn cache_ground_texture_writes_raw_file() {
        let dir = std::env::temp_dir().join("wz_test_cache_ground_write");
        let _ = std::fs::remove_dir_all(&dir);

        let expected_bytes = 256 * 256 * 4;
        let mut data = vec![0u8; expected_bytes];
        data[0] = 10;
        data[1] = 20;
        data[2] = 30;
        data[3] = 255;
        cache_ground_texture_raw(&dir, "test-texture.bin", &data).expect("cache write failed");

        let cached = dir.join("test-texture.bin");
        assert!(cached.exists());

        // Read it back and verify raw bytes.
        let loaded = std::fs::read(&cached).unwrap();
        assert_eq!(loaded.len(), expected_bytes);
        assert_eq!(loaded[0], 10);
        assert_eq!(loaded[1], 20);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_ground_texture_from_png() {
        let dir = std::env::temp_dir().join("wz_test_load_ground_png");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        let img = image::RgbaImage::from_pixel(256, 256, image::Rgba([42, 84, 126, 255]));
        img.save(dir.join("test-ground.png")).unwrap();

        let result = load_ground_texture(
            &fs(&dir),
            std::path::Path::new(ROOT),
            "test-ground.png",
            256,
        );
        assert!(result.is_some());
        let data = result.unwrap();
        assert_eq!(data.len(), 256 * 256 * 4);
        assert_eq!(data[0], 42);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_ground_texture_falls_back_to_cache() {
        let texpages_dir = std::env::temp_dir().join("wz_test_load_cache_fallback");
        let _ = std::fs::remove_dir_all(&texpages_dir);
        let _ = std::fs::create_dir_all(&texpages_dir);
        // No PNG in texpages_dir, but put raw .bin in the cache.
        let cache_dir = crate::config::ground_cache_dir();
        let _ = std::fs::create_dir_all(&cache_dir);
        let mut raw_data = vec![0u8; 256 * 256 * 4];
        raw_data[0] = 99;
        raw_data[1] = 88;
        raw_data[2] = 77;
        std::fs::write(cache_dir.join("cached-only.bin"), &raw_data).unwrap();

        let result = load_ground_texture(
            &fs(&texpages_dir),
            std::path::Path::new(ROOT),
            "cached-only.png",
            256,
        );
        assert!(result.is_some());
        let data = result.unwrap();
        assert_eq!(data[0], 99);

        let _ = std::fs::remove_dir_all(&texpages_dir);
        let _ = std::fs::remove_file(cache_dir.join("cached-only.bin"));
    }

    #[test]
    fn load_ground_texture_data_produces_correct_buffer_size() {
        let dir = std::env::temp_dir().join("wz_test_load_data_buf");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        // Create 3 ground textures.
        for i in 0..3 {
            let color = (i as u8 + 1) * 50;
            let img = image::RgbaImage::from_pixel(1024, 1024, image::Rgba([color, 0, 0, 255]));
            img.save(dir.join(format!("page-{i}.png"))).unwrap();
        }

        let ground_types = vec![
            GroundTexture {
                name: "a".to_string(),
                filename: "page-0.png".to_string(),
                scale: 1.0,
                normal_filename: None,
                specular_filename: None,
            },
            GroundTexture {
                name: "b".to_string(),
                filename: "page-1.png".to_string(),
                scale: 1.0,
                normal_filename: None,
                specular_filename: None,
            },
            GroundTexture {
                name: "c".to_string(),
                filename: "page-2.png".to_string(),
                scale: 1.0,
                normal_filename: None,
                specular_filename: None,
            },
        ];

        let data = load_ground_texture_data(&fs(&dir), std::path::Path::new(ROOT), &ground_types);
        let expected_size = 3 * 1024 * 1024 * 4;
        assert_eq!(data.len(), expected_size);

        // First layer should be color (50, 0, 0).
        assert_eq!(data[0], 50);
        // Second layer starts at 1024*1024*4.
        let layer2_offset = 1024 * 1024 * 4;
        assert_eq!(data[layer2_offset], 100);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_ground_texture_missing_returns_none() {
        let dir = std::env::temp_dir().join("wz_test_load_ground_missing");
        let _ = std::fs::create_dir_all(&dir);

        let result = load_ground_texture(
            &fs(&dir),
            std::path::Path::new(ROOT),
            "nonexistent-texture.png",
            256,
        );
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_ground_normal_specular_data_defaults_for_missing_textures() {
        let dir = std::env::temp_dir().join("wz_test_nm_sm_defaults");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        let ground_types = vec![GroundTexture {
            name: "a".to_string(),
            filename: "page-0.png".to_string(),
            scale: 1.0,
            normal_filename: None, // No _nm file
            specular_filename: None,
        }];

        // Normal maps default to flat normal (128, 128, 255, 255).
        let nm_data = load_ground_normal_specular_data(
            &fs(&dir),
            std::path::Path::new(ROOT),
            &ground_types,
            "_nm",
        );
        let layer_bytes = 1024 * 1024 * 4;
        assert_eq!(nm_data.len(), layer_bytes);
        assert_eq!(nm_data[0], 128);
        assert_eq!(nm_data[1], 128);
        assert_eq!(nm_data[2], 255);
        assert_eq!(nm_data[3], 255);

        // Specular maps default to black (0, 0, 0, 255).
        let sm_data = load_ground_normal_specular_data(
            &fs(&dir),
            std::path::Path::new(ROOT),
            &ground_types,
            "_sm",
        );
        assert_eq!(sm_data.len(), layer_bytes);
        assert_eq!(sm_data[0], 0);
        assert_eq!(sm_data[1], 0);
        assert_eq!(sm_data[2], 0);
        assert_eq!(sm_data[3], 255);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_ground_normal_specular_data_loads_existing_texture() {
        let dir = std::env::temp_dir().join("wz_test_nm_sm_existing");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        // Create a normal map PNG.
        let img = image::RgbaImage::from_pixel(1024, 1024, image::Rgba([100, 200, 250, 255]));
        img.save(dir.join("page-0_nm.png")).unwrap();

        let ground_types = vec![GroundTexture {
            name: "a".to_string(),
            filename: "page-0.png".to_string(),
            scale: 1.0,
            normal_filename: Some("page-0_nm.png".to_string()),
            specular_filename: None,
        }];

        let nm_data = load_ground_normal_specular_data(
            &fs(&dir),
            std::path::Path::new(ROOT),
            &ground_types,
            "_nm",
        );
        let layer_bytes = 1024 * 1024 * 4;
        assert_eq!(nm_data.len(), layer_bytes);
        // Should have loaded the actual texture, not the default.
        assert_eq!(nm_data[0], 100);
        assert_eq!(nm_data[1], 200);
        assert_eq!(nm_data[2], 250);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_ground_normal_specular_data_multiple_layers() {
        let dir = std::env::temp_dir().join("wz_test_nm_sm_multi");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        let ground_types = vec![
            GroundTexture {
                name: "a".to_string(),
                filename: "page-0.png".to_string(),
                scale: 1.0,
                normal_filename: None,
                specular_filename: None,
            },
            GroundTexture {
                name: "b".to_string(),
                filename: "page-1.png".to_string(),
                scale: 1.0,
                normal_filename: None,
                specular_filename: None,
            },
        ];

        let data = load_ground_normal_specular_data(
            &fs(&dir),
            std::path::Path::new(ROOT),
            &ground_types,
            "_nm",
        );
        let layer_bytes = 1024 * 1024 * 4;
        assert_eq!(data.len(), layer_bytes * 2);
        // Both layers should have the flat normal default.
        assert_eq!(data[0], 128);
        assert_eq!(data[layer_bytes], 128);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn solid_color_array_fills_every_pixel() {
        let data = solid_color_array([128, 128, 255, 255], 4, 2);
        assert_eq!(data.len(), 4 * 4 * 4 * 2);
        for px in data.chunks_exact(4) {
            assert_eq!(px, [128, 128, 255, 255]);
        }
    }

    #[test]
    fn solid_color_array_zero_layers_is_empty() {
        assert!(solid_color_array([0, 0, 0, 255], 256, 0).is_empty());
    }

    #[test]
    fn write_array_layer_copies_into_its_slot() {
        let mut buffer = vec![0u8; 12];
        assert!(write_array_layer(&mut buffer, 4, &[1, 2, 3, 4]));
        assert_eq!(buffer, [0, 0, 0, 0, 1, 2, 3, 4, 0, 0, 0, 0]);
    }

    #[test]
    fn write_array_layer_rejects_overrun_without_corrupting() {
        let mut buffer = vec![9u8; 6];
        // A layer that would spill past the end must be dropped, not clamped.
        assert!(!write_array_layer(&mut buffer, 4, &[1, 2, 3, 4]));
        assert_eq!(buffer, [9, 9, 9, 9, 9, 9]);
    }

    #[test]
    fn linear_to_srgb_maps_endpoints_and_midpoint() {
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 128, 255, 200]));
        linear_to_srgb(&mut img);
        let px = img.get_pixel(0, 0).0;
        assert_eq!(px[0], 0);
        assert_eq!(px[2], 255);
        assert_eq!(px[3], 200);
        // Linear 128/255 encodes to ~188 in sRGB; allow a little rounding slack.
        assert!((187..=189).contains(&px[1]), "got {}", px[1]);
    }

    #[test]
    fn mip_chain_level_count_and_dims() {
        let base = vec![0u8; 1024 * 1024 * 4];
        let chain = generate_mip_chain(&base, 1024, 1024, false, None);
        assert_eq!(chain.len(), 11);
        for (i, level) in chain.iter().enumerate() {
            let expected = 1024u32 >> i;
            assert_eq!((level.width, level.height), (expected, expected));
            assert_eq!(level.data.len() as u32, expected * expected * 4);
        }
        assert_eq!((chain[10].width, chain[10].height), (1, 1));

        let base256 = vec![0u8; 256 * 256 * 4];
        let chain256 = generate_mip_chain(&base256, 256, 256, false, None);
        assert_eq!(chain256.len(), 9);
        assert_eq!((chain256[8].width, chain256[8].height), (1, 1));

        // Capping stops the chain before 1x1: 1024 -> 512 -> ... -> 32.
        let capped = generate_mip_chain(&base, 1024, 1024, false, Some(6));
        assert_eq!(capped.len(), 6);
        assert_eq!((capped[5].width, capped[5].height), (32, 32));
    }

    #[test]
    fn mip_chain_preserves_solid_color() {
        let color = [200u8, 100, 50, 180];
        for srgb in [false, true] {
            let mut base = Vec::with_capacity(64 * 64 * 4);
            for _ in 0..64 * 64 {
                base.extend_from_slice(&color);
            }
            let chain = generate_mip_chain(&base, 64, 64, srgb, None);
            for level in &chain {
                for px in level.data.chunks_exact(4) {
                    for c in 0..4 {
                        assert!(
                            (i16::from(px[c]) - i16::from(color[c])).abs() <= 1,
                            "srgb={srgb} channel {c}: got {} want {}",
                            px[c],
                            color[c],
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn downsample_srgb_false_is_exact_byte_mean() {
        // One 2x2 block of distinct values -> 1x1 rounded mean (n+2)/4.
        let src = [
            10, 20, 30, 40, //
            50, 60, 70, 80, //
            90, 100, 110, 120, //
            130, 140, 150, 160,
        ];
        let mip = downsample_2x(&src, 2, 2, false);
        assert_eq!((mip.width, mip.height), (1, 1));
        assert_eq!(&mip.data, &[70, 80, 90, 100]);
    }

    #[test]
    fn downsample_srgb_false_matches_rounded_mean_formula() {
        let src = [
            3, 250, 17, 99, //
            200, 4, 128, 1, //
            60, 61, 62, 63, //
            7, 254, 9, 255,
        ];
        let mip = downsample_2x(&src, 2, 2, false);
        for c in 0..4 {
            let sum = u32::from(src[c])
                + u32::from(src[4 + c])
                + u32::from(src[8 + c])
                + u32::from(src[12 + c]);
            assert_eq!(u32::from(mip.data[c]), (sum + 2) / 4, "channel {c}");
        }
    }

    #[test]
    fn downsample_srgb_true_is_gamma_correct() {
        // Two black + two white texels. The gamma-correct mean is linear 0.5,
        // re-encoding to ~188 in sRGB -- explicitly not the naive byte mean 127.
        let src = [
            0, 0, 0, 0, //
            255, 255, 255, 255, //
            0, 0, 0, 0, //
            255, 255, 255, 255,
        ];
        let mip = downsample_2x(&src, 2, 2, true);
        let r = mip.data[0];
        assert!(
            (187..=189).contains(&r),
            "gamma-correct mean should be ~188, got {r}"
        );
    }

    #[test]
    fn downsample_srgb_true_averages_alpha_as_raw_bytes() {
        // Alpha carries no gamma: with srgb=true it must still be the raw byte
        // mean. Two alpha=0 + two alpha=255 -> ~128, not the gamma-decoded 188.
        let src = [
            0, 0, 0, 0, //
            0, 0, 0, 255, //
            0, 0, 0, 0, //
            0, 0, 0, 255,
        ];
        let mip = downsample_2x(&src, 2, 2, true);
        assert_eq!(mip.data[3], 128);
    }
}
