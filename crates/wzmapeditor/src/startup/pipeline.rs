//! Startup pipeline state machine.
//!
//! Defines [`StartupPhase`] which gates the editor UI until all critical
//! background loads complete, and [`create_startup`] which spawns all initial
//! background threads and constructs the phase.

#[cfg(not(target_arch = "wasm32"))]
use crate::app::EditorApp;
use crate::config::{EditorConfig, Tileset};
use crate::startup::task::TaskHandle;
use crate::startup::{LoadMapMeta, TilesetPayload};

/// Spawn a background thread that loads tileset images and builds the atlas.
pub(crate) fn spawn_tileset_load(dir: std::path::PathBuf) -> TaskHandle<Option<TilesetPayload>> {
    TaskHandle::spawn("tileset_load", move || {
        let tile_images = crate::ui::tileset_browser::load_tile_images_from_dir(&dir);
        let atlas = crate::viewport::atlas::TileAtlas::build(&dir);
        if tile_images.is_empty() {
            None
        } else {
            log::info!(
                "Pre-loaded {} tileset tile images from {}",
                tile_images.len(),
                dir.display()
            );
            Some(TilesetPayload {
                tile_images,
                source_dir: dir,
                atlas,
            })
        }
    })
}

/// Spawn a tileset load resolved from `config` for the given `tileset`.
///
/// Returns `None` if the tileset dir is unconfigured or missing on disk.
pub(crate) fn spawn_tileset_load_for(
    config: &EditorConfig,
    tileset: Tileset,
) -> Option<TaskHandle<Option<TilesetPayload>>> {
    let dir = config.tileset_dir_for(tileset)?;
    if !dir.exists() {
        return None;
    }
    Some(spawn_tileset_load(dir))
}

/// Default map to open on a fresh first-run, after the user picks a data
/// directory and there's no CLI argument or remembered last map. Loaded
/// from `base.wz` directly when present, otherwise from the extracted tree.
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_FIRST_RUN_MAP_PREFIX: &str = "multiplay/maps/3c-Gamma";

/// Spawn a thread that loads the default first-run map (`3c-Gamma`).
///
/// 3c-Gamma is multiplayer content, so it lives in `mp.wz`, not `base.wz`.
/// Tries `mp.wz` first (parallelisable with the base.wz extraction worker),
/// then falls back to a pre-extracted directory layout. Returns `None` when
/// neither source is available; callers treat that as "no default map".
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn_default_first_run_map(
    config: &EditorConfig,
) -> Option<TaskHandle<Option<(wz_maplib::WzMap, LoadMapMeta)>>> {
    let from_wz = config
        .game_install_dir
        .as_ref()
        .map(|p| p.join("mp.wz"))
        .filter(|p| p.exists());
    let from_dir = config
        .data_dir
        .as_ref()
        .map(|p| p.join(DEFAULT_FIRST_RUN_MAP_PREFIX))
        .filter(|p| p.exists());

    if from_wz.is_none() && from_dir.is_none() {
        return None;
    }

    Some(TaskHandle::spawn("first_run_map_load", move || {
        if let Some(wz_path) = from_wz {
            let prefix = format!("{DEFAULT_FIRST_RUN_MAP_PREFIX}/");
            match wz_maplib::io_wz::load_map_from_archive_prefix(&wz_path, &prefix) {
                Ok(map) => {
                    log::info!(
                        "Loaded default first-run map 3c-Gamma from {}",
                        wz_path.display()
                    );
                    Some((
                        map,
                        LoadMapMeta {
                            source_path: Some(wz_path),
                            save_path: None,
                            archive_prefix: Some(prefix),
                        },
                    ))
                }
                Err(e) => {
                    log::warn!("Failed to load default 3c-Gamma from mp.wz: {e}");
                    None
                }
            }
        } else if let Some(dir) = from_dir {
            match wz_maplib::io_wz::load_from_directory(&dir) {
                Ok(map) => {
                    log::info!(
                        "Loaded default first-run map 3c-Gamma from {}",
                        dir.display()
                    );
                    Some((
                        map,
                        LoadMapMeta {
                            source_path: Some(dir),
                            save_path: None,
                            archive_prefix: None,
                        },
                    ))
                }
                Err(e) => {
                    log::warn!("Failed to load default 3c-Gamma from extracted tree: {e}");
                    None
                }
            }
        } else {
            None
        }
    }))
}

/// Spawn a background thread that loads stats from `data_dir/base/stats`,
/// merging `data_dir/mp/stats` (Dragon, Wyvern, etc.) when present.
///
/// Returns `None` if `base/stats` does not exist under `data_dir`.
pub(crate) fn spawn_stats_load(
    data_dir: &std::path::Path,
) -> Option<TaskHandle<Option<wz_stats::StatsDatabase>>> {
    let stats_dir = data_dir.join("base/stats");
    if !stats_dir.exists() {
        return None;
    }
    let data_dir_for_mp = data_dir.to_path_buf();
    Some(TaskHandle::spawn(
        "stats_load",
        move || match wz_stats::StatsDatabase::load_from_dir(&stats_dir) {
            Ok(mut db) => {
                let base_template_count = db.templates.len();
                let mp_stats_dir = data_dir_for_mp.join("mp").join("stats");
                if mp_stats_dir.exists() {
                    if let Err(e) = db.merge_from_dir(&mp_stats_dir) {
                        log::warn!("Failed to merge mp stats: {e}");
                    }
                } else {
                    log::warn!(
                        "mp/stats/ not found at {} - skirmish-only templates \
                         (Cobra/Mantis/Python/Tiger/Vengeance variants, \
                         204 entries) will be missing and the asset browser \
                         can't tell campaign-only templates apart",
                        mp_stats_dir.display()
                    );
                }
                log::info!(
                    "Background stats load: {} structures, {} features, \
                     {} bodies, {} templates ({} base + {} mp-only)",
                    db.structures.len(),
                    db.features.len(),
                    db.bodies.len(),
                    db.templates.len(),
                    base_template_count,
                    db.templates.len().saturating_sub(base_template_count),
                );
                Some(db)
            }
            Err(e) => {
                log::warn!("Failed to load stats: {e}");
                None
            }
        },
    ))
}

/// Gates the editor UI until critical data finishes loading.
pub enum StartupPhase {
    /// All critical background loads complete. Normal editor operation.
    Ready,
    /// First-run launcher. No data directory yet, or the configured one is
    /// missing. Editor stays hidden; the launcher shows a Browse button.
    Setup {
        /// Last error from a Browse attempt (e.g. "no base.wz found").
        error: Option<String>,
        /// Map load task kept alive across Setup so a CLI arg or
        /// `last_opened_map` can be drained after the user picks a directory.
        #[cfg_attr(
            target_arch = "wasm32",
            expect(
                dead_code,
                reason = "drained only by the native data-directory transition; the web build has no data-dir picker yet"
            )
        )]
        map_rx: Option<TaskHandle<Option<(wz_maplib::WzMap, LoadMapMeta)>>>,
    },
    /// Background loads in progress. Show splash screen, skip editor UI.
    Loading {
        /// Delivers the loaded `WzMap` (or `None` on failure).
        map_rx: Option<TaskHandle<Option<(wz_maplib::WzMap, LoadMapMeta)>>>,
        /// Delivers pre-decoded tileset images and atlas built off-thread.
        tileset_rx: Option<TaskHandle<Option<TilesetPayload>>>,
        /// Delivers `StatsDatabase` loaded off-thread.
        stats_rx: Option<TaskHandle<Option<wz_stats::StatsDatabase>>>,
        /// Whether the map load is complete.
        map_done: bool,
        /// Whether the tileset load is complete.
        tileset_done: bool,
        /// Whether the stats load is complete.
        stats_done: bool,
        /// Whether base.wz extraction is in-flight.
        extracting: bool,
        /// Whether extraction was started this session (for showing a check mark after completion).
        extraction_started: bool,
        /// Whether post-load tasks have been kicked off.
        post_load_started: bool,
    },
}

/// Result of [`create_startup`]: startup phase plus any extraction state.
pub struct StartupInit {
    /// The startup phase (Loading or Ready).
    pub phase: StartupPhase,
    /// Extraction progress arc (present if extraction was started).
    pub extraction_progress: Option<std::sync::Arc<std::sync::atomic::AtomicU32>>,
    /// Extraction task (present if extraction was started).
    pub extraction_rx: Option<TaskHandle<Result<std::path::PathBuf, String>>>,
    /// CLI path (if provided), used as initial `save_path`.
    pub cli_path: Option<std::path::PathBuf>,
}

/// Spawn all initial background threads and construct the startup phase.
///
/// Spawns up to 4 background threads:
/// - Map load (if CLI arg or `last_opened_map`)
/// - Base.wz extraction (if cache is missing/stale)
/// - Tileset load (if `data_dir` exists and no extraction needed)
/// - Stats load (if `data_dir` exists and no extraction needed)
pub fn create_startup(config: &EditorConfig) -> StartupInit {
    // CLI argument takes priority: `wzmapeditor path/to/map.wz`
    let cli_path = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists());

    let map_rx = {
        let cli = cli_path.clone();
        let last_map = config.last_opened_map.clone();
        let last_prefix = config.last_opened_map_prefix.clone();
        let needs_load = cli.is_some() || last_map.is_some();
        if needs_load {
            Some(TaskHandle::spawn("startup_map_load", move || {
                if let Some(ref path) = cli {
                    log::info!("Opening from CLI argument: {}", path.display());
                    wz_maplib::io_wz::load_from_wz_archive(path)
                        .ok()
                        .map(|map| {
                            (
                                map,
                                LoadMapMeta {
                                    source_path: Some(path.clone()),
                                    save_path: Some(path.clone()),
                                    archive_prefix: None,
                                },
                            )
                        })
                } else if let Some(ref path) = last_map {
                    let prefix = last_prefix.as_deref().unwrap_or("");
                    let load_result = if path.extension().is_some_and(|ext| ext == "wz") {
                        if prefix.is_empty() {
                            wz_maplib::io_wz::load_from_wz_archive(path)
                        } else {
                            wz_maplib::io_wz::load_map_from_archive_prefix(path, prefix)
                        }
                    } else {
                        wz_maplib::io_wz::load_from_directory(path)
                    };
                    match load_result {
                        Ok(map) => {
                            log::info!("Auto-loaded last map: {}", map.map_name);
                            // Sub-map of a multi-map archive: leave save_path
                            // unset so Save falls back to Save As; overwriting
                            // the outer .wz would clobber sibling maps.
                            let save_path = if prefix.is_empty() {
                                Some(path.clone())
                            } else {
                                None
                            };
                            Some((
                                map,
                                LoadMapMeta {
                                    source_path: Some(path.clone()),
                                    save_path,
                                    archive_prefix: last_prefix,
                                },
                            ))
                        }
                        Err(e) => {
                            log::warn!(
                                "Failed to auto-load last map from {}: {}",
                                path.display(),
                                e
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            }))
        } else {
            None
        }
    };

    // First-run launcher: editor stays hidden until the user picks a valid
    // data directory. The map_rx (CLI arg or last_opened_map) is parked on
    // the Setup phase and consumed once Loading begins.
    let needs_setup =
        !config.setup_complete || config.data_dir.as_deref().is_none_or(|p| !p.exists());
    if needs_setup {
        log::info!("Launcher: entering Setup phase (first run or missing data_dir)");
        return StartupInit {
            phase: StartupPhase::Setup {
                error: None,
                map_rx,
            },
            extraction_progress: None,
            extraction_rx: None,
            cli_path,
        };
    }

    let needs_extraction = if let Some(ref data_dir) = config.data_dir {
        let cache_dir = crate::config::extraction_cache_dir();
        let overlay_marker = cache_dir.join(".overlays_v9");
        let cache_valid = (cache_dir.join("base").join("texpages").exists()
            || cache_dir.join("base").join("stats").exists())
            && overlay_marker.exists();
        if cache_valid {
            false
        } else {
            let files_missing = !data_dir.join("base").join("texpages").exists()
                && !data_dir.join("base").join("stats").exists();
            let marker_stale = !data_dir.join(".overlays_v9").exists()
                && (data_dir.join("base").join("texpages").exists()
                    || data_dir.join("base").join("stats").exists());
            files_missing || marker_stale
        }
    } else {
        false
    };

    let mut extraction_progress_arc = None;
    let mut extraction_rx_channel = None;
    let skip_asset_threads = if needs_extraction {
        if let Some(ref install_dir) = config.game_install_dir {
            let base_wz = install_dir.join("base.wz");
            if base_wz.exists() {
                log::info!("Starting base.wz extraction during startup");
                let output_dir = crate::config::extraction_cache_dir();

                if let Some(ref data_dir) = config.data_dir {
                    let marker_stale = !data_dir.join(".overlays_v9").exists()
                        && (data_dir.join("base").join("texpages").exists()
                            || data_dir.join("base").join("stats").exists());
                    if marker_stale {
                        log::info!("Overlay marker stale, re-extracting base.wz...");
                        let _ = std::fs::remove_dir_all(data_dir);
                        let ground_cache = crate::config::ground_cache_dir();
                        let _ = std::fs::remove_dir_all(&ground_cache);
                    }
                }
                if output_dir.join("texpages").exists() || output_dir.join("stats").exists() {
                    let _ = std::fs::remove_dir_all(&output_dir);
                }

                let overrides_dir = base_wz.parent().map(|p| p.join("terrain_overrides"));
                let classic_wz = overrides_dir.as_ref().map(|p| p.join("classic.wz"));
                let high_wz = overrides_dir.as_ref().map(|p| p.join("high.wz"));

                let progress = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

                let task = TaskHandle::spawn_with_progress(
                    "base_wz_extract",
                    progress.clone(),
                    move |p| {
                        let base_subdir = output_dir.join("base");
                        let result =
                            wz_maplib::io_wz::extract_wz_to_dir(&base_wz, &base_subdir, |frac| {
                                p.store(
                                    (frac * 1000.0) as u32,
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                            });

                        if let Err(ref e) = result {
                            return Err(e.to_string());
                        }

                        if let Some(ref cwz) = classic_wz
                            && cwz.exists()
                        {
                            log::info!("Overlaying classic.wz from {}", cwz.display());
                            if let Err(e) = wz_maplib::io_wz::extract_wz_to_dir_overwrite(
                                cwz,
                                &base_subdir,
                                |_| {},
                            ) {
                                log::warn!("Failed to extract classic.wz: {e}");
                            }
                        }

                        if let Some(ref hwz) = high_wz
                            && hwz.exists()
                        {
                            log::info!("Extracting high.wz from {}", hwz.display());
                            if let Err(e) = wz_maplib::io_wz::extract_wz_to_dir_overwrite(
                                hwz,
                                &base_subdir,
                                |_| {},
                            ) {
                                log::warn!("Failed to extract high.wz: {e}");
                            }
                        }

                        // mp.wz holds multiplayer stats (Dragon, Wyvern, etc.).
                        let mp_wz = base_wz.with_file_name("mp.wz");
                        if mp_wz.exists() {
                            let mp_subdir = output_dir.join("mp");
                            if let Err(e) =
                                wz_maplib::io_wz::extract_wz_to_dir(&mp_wz, &mp_subdir, |_| {})
                            {
                                log::warn!("Failed to extract mp.wz: {e}");
                            }
                        }

                        let overlay_marker = output_dir.join(".overlays_v9");
                        let _ = std::fs::File::create(&overlay_marker);

                        result.map(|()| output_dir).map_err(|e| e.to_string())
                    },
                );

                extraction_progress_arc = Some(progress);
                extraction_rx_channel = Some(task);
                true // skip tileset/stats threads, dirs don't exist yet
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    // Use remembered tileset from last session, falling back to Arizona.
    let startup_tileset = config
        .last_tileset
        .as_deref()
        .and_then(Tileset::from_name)
        .unwrap_or(Tileset::Arizona);
    let tileset_rx = if !skip_asset_threads && config.data_dir.is_some() {
        spawn_tileset_load_for(config, startup_tileset)
    } else {
        None
    };

    let stats_rx = if skip_asset_threads {
        None
    } else {
        config.data_dir.as_deref().and_then(spawn_stats_load)
    };

    let has_background_work =
        map_rx.is_some() || tileset_rx.is_some() || stats_rx.is_some() || skip_asset_threads;
    log::info!(
        "Startup phase: map_rx={}, tileset_rx={}, stats_rx={}, extracting={}, background_work={}",
        map_rx.is_some(),
        tileset_rx.is_some(),
        stats_rx.is_some(),
        skip_asset_threads,
        has_background_work
    );
    if skip_asset_threads {
        log::info!("[splash] Extracting game data: started");
    }
    if map_rx.is_some() {
        log::info!("[splash] Loading map: started");
    }
    if tileset_rx.is_some() {
        log::info!("[splash] Loading tileset: started");
    }
    if stats_rx.is_some() {
        log::info!("[splash] Loading stats: started");
    }

    let phase = if has_background_work {
        StartupPhase::Loading {
            map_done: map_rx.is_none(),
            tileset_done: tileset_rx.is_none() && !skip_asset_threads,
            stats_done: stats_rx.is_none() && !skip_asset_threads,
            map_rx,
            tileset_rx,
            stats_rx,
            extracting: skip_asset_threads,
            extraction_started: skip_asset_threads,
            post_load_started: false,
        }
    } else {
        StartupPhase::Ready
    };

    StartupInit {
        phase,
        extraction_progress: extraction_progress_arc,
        extraction_rx: extraction_rx_channel,
        cli_path,
    }
}

/// Move the launcher from `Setup` into `Loading` after the user picks a valid
/// data directory. Marks setup complete in the persisted config and (when
/// extraction is not pending) spawns tileset and stats loads. The parked
/// `map_rx` from `Setup` is forwarded so a CLI arg or `last_opened_map`
/// resolves once Loading drains.
///
/// `extraction_in_flight` should be `true` if the caller has already invoked
/// `start_base_wz_extraction`; in that case tileset and stats are spawned
/// later by `workers::poll_startup_loads` once extraction completes.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn transition_setup_to_loading(app: &mut EditorApp, extraction_in_flight: bool) {
    let map_rx = match std::mem::replace(&mut app.startup_phase, StartupPhase::Ready) {
        StartupPhase::Setup { map_rx, .. } => map_rx,
        other => {
            // Defensive: only meaningful when called from Setup.
            app.startup_phase = other;
            return;
        }
    };

    // Default to opening 3c-Gamma when this is a true first run with no
    // CLI argument or remembered last map.
    let map_rx = map_rx.or_else(|| {
        if app.config.last_opened_map.is_some() {
            return None;
        }
        let rx = spawn_default_first_run_map(&app.config)?;
        log::info!("[splash] Loading map: started (default 3c-Gamma)");
        Some(rx)
    });

    app.config.setup_complete = true;
    app.config.save();

    let (tileset_rx, stats_rx) = if extraction_in_flight {
        (None, None)
    } else {
        let last_tileset = app
            .config
            .last_tileset
            .as_deref()
            .and_then(Tileset::from_name)
            .unwrap_or(Tileset::Arizona);
        let trx = spawn_tileset_load_for(&app.config, last_tileset);
        let srx = app.config.data_dir.as_deref().and_then(spawn_stats_load);
        (trx, srx)
    };

    if extraction_in_flight {
        log::info!("[splash] Extracting game data: started");
    }
    if tileset_rx.is_some() {
        log::info!("[splash] Loading tileset: started");
    }
    if stats_rx.is_some() {
        log::info!("[splash] Loading stats: started");
    }
    log::info!("Setup complete, entering Loading phase");

    app.startup_phase = StartupPhase::Loading {
        map_done: map_rx.is_none(),
        tileset_done: tileset_rx.is_none() && !extraction_in_flight,
        stats_done: stats_rx.is_none() && !extraction_in_flight,
        map_rx,
        tileset_rx,
        stats_rx,
        extracting: extraction_in_flight,
        extraction_started: extraction_in_flight,
        post_load_started: false,
    };
}
