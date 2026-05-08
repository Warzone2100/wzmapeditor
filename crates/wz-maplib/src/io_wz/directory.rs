//! Load and save a [`WzMap`] using a plain on-disk directory layout.
//!
//! WZ2100 maps unpacked from a `.wz` archive look like a folder containing
//! `game.map`, the per-object JSON (or legacy `.bjo`) sidecars, and the
//! optional `ttypes.ttp`, `labels.json`, and `templates.json` files.

use std::path::Path;

use crate::MapError;
use crate::io_binary::{self, OutputFormat};
use crate::io_bjo;
use crate::io_json;
use crate::io_ttp;

use super::WzMap;
use super::common::{
    detect_tileset_from_ttp, parse_player_count, read_optional_bjo, read_optional_json,
};

/// Load a map from a directory on disk (containing game.map, struct.json, etc.).
pub fn load_from_directory(dir: &Path) -> Result<WzMap, MapError> {
    let map_name = dir.file_name().map_or_else(
        || "unnamed".to_string(),
        |n| n.to_string_lossy().to_string(),
    );

    let game_map_path = dir.join("game.map");
    let map_bytes = std::fs::read(&game_map_path).map_err(|e| MapError::Io {
        context: format!("reading {}", game_map_path.display()),
        source: e,
    })?;
    let map_data = io_binary::read_game_map(&map_bytes)?;

    // Campaign maps predate JSON objects and only ship `.bjo` siblings, so
    // match WZ2100 and try JSON first, then fall back.
    let players_hint = parse_player_count(&map_name);
    let max_players = u32::from(players_hint.max(1));

    let structures = read_optional_json(dir, "struct.json", io_json::read_structures)
        .unwrap_or_else(|| {
            read_optional_bjo(dir, "struct.bjo", max_players, io_bjo::read_structures)
        });

    let droids = read_optional_json(dir, "droid.json", io_json::read_droids)
        .unwrap_or_else(|| read_optional_bjo(dir, "dinit.bjo", max_players, io_bjo::read_droids));

    let features = read_optional_json(dir, "feature.json", io_json::read_features)
        .unwrap_or_else(|| read_optional_bjo(dir, "feat.bjo", max_players, io_bjo::read_features));

    let terrain_types = {
        let ttp_path = dir.join("ttypes.ttp");
        if ttp_path.exists() {
            let bytes = std::fs::read(&ttp_path).map_err(|e| MapError::Io {
                context: format!("reading {}", ttp_path.display()),
                source: e,
            })?;
            Some(io_ttp::read_ttp(&bytes)?)
        } else {
            None
        }
    };

    let labels = {
        let path = dir.join("labels.json");
        if path.exists() {
            match std::fs::read(&path) {
                Ok(bytes) => crate::labels::read_labels(&bytes).unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        }
    };

    let custom_templates_json = {
        let path = dir.join("templates.json");
        if path.exists() {
            std::fs::read_to_string(&path).ok()
        } else {
            None
        }
    };

    let players = parse_player_count(&map_name);
    let tileset = detect_tileset_from_ttp(terrain_types.as_ref());

    Ok(WzMap {
        map_data,
        structures,
        droids,
        features,
        terrain_types,
        labels,
        map_name,
        players,
        tileset,
        custom_templates_json,
        author: None,
        additional_authors: Vec::new(),
        license: None,
    })
}

/// Save a map to a directory on disk.
pub fn save_to_directory(map: &WzMap, dir: &Path, format: OutputFormat) -> Result<(), MapError> {
    std::fs::create_dir_all(dir).map_err(|e| MapError::Io {
        context: format!("creating directory {}", dir.display()),
        source: e,
    })?;

    let write_file = |name: &str, data: &[u8]| -> Result<(), MapError> {
        std::fs::write(dir.join(name), data).map_err(|e| MapError::Io {
            context: format!("writing {name}"),
            source: e,
        })
    };

    let map_bytes = io_binary::write_game_map(&map.map_data, format)?;
    write_file("game.map", &map_bytes)?;

    let struct_json = io_json::write_structures(&map.structures)?;
    write_file("struct.json", struct_json.as_bytes())?;

    let droid_json = io_json::write_droids(&map.droids)?;
    write_file("droid.json", droid_json.as_bytes())?;

    let feature_json = io_json::write_features(&map.features)?;
    write_file("feature.json", feature_json.as_bytes())?;

    if let Some(ref tt) = map.terrain_types {
        let ttp_bytes = io_ttp::write_ttp(tt)?;
        write_file("ttypes.ttp", &ttp_bytes)?;
    }

    if !map.labels.is_empty() {
        let labels_bytes = crate::labels::write_labels(&map.labels)?;
        write_file("labels.json", &labels_bytes)?;
    }

    if let Some(ref json) = map.custom_templates_json {
        write_file("templates.json", json.as_bytes())?;
    }

    Ok(())
}
