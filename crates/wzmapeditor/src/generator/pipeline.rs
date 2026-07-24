//! Generation pipeline orchestrator. Runs every stage in sequence on a
//! background thread and reports progress to the UI via shared atomics.

use wz_maplib::constants::TILE_MAX_HEIGHT;
use wz_maplib::io_wz::WzMap;
use wz_maplib::map_data::MapTile;
use wz_maplib::terrain_types::TerrainTypeData;

use super::heightmap::Heightmap;
use super::{GeneratorConfig, GeneratorResult, ProgressReporter};
use crate::app::map_io::strip_player_count_prefix;

/// Run the full map generation pipeline. Entry point called from the
/// background thread.
#[expect(
    clippy::unnecessary_wraps,
    reason = "returns GeneratorResult so future failure paths can be added without changing the caller"
)]
pub(crate) fn generate_map(
    config: &GeneratorConfig,
    reporter: &ProgressReporter,
) -> GeneratorResult {
    let mut rng = if config.seed == 0 {
        fastrand::Rng::new()
    } else {
        fastrand::Rng::with_seed(config.seed)
    };

    // Stage order matters: terrain comes before placement so objects can
    // sample final heights, and texturing runs last so it sees water carved
    // by erosion.
    reporter.set("Building terrain layout", 0.0);
    let network = super::nodes::build_node_network(config, &mut rng);

    reporter.set("Assigning height levels", 0.10);
    let levels = super::terrain::assign_height_levels(&network, config, &mut rng);

    reporter.set("Generating terrain heights", 0.20);
    let mut heightmap = super::heightmap::generate(config, &network, &levels, &mut rng);

    reporter.set("Creating ramps", 0.50);
    super::terrain::carve_ramps(
        &mut heightmap.data,
        config.width,
        config.height,
        &network,
        &levels,
        config,
    );

    let max_h = TILE_MAX_HEIGHT as f32;
    for h in &mut heightmap.data {
        *h = h.clamp(0.0, max_h);
    }

    reporter.set("Building map data", 0.60);
    let mut map = build_map_data(config, &heightmap);

    reporter.set("Placing resources", 0.65);
    super::placement::place_resources(&mut map, config, &network, &levels, &mut rng);

    reporter.set("Scattering oil drums", 0.72);
    super::placement::place_oil_drums(&mut map, config, &network, &levels, &mut rng);

    reporter.set("Placing scavengers", 0.74);
    super::placement::place_scavengers(&mut map, config, &network, &levels, &mut rng);

    reporter.set("Placing player bases", 0.78);
    super::placement::place_player_starts(&mut map, config, &network);

    reporter.set("Applying textures", 0.80);
    super::texturing::auto_texture(&mut map, config);

    reporter.set("Done", 1.0);
    Ok(map)
}

/// Build the base `WzMap` from the generated heightmap.
fn build_map_data(config: &GeneratorConfig, heightmap: &Heightmap) -> WzMap {
    let mut map = WzMap::new(
        strip_player_count_prefix(&config.map_name),
        config.width,
        config.height,
    );
    map.tileset = config.tileset.as_str().to_string();
    map.players = config.players;
    map.terrain_types = Some(TerrainTypeData {
        terrain_types: config.tileset.default_terrain_types(),
    });

    for y in 0..config.height {
        for x in 0..config.width {
            let h = heightmap.get(x, y).clamp(0.0, TILE_MAX_HEIGHT as f32) as u16;
            if let Some(tile) = map.map_data.tile_mut(x, y) {
                tile.height = h;
            }
        }
    }

    // Pick the lower-slope diagonal for each tile's triangle split so the
    // shared edge runs along the gentler ridge.
    for y in 0..config.height {
        for x in 0..config.width {
            let h00 = heightmap.get(x, y);
            let h10 = heightmap.get(x + 1, y);
            let h01 = heightmap.get(x, y + 1);
            let h11 = heightmap.get(x + 1, y + 1);

            let diag1 = (h00 - h11).abs();
            let diag2 = (h10 - h01).abs();

            let tri_flip = diag2 < diag1;

            if let Some(tile) = map.map_data.tile_mut(x, y) {
                let tex_id = tile.texture & wz_maplib::constants::TILE_NUMMASK;
                tile.texture = MapTile::make_texture(
                    tex_id,
                    tile.x_flip(),
                    tile.y_flip(),
                    tile.rotation(),
                    tri_flip,
                );
            }
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Tileset;
    use crate::tools::MirrorMode;
    use std::sync::atomic::AtomicU32;
    use std::sync::{Arc, Mutex};

    fn noop_reporter() -> ProgressReporter {
        ProgressReporter::new(
            Arc::new(AtomicU32::new(0)),
            Arc::new(Mutex::new(String::new())),
        )
    }

    fn default_config() -> GeneratorConfig {
        GeneratorConfig {
            seed: 42,
            ..GeneratorConfig::default()
        }
    }

    /// Print heightmap stats for several seeds so we can compare against
    /// real WZ2100 maps. Run with:
    ///   `cargo test -p wzmapeditor print_generator_stats -- --ignored --nocapture`
    #[test]
    #[ignore = "diagnostic-only; prints stats and is opt-in"]
    fn print_generator_stats() {
        let seeds = [1u64, 7, 42, 99, 777];
        println!();
        for seed in seeds {
            let config = GeneratorConfig {
                seed,
                width: 128,
                height: 128,
                ..GeneratorConfig::default()
            };
            let reporter = noop_reporter();
            let map = generate_map(&config, &reporter).unwrap();

            let heights: Vec<f32> = map.map_data.tiles.iter().map(|t| t.height as f32).collect();
            let n = heights.len() as f32;
            let min = heights.iter().copied().fold(f32::MAX, f32::min);
            let max = heights.iter().copied().fold(f32::MIN, f32::max);
            let mean = heights.iter().sum::<f32>() / n;
            let variance = heights.iter().map(|h| (h - mean).powi(2)).sum::<f32>() / n;
            let std_dev = variance.sqrt();

            let w = map.map_data.width as usize;
            let h = map.map_data.height as usize;
            let mut slopes = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let cur = heights[y * w + x];
                    if x + 1 < w {
                        slopes.push((cur - heights[y * w + x + 1]).abs());
                    }
                    if y + 1 < h {
                        slopes.push((cur - heights[(y + 1) * w + x]).abs());
                    }
                }
            }
            let max_slope = slopes.iter().copied().fold(f32::MIN, f32::max);
            let avg_slope = slopes.iter().sum::<f32>() / slopes.len() as f32;
            let steep_count = slopes.iter().filter(|&&s| s > 80.0).count();
            let steep_pct = steep_count as f32 / slopes.len() as f32 * 100.0;

            let bucket_size = (max / 10.0).max(1.0);
            let mut hist = [0u32; 10];
            for &h in &heights {
                let b = (h / bucket_size) as usize;
                hist[b.min(9)] += 1;
            }
            let hist_str: String = hist
                .iter()
                .map(|&c| format!("{:.0}", c as f32 / n * 100.0))
                .collect::<Vec<_>>()
                .join(",");

            println!(
                "seed={seed:<5} {:3}x{:<3}  h: {min:5.0}..{max:5.0}  \
                 mean={mean:6.1}  stddev={std_dev:5.1}  \
                 slope: avg={avg_slope:4.1} max={max_slope:4.0} \
                 steep%={steep_pct:4.1}  hist=[{hist_str}]",
                map.map_data.width, map.map_data.height,
            );
        }
    }

    #[test]
    fn test_full_generation_default_config() {
        let config = default_config();
        let reporter = noop_reporter();
        let result = generate_map(&config, &reporter);

        assert!(result.is_ok(), "Generation failed: {:?}", result.err());
        let map = result.unwrap();

        assert_eq!(map.map_data.width, 128);
        assert_eq!(map.map_data.height, 128);
        assert_eq!(map.players, 2);
        assert_eq!(map.tileset, "arizona");
    }

    #[test]
    fn test_full_generation_has_structures() {
        let config = default_config();
        let reporter = noop_reporter();
        let map = generate_map(&config, &reporter).unwrap();

        assert!(
            map.structures.len() >= config.players as usize * 3,
            "Expected at least {} structures, got {}",
            config.players as usize * 3,
            map.structures.len()
        );
    }

    #[test]
    fn test_full_generation_has_droids() {
        let config = GeneratorConfig {
            trucks_per_player: 4,
            seed: 42,
            ..GeneratorConfig::default()
        };
        let reporter = noop_reporter();
        let map = generate_map(&config, &reporter).unwrap();

        assert!(
            map.droids.len() >= config.players as usize * config.trucks_per_player as usize,
            "Expected at least {} droids, got {}",
            config.players as usize * config.trucks_per_player as usize,
            map.droids.len()
        );
    }

    #[test]
    fn test_full_generation_has_features() {
        let config = GeneratorConfig {
            base_oil: 4,
            extra_oil: 6,
            seed: 42,
            ..GeneratorConfig::default()
        };
        let reporter = noop_reporter();
        let map = generate_map(&config, &reporter).unwrap();

        let oil_count = map
            .features
            .iter()
            .filter(|f| f.name == "OilResource")
            .count();
        assert!(oil_count > 0, "No oil resources placed");
    }

    #[test]
    fn test_full_generation_heights_valid() {
        let config = default_config();
        let reporter = noop_reporter();
        let map = generate_map(&config, &reporter).unwrap();

        let max = TILE_MAX_HEIGHT;
        for (i, tile) in map.map_data.tiles.iter().enumerate() {
            assert!(
                tile.height <= max,
                "Tile {i} height {} exceeds max {max}",
                tile.height
            );
        }
    }

    #[test]
    fn test_full_generation_textures_applied() {
        let config = default_config();
        let reporter = noop_reporter();
        let map = generate_map(&config, &reporter).unwrap();

        let textured = map
            .map_data
            .tiles
            .iter()
            .filter(|t| t.texture_id() != 0)
            .count();
        let total = map.map_data.tiles.len();
        let ratio = textured as f32 / total as f32;
        assert!(
            ratio > 0.5,
            "Only {:.1}% tiles textured, expected >50%",
            ratio * 100.0
        );
    }

    #[test]
    fn test_deterministic_same_seed() {
        let config = GeneratorConfig {
            seed: 123,
            ..GeneratorConfig::default()
        };

        let r1 = noop_reporter();
        let map1 = generate_map(&config, &r1).unwrap();

        let r2 = noop_reporter();
        let map2 = generate_map(&config, &r2).unwrap();

        for (i, (t1, t2)) in map1
            .map_data
            .tiles
            .iter()
            .zip(map2.map_data.tiles.iter())
            .enumerate()
        {
            assert_eq!(
                t1.height, t2.height,
                "Height mismatch at tile {i}: {} vs {}",
                t1.height, t2.height
            );
            assert_eq!(
                t1.texture, t2.texture,
                "Texture mismatch at tile {i}: {} vs {}",
                t1.texture, t2.texture
            );
        }

        assert_eq!(map1.structures.len(), map2.structures.len());
        assert_eq!(map1.features.len(), map2.features.len());
        assert_eq!(map1.droids.len(), map2.droids.len());
    }

    #[test]
    fn test_different_seeds_produce_different_maps() {
        let r1 = noop_reporter();
        let map1 = generate_map(
            &GeneratorConfig {
                seed: 1,
                ..GeneratorConfig::default()
            },
            &r1,
        )
        .unwrap();

        let r2 = noop_reporter();
        let map2 = generate_map(
            &GeneratorConfig {
                seed: 2,
                ..GeneratorConfig::default()
            },
            &r2,
        )
        .unwrap();

        let diff_count = map1
            .map_data
            .tiles
            .iter()
            .zip(map2.map_data.tiles.iter())
            .filter(|(t1, t2)| t1.height != t2.height)
            .count();
        assert!(
            diff_count > 0,
            "Different seeds should produce different maps"
        );
    }

    #[test]
    fn test_various_player_counts() {
        for players in [2u8, 4, 6, 8, 10] {
            let config = GeneratorConfig {
                players,
                symmetry: MirrorMode::None,
                seed: 42,
                ..GeneratorConfig::default()
            };
            let reporter = noop_reporter();
            let result = generate_map(&config, &reporter);
            assert!(
                result.is_ok(),
                "Generation failed for {players} players: {:?}",
                result.err()
            );
            let map = result.unwrap();
            assert_eq!(map.players, players);
        }
    }

    #[test]
    fn test_various_symmetry_modes() {
        for mode in [
            MirrorMode::None,
            MirrorMode::Vertical,
            MirrorMode::Horizontal,
            MirrorMode::Both,
            MirrorMode::Central,
            MirrorMode::Diagonal,
        ] {
            let config = GeneratorConfig {
                symmetry: mode,
                seed: 42,
                ..GeneratorConfig::default()
            };
            let reporter = noop_reporter();
            let result = generate_map(&config, &reporter);
            assert!(
                result.is_ok(),
                "Generation failed for symmetry {mode:?}: {:?}",
                result.err()
            );
        }
    }

    #[test]
    fn test_diagonal_symmetry_square_map() {
        let config = GeneratorConfig {
            width: 128,
            height: 128,
            symmetry: MirrorMode::Diagonal,
            players: 4,
            seed: 42,
            ..GeneratorConfig::default()
        };
        let reporter = noop_reporter();
        let result = generate_map(&config, &reporter);
        assert!(
            result.is_ok(),
            "Diagonal symmetry failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_all_tilesets() {
        for tileset in [Tileset::Arizona, Tileset::Urban, Tileset::Rockies] {
            let config = GeneratorConfig {
                tileset,
                seed: 42,
                ..GeneratorConfig::default()
            };
            let reporter = noop_reporter();
            let result = generate_map(&config, &reporter);
            assert!(
                result.is_ok(),
                "Generation failed for tileset {tileset:?}: {:?}",
                result.err()
            );
            let map = result.unwrap();
            assert_eq!(map.tileset, tileset.as_str());
        }
    }

    #[test]
    fn test_small_map() {
        let config = GeneratorConfig {
            width: 48,
            height: 48,
            seed: 42,
            ..GeneratorConfig::default()
        };
        let reporter = noop_reporter();
        let result = generate_map(&config, &reporter);
        assert!(result.is_ok(), "Small map failed: {:?}", result.err());
    }

    #[test]
    fn test_large_map() {
        let config = GeneratorConfig {
            width: 250,
            height: 250,
            seed: 42,
            ..GeneratorConfig::default()
        };
        let reporter = noop_reporter();
        let result = generate_map(&config, &reporter);
        assert!(result.is_ok(), "Large map failed: {:?}", result.err());
    }

    #[test]
    fn test_objects_within_bounds() {
        let config = default_config();
        let reporter = noop_reporter();
        let map = generate_map(&config, &reporter).unwrap();

        let max_x = config.width * wz_maplib::constants::TILE_UNITS;
        let max_y = config.height * wz_maplib::constants::TILE_UNITS;

        for (i, s) in map.structures.iter().enumerate() {
            assert!(
                s.position.x < max_x && s.position.y < max_y,
                "Structure {i} '{}' at ({},{}) out of bounds (max {max_x},{max_y})",
                s.name,
                s.position.x,
                s.position.y
            );
        }
        for (i, d) in map.droids.iter().enumerate() {
            assert!(
                d.position.x < max_x && d.position.y < max_y,
                "Droid {i} '{}' at ({},{}) out of bounds",
                d.name,
                d.position.x,
                d.position.y
            );
        }
        for (i, f) in map.features.iter().enumerate() {
            assert!(
                f.position.x < max_x && f.position.y < max_y,
                "Feature {i} '{}' at ({},{}) out of bounds",
                f.name,
                f.position.x,
                f.position.y
            );
        }
    }

    #[test]
    fn test_progress_updates() {
        let progress = Arc::new(AtomicU32::new(0));
        let label = Arc::new(Mutex::new(String::new()));
        let reporter = ProgressReporter::new(progress.clone(), label.clone());

        let config = default_config();
        let _ = generate_map(&config, &reporter);

        assert_eq!(
            progress.load(std::sync::atomic::Ordering::Relaxed),
            1000,
            "Progress should be 1000 after completion"
        );
        assert_eq!(
            *label.lock().unwrap(),
            "Done",
            "Label should be 'Done' after completion"
        );
    }
}
