//! Tileset texture atlas builder.

use std::path::Path;

/// Each tile is uploaded as its own layer of a texture array so it gets an
/// independent mip chain. A packed atlas would bleed neighbouring tiles across
/// shared edges at coarse mips (no gutters), producing bright tile-edge seams.
const TILE_SIZE: u32 = 256;

/// Built tile texture array, ready for GPU upload: `layers` × `tile_size`²
/// RGBA8, one tile per layer.
pub struct TileAtlas {
    pub data: Vec<u8>,
    pub tile_size: u32,
    pub layers: u32,
    pub tile_count: u32,
}

impl std::fmt::Debug for TileAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileAtlas")
            .field("tile_size", &self.tile_size)
            .field("layers", &self.layers)
            .field("tile_count", &self.tile_count)
            .finish_non_exhaustive()
    }
}

impl TileAtlas {
    /// Build a texture atlas from `tile-XX.png` files in a directory.
    ///
    /// RGBA (transition) tiles are pre-composited so transparent pixels
    /// take the tile's average opaque color, avoiding the need for
    /// multi-pass blending. Missing tile slots fall back to tile-00's
    /// average. Returns `None` if no tiles were found.
    pub fn build(assets: &dyn crate::assets::AssetSource, tileset_rel: &Path) -> Option<Self> {
        let mut max_index: u32 = 0;
        let mut tile_count: u32 = 0;
        for i in 0u32..256 {
            let filename = format!("tile-{i:02}.png");
            if assets.exists(&tileset_rel.join(&filename)) {
                max_index = i;
                tile_count += 1;
            }
        }

        if tile_count == 0 {
            log::warn!("No tile images found in {}", tileset_rel.display());
            return None;
        }

        let tile_size = TILE_SIZE;
        let layers = max_index + 1;
        let layer_bytes = (tile_size * tile_size * 4) as usize;

        // Neutral desert tan if tile-00 fails to load.
        let fallback_rgb =
            compute_tile_avg_opaque_color(assets, tileset_rel, 0).unwrap_or([0x80, 0x70, 0x50]);

        // One layer per tile slot, pre-filled with the fallback so slots with
        // no source image render as tan rather than black.
        let mut data = vec![0u8; layer_bytes * layers as usize];
        for px in data.chunks_exact_mut(4) {
            px[0] = fallback_rgb[0];
            px[1] = fallback_rgb[1];
            px[2] = fallback_rgb[2];
            px[3] = 255;
        }

        for i in 0u32..=max_index {
            let filename = format!("tile-{i:02}.png");
            let Some(bytes) = assets.bytes(&tileset_rel.join(&filename)) else {
                continue;
            };

            let img = match image::load_from_memory(&bytes) {
                Ok(img) => img.to_rgba8(),
                Err(e) => {
                    log::warn!("Failed to load tile {filename}: {e}");
                    continue;
                }
            };

            // Source tiles ship at 128px; upscale to TILE_SIZE when needed.
            // Preserve alpha (Medium's decal overlay uses it; Classic ignores it).
            let tile_data = if img.width() == tile_size && img.height() == tile_size {
                img.into_raw()
            } else {
                image::imageops::resize(
                    &img,
                    tile_size,
                    tile_size,
                    image::imageops::FilterType::Lanczos3,
                )
                .into_raw()
            };

            let offset = i as usize * layer_bytes;
            data[offset..offset + layer_bytes].copy_from_slice(&tile_data);
        }

        log::info!(
            "Built tileset atlas: {tile_count} tiles, {layers} layers x {tile_size}px (fallback RGB: {fallback_rgb:?})"
        );

        Some(TileAtlas {
            data,
            tile_size,
            layers,
            tile_count,
        })
    }
}

fn avg_opaque_color(img: &image::RgbaImage) -> [u8; 3] {
    let mut r_sum = 0u64;
    let mut g_sum = 0u64;
    let mut b_sum = 0u64;
    let mut count = 0u64;

    for pixel in img.pixels() {
        if pixel[3] > 128 {
            r_sum += pixel[0] as u64;
            g_sum += pixel[1] as u64;
            b_sum += pixel[2] as u64;
            count += 1;
        }
    }

    if count == 0 {
        return [0x80, 0x70, 0x50]; // Neutral fallback.
    }

    [
        (r_sum / count) as u8,
        (g_sum / count) as u8,
        (b_sum / count) as u8,
    ]
}

/// Compute the average opaque color of a specific tile PNG by index.
fn compute_tile_avg_opaque_color(
    assets: &dyn crate::assets::AssetSource,
    tileset_rel: &Path,
    index: u32,
) -> Option<[u8; 3]> {
    let bytes = assets.bytes(&tileset_rel.join(format!("tile-{index:02}.png")))?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    Some(avg_opaque_color(&img))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(dir: &Path) -> crate::assets::FsAssetSource {
        crate::assets::FsAssetSource::new(dir.to_path_buf())
    }

    #[test]
    fn build_returns_none_for_empty_directory() {
        let dir = std::env::temp_dir().join("wz2100_atlas_test_empty");
        // Ensure the directory exists but is empty
        let _ = std::fs::create_dir_all(&dir);
        // Remove any leftover tile files from prior runs
        for i in 0u32..256 {
            let _ = std::fs::remove_file(dir.join(format!("tile-{i:02}.png")));
        }

        let result = TileAtlas::build(&fs(&dir), Path::new(""));
        assert!(result.is_none(), "Expected None for empty tile directory");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_returns_none_for_nonexistent_directory() {
        let dir = std::env::temp_dir().join("wz2100_atlas_test_nonexistent_dir_xyz");
        // Make sure it doesn't exist
        let _ = std::fs::remove_dir_all(&dir);

        let result = TileAtlas::build(&fs(&dir), Path::new(""));
        assert!(result.is_none(), "Expected None for nonexistent directory");
    }

    #[test]
    fn tile_size_is_256() {
        assert_eq!(TILE_SIZE, 256);
    }

    #[test]
    fn built_atlas_has_one_layer_per_tile() {
        let dir = std::env::temp_dir().join("wz2100_atlas_test_layers");
        let _ = std::fs::create_dir_all(&dir);

        let img = image::RgbaImage::from_pixel(128, 128, image::Rgba([255, 0, 0, 255]));
        img.save(dir.join("tile-00.png"))
            .expect("Failed to save test tile PNG");

        let atlas = TileAtlas::build(&fs(&dir), Path::new(""))
            .expect("Expected Some atlas from directory with one tile");
        assert_eq!(atlas.tile_size, 256);
        assert_eq!(atlas.layers, 1);
        assert_eq!(atlas.tile_count, 1);
        assert_eq!(atlas.data.len(), 256 * 256 * 4);
        // The 128px source upscales to solid 256px red.
        assert_eq!(&atlas.data[0..4], &[255, 0, 0, 255]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn avg_opaque_color_ignores_transparent_pixels() {
        let mut img = image::RgbaImage::new(2, 2);
        // Two opaque red pixels, two fully transparent green pixels.
        img.put_pixel(0, 0, image::Rgba([200, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([100, 0, 0, 255]));
        img.put_pixel(0, 1, image::Rgba([0, 255, 0, 0])); // Transparent.
        img.put_pixel(1, 1, image::Rgba([0, 255, 0, 50])); // Below threshold.
        let avg = avg_opaque_color(&img);
        assert_eq!(avg, [150, 0, 0]); // Average of 200 and 100.
    }

    #[test]
    fn avg_opaque_color_fallback_when_all_transparent() {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
        let avg = avg_opaque_color(&img);
        // Neutral fallback color when no opaque pixels.
        assert_eq!(avg, [0x80, 0x70, 0x50]);
    }
}
