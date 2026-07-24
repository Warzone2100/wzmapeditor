//! Map loading, saving, and auto-save management.

use super::EditorApp;
use crate::map::document::MapDocument;

/// Load a new map document and mark terrain for re-upload.
///
/// `source_path` - where the map was loaded from (persisted for auto-reload
/// on next launch). `save_path` - writable location for Ctrl+S quick-save;
/// `None` for maps loaded from read-only game archives.
pub(super) fn load_map(
    app: &mut EditorApp,
    mut map: wz_maplib::WzMap,
    source_path: Option<std::path::PathBuf>,
    save_path: Option<std::path::PathBuf>,
    archive_prefix: Option<String>,
) {
    // Rewrite CWall stat entries to their paired base WALL stat. New saves
    // then contain base-only names, which the patched WZ2100 loader needs
    // for structChooseWallType to pick the right L/T/cross variant in-game.
    let migrated = crate::tools::wall_tool::migrate_corner_walls_to_base(&mut map.structures);
    if migrated > 0 {
        log::info!("Rewrote {migrated} corner-wall stat entries to their base wall");
    }

    let name = map.map_name.clone();
    let map_players = map.players;
    let n_structs = map.structures.len();
    let n_droids = map.droids.len();
    let n_feats = map.features.len();

    let new_tileset = if let Some(ref ttp) = map.terrain_types {
        let raw: Vec<u16> = ttp.terrain_types.iter().map(|t| *t as u16).collect();
        crate::config::Tileset::from_terrain_types(&raw)
    } else {
        crate::config::Tileset::Arizona
    };
    let tileset_changed = new_tileset != app.current_tileset;
    app.current_tileset = new_tileset;

    // Repair maps saved with the editor's old 3-entry terrain table (or with
    // none at all): backfill the full canonical table for the detected tileset
    // so cliff and water tiles are typed correctly for the tileset browser,
    // the heatmap, and in-game pathfinding. Real maps ship the full per-texture
    // table and are left untouched. The tileset is detected above from the
    // first-3 signature, which a 3-entry table still carries.
    let ttp_repaired = match &map.terrain_types {
        Some(ttp) => ttp.terrain_types.len() <= 3,
        None => true,
    };
    if ttp_repaired {
        map.terrain_types = Some(wz_maplib::TerrainTypeData {
            terrain_types: new_tileset.full_terrain_types(),
        });
    }

    // Persist the tileset so next startup pre-loads the right one.
    let new_name = new_tileset.as_str();
    if app.config.last_tileset.as_deref() != Some(new_name) {
        app.config.last_tileset = Some(new_name.to_string());
        app.config.save();
    }

    // Clear ground data when tileset changes (will be reloaded on demand).
    if tileset_changed {
        app.ground_data = None;
        app.rt.ground_texture_load = None;
    }
    // Always clear tile pools on map load - they will be rebuilt once the
    // correct tileset's ground data finishes loading.
    app.tool_state.tile_pools.clear();

    app.render_settings.fog_color = match new_tileset {
        crate::config::Tileset::Arizona => crate::viewport::renderer::FOG_ARIZONA,
        crate::config::Tileset::Urban => crate::viewport::renderer::FOG_URBAN,
        crate::config::Tileset::Rockies => crate::viewport::renderer::FOG_ROCKIES,
    };

    // Re-scope custom templates: clear the previous map's overlay from the
    // stats DB and register templates.json entries from the new map.
    if let Some(ref mut stats) = app.stats {
        app.custom_templates.clear(stats);
        if let Some(ref json) = map.custom_templates_json {
            app.custom_templates.load_from_json(json, stats);
        }
    }

    map.map_name = strip_player_count_prefix(&map.map_name).to_string();
    app.document = Some(MapDocument::new(map));
    // Mark repaired maps dirty so the backfilled terrain table is written on
    // the next save, fixing the map on disk for in-game use.
    if ttp_repaired && let Some(doc) = app.document.as_mut() {
        doc.dirty = true;
    }
    // Tile pools are rebuilt lazily by ensure_tile_pools() in update().
    app.terrain_dirty = true;
    app.terrain_dirty_tiles.clear();
    app.lightmap_dirty = true;
    app.objects_dirty = true;
    app.shadow_dirty = true;
    app.water_dirty = true;
    app.minimap.dirty = true;
    app.selection.clear();
    app.balance.clear();
    app.autosave.cleanup();
    app.autosave.reset_timer();
    app.save_path = save_path;
    app.map_players = if map_players > 0 {
        map_players
    } else {
        parse_player_count_from_name(&name)
    };
    if let Some(path) = source_path {
        app.config.last_opened_map = Some(path);
        app.config.last_opened_map_prefix = archive_prefix;
        app.config.save();
    }

    if tileset_changed {
        app.tileset = None;
        app.rt.tileset_load_attempted = false;
        app.model_thumbnails
            .switch_tileset(&format!("{new_tileset:?}"));
        app.log(format!("Tileset: {new_tileset:?}"));
    }

    app.log(format!(
        "Loaded map: {name} ({n_structs} structures, {n_droids} droids, {n_feats} features)"
    ));
    if ttp_repaired {
        app.log(format!(
            "Repaired incomplete terrain table for this {new_tileset} map; save to keep the fix"
        ));
    }

    app.run_validation();
}

/// Save to the remembered path (quick save). Returns false if no path is set.
pub(super) fn save_to_current(app: &mut EditorApp) -> bool {
    let Some(ref path) = app.save_path else {
        return false;
    };
    let path = path.clone();
    if path.extension().is_some_and(|ext| ext == "wz") {
        save_to_wz(app, &path);
    } else {
        save_to_directory(app, &path);
    }
    true
}

/// Run validation before save and log a warning if problems exist.
pub(super) fn warn_on_validation_problems(app: &mut EditorApp) {
    app.run_validation();
    if app
        .validation_results
        .as_ref()
        .is_some_and(wz_maplib::validate::ValidationResults::has_problems)
    {
        let n = app
            .validation_results
            .as_ref()
            .map_or(0, wz_maplib::validate::ValidationResults::problem_count);
        app.log(format!(
            "WARNING: Map has {n} validation problems. Saving anyway. Check the Problems tab."
        ));
    }
}

/// Save the current map to a directory on disk.
pub(super) fn save_to_directory(app: &mut EditorApp, path: &std::path::Path) {
    warn_on_validation_problems(app);
    let Some(ref doc) = app.document else {
        return;
    };
    match wz_maplib::io_wz::save_to_directory(&doc.map, path, wz_maplib::OutputFormat::Ver3) {
        Ok(()) => {
            app.save_path = Some(path.to_path_buf());
            app.log(format!("Saved map folder to {}", path.display()));
            if let Some(ref mut doc) = app.document {
                doc.mark_clean();
            }
            app.autosave.cleanup();
        }
        Err(e) => app.log(format!("Failed to save: {e}")),
    }
}

/// Return the current tileset as a lowercase string for `level.json`.
pub(super) fn current_tileset_name(app: &EditorApp) -> String {
    if let Some(ref doc) = app.document
        && let Some(ref tt) = doc.map.terrain_types
    {
        let raw: Vec<u16> = tt.terrain_types.iter().map(|t| *t as u16).collect();
        return match crate::config::Tileset::from_terrain_types(&raw) {
            crate::config::Tileset::Arizona => "arizona",
            crate::config::Tileset::Urban => "urban",
            crate::config::Tileset::Rockies => "rockies",
        }
        .to_string();
    }
    "arizona".to_string()
}

/// Strip a WZ2100 `"Nc-"` or `"Np-"` player-count prefix from a map name.
pub(crate) fn strip_player_count_prefix(name: &str) -> &str {
    ["c-", "p-"]
        .into_iter()
        .find_map(|sep| {
            name.find(sep).and_then(|idx| {
                (idx > 0 && name[..idx].chars().all(|c| c.is_ascii_digit()))
                    .then(|| &name[idx + sep.len()..])
            })
        })
        .unwrap_or(name)
}

/// Build a suggested `.wz` filename from the map name and player count.
pub(super) fn suggested_wz_filename(app: &EditorApp) -> String {
    let raw_name = app
        .document
        .as_ref()
        .map_or("NewMap", |d| d.map.map_name.as_str());
    let base = strip_player_count_prefix(raw_name);
    format!("{}c-{base}.wz", app.map_players)
}

/// Save the current map as a `.wz` archive.
pub(super) fn save_to_wz(app: &mut EditorApp, path: &std::path::Path) {
    warn_on_validation_problems(app);
    let tileset_name = current_tileset_name(app);
    let players = app.map_players;
    let custom_templates_json = app.custom_templates.to_json();
    let Some(ref mut doc) = app.document else {
        return;
    };
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        doc.map.map_name = strip_player_count_prefix(stem).to_string();
    }
    doc.map.players = players;
    doc.map.tileset = tileset_name;
    doc.map.custom_templates_json = custom_templates_json;
    match write_wz(&doc.map, path) {
        Ok(()) => {
            app.save_path = Some(path.to_path_buf());
            #[cfg(target_arch = "wasm32")]
            app.log(format!("Downloaded .wz archive: {}", path.display()));
            #[cfg(not(target_arch = "wasm32"))]
            app.log(format!("Saved .wz archive to {}", path.display()));
            if let Some(ref mut doc) = app.document {
                doc.mark_clean();
            }
            app.autosave.cleanup();
        }
        Err(e) => app.log(format!("Failed to save .wz: {e}")),
    }
}

/// Write the `.wz` archive to `path` on disk.
#[cfg(not(target_arch = "wasm32"))]
fn write_wz(map: &wz_maplib::WzMap, path: &std::path::Path) -> Result<(), String> {
    wz_maplib::io_wz::save_to_wz_archive(map, path, wz_maplib::OutputFormat::Ver3)
        .map_err(|e| e.to_string())
}

/// Serialize the `.wz` archive and trigger a browser download.
#[cfg(target_arch = "wasm32")]
fn write_wz(map: &wz_maplib::WzMap, path: &std::path::Path) -> Result<(), String> {
    let bytes = wz_maplib::io_wz::save_to_wz_writer(
        map,
        std::io::Cursor::new(Vec::new()),
        wz_maplib::OutputFormat::Ver3,
    )
    .map(std::io::Cursor::into_inner)
    .map_err(|e| e.to_string())?;
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("map.wz");
    crate::web_map_io::download(filename, &bytes)
}

/// Poll background auto-save completion.
pub(super) fn poll_autosave(app: &mut EditorApp) {
    if let Some(result) = app.autosave.poll() {
        match result {
            Ok(()) => log::info!("Auto-save complete"),
            Err(ref msg) => log::warn!("Auto-save failed: {msg}"),
        }
    }
}

/// Check whether it's time to auto-save and kick one off if needed.
pub(super) fn tick_autosave(app: &mut EditorApp) {
    if !app.config.autosave_enabled {
        return;
    }
    let doc_dirty = app.document.as_ref().is_some_and(|d| d.dirty);
    if !doc_dirty || !app.autosave.should_save(app.config.autosave_interval_secs) {
        return;
    }
    let doc = app.document.as_ref().expect("checked above");
    app.autosave
        .start_save(&doc.map, app.save_path.as_deref(), app.map_players);
    log::info!("Auto-save started");
}

/// Parse player count from a WZ2100 map name prefix (e.g. "2c-MapName"
/// or "2p-AValley" -> 2).
pub(super) fn parse_player_count_from_name(name: &str) -> u8 {
    for sep in ["c-", "p-"] {
        if let Some(idx) = name.find(sep)
            && idx > 0
            && let Ok(n) = name[..idx].parse::<u8>()
            && n > 0
        {
            return n;
        }
    }
    // Default to 2 for maps without the convention.
    2
}

#[cfg(test)]
mod tests {
    use super::strip_player_count_prefix;

    #[test]
    fn strips_player_count_prefixes() {
        assert_eq!(strip_player_count_prefix("2c-FunMap"), "FunMap");
        assert_eq!(strip_player_count_prefix("4c-Sk-Rush"), "Sk-Rush");
        assert_eq!(strip_player_count_prefix("10p-Big"), "Big");
    }

    #[test]
    fn leaves_names_without_a_valid_prefix_unchanged() {
        assert_eq!(strip_player_count_prefix("FunMap"), "FunMap");
        assert_eq!(strip_player_count_prefix("2cFunMap"), "2cFunMap");
        assert_eq!(strip_player_count_prefix("c-Foo"), "c-Foo");
        assert_eq!(strip_player_count_prefix(""), "");
    }
}
