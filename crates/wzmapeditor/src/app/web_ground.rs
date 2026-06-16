//! Frame-budgeted decode of the Remastered (HQ) ground texture arrays (web).
//!
//! The native build decodes the 1024² KTX2 ground/decal textures on a
//! background thread, then streams the GPU upload across frames. The browser
//! has no usable worker thread (GitHub Pages cannot send the COOP/COEP headers
//! `SharedArrayBuffer` needs), so decoding the whole set inline would freeze the
//! tab. This module decodes a few layers per frame into the same
//! [`GroundTexturePayload`] the shared upload state machine consumes, so the UI
//! stays responsive throughout.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;

use crate::app::EditorApp;
use crate::assets::AssetSource;
use crate::config::Tileset;
use crate::startup::{GroundTextureLoadState, GroundTexturePayload, GroundUploadViews};
use crate::viewport::ground_types::GroundData;
use crate::viewport::texture_loader::{solid_color_array, write_array_layer};

/// Ground-type texture-array resolution (per layer).
const GROUND_TEX_SIZE: u32 = 1024;
/// Decal tile resolution (per layer), matching the native HQ decode.
const DECAL_TEX_SIZE: u32 = 256;

/// Wall-clock budget for one frame's worth of texture decoding.
///
/// A single 1024² UASTC→RGBA transcode runs ~20-40 ms on the main thread, so
/// the loop always decodes at least one layer, then stops once this budget is
/// spent — keeping frames near interactive rates while a progress overlay
/// animates. ~16 ms is one frame at 60 Hz.
const DECODE_BUDGET_MS: u128 = 16;

/// Which array a decode job writes into.
#[derive(Clone, Copy, Debug)]
enum Target {
    Diffuse,
    Normal,
    Specular,
    DecalDiffuse,
    DecalNormal,
    DecalSpecular,
}

/// One layer to decode and where its RGBA bytes land in the target array.
struct Job {
    /// Source texture filename; doubles as the per-layer cache key.
    name: String,
    load: Box<dyn FnOnce() -> Option<Vec<u8>>>,
    target: Target,
    offset: usize,
}

/// In-flight HQ ground decode: pre-allocated arrays plus the remaining
/// per-layer decode jobs, drained a few per frame.
pub(crate) struct WebGroundDecode {
    tileset: Tileset,
    ground_data: GroundData,
    num_decal_tiles: u32,
    diffuse: Vec<u8>,
    normal: Vec<u8>,
    specular: Vec<u8>,
    decal_diffuse: Vec<u8>,
    decal_normal: Vec<u8>,
    decal_specular: Vec<u8>,
    /// Decoded layers restored from Cache Storage, keyed by source filename. A
    /// job whose `name` is present here copies from the map and skips the
    /// transcode; the entry is removed as it is consumed.
    cache: HashMap<String, Vec<u8>>,
    jobs: Vec<Job>,
    total_jobs: usize,
    progress: Arc<AtomicU32>,
}

impl std::fmt::Debug for WebGroundDecode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebGroundDecode")
            .field("tileset", &self.tileset)
            .field("remaining_jobs", &self.jobs.len())
            .field("total_jobs", &self.total_jobs)
            .finish_non_exhaustive()
    }
}

impl WebGroundDecode {
    /// Decode progress as a fraction in `[0.0, 1.0]`.
    pub(crate) fn fraction(&self) -> f32 {
        let done = self.total_jobs.saturating_sub(self.jobs.len());
        done as f32 / self.total_jobs.max(1) as f32
    }

    fn buffer_mut(&mut self, target: Target) -> &mut Vec<u8> {
        match target {
            Target::Diffuse => &mut self.diffuse,
            Target::Normal => &mut self.normal,
            Target::Specular => &mut self.specular,
            Target::DecalDiffuse => &mut self.decal_diffuse,
            Target::DecalNormal => &mut self.decal_normal,
            Target::DecalSpecular => &mut self.decal_specular,
        }
    }
}

/// Begin loading the HQ ground arrays when the user has selected Remastered
/// quality and the pack + transcoder are both ready.
///
/// Normally this kicks an async prefetch of any cached decoded layers; the
/// decode itself starts once the prefetch resolves (see [`poll`]). Immediately
/// after a fresh `high.wz` upload it skips the prefetch and decodes from
/// scratch so a previous pack's cached layers are never read.
pub(crate) fn maybe_start(app: &mut EditorApp) {
    use crate::viewport::renderer::TerrainQuality;

    if app.render_settings.terrain_quality != TerrainQuality::High {
        return;
    }
    if app.rt.web_ground_decode.is_some()
        || app.rt.ground_texture_load.is_some()
        || app.rt.web_hq_prefetch.is_some()
    {
        return;
    }
    if app.rt.web_hq_loaded_tileset == Some(app.current_tileset) {
        return;
    }
    let ready =
        app.rt.web_vfs.as_ref().is_some_and(|v| v.has_high()) && crate::viewport::basis::is_ready();
    if !ready {
        return;
    }
    // Metadata must be loaded first (the Classic path publishes it); the HQ
    // arrays reuse the same ground-type/decal layout.
    if app.tileset.is_none() || app.assets.is_none() {
        return;
    }

    let tileset = app.current_tileset;
    if std::mem::take(&mut app.rt.web_hq_skip_cache) {
        start_decode(app, tileset, HashMap::new());
        return;
    }

    let (tx, rx) = mpsc::channel();
    crate::app::web_ground_cache::start_prefetch(tileset, tx);
    app.rt.web_hq_prefetch = Some((tileset, rx));
    app.log(format!(
        "Checking cached Remastered (HQ) terrain for {}…",
        tileset.as_str()
    ));
}

/// Load ground metadata and arm the frame-budgeted decode for `tileset`,
/// seeding it with any `cache`d layers. On metadata failure, latch the tileset
/// as "done" so the splash gate can dismiss with Classic terrain still shown.
fn start_decode(app: &mut EditorApp, tileset: Tileset, cache: HashMap<String, Vec<u8>>) {
    let Some(assets) = app.assets.clone() else {
        return;
    };
    let Some(ground_data) = GroundData::load(assets.as_ref(), tileset.as_str()) else {
        log::warn!(
            "HQ decode: failed to load ground data for {}",
            tileset.as_str()
        );
        app.rt.web_hq_loaded_tileset = Some(tileset);
        return;
    };

    let cached = cache.len();
    let decode = build(&assets, tileset, ground_data, cache);
    let total = decode.total_jobs;
    app.rt.web_hq_loaded_tileset = Some(tileset);
    app.rt.web_ground_decode = Some(decode);
    app.log(format!(
        "Decoding Remastered (HQ) terrain ({total} layers, {cached} cached)…"
    ));
}

/// Build the decode state: pre-fill the arrays with their neutral defaults and
/// enqueue one decode job per present layer.
fn build(
    assets: &Arc<dyn AssetSource>,
    tileset: Tileset,
    ground_data: GroundData,
    cache: HashMap<String, Vec<u8>>,
) -> WebGroundDecode {
    let num_ground_layers = ground_data.ground_types.len() as u32;
    let num_decal_tiles = ground_data.tile_grounds.len() as u32;

    let texpages_rel = std::path::PathBuf::from("base/texpages");
    let tileset_128_rel = std::path::PathBuf::from(tileset.subpath());
    let tileset_256_rel = std::path::PathBuf::from(tileset.subpath_256());

    let ground_layer_bytes = (GROUND_TEX_SIZE * GROUND_TEX_SIZE * 4) as usize;
    let decal_layer_bytes = (DECAL_TEX_SIZE * DECAL_TEX_SIZE * 4) as usize;

    let mut jobs: Vec<Job> = Vec::new();

    for (i, gt) in ground_data.ground_types.iter().enumerate() {
        let offset = i * ground_layer_bytes;
        let filename = gt.filename.clone();
        let assets_d = assets.clone();
        let dir_d = texpages_rel.clone();
        jobs.push(Job {
            name: gt.filename.clone(),
            load: Box::new(move || {
                crate::viewport::texture_loader::load_ground_texture(
                    assets_d.as_ref(),
                    &dir_d,
                    &filename,
                    GROUND_TEX_SIZE,
                )
            }),
            target: Target::Diffuse,
            offset,
        });
        if let Some(nm) = gt.normal_filename.clone() {
            let assets_n = assets.clone();
            let dir_n = texpages_rel.clone();
            jobs.push(Job {
                name: nm.clone(),
                load: Box::new(move || {
                    crate::viewport::texture_loader::load_ground_texture(
                        assets_n.as_ref(),
                        &dir_n,
                        &nm,
                        GROUND_TEX_SIZE,
                    )
                }),
                target: Target::Normal,
                offset,
            });
        }
        if let Some(sm) = gt.specular_filename.clone() {
            let assets_s = assets.clone();
            let dir_s = texpages_rel.clone();
            jobs.push(Job {
                name: sm.clone(),
                load: Box::new(move || {
                    crate::viewport::texture_loader::load_ground_texture(
                        assets_s.as_ref(),
                        &dir_s,
                        &sm,
                        GROUND_TEX_SIZE,
                    )
                }),
                target: Target::Specular,
                offset,
            });
        }
    }

    for i in 0..num_decal_tiles {
        let offset = i as usize * decal_layer_bytes;
        let assets_d = assets.clone();
        let dir128 = tileset_128_rel.clone();
        let dir256 = tileset_256_rel.clone();
        jobs.push(Job {
            name: format!("tile-{i:02}.png"),
            load: Box::new(move || {
                crate::viewport::texture_loader::load_decal_tile(
                    assets_d.as_ref(),
                    &dir128,
                    &dir256,
                    i,
                    DECAL_TEX_SIZE,
                )
            }),
            target: Target::DecalDiffuse,
            offset,
        });
        let assets_n = assets.clone();
        let dir256_n = tileset_256_rel.clone();
        let nm = format!("tile-{i:02}_nm.png");
        jobs.push(Job {
            name: nm.clone(),
            load: Box::new(move || {
                crate::viewport::texture_loader::load_ground_texture(
                    assets_n.as_ref(),
                    &dir256_n,
                    &nm,
                    DECAL_TEX_SIZE,
                )
            }),
            target: Target::DecalNormal,
            offset,
        });
        let assets_s = assets.clone();
        let dir256_s = tileset_256_rel.clone();
        let sm = format!("tile-{i:02}_sm.png");
        jobs.push(Job {
            name: sm.clone(),
            load: Box::new(move || {
                crate::viewport::texture_loader::load_ground_texture(
                    assets_s.as_ref(),
                    &dir256_s,
                    &sm,
                    DECAL_TEX_SIZE,
                )
            }),
            target: Target::DecalSpecular,
            offset,
        });
    }

    let total_jobs = jobs.len();
    // Jobs are popped from the end; reverse so layer 0 decodes first.
    jobs.reverse();

    WebGroundDecode {
        tileset,
        ground_data,
        num_decal_tiles,
        // Diffuse defaults to mid-gray, matching the native loader's fallback.
        diffuse: vec![128u8; ground_layer_bytes * num_ground_layers as usize],
        normal: solid_color_array([128, 128, 255, 255], GROUND_TEX_SIZE, num_ground_layers),
        specular: solid_color_array([0, 0, 0, 255], GROUND_TEX_SIZE, num_ground_layers),
        // Missing decal tiles stay fully transparent (zeroed).
        decal_diffuse: vec![0u8; decal_layer_bytes * num_decal_tiles as usize],
        decal_normal: solid_color_array([128, 128, 255, 255], DECAL_TEX_SIZE, num_decal_tiles),
        decal_specular: solid_color_array([0, 0, 0, 255], DECAL_TEX_SIZE, num_decal_tiles),
        cache,
        jobs,
        total_jobs,
        progress: Arc::new(AtomicU32::new(0)),
    }
}

/// Drive the HQ ground load each frame: first resolve a pending cache prefetch
/// (which kicks the decode once it arrives), then drain a budget of decode
/// jobs and hand the finished payload to the shared GPU-upload state machine.
pub(crate) fn poll(ctx: &egui::Context, app: &mut EditorApp) {
    if poll_prefetch(ctx, app) {
        return;
    }

    let Some(mut decode) = app.rt.web_ground_decode.take() else {
        return;
    };

    // A tileset switch mid-decode invalidates the in-flight arrays; drop them
    // and let the next frame re-arm for the new tileset.
    if decode.tileset != app.current_tileset {
        return;
    }

    let start = web_time::Instant::now();
    while let Some(job) = decode.jobs.pop() {
        if let Some(rgba) = decode.cache.remove(&job.name) {
            let buffer = decode.buffer_mut(job.target);
            if !write_array_layer(buffer, job.offset, &rgba) {
                log::warn!(
                    "HQ cache: layer {} of {} bytes overruns array at offset {}",
                    job.name,
                    rgba.len(),
                    job.offset
                );
            }
        } else if let Some(rgba) = (job.load)() {
            {
                let buffer = decode.buffer_mut(job.target);
                if !write_array_layer(buffer, job.offset, &rgba) {
                    log::warn!(
                        "HQ decode: layer {} of {} bytes overruns array at offset {}",
                        job.name,
                        rgba.len(),
                        job.offset
                    );
                }
            }
            // Persist the freshly decoded layer so later sessions skip the
            // transcode. Fire-and-forget, best-effort.
            crate::app::web_ground_cache::save(decode.tileset, &job.name, rgba);
        }
        if decode.jobs.is_empty() || start.elapsed().as_millis() >= DECODE_BUDGET_MS {
            break;
        }
    }

    // No progress store here: the splash/overlay read decode progress via
    // `fraction()` (computed from the job counts), and the carried-over
    // `progress` atomic only matters once `finish` flips it to the upload phase.
    if decode.jobs.is_empty() {
        finish(app, decode);
    } else {
        app.rt.web_ground_decode = Some(decode);
    }
    ctx.request_repaint();
}

/// Resolve an in-flight cache prefetch, kicking the decode (seeded with the
/// returned layers) once it arrives. Returns `true` while a prefetch is in
/// flight, so the caller skips the decode pass this frame.
fn poll_prefetch(ctx: &egui::Context, app: &mut EditorApp) -> bool {
    let Some((tileset, rx)) = app.rt.web_hq_prefetch.as_ref() else {
        return false;
    };
    let tileset = *tileset;
    ctx.request_repaint();
    if tileset != app.current_tileset {
        // Switched away mid-prefetch; discard and let maybe_start re-arm.
        app.rt.web_hq_prefetch = None;
        return true;
    }
    match rx.try_recv() {
        Ok(cache) => {
            app.rt.web_hq_prefetch = None;
            start_decode(app, tileset, cache);
        }
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => {
            app.rt.web_hq_prefetch = None;
            start_decode(app, tileset, HashMap::new());
        }
    }
    true
}

/// Hand the fully decoded arrays to the shared chunked-upload state machine.
fn finish(app: &mut EditorApp, decode: WebGroundDecode) {
    let WebGroundDecode {
        ground_data,
        num_decal_tiles,
        diffuse,
        normal,
        specular,
        decal_diffuse,
        decal_normal,
        decal_specular,
        progress,
        ..
    } = decode;

    let payload = GroundTexturePayload {
        diffuse,
        normals: normal,
        specular,
        decal_diffuse,
        decal_normal,
        decal_specular,
        num_decal_tiles,
    };

    // The shared upload machine reads `receiver` only while `payload` is
    // `None`; the payload is already in hand, so this closed channel is never
    // polled. `progress` carries over into the overlay's "Uploading" phase.
    let (_tx, receiver) = std::sync::mpsc::channel();
    progress.store(1000, Ordering::Relaxed);

    app.rt.ground_texture_load = Some(GroundTextureLoadState {
        receiver,
        ground_data,
        progress,
        payload: Some(payload),
        upload_step: Some(0),
        upload_views: GroundUploadViews::default(),
    });
    app.log("Remastered (HQ) terrain decoded; uploading…".to_string());
}
