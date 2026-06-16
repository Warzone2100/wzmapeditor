//! Background data loading - ground textures, base.wz extraction, game stats.

use super::EditorApp;
#[cfg(not(target_arch = "wasm32"))]
use super::{GroundTextureLoadState, GroundTexturePayload, GroundUploadViews};
#[cfg(not(target_arch = "wasm32"))]
use crate::startup::GroundPrecacheResult;
use crate::viewport::model_loader::ModelLoader;

/// Loads metadata synchronously, then spawns a background thread to read
/// texture RGBA data. The GPU upload happens on the main thread when the
/// background thread completes (polled each frame).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn start_ground_data_load(app: &mut EditorApp) {
    if app.ground_data.is_some() || app.rt.ground_texture_load.is_some() {
        return;
    }
    let assets = match app.assets {
        Some(ref a) => a.clone(),
        None => return,
    };
    let tileset_name = match app.current_tileset {
        crate::config::Tileset::Arizona => "arizona",
        crate::config::Tileset::Urban => "urban",
        crate::config::Tileset::Rockies => "rockies",
    };
    let gd = if let Some(cached) = app.rt.precached_ground_data.remove(tileset_name) {
        log::info!("Reusing pre-cached ground data for {tileset_name}");
        cached
    } else if let Some(gd) =
        crate::viewport::ground_types::GroundData::load(assets.as_ref(), tileset_name)
    {
        gd
    } else {
        log::warn!("Failed to load ground data for tileset {tileset_name:?}");
        return;
    };

    // Decal detection is deferred: the background thread reads original
    // tiles from base.wz (before classic.wz overlay) and returns alpha
    // flags alongside the texture data. The override happens in
    // poll_ground_texture_load after the thread completes.

    let texpages_rel = std::path::PathBuf::from("base/texpages");
    let ground_types = gd.ground_types.clone();
    let num_decal_tiles = gd.tile_grounds.len() as u32;
    let tileset_128_rel = std::path::PathBuf::from(app.current_tileset.subpath());
    let tileset_256_rel = std::path::PathBuf::from(app.current_tileset.subpath_256());
    let base_wz_path = app
        .config
        .game_install_dir
        .as_ref()
        .map(|d| d.join("base.wz"))
        .filter(|p| p.exists());
    let tileset_subpath = format!(
        "texpages/{}",
        app.current_tileset
            .subpath()
            .strip_prefix("base/texpages/")
            .unwrap_or(app.current_tileset.subpath())
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let load_progress = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let load_progress_clone = load_progress.clone();

    let work = move || {
        // 6 steps total - report progress after each (thousandths: 0..1000).
        let diffuse = crate::viewport::renderer::load_ground_texture_data(
            assets.as_ref(),
            &texpages_rel,
            &ground_types,
        );
        load_progress_clone.store(167, std::sync::atomic::Ordering::Relaxed); // 1/6

        // Always load normal/specular maps - the loader generates sensible
        // defaults (flat normals, zero specular) for missing files, so the
        // High quality pipeline works even without dedicated _nm/_sm assets.
        let nm = crate::viewport::renderer::load_ground_normal_specular_data(
            assets.as_ref(),
            &texpages_rel,
            &ground_types,
            "_nm",
        );
        load_progress_clone.store(333, std::sync::atomic::Ordering::Relaxed); // 2/6

        let sm = crate::viewport::renderer::load_ground_normal_specular_data(
            assets.as_ref(),
            &texpages_rel,
            &ground_types,
            "_sm",
        );
        load_progress_clone.store(500, std::sync::atomic::Ordering::Relaxed); // 3/6

        // Load decal tile textures from the original base.wz (preserves alpha
        // that classic.wz overlay removes). Also returns per-tile alpha flags
        // for correct decal detection.
        let base_archive = base_wz_path
            .as_deref()
            .and_then(wz_maplib::io_wz::WzArchiveReader::open);
        let (decal_diffuse, _decal_alpha_flags) =
            crate::viewport::renderer::load_decal_texture_data_from_wz(
                base_archive,
                &tileset_subpath,
                assets.as_ref(),
                &tileset_128_rel,
                &tileset_256_rel,
                num_decal_tiles,
            );
        load_progress_clone.store(667, std::sync::atomic::Ordering::Relaxed); // 4/6

        let dn = crate::viewport::renderer::load_decal_normal_specular_data(
            assets.as_ref(),
            &tileset_256_rel,
            num_decal_tiles,
            "_nm",
        );
        load_progress_clone.store(833, std::sync::atomic::Ordering::Relaxed); // 5/6

        let ds = crate::viewport::renderer::load_decal_normal_specular_data(
            assets.as_ref(),
            &tileset_256_rel,
            num_decal_tiles,
            "_sm",
        );
        load_progress_clone.store(1000, std::sync::atomic::Ordering::Relaxed); // 6/6

        let _ = tx.send(GroundTexturePayload {
            diffuse,
            normals: nm,
            specular: sm,
            decal_diffuse,
            decal_normal: dn,
            decal_specular: ds,
            num_decal_tiles,
        });
    };
    std::thread::spawn(work);

    app.rt.ground_texture_load = Some(GroundTextureLoadState {
        receiver: rx,
        ground_data: gd,
        progress: load_progress,
        payload: None,
        upload_step: None,
        upload_views: GroundUploadViews::default(),
    });
    app.log("Loading ground textures...".to_string());
}

/// Web variant: publish the ground metadata and skip the texture decode.
///
/// The web build is locked to Classic terrain, which textures from the tile
/// atlas rather than the 1024-pixel ground-type arrays the native loader
/// decodes. Decoding those on the single-threaded browser would freeze the tab
/// for textures that are never sampled, so this loads only the metadata the
/// terrain mesh needs and returns.
#[cfg(target_arch = "wasm32")]
pub(super) fn start_ground_data_load(app: &mut EditorApp) {
    if app.ground_data.is_some() {
        return;
    }
    let Some(assets) = app.assets.clone() else {
        return;
    };
    let tileset_name = match app.current_tileset {
        crate::config::Tileset::Arizona => "arizona",
        crate::config::Tileset::Urban => "urban",
        crate::config::Tileset::Rockies => "rockies",
    };
    let Some(gd) = crate::viewport::ground_types::GroundData::load(assets.as_ref(), tileset_name)
    else {
        log::warn!(
            "Failed to load ground data for tileset {:?}",
            app.current_tileset
        );
        return;
    };
    app.ground_data = Some(gd);
    app.terrain_dirty = true;
    app.log("Loaded ground data (Classic terrain quality)".to_string());
}

/// Pre-decode and cache ground textures for all tilesets in the background.
///
/// Spawns a thread that decodes KTX2 ground textures for Arizona, Urban,
/// and Rockies, saving decoded PNGs to the ground-cache directory. Only
/// runs once; skipped if the cache marker already exists.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn start_ground_precache(app: &mut EditorApp) {
    let data_dir = match app.config.data_dir {
        Some(ref d) => d.clone(),
        None => return,
    };

    let cache_dir = crate::config::ground_cache_dir();
    let marker = cache_dir.join(".precache_v9");
    if marker.exists() {
        log::debug!("Ground texture cache already populated, skipping pre-cache");
        return;
    }

    if app.rt.ground_precache_rx.is_some() {
        return;
    }

    let progress = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let (tx, rx) = std::sync::mpsc::channel();
    let progress_clone = progress.clone();

    std::thread::spawn(move || {
        let result = crate::startup::workers::precache_ground_textures(
            &data_dir,
            &cache_dir,
            &progress_clone,
        );
        match result {
            Ok((count, ground_data)) => {
                // Write marker so subsequent launches skip pre-cache.
                let _ = std::fs::File::create(&marker);
                let _ = tx.send(GroundPrecacheResult {
                    message: format!("Pre-cached {count} ground textures for all tilesets"),
                    ground_data,
                });
            }
            Err(e) => {
                let _ = tx.send(GroundPrecacheResult {
                    message: format!("Ground texture pre-cache failed: {e}"),
                    ground_data: std::collections::HashMap::new(),
                });
            }
        }
    });

    app.rt.ground_precache_progress = Some(progress);
    app.rt.ground_precache_rx = Some(rx);
    app.log("Pre-caching ground textures for all tilesets...".to_string());
}

/// Ground-texture pre-caching decodes KTX2 to disk, neither of which the
/// web build supports; textures decode on demand in memory instead.
#[cfg(target_arch = "wasm32")]
pub(super) fn start_ground_precache(_app: &mut EditorApp) {}

/// Set the resolved asset root, save config, and schedule tileset + stats reload.
///
/// The actual loading is deferred to the next frame via the auto-load
/// checks in `update()`. Loading synchronously mid-frame destroys old
/// egui `TextureHandle`s while the renderer still references them, causing
/// a wgpu validation error ("Texture has been destroyed").
pub(super) fn set_data_dir(app: &mut EditorApp, dir: std::path::PathBuf, _ctx: &egui::Context) {
    app.has_hq_textures = detect_hq_textures(&dir);
    if !app.has_hq_textures
        && app.render_settings.terrain_quality == crate::viewport::renderer::TerrainQuality::High
    {
        log::info!("high.wz textures unavailable; falling back to Classic terrain quality");
        app.render_settings.terrain_quality = crate::viewport::renderer::TerrainQuality::Classic;
        app.terrain_dirty = true;
    }

    // If the path is unchanged (common after a config-load failure, where
    // the user is re-pointed at the same cache directory), keep everything
    // on-disk: thumbnails, ground texture cache, tileset textures. Only a
    // real dir change should invalidate caches keyed by content.
    let same_dir = app.config.data_dir.as_deref() == Some(dir.as_path());
    app.assets = Some(std::sync::Arc::new(crate::assets::FsAssetSource::new(
        dir.clone(),
    )));
    app.config.data_dir = Some(dir);
    app.config.save();
    if same_dir {
        return;
    }

    app.rt.stats_load_attempted = false;
    app.rt.tileset_load_attempted = false;
    app.stats = None;
    app.model_loader = None;
    app.tileset = None;
    app.ground_data = None;
    app.rt.ground_texture_load = None;
    app.model_thumbnails.invalidate_all();
    let _ = std::fs::remove_file(crate::config::ground_cache_dir().join(".precache_v9"));
    app.rt.ground_precache_attempted = false;
}

/// Returns true if `high.wz` was extracted into `data_dir`, detected by the
/// presence of any `tertiles*hw-256` decal directory under `base/texpages/`.
fn detect_hq_textures(data_dir: &std::path::Path) -> bool {
    let texpages = data_dir.join("base").join("texpages");
    for name in ["tertilesc1hw-256", "tertilesc2hw-256", "tertilesc3hw-256"] {
        if texpages.join(name).is_dir() {
            return true;
        }
    }
    false
}

/// Resolve the native startup asset wiring from the configured data directory.
///
/// Returns the asset source rooted at `data_dir` (when one is configured) and
/// whether Remastered (HQ) terrain textures are present. `EditorApp::new`
/// cannot call [`set_data_dir`] (it defers cache invalidation to a later
/// frame), so the constructor uses this to make the asset source and HQ flag
/// live before the first background load — without it the model loader is never
/// built and the HQ terrain option stays disabled.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn native_asset_init(
    config: &crate::config::EditorConfig,
) -> (Option<std::sync::Arc<dyn crate::assets::AssetSource>>, bool) {
    match config.data_dir.as_deref() {
        Some(dir) => (
            Some(
                std::sync::Arc::new(crate::assets::FsAssetSource::new(dir.to_path_buf()))
                    as std::sync::Arc<dyn crate::assets::AssetSource>,
            ),
            detect_hq_textures(dir),
        ),
        None => (None, false),
    }
}

/// Begin background extraction of `base.wz` into the persistent cache directory.
///
/// Also extracts `terrain_overrides/classic.wz` on top, which provides
/// pre-composited tile textures (transition tiles with ground types
/// already baked in, eliminating transparency issues).
///
/// If the cache already contains a valid `base/` tree the extraction is
/// skipped and `set_data_dir` is called immediately instead.
pub(super) fn start_base_wz_extraction(
    app: &mut EditorApp,
    wz_path: std::path::PathBuf,
    ctx: &egui::Context,
) {
    let output_dir = crate::config::extraction_cache_dir();

    // Marker file that indicates overlays (classic.wz + high.wz) have been applied.
    let overlay_marker = output_dir.join(".overlays_v9");

    // Fast path: cache already populated from a previous run AND
    // overlays have been applied.
    let already_extracted = (output_dir.join("base").join("texpages").exists()
        || output_dir.join("base").join("stats").exists())
        && overlay_marker.exists();
    if already_extracted {
        app.log("Using previously extracted base data.".to_string());
        set_data_dir(app, output_dir, ctx);
        return;
    }

    // Clean up stale cache from an older extraction that used the wrong
    // layout (files directly in cache_dir/ instead of cache_dir/base/).
    if output_dir.join("texpages").exists() || output_dir.join("stats").exists() {
        log::info!(
            "Removing stale extraction cache at {}",
            output_dir.display()
        );
        let _ = std::fs::remove_dir_all(&output_dir);
    }

    // Derive overlay paths from the base.wz parent directory.
    let overrides_dir = wz_path.parent().map(|p| p.join("terrain_overrides"));
    let classic_wz = overrides_dir.as_ref().map(|p| p.join("classic.wz"));
    let high_wz = overrides_dir.as_ref().map(|p| p.join("high.wz"));
    // mp.wz lives next to base.wz and carries the skirmish/multiplayer
    // template + structure set. Without it the editor only sees the 168
    // campaign templates and can't tell mp-allowed entries apart.
    let mp_wz = wz_path.with_file_name("mp.wz");

    let progress = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    let task = crate::startup::task::TaskHandle::spawn_with_progress(
        "base_wz_extract",
        progress.clone(),
        move |p| {
            // base.wz contents have no "base/" prefix (e.g. "texpages/...",
            // "stats/..."), but all loaders expect data_dir/base/... - so
            // extract into a "base/" subdirectory to bridge the gap.
            let base_subdir = output_dir.join("base");
            let result = wz_maplib::io_wz::extract_wz_to_dir(&wz_path, &base_subdir, |frac| {
                p.store((frac * 1000.0) as u32, std::sync::atomic::Ordering::Relaxed);
            });

            if let Err(ref e) = result {
                return Err(e.to_string());
            }

            // Overlay classic.wz pre-composited tiles on top of base.wz.
            // These replace RGBA transition tiles with fully opaque versions
            // that have ground type textures already baked in.
            if let Some(ref cwz) = classic_wz
                && cwz.exists()
            {
                log::info!(
                    "Overlaying classic.wz pre-composited tiles from {}",
                    cwz.display()
                );
                if let Err(e) =
                    wz_maplib::io_wz::extract_wz_to_dir_overwrite(cwz, &base_subdir, |_| {})
                {
                    log::warn!("Failed to extract classic.wz: {e}");
                }
            }

            // Overlay high.wz textures for High/Medium quality terrain.
            // Extract everything - ground diffuse, normal/specular maps,
            // 256px decal tiles, ground type definitions. high.wz ground
            // textures are in linear color space and converted to sRGB
            // during loading via `linear_to_srgb`.
            if let Some(ref hwz) = high_wz
                && hwz.exists()
            {
                log::info!(
                    "Extracting high.wz HQ terrain textures from {}",
                    hwz.display()
                );
                if let Err(e) =
                    wz_maplib::io_wz::extract_wz_to_dir_overwrite(hwz, &base_subdir, |_| {})
                {
                    log::warn!("Failed to extract high.wz: {e}");
                }
            }

            // Extract mp.wz alongside base.wz so the merged stats database
            // sees the skirmish/multiplayer template + structure set.
            if mp_wz.exists() {
                let mp_subdir = output_dir.join("mp");
                log::info!("Extracting mp.wz from {}", mp_wz.display());
                if let Err(e) = wz_maplib::io_wz::extract_wz_to_dir(&mp_wz, &mp_subdir, |_| {}) {
                    log::warn!("Failed to extract mp.wz: {e}");
                }
            } else {
                log::warn!(
                    "mp.wz not found next to base.wz at {}; skirmish-only \
                     templates and the campaign-only filter toggle will be \
                     unavailable",
                    mp_wz.display()
                );
            }

            // Write marker so subsequent launches skip re-extraction.
            let _ = std::fs::File::create(&overlay_marker);

            result.map(|()| output_dir).map_err(|e| e.to_string())
        },
    );

    app.rt.extraction_progress = Some(progress);
    app.rt.extraction_rx = Some(task);
    app.log("Extracting base.wz in the background...".to_string());
}

/// Try to load game stats from the configured data source.
///
/// Native reads the stats JSON files from disk; the web build reads them from
/// the in-memory `.wz` VFS through [`AssetSource`](crate::assets::AssetSource).
/// Both feed the same model loader, which already resolves PIE models through
/// the asset source.
pub(super) fn try_load_stats(app: &mut EditorApp, ctx: &egui::Context) {
    if app.stats.is_some() || app.rt.stats_load_attempted {
        return;
    }
    app.rt.stats_load_attempted = true;
    let Some(data_dir) = app.config.data_dir.clone() else {
        return;
    };
    let assets: std::sync::Arc<dyn crate::assets::AssetSource> =
        app.assets.clone().unwrap_or_else(|| {
            std::sync::Arc::new(crate::assets::FsAssetSource::new(data_dir.clone()))
        });

    let (db, base_template_count, mp_present) =
        match load_stats_database(&data_dir, assets.as_ref()) {
            Ok(Some(loaded)) => loaded,
            Ok(None) => return,
            Err(e) => {
                app.log(format!("Failed to load stats: {e}"));
                return;
            }
        };

    if !mp_present {
        let msg = "mp/stats/ missing; skirmish-only droid templates \
             (Cobra/Mantis/Python/Tiger/Vengeance variants) won't be available \
             and campaign-only templates can't be filtered"
            .to_string();
        log::warn!("{msg}");
        app.log(msg);
    }
    app.log(format!(
        "Loaded stats: {} structures, {} features, {} bodies, \
         {} templates ({} base + {} mp-only)",
        db.structures.len(),
        db.features.len(),
        db.bodies.len(),
        db.templates.len(),
        base_template_count,
        db.templates.len().saturating_sub(base_template_count),
    ));
    app.model_loader = Some(ModelLoader::new(assets, &db));
    app.stats = Some(db);
    app.objects_dirty = true;
    if app.validation_results.is_some() {
        app.run_validation();
    }
    ctx.request_repaint();
}

/// Load the base stats database and merge the `mp/stats` overlay when present.
///
/// Returns the database, its pre-merge template count, and whether an mp overlay
/// was found; `Ok(None)` when no base stats exist. Native reads the filesystem
/// directly so read errors surface; a failed mp merge is logged, not fatal.
#[cfg(not(target_arch = "wasm32"))]
fn load_stats_database(
    data_dir: &std::path::Path,
    _assets: &dyn crate::assets::AssetSource,
) -> Result<Option<(wz_stats::StatsDatabase, usize, bool)>, wz_stats::StatsError> {
    let stats_dir = data_dir.join("base/stats");
    if !stats_dir.exists() {
        return Ok(None);
    }
    let mut db = wz_stats::StatsDatabase::load_from_dir(&stats_dir)?;
    let base_template_count = db.templates.len();
    let mp_stats_dir = data_dir.join("mp/stats");
    let mp_present = mp_stats_dir.exists();
    if mp_present && let Err(e) = db.merge_from_dir(&mp_stats_dir) {
        log::warn!("Failed to merge mp stats: {e}");
    }
    Ok(Some((db, base_template_count, mp_present)))
}

/// Web variant of [`load_stats_database`]: reads the in-memory `.wz` VFS.
///
/// The VFS returns `None` for absent or unreadable entries alike, matching the
/// rest of the web asset pipeline.
#[cfg(target_arch = "wasm32")]
fn load_stats_database(
    _data_dir: &std::path::Path,
    assets: &dyn crate::assets::AssetSource,
) -> Result<Option<(wz_stats::StatsDatabase, usize, bool)>, wz_stats::StatsError> {
    let base_stats = std::path::Path::new("base/stats");
    if !assets.is_dir(base_stats) {
        return Ok(None);
    }
    let mut db =
        wz_stats::StatsDatabase::load_from_source(|name| Ok(assets.text(&base_stats.join(name))))?;
    let base_template_count = db.templates.len();
    let mp_stats = std::path::Path::new("mp/stats");
    let mp_present = assets.is_dir(mp_stats);
    if mp_present && let Err(e) = db.merge_from_source(|name| Ok(assets.text(&mp_stats.join(name))))
    {
        log::warn!("Failed to merge mp stats: {e}");
    }
    Ok(Some((db, base_template_count, mp_present)))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use crate::config::EditorConfig;

    fn config_with_data_dir(dir: Option<std::path::PathBuf>) -> EditorConfig {
        EditorConfig {
            data_dir: dir,
            ..EditorConfig::default()
        }
    }

    #[test]
    fn detect_hq_textures_true_when_hw256_dir_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let texpages = tmp.path().join("base").join("texpages");
        std::fs::create_dir_all(texpages.join("tertilesc2hw-256")).expect("mkdir");
        assert!(super::detect_hq_textures(tmp.path()));
    }

    #[test]
    fn detect_hq_textures_false_without_hw256_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("base").join("texpages")).expect("mkdir");
        assert!(!super::detect_hq_textures(tmp.path()));
    }

    #[test]
    fn native_asset_init_builds_source_and_detects_hq() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(
            tmp.path()
                .join("base")
                .join("texpages")
                .join("tertilesc1hw-256"),
        )
        .expect("mkdir");
        let config = config_with_data_dir(Some(tmp.path().to_path_buf()));
        let (assets, has_hq) = super::native_asset_init(&config);
        // A configured data dir must yield a live asset source, otherwise the
        // model loader is never built and thumbnails/textures never load.
        assert!(
            assets.is_some(),
            "expected an asset source for a set data_dir"
        );
        assert!(has_hq, "expected HQ detection from the hw-256 dir");
    }

    #[test]
    fn native_asset_init_enables_model_loader() {
        // Regression guard for the bug that produced zero thumbnails: the
        // preload renders nothing without a model loader, and the loader is
        // only built (in the stats-load handler) when startup hands back an
        // asset source. If a configured data dir yields `None` here, the loader
        // step is silently skipped and no thumbnails ever generate.
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("base").join("texpages")).expect("mkdir");
        let config = config_with_data_dir(Some(tmp.path().to_path_buf()));
        let (assets, _) = super::native_asset_init(&config);
        let assets = assets.expect("a configured data dir must yield an asset source");
        // Mirrors the stats-load handler; building this is what unblocks the
        // thumbnail preload. No GPU is required to construct the loader.
        let _loader = crate::viewport::model_loader::ModelLoader::new(
            assets,
            &wz_stats::StatsDatabase::default(),
        );
    }

    #[test]
    fn native_asset_init_source_present_without_hq() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("base").join("texpages")).expect("mkdir");
        let config = config_with_data_dir(Some(tmp.path().to_path_buf()));
        let (assets, has_hq) = super::native_asset_init(&config);
        assert!(
            assets.is_some(),
            "expected an asset source for a set data_dir"
        );
        assert!(!has_hq, "no hw-256 dir means HQ must be unavailable");
    }

    #[test]
    fn native_asset_init_none_without_data_dir() {
        let config = config_with_data_dir(None);
        let (assets, has_hq) = super::native_asset_init(&config);
        assert!(assets.is_none());
        assert!(!has_hq);
    }
}
