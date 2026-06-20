//! Test game launching - skirmish config, process management, temp file cleanup.

use super::EditorApp;

/// Fixed map name for temp test archives and skirmish configs.
/// Using a constant avoids accumulating stale temp maps in the user's
/// WZ2100 maps directory; each test run overwrites the previous one.
const TEST_MAP_NAME: &str = "__wzmapeditor_test";

/// Default AI script used for opponent slots in test skirmish games.
/// `SemperFi` is bundled with all WZ2100 installs and provides a basic opponent.
pub(super) const TEST_AI_SCRIPT: &str = "multiplay/skirmish/semperfi.js";

/// WZ2100 requires at least 2 players for a skirmish game.
const MIN_SKIRMISH_PLAYERS: u8 = 2;

/// Whether the currently loaded map is a campaign map.
///
/// Campaign maps are identified by an archive prefix starting with
/// `"wrf/"` (e.g. `"wrf/cam1/cam1a/"`). These maps cannot be tested
/// via `--skirmish` because they depend on campaign datasets and
/// scripted mission flow.
pub(super) fn is_campaign_map(app: &EditorApp) -> bool {
    app.config
        .last_opened_map_prefix
        .as_ref()
        .is_some_and(|p| p.starts_with("wrf/"))
}

/// Whether the test-map button should be enabled.
///
/// A missing executable is intentionally not a disabling condition: the
/// click handler routes the user to Settings → Game so they can set one.
pub(super) fn can_test_map(app: &EditorApp) -> bool {
    // The web build cannot spawn the game process, so test-launching is never
    // available there.
    !cfg!(target_arch = "wasm32")
        && app.document.is_some()
        && app.test_process.is_none()
        && !is_campaign_map(app)
}

/// Tooltip explaining the test map button's current state.
pub(super) fn test_map_tooltip(app: &EditorApp) -> &'static str {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        "Test-launching is not available in the web build"
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if app.test_process.is_some() {
            "Test game is already running"
        } else if app.document.is_none() {
            "Load a map first"
        } else if is_campaign_map(app) {
            "Campaign maps cannot be test-launched (skirmish only)"
        } else if resolve_wz_executable(app).is_none() {
            "Choose a Warzone 2100 executable in Settings \u{2192} Game (F5)"
        } else {
            "Launch map in WZ2100 (F5)"
        }
    }
}

/// Resolve the executable used to launch test games.
///
/// Prefers the explicit `wz_executable` config setting; falls back to
/// auto-detection from `game_install_dir`.
pub(super) fn resolve_wz_executable(app: &EditorApp) -> Option<std::path::PathBuf> {
    if let Some(ref exe) = app.config.wz_executable
        && exe.exists()
    {
        return Some(exe.clone());
    }
    let install_dir = app.config.game_install_dir.as_ref()?;
    crate::config::wz2100_executable(install_dir)
}

/// Launch the current map in WZ2100 as a skirmish test game.
///
/// Saves the map to a temp `.wz` in WZ2100's user maps directory,
/// writes a skirmish config JSON, and spawns the game process.
/// Does not modify `save_path` or the document's dirty flag.
pub(super) fn test_map(app: &mut EditorApp) {
    let Some(ref doc) = app.document else {
        app.log("No map loaded");
        return;
    };
    if app.test_process.is_some() {
        app.log_warn("Test game is already running");
        return;
    }

    let Some(exe) = resolve_wz_executable(app) else {
        app.log("No Warzone 2100 executable set; opening Settings -> Game");
        app.settings_open = true;
        app.settings_page = crate::ui::settings_window::SettingsPage::Game;
        return;
    };

    let Some(wz_config) = crate::config::resolve_wz_config_dir(&app.config) else {
        app.log("Could not detect WZ2100 user config directory");
        return;
    };

    let maps_dir = wz_config.join("maps");
    let tests_dir = wz_config.join("tests");
    if let Err(e) = std::fs::create_dir_all(&maps_dir) {
        report_test_map_io_error(app, &maps_dir, &e, "create maps dir");
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&tests_dir) {
        report_test_map_io_error(app, &tests_dir, &e, "create tests dir");
        return;
    }

    // Clone the map so the original document stays untouched while we patch
    // in the test-only metadata below.
    let mut test_map = doc.map.clone();
    let test_name = TEST_MAP_NAME;
    test_map.map_name = test_name.to_string();
    test_map.players = app.map_players;
    test_map.tileset = app.current_tileset_name();

    let wz_path = maps_dir.join(format!("{test_name}.wz"));
    log::info!("Saving test map to {}", wz_path.display());
    if let Err(e) =
        wz_maplib::io_wz::save_to_wz_archive(&test_map, &wz_path, wz_maplib::OutputFormat::Ver3)
    {
        if let wz_maplib::MapError::Io { ref source, .. } = e
            && source.kind() == std::io::ErrorKind::PermissionDenied
        {
            open_permission_dialog(app, &wz_path, source);
        }
        app.log(format!("Failed to save test map: {e}"));
        return;
    }

    let config_path = tests_dir.join(format!("{test_name}.json"));
    let config_json = build_skirmish_config(test_name, app.map_players);
    log::info!("Writing skirmish config to {}", config_path.display());
    if let Err(e) = std::fs::write(&config_path, &config_json) {
        app.log(format!("Failed to write test config: {e}"));
        return;
    }
    log::debug!("Skirmish config:\n{config_json}");

    // WZ2100's --skirmish arg is used verbatim as the filename under tests/,
    // so it must include the .json extension.
    let arg = format!("--skirmish={test_name}.json");
    log::info!("Launching: {} {}", exe.display(), arg);
    match std::process::Command::new(&exe).arg(&arg).spawn() {
        Ok(child) => {
            app.log(format!("Launched test game (pid {})", child.id()));
            app.test_process = Some(child);
            app.test_temp_files = vec![wz_path, config_path];
        }
        Err(e) => {
            app.log_error(format!("Failed to launch WZ2100: {e}"));
            let _ = std::fs::remove_file(&wz_path);
            let _ = std::fs::remove_file(&config_path);
        }
    }
}

/// Poll the test game process and clean up temp files when it exits.
pub(super) fn poll_test_process(app: &mut EditorApp) {
    let exited = if let Some(ref mut child) = app.test_process {
        matches!(child.try_wait(), Ok(Some(_)))
    } else {
        false
    };
    if exited {
        app.test_process = None;
        for path in app.test_temp_files.drain(..) {
            let _ = std::fs::remove_file(&path);
        }
        app.log("Test game ended. Temp files cleaned up");
    }
}

/// Build a skirmish test config JSON for the given map.
///
/// Player 0 is human; remaining slots up to `players` are filled with
/// AI opponents on separate teams.
pub(super) fn build_skirmish_config(map_name: &str, players: u8) -> String {
    let players = players.max(MIN_SKIRMISH_PLAYERS);
    let mut config = serde_json::json!({
        "challenge": {
            "map": map_name,
            "maxPlayers": players,
            "scavengers": 0,
            "difficulty": "Medium",
            "powerLevel": 1,
            "bases": 1
        },
        "player_0": { "team": 0 }
    });

    for i in 1..players {
        let key = format!("player_{i}");
        config[key] = serde_json::json!({
            "team": i,
            "ai": TEST_AI_SCRIPT,
            "difficulty": "Medium"
        });
    }

    serde_json::to_string_pretty(&config).expect("skirmish config serialization cannot fail")
}

fn report_test_map_io_error(
    app: &mut EditorApp,
    target: &std::path::Path,
    error: &std::io::Error,
    action: &str,
) {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        open_permission_dialog(app, target, error);
    }
    app.log(format!("Failed to {action}: {error}"));
}

fn open_permission_dialog(app: &mut EditorApp, target: &std::path::Path, error: &std::io::Error) {
    app.permission_error_dialog.open = true;
    app.permission_error_dialog.target_path = target.to_path_buf();
    app.permission_error_dialog.error_message = error.to_string();
}
