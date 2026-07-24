//! Scatter mode: random object placement within a circular brush.

use wz_maplib::WzMap;
use wz_maplib::constants::TILE_UNITS;
use wz_maplib::objects::WorldPos;

use super::command::{ObjectAccum, StampCommand};
use super::pattern::StampPattern;
use super::transform::transform_object_direction;

/// Number of random candidate positions tried per target when `min_spacing_world > 0`
/// before giving up on that target. Higher means denser bursts at the cost of CPU.
const RETRIES_PER_TARGET: u32 = 6;

/// Scatter random objects sampled from the pattern within a circular brush.
///
/// Tiles are not stamped in scatter mode; it is an object-only brush. For each
/// placement, a random object is sampled from `pattern.objects` and placed at
/// a uniformly-distributed point inside a disk of radius `radius_tiles` tiles
/// centered on tile `(center_x_tile, center_y_tile)`.
///
/// The number of placements is `round(π · radius_tiles² · density)`, clamped
/// to at least 1 when `density > 0`. When `min_spacing_world > 0`, candidate
/// positions closer than `min_spacing_world` world units to any already-placed
/// position in this burst are rejected (up to a small retry budget). Returns
/// an empty command if `pattern.objects` is empty, `density <= 0.0`, or
/// `radius_tiles == 0`.
pub(super) fn apply_scatter(
    map: &mut WzMap,
    pattern: &StampPattern,
    center_x_tile: u32,
    center_y_tile: u32,
    radius_tiles: u32,
    density: f32,
    min_spacing_world: u32,
    random_rotation: bool,
    random_flip: bool,
    rng: &mut fastrand::Rng,
) -> StampCommand {
    if pattern.objects.is_empty() || density <= 0.0 || radius_tiles == 0 {
        return ObjectAccum::default().into_command(Vec::new());
    }

    let radius_world = (radius_tiles * TILE_UNITS) as f32;
    // WZ2100 tile centres sit at `tile*TILE + TILE/2`.
    let center_wx = (center_x_tile * TILE_UNITS + TILE_UNITS / 2) as f32;
    let center_wy = (center_y_tile * TILE_UNITS + TILE_UNITS / 2) as f32;

    let area_tiles_sq = std::f32::consts::PI * (radius_tiles as f32).powi(2);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "count is a small non-negative integer after round()"
    )]
    let target_count = (area_tiles_sq * density).round().max(1.0) as usize;

    let map_w_world = (map.map_data.width * TILE_UNITS) as f32;
    let map_h_world = (map.map_data.height * TILE_UNITS) as f32;

    let min_spacing_sq = (min_spacing_world as f32).powi(2);
    let retries = if min_spacing_world > 0 {
        RETRIES_PER_TARGET
    } else {
        1
    };

    let mut accum = ObjectAccum::default();
    let mut placed_positions: Vec<(f32, f32)> = Vec::with_capacity(target_count);

    for _ in 0..target_count {
        let mut chosen: Option<(f32, f32)> = None;
        for _ in 0..retries {
            // Uniform sample inside a disk: theta uniform in [0, 2pi), r = R*sqrt(u).
            let theta = rng.f32() * std::f32::consts::TAU;
            let r = radius_world * rng.f32().sqrt();
            let wx = center_wx + r * theta.cos();
            let wy = center_wy + r * theta.sin();

            if wx < 0.0 || wy < 0.0 || wx >= map_w_world || wy >= map_h_world {
                continue;
            }

            if min_spacing_world > 0 {
                let too_close = placed_positions.iter().any(|&(px, py)| {
                    let dx = wx - px;
                    let dy = wy - py;
                    dx * dx + dy * dy < min_spacing_sq
                });
                if too_close {
                    continue;
                }
            }

            chosen = Some((wx, wy));
            break;
        }

        let Some((wx, wy)) = chosen else {
            continue;
        };

        let idx = rng.usize(..pattern.objects.len());
        let sample = &pattern.objects[idx];

        let rot_steps = if random_rotation { rng.u8(..4) } else { 0 };
        let (flip_x, flip_y) = if random_flip {
            (rng.bool(), rng.bool())
        } else {
            (false, false)
        };

        let (_, _, _, base_dir) = sample.name_offset_dir();
        let new_dir = transform_object_direction(base_dir, rot_steps, flip_x, flip_y);

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bounds checked non-negative above; world coords fit in u32"
        )]
        let position = WorldPos {
            x: wx as u32,
            y: wy as u32,
        };

        accum.place(map, sample, position, new_dir);
        placed_positions.push((wx, wy));
    }

    accum.into_command(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::history::EditCommand;
    use crate::tools::stamp::pattern::{StampObject, StampTile};
    use wz_maplib::map_data::MapData;

    fn make_test_map(w: u32, h: u32) -> WzMap {
        WzMap {
            map_data: MapData::new(w, h),
            structures: Vec::new(),
            droids: Vec::new(),
            features: Vec::new(),
            terrain_types: None,
            labels: Vec::new(),
            map_name: String::new(),
            players: 0,
            tileset: String::new(),
            custom_templates_json: None,
            author: None,
            additional_authors: Vec::new(),
            license: None,
            created_date: None,
        }
    }

    fn tree_pattern() -> StampPattern {
        StampPattern {
            width: 1,
            height: 1,
            tiles: Vec::new(),
            objects: vec![StampObject::Feature {
                name: "Tree".into(),
                offset_x: 64,
                offset_y: 64,
                direction: 0,
                player: None,
            }],
        }
    }

    #[test]
    fn scatter_empty_pattern_is_noop() {
        let mut map = make_test_map(20, 20);
        let pattern = StampPattern {
            width: 1,
            height: 1,
            tiles: Vec::new(),
            objects: Vec::new(),
        };
        let mut rng = fastrand::Rng::with_seed(1);
        let cmd = apply_scatter(
            &mut map, &pattern, 10, 10, 3, 0.5, 0, false, false, &mut rng,
        );
        assert!(cmd.features.is_empty());
        assert!(cmd.tile_changes.is_empty());
        assert!(map.features.is_empty());
    }

    #[test]
    fn scatter_zero_density_is_noop() {
        let mut map = make_test_map(20, 20);
        let pattern = tree_pattern();
        let mut rng = fastrand::Rng::with_seed(1);
        let cmd = apply_scatter(
            &mut map, &pattern, 10, 10, 3, 0.0, 0, false, false, &mut rng,
        );
        assert!(cmd.features.is_empty());
        assert!(map.features.is_empty());
    }

    #[test]
    fn scatter_zero_radius_is_noop() {
        let mut map = make_test_map(20, 20);
        let pattern = tree_pattern();
        let mut rng = fastrand::Rng::with_seed(1);
        let cmd = apply_scatter(
            &mut map, &pattern, 10, 10, 0, 0.5, 0, false, false, &mut rng,
        );
        assert!(cmd.features.is_empty());
        assert!(map.features.is_empty());
    }

    #[test]
    fn scatter_does_not_stamp_tiles() {
        let mut map = make_test_map(20, 20);
        let pattern = StampPattern {
            width: 2,
            height: 2,
            tiles: vec![StampTile {
                dx: 0,
                dy: 0,
                texture: 42,
                height: 100,
            }],
            objects: vec![StampObject::Feature {
                name: "Tree".into(),
                offset_x: 64,
                offset_y: 64,
                direction: 0,
                player: None,
            }],
        };
        let mut rng = fastrand::Rng::with_seed(7);
        let cmd = apply_scatter(
            &mut map, &pattern, 10, 10, 4, 0.5, 0, false, false, &mut rng,
        );
        assert!(cmd.tile_changes.is_empty());
        assert_eq!(map.map_data.tile(10, 10).unwrap().texture, 0);
    }

    #[test]
    fn scatter_places_within_radius() {
        let mut map = make_test_map(40, 40);
        let pattern = tree_pattern();
        let (cx, cy) = (20u32, 20u32);
        let radius_tiles = 4u32;
        let mut rng = fastrand::Rng::with_seed(42);
        let cmd = apply_scatter(
            &mut map,
            &pattern,
            cx,
            cy,
            radius_tiles,
            1.0,
            0,
            false,
            false,
            &mut rng,
        );
        assert!(!cmd.features.is_empty(), "expected some features to place");

        let cx_w = (cx * TILE_UNITS + TILE_UNITS / 2) as f64;
        let cy_w = (cy * TILE_UNITS + TILE_UNITS / 2) as f64;
        let r_w = (radius_tiles * TILE_UNITS) as f64;
        // `wx as u32` can truncate up to 1 unit, so allow slack.
        let r_sq = (r_w + 1.0).powi(2);
        for f in &cmd.features {
            let dx = f.position.x as f64 - cx_w;
            let dy = f.position.y as f64 - cy_w;
            assert!(
                dx * dx + dy * dy <= r_sq,
                "feature at ({}, {}) outside radius",
                f.position.x,
                f.position.y
            );
        }
    }

    #[test]
    fn scatter_count_scales_with_density() {
        let pattern = tree_pattern();

        let mut map_low = make_test_map(40, 40);
        let mut rng_low = fastrand::Rng::with_seed(1);
        let low = apply_scatter(
            &mut map_low,
            &pattern,
            20,
            20,
            5,
            0.5,
            0,
            false,
            false,
            &mut rng_low,
        );

        let mut map_hi = make_test_map(40, 40);
        let mut rng_hi = fastrand::Rng::with_seed(1);
        let hi = apply_scatter(
            &mut map_hi,
            &pattern,
            20,
            20,
            5,
            1.0,
            0,
            false,
            false,
            &mut rng_hi,
        );

        assert!(
            hi.features.len() > low.features.len(),
            "higher density must produce more placements: low={} hi={}",
            low.features.len(),
            hi.features.len()
        );
    }

    #[test]
    fn scatter_undo_removes_all_added_objects() {
        let mut map = make_test_map(30, 30);
        let pattern = tree_pattern();
        let mut rng = fastrand::Rng::with_seed(11);
        let cmd = apply_scatter(
            &mut map, &pattern, 15, 15, 3, 0.8, 0, false, false, &mut rng,
        );
        let placed = cmd.features.len();
        assert!(placed > 0);
        assert_eq!(map.features.len(), placed);

        cmd.undo(&mut map);
        assert!(map.features.is_empty());
    }

    #[test]
    fn scatter_samples_all_object_types() {
        let mut map = make_test_map(40, 40);
        let pattern = StampPattern {
            width: 1,
            height: 1,
            tiles: Vec::new(),
            objects: vec![
                StampObject::Structure {
                    name: "Wall".into(),
                    offset_x: 64,
                    offset_y: 64,
                    direction: 0,
                    player: 0,
                    modules: 0,
                },
                StampObject::Droid {
                    name: "Truck".into(),
                    offset_x: 64,
                    offset_y: 64,
                    direction: 0,
                    player: 0,
                },
                StampObject::Feature {
                    name: "Tree".into(),
                    offset_x: 64,
                    offset_y: 64,
                    direction: 0,
                    player: None,
                },
            ],
        };
        let mut rng = fastrand::Rng::with_seed(99);
        let cmd = apply_scatter(
            &mut map, &pattern, 20, 20, 6, 1.0, 0, false, false, &mut rng,
        );
        let total = cmd.structures.len() + cmd.droids.len() + cmd.features.len();
        assert!(
            total >= 3,
            "expected at least a few placements, got {total}"
        );
    }

    #[test]
    fn scatter_min_spacing_enforces_distance() {
        let mut map = make_test_map(40, 40);
        let pattern = tree_pattern();
        let min_spacing = 128u32;
        let mut rng = fastrand::Rng::with_seed(42);
        let cmd = apply_scatter(
            &mut map,
            &pattern,
            20,
            20,
            6,
            1.0,
            min_spacing,
            false,
            false,
            &mut rng,
        );

        // `wx as u32` can truncate up to 1 unit per axis, so allow ~3 units of slack.
        let effective_min = (min_spacing.saturating_sub(3)) as f64;
        let effective_min_sq = effective_min * effective_min;
        for (i, a) in cmd.features.iter().enumerate() {
            for b in cmd.features.iter().skip(i + 1) {
                let dx = a.position.x as f64 - b.position.x as f64;
                let dy = a.position.y as f64 - b.position.y as f64;
                let d_sq = dx * dx + dy * dy;
                assert!(
                    d_sq >= effective_min_sq,
                    "features too close: ({}, {}) and ({}, {}), d²={d_sq} < {effective_min_sq}",
                    a.position.x,
                    a.position.y,
                    b.position.x,
                    b.position.y,
                );
            }
        }
    }

    #[test]
    fn scatter_random_rotation_varies_object_direction() {
        let mut map = make_test_map(40, 40);
        let pattern = tree_pattern();
        let mut rng = fastrand::Rng::with_seed(123);
        let cmd = apply_scatter(&mut map, &pattern, 20, 20, 5, 1.0, 0, true, false, &mut rng);
        assert!(cmd.features.len() > 4);
        let directions: std::collections::BTreeSet<u16> =
            cmd.features.iter().map(|f| f.direction).collect();
        assert!(
            directions.len() > 1,
            "random rotation must produce more than one distinct direction"
        );
    }
}
