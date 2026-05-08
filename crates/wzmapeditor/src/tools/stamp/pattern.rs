//! Captured stamp pattern data and rectangular capture from a map.

use wz_maplib::WzMap;
use wz_maplib::constants::TILE_UNITS;

/// A single tile in a captured stamp pattern.
#[derive(Debug, Clone)]
pub struct StampTile {
    /// Column offset from the pattern's top-left corner.
    pub dx: u32,
    /// Row offset from the pattern's top-left corner.
    pub dy: u32,
    /// Full packed texture value (`texture_id` + orientation bits).
    pub texture: u16,
    /// Absolute tile height.
    pub height: u16,
}

/// An object captured within a stamp pattern, with position relative to the
/// pattern's top-left corner in world units.
#[derive(Debug, Clone)]
pub enum StampObject {
    Structure {
        name: String,
        offset_x: i32,
        offset_y: i32,
        direction: u16,
        player: i8,
        modules: u8,
    },
    Droid {
        name: String,
        offset_x: i32,
        offset_y: i32,
        direction: u16,
        player: i8,
    },
    Feature {
        name: String,
        offset_x: i32,
        offset_y: i32,
        direction: u16,
        player: Option<i8>,
    },
}

impl StampObject {
    /// Returns the object name, world-unit offsets, and direction.
    pub fn name_offset_dir(&self) -> (&str, i32, i32, u16) {
        match self {
            Self::Structure {
                name,
                offset_x,
                offset_y,
                direction,
                ..
            }
            | Self::Droid {
                name,
                offset_x,
                offset_y,
                direction,
                ..
            }
            | Self::Feature {
                name,
                offset_x,
                offset_y,
                direction,
                ..
            } => (name, *offset_x, *offset_y, *direction),
        }
    }

    /// Returns the world-unit offsets and direction (no name borrow).
    pub fn offset_dir(&self) -> (i32, i32, u16) {
        let (_, ox, oy, dir) = self.name_offset_dir();
        (ox, oy, dir)
    }
}

/// A captured rectangular pattern of tiles and objects.
#[derive(Debug, Clone)]
pub struct StampPattern {
    /// Width of the pattern in tiles.
    pub width: u32,
    /// Height of the pattern in tiles.
    pub height: u32,
    /// Tile data (one entry per tile in the rectangle).
    pub tiles: Vec<StampTile>,
    /// Objects with positions relative to the pattern origin.
    pub objects: Vec<StampObject>,
}

/// Capture tiles and objects within a rectangle into a `StampPattern`.
pub fn capture_pattern(map: &WzMap, sx: u32, sy: u32, ex: u32, ey: u32) -> StampPattern {
    let min_x = sx.min(ex);
    let min_y = sy.min(ey);
    let max_x = sx.max(ex);
    let max_y = sy.max(ey);

    let width = max_x - min_x + 1;
    let height = max_y - min_y + 1;

    let mut tiles = Vec::with_capacity((width * height) as usize);
    for dy in 0..height {
        for dx in 0..width {
            let tx = min_x + dx;
            let ty = min_y + dy;
            if let Some(tile) = map.map_data.tile(tx, ty) {
                tiles.push(StampTile {
                    dx,
                    dy,
                    texture: tile.texture,
                    height: tile.height,
                });
            }
        }
    }

    let world_min_x = (min_x * TILE_UNITS) as i32;
    let world_min_y = (min_y * TILE_UNITS) as i32;
    let world_max_x = ((max_x + 1) * TILE_UNITS) as i32;
    let world_max_y = ((max_y + 1) * TILE_UNITS) as i32;

    let mut objects = Vec::new();

    for s in &map.structures {
        let wx = s.position.x as i32;
        let wy = s.position.y as i32;
        if wx >= world_min_x && wx < world_max_x && wy >= world_min_y && wy < world_max_y {
            objects.push(StampObject::Structure {
                name: s.name.clone(),
                offset_x: wx - world_min_x,
                offset_y: wy - world_min_y,
                direction: s.direction,
                player: s.player,
                modules: s.modules,
            });
        }
    }

    for d in &map.droids {
        let wx = d.position.x as i32;
        let wy = d.position.y as i32;
        if wx >= world_min_x && wx < world_max_x && wy >= world_min_y && wy < world_max_y {
            objects.push(StampObject::Droid {
                name: d.name.clone(),
                offset_x: wx - world_min_x,
                offset_y: wy - world_min_y,
                direction: d.direction,
                player: d.player,
            });
        }
    }

    for f in &map.features {
        let wx = f.position.x as i32;
        let wy = f.position.y as i32;
        if wx >= world_min_x && wx < world_max_x && wy >= world_min_y && wy < world_max_y {
            objects.push(StampObject::Feature {
                name: f.name.clone(),
                offset_x: wx - world_min_x,
                offset_y: wy - world_min_y,
                direction: f.direction,
                player: f.player,
            });
        }
    }

    StampPattern {
        width,
        height,
        tiles,
        objects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wz_maplib::map_data::MapData;
    use wz_maplib::objects::{Droid, Feature, Structure, WorldPos};

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
        }
    }

    #[test]
    fn capture_empty_region() {
        let map = make_test_map(10, 10);
        let pat = capture_pattern(&map, 2, 3, 4, 5);
        assert_eq!(pat.width, 3);
        assert_eq!(pat.height, 3);
        assert_eq!(pat.tiles.len(), 9);
        assert!(pat.objects.is_empty());
    }

    #[test]
    fn capture_single_tile() {
        let mut map = make_test_map(10, 10);
        if let Some(t) = map.map_data.tile_mut(5, 5) {
            t.texture = 42;
            t.height = 100;
        }
        let pat = capture_pattern(&map, 5, 5, 5, 5);
        assert_eq!(pat.width, 1);
        assert_eq!(pat.height, 1);
        assert_eq!(pat.tiles.len(), 1);
        assert_eq!(pat.tiles[0].texture, 42);
        assert_eq!(pat.tiles[0].height, 100);
    }

    #[test]
    fn capture_objects_within_rect() {
        let mut map = make_test_map(10, 10);
        // Place a structure at tile (3, 3) center = world (3*128+64, 3*128+64) = (448, 448).
        map.structures.push(Structure {
            name: "A0PowerGenerator".into(),
            position: WorldPos { x: 448, y: 448 },
            direction: 0x4000,
            player: 0,
            modules: 0,
            id: Some(1),
        });
        // Place a feature outside the rectangle.
        map.features.push(Feature {
            name: "Tree1".into(),
            position: WorldPos { x: 0, y: 0 },
            direction: 0,
            id: None,
            player: None,
        });

        let pat = capture_pattern(&map, 2, 2, 4, 4);
        assert_eq!(pat.objects.len(), 1);
        let StampObject::Structure {
            name, direction, ..
        } = &pat.objects[0]
        else {
            unreachable!("capture region holds only the structure pushed above");
        };
        assert_eq!(name, "A0PowerGenerator");
        assert_eq!(*direction, 0x4000);
    }

    #[test]
    fn capture_reversed_coordinates() {
        // Dragging from bottom-right to top-left should produce the same pattern.
        let mut map = make_test_map(10, 10);
        if let Some(t) = map.map_data.tile_mut(2, 3) {
            t.texture = 77;
        }
        let pat_forward = capture_pattern(&map, 2, 3, 4, 5);
        let pat_reverse = capture_pattern(&map, 4, 5, 2, 3);
        assert_eq!(pat_forward.width, pat_reverse.width);
        assert_eq!(pat_forward.height, pat_reverse.height);
        assert_eq!(pat_forward.tiles.len(), pat_reverse.tiles.len());
        assert_eq!(pat_forward.tiles[0].texture, pat_reverse.tiles[0].texture);
    }

    #[test]
    fn capture_all_object_types() {
        let mut map = make_test_map(10, 10);
        let center = 2 * TILE_UNITS + TILE_UNITS / 2;
        map.structures.push(Structure {
            name: "Wall".into(),
            position: WorldPos {
                x: center,
                y: center,
            },
            direction: 0,
            player: 0,
            modules: 0,
            id: None,
        });
        map.droids.push(Droid {
            name: "Truck".into(),
            position: WorldPos {
                x: center,
                y: center,
            },
            direction: 0x8000,
            player: 1,
            id: None,
        });
        map.features.push(Feature {
            name: "OilDrum".into(),
            position: WorldPos {
                x: center,
                y: center,
            },
            direction: 0,
            id: None,
            player: None,
        });

        let pat = capture_pattern(&map, 2, 2, 2, 2);
        assert_eq!(pat.objects.len(), 3);
        assert!(
            pat.objects
                .iter()
                .any(|o| matches!(o, StampObject::Structure { name, .. } if name == "Wall"))
        );
        assert!(
            pat.objects
                .iter()
                .any(|o| matches!(o, StampObject::Droid { name, .. } if name == "Truck"))
        );
        assert!(
            pat.objects
                .iter()
                .any(|o| matches!(o, StampObject::Feature { name, .. } if name == "OilDrum"))
        );
    }
}
