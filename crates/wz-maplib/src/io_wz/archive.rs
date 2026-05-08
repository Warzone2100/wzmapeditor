//! Load, save, and extract `.wz` map archives.
//!
//! `.wz` files are zip archives that hold either a single map flattened at
//! the root or one (or many) maps under a path prefix such as
//! `multiplay/maps/2c-Foo/`. The loader uses `level.json` for canonical
//! metadata when present and falls back to filename heuristics otherwise.
//! The extractor variants handle the base/terrain-overlay layering needed
//! to assemble the editor's data cache.

use std::path::Path;

use crate::MapError;
use crate::io_binary::{self, OutputFormat};
use crate::io_bjo;
use crate::io_json;
use crate::io_ttp;

use super::WzMap;
use super::common::{
    detect_tileset_from_ttp, find_map_prefix, parse_player_count, read_zip_bjo, read_zip_file,
    read_zip_json,
};
use super::level_json::{build_gam_json, build_level_json, read_level_json};

/// Load a map from a .wz archive (zip file).
pub fn load_from_wz_archive(wz_path: &Path) -> Result<WzMap, MapError> {
    let file = std::fs::File::open(wz_path).map_err(|e| MapError::Io {
        context: format!("opening {}", wz_path.display()),
        source: e,
    })?;
    let mut archive = zip::ZipArchive::new(file)?;

    let map_prefix = find_map_prefix(&archive)?;
    let map_name = {
        let from_prefix = map_prefix
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("");
        if from_prefix.is_empty() {
            // Flattened archive: derive name from the .wz filename.
            wz_path.file_stem().map_or_else(
                || "unnamed".to_string(),
                |s| s.to_string_lossy().to_string(),
            )
        } else {
            from_prefix.to_string()
        }
    };

    let map_bytes = read_zip_file(&mut archive, &format!("{map_prefix}game.map"))?;
    let map_data = io_binary::read_game_map(&map_bytes)?;

    // Player count is needed up front for BJO scavenger-slot conversion.
    let level_meta = read_level_json(&mut archive, &map_prefix);
    let max_players = u32::from(
        level_meta
            .as_ref()
            .map_or_else(|| parse_player_count(&map_name), |m| m.players)
            .max(1),
    );

    let structures = read_zip_json(
        &mut archive,
        &format!("{map_prefix}struct.json"),
        io_json::read_structures,
    )
    .unwrap_or_else(|| {
        read_zip_bjo(
            &mut archive,
            &format!("{map_prefix}struct.bjo"),
            max_players,
            io_bjo::read_structures,
        )
    });

    let droids = read_zip_json(
        &mut archive,
        &format!("{map_prefix}droid.json"),
        io_json::read_droids,
    )
    .unwrap_or_else(|| {
        read_zip_bjo(
            &mut archive,
            &format!("{map_prefix}dinit.bjo"),
            max_players,
            io_bjo::read_droids,
        )
    });

    let features = read_zip_json(
        &mut archive,
        &format!("{map_prefix}feature.json"),
        io_json::read_features,
    )
    .unwrap_or_else(|| {
        read_zip_bjo(
            &mut archive,
            &format!("{map_prefix}feat.bjo"),
            max_players,
            io_bjo::read_features,
        )
    });

    let terrain_types = match read_zip_file(&mut archive, &format!("{map_prefix}ttypes.ttp")) {
        Ok(bytes) => Some(io_ttp::read_ttp(&bytes)?),
        Err(_) => None,
    };

    let labels = match read_zip_file(&mut archive, &format!("{map_prefix}labels.json")) {
        Ok(bytes) => crate::labels::read_labels(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let custom_templates_json = read_zip_file(&mut archive, &format!("{map_prefix}templates.json"))
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());

    let (map_name, players, tileset, author, additional_authors, license) =
        if let Some(meta) = level_meta {
            (
                meta.name,
                meta.players,
                meta.tileset,
                meta.author,
                meta.additional_authors,
                meta.license,
            )
        } else {
            let p = parse_player_count(&map_name);
            let t = detect_tileset_from_ttp(terrain_types.as_ref());
            (map_name, p, t, None, Vec::new(), None)
        };

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
        author,
        additional_authors,
        license,
    })
}

/// Save a map as a .wz archive (zip file).
pub fn save_to_wz_archive(
    map: &WzMap,
    wz_path: &Path,
    format: OutputFormat,
) -> Result<(), MapError> {
    let file = std::fs::File::create(wz_path).map_err(|e| MapError::Io {
        context: format!("creating {}", wz_path.display()),
        source: e,
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Flattened layout (files at archive root, with level.json) matches the
    // community-map format WZ2100 recognizes at runtime.
    let io_err = |e: std::io::Error| MapError::Io {
        context: "writing archive".into(),
        source: e,
    };

    let level_json = build_level_json(map);
    zip.start_file("level.json", options)?;
    std::io::Write::write_all(&mut zip, level_json.as_bytes()).map_err(&io_err)?;

    let gam_json = build_gam_json(&map.map_data);
    zip.start_file("gam.json", options)?;
    std::io::Write::write_all(&mut zip, gam_json.as_bytes()).map_err(&io_err)?;

    let map_bytes = io_binary::write_game_map(&map.map_data, format)?;
    zip.start_file("game.map", options)?;
    std::io::Write::write_all(&mut zip, &map_bytes).map_err(&io_err)?;

    let struct_json = io_json::write_structures(&map.structures)?;
    zip.start_file("struct.json", options)?;
    std::io::Write::write_all(&mut zip, struct_json.as_bytes()).map_err(&io_err)?;

    let droid_json = io_json::write_droids(&map.droids)?;
    zip.start_file("droid.json", options)?;
    std::io::Write::write_all(&mut zip, droid_json.as_bytes()).map_err(&io_err)?;

    let feature_json = io_json::write_features(&map.features)?;
    zip.start_file("feature.json", options)?;
    std::io::Write::write_all(&mut zip, feature_json.as_bytes()).map_err(&io_err)?;

    if let Some(ref tt) = map.terrain_types {
        let ttp_bytes = io_ttp::write_ttp(tt)?;
        zip.start_file("ttypes.ttp", options)?;
        std::io::Write::write_all(&mut zip, &ttp_bytes).map_err(&io_err)?;
    }

    if !map.labels.is_empty() {
        let labels_bytes = crate::labels::write_labels(&map.labels)?;
        zip.start_file("labels.json", options)?;
        std::io::Write::write_all(&mut zip, &labels_bytes).map_err(&io_err)?;
    }

    if let Some(ref json) = map.custom_templates_json {
        zip.start_file("templates.json", options)?;
        std::io::Write::write_all(&mut zip, json.as_bytes()).map_err(&io_err)?;
    }

    zip.finish()?;
    Ok(())
}

/// Load a specific map from a multi-map `.wz` archive using its prefix.
///
/// The prefix is the internal path (e.g. `multiplay/maps/2c-startup/`)
/// obtained from `MapPreview::archive_prefix`.
pub fn load_map_from_archive_prefix(wz_path: &Path, prefix: &str) -> Result<WzMap, MapError> {
    let file = std::fs::File::open(wz_path).map_err(|e| MapError::Io {
        context: format!("opening {}", wz_path.display()),
        source: e,
    })?;
    let mut archive = zip::ZipArchive::new(file)?;

    let map_name = prefix
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("unnamed")
        .to_string();

    let map_bytes = read_zip_file(&mut archive, &format!("{prefix}game.map"))?;
    let map_data = io_binary::read_game_map(&map_bytes)?;

    let level_meta = read_level_json(&mut archive, prefix);
    let max_players = u32::from(
        level_meta
            .as_ref()
            .map_or_else(|| parse_player_count(&map_name), |m| m.players)
            .max(1),
    );

    let structures = read_zip_json(
        &mut archive,
        &format!("{prefix}struct.json"),
        io_json::read_structures,
    )
    .unwrap_or_else(|| {
        read_zip_bjo(
            &mut archive,
            &format!("{prefix}struct.bjo"),
            max_players,
            io_bjo::read_structures,
        )
    });
    let droids = read_zip_json(
        &mut archive,
        &format!("{prefix}droid.json"),
        io_json::read_droids,
    )
    .unwrap_or_else(|| {
        read_zip_bjo(
            &mut archive,
            &format!("{prefix}dinit.bjo"),
            max_players,
            io_bjo::read_droids,
        )
    });
    let features = read_zip_json(
        &mut archive,
        &format!("{prefix}feature.json"),
        io_json::read_features,
    )
    .unwrap_or_else(|| {
        read_zip_bjo(
            &mut archive,
            &format!("{prefix}feat.bjo"),
            max_players,
            io_bjo::read_features,
        )
    });
    let terrain_types = match read_zip_file(&mut archive, &format!("{prefix}ttypes.ttp")) {
        Ok(bytes) => Some(io_ttp::read_ttp(&bytes)?),
        Err(_) => None,
    };

    let labels = match read_zip_file(&mut archive, &format!("{prefix}labels.json")) {
        Ok(bytes) => crate::labels::read_labels(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let (map_name, players, tileset, author, additional_authors, license) =
        if let Some(meta) = level_meta {
            (
                meta.name,
                meta.players,
                meta.tileset,
                meta.author,
                meta.additional_authors,
                meta.license,
            )
        } else {
            let p = parse_player_count(&map_name);
            let t = detect_tileset_from_ttp(terrain_types.as_ref());
            (map_name, p, t, None, Vec::new(), None)
        };

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
        custom_templates_json: None,
        author,
        additional_authors,
        license,
    })
}

/// Extract all files from a `.wz` archive into a directory on disk.
///
/// Preserves the archive's internal directory structure. Already-extracted
/// files are skipped, so the operation is safe to repeat on a partial cache.
/// `progress_cb` is called with a value in `[0.0, 1.0]` after each entry.
pub fn extract_wz_to_dir(
    wz_path: &Path,
    output_dir: &Path,
    progress_cb: impl Fn(f32),
) -> Result<(), MapError> {
    extract_wz_to_dir_impl(
        wz_path,
        output_dir,
        false,
        None::<fn(&str) -> bool>,
        progress_cb,
    )
}

/// Extract a `.wz` archive, overwriting existing files.
///
/// Used to overlay `terrain_overrides/classic.wz` pre-composited
/// tiles on top of `base.wz` RGBA tiles.
pub fn extract_wz_to_dir_overwrite(
    wz_path: &Path,
    output_dir: &Path,
    progress_cb: impl Fn(f32),
) -> Result<(), MapError> {
    extract_wz_to_dir_impl(
        wz_path,
        output_dir,
        true,
        None::<fn(&str) -> bool>,
        progress_cb,
    )
}

/// Extract only entries whose name satisfies `filter`, overwriting existing files.
///
/// Used for `terrain_overrides/high.wz` to extract only `_nm` and `_sm`
/// texture variants without overwriting the base diffuse textures.
pub fn extract_wz_to_dir_filtered(
    wz_path: &Path,
    output_dir: &Path,
    filter: impl Fn(&str) -> bool,
    progress_cb: impl Fn(f32),
) -> Result<(), MapError> {
    extract_wz_to_dir_impl(wz_path, output_dir, true, Some(filter), progress_cb)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "private impl, takes ownership for caller convenience"
)]
fn extract_wz_to_dir_impl(
    wz_path: &Path,
    output_dir: &Path,
    overwrite: bool,
    filter: Option<impl Fn(&str) -> bool>,
    progress_cb: impl Fn(f32),
) -> Result<(), MapError> {
    let file = std::fs::File::open(wz_path).map_err(|e| MapError::Io {
        context: format!("opening {}", wz_path.display()),
        source: e,
    })?;
    let mut archive = zip::ZipArchive::new(file)?;
    let total = archive.len();

    for i in 0..total {
        let mut entry = archive.by_index(i)?;
        let out_path = output_dir.join(entry.name());

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| MapError::Io {
                context: format!("creating directory {}", out_path.display()),
                source: e,
            })?;
        } else {
            if let Some(ref f) = filter
                && !f(entry.name())
            {
                progress_cb((i + 1) as f32 / total as f32);
                continue;
            }
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| MapError::Io {
                    context: format!("creating parent directory {}", parent.display()),
                    source: e,
                })?;
            }
            // Skipping existing files keeps extraction resumable; overwrite
            // is reserved for the classic.wz / high.wz overlays.
            if overwrite || !out_path.exists() {
                let mut out_file = std::fs::File::create(&out_path).map_err(|e| MapError::Io {
                    context: format!("creating {}", out_path.display()),
                    source: e,
                })?;
                std::io::copy(&mut entry, &mut out_file).map_err(|e| MapError::Io {
                    context: format!("writing {}", out_path.display()),
                    source: e,
                })?;
            }
        }

        progress_cb((i + 1) as f32 / total as f32);
    }

    Ok(())
}

/// Read a single file's raw bytes from a `.wz` archive by entry name.
///
/// Returns `None` if the entry does not exist. `entry_name` must match
/// the zip entry exactly (e.g. `"texpages/tertilesc1hw-128/tile-13.png"`).
pub fn read_wz_entry(wz_path: &Path, entry_name: &str) -> Option<Vec<u8>> {
    let file = std::fs::File::open(wz_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(entry_name).ok()?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    std::io::Read::read_to_end(&mut entry, &mut buf).ok()?;
    Some(buf)
}

/// A handle to an opened `.wz` archive for reading multiple entries without
/// re-opening the file each time.
#[derive(Debug)]
pub struct WzArchiveReader {
    archive: zip::ZipArchive<std::fs::File>,
}

impl WzArchiveReader {
    /// Open a `.wz` archive for batch reading.
    ///
    /// Returns `None` if the file cannot be opened or is not a valid zip.
    pub fn open(wz_path: &Path) -> Option<Self> {
        let file = std::fs::File::open(wz_path).ok()?;
        let archive = zip::ZipArchive::new(file).ok()?;
        Some(Self { archive })
    }

    /// Read a single entry's raw bytes by name.
    ///
    /// Returns `None` if the entry does not exist.
    pub fn read_entry(&mut self, entry_name: &str) -> Option<Vec<u8>> {
        let mut entry = self.archive.by_name(entry_name).ok()?;
        let mut buf = Vec::with_capacity(entry.size() as usize);
        std::io::Read::read_to_end(&mut entry, &mut buf).ok()?;
        Some(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extract_wz_filtered_only_extracts_matching_files() {
        let dir = std::env::temp_dir().join("wz_test_filtered_extract");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        let wz_path = dir.join("test.wz");
        {
            let file = std::fs::File::create(&wz_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("texpages/page-1.ktx2", opts).unwrap();
            zip.write_all(b"diffuse data").unwrap();
            zip.start_file("texpages/page-1_nm.ktx2", opts).unwrap();
            zip.write_all(b"normal data").unwrap();
            zip.start_file("texpages/page-1_sm.ktx2", opts).unwrap();
            zip.write_all(b"specular data").unwrap();
            zip.finish().unwrap();
        }

        let output = dir.join("output");
        extract_wz_to_dir_filtered(
            &wz_path,
            &output,
            |name| name.contains("_nm") || name.contains("_sm"),
            |_| {},
        )
        .unwrap();

        assert!(!output.join("texpages/page-1.ktx2").exists());
        assert!(output.join("texpages/page-1_nm.ktx2").exists());
        assert!(output.join("texpages/page-1_sm.ktx2").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
