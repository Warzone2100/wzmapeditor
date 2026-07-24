//! Per-tile stamp placement and the shared `push_object` helper.

use wz_maplib::WzMap;
use wz_maplib::constants::TILE_UNITS;
use wz_maplib::objects::{Droid, Feature, Structure, WorldPos};

use super::command::{ObjectAccum, StampCommand};
use super::pattern::{StampObject, StampPattern};

/// Result of pushing one templated object onto a map: which collection it
/// landed in, the index it was inserted at, and a clone of the inserted value
/// so callers can build an undo trail without re-reading the map.
#[derive(Debug)]
pub(super) enum PushedObject {
    Structure(usize, Structure),
    Droid(usize, Droid),
    Feature(usize, Feature),
}

/// Materialise a `StampObject` template onto `map` at the given world position
/// and direction. Stamped objects always get `id: None` so they cannot collide
/// with existing IDs in the map.
pub(super) fn push_object(
    map: &mut WzMap,
    template: &StampObject,
    position: WorldPos,
    direction: u16,
) -> PushedObject {
    match template {
        StampObject::Structure {
            name,
            player,
            modules,
            ..
        } => {
            let s = Structure {
                name: name.clone(),
                position,
                direction,
                player: *player,
                modules: *modules,
                id: None,
            };
            let idx = map.structures.len();
            map.structures.push(s.clone());
            PushedObject::Structure(idx, s)
        }
        StampObject::Droid { name, player, .. } => {
            let d = Droid {
                name: name.clone(),
                position,
                direction,
                player: *player,
                id: None,
            };
            let idx = map.droids.len();
            map.droids.push(d.clone());
            PushedObject::Droid(idx, d)
        }
        StampObject::Feature { name, player, .. } => {
            let f = Feature {
                name: name.clone(),
                position,
                direction,
                id: None,
                player: *player,
            };
            let idx = map.features.len();
            map.features.push(f.clone());
            PushedObject::Feature(idx, f)
        }
    }
}

/// Apply a stamp pattern at the given tile position, returning the undo command.
///
/// - `stamp_tiles` writes the pattern's tile texture and orientation bits.
/// - `stamp_terrain` writes the pattern's tile height.
/// - `stamp_objects` places the pattern's structures, droids, and features.
///
/// The three are independent. Enabling only `stamp_terrain` stamps the captured
/// heightfield while leaving existing textures untouched.
pub(super) fn apply_stamp(
    map: &mut WzMap,
    pattern: &StampPattern,
    target_x: u32,
    target_y: u32,
    stamp_tiles: bool,
    stamp_terrain: bool,
    stamp_objects: bool,
) -> StampCommand {
    let map_w = map.map_data.width;
    let map_h = map.map_data.height;

    let mut tile_changes = Vec::new();

    if stamp_tiles || stamp_terrain {
        for tile in &pattern.tiles {
            let tx = target_x + tile.dx;
            let ty = target_y + tile.dy;
            if tx >= map_w || ty >= map_h {
                continue;
            }
            if let Some(map_tile) = map.map_data.tile(tx, ty) {
                let old_tex = map_tile.texture;
                let old_h = map_tile.height;
                let new_tex = if stamp_tiles { tile.texture } else { old_tex };
                let new_h = if stamp_terrain { tile.height } else { old_h };
                if old_tex != new_tex || old_h != new_h {
                    tile_changes.push((tx, ty, old_tex, old_h, new_tex, new_h));
                }
            }
        }

        for &(x, y, _, _, new_tex, new_h) in &tile_changes {
            if let Some(t) = map.map_data.tile_mut(x, y) {
                t.texture = new_tex;
                t.height = new_h;
            }
        }
    }

    let mut accum = ObjectAccum::default();

    if stamp_objects {
        let origin_x = (target_x * TILE_UNITS) as i32;
        let origin_y = (target_y * TILE_UNITS) as i32;

        for obj in &pattern.objects {
            let (offset_x, offset_y, direction) = obj.offset_dir();
            let position = WorldPos {
                x: (origin_x + offset_x) as u32,
                y: (origin_y + offset_y) as u32,
            };
            accum.place(map, obj, position, direction);
        }
    }

    accum.into_command(tile_changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::history::EditCommand;
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

    fn structure_template() -> StampObject {
        StampObject::Structure {
            name: "HQ".into(),
            offset_x: 0,
            offset_y: 0,
            direction: 0,
            player: 2,
            modules: 1,
        }
    }

    #[test]
    fn push_object_structure_returns_index_and_clears_id() {
        let mut map = make_test_map(5, 5);
        map.structures.push(Structure {
            name: "Existing".into(),
            position: WorldPos { x: 0, y: 0 },
            direction: 0,
            player: 0,
            modules: 0,
            id: Some(99),
        });

        let result = push_object(
            &mut map,
            &structure_template(),
            WorldPos { x: 100, y: 200 },
            0x4000,
        );

        let PushedObject::Structure(idx, value) = result else {
            unreachable!("template is a Structure; push_object preserves variant");
        };
        assert_eq!(idx, 1);
        assert!(value.id.is_none());
        assert_eq!(value.player, 2);
        assert_eq!(value.modules, 1);
        assert_eq!(value.direction, 0x4000);
        assert_eq!(map.structures.len(), 2);
        assert_eq!(map.structures[1].position.x, 100);
    }

    #[test]
    fn push_object_droid_appends_at_end() {
        let mut map = make_test_map(5, 5);
        let template = StampObject::Droid {
            name: "Truck".into(),
            offset_x: 0,
            offset_y: 0,
            direction: 0,
            player: 1,
        };
        let r1 = push_object(&mut map, &template, WorldPos { x: 0, y: 0 }, 0);
        let r2 = push_object(&mut map, &template, WorldPos { x: 1, y: 1 }, 0);
        let PushedObject::Droid(i1, _) = r1 else {
            unreachable!("variant matches template");
        };
        let PushedObject::Droid(i2, _) = r2 else {
            unreachable!("variant matches template");
        };
        assert_eq!(i1, 0);
        assert_eq!(i2, 1);
        assert_eq!(map.droids.len(), 2);
    }

    #[test]
    fn push_object_feature_preserves_player_option() {
        let mut map = make_test_map(5, 5);
        let template = StampObject::Feature {
            name: "Tree".into(),
            offset_x: 0,
            offset_y: 0,
            direction: 0,
            player: Some(3),
        };
        let r = push_object(&mut map, &template, WorldPos { x: 50, y: 60 }, 0x8000);
        let PushedObject::Feature(idx, value) = r else {
            unreachable!("variant matches template");
        };
        assert_eq!(idx, 0);
        assert_eq!(value.player, Some(3));
        assert_eq!(value.direction, 0x8000);
    }

    #[test]
    fn apply_stamp_tiles_only() {
        let mut map = make_test_map(10, 10);
        let pattern = StampPattern {
            width: 2,
            height: 2,
            tiles: vec![
                super::super::pattern::StampTile {
                    dx: 0,
                    dy: 0,
                    texture: 42,
                    height: 100,
                },
                super::super::pattern::StampTile {
                    dx: 1,
                    dy: 0,
                    texture: 43,
                    height: 110,
                },
                super::super::pattern::StampTile {
                    dx: 0,
                    dy: 1,
                    texture: 44,
                    height: 120,
                },
                super::super::pattern::StampTile {
                    dx: 1,
                    dy: 1,
                    texture: 45,
                    height: 130,
                },
            ],
            objects: Vec::new(),
        };

        let cmd = apply_stamp(&mut map, &pattern, 3, 3, true, true, false);
        assert_eq!(cmd.tile_changes.len(), 4);
        assert_eq!(map.map_data.tile(3, 3).unwrap().texture, 42);
        assert_eq!(map.map_data.tile(4, 3).unwrap().texture, 43);
        assert_eq!(map.map_data.tile(3, 4).unwrap().texture, 44);
        assert_eq!(map.map_data.tile(4, 4).unwrap().texture, 45);
    }

    #[test]
    fn apply_stamp_clips_at_map_edge() {
        let mut map = make_test_map(5, 5);
        let pattern = StampPattern {
            width: 3,
            height: 3,
            tiles: (0..9)
                .map(|i| super::super::pattern::StampTile {
                    dx: i % 3,
                    dy: i / 3,
                    texture: 10 + i as u16,
                    height: 50,
                })
                .collect(),
            objects: Vec::new(),
        };

        let cmd = apply_stamp(&mut map, &pattern, 4, 4, true, true, false);
        assert_eq!(cmd.tile_changes.len(), 1);
        assert_eq!(map.map_data.tile(4, 4).unwrap().texture, 10);
    }

    #[test]
    fn apply_stamp_tiles_only_preserves_terrain() {
        let mut map = make_test_map(5, 5);
        if let Some(t) = map.map_data.tile_mut(1, 1) {
            t.texture = 5;
            t.height = 77;
        }
        let pattern = StampPattern {
            width: 1,
            height: 1,
            tiles: vec![super::super::pattern::StampTile {
                dx: 0,
                dy: 0,
                texture: 42,
                height: 200,
            }],
            objects: Vec::new(),
        };
        let _cmd = apply_stamp(&mut map, &pattern, 1, 1, true, false, false);
        let tile = map.map_data.tile(1, 1).unwrap();
        assert_eq!(tile.texture, 42);
        assert_eq!(tile.height, 77, "terrain height must be untouched");
    }

    #[test]
    fn apply_stamp_terrain_only_preserves_tiles() {
        let mut map = make_test_map(5, 5);
        if let Some(t) = map.map_data.tile_mut(1, 1) {
            t.texture = 5;
            t.height = 77;
        }
        let pattern = StampPattern {
            width: 1,
            height: 1,
            tiles: vec![super::super::pattern::StampTile {
                dx: 0,
                dy: 0,
                texture: 42,
                height: 200,
            }],
            objects: Vec::new(),
        };
        let _cmd = apply_stamp(&mut map, &pattern, 1, 1, false, true, false);
        let tile = map.map_data.tile(1, 1).unwrap();
        assert_eq!(tile.texture, 5, "texture must be untouched");
        assert_eq!(tile.height, 200);
    }

    #[test]
    fn stamp_undo_restores_tiles() {
        let mut map = make_test_map(10, 10);
        if let Some(t) = map.map_data.tile_mut(3, 3) {
            t.texture = 99;
            t.height = 200;
        }

        let pattern = StampPattern {
            width: 1,
            height: 1,
            tiles: vec![super::super::pattern::StampTile {
                dx: 0,
                dy: 0,
                texture: 42,
                height: 100,
            }],
            objects: Vec::new(),
        };

        let cmd = apply_stamp(&mut map, &pattern, 3, 3, true, true, false);
        assert_eq!(map.map_data.tile(3, 3).unwrap().texture, 42);

        cmd.undo(&mut map);
        assert_eq!(map.map_data.tile(3, 3).unwrap().texture, 99);
        assert_eq!(map.map_data.tile(3, 3).unwrap().height, 200);
    }

    #[test]
    fn stamp_with_objects_and_undo() {
        let mut map = make_test_map(10, 10);
        let pattern = StampPattern {
            width: 2,
            height: 2,
            tiles: Vec::new(),
            objects: vec![StampObject::Feature {
                name: "Tree1".into(),
                offset_x: 64,
                offset_y: 64,
                direction: 0,
                player: None,
            }],
        };

        assert!(map.features.is_empty());
        let cmd = apply_stamp(&mut map, &pattern, 5, 5, false, false, true);
        assert_eq!(map.features.len(), 1);
        assert_eq!(map.features[0].position.x, 5 * 128 + 64);

        cmd.undo(&mut map);
        assert!(map.features.is_empty());
    }

    #[test]
    fn stamp_noop_produces_empty_changes() {
        let map = make_test_map(10, 10);
        let default_tex = map.map_data.tile(0, 0).unwrap().texture;
        let default_h = map.map_data.tile(0, 0).unwrap().height;

        let pattern = StampPattern {
            width: 1,
            height: 1,
            tiles: vec![super::super::pattern::StampTile {
                dx: 0,
                dy: 0,
                texture: default_tex,
                height: default_h,
            }],
            objects: Vec::new(),
        };

        let mut map = map;
        let cmd = apply_stamp(&mut map, &pattern, 0, 0, true, true, false);
        assert!(
            cmd.tile_changes.is_empty(),
            "no-op stamp should produce zero tile changes"
        );
    }

    #[test]
    fn stamp_execute_redo() {
        let mut map = make_test_map(10, 10);
        let pattern = StampPattern {
            width: 1,
            height: 1,
            tiles: vec![super::super::pattern::StampTile {
                dx: 0,
                dy: 0,
                texture: 55,
                height: 150,
            }],
            objects: vec![StampObject::Structure {
                name: "Wall1".into(),
                offset_x: 64,
                offset_y: 64,
                direction: 0,
                player: 1,
                modules: 0,
            }],
        };

        let cmd = apply_stamp(&mut map, &pattern, 2, 2, true, true, true);
        assert_eq!(map.map_data.tile(2, 2).unwrap().texture, 55);
        assert_eq!(map.structures.len(), 1);

        cmd.undo(&mut map);
        assert_eq!(map.map_data.tile(2, 2).unwrap().texture, 0);
        assert!(map.structures.is_empty());

        cmd.execute(&mut map);
        assert_eq!(map.map_data.tile(2, 2).unwrap().texture, 55);
        assert_eq!(map.structures.len(), 1);
        assert_eq!(map.structures[0].name, "Wall1");
        assert_eq!(map.structures[0].player, 1);
    }

    #[test]
    fn capture_stamp_roundtrip() {
        use super::super::pattern::capture_pattern;
        let mut map = make_test_map(10, 10);
        for dy in 0..2u32 {
            for dx in 0..2u32 {
                if let Some(t) = map.map_data.tile_mut(1 + dx, 1 + dy) {
                    t.texture = (10 + dy * 2 + dx) as u16;
                    t.height = (100 + dy * 10 + dx) as u16;
                }
            }
        }

        let pat = capture_pattern(&map, 1, 1, 2, 2);
        let _cmd = apply_stamp(&mut map, &pat, 5, 5, true, true, false);

        for dy in 0..2u32 {
            for dx in 0..2u32 {
                let src = map.map_data.tile(1 + dx, 1 + dy).unwrap();
                let dst = map.map_data.tile(5 + dx, 5 + dy).unwrap();
                assert_eq!(src.texture, dst.texture, "texture mismatch at ({dx},{dy})");
                assert_eq!(src.height, dst.height, "height mismatch at ({dx},{dy})");
            }
        }
    }

    #[test]
    fn stamp_objects_get_no_id() {
        let mut map = make_test_map(10, 10);
        let pattern = StampPattern {
            width: 1,
            height: 1,
            tiles: Vec::new(),
            objects: vec![
                StampObject::Structure {
                    name: "HQ".into(),
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

        let _cmd = apply_stamp(&mut map, &pattern, 0, 0, false, false, true);
        assert!(map.structures[0].id.is_none());
        assert!(map.droids[0].id.is_none());
        assert!(map.features[0].id.is_none());
    }
}
