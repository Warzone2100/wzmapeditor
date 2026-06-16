//! GPU-rendered thumbnail cache for 3D models, droids, and structures.
//!
//! Used by the asset browser and droid designer. Thumbnails are rendered
//! on the GPU and cached to disk as PNG files so subsequent launches load
//! instantly. On first run (or after a [`CACHE_VERSION`] bump) a loading
//! screen generates all thumbnails up-front via [`ThumbnailCache::start_preload`].

mod cache;
mod generator;
mod preload;
#[cfg(target_arch = "wasm32")]
mod web_cache;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use eframe::egui_wgpu;
use egui::TextureHandle;

/// Maximum thumbnails to render per frame during normal browsing.
const THUMBNAILS_PER_FRAME: usize = 4;

/// Maximum new model disk loads per frame during normal browsing.
///
/// Each load involves filesystem search + PNG decode + GPU upload (~50-500ms).
/// Capping at 2 keeps frame times under ~1s during incremental population.
const MODEL_LOADS_PER_FRAME: usize = 2;

/// Per-frame preload caps when the editor is idle on the splash screen.
/// Aggressive: nothing else competes for the frame, so spend most of it.
const PRELOAD_PER_FRAME_SPLASH: usize = 32;
const PRELOAD_FRAME_BUDGET_MS_SPLASH: u128 = 50;

/// Per-frame preload caps when running alongside the live editor.
/// Conservative: keeps the asset browser and viewport responsive while
/// background tilesets fill in.
const PRELOAD_PER_FRAME_EDITOR: usize = 8;
const PRELOAD_FRAME_BUDGET_MS_EDITOR: u128 = 12;

/// Bump this when the rendering pipeline changes (shaders, lighting, camera
/// angle, clear color, `TCMask`, composite logic) to force re-generation.
///
/// Persisted thumbnails key off this: the native disk cache and the web
/// Cache Storage both version their entries by it.
pub(crate) const CACHE_VERSION: u32 = 5;

/// Tileset index (for `ModelLoader::set_tileset`) by name.
///
/// Order: arizona=0, urban=1, rockies=2.
pub fn tileset_index(name: &str) -> usize {
    match name {
        "urban" => 1,
        "rockies" => 2,
        _ => 0, // arizona (default)
    }
}

/// Progress state for the thumbnail preload loading screen.
#[derive(Debug)]
pub enum PreloadState {
    /// Preload not started, waiting for stats to load.
    Idle,
    /// Rendering thumbnails. `done` / `total` drives the progress bar.
    /// Both single-PIE component thumbnails (bodies, propulsions,
    /// weapons, turrets) and assembled composites (structures, features,
    /// droid templates) are bundled into one queue so the user sees a
    /// single progress bar rather than several flickering tasks.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "web skips the eager preload that constructs this state"
        )
    )]
    Rendering {
        work: Vec<PreloadItem>,
        done: usize,
        total: usize,
    },
    /// Current tileset complete, may have pending background tilesets.
    Complete,
    /// All tilesets fully processed, will not restart.
    Done,
}

/// A single thumbnail to generate during preload.
#[derive(Debug, Clone)]
pub struct PreloadItem {
    /// Cache key used for both the in-memory `HashMap` and the disk PNG filename.
    pub cache_key: String,
    /// Component models and offsets (single model = 1 entry, composite = many).
    pub parts: Vec<(String, glam::Vec3)>,
}

/// Thumbnail cache with per-frame generation budget and hover rotation.
pub struct ThumbnailCache {
    /// Generated textures keyed by `cache_key`.
    pub textures: HashMap<String, TextureHandle>,
    /// Cache keys that genuinely failed (file not found, parse error). Never retried.
    pub(crate) failed: HashSet<String>,
    /// Number of thumbnails rendered this frame.
    pub(crate) frame_budget: usize,
    /// Number of new model disk loads this frame.
    pub(crate) load_budget: usize,
    /// Resolved disk cache directory (set once preload starts).
    ///
    /// Includes the tileset name so each tileset gets its own cache:
    /// `thumb-cache/v1/arizona/`, `thumb-cache/v1/rockies/`, etc.
    pub disk_cache_dir: Option<PathBuf>,
    /// Preload loading screen state.
    pub preload: PreloadState,
    /// Current tileset name for per-tileset disk cache directories.
    pub current_tileset: String,
    /// Tilesets whose disk caches still need populating (background preload).
    pub pending_tilesets: Vec<String>,
    /// The app's active tileset, restored after background preload finishes.
    pub active_tileset: String,
    /// egui-side id of the droid designer's live preview texture.
    ///
    /// The preview renders directly into the renderer's `preview_thumb`
    /// GPU target every frame; this `TextureId` is handed out to
    /// `egui::Image` so egui samples that wgpu texture in the main pass,
    /// no per-frame GPU to CPU readback. Registered lazily on the first
    /// successful preview render and kept for the lifetime of the
    /// renderer (freeing it while the gpu pass is in-flight triggers an
    /// `egui_texid` shutdown panic).
    pub preview_texture_id: Option<egui::TextureId>,
    /// Thumbnail GPU-to-CPU readbacks awaiting completion.
    ///
    /// A single staging buffer backs all readbacks, so this holds at most
    /// one in-flight entry; [`ThumbnailCache::tick_readbacks`] drains
    /// completed entries each frame and routes the pixels to the texture
    /// cache and disk.
    pub(crate) pending_readbacks: Vec<cache::PendingReadback>,
    /// Thumbnails decoded from the browser Cache Storage, delivered across
    /// frames into [`Self::textures`] so the web build reuses generated
    /// thumbnails instead of re-rendering them every session.
    #[cfg(target_arch = "wasm32")]
    pub(crate) web_thumb_rx: Option<std::sync::mpsc::Receiver<(String, egui::ColorImage)>>,
    /// Tileset whose cached thumbnails the async loader has been started for.
    #[cfg(target_arch = "wasm32")]
    pub(crate) web_thumb_loaded_tileset: Option<String>,
}

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self {
            textures: HashMap::new(),
            failed: HashSet::new(),
            frame_budget: 0,
            load_budget: 0,
            disk_cache_dir: None,
            preload: PreloadState::Idle,
            current_tileset: "arizona".to_string(),
            pending_tilesets: Vec::new(),
            active_tileset: "arizona".to_string(),
            preview_texture_id: None,
            pending_readbacks: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            web_thumb_rx: None,
            #[cfg(target_arch = "wasm32")]
            web_thumb_loaded_tileset: None,
        }
    }
}

impl ThumbnailCache {
    /// Reset the per-frame budgets. Call once at the start of each frame.
    pub fn reset_frame_budget(&mut self) {
        self.frame_budget = 0;
        self.load_budget = 0;
    }

    /// Whether more thumbnail work is outstanding: either this frame's
    /// render budget is exhausted, or a readback is still in flight. Callers
    /// use this to schedule another repaint so the work completes next frame.
    pub fn has_pending(&self) -> bool {
        self.frame_budget >= THUMBNAILS_PER_FRAME || !self.pending_readbacks.is_empty()
    }

    /// Per-frame web thumbnail-cache pump: start the async load of the current
    /// tileset's cached thumbnails once, then drain decoded entries into the
    /// in-memory texture map so `cache_lookup` resolves them without a render.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn web_thumb_tick(&mut self, ctx: &egui::Context) {
        if self.web_thumb_loaded_tileset.as_deref() != Some(self.current_tileset.as_str()) {
            self.web_thumb_loaded_tileset = Some(self.current_tileset.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            self.web_thumb_rx = Some(rx);
            web_cache::start_load(self.current_tileset.clone(), tx, ctx.clone());
        }

        let mut received = Vec::new();
        let mut loader_finished = false;
        if let Some(rx) = &self.web_thumb_rx {
            loop {
                match rx.try_recv() {
                    Ok(item) => received.push(item),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        loader_finished = true;
                        break;
                    }
                }
            }
        }
        if loader_finished {
            self.web_thumb_rx = None;
        }
        for (key, image) in received {
            if self.textures.contains_key(&key) || self.failed.contains(&key) {
                continue;
            }
            let tex = ctx.load_texture(format!("thumb_{key}"), image, egui::TextureOptions::LINEAR);
            self.textures.insert(key, tex);
        }
    }
}

/// Borrow [`crate::viewport::ViewportResources`] from the egui renderer
/// under a read lock and pass it (with the device and queue) to `f`.
///
/// Collapses the unpack pattern that read every callback site:
/// acquire the renderer lock, downcast the callback resources, drop the
/// lock at the end of the closure. Returns whatever `f` returns, or
/// `None` if there is no `RenderState` or the resources are missing.
pub(crate) fn with_render_resources<T>(
    rs: &egui_wgpu::RenderState,
    f: impl FnOnce(&crate::viewport::ViewportResources, &wgpu::Device, &wgpu::Queue) -> Option<T>,
) -> Option<T> {
    let egui_rdr = rs.renderer.read();
    let resources = egui_rdr
        .callback_resources
        .get::<crate::viewport::ViewportResources>()?;
    f(resources, &rs.device, &rs.queue)
}

/// Mutable variant of [`with_render_resources`] that takes a write lock
/// on the renderer.
pub(crate) fn with_render_resources_mut<T>(
    rs: &egui_wgpu::RenderState,
    f: impl FnOnce(&mut crate::viewport::ViewportResources, &wgpu::Device, &wgpu::Queue) -> Option<T>,
) -> Option<T> {
    let mut egui_rdr = rs.renderer.write();
    let resources = egui_rdr
        .callback_resources
        .get_mut::<crate::viewport::ViewportResources>()?;
    f(resources, &rs.device, &rs.queue)
}
