//! Object position validation: off-map, near-edge, overlapping structures.

use std::collections::{BTreeMap, HashSet};

use crate::constants::{TILE_SHIFT, TILE_UNITS, TOO_NEAR_EDGE};
use crate::io_wz::WzMap;

use super::helpers::structure_packability;
use super::types::{
    IssueLocation, StatsLookup, ValidationCategory, ValidationConfig, ValidationResults,
    WarningRule,
};
use super::{push_problem, push_warning};

/// Object position checks: off-map, near-edge.
pub(super) fn validate_object_positions(
    map: &WzMap,
    stats: Option<&dyn StatsLookup>,
    config: &ValidationConfig,
    results: &mut ValidationResults,
) {
    let cat = ValidationCategory::ObjectPositions;
    let max_x = map.map_data.width * TILE_UNITS;
    let max_y = map.map_data.height * TILE_UNITS;
    let edge_min = TOO_NEAR_EDGE * TILE_UNITS;
    let edge_max_x = map.map_data.width.saturating_sub(TOO_NEAR_EDGE) * TILE_UNITS;
    let edge_max_y = map.map_data.height.saturating_sub(TOO_NEAR_EDGE) * TILE_UNITS;
    let check_near_edge = config.is_enabled(WarningRule::ObjectNearEdge);

    for s in &map.structures {
        let (x, y) = (s.position.x, s.position.y);
        if x >= max_x || y >= max_y {
            push_problem(
                results,
                cat,
                format!("Structure \"{}\" at ({x}, {y}) is off the map.", s.name),
                IssueLocation::WorldPos { x, y },
            );
        } else if check_near_edge
            && (x < edge_min || y < edge_min || x >= edge_max_x || y >= edge_max_y)
        {
            push_warning(
                results,
                WarningRule::ObjectNearEdge,
                cat,
                format!(
                    "Structure \"{}\" at ({x}, {y}) is within {TOO_NEAR_EDGE} tiles of the map edge.",
                    s.name
                ),
                IssueLocation::WorldPos { x, y },
            );
        }
    }

    for d in &map.droids {
        let (x, y) = (d.position.x, d.position.y);
        if x >= max_x || y >= max_y {
            push_problem(
                results,
                cat,
                format!("Droid \"{}\" at ({x}, {y}) is off the map.", d.name),
                IssueLocation::WorldPos { x, y },
            );
        } else if check_near_edge
            && (x < edge_min || y < edge_min || x >= edge_max_x || y >= edge_max_y)
        {
            push_warning(
                results,
                WarningRule::ObjectNearEdge,
                cat,
                format!(
                    "Droid \"{}\" at ({x}, {y}) is within {TOO_NEAR_EDGE} tiles of the map edge.",
                    d.name
                ),
                IssueLocation::WorldPos { x, y },
            );
        }
    }

    for f in &map.features {
        let (x, y) = (f.position.x, f.position.y);
        if x >= max_x || y >= max_y {
            push_problem(
                results,
                cat,
                format!("Feature \"{}\" at ({x}, {y}) is off the map.", f.name),
                IssueLocation::WorldPos { x, y },
            );
        } else if check_near_edge
            && (x < edge_min || y < edge_min || x >= edge_max_x || y >= edge_max_y)
        {
            push_warning(
                results,
                WarningRule::ObjectNearEdge,
                cat,
                format!(
                    "Feature \"{}\" at ({x}, {y}) is within {TOO_NEAR_EDGE} tiles of the map edge.",
                    f.name
                ),
                IssueLocation::WorldPos { x, y },
            );
        }
    }

    if config.is_enabled(WarningRule::OverlappingStructures) {
        validate_structure_overlap(map, stats, results);
    }
}

/// Detect overlapping structures using tile occupancy.
///
/// When `stats` is available, uses the actual structure footprint (width x breadth)
/// and packability from the stats database. Without stats, assumes 1x1 footprint
/// and default packability of 2.
fn validate_structure_overlap(
    map: &WzMap,
    stats: Option<&dyn StatsLookup>,
    results: &mut ValidationResults,
) {
    let cat = ValidationCategory::ObjectPositions;

    // Sorted iteration is required so the dedup-by-pair below picks the same
    // (top-left-most) tile every run; HashMap's randomized order would jitter
    // the reported coordinate.
    let mut occupancy: BTreeMap<(u32, u32), Vec<(usize, u8)>> = BTreeMap::new();

    for (idx, s) in map.structures.iter().enumerate() {
        let base_tx = s.position.x >> TILE_SHIFT;
        let base_ty = s.position.y >> TILE_SHIFT;

        let (w, b, stype) = stats
            .and_then(|st| st.structure_info(&s.name))
            .map_or((1, 1, None), |info| {
                (info.width, info.breadth, info.structure_type)
            });
        let pack = structure_packability(stype.as_deref());

        for dy in 0..b {
            for dx in 0..w {
                occupancy
                    .entry((base_tx + dx, base_ty + dy))
                    .or_default()
                    .push((idx, pack));
            }
        }
    }

    // One warning per pair, even when they overlap across many tiles.
    let mut reported: HashSet<(usize, usize)> = HashSet::new();

    for (&(tx, ty), occupants) in &occupancy {
        if occupants.len() < 2 {
            continue;
        }
        for i in 0..occupants.len() {
            for j in (i + 1)..occupants.len() {
                let (idx_a, pack_a) = occupants[i];
                let (idx_b, pack_b) = occupants[j];
                // Walls (packability 1) are allowed to stack.
                if pack_a == 1 && pack_b == 1 {
                    continue;
                }
                let pair = (idx_a.min(idx_b), idx_a.max(idx_b));
                if !reported.insert(pair) {
                    continue;
                }
                let name_a = &map.structures[idx_a].name;
                let name_b = &map.structures[idx_b].name;
                push_warning(
                    results,
                    WarningRule::OverlappingStructures,
                    cat,
                    format!(
                        "Overlapping structures on tile ({tx}, {ty}): \"{name_a}\" and \"{name_b}\".",
                    ),
                    IssueLocation::TilePos { x: tx, y: ty },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::test_support::*;
    use crate::validate::types::Severity;

    #[test]
    fn structure_centered_in_map_ok() {
        let mut map = valid_map(64, 64);
        let center = 32 * TILE_UNITS + 64;
        map.structures
            .push(make_structure("Test", center, center, 0));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(results.issues.is_empty(), "issues: {:?}", results.issues);
    }

    #[test]
    fn structure_position_off_map_east() {
        let mut map = valid_map(64, 64);
        let off_x = 65 * TILE_UNITS;
        map.structures
            .push(make_structure("Test", off_x, 32 * TILE_UNITS, 0));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            results
                .issues
                .iter()
                .any(|i| i.severity == Severity::Problem && i.message.contains("off the map"))
        );
    }

    #[test]
    fn structure_position_off_map_south() {
        let mut map = valid_map(64, 64);
        let off_y = 65 * TILE_UNITS;
        map.structures
            .push(make_structure("Test", 32 * TILE_UNITS, off_y, 0));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            results
                .issues
                .iter()
                .any(|i| i.severity == Severity::Problem && i.message.contains("off the map"))
        );
    }

    #[test]
    fn structure_at_origin_near_edge() {
        let mut map = valid_map(64, 64);
        map.structures.push(make_structure("Test", 0, 0, 0));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            results
                .issues
                .iter()
                .any(|i| i.severity == Severity::Warning && i.message.contains("map edge"))
        );
    }

    #[test]
    fn structure_exactly_at_edge_buffer() {
        let mut map = valid_map(64, 64);
        // First valid position: TOO_NEAR_EDGE (3) * TILE_UNITS (128) = 384.
        let edge_pos = TOO_NEAR_EDGE * TILE_UNITS;
        map.structures
            .push(make_structure("Test", edge_pos, edge_pos, 0));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            !results
                .issues
                .iter()
                .any(|i| i.message.contains("map edge")),
            "issues: {:?}",
            results.issues
        );
    }

    #[test]
    fn structure_one_tile_inside_edge_buffer() {
        let mut map = valid_map(64, 64);
        let near_edge = 2 * TILE_UNITS;
        map.structures
            .push(make_structure("Test", near_edge, near_edge, 0));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            results
                .issues
                .iter()
                .any(|i| i.message.contains("map edge"))
        );
    }

    #[test]
    fn droid_off_map() {
        let mut map = valid_map(32, 32);
        map.droids
            .push(make_droid("Test", 33 * TILE_UNITS, 16 * TILE_UNITS, 0));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            results
                .issues
                .iter()
                .any(|i| i.severity == Severity::Problem
                    && i.message.contains("Droid")
                    && i.message.contains("off the map"))
        );
    }

    #[test]
    fn droid_near_edge_warns() {
        let mut map = valid_map(64, 64);
        map.droids
            .push(make_droid("Test", TILE_UNITS, 32 * TILE_UNITS, 0));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            results
                .issues
                .iter()
                .any(|i| i.severity == Severity::Warning
                    && i.message.contains("Droid")
                    && i.message.contains("map edge"))
        );
    }

    #[test]
    fn feature_off_map() {
        let mut map = valid_map(32, 32);
        map.features
            .push(make_feature("Test", 33 * TILE_UNITS, 16 * TILE_UNITS));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            results
                .issues
                .iter()
                .any(|i| i.severity == Severity::Problem
                    && i.message.contains("Feature")
                    && i.message.contains("off the map"))
        );
    }

    #[test]
    fn feature_near_edge_warns() {
        let mut map = valid_map(64, 64);
        map.features
            .push(make_feature("Test", TILE_UNITS, 32 * TILE_UNITS));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            results
                .issues
                .iter()
                .any(|i| i.severity == Severity::Warning
                    && i.message.contains("Feature")
                    && i.message.contains("map edge"))
        );
    }

    #[test]
    fn overlapping_structures_same_tile() {
        let mut map = valid_map(64, 64);
        let pos = 32 * TILE_UNITS + 64;
        map.structures.push(make_structure("A", pos, pos, 0));
        map.structures.push(make_structure("B", pos, pos, 1));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            results
                .issues
                .iter()
                .any(|i| i.message.contains("Overlapping"))
        );
    }

    #[test]
    fn structures_adjacent_tiles_ok() {
        let mut map = valid_map(64, 64);
        let pos_a = 32 * TILE_UNITS + 64;
        let pos_b = 33 * TILE_UNITS + 64;
        map.structures.push(make_structure("A", pos_a, pos_a, 0));
        map.structures.push(make_structure("B", pos_b, pos_a, 1));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            !results
                .issues
                .iter()
                .any(|i| i.message.contains("Overlapping")),
            "issues: {:?}",
            results.issues
        );
    }

    #[test]
    fn many_objects_off_map() {
        let mut map = valid_map(32, 32);
        let off = 33 * TILE_UNITS;
        for i in 0..10 {
            map.structures.push(make_structure(
                &format!("S{i}"),
                off + i * TILE_UNITS,
                off,
                0,
            ));
        }
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        let off_map_count = results
            .issues
            .iter()
            .filter(|i| i.message.contains("off the map"))
            .count();
        assert_eq!(off_map_count, 10);
    }

    #[test]
    fn object_at_max_valid_position() {
        let mut map = valid_map(64, 64);
        let max_pos = 64 * TILE_UNITS - 1;
        map.structures
            .push(make_structure("Test", max_pos, max_pos, 0));
        let mut results = ValidationResults::default();
        validate_object_positions(&map, None, &ValidationConfig::default(), &mut results);
        assert!(
            !results
                .issues
                .iter()
                .any(|i| i.severity == Severity::Problem && i.message.contains("off the map"))
        );
    }

    #[test]
    fn disabled_near_edge_suppresses_warning() {
        let mut config = ValidationConfig::default();
        config.disabled.insert(WarningRule::ObjectNearEdge);

        let mut map = valid_map(20, 20);
        // Tile 1 (world 128) sits inside the 3-tile edge buffer.
        map.structures
            .push(make_structure("A0LightFactory", 128, 128, 0));

        let r = crate::validate::validate_map(&map, None, &ValidationConfig::default());
        assert!(
            r.issues
                .iter()
                .any(|i| i.severity == Severity::Warning && i.message.contains("map edge")),
            "default config should warn about near-edge"
        );

        let r = crate::validate::validate_map(&map, None, &config);
        assert!(
            !r.issues
                .iter()
                .any(|i| i.severity == Severity::Warning && i.message.contains("map edge")),
            "disabled ObjectNearEdge should suppress near-edge warning"
        );
    }
}
