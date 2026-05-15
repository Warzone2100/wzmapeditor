//! High-level map loading and saving (directories and .wz archives).
//!
//! Submodules:
//! - [`directory`] reads and writes the unpacked folder layout.
//! - [`archive`] reads and writes `.wz` zip archives plus `extract_*` helpers.
//! - [`preview`] reads only the metadata needed for minimap thumbnails.
//! - [`campaign`] handles `gamedesc.lev` and `expand`-style overlays.
//! - [`level_json`] and [`common`] hold shared zip / sidecar helpers.

pub(crate) mod archive;
mod campaign;
mod classify;
pub(crate) mod common;
mod directory;
pub(crate) mod level_json;
mod preview;
mod script;

pub use archive::{
    WzArchiveReader, extract_wz_to_dir, extract_wz_to_dir_filtered, extract_wz_to_dir_overwrite,
    load_from_wz_archive, load_map_from_archive_prefix, read_wz_entry, save_to_wz_archive,
};
pub use campaign::{load_campaign_level, load_campaign_level_by_name, read_campaign_index};
pub use classify::{WzArchiveKind, classify_wz_archive};
pub use directory::{load_from_directory, save_to_directory};
pub use preview::{MapPreview, peek_map_preview, scan_map_directory, scan_wz_archive_maps};
pub use script::{ScriptError, run_script_map, run_script_source};

use crate::map_data::MapData;
use crate::objects::{Droid, Feature, Structure};
use crate::terrain_types::TerrainTypeData;

/// Weather effect for the editor viewport preview.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
pub enum Weather {
    /// No weather override; uses the game's default tileset-based cycling.
    #[default]
    Default,
    Clear,
    Rain,
    Snow,
}

impl Weather {
    pub const ALL: [Self; 4] = [Self::Default, Self::Clear, Self::Rain, Self::Snow];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default (tileset)",
            Self::Clear => "Clear",
            Self::Rain => "Rain",
            Self::Snow => "Snow",
        }
    }
}

/// A complete loaded WZ2100 map with all its components.
#[derive(Debug, Clone)]
pub struct WzMap {
    pub map_data: MapData,
    pub structures: Vec<Structure>,
    pub droids: Vec<Droid>,
    pub features: Vec<Feature>,
    pub terrain_types: Option<TerrainTypeData>,
    /// Script labels (positions, areas) loaded from `labels.json`.
    pub labels: Vec<(String, crate::labels::ScriptLabel)>,
    /// The folder name inside the archive (if loaded from .wz).
    pub map_name: String,
    /// Player count for the map (used in `level.json` and the Nc- filename convention).
    pub players: u8,
    /// Tileset name: `"arizona"`, `"urban"`, or `"rockies"`.
    pub tileset: String,
    /// Raw contents of the map's bundled `templates.json`, when present.
    ///
    /// `wz-maplib` does not parse the schema (that's owned by `wz-stats`); it
    /// round-trips the raw JSON bytes so the editor can ship custom droid
    /// templates alongside the map. `None` means the archive has no templates.
    pub custom_templates_json: Option<String>,
    /// Primary author name from `level.json`'s `author` field.
    pub author: Option<String>,
    /// Secondary authors carried in `level.json`'s `author.additional-authors`.
    pub additional_authors: Vec<String>,
    /// SPDX license expression from `level.json`'s `license` field.
    pub license: Option<String>,
}

impl WzMap {
    pub fn new(name: &str, width: u32, height: u32) -> Self {
        Self {
            map_data: MapData::new(width, height),
            structures: Vec::new(),
            droids: Vec::new(),
            features: Vec::new(),
            terrain_types: None,
            labels: Vec::new(),
            map_name: name.to_string(),
            players: common::parse_player_count(name),
            tileset: "arizona".to_string(),
            custom_templates_json: None,
            author: None,
            additional_authors: Vec::new(),
            license: None,
        }
    }

    /// Build a resized copy of this map.
    ///
    /// Tiles, gateways, structures, droids, features, and labels are shifted by
    /// `-offset` (in tile units, scaled to world units for objects) and dropped
    /// when they fall outside the new bounds. Area labels straddling the border
    /// are dropped wholesale rather than clamped, because clamping would silently
    /// alter the trigger zone the script reacts to. Tileset, terrain type table,
    /// map name, players, and bundled templates are copied verbatim.
    ///
    /// Returns the new map and a [`ResizeReport`] of how many objects were
    /// removed by the shift.
    pub fn resized(
        &self,
        new_width: u32,
        new_height: u32,
        offset_x: i32,
        offset_y: i32,
    ) -> (Self, ResizeReport) {
        let new_map_data = self
            .map_data
            .resized(new_width, new_height, offset_x, offset_y);

        let dx = i64::from(crate::constants::world_coord(offset_x));
        let dy = i64::from(crate::constants::world_coord(offset_y));
        let max_x = i64::from(crate::constants::world_coord(new_width as i32));
        let max_y = i64::from(crate::constants::world_coord(new_height as i32));

        let shift = |p: crate::objects::WorldPos| -> Option<crate::objects::WorldPos> {
            let nx = i64::from(p.x) - dx;
            let ny = i64::from(p.y) - dy;
            if nx < 0 || ny < 0 || nx >= max_x || ny >= max_y {
                return None;
            }
            Some(crate::objects::WorldPos {
                x: nx as u32,
                y: ny as u32,
            })
        };

        let orig_structs = self.structures.len();
        let new_structures: Vec<Structure> = self
            .structures
            .iter()
            .filter_map(|s| {
                shift(s.position).map(|p| Structure {
                    position: p,
                    ..s.clone()
                })
            })
            .collect();

        let orig_droids = self.droids.len();
        let new_droids: Vec<Droid> = self
            .droids
            .iter()
            .filter_map(|d| {
                shift(d.position).map(|p| Droid {
                    position: p,
                    ..d.clone()
                })
            })
            .collect();

        let orig_feats = self.features.len();
        let new_features: Vec<Feature> = self
            .features
            .iter()
            .filter_map(|f| {
                shift(f.position).map(|p| Feature {
                    position: p,
                    ..f.clone()
                })
            })
            .collect();

        let orig_labels = self.labels.len();
        let new_labels: Vec<(String, crate::labels::ScriptLabel)> = self
            .labels
            .iter()
            .filter_map(|(name, label)| match label {
                crate::labels::ScriptLabel::Position { label: l, pos } => {
                    let p = shift(crate::objects::WorldPos {
                        x: pos[0],
                        y: pos[1],
                    })?;
                    Some((
                        name.clone(),
                        crate::labels::ScriptLabel::Position {
                            label: l.clone(),
                            pos: [p.x, p.y],
                        },
                    ))
                }
                crate::labels::ScriptLabel::Area {
                    label: l,
                    pos1,
                    pos2,
                } => {
                    let a = shift(crate::objects::WorldPos {
                        x: pos1[0],
                        y: pos1[1],
                    })?;
                    let b = shift(crate::objects::WorldPos {
                        x: pos2[0],
                        y: pos2[1],
                    })?;
                    Some((
                        name.clone(),
                        crate::labels::ScriptLabel::Area {
                            label: l.clone(),
                            pos1: [a.x, a.y],
                            pos2: [b.x, b.y],
                        },
                    ))
                }
            })
            .collect();

        let report = ResizeReport {
            structures_removed: orig_structs - new_structures.len(),
            droids_removed: orig_droids - new_droids.len(),
            features_removed: orig_feats - new_features.len(),
            labels_removed: orig_labels - new_labels.len(),
            gateways_removed: self.map_data.gateways.len() - new_map_data.gateways.len(),
        };

        let out = Self {
            map_data: new_map_data,
            structures: new_structures,
            droids: new_droids,
            features: new_features,
            terrain_types: self.terrain_types.clone(),
            labels: new_labels,
            map_name: self.map_name.clone(),
            players: self.players,
            tileset: self.tileset.clone(),
            custom_templates_json: self.custom_templates_json.clone(),
            author: self.author.clone(),
            additional_authors: self.additional_authors.clone(),
            license: self.license.clone(),
        };
        (out, report)
    }

    /// Count what would be dropped by [`Self::resized`] without building the
    /// new map. Suitable for live-preview UIs that recompute on every frame.
    pub fn resize_report(
        &self,
        new_width: u32,
        new_height: u32,
        offset_x: i32,
        offset_y: i32,
    ) -> ResizeReport {
        let dx = i64::from(crate::constants::world_coord(offset_x));
        let dy = i64::from(crate::constants::world_coord(offset_y));
        let max_x = i64::from(crate::constants::world_coord(new_width as i32));
        let max_y = i64::from(crate::constants::world_coord(new_height as i32));

        let in_bounds = |p: crate::objects::WorldPos| -> bool {
            let nx = i64::from(p.x) - dx;
            let ny = i64::from(p.y) - dy;
            nx >= 0 && ny >= 0 && nx < max_x && ny < max_y
        };

        let structures_removed = self
            .structures
            .iter()
            .filter(|s| !in_bounds(s.position))
            .count();
        let droids_removed = self
            .droids
            .iter()
            .filter(|d| !in_bounds(d.position))
            .count();
        let features_removed = self
            .features
            .iter()
            .filter(|f| !in_bounds(f.position))
            .count();

        let labels_removed = self
            .labels
            .iter()
            .filter(|(_, label)| match label {
                crate::labels::ScriptLabel::Position { pos, .. } => {
                    !in_bounds(crate::objects::WorldPos {
                        x: pos[0],
                        y: pos[1],
                    })
                }
                crate::labels::ScriptLabel::Area { pos1, pos2, .. } => {
                    !in_bounds(crate::objects::WorldPos {
                        x: pos1[0],
                        y: pos1[1],
                    }) || !in_bounds(crate::objects::WorldPos {
                        x: pos2[0],
                        y: pos2[1],
                    })
                }
            })
            .count();

        // Gateways live in tile space and use u8, mirroring `MapData::resized`.
        let nw = i64::from(new_width);
        let nh = i64::from(new_height);
        let ox = i64::from(offset_x);
        let oy = i64::from(offset_y);
        let max_u8 = i64::from(u8::MAX);
        let gateway_in =
            |x: i64, y: i64| x >= 0 && y >= 0 && x < nw && y < nh && x <= max_u8 && y <= max_u8;
        let gateways_removed = self
            .map_data
            .gateways
            .iter()
            .filter(|gw| {
                let nx1 = i64::from(gw.x1) - ox;
                let ny1 = i64::from(gw.y1) - oy;
                let nx2 = i64::from(gw.x2) - ox;
                let ny2 = i64::from(gw.y2) - oy;
                !(gateway_in(nx1, ny1) && gateway_in(nx2, ny2))
            })
            .count();

        ResizeReport {
            structures_removed,
            droids_removed,
            features_removed,
            labels_removed,
            gateways_removed,
        }
    }
}

/// Per-category counts of objects dropped during a [`WzMap::resized`] call
/// or projected by [`WzMap::resize_report`].
///
/// `labels_removed` covers both `Position` and `Area` script labels. An area
/// label is counted when either endpoint falls outside the new bounds.
#[derive(Debug, Default, Clone, Copy)]
#[must_use]
pub struct ResizeReport {
    /// Structures whose origin lands outside the resized map.
    pub structures_removed: usize,
    /// Droids whose origin lands outside the resized map.
    pub droids_removed: usize,
    /// Features whose origin lands outside the resized map.
    pub features_removed: usize,
    /// Script labels dropped by the resize.
    pub labels_removed: usize,
    /// Gateways with at least one endpoint outside the new bounds, including
    /// any that exceed the gateway u8 coordinate range.
    pub gateways_removed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OutputFormat;

    #[test]
    fn wz_map_new() {
        let map = WzMap::new("test", 64, 64);
        assert_eq!(map.map_name, "test");
        assert_eq!(map.map_data.width, 64);
        assert_eq!(map.map_data.height, 64);
        assert!(map.structures.is_empty());
        assert!(map.droids.is_empty());
        assert!(map.features.is_empty());
        assert!(map.labels.is_empty());
        assert!(map.terrain_types.is_none());
    }

    #[test]
    fn wz_map_new_sets_players_from_name() {
        let map = WzMap::new("4c-BigMap", 16, 16);
        assert_eq!(map.players, 4);
        assert_eq!(map.tileset, "arizona");

        let map2 = WzMap::new("NoPrefix", 8, 8);
        assert_eq!(map2.players, 0);
    }

    fn make_test_map() -> WzMap {
        use crate::labels::ScriptLabel;
        use crate::objects::WorldPos;
        use crate::terrain_types::TerrainType;

        let mut map = WzMap::new("2c-TestMap", 8, 8);
        map.map_data.tiles[0].height = 200;
        map.map_data.tiles[0].texture = 42;
        map.map_data.tiles[63].height = 510;
        map.map_data.tiles[63].texture = 5;

        map.structures = vec![Structure {
            name: "A0PowerGenerator".into(),
            position: WorldPos { x: 3200, y: 4480 },
            direction: 16384,
            player: 0,
            modules: 1,
            id: Some(1),
        }];
        map.droids = vec![Droid {
            name: "ViperMG".into(),
            position: WorldPos { x: 1280, y: 1280 },
            direction: 32768,
            player: 1,
            id: Some(2),
        }];
        map.features = vec![Feature {
            name: "OilResource".into(),
            position: WorldPos { x: 5120, y: 6400 },
            direction: 0,
            id: Some(100),
            player: None,
        }];
        map.terrain_types = Some(TerrainTypeData {
            terrain_types: vec![
                TerrainType::SandYellow,
                TerrainType::Sand,
                TerrainType::Bakedearth,
            ],
        });
        map.labels = vec![(
            "startPos".into(),
            ScriptLabel::Position {
                label: "startPos".into(),
                pos: [1024, 2048],
            },
        )];
        map
    }

    fn assert_maps_equal(loaded: &WzMap, original: &WzMap) {
        assert_eq!(loaded.map_data.width, original.map_data.width);
        assert_eq!(loaded.map_data.height, original.map_data.height);
        assert_eq!(loaded.map_data.tiles.len(), original.map_data.tiles.len());
        assert_eq!(
            loaded.map_data.tiles[0].height,
            original.map_data.tiles[0].height
        );
        assert_eq!(
            loaded.map_data.tiles[0].texture,
            original.map_data.tiles[0].texture
        );
        assert_eq!(
            loaded.map_data.tiles[63].height,
            original.map_data.tiles[63].height
        );

        assert_eq!(loaded.structures.len(), 1);
        assert_eq!(loaded.structures[0].name, "A0PowerGenerator");
        assert_eq!(loaded.structures[0].position.x, 3200);
        assert_eq!(loaded.structures[0].direction, 16384);
        assert_eq!(loaded.structures[0].modules, 1);

        assert_eq!(loaded.droids.len(), 1);
        assert_eq!(loaded.droids[0].name, "ViperMG");
        assert_eq!(loaded.droids[0].direction, 32768);
        assert_eq!(loaded.droids[0].player, 1);

        assert_eq!(loaded.features.len(), 1);
        assert_eq!(loaded.features[0].name, "OilResource");
        assert_eq!(loaded.features[0].id, Some(100));

        let ttp = loaded.terrain_types.as_ref().expect("ttp should be saved");
        assert_eq!(ttp.terrain_types.len(), 3);

        assert_eq!(loaded.labels.len(), 1);
    }

    #[test]
    fn save_and_load_directory_roundtrip() {
        let original = make_test_map();
        let dir = std::env::temp_dir().join("wzmapeditor_test_save_dir");
        let _ = std::fs::remove_dir_all(&dir);

        save_to_directory(&original, &dir, OutputFormat::Ver3).unwrap();

        assert!(dir.join("game.map").exists());
        assert!(dir.join("struct.json").exists());
        assert!(dir.join("droid.json").exists());
        assert!(dir.join("feature.json").exists());
        assert!(dir.join("ttypes.ttp").exists());
        assert!(dir.join("labels.json").exists());

        let loaded = load_from_directory(&dir).unwrap();
        assert_maps_equal(&loaded, &original);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_and_load_wz_archive_roundtrip() {
        let original = make_test_map();
        let wz_path = std::env::temp_dir().join("wzmapeditor_test_roundtrip.wz");
        let _ = std::fs::remove_file(&wz_path);

        save_to_wz_archive(&original, &wz_path, OutputFormat::Ver3).unwrap();
        assert!(wz_path.exists());

        let loaded = load_from_wz_archive(&wz_path).unwrap();
        assert_eq!(loaded.map_name, "2c-TestMap");
        assert_maps_equal(&loaded, &original);

        let _ = std::fs::remove_file(&wz_path);
    }

    #[test]
    fn save_empty_map_no_optional_files() {
        let map = WzMap::new("empty", 4, 4);
        let dir = std::env::temp_dir().join("wzmapeditor_test_empty");
        let _ = std::fs::remove_dir_all(&dir);

        save_to_directory(&map, &dir, OutputFormat::Ver3).unwrap();

        assert!(dir.join("game.map").exists());
        assert!(!dir.join("labels.json").exists());
        assert!(!dir.join("ttypes.ttp").exists());

        let loaded = load_from_directory(&dir).unwrap();
        assert_eq!(loaded.map_data.width, 4);
        assert!(loaded.structures.is_empty());
        assert!(loaded.terrain_types.is_none());
        assert!(loaded.labels.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wz_archive_has_wz2100_compatible_structure() {
        let original = make_test_map();
        let wz_path = std::env::temp_dir().join("wzmapeditor_test_structure.wz");
        let _ = std::fs::remove_file(&wz_path);

        save_to_wz_archive(&original, &wz_path, OutputFormat::Ver3).unwrap();

        let file = std::fs::File::open(&wz_path).unwrap();
        let archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<&str> = (0..archive.len())
            .filter_map(|i| archive.name_for_index(i))
            .collect();

        // Community-map flattened layout: all files at the archive root.
        assert!(names.contains(&"game.map"));
        assert!(names.contains(&"level.json"));
        assert!(names.contains(&"gam.json"));
        assert!(names.contains(&"struct.json"));
        assert!(names.contains(&"droid.json"));
        assert!(names.contains(&"feature.json"));
        assert!(names.contains(&"ttypes.ttp"));

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&wz_path).unwrap()).unwrap();
        let mut level_file = archive.by_name("level.json").unwrap();
        let mut level_bytes = Vec::new();
        std::io::Read::read_to_end(&mut level_file, &mut level_bytes).unwrap();
        drop(level_file);
        let level: serde_json::Value = serde_json::from_slice(&level_bytes).unwrap();
        assert_eq!(level["name"], "2c-TestMap");
        assert_eq!(level["type"], "skirmish");
        assert_eq!(level["players"], 2);
        assert_eq!(level["tileset"], "arizona");

        let mut gam_file = archive.by_name("gam.json").unwrap();
        let mut gam_bytes = Vec::new();
        std::io::Read::read_to_end(&mut gam_file, &mut gam_bytes).unwrap();
        let gam: serde_json::Value = serde_json::from_slice(&gam_bytes).unwrap();
        assert_eq!(gam["ScrollMaxX"], 8);
        assert_eq!(gam["ScrollMaxY"], 8);

        let _ = std::fs::remove_file(&wz_path);
    }

    #[test]
    fn wz_archive_peek_preview_matches_full_load() {
        let original = make_test_map();
        // Filename matches map name so the "2c-" prefix parses to player count.
        let wz_path = std::env::temp_dir().join("2c-TestMap.wz");
        let _ = std::fs::remove_file(&wz_path);

        save_to_wz_archive(&original, &wz_path, OutputFormat::Ver3).unwrap();

        let preview = peek_map_preview(&wz_path).unwrap();
        assert_eq!(preview.width, 8);
        assert_eq!(preview.height, 8);
        assert_eq!(preview.players, 2);
        assert_eq!(preview.heights[0], 200);
        assert_eq!(preview.heights[63], 510);
        assert_eq!(preview.terrain_types.len(), 3);

        let _ = std::fs::remove_file(&wz_path);
    }

    #[test]
    fn wz_archive_roundtrip_preserves_author_and_license() {
        let mut map = make_test_map();
        map.author = Some("Liam".to_string());
        map.additional_authors = vec!["Olrox".to_string(), "Past-due".to_string()];
        map.license = Some("CC-BY-SA-3.0 OR GPL-2.0-or-later".to_string());

        let wz_path = std::env::temp_dir().join("wzmapeditor_test_metadata.wz");
        let _ = std::fs::remove_file(&wz_path);

        save_to_wz_archive(&map, &wz_path, OutputFormat::Ver3).unwrap();
        let loaded = load_from_wz_archive(&wz_path).unwrap();

        assert_eq!(loaded.author.as_deref(), Some("Liam"));
        assert_eq!(loaded.additional_authors, vec!["Olrox", "Past-due"]);
        assert_eq!(
            loaded.license.as_deref(),
            Some("CC-BY-SA-3.0 OR GPL-2.0-or-later")
        );

        let _ = std::fs::remove_file(&wz_path);
    }

    #[test]
    fn level_json_includes_generator_with_version() {
        let map = make_test_map();
        let wz_path = std::env::temp_dir().join("wzmapeditor_test_generator.wz");
        let _ = std::fs::remove_file(&wz_path);

        save_to_wz_archive(&map, &wz_path, OutputFormat::Ver3).unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&wz_path).unwrap()).unwrap();
        let mut level_file = archive.by_name("level.json").unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut level_file, &mut bytes).unwrap();
        let level: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let generator = level["generator"].as_str().unwrap();
        assert!(
            generator.starts_with("wzmapeditor "),
            "expected `wzmapeditor <version>`, got {generator:?}"
        );
        assert!(
            generator.len() > "wzmapeditor ".len(),
            "version must follow the prefix"
        );

        let _ = std::fs::remove_file(&wz_path);
    }

    #[test]
    fn wz_archive_roundtrip_preserves_players_and_tileset() {
        let mut map = make_test_map();
        map.players = 4;
        map.tileset = "rockies".to_string();

        let wz_path = std::env::temp_dir().join("wzmapeditor_test_meta.wz");
        let _ = std::fs::remove_file(&wz_path);

        save_to_wz_archive(&map, &wz_path, OutputFormat::Ver3).unwrap();
        let loaded = load_from_wz_archive(&wz_path).unwrap();

        assert_eq!(loaded.players, 4);
        assert_eq!(loaded.tileset, "rockies");

        let _ = std::fs::remove_file(&wz_path);
    }

    #[test]
    fn flattened_archive_gets_name_from_level_json() {
        // map_name in level.json takes priority over the .wz filename.
        let map = make_test_map();
        let wz_path = std::env::temp_dir().join("totally_different_name.wz");
        let _ = std::fs::remove_file(&wz_path);

        save_to_wz_archive(&map, &wz_path, OutputFormat::Ver3).unwrap();
        let loaded = load_from_wz_archive(&wz_path).unwrap();

        assert_eq!(loaded.map_name, "2c-TestMap");

        let _ = std::fs::remove_file(&wz_path);
    }

    mod resize {
        use super::*;
        use crate::constants::world_coord;
        use crate::labels::ScriptLabel;
        use crate::objects::WorldPos;

        fn sample_map() -> WzMap {
            // 16x16 with one of each object well inside, plus the same on the
            // bottom-right edge so a strong crop drops them.
            let mut m = WzMap::new("3c-test", 16, 16);
            m.tileset = "rockies".to_string();
            m.players = 4;
            m.terrain_types = Some(TerrainTypeData {
                terrain_types: vec![crate::terrain_types::TerrainType::Sand; 4],
            });
            m.custom_templates_json = Some("{\"templates\":[]}".to_string());

            // Inside (tile 4,4) and outside-after-shrink (tile 12,12).
            let inside = WorldPos {
                x: world_coord(4) as u32,
                y: world_coord(4) as u32,
            };
            let outside = WorldPos {
                x: world_coord(12) as u32,
                y: world_coord(12) as u32,
            };
            m.structures.push(Structure {
                name: "A0PowMod1".to_string(),
                position: inside,
                direction: 0,
                player: 0,
                modules: 0,
                id: None,
            });
            m.structures.push(Structure {
                name: "A0PowMod1".to_string(),
                position: outside,
                direction: 0,
                player: 0,
                modules: 0,
                id: None,
            });
            m.droids.push(Droid {
                name: "Viper".to_string(),
                position: inside,
                direction: 0,
                player: 0,
                id: None,
            });
            m.droids.push(Droid {
                name: "Viper".to_string(),
                position: outside,
                direction: 0,
                player: 0,
                id: None,
            });
            m.features.push(Feature {
                name: "Tree".to_string(),
                position: outside,
                direction: 0,
                id: None,
                player: None,
            });
            m.labels.push((
                "spawn_a".to_string(),
                ScriptLabel::Position {
                    label: "spawn_a".to_string(),
                    pos: [inside.x, inside.y],
                },
            ));
            // Area straddling the new border after a 0,0 -> 8,8 crop.
            m.labels.push((
                "zone_straddle".to_string(),
                ScriptLabel::Area {
                    label: "zone_straddle".to_string(),
                    pos1: [inside.x, inside.y],
                    pos2: [outside.x, outside.y],
                },
            ));
            m
        }

        #[test]
        fn drops_outside_objects_with_correct_counts() {
            let src = sample_map();
            let (out, report) = src.resized(8, 8, 0, 0);
            assert_eq!(out.structures.len(), 1);
            assert_eq!(out.droids.len(), 1);
            assert_eq!(out.features.len(), 0);
            assert_eq!(out.labels.len(), 1);
            assert_eq!(report.structures_removed, 1);
            assert_eq!(report.droids_removed, 1);
            assert_eq!(report.features_removed, 1);
            assert_eq!(report.labels_removed, 1);
            assert_eq!(report.gateways_removed, 0);
        }

        #[test]
        fn shifts_inside_object_position() {
            let src = sample_map();
            // Crop 4 from top/left: inside object at tile (4,4) lands at (0,0).
            let (out, _) = src.resized(8, 8, 4, 4);
            assert_eq!(out.structures.len(), 1);
            assert_eq!(out.structures[0].position.x, 0);
            assert_eq!(out.structures[0].position.y, 0);
        }

        #[test]
        fn drops_area_label_partially_outside() {
            let src = sample_map();
            let (out, report) = src.resized(8, 8, 0, 0);
            assert!(out.labels.iter().all(|(name, _)| name == "spawn_a"));
            assert_eq!(report.labels_removed, 1);
        }

        #[test]
        fn preserves_tileset_and_metadata() {
            let src = sample_map();
            let (out, _) = src.resized(8, 8, 0, 0);
            assert_eq!(out.tileset, "rockies");
            assert_eq!(out.players, 4);
            assert_eq!(out.map_name, "3c-test");
            assert!(out.terrain_types.is_some());
            assert_eq!(
                out.custom_templates_json.as_deref(),
                Some("{\"templates\":[]}")
            );
        }

        #[test]
        fn report_only_matches_full_resize() {
            let src = sample_map();
            for (w, h, ox, oy) in [(8, 8, 0, 0), (8, 8, 4, 4), (16, 16, 0, 0), (32, 32, -8, -8)] {
                let (_, full) = src.resized(w, h, ox, oy);
                let only = src.resize_report(w, h, ox, oy);
                assert_eq!(
                    only.structures_removed, full.structures_removed,
                    "{w}x{h}+{ox},{oy}"
                );
                assert_eq!(only.droids_removed, full.droids_removed);
                assert_eq!(only.features_removed, full.features_removed);
                assert_eq!(only.labels_removed, full.labels_removed);
                assert_eq!(only.gateways_removed, full.gateways_removed);
            }
        }
    }
}
