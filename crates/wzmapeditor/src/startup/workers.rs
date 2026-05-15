//! Background thread spawning and startup polling.
//!
//! Contains `poll_startup_loads` (the frame-by-frame startup orchestrator)
//! and `precache_ground_textures` (the heavy KTX2 decode worker).

use crate::app::{EditorApp, MapModelLoadState, StartupPhase};
use crate::startup::pipeline::{spawn_stats_load, spawn_tileset_load};
use crate::ui::tileset_browser::TilesetData;
use crate::viewport::model_loader::ModelLoader;

/// Poll startup background load receivers and apply results when ready.
///
/// Transitions `startup_phase` from `Loading` to `Ready` once all receivers
/// have been consumed. Takes the phase out temporarily to avoid borrow
/// conflicts with `app.load_map()` and other `&mut self` methods.
pub fn poll_startup_loads(ctx: &egui::Context, app: &mut EditorApp) {
    // Take the phase out so we can mutate `app` freely while polling receivers.
    let mut phase = std::mem::replace(&mut app.startup_phase, StartupPhase::Ready);

    let StartupPhase::Loading {
        ref mut map_rx,
        ref mut tileset_rx,
        ref mut stats_rx,
        ref mut map_done,
        ref mut tileset_done,
        ref mut stats_done,
        ref mut extracting,
        extraction_started: _,
        ref mut post_load_started,
    } = phase
    else {
        app.startup_phase = phase;
        return;
    };

    if *extracting {
        let finished = app
            .rt
            .extraction_rx
            .as_ref()
            .and_then(super::task::TaskHandle::try_take);
        if let Some(result) = finished {
            app.rt.extraction_rx = None;
            app.rt.extraction_progress = None;
            *extracting = false;
            log::info!("[splash] Extracting game data: done");
            match result {
                Ok(data_dir) => {
                    app.log(format!(
                        "Extraction complete, data dir: {}",
                        data_dir.display()
                    ));
                    // Like set_data_dir but skip clearing tileset/stats
                    // flags, those haven't been loaded yet.
                    app.config.data_dir = Some(data_dir.clone());
                    app.config.save();
                    app.has_hq_textures = crate::app::data_loading::detect_hq_textures(&data_dir);

                    // Spawn tileset + stats threads now that dirs exist.
                    match crate::startup::pipeline::spawn_tileset_load_for(
                        &app.config,
                        app.current_tileset,
                    ) {
                        Some(rx) => {
                            *tileset_rx = Some(rx);
                            log::info!("[splash] Loading tileset: started");
                        }
                        None => *tileset_done = true,
                    }

                    match spawn_stats_load(&data_dir) {
                        Some(rx) => {
                            *stats_rx = Some(rx);
                            log::info!("[splash] Loading stats: started");
                        }
                        None => *stats_done = true,
                    }
                }
                Err(msg) => {
                    app.log(format!("Extraction failed: {msg}"));
                    *tileset_done = true;
                    *stats_done = true;
                }
            }
        }
    }

    if let Some(task) = map_rx.as_ref()
        && let Some(result) = task.try_take()
    {
        if let Some((map, meta)) = result {
            app.load_map(map, meta.source_path, meta.save_path, meta.archive_prefix);
        }
        *map_rx = None;
        *map_done = true;
        log::info!("[splash] Loading map: done");
    }

    if let Some(task) = tileset_rx.as_ref()
        && let Some(result) = task.try_take()
    {
        if let Some(payload) = result {
            // The startup tileset thread defaults to Arizona, so a later
            // map load may have switched current_tileset; discard and
            // re-spawn if so.
            let expected_subpath = app.current_tileset.subpath();
            let source_matches = payload
                .source_dir
                .to_string_lossy()
                .replace('\\', "/")
                .contains(expected_subpath);

            if !source_matches {
                log::info!(
                    "Discarding pre-loaded tileset (loaded {}, need {:?}), re-spawning",
                    payload.source_dir.display(),
                    app.current_tileset
                );
                let correct_dir = app.config.tileset_dir_for(app.current_tileset);
                if let Some(dir) = correct_dir.filter(|d| d.exists()) {
                    *tileset_rx = Some(spawn_tileset_load(dir));
                    *tileset_done = false;
                    log::info!(
                        "[splash] Loading tileset: re-spawned for {:?}",
                        app.current_tileset
                    );
                } else {
                    *tileset_rx = None;
                    *tileset_done = true;
                    app.rt.tileset_load_attempted = false;
                }
                app.startup_phase = phase;
                return;
            }

            if let Some(ts) = TilesetData::from_preloaded(ctx, payload.tile_images) {
                app.log(format!(
                    "Loaded {} {:?} tileset tiles",
                    ts.tile_count(),
                    app.current_tileset
                ));
                app.tileset = Some(ts);
            }
            let mut atlas_msg = None;
            if let Some(atlas) = payload.atlas
                && let Some(ref render_state) = app.wgpu_render_state
            {
                let device = &render_state.device;
                let queue = &render_state.queue;
                let mut renderer = render_state.renderer.write();
                if let Some(resources) = renderer
                    .callback_resources
                    .get_mut::<crate::viewport::ViewportResources>()
                {
                    resources.renderer.upload_atlas(
                        device,
                        queue,
                        &atlas.data,
                        atlas.width,
                        atlas.height,
                    );
                    atlas_msg = Some(format!(
                        "Uploaded tileset atlas: {} tiles, {}x{}",
                        atlas.tile_count, atlas.width, atlas.height
                    ));
                }
            }
            if let Some(msg) = atlas_msg {
                app.log(msg);
            }
            app.rt.tileset_load_attempted = true;
        }
        *tileset_rx = None;
        *tileset_done = true;
        log::info!("[splash] Loading tileset: done");
    }

    if let Some(task) = stats_rx.as_ref()
        && let Some(result) = task.try_take()
    {
        if let Some(db) = result {
            app.log(format!(
                "Loaded stats: {} structures, {} features",
                db.structures.len(),
                db.features.len()
            ));
            if let Some(ref data_dir) = app.config.data_dir {
                app.model_loader = Some(ModelLoader::new(data_dir, &db));
            }
            app.stats = Some(db);
            app.objects_dirty = true;
            // The map task may have finished first and cached a validation
            // result with stats=None; rerun now so real footprints are used.
            if app.validation_results.is_some() {
                app.run_validation();
            }
            ctx.request_repaint();
        }
        app.rt.stats_load_attempted = true;
        *stats_rx = None;
        *stats_done = true;
        log::info!("[splash] Loading stats: done");
    }

    // Once primary loads finish, kick off post-load background tasks
    // before transitioning to Ready.
    let primary_done = *map_done && *tileset_done && *stats_done && !*extracting;

    if primary_done && !*post_load_started {
        *post_load_started = true;
        log::info!("Primary startup loads complete, starting post-load tasks");

        // Ground texture precache covers all tilesets, needs data_dir only.
        if app.config.data_dir.is_some() && !app.rt.ground_precache_attempted {
            app.rt.ground_precache_attempted = true;
            log::info!("[splash] Caching ground textures: started");
            app.start_ground_precache();
        }

        // Ground texture loading for current tileset.
        if app.tileset.is_some()
            && app.ground_data.is_none()
            && app.rt.ground_texture_load.is_none()
            && app.rt.ground_precache_rx.is_none()
        {
            log::info!("[splash] Caching ground textures: loading current tileset GPU data");
            app.start_ground_data_load();
        }

        // Warm the model loader's connector cache off-thread so thumbnail
        // work-list building is instant.
        if app.rt.connector_precache_rx.is_none()
            && let (Some(stats), Some(loader)) = (app.stats.as_ref(), app.model_loader.as_ref())
        {
            let rx = loader.precache_connectors_background(stats);
            app.rt.connector_precache_rx = Some(rx);
            log::info!("[splash] Caching model connectors: started");
        }
    }

    if primary_done && *post_load_started {
        crate::startup::loading_ui::show_ground_precache_progress(ctx, app);

        // Once precache finishes, start ground texture loading.
        if app.tileset.is_some()
            && app.ground_data.is_none()
            && app.rt.ground_texture_load.is_none()
            && app.rt.ground_precache_rx.is_none()
        {
            app.start_ground_data_load();
        }

        crate::startup::loading_ui::poll_ground_texture_load(ctx, app);

        app.ensure_tile_pools();

        let connectors_ready = app.rt.connector_precache_rx.is_none();
        if !connectors_ready
            && let Some(ref rx) = app.rt.connector_precache_rx
            && let Ok(cache) = rx.try_recv()
        {
            if let Some(loader) = app.model_loader.as_mut() {
                loader.merge_connector_precache(cache);
            }
            app.rt.connector_precache_rx = None;
            log::info!("[splash] Caching model connectors: done");
            ctx.request_repaint();
        }

        // Start object model loading once connectors are ready.
        if app.rt.connector_precache_rx.is_none()
            && app.rt.map_model_load.is_none()
            && app.document.is_some()
            && app.model_loader.is_some()
            && app.objects_dirty
        {
            // Sync tileset on loader before collecting unloaded models.
            if let Some(loader) = app.model_loader.as_mut() {
                loader.set_tileset(app.current_tileset.texture_index());
            }
            let unloaded = crate::ui::viewport_panel::collect_unloaded_models(app);
            // Leave objects_dirty set when unloaded is empty: the viewport
            // hasn't called prepare_object_rendering() yet (still on splash),
            // so the first viewport frame still needs to build draw calls.
            'spawn: {
                if unloaded.is_empty() {
                    break 'spawn;
                }
                let Some(loader) = app.model_loader.as_ref() else {
                    log::warn!("[splash] model_loader missing, skipping object model load");
                    break 'spawn;
                };
                let total = unloaded.len();
                let receiver = loader.prepare_models_background(unloaded);
                log::info!("[splash] Caching object models: started ({total} to upload)");
                app.rt.map_model_load = Some(MapModelLoadState {
                    receiver,
                    total,
                    uploaded: 0,
                });
            }
        }

        // Only start thumbnail preload after connector precache finishes.
        if app.rt.connector_precache_rx.is_none() {
            crate::startup::loading_ui::show_thumbnail_preload(ctx, app, true);

            // Restart preload when it cycles to the next tileset (Idle).
            if let Some(stats) = app.stats.as_ref()
                && matches!(
                    app.model_thumbnails.preload,
                    crate::thumbnails::PreloadState::Idle
                )
            {
                app.model_thumbnails
                    .start_preload(ctx, stats, &mut app.model_loader);
            }
        }

        crate::startup::loading_ui::show_map_model_loading(ctx, app);

        // Ground is done when precache was attempted and finished (rx
        // consumed), or when there's nothing to do (no data_dir).
        let ground_done = (app.rt.ground_precache_attempted
            && app.rt.ground_precache_rx.is_none()
            && app.rt.ground_texture_load.is_none())
            || app.config.data_dir.is_none();
        let thumbnails_done = matches!(
            app.model_thumbnails.preload,
            crate::thumbnails::PreloadState::Done
        ) || app.stats.is_none();
        let models_done = app.rt.map_model_load.is_none();
        let connectors_done = app.rt.connector_precache_rx.is_none();

        if ground_done && thumbnails_done && models_done && connectors_done {
            app.startup_phase = StartupPhase::Ready;
            log::info!("Startup loading complete (all post-load tasks done)");
        } else {
            app.startup_phase = phase;
        }
    } else {
        app.startup_phase = phase;
    }
}

/// Pre-decode and cache KTX2 ground textures for all three tilesets.
pub(crate) fn precache_ground_textures(
    data_dir: &std::path::Path,
    cache_dir: &std::path::Path,
    progress: &std::sync::atomic::AtomicU32,
) -> Result<
    (
        u32,
        std::collections::HashMap<String, crate::viewport::ground_types::GroundData>,
    ),
    String,
> {
    use crate::viewport::ground_types::GroundData;

    let tilesets = ["arizona", "urban", "rockies"];
    let texpages_dir = data_dir.join("base").join("texpages");

    let mut seen = std::collections::HashSet::new();
    let mut filenames: Vec<String> = Vec::new();
    let mut loaded_ground_data = std::collections::HashMap::new();
    for tileset in &tilesets {
        if let Some(gd) = GroundData::load(data_dir, tileset) {
            for gt in &gd.ground_types {
                if seen.insert(gt.filename.clone()) {
                    filenames.push(gt.filename.clone());
                }
                if let Some(ref nm) = gt.normal_filename
                    && seen.insert(nm.clone())
                {
                    filenames.push(nm.clone());
                }
                if let Some(ref sm) = gt.specular_filename
                    && seen.insert(sm.clone())
                {
                    filenames.push(sm.clone());
                }
            }
            loaded_ground_data.insert((*tileset).to_string(), gd);
        }
    }

    if filenames.is_empty() {
        return Err("No ground type data found for any tileset".to_string());
    }

    let _ = std::fs::create_dir_all(cache_dir);
    let total = filenames.len() as u32;
    let done_counter = std::sync::atomic::AtomicU32::new(0);

    let cached = std::thread::scope(|s| {
        let handles: Vec<_> = filenames
            .iter()
            .map(|filename| {
                let done = &done_counter;
                let prog = progress;
                let tp = &texpages_dir;
                let cd = cache_dir;
                s.spawn(move || {
                    let cache_name = filename.replace(".png", ".bin");
                    let cached_path = cd.join(&cache_name);

                    let result = if cached_path.exists() {
                        true
                    } else {
                        let ktx2_name = filename.replace(".png", ".ktx2");
                        let ktx2_path = tp.join(&ktx2_name);
                        if ktx2_path.exists() {
                            match crate::viewport::renderer::load_ktx2_as_rgba(&ktx2_path) {
                                Ok(mut rgba) => {
                                    let is_diffuse =
                                        !filename.contains("_nm") && !filename.contains("_sm");
                                    if is_diffuse {
                                        crate::viewport::renderer::linear_to_srgb(&mut rgba);
                                    }
                                    let resized = image::imageops::resize(
                                        &rgba,
                                        1024,
                                        1024,
                                        image::imageops::FilterType::CatmullRom,
                                    );
                                    let raw_data = resized.into_raw();
                                    if let Err(e) = std::fs::write(&cached_path, &raw_data) {
                                        log::warn!(
                                            "Failed to cache ground texture {cache_name}: {e}"
                                        );
                                        false
                                    } else {
                                        log::info!("Pre-cached ground texture: {cache_name}");
                                        true
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Failed to decode KTX2 {ktx2_name}: {e}");
                                    false
                                }
                            }
                        } else {
                            false
                        }
                    };

                    let completed = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    prog.store(
                        (completed * 1000) / total,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    result
                })
            })
            .collect();

        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter(|&ok| ok)
            .count() as u32
    });

    progress.store(1000, std::sync::atomic::Ordering::Relaxed);
    Ok((cached, loaded_ground_data))
}
