//! Shared action handlers invoked from both the menu bar and the toolbar.

use crate::app::EditorApp;

pub(crate) fn open_wz_dialog(app: &mut EditorApp) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Open .wz Map")
        .add_filter("WZ Map", &["wz"])
        .pick_file()
    {
        match wz_maplib::io_wz::load_from_wz_archive(&path) {
            Ok(map) => {
                let save = Some(path.clone());
                app.load_map(map, Some(path), save, None);
            }
            Err(e) => app.report_wz_load_error(&path, &e),
        }
    }
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
