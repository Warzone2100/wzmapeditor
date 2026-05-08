//! Campaign-specific loading on top of the regular archive reader.
//!
//! WZ2100 campaign archives use `gamedesc.lev` as a manifest of named
//! levels and split each mission across `camstart`, `miss_keep`, and
//! `expand` folders. `expand` levels reuse the terrain of the previous
//! mission while overlaying their own objects, so the loader has to
//! splice base terrain with overlay objects rather than reading a single
//! self-contained folder.

use std::path::Path;

use crate::MapError;
use crate::io_binary;
use crate::io_bjo;
use crate::io_json;
use crate::io_lev::{CampaignIndex, LevelKind};
use crate::io_ttp;

use super::WzMap;
use super::archive::load_map_from_archive_prefix;
use super::common::{detect_tileset_from_ttp, read_zip_bjo, read_zip_file, read_zip_json};

/// Load a campaign level by name, handling `expand` overlays that reuse a
/// prior mission's terrain.
///
/// For `camstart` / `miss_keep` this delegates to the regular prefix
/// loader. For `expand`, terrain (`game.map` + `ttypes.ttp`) is read from
/// the resolved base folder while objects and labels come from the
/// overlay folder. Gateways live in the base terrain binary.
pub fn load_campaign_level(
    wz_path: &Path,
    index: &CampaignIndex,
    level_name: &str,
) -> Result<WzMap, MapError> {
    let resolved = index.find(level_name).ok_or_else(|| {
        MapError::JsonFormat(format!("campaign level {level_name} not found in index"))
    })?;

    // Full missions load with the standard reader; `load_map_from_archive_prefix`
    // already handles JSON/BJO fallback, level.json, terrain types, labels.
    if !matches!(resolved.kind, LevelKind::Expand) {
        let mut map = load_map_from_archive_prefix(wz_path, &resolved.folder)?;
        // Prefer the manifest's level name over the folder stem.
        map.map_name.clone_from(&resolved.name);
        return Ok(map);
    }

    let base_folder = resolved.base_folder.as_deref().ok_or_else(|| {
        MapError::JsonFormat(format!("expand level {level_name} has no base folder"))
    })?;

    let file = std::fs::File::open(wz_path).map_err(|e| MapError::Io {
        context: format!("opening {}", wz_path.display()),
        source: e,
    })?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Terrain (game.map + ttypes.ttp) comes from the base mission.
    let map_bytes = read_zip_file(&mut archive, &format!("{base_folder}game.map"))?;
    let map_data = io_binary::read_game_map(&map_bytes)?;
    let terrain_types = match read_zip_file(&mut archive, &format!("{base_folder}ttypes.ttp")) {
        Ok(bytes) => Some(io_ttp::read_ttp(&bytes)?),
        Err(_) => None,
    };

    // Campaign missions have no Nc- prefix; the BJO scavenger-slot formula
    // (max(map_max_players, 7)) still resolves correctly with 1.
    let max_players: u32 = 1;

    let overlay_folder = resolved.folder.as_str();
    let structures = read_zip_json(
        &mut archive,
        &format!("{overlay_folder}struct.json"),
        io_json::read_structures,
    )
    .unwrap_or_else(|| {
        read_zip_bjo(
            &mut archive,
            &format!("{overlay_folder}struct.bjo"),
            max_players,
            io_bjo::read_structures,
        )
    });
    let droids = read_zip_json(
        &mut archive,
        &format!("{overlay_folder}droid.json"),
        io_json::read_droids,
    )
    .unwrap_or_else(|| {
        read_zip_bjo(
            &mut archive,
            &format!("{overlay_folder}dinit.bjo"),
            max_players,
            io_bjo::read_droids,
        )
    });
    let features = read_zip_json(
        &mut archive,
        &format!("{overlay_folder}feature.json"),
        io_json::read_features,
    )
    .unwrap_or_else(|| {
        read_zip_bjo(
            &mut archive,
            &format!("{overlay_folder}feat.bjo"),
            max_players,
            io_bjo::read_features,
        )
    });

    let labels = match read_zip_file(&mut archive, &format!("{overlay_folder}labels.json")) {
        Ok(bytes) => crate::labels::read_labels(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let tileset = detect_tileset_from_ttp(terrain_types.as_ref());

    Ok(WzMap {
        map_data,
        structures,
        droids,
        features,
        terrain_types,
        labels,
        map_name: resolved.name.clone(),
        players: 0,
        tileset,
        custom_templates_json: None,
        author: None,
        additional_authors: Vec::new(),
        license: None,
    })
}

/// Convenience wrapper that parses `gamedesc.lev` from the archive and
/// loads the requested campaign level in one call.
pub fn load_campaign_level_by_name(wz_path: &Path, level_name: &str) -> Result<WzMap, MapError> {
    let index = read_campaign_index(wz_path)?;
    load_campaign_level(wz_path, &index, level_name)
}

/// Read `gamedesc.lev` from the root of a `.wz` archive and build the
/// resolved campaign index.
pub fn read_campaign_index(wz_path: &Path) -> Result<CampaignIndex, MapError> {
    let file = std::fs::File::open(wz_path).map_err(|e| MapError::Io {
        context: format!("opening {}", wz_path.display()),
        source: e,
    })?;
    let mut archive = zip::ZipArchive::new(file)?;
    let bytes = read_zip_file(&mut archive, "gamedesc.lev")?;
    let text = String::from_utf8_lossy(&bytes);
    let entries = crate::io_lev::parse_gamedesc(&text)?;
    Ok(crate::io_lev::build_index(&entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_campaign_level_unknown_name_errors() {
        // Loader must fail cleanly before touching the filesystem.
        let index = CampaignIndex { levels: Vec::new() };
        let err = load_campaign_level(Path::new("/nonexistent.wz"), &index, "CAM_DOES_NOT_EXIST")
            .unwrap_err();
        assert!(matches!(err, MapError::JsonFormat(ref m) if m.contains("CAM_DOES_NOT_EXIST")));
    }
}
