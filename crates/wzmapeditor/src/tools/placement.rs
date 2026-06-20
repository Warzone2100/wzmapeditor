//! Snap, validate, and wall-replace helpers for object placement.
//!
//! Pure functions over the map, stats, and placement parameters. Living
//! under `tools` lets the `ObjectPlace` and Wall tools share them without
//! going through the viewport module.

use crate::map::history::{CompoundCommand, EditCommand};
use crate::tools::object_edit::{DeleteObjectCommand, PlaceFeatureCommand, PlaceStructureCommand};

use wz_maplib::constants::{MAX_INCLINE, TILE_MASK, TILE_UNITS as TILE_UNITS_U32, TOO_NEAR_EDGE};
use wz_maplib::objects::{Feature, Structure};
use wz_maplib::validate::{is_wall_or_defense, structure_packability};

/// WZ2100 stat `type` of an oil derrick; valid only on an oil-resource tile.
const RESOURCE_EXTRACTOR_TYPE: &str = "RESOURCE EXTRACTOR";
/// WZ2100 stat `type` of the oil-resource feature a derrick is built on.
const OIL_RESOURCE_TYPE: &str = "OIL RESOURCE";
/// Canonical stat id for the oil-resource feature paired with a derrick.
const OIL_RESOURCE_NAME: &str = "OilResource";

/// Existing-tile structure types that a wall-combining tower can land on
/// and replace. Restricted to the types `build_placement_with_wall_replace`
/// actually deletes, so we never approve a stack we can't clean up.
fn is_wall_tile_type(stype: Option<&str>) -> bool {
    matches!(stype, Some("WALL" | "CORNER WALL"))
}

/// Per-tile occupancy info used by the placement overlap and spacing checks.
#[derive(Clone, Copy)]
struct TileOccupancy {
    packability: u8,
    is_wall: bool,
}

/// Snap a world position for building placement, matching WZ2100's grid logic.
///
/// Even-sized buildings (2x2) snap to tile corners (multiples of 128).
/// Odd-sized (1x1, 3x3) snap to tile centers (multiples of 128 + 64).
/// Direction snaps to the nearest 90°, with width/breadth swapped for
/// 90/270° rotations.
fn snap_building_position(
    world_x: u32,
    world_z: u32,
    width: u32,
    breadth: u32,
    direction: u16,
) -> (u32, u32, u16) {
    let snap_dir = direction.wrapping_add(0x2000) & 0xC000;

    let (size_x, size_z) = if snap_dir == 0x4000 || snap_dir == 0xC000 {
        (breadth, width)
    } else {
        (width, breadth)
    };

    // WZ2100: pos = (pos & ~TILE_MASK) + (size % 2) * (TILE_UNITS / 2).
    let sx = (world_x & !TILE_MASK) + (size_x % 2) * (TILE_UNITS_U32 / 2);
    let sz = (world_z & !TILE_MASK) + (size_z % 2) * (TILE_UNITS_U32 / 2);

    (sx, sz, snap_dir)
}

/// Look up a structure's width/breadth from stats, defaulting to 1.
fn get_structure_size(stats: Option<&wz_stats::StatsDatabase>, name: &str) -> Option<(u32, u32)> {
    let s = stats?.structures.get(name)?;
    Some((s.width.unwrap_or(1), s.breadth.unwrap_or(1)))
}

/// Look up a feature's width/breadth from stats, defaulting to 1.
fn get_feature_size(stats: Option<&wz_stats::StatsDatabase>, name: &str) -> Option<(u32, u32)> {
    let f = stats?.features.get(name)?;
    Some((f.width.unwrap_or(1), f.breadth.unwrap_or(1)))
}

/// Snap a feature or droid position to the nearest half-tile (64-unit grid).
/// Allows placement at both tile centers and corners, matching WZ2100
/// behaviour where features are not restricted to tile centers.
fn snap_half_tile(world_x: u32, world_z: u32) -> (u32, u32) {
    let half = TILE_UNITS_U32 / 2;
    let sx = ((world_x + half / 2) / half) * half;
    let sz = ((world_z + half / 2) / half) * half;
    (sx, sz)
}

/// Snap the placement position based on the currently selected asset type.
///
/// Structures use their width/breadth for grid alignment; features and
/// droids snap to the nearest half-tile (tile centers or corners).
/// Returns `(world_x, world_z)` unchanged when no object is selected.
pub fn snap_placement_pos(
    stats: Option<&wz_stats::StatsDatabase>,
    placement_object: Option<&str>,
    placement_direction: u16,
    world_x: u32,
    world_z: u32,
) -> (u32, u32) {
    let Some(name) = placement_object else {
        return (world_x, world_z);
    };

    if let Some((w, b)) = get_structure_size(stats, name) {
        let (sx, sz, _) = snap_building_position(world_x, world_z, w, b, placement_direction);
        (sx, sz)
    } else if let Some((w, b)) = get_feature_size(stats, name) {
        let (sx, sz, _) = snap_building_position(world_x, world_z, w, b, placement_direction);
        (sx, sz)
    } else {
        snap_half_tile(world_x, world_z)
    }
}

/// Validate whether a structure can be placed at the given snapped world position.
///
/// Mirrors WZ2100's `validLocation()`:
/// - Map edge buffer (3 tiles)
/// - Water/cliff terrain types
/// - Terrain slope (max 50 units height difference per tile)
/// - Tile overlap with existing structures
/// - 1-tile spacing between non-packable buildings
///
/// `force` skips overlap checks for features/droids (shift-place override).
pub fn validate_placement(
    map: &wz_maplib::WzMap,
    stats: Option<&wz_stats::StatsDatabase>,
    placement_object: Option<&str>,
    placement_direction: u16,
    world_x: u32,
    world_z: u32,
    force: bool,
) -> bool {
    let map_data = &map.map_data;

    let Some(obj_name) = placement_object else {
        return false;
    };

    // Features/droids: bounds + overlap (skip overlap when forced).
    let Some(struct_stats) = stats.and_then(|s| s.structures.get(obj_name)) else {
        let tile_x = world_x >> 7;
        let tile_z = world_z >> 7;
        let in_bounds = tile_x >= TOO_NEAR_EDGE
            && tile_z >= TOO_NEAR_EDGE
            && tile_x < map_data.width.saturating_sub(TOO_NEAR_EDGE)
            && tile_z < map_data.height.saturating_sub(TOO_NEAR_EDGE);
        if !in_bounds {
            return false;
        }
        if !force {
            let half_tile = TILE_UNITS_U32 / 2;
            for f in &map.features {
                let dx = world_x.abs_diff(f.position.x);
                let dz = world_z.abs_diff(f.position.y);
                if dx < half_tile && dz < half_tile {
                    return false;
                }
            }
            for d in &map.droids {
                let dx = world_x.abs_diff(d.position.x);
                let dz = world_z.abs_diff(d.position.y);
                if dx < half_tile && dz < half_tile {
                    return false;
                }
            }
        }
        return true;
    };

    let (width, breadth) = (
        struct_stats.width.unwrap_or(1),
        struct_stats.breadth.unwrap_or(1),
    );
    let stype = struct_stats.structure_type.as_deref();

    let snap_dir = placement_direction.wrapping_add(0x2000) & 0xC000;
    let (size_x, size_z) = if snap_dir == 0x4000 || snap_dir == 0xC000 {
        (breadth, width)
    } else {
        (width, breadth)
    };

    let center_tx = world_x >> 7;
    let center_tz = world_z >> 7;
    let top_x = center_tx.saturating_sub(size_x / 2);
    let top_z = center_tz.saturating_sub(size_z / 2);

    if top_x < TOO_NEAR_EDGE
        || top_z < TOO_NEAR_EDGE
        || top_x + size_x > map_data.width.saturating_sub(TOO_NEAR_EDGE)
        || top_z + size_z > map_data.height.saturating_sub(TOO_NEAR_EDGE)
    {
        return false;
    }

    // A resource extractor skips the terrain/slope/spacing/overlap checks
    // that apply to other buildings (WZ2100 structure.cpp `validLocation`,
    // dedicated `REF_RESOURCE_EXTRACTOR` case). Placement adds the oil-resource
    // feature underneath, so it needs no pre-existing oil here.
    if stype == Some(RESOURCE_EXTRACTOR_TYPE) {
        return true;
    }

    let ttp = map.terrain_types.as_ref();

    let terrain_type_at = |tx: u32, tz: u32| -> Option<wz_maplib::TerrainType> {
        let tile = map_data.tile(tx, tz)?;
        let tex_id = tile.texture_id() as usize;
        let ttp_data = ttp?;
        ttp_data.terrain_types.get(tex_id).copied()
    };

    let tile_max_min = |tx: u32, tz: u32| -> (u16, u16) {
        let mut min_h = u16::MAX;
        let mut max_h = 0u16;
        for dz in 0..=1 {
            for dx in 0..=1 {
                let h = map_data.tile(tx + dx, tz + dz).map_or(0, |t| t.height);
                min_h = min_h.min(h);
                max_h = max_h.max(h);
            }
        }
        (max_h, min_h)
    };

    let exempt = is_wall_or_defense(Some(stype.unwrap_or("")));

    for tz in top_z..top_z + size_z {
        for tx in top_x..top_x + size_x {
            if let Some(tt) = terrain_type_at(tx, tz) {
                if tt == wz_maplib::TerrainType::Water {
                    return false;
                }
                if tt == wz_maplib::TerrainType::Cliffface && !exempt {
                    return false;
                }
            }
            if !exempt {
                let (max_h, min_h) = tile_max_min(tx, tz);
                if max_h - min_h > MAX_INCLINE {
                    return false;
                }
            }
        }
    }

    let pack_this = structure_packability(Some(stype.unwrap_or("")));
    let incoming_combines_with_wall = struct_stats.combines_with_wall;

    let occupied = build_occupancy_set(map, stats);

    for tz in top_z..top_z + size_z {
        for tx in top_x..top_x + size_x {
            if let Some(occ) = occupied.get(&(tx, tz)) {
                if incoming_combines_with_wall && occ.is_wall {
                    continue;
                }
                return false;
            }
        }
    }

    let border_x0 = top_x.saturating_sub(1);
    let border_z0 = top_z.saturating_sub(1);
    let border_x1 = (top_x + size_x + 1).min(map_data.width);
    let border_z1 = (top_z + size_z + 1).min(map_data.height);

    for tz in border_z0..border_z1 {
        for tx in border_x0..border_x1 {
            if tx >= top_x && tx < top_x + size_x && tz >= top_z && tz < top_z + size_z {
                continue;
            }
            if let Some(occ) = occupied.get(&(tx, tz))
                && (pack_this as u16 + occ.packability as u16) > 3
            {
                return false;
            }
        }
    }

    true
}

fn is_resource_extractor(stats: Option<&wz_stats::StatsDatabase>, name: &str) -> bool {
    stats
        .and_then(|s| s.structures.get(name))
        .and_then(|st| st.structure_type.as_deref())
        == Some(RESOURCE_EXTRACTOR_TYPE)
}

fn is_oil_resource(stats: Option<&wz_stats::StatsDatabase>, name: &str) -> bool {
    stats
        .and_then(|s| s.features.get(name))
        .and_then(|f| f.feature_type.as_deref())
        == Some(OIL_RESOURCE_TYPE)
}

/// Index of an oil-resource feature occupying tile `(tx, tz)`, if any.
fn oil_resource_index_at(
    map: &wz_maplib::WzMap,
    stats: Option<&wz_stats::StatsDatabase>,
    tx: u32,
    tz: u32,
) -> Option<usize> {
    map.features.iter().position(|f| {
        (f.position.x >> 7) == tx && (f.position.y >> 7) == tz && is_oil_resource(stats, &f.name)
    })
}

/// Build the placement command for a structure, applying WZ2100's special
/// cases: a resource extractor is paired with an oil-resource feature, and a
/// wall-combining tower replaces a wall underneath it.
pub(crate) fn build_structure_placement(
    map: &wz_maplib::WzMap,
    stats: Option<&wz_stats::StatsDatabase>,
    structure: Structure,
) -> Box<dyn EditCommand> {
    if is_resource_extractor(stats, &structure.name) {
        let tx = structure.position.x >> 7;
        let tz = structure.position.y >> 7;
        // Shipped maps pair every derrick with an oil-resource feature on the
        // same tile; the game removes the feature at runtime. Keep an existing
        // oil and add one when missing so the derrick always has its oil.
        if oil_resource_index_at(map, stats, tx, tz).is_some() {
            return Box::new(PlaceStructureCommand { structure });
        }
        let oil = Feature {
            name: OIL_RESOURCE_NAME.to_string(),
            position: structure.position,
            direction: 0,
            id: None,
            player: None,
        };
        return Box::new(CompoundCommand::new(vec![
            Box::new(PlaceFeatureCommand { feature: oil }),
            Box::new(PlaceStructureCommand { structure }),
        ]));
    }
    build_placement_with_wall_replace(map, stats, structure)
}

/// Build a placement command that also removes any plain wall underneath
/// a wall-combining structure (tower, hardpoint, gate). Used by the
/// Object Place tool so towers landing on a wall replace it instead of
/// stacking.
///
/// Returns a bare `PlaceStructureCommand` when no wall is underneath.
pub(crate) fn build_placement_with_wall_replace(
    map: &wz_maplib::WzMap,
    stats: Option<&wz_stats::StatsDatabase>,
    structure: Structure,
) -> Box<dyn EditCommand> {
    let Some(stats) = stats else {
        return Box::new(PlaceStructureCommand { structure });
    };
    let Some(incoming_stat) = stats.structures.get(&structure.name) else {
        return Box::new(PlaceStructureCommand { structure });
    };
    if !incoming_stat.combines_with_wall {
        return Box::new(PlaceStructureCommand { structure });
    }

    let footprint = structure_footprint_tiles(&structure, stats);

    let mut wall_indices: Vec<usize> = map
        .structures
        .iter()
        .enumerate()
        .filter_map(|(idx, s)| {
            let stype = stats
                .structures
                .get(&s.name)
                .and_then(|st| st.structure_type.as_deref())?;
            if !matches!(stype, "WALL" | "CORNER WALL") {
                return None;
            }
            let other = structure_footprint_tiles(s, stats);
            if footprint.iter().any(|t| other.contains(t)) {
                Some(idx)
            } else {
                None
            }
        })
        .collect();

    if wall_indices.is_empty() {
        return Box::new(PlaceStructureCommand { structure });
    }

    // Delete from highest index first so earlier indices stay valid;
    // undo re-inserts back-to-front.
    wall_indices.sort_unstable_by(|a, b| b.cmp(a));

    let mut commands: Vec<Box<dyn EditCommand>> = Vec::with_capacity(wall_indices.len() + 1);
    for idx in wall_indices {
        let saved = map.structures[idx].clone();
        commands.push(Box::new(DeleteObjectCommand::structure(idx, saved)));
    }
    commands.push(Box::new(PlaceStructureCommand { structure }));
    Box::new(CompoundCommand::new(commands))
}

fn structure_footprint_tiles(s: &Structure, stats: &wz_stats::StatsDatabase) -> Vec<(u32, u32)> {
    let (w, b) = stats.structures.get(&s.name).map_or((1, 1), |st| {
        (st.width.unwrap_or(1), st.breadth.unwrap_or(1))
    });
    let snap_dir = s.direction.wrapping_add(0x2000) & 0xC000;
    let (sx, sz) = if snap_dir == 0x4000 || snap_dir == 0xC000 {
        (b, w)
    } else {
        (w, b)
    };
    let cx = s.position.x >> 7;
    let cz = s.position.y >> 7;
    let ox = cx.saturating_sub(sx / 2);
    let oz = cz.saturating_sub(sz / 2);
    let mut tiles = Vec::with_capacity((sx * sz) as usize);
    for tz in oz..oz + sz {
        for tx in ox..ox + sx {
            tiles.push((tx, tz));
        }
    }
    tiles
}

fn build_occupancy_set(
    map: &wz_maplib::WzMap,
    stats: Option<&wz_stats::StatsDatabase>,
) -> std::collections::HashMap<(u32, u32), TileOccupancy> {
    let mut occupied = std::collections::HashMap::new();

    for s in &map.structures {
        let ss = stats.and_then(|st| st.structures.get(&s.name));
        let (w, b) = ss.map_or((1, 1), |st| {
            (st.width.unwrap_or(1), st.breadth.unwrap_or(1))
        });
        let stype = ss.and_then(|st| st.structure_type.as_deref());
        let pack = structure_packability(stype);
        let is_wall = is_wall_tile_type(stype);

        let snap_dir = s.direction.wrapping_add(0x2000) & 0xC000;
        let (sx, sz) = if snap_dir == 0x4000 || snap_dir == 0xC000 {
            (b, w)
        } else {
            (w, b)
        };

        let cx = s.position.x >> 7;
        let cz = s.position.y >> 7;
        let ox = cx.saturating_sub(sx / 2);
        let oz = cz.saturating_sub(sz / 2);

        for tz in oz..oz + sz {
            for tx in ox..ox + sx {
                occupied.insert(
                    (tx, tz),
                    TileOccupancy {
                        packability: pack,
                        is_wall,
                    },
                );
            }
        }
    }

    // Features occupy a single tile (e.g. oil resources block placement).
    for f in &map.features {
        let fx = f.position.x >> 7;
        let fz = f.position.y >> 7;
        occupied.insert(
            (fx, fz),
            TileOccupancy {
                packability: 2,
                is_wall: false,
            },
        );
    }

    occupied
}

#[cfg(test)]
mod tests {
    use super::*;
    use wz_maplib::constants::TILE_UNITS;
    use wz_maplib::objects::WorldPos;
    use wz_stats::StatsDatabase;
    use wz_stats::features::FeatureStats;
    use wz_stats::structures::StructureStats;

    fn stats_with_oil_resource() -> StatsDatabase {
        let mut db = StatsDatabase::default();
        db.features.insert(
            "OilResource".to_string(),
            FeatureStats {
                id: "OilResource".to_string(),
                name: "Oil Resource".to_string(),
                feature_type: Some("OIL RESOURCE".to_string()),
                hitpoints: None,
                armour: None,
                model: None,
                imd_name: None,
                base_imd: None,
                line_of_sight: None,
                start_visible: None,
                width: Some(1),
                breadth: Some(1),
            },
        );
        db
    }

    #[test]
    fn snap_placement_pos_centers_one_by_one_feature_on_tile() {
        let stats = stats_with_oil_resource();
        let half = TILE_UNITS / 2;

        for cursor_x in [0u32, 10, 63, 64, 100, 127, 128, 200, 1000] {
            for cursor_z in [0u32, 10, 63, 64, 100, 127, 128, 200, 1000] {
                let (sx, sz) =
                    snap_placement_pos(Some(&stats), Some("OilResource"), 0, cursor_x, cursor_z);
                assert_eq!(
                    (sx % TILE_UNITS, sz % TILE_UNITS),
                    (half, half),
                    "cursor ({cursor_x}, {cursor_z}) snapped to ({sx}, {sz})",
                );
            }
        }
    }

    fn stats_with_oil_and_extractor() -> StatsDatabase {
        let mut db = stats_with_oil_resource();
        db.structures.insert(
            "A0ResourceExtractor".to_string(),
            StructureStats {
                id: "A0ResourceExtractor".to_string(),
                name: "Oil Derrick".to_string(),
                structure_type: Some(RESOURCE_EXTRACTOR_TYPE.to_string()),
                width: Some(1),
                breadth: Some(1),
                ..Default::default()
            },
        );
        db
    }

    fn tile_center(tx: u32, tz: u32) -> WorldPos {
        let half = TILE_UNITS / 2;
        WorldPos {
            x: tx * TILE_UNITS + half,
            y: tz * TILE_UNITS + half,
        }
    }

    fn oil_resource(pos: WorldPos) -> Feature {
        Feature {
            name: OIL_RESOURCE_NAME.to_string(),
            position: pos,
            direction: 0,
            id: None,
            player: None,
        }
    }

    fn derrick(pos: WorldPos) -> Structure {
        Structure {
            name: "A0ResourceExtractor".to_string(),
            position: pos,
            direction: 0,
            player: 0,
            modules: 0,
            id: None,
        }
    }

    #[test]
    fn resource_extractor_valid_on_or_off_oil() {
        let stats = stats_with_oil_and_extractor();
        let mut map = wz_maplib::WzMap::new("Test", 32, 32);
        let oil_pos = tile_center(8, 8);
        map.features.push(oil_resource(oil_pos));

        let valid = |pos: WorldPos| {
            validate_placement(
                &map,
                Some(&stats),
                Some("A0ResourceExtractor"),
                0,
                pos.x,
                pos.y,
                false,
            )
        };

        assert!(valid(oil_pos), "valid on an existing oil-resource tile");
        assert!(
            valid(tile_center(12, 12)),
            "also valid off oil; placement adds the oil underneath"
        );
        assert!(
            !valid(tile_center(0, 0)),
            "still rejected inside the map-edge buffer"
        );
    }

    #[test]
    fn placing_extractor_on_oil_keeps_it() {
        let stats = stats_with_oil_and_extractor();
        let mut map = wz_maplib::WzMap::new("Test", 32, 32);
        let pos = tile_center(8, 8);
        map.features.push(oil_resource(pos));

        let cmd = build_structure_placement(&map, Some(&stats), derrick(pos));
        cmd.execute(&mut map);
        assert_eq!(map.structures.len(), 1);
        assert_eq!(
            map.features.len(),
            1,
            "the oil resource is kept alongside the derrick"
        );

        cmd.undo(&mut map);
        assert!(map.structures.is_empty());
        assert_eq!(map.features.len(), 1, "undo leaves the pre-existing oil");
    }

    #[test]
    fn placing_extractor_without_oil_adds_it() {
        let stats = stats_with_oil_and_extractor();
        let mut map = wz_maplib::WzMap::new("Test", 32, 32);
        let pos = tile_center(8, 8);

        let cmd = build_structure_placement(&map, Some(&stats), derrick(pos));
        cmd.execute(&mut map);
        assert_eq!(map.structures.len(), 1);
        assert_eq!(map.features.len(), 1, "an oil resource is added under it");
        assert_eq!(map.features[0].name, OIL_RESOURCE_NAME);
        assert_eq!(map.features[0].position.x, pos.x);
        assert_eq!(map.features[0].position.y, pos.y);

        cmd.undo(&mut map);
        assert!(map.structures.is_empty(), "undo removes the derrick");
        assert!(map.features.is_empty(), "undo removes the added oil");
    }
}
