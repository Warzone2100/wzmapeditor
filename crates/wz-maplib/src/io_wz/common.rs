//! Shared helpers for the `io_wz` submodules.
//!
//! These helpers are crate-internal: directory loading, archive loading,
//! preview scanning, and campaign loading all need to read optional
//! sidecars (JSON or BJO) and locate the map folder inside a zip. Keeping
//! them in one place avoids drift between the directory and archive code
//! paths.

use std::io::Read;
use std::path::Path;

use crate::MapError;
use crate::terrain_types::TerrainTypeData;

/// Parse the player count from a WZ2100 map filename convention.
///
/// Maps named like "2c-MapName" or "2p-AValley" have 2 players, etc.
pub(super) fn parse_player_count(name: &str) -> u8 {
    for sep in ["c-", "p-"] {
        if let Some(idx) = name.find(sep)
            && idx > 0
            && let Ok(n) = name[..idx].parse::<u8>()
        {
            return n;
        }
    }
    0
}

/// Detect tileset name from TTP terrain type data.
pub(super) fn detect_tileset_from_ttp(terrain_types: Option<&TerrainTypeData>) -> String {
    if let Some(tt) = terrain_types {
        let types: Vec<u16> = tt.terrain_types.iter().map(|t| *t as u16).collect();
        if types.len() >= 3 {
            return match (types[0], types[1], types[2]) {
                (2, 2, 2) => "urban",
                (0, 0, 2) => "rockies",
                _ => "arizona",
            }
            .to_string();
        }
    }
    "arizona".to_string()
}

/// Read an optional JSON sidecar.
///
/// Returns `None` when the file is absent so the caller can try a binary
/// fallback. A parse error returns `Some(Vec::new())` so we don't silently
/// reload the legacy file on top of a corrupt JSON.
pub(super) fn read_optional_json<T>(
    dir: &Path,
    filename: &str,
    parser: fn(&str) -> Result<Vec<T>, MapError>,
) -> Option<Vec<T>> {
    let path = dir.join(filename);
    if !path.exists() {
        return None;
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => Some(parser(&content).unwrap_or_else(|e| {
            log::warn!("Failed to parse {filename}: {e}");
            Vec::new()
        })),
        Err(e) => {
            log::warn!("Failed to read {}: {}", path.display(), e);
            Some(Vec::new())
        }
    }
}

/// Read an optional BJO sidecar. Returns an empty vec on any failure
/// (including absence) so the loader stays forgiving.
pub(super) fn read_optional_bjo<T>(
    dir: &Path,
    filename: &str,
    map_max_players: u32,
    parser: fn(&[u8], u32) -> Result<Vec<T>, MapError>,
) -> Vec<T> {
    let path = dir.join(filename);
    if !path.exists() {
        return Vec::new();
    }
    match std::fs::read(&path) {
        Ok(bytes) => parser(&bytes, map_max_players).unwrap_or_else(|e| {
            log::warn!("Failed to parse {filename}: {e}");
            Vec::new()
        }),
        Err(e) => {
            log::warn!("Failed to read {}: {}", path.display(), e);
            Vec::new()
        }
    }
}

pub(super) fn find_map_prefix<R: Read + std::io::Seek>(
    archive: &zip::ZipArchive<R>,
) -> Result<String, MapError> {
    for i in 0..archive.len() {
        if let Some(file) = archive.name_for_index(i)
            && let Some(prefix) = file.strip_suffix("game.map")
        {
            return Ok(prefix.to_string());
        }
    }
    Err(MapError::NoGameMap)
}

pub(super) fn read_zip_file<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, MapError> {
    let mut file = archive.by_name(name)?;
    let mut buf = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut buf).map_err(|e| MapError::Io {
        context: format!("reading {name} from archive"),
        source: e,
    })?;
    Ok(buf)
}

/// Read an optional JSON sidecar from a zip archive.
///
/// `None` means the entry was missing, letting the caller fall back to a
/// legacy `.bjo`. Parse failure returns `Some(Vec::new())` so we don't
/// quietly substitute stale binary data for a broken JSON.
pub(super) fn read_zip_json<T, R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    parser: fn(&str) -> Result<Vec<T>, MapError>,
) -> Option<Vec<T>> {
    match read_zip_file(archive, name) {
        Ok(bytes) => {
            let content = String::from_utf8_lossy(&bytes);
            Some(parser(&content).unwrap_or_else(|e| {
                log::warn!("Failed to parse {name} in archive: {e}");
                Vec::new()
            }))
        }
        Err(_) => None,
    }
}

/// Read an optional `.bjo` sidecar from a zip archive. Empty vec on any failure.
pub(super) fn read_zip_bjo<T, R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    map_max_players: u32,
    parser: fn(&[u8], u32) -> Result<Vec<T>, MapError>,
) -> Vec<T> {
    match read_zip_file(archive, name) {
        Ok(bytes) => parser(&bytes, map_max_players).unwrap_or_else(|e| {
            log::warn!("Failed to parse {name} in archive: {e}");
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain_types::TerrainType;

    #[test]
    fn parse_player_count_standard() {
        assert_eq!(parse_player_count("2c-MapName"), 2);
        assert_eq!(parse_player_count("4c-Sk-Rush"), 4);
        assert_eq!(parse_player_count("8c-Sk-Startup"), 8);
        assert_eq!(parse_player_count("10c-BigMap"), 10);
    }

    #[test]
    fn parse_player_count_no_prefix() {
        assert_eq!(parse_player_count("MapName"), 0);
        assert_eq!(parse_player_count(""), 0);
    }

    #[test]
    fn parse_player_count_no_hyphen() {
        assert_eq!(parse_player_count("2cMapName"), 0);
    }

    #[test]
    fn parse_player_count_no_number() {
        assert_eq!(parse_player_count("c-MapName"), 0);
    }

    #[test]
    fn parse_player_count_non_numeric() {
        assert_eq!(parse_player_count("abc-MapName"), 0);
    }

    #[test]
    fn detect_tileset_arizona() {
        let ttp = TerrainTypeData {
            terrain_types: vec![
                TerrainType::SandYellow,
                TerrainType::Sand,
                TerrainType::Bakedearth,
            ],
        };
        assert_eq!(detect_tileset_from_ttp(Some(&ttp)), "arizona");
    }

    #[test]
    fn detect_tileset_urban() {
        let ttp = TerrainTypeData {
            terrain_types: vec![
                TerrainType::Bakedearth,
                TerrainType::Bakedearth,
                TerrainType::Bakedearth,
            ],
        };
        assert_eq!(detect_tileset_from_ttp(Some(&ttp)), "urban");
    }

    #[test]
    fn detect_tileset_rockies() {
        let ttp = TerrainTypeData {
            terrain_types: vec![
                TerrainType::Sand,
                TerrainType::Sand,
                TerrainType::Bakedearth,
            ],
        };
        assert_eq!(detect_tileset_from_ttp(Some(&ttp)), "rockies");
    }

    #[test]
    fn detect_tileset_none_defaults_to_arizona() {
        assert_eq!(detect_tileset_from_ttp(None), "arizona");
    }
}
