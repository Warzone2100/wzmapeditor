//! Web data source: fetch the bundled Warzone 2100 `.wz` archives.
//!
//! The native build points [`AssetSource`](crate::assets::AssetSource) at a
//! directory on disk. The browser has no such path, so this module downloads
//! the archives the editor needs (`base.wz`, plus optional `mp.wz` and
//! `terrain_overrides/classic.wz`) from the host that serves the app, builds a
//! [`WebVfsAssetSource`](crate::assets::WebVfsAssetSource) over the bytes, and
//! advances the launcher straight to `Ready`.
//!
//! Each archive is fetched once and stored in the browser's Cache Storage, so
//! later reloads serve the ~120 MB of game data from cache rather than the
//! network. The download streams through a [`ReadableStream`] reader so the
//! launcher can show real byte-level progress, and the bytes arrive over a
//! channel that [`poll_web_data`] drains each frame.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::app::{EditorApp, StartupPhase};
use crate::assets::{WebDataArchives, WebVfsAssetSource};
use crate::web::{Drain, cache, drain_once};

/// Base path the archives are served from, relative to the page's base URL.
const DATA_BASE_PATH: &str = "data/";
/// Cache Storage bucket name; bump the suffix to invalidate cached archives.
const CACHE_NAME: &str = "wz-data-v1";

/// Archive paths relative to [`DATA_BASE_PATH`].
const BASE_WZ: &str = "base.wz";
const MP_WZ: &str = "mp.wz";
const CLASSIC_WZ: &str = "terrain_overrides/classic.wz";

/// Cache Storage key for the user-uploaded `high.wz`. It is never auto-fetched
/// (HQ is opt-in), so it is read from cache only — never from the network.
const HIGH_WZ_CACHE_KEY: &str = "user-upload/high.wz";

/// Upper bound on the up-front buffer reservation for a download. The
/// `content-length` header is server-supplied and only a progress hint, so it
/// is clamped before use as a capacity: on wasm `usize` is 32-bit, and a bogus
/// or hostile length must not provoke an allocation that aborts the module.
const MAX_PREALLOC_BYTES: u64 = 512 * 1024 * 1024;

/// Live progress of the current archive download, shared with the launcher UI.
#[derive(Debug)]
pub(crate) struct WebFetchProgress {
    received: AtomicU64,
    total: AtomicU64,
    label: Mutex<String>,
}

impl WebFetchProgress {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            received: AtomicU64::new(0),
            total: AtomicU64::new(0),
            label: Mutex::new("Connecting...".to_string()),
        })
    }

    /// Begin tracking a new archive: reset counters and set the display label.
    fn begin(&self, name: &str) {
        if let Ok(mut label) = self.label.lock() {
            *label = format!("Downloading {name}");
        }
        self.received.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
    }

    fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    fn add(&self, n: u64) {
        self.received.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn label(&self) -> String {
        self.label
            .lock()
            .map_or_else(|_| "Downloading game data".to_string(), |l| l.clone())
    }

    pub(crate) fn received_bytes(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    pub(crate) fn total_bytes(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Download fraction in `[0.0, 1.0]`, or `None` when the size is unknown.
    pub(crate) fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return None;
        }
        let received = self.received.load(Ordering::Relaxed);
        Some((received as f32 / total as f32).min(1.0))
    }
}

/// Start downloading the bundled archives and route them into `app`.
///
/// Stores the receiving end and a progress handle on
/// [`RuntimeTasks`](crate::startup::RuntimeTasks); [`poll_web_data`] drains the
/// channel each frame. Returns immediately — the fetch runs asynchronously.
pub(crate) fn begin_load(app: &mut EditorApp, ctx: &egui::Context) {
    let (tx, rx) = mpsc::channel();
    let progress = WebFetchProgress::new();
    app.rt.web_data_rx = Some(rx);
    app.rt.web_data_progress = Some(progress.clone());
    app.rt.web_data_load_started = true;
    // Clear any prior error so the launcher shows the progress bar on a retry.
    if let StartupPhase::Setup { error, .. } = &mut app.startup_phase {
        *error = None;
    }
    request_persistent_storage();

    let ctx = ctx.clone();
    spawn_local(async move {
        let result = load_all(&progress).await;
        let _ = tx.send(result);
        // Wake egui so `poll_web_data` runs on the next frame.
        ctx.request_repaint();
    });
}

/// Drain the fetch channel; inject the VFS or surface an error.
pub(crate) fn poll_web_data(app: &mut EditorApp, ctx: &egui::Context) {
    let outcome = match drain_once(&mut app.rt.web_data_rx, ctx, true) {
        Drain::Pending => return,
        Drain::Ready(outcome) => outcome,
        // The fetch task dropped its sender without producing a result (a
        // panicked or dropped future). Surface it so the setup card shows the
        // error and its Retry button instead of a frozen "Preparing..." bar.
        Drain::Closed => {
            app.rt.web_data_progress = None;
            set_setup_error(app, "The game-data download was interrupted. Please retry.");
            return;
        }
    };
    app.rt.web_data_progress = None;
    match outcome {
        Ok(archives) => {
            apply(app, archives);
            // A cached HQ pack restored at startup: start loading the transcoder.
            if app.rt.web_vfs.as_ref().is_some_and(|vfs| vfs.has_high()) {
                crate::viewport::basis::ensure_initialized(ctx);
            }
        }
        Err(msg) => set_setup_error(app, &msg),
    }
}

/// Build the VFS from the downloaded bytes and jump the launcher to `Ready`.
fn apply(app: &mut EditorApp, archives: WebDataArchives) {
    let Some(vfs) = WebVfsAssetSource::from_archives(archives) else {
        set_setup_error(app, "Downloaded base.wz is not a valid archive.");
        return;
    };

    let vfs = std::sync::Arc::new(vfs);
    app.rt.web_vfs = Some(vfs.clone());
    let assets: std::sync::Arc<dyn crate::assets::AssetSource> = vfs;
    app.assets = Some(assets);
    // Sentinel root: only its `is_some()`-ness gates the per-frame auto-loads;
    // the bytes come from the VFS, and the path is never read on the web build.
    app.config.data_dir = Some(std::path::PathBuf::from("/web-data"));
    app.config.setup_complete = true;
    // HQ terrain needs `high.wz` (KTX2), which the user uploads separately; the
    // toggle stays off until both the archive and the wasm decoder are present.
    app.has_hq_textures = false;

    app.rt.tileset_load_attempted = false;
    app.rt.stats_load_attempted = false;
    app.rt.ground_precache_attempted = false;
    app.stats = None;
    app.tileset = None;
    app.ground_data = None;
    app.model_loader = None;

    app.startup_phase = StartupPhase::Ready;
    app.log("Loaded Warzone 2100 data.".to_string());
}

fn set_setup_error(app: &mut EditorApp, msg: &str) {
    log::warn!("{msg}");
    app.log(msg.to_string());
    if let StartupPhase::Setup { error, .. } = &mut app.startup_phase {
        *error = Some(msg.to_string());
    }
}

/// Download every archive the VFS needs. `base.wz` is required; the optional
/// archives are skipped when absent (HTTP 404) or unreadable.
async fn load_all(progress: &WebFetchProgress) -> Result<WebDataArchives, String> {
    let base = match fetch_archive(BASE_WZ, progress).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Err(format!("{BASE_WZ} was not found on the server.")),
        Err(e) => return Err(format!("Failed to download {BASE_WZ}: {e}")),
    };
    let classic = fetch_archive(CLASSIC_WZ, progress).await.ok().flatten();
    let mp = fetch_archive(MP_WZ, progress).await.ok().flatten();
    let high = load_cached_high().await;
    Ok(WebDataArchives {
        base,
        classic,
        mp,
        high,
    })
}

/// Read a previously-uploaded `high.wz` back from Cache Storage, if present.
///
/// HQ is opt-in and user-supplied, so this only ever consults the cache — it
/// makes no network request.
async fn load_cached_high() -> Option<Vec<u8>> {
    let cache = cache::open(CACHE_NAME).await?;
    let resp = cache::match_url(&cache, HIGH_WZ_CACHE_KEY).await?;
    cache::read_response_bytes(&resp).await
}

/// Fetch one archive, preferring a cached copy. `Ok(None)` means the server
/// returned 404 (an optional archive that simply isn't published).
async fn fetch_archive(name: &str, progress: &WebFetchProgress) -> Result<Option<Vec<u8>>, String> {
    progress.begin(name);
    let url = format!("{DATA_BASE_PATH}{name}");
    let window = web_sys::window().ok_or("No browser window")?;

    let cache = cache::open(CACHE_NAME).await;
    if let Some(cache) = &cache
        && let Some(resp) = cache::match_url(cache, &url).await
    {
        return Ok(Some(stream_response(resp, progress).await?));
    }

    let resp = fetch(&window, &url).await?;
    if resp.status() == 404 {
        return Ok(None);
    }
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    // Tee the body so the cache write runs in the background while the caller
    // streams the original with live progress. Awaiting the cache write here
    // would buffer the whole archive before progress could move, so the first
    // (uncached) load — exactly when progress matters — would show nothing.
    if let Some(cache) = &cache
        && let Ok(clone) = resp.clone()
    {
        let cache = cache.clone();
        let url = url.clone();
        spawn_local(async move {
            cache::put_response(&cache, &url, &clone).await;
        });
    }
    Ok(Some(stream_response(resp, progress).await?))
}

/// Read a `Response` body to completion, reporting byte progress as it goes.
async fn stream_response(
    resp: web_sys::Response,
    progress: &WebFetchProgress,
) -> Result<Vec<u8>, String> {
    let total = content_length(&resp);
    let body = resp.body().ok_or("Response had no body")?;
    read_stream(body, total, progress).await
}

/// Drain a `ReadableStream` into a byte vector, reporting byte progress.
///
/// `total` seeds the progress bar and the output capacity; pass `0` when the
/// length is unknown.
async fn read_stream(
    stream: web_sys::ReadableStream,
    total: u64,
    progress: &WebFetchProgress,
) -> Result<Vec<u8>, String> {
    progress.set_total(total);

    let reader_val: JsValue = stream.get_reader().into();
    let reader: web_sys::ReadableStreamDefaultReader = reader_val
        .dyn_into()
        .map_err(|_| "Could not acquire a stream reader".to_string())?;

    let mut out: Vec<u8> = Vec::with_capacity(total.min(MAX_PREALLOC_BYTES) as usize);
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|e| js_err(&e))?;
        let done = js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if done {
            break;
        }
        let value =
            js_sys::Reflect::get(&chunk, &JsValue::from_str("value")).map_err(|e| js_err(&e))?;
        let bytes = js_sys::Uint8Array::new(&value);
        let len = bytes.length() as usize;
        let start = out.len();
        out.resize(start + len, 0);
        bytes.copy_to(&mut out[start..]);
        progress.add(len as u64);
    }
    Ok(out)
}

async fn fetch(window: &web_sys::Window, url: &str) -> Result<web_sys::Response, String> {
    let value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| js_err(&e))?;
    value
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "fetch did not return a Response".to_string())
}

fn content_length(resp: &web_sys::Response) -> u64 {
    resp.headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

/// Ask the browser to keep our storage durable so the cached archives are not
/// evicted under storage pressure. Fire-and-forget; failure is non-fatal.
fn request_persistent_storage() {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Ok(promise) = window.navigator().storage().persist() {
        spawn_local(async move {
            let _ = JsFuture::from(promise).await;
        });
    }
}

fn js_err(v: &JsValue) -> String {
    format!("{v:?}")
}

/// Read a `File`'s bytes via the asynchronous `Blob.arrayBuffer()` API.
///
/// Used by the web "Open map" file picker; the data archives are fetched from
/// the server rather than read from a local `File`.
pub(crate) async fn read_file_bytes(file: &web_sys::File) -> Result<Vec<u8>, String> {
    let buffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(|e: JsValue| format!("{e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Open a file picker for `high.wz`, then stream-read, cache, and deliver it.
///
/// The read runs in the background over a [`ReadableStream`] so the UI never
/// blocks and can show byte progress via [`RuntimeTasks::web_high_progress`].
/// The bytes are stored in Cache Storage so HQ survives reloads, and arrive
/// over a channel that [`poll_high_upload`] drains each frame.
///
/// [`RuntimeTasks::web_high_progress`]: crate::startup::RuntimeTasks::web_high_progress
pub(crate) fn begin_high_upload(app: &mut EditorApp, ctx: &egui::Context) {
    if app.rt.web_high_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let progress = WebFetchProgress::new();
    app.rt.web_high_rx = Some(rx);
    app.rt.web_high_progress = Some(progress.clone());

    let ctx = ctx.clone();
    crate::web::dom::pick_file(".wz", move |file| match file {
        Some(file) => spawn_local(async move {
            let _ = tx.send(read_and_cache_high(file, &progress).await);
            ctx.request_repaint();
        }),
        None => {
            // Picker dismissed: drop `tx`/`progress` so `poll_high_upload` sees
            // the channel disconnect and releases the upload latch.
            ctx.request_repaint();
        }
    });
}

/// Drain the HQ-upload channel; install the pack into the live VFS or report.
pub(crate) fn poll_high_upload(app: &mut EditorApp, ctx: &egui::Context) {
    let outcome = match drain_once(&mut app.rt.web_high_rx, ctx, true) {
        Drain::Pending => return,
        Drain::Ready(outcome) => outcome,
        // The picker was dismissed or the read task died: release the latch so
        // the upload affordance is usable again.
        Drain::Closed => {
            app.rt.web_high_progress = None;
            return;
        }
    };
    app.rt.web_high_progress = None;
    match outcome {
        Ok(bytes) => {
            if install_high(app, bytes) {
                // Warm up the transcoder now so HQ is ready to select shortly.
                crate::viewport::basis::ensure_initialized(ctx);
            }
        }
        Err(msg) => {
            log::warn!("{msg}");
            app.log(msg);
        }
    }
}

fn install_high(app: &mut EditorApp, bytes: Vec<u8>) -> bool {
    let Some(vfs) = app.rt.web_vfs.as_ref() else {
        app.log("Cannot install HQ terrain: data source not ready.".to_string());
        return false;
    };
    if vfs.set_high_archive(bytes) {
        // A new pack invalidates any in-flight or completed HQ decode; re-arm
        // it and force the next decode to bypass (and overwrite) cached layers
        // from a previous pack.
        app.rt.web_ground_decode = None;
        app.rt.web_hq_prefetch = None;
        app.rt.web_hq_loaded_tileset = None;
        app.rt.web_hq_skip_cache = true;
        app.log("High-quality terrain pack loaded.".to_string());
        true
    } else {
        app.log("Uploaded high.wz is not a valid archive.".to_string());
        false
    }
}

/// Read the picked file's bytes (with progress), then cache them best-effort.
async fn read_and_cache_high(
    file: web_sys::File,
    progress: &WebFetchProgress,
) -> Result<Vec<u8>, String> {
    let bytes = stream_file(&file, progress).await?;
    if bytes.is_empty() {
        return Err("The selected high.wz is empty.".to_string());
    }
    cache_uploaded_high(&bytes).await;
    Ok(bytes)
}

/// Read a picked file's bytes via its `ReadableStream`, reporting byte progress.
async fn stream_file(file: &web_sys::File, progress: &WebFetchProgress) -> Result<Vec<u8>, String> {
    read_stream(file.stream(), file.size() as u64, progress).await
}

/// Store the uploaded `high.wz` bytes in Cache Storage. Best-effort: a failure
/// only costs a re-upload next session, so errors are swallowed.
async fn cache_uploaded_high(bytes: &[u8]) {
    let mib = bytes.len() / (1024 * 1024);
    let Some(cache) = cache::open(CACHE_NAME).await else {
        return;
    };
    if cache::put_bytes(&cache, HIGH_WZ_CACHE_KEY, bytes.to_vec()).await {
        log::info!("Cached high.wz ({mib} MiB); it will be restored on reload.");
    } else {
        log::warn!("Failed to cache high.wz ({mib} MiB); it won't persist across reloads.");
    }
}

/// Latest HQ terrain release; the user downloads `high.wz` from here, then
/// uploads it. A normal browser navigation, so it is not subject to the CORS
/// rules that block programmatic fetches of GitHub release assets.
const HQ_RELEASE_URL: &str = "https://github.com/Warzone2100/data-terrain-high/releases/latest";

/// Open the HQ terrain release page in a new browser tab.
pub(crate) fn open_hq_release_page() {
    if let Some(window) = web_sys::window() {
        let _ = window.open_with_url_and_target(HQ_RELEASE_URL, "_blank");
    }
}

/// Draw a non-blocking progress overlay while a `high.wz` upload is in flight.
///
/// Shown only while [`RuntimeTasks::web_high_progress`] is set; the rest of the
/// editor stays interactive behind it.
///
/// [`RuntimeTasks::web_high_progress`]: crate::startup::RuntimeTasks::web_high_progress
pub(crate) fn draw_high_upload_progress(app: &EditorApp, ctx: &egui::Context) {
    let Some(progress) = app.rt.web_high_progress.as_ref() else {
        return;
    };
    egui::Window::new("hq_upload_progress")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .show(ctx, |ui| {
            ui.label("Loading high-quality terrain\u{2026}");
            match progress.fraction() {
                Some(fraction) => {
                    ui.add(egui::ProgressBar::new(fraction).show_percentage());
                }
                None => {
                    ui.spinner();
                }
            }
        });
}
