//! Tileset texture atlas builder.

use std::path::Path;

/// 16x16 grid is required by the shader: `tile_size = 1.0 / atlas_cols`
/// is used for both U and V, so the atlas must be square.
const ATLAS_COLS: u32 = 16;
const TILE_SIZE: u32 = 256;

/// Built tile atlas, ready for GPU upload.
pub struct TileAtlas {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub tile_count: u32,
}

impl std::fmt::Debug for TileAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileAtlas")
            .field("width", &self.width)
            .field("height", &self.height)
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

        let atlas_width = ATLAS_COLS * TILE_SIZE;
        let atlas_height = ATLAS_COLS * TILE_SIZE;
        let mut data = vec![0u8; (atlas_width * atlas_height * 4) as usize];

        // Neutral desert tan if tile-00 fails to load.
        let fallback_rgb =
            compute_tile_avg_opaque_color(assets, tileset_rel, 0).unwrap_or([0x80, 0x70, 0x50]);

        // Pre-fill so missing tile slots render as the fallback, not black.
        for pixel_offset in (0..data.len()).step_by(4) {
            data[pixel_offset] = fallback_rgb[0];
            data[pixel_offset + 1] = fallback_rgb[1];
            data[pixel_offset + 2] = fallback_rgb[2];
            data[pixel_offset + 3] = 255;
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

            let col = i % ATLAS_COLS;
            let row = i / ATLAS_COLS;
            let dst_x = col * TILE_SIZE;
            let dst_y = row * TILE_SIZE;

            // Source tiles ship at 128px; upscale to TILE_SIZE when needed.
            let resized;
            let tile_img = if img.width() != TILE_SIZE || img.height() != TILE_SIZE {
                resized = image::imageops::resize(
                    &img,
                    TILE_SIZE,
                    TILE_SIZE,
                    image::imageops::FilterType::Lanczos3,
                );
                &resized
            } else {
                &img
            };

            // Preserve alpha: Medium uses it to blend decals over ground
            // splatting; Classic ignores it.
            for py in 0..TILE_SIZE {
                for px in 0..TILE_SIZE {
                    let pixel = tile_img.get_pixel(px, py);
                    let dst_offset = ((dst_y + py) * atlas_width + (dst_x + px)) as usize * 4;
                    if dst_offset + 3 >= data.len() {
                        continue;
                    }

                    data[dst_offset] = pixel[0];
                    data[dst_offset + 1] = pixel[1];
                    data[dst_offset + 2] = pixel[2];
                    data[dst_offset + 3] = pixel[3];
                }
            }
        }

        log::info!(
            "Built tileset atlas: {tile_count} tiles, {atlas_width}x{atlas_height} pixels (fallback RGB: {fallback_rgb:?})"
        );

        Some(TileAtlas {
            data,
            width: atlas_width,
            height: atlas_height,
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
    fn atlas_constants_produce_4096_square() {
        // 16 columns × 256px tiles = 4096px per side.
        assert_eq!(ATLAS_COLS * TILE_SIZE, 4096);
    }

    #[test]
    fn built_atlas_is_square_4096() {
        let dir = std::env::temp_dir().join("wz2100_atlas_test_square");
        let _ = std::fs::create_dir_all(&dir);

        let img = image::RgbaImage::from_pixel(128, 128, image::Rgba([255, 0, 0, 255]));
        img.save(dir.join("tile-00.png"))
            .expect("Failed to save test tile PNG");

        let atlas = TileAtlas::build(&fs(&dir), Path::new(""))
            .expect("Expected Some atlas from directory with one tile");
        assert_eq!(atlas.width, 4096);
        assert_eq!(atlas.height, 4096);
        assert_eq!(atlas.tile_count, 1);

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
