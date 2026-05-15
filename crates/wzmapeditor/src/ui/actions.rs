//! Shared action handlers invoked from both the menu bar and the toolbar.

use crate::app::EditorApp;
use crate::publish;

pub(crate) fn open_wz_dialog(app: &mut EditorApp) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Open .wz Map")
        .add_filter("WZ Map", &["wz"])
        .pick_file()
    {
        // Script maps need a seed; route them through the seed-prompt dialog
        // instead of trying the static-map loader first.
        if matches!(
            wz_maplib::io_wz::classify_wz_archive(&path),
            wz_maplib::io_wz::WzArchiveKind::ScriptMap
        ) {
            open_script_seed_dialog(app, path);
            return;
        }
        match wz_maplib::io_wz::load_from_wz_archive(&path) {
            Ok(map) => {
                let save = Some(path.clone());
                app.load_map(map, Some(path), save, None);
            }
            Err(e) => app.report_wz_load_error(&path, &e),
        }
    }
}

/// Open the seed-prompt dialog for a known script-map path. Called by the
/// import flow when the user first picks a `.wz` script archive.
pub(crate) fn open_script_seed_dialog(app: &mut EditorApp, path: std::path::PathBuf) {
    let seed = seed_suggestion();
    app.script_seed_dialog = crate::app::ScriptSeedDialog {
        open: true,
        source_path: path,
        seed_input: format!("{seed}"),
        error: None,
    };
}

/// Re-run the active script map with a fresh random seed, with no dialog.
/// Triggered by the "Re-roll seed" toolbar button.
pub(crate) fn reroll_script_seed(app: &mut EditorApp) {
    let Some(path) = app.document.as_ref().and_then(|d| d.script_source.clone()) else {
        return;
    };
    let seed = seed_suggestion();
    crate::app::dialogs::load_script_map_with_seed(app, &path, seed);
}

pub(crate) fn seed_suggestion() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = dur.subsec_nanos();
    let secs = dur.as_secs() as u32;
    nanos.wrapping_mul(2_654_435_761).wrapping_add(secs)
}

pub(crate) fn browse_maps(app: &mut EditorApp, ctx: &egui::Context) {
    if app.map_browser.maps.is_empty() {
        let game_dir = app.config.game_install_dir.clone();
        let data_dir = app.config.data_dir.clone();
        if let Some(ref gd) = game_dir {
            app.map_browser.scan_game_dirs(gd, data_dir.as_deref(), ctx);
        }
    }
    app.map_browser.open = true;
}

pub(crate) fn save_current_or_prompt(app: &mut EditorApp) {
    if !app.save_to_current() {
        save_as_wz(app);
    }
}

/// Validate `dir` as a WZ2100 install/data directory and load it.
///
/// Triggers either `set_data_dir` (extracted layout) or
/// `start_base_wz_extraction` (`base.wz` archive). If neither marker is
/// present, logs an error and leaves the existing config alone.
pub(crate) fn apply_data_directory(
    app: &mut EditorApp,
    ctx: &egui::Context,
    dir: std::path::PathBuf,
) {
    if !dir.is_dir() {
        app.log(format!("Not a directory: {}", dir.display()));
        return;
    }
    let has_base_dir =
        dir.join("base").join("stats").exists() || dir.join("base").join("texpages").exists();
    let base_wz = dir.join("base.wz");

    if has_base_dir {
        app.config.game_install_dir = Some(dir.clone());
        app.set_data_dir(dir, ctx);
    } else if base_wz.exists() {
        app.config.game_install_dir = Some(dir);
        app.config.save();
        app.start_base_wz_extraction(base_wz, ctx);
    } else {
        app.log(format!(
            "No base.wz or base/ tree found in: {}",
            dir.display()
        ));
    }
}

pub(crate) fn open_config_dir(app: &mut EditorApp) {
    let dir = crate::config::config_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        app.log_error(format!(
            "Failed to create config dir {}: {e}",
            dir.display()
        ));
        return;
    }

    #[cfg(target_os = "windows")]
    let opener = "explorer.exe";
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let opener = "xdg-open";

    match std::process::Command::new(opener).arg(&dir).spawn() {
        Ok(_) => app.log(format!("Opened config dir: {}", dir.display())),
        Err(e) => app.log_error(format!("Failed to open {}: {e}", dir.display())),
    }
}

pub(crate) fn undo(app: &mut EditorApp) {
    if let Some(ref mut doc) = app.document {
        if doc.is_read_only() {
            return;
        }
        let dirties_objects = doc.undo();
        app.terrain_dirty = true;
        app.minimap.dirty = true;
        if dirties_objects {
            app.objects_dirty = true;
        }
    }
}

pub(crate) fn redo(app: &mut EditorApp) {
    if let Some(ref mut doc) = app.document {
        if doc.is_read_only() {
            return;
        }
        let dirties_objects = doc.redo();
        app.terrain_dirty = true;
        app.minimap.dirty = true;
        if dirties_objects {
            app.objects_dirty = true;
        }
    }
}

pub(crate) fn import_wz(app: &mut EditorApp) {
    open_wz_dialog(app);
}

pub(crate) fn import_map_folder(app: &mut EditorApp) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Import Map Folder")
        .pick_folder()
    {
        match wz_maplib::io_wz::load_from_directory(&path) {
            Ok(map) => {
                let save = Some(path.clone());
                app.load_map(map, Some(path), save, None);
            }
            Err(e) => app.log_error(format!("Failed to load map folder: {e}")),
        }
    }
}

pub(crate) fn import_game_map(app: &mut EditorApp) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Import Binary game.map")
        .add_filter("game.map", &["map"])
        .pick_file()
    else {
        return;
    };

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            app.log_error(format!("Failed to read {}: {e}", path.display()));
            return;
        }
    };

    let map_data = match wz_maplib::io_binary::read_game_map(&bytes) {
        Ok(md) => md,
        Err(e) => {
            app.log_error(format!("Failed to parse game.map: {e}"));
            return;
        }
    };

    // Raw game.map is terrain only; force Save As so we don't overwrite
    // the source with an incomplete export.
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("ImportedMap")
        .to_string();
    let mut map = wz_maplib::WzMap::new(&stem, map_data.width, map_data.height);
    map.map_data = map_data;
    app.load_map(map, None, None, None);
    app.log("Imported game.map (terrain only). Tileset defaults to Arizona. Use Save As to write a complete map.");
}

pub(crate) fn save_as_wz(app: &mut EditorApp) {
    if app.document.is_none() {
        return;
    }
    crate::app::dialogs::open_save_as_metadata_dialog(app);
}

pub(crate) fn save_as_directory(app: &mut EditorApp) {
    if app.document.is_none() {
        return;
    }
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Save As Map Folder")
        .pick_folder()
    {
        app.save_to_directory(&path);
    }
}

pub(crate) fn save_as_game_map(app: &mut EditorApp) {
    let Some(ref doc) = app.document else {
        return;
    };
    let Some(path) = rfd::FileDialog::new()
        .set_title("Save As Binary game.map")
        .set_file_name("game.map")
        .add_filter("game.map", &["map"])
        .save_file()
    else {
        return;
    };
    let bytes = match wz_maplib::io_binary::write_game_map(
        &doc.map.map_data,
        wz_maplib::OutputFormat::Ver3,
    ) {
        Ok(b) => b,
        Err(e) => {
            app.log_error(format!("Failed to encode game.map: {e}"));
            return;
        }
    };
    match std::fs::write(&path, bytes) {
        Ok(()) => app.log(format!("Wrote game.map to {}", path.display())),
        Err(e) => app.log_error(format!("Failed to write {}: {e}", path.display())),
    }
}

pub(crate) fn publish_to_maps_db(app: &mut EditorApp) {
    let Some(ref doc) = app.document else {
        return;
    };

    let Some(wz_path) = app.save_path.clone() else {
        app.log_error("Save the map to a .wz file before publishing to the Maps Database.");
        return;
    };

    if !wz_path.is_file() {
        app.log_error(format!(
            "Saved map file is missing: {}. Save the map again before publishing.",
            wz_path.display(),
        ));
        return;
    }

    let map_name = if doc.map.map_name.trim().is_empty() {
        wz_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("UntitledMap")
            .to_string()
    } else {
        doc.map.map_name.clone()
    };

    continue_publish(app, &wz_path, &map_name);
}

fn continue_publish(app: &mut EditorApp, wz_path: &std::path::Path, map_name: &str) {
    let zip_path = match publish::write_wz_zip(wz_path) {
        Ok(p) => p,
        Err(e) => {
            app.log_error(format!("Failed to prepare .wz.zip for publish: {e}"));
            return;
        }
    };

    let submission_url = publish::submission_url(map_name);

    app.log(format!(
        "Prepared submission for \"{map_name}\". Wrote {}",
        zip_path.display(),
    ));
    app.publish_instructions_dialog = crate::app::PublishInstructionsDialog {
        open: true,
        zip_path,
        map_name: map_name.to_string(),
        submission_url,
        browser_opened: false,
    };
}

pub(crate) fn save_as_preview_png(app: &mut EditorApp) {
    let Some(ref doc) = app.document else {
        return;
    };

    let stem = app
        .suggested_wz_filename()
        .strip_suffix(".wz")
        .map_or_else(|| "preview".to_string(), str::to_string);
    let default_name = format!("{stem}_preview.png");

    let Some(path) = rfd::FileDialog::new()
        .set_title("Export Preview PNG")
        .set_file_name(default_name)
        .add_filter("PNG", &["png"])
        .save_file()
    else {
        return;
    };

    let Some(image) = crate::ui::minimap::build_minimap_image(
        &doc.map.map_data,
        doc.map.terrain_types.as_ref(),
        app.current_tileset,
    ) else {
        app.log_error("Cannot export preview: map has zero size");
        return;
    };

    let [w, h] = [image.size[0] as u32, image.size[1] as u32];
    let raw: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(egui::Color32::to_array)
        .collect();
    let Some(rgba) = image::RgbaImage::from_raw(w, h, raw) else {
        app.log_error("Failed to build preview pixel buffer");
        return;
    };

    match rgba.save(&path) {
        Ok(()) => app.log(format!("Wrote preview PNG: {}", path.display())),
        Err(e) => app.log_error(format!("Failed to write PNG: {e}")),
    }
}
