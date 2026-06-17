//! Startup system: splash screen, background loading pipeline, progress.
//!
//! Coordinates all background work that must complete before the editor
//! is usable: base.wz extraction, tileset/stats loading, ground texture
//! caching, model preparation, and thumbnail generation.

pub mod loading_ui;
pub mod pipeline;
pub mod splash_ui;
pub mod task;
pub mod workers;

use std::collections::HashMap;
use std::sync::mpsc;

/// Tileset being prefetched, paired with the channel that delivers its cached
/// decoded HQ layers (`filename -> RGBA bytes`) once.
#[cfg(target_arch = "wasm32")]
type HqPrefetch = (
    crate::config::Tileset,
    mpsc::Receiver<HashMap<String, Vec<u8>>>,
);

/// A cached last-opened map: its name stem and raw `.wz` archive bytes.
#[cfg(target_arch = "wasm32")]
type CachedMap = (String, Vec<u8>);

/// All in-flight background loading state for startup and mid-session reloads.
pub struct RuntimeTasks {
    /// Extraction progress in thousandths (0-1000) while base.wz extracts.
    pub extraction_progress: Option<std::sync::Arc<std::sync::atomic::AtomicU32>>,
    /// Delivers `Ok(data_dir)` or `Err(message)` when extraction finishes.
    pub extraction_rx: Option<task::TaskHandle<Result<std::path::PathBuf, String>>>,
    pub ground_texture_load: Option<GroundTextureLoadState>,
    pub ground_precache_rx: Option<mpsc::Receiver<GroundPrecacheResult>>,
    /// Pre-cache progress in thousandths (0-1000).
    pub ground_precache_progress: Option<std::sync::Arc<std::sync::atomic::AtomicU32>>,
    /// Latches once attempted, so we don't retry every frame.
    pub ground_precache_attempted: bool,
    /// Pre-cached ground data keyed by tileset name.
    pub precached_ground_data: HashMap<String, crate::viewport::ground_types::GroundData>,
    pub connector_precache_rx: Option<mpsc::Receiver<HashMap<String, Vec<glam::Vec3>>>>,
    pub map_model_load: Option<MapModelLoadState>,
    /// Latches once attempted, so we don't retry every frame on failure.
    pub stats_load_attempted: bool,
    /// Latches once attempted, so we don't retry every frame.
    pub tileset_load_attempted: bool,
    /// Delivers the downloaded `.wz` bytes, or an error message when the
    /// fetch fails.
    #[cfg(target_arch = "wasm32")]
    pub web_data_rx: Option<mpsc::Receiver<Result<crate::assets::WebDataArchives, String>>>,
    /// Live byte progress of the in-flight data download, for the launcher.
    #[cfg(target_arch = "wasm32")]
    pub web_data_progress: Option<std::sync::Arc<crate::web_data::WebFetchProgress>>,
    /// One-shot latch: set true when the auto-download first starts so the
    /// per-frame auto-trigger doesn't re-fire. Retrying after a failure goes
    /// through the setup card's button, which calls `begin_load` directly, so
    /// this flag is never reset.
    #[cfg(target_arch = "wasm32")]
    pub web_data_load_started: bool,
    /// Delivers the bytes of a `.wz` map the user picks via the web file
    /// `<input>`, or an error message when the read fails.
    #[cfg(target_arch = "wasm32")]
    pub web_open_map_rx: Option<mpsc::Receiver<Result<crate::web_map_io::PickedMap, String>>>,
    /// Concrete handle to the in-memory VFS, kept so an uploaded `high.wz` can
    /// be installed after the source is already live behind `EditorApp::assets`.
    #[cfg(target_arch = "wasm32")]
    pub web_vfs: Option<std::sync::Arc<crate::assets::WebVfsAssetSource>>,
    /// Delivers the uploaded `high.wz` bytes once read + cached, or an error.
    #[cfg(target_arch = "wasm32")]
    pub web_high_rx: Option<mpsc::Receiver<Result<Vec<u8>, String>>>,
    /// Live byte progress of the in-flight `high.wz` upload, for the UI.
    #[cfg(target_arch = "wasm32")]
    pub web_high_progress: Option<std::sync::Arc<crate::web_data::WebFetchProgress>>,
    /// In-flight frame-budgeted decode of the Remastered (HQ) ground/decal
    /// texture arrays. The browser has no worker thread, so the arrays decode
    /// a few layers per frame before the shared upload state machine runs.
    #[cfg(target_arch = "wasm32")]
    pub web_ground_decode: Option<crate::app::web_ground::WebGroundDecode>,
    /// Tileset whose HQ arrays have been decoded, so the decode isn't
    /// retriggered every frame. A tileset switch changes `current_tileset`,
    /// which no longer matches and re-arms the decode.
    #[cfg(target_arch = "wasm32")]
    pub web_hq_loaded_tileset: Option<crate::config::Tileset>,
    /// In-flight prefetch of cached decoded HQ layers for a tileset. Delivers a
    /// `filename -> RGBA bytes` map once; layers present here skip the
    /// transcode, the rest decode and are cached as they finish.
    #[cfg(target_arch = "wasm32")]
    pub web_hq_prefetch: Option<HqPrefetch>,
    /// Set when a fresh `high.wz` is uploaded: the next HQ arm skips the cache
    /// prefetch and decodes from scratch, overwriting any layers cached from a
    /// previous pack. One-shot.
    #[cfg(target_arch = "wasm32")]
    pub web_hq_skip_cache: bool,
    /// Latches true once the initial web load (Classic terrain, stats, and --
    /// when Remastered is selected -- the HQ ground decode) has completed once,
    /// permanently dismissing the full-screen loading overlay. Later
    /// mid-session reloads use the compact bottom-left indicator instead.
    #[cfg(target_arch = "wasm32")]
    pub web_initial_load_done: bool,
    /// Delivers the cached last-opened map (name + bytes) read at startup, or
    /// `None` when nothing was cached.
    #[cfg(target_arch = "wasm32")]
    pub web_last_map_rx: Option<mpsc::Receiver<Option<CachedMap>>>,
    /// The cached last-opened map, parked until the editor finishes booting.
    #[cfg(target_arch = "wasm32")]
    pub web_last_map_pending: Option<CachedMap>,
    /// One-shot latch: set once the startup auto-reopen has run or been skipped,
    /// so it never fires twice or clobbers a map the user opened during boot.
    #[cfg(target_arch = "wasm32")]
    pub web_last_map_restore_attempted: bool,
}

impl RuntimeTasks {
    pub fn new() -> Self {
        Self {
            extraction_progress: None,
            extraction_rx: None,
            ground_texture_load: None,
            ground_precache_rx: None,
            ground_precache_progress: None,
            ground_precache_attempted: false,
            precached_ground_data: HashMap::new(),
            connector_precache_rx: None,
            map_model_load: None,
            stats_load_attempted: false,
            tileset_load_attempted: false,
            #[cfg(target_arch = "wasm32")]
            web_data_rx: None,
            #[cfg(target_arch = "wasm32")]
            web_data_progress: None,
            #[cfg(target_arch = "wasm32")]
            web_data_load_started: false,
            #[cfg(target_arch = "wasm32")]
            web_open_map_rx: None,
            #[cfg(target_arch = "wasm32")]
            web_vfs: None,
            #[cfg(target_arch = "wasm32")]
            web_high_rx: None,
            #[cfg(target_arch = "wasm32")]
            web_high_progress: None,
            #[cfg(target_arch = "wasm32")]
            web_ground_decode: None,
            #[cfg(target_arch = "wasm32")]
            web_hq_loaded_tileset: None,
            #[cfg(target_arch = "wasm32")]
            web_hq_prefetch: None,
            #[cfg(target_arch = "wasm32")]
            web_hq_skip_cache: false,
            #[cfg(target_arch = "wasm32")]
            web_initial_load_done: false,
            #[cfg(target_arch = "wasm32")]
            web_last_map_rx: None,
            #[cfg(target_arch = "wasm32")]
            web_last_map_pending: None,
            #[cfg(target_arch = "wasm32")]
            web_last_map_restore_attempted: false,
        }
    }

    /// Extraction progress as a fraction in `[0.0, 1.0]`, or `None`.
    pub fn extraction_fraction(&self) -> Option<f32> {
        self.extraction_progress
            .as_ref()
            .map(|p| p.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0)
    }

    pub fn ground_precache_fraction(&self) -> Option<f32> {
        self.ground_precache_progress
            .as_ref()
            .map(|p| p.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0)
    }

    pub fn connectors_done(&self) -> bool {
        self.connector_precache_rx.is_none()
    }

    pub fn models_done(&self) -> bool {
        self.map_model_load.is_none()
    }

    pub fn model_fraction(&self) -> Option<f32> {
        self.map_model_load
            .as_ref()
            .map(|s| s.uploaded as f32 / s.total.max(1) as f32)
    }
}

/// Pre-decoded tileset images and atlas built off the main thread.
pub struct TilesetPayload {
    /// Pre-decoded tiles ready for `ctx.load_texture()`.
    pub tile_images: Vec<(u16, egui::ColorImage)>,
    pub source_dir: std::path::PathBuf,
    /// Flat RGBA atlas data ready for GPU upload.
    pub atlas: Option<crate::viewport::atlas::TileAtlas>,
}

/// Metadata that travels with a loaded map from the background thread.
pub struct LoadMapMeta {
    /// Source path (persisted for auto-reload on next launch).
    pub source_path: Option<std::path::PathBuf>,
    /// Writable location for Ctrl+S quick-save.
    pub save_path: Option<std::path::PathBuf>,
    /// Archive prefix for multi-map .wz files.
    pub archive_prefix: Option<String>,
}

/// Payload from the background ground texture loader. Each buffer is
/// flat RGBA; per-layer count matches across diffuse/normal/specular.
pub struct GroundTexturePayload {
    pub diffuse: Vec<u8>,
    /// Empty if normal maps are unavailable.
    pub normals: Vec<u8>,
    /// Empty if specular maps are unavailable.
    pub specular: Vec<u8>,
    /// One layer per tile index.
    pub decal_diffuse: Vec<u8>,
    /// Empty if decal normal maps are unavailable.
    pub decal_normal: Vec<u8>,
    /// Empty if decal specular maps are unavailable.
    pub decal_specular: Vec<u8>,
    pub num_decal_tiles: u32,
}

pub(crate) struct GroundPrecacheResult {
    pub message: String,
    /// Parsed ground data per tileset, keyed by name (e.g. "arizona").
    pub ground_data: HashMap<String, crate::viewport::ground_types::GroundData>,
}

/// Background ground texture loading + chunked GPU upload state.
///
/// Once the worker delivers `GroundTexturePayload`, GPU uploads happen
/// one chunk per frame (7 steps) so the UI stays responsive.
pub struct GroundTextureLoadState {
    pub receiver: mpsc::Receiver<GroundTexturePayload>,
    /// Needed for GPU upload and terrain mesh.
    pub ground_data: crate::viewport::ground_types::GroundData,
    /// Worker progress (0..1000). During upload this is repurposed to
    /// 1001..2000, mapped back to 0.0..1.0 in the UI.
    pub progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Payload waiting for GPU upload, populated on first `try_recv`.
    pub payload: Option<GroundTexturePayload>,
    /// Current upload step (0..=6). `None` means still awaiting payload.
    pub upload_step: Option<u32>,
    pub upload_views: GroundUploadViews,
}

/// Intermediate wgpu `TextureView`s accumulated during chunked upload.
/// Each field is populated by one upload step and consumed when the
/// final bind group is assembled in step 6.
#[derive(Default)]
pub struct GroundUploadViews {
    pub high_diffuse: Option<wgpu::TextureView>,
    pub high_normal: Option<wgpu::TextureView>,
    pub high_specular: Option<wgpu::TextureView>,
    pub decal_diffuse: Option<wgpu::TextureView>,
    pub decal_normal: Option<wgpu::TextureView>,
    pub decal_specular: Option<wgpu::TextureView>,
}

impl std::fmt::Debug for GroundTextureLoadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroundTextureLoadState")
            .finish_non_exhaustive()
    }
}

/// Incremental model loading state for map objects.
///
/// Models are prepared (disk I/O, parse, mesh build) on background
/// threads and delivered via channel. The main thread polls each frame
/// and does the cheap GPU upload.
pub struct MapModelLoadState {
    pub receiver: mpsc::Receiver<crate::viewport::model_loader::PreparedModel>,
    pub total: usize,
    pub uploaded: usize,
}

impl std::fmt::Debug for MapModelLoadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapModelLoadState")
            .field("total", &self.total)
            .field("uploaded", &self.uploaded)
            .finish_non_exhaustive()
    }
}
