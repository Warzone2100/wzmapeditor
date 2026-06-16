//! Disk-backed cache for rendered thumbnails.
//!
//! Owns the versioned cache directory layout (`thumb-cache/v{N}/{tileset}/`),
//! the in-memory texture/failed sets lookup, and the background PNG save
//! worker. Anything that needs to "look up an existing entry or fall back
//! to disk" goes through [`CacheLookup`] so the version-and-eviction path
//! lives in one place.

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

use eframe::egui_wgpu;
use egui::{TextureHandle, TextureOptions};

#[cfg(not(target_arch = "wasm32"))]
use super::CACHE_VERSION;
use super::{ThumbnailCache, with_render_resources};
use crate::viewport::renderer::{
    READBACK_POOL_SIZE, ReadbackStatus, ThumbnailEntry, ThumbnailReadback,
};

/// Outcome of [`ThumbnailCache::cache_lookup`]: either the texture is
/// already known (hit, miss-to-failed, or freshly loaded from disk), or
/// we still need to generate it.
pub(super) enum CacheLookup {
    /// In-memory or disk hit. The caller can fetch the handle from
    /// `self.textures.get(cache_key)`.
    Resolved,
    /// Previously failed, do not retry.
    Failed,
    /// Not present anywhere. Caller should generate.
    NeedsGenerate,
}

/// A thumbnail readback in flight, plus the routing it resolves to once the
/// GPU-to-CPU copy completes.
///
/// `tileset` and `disk_dir` are captured at kickoff so a tileset switch
/// while the readback is in flight cannot surface stale pixels or write to
/// the wrong cache directory.
pub(crate) struct PendingReadback {
    readback: ThumbnailReadback,
    cache_key: String,
    tileset: String,
    disk_dir: Option<PathBuf>,
}

/// Outcome of dispatching a thumbnail render + readback.
pub(super) enum Kickoff {
    /// Submitted; the texture arrives via [`ThumbnailCache::tick_readbacks`].
    Dispatched,
    /// The staging-buffer pool is full; the caller should retry next frame.
    Busy,
    /// Nothing to render (no uploaded models); the caller should mark failed.
    Failed,
}

impl ThumbnailCache {
    /// Dispatch a thumbnail render + readback for `cache_key`.
    ///
    /// Refuses (returns [`Kickoff::Busy`]) once [`READBACK_POOL_SIZE`] readbacks
    /// are already in flight, so each holds a distinct staging slot.
    pub(super) fn kickoff_thumbnail(
        &mut self,
        rs: &egui_wgpu::RenderState,
        entries: &[ThumbnailEntry<'_>],
        cache_key: String,
    ) -> Kickoff {
        if self.pending_readbacks.len() >= READBACK_POOL_SIZE {
            return Kickoff::Busy;
        }
        let readback = with_render_resources(rs, |resources, device, queue| {
            resources
                .renderer
                .begin_thumbnail_readback(device, queue, entries, 0.0)
        });
        match readback {
            Some(readback) => {
                self.pending_readbacks.push(PendingReadback {
                    readback,
                    cache_key,
                    tileset: self.current_tileset.clone(),
                    disk_dir: self.disk_cache_dir.clone(),
                });
                Kickoff::Dispatched
            }
            None => Kickoff::Failed,
        }
    }

    /// Complete any thumbnail readbacks whose GPU copy has finished.
    ///
    /// Call once per frame before new kickoffs so finished readbacks free their
    /// staging slots for reuse. Non-blocking on both targets: on native a
    /// `PollType::Poll` advances the `map_async` callbacks; on web the
    /// browser fires them from its own event loop.
    pub fn tick_readbacks(
        &mut self,
        ctx: &egui::Context,
        render_state: Option<&egui_wgpu::RenderState>,
    ) {
        if self.pending_readbacks.is_empty() {
            return;
        }
        let Some(rs) = render_state else {
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        let _ = rs.device.poll(wgpu::PollType::Poll);

        let mut still_pending = Vec::new();
        for pending in std::mem::take(&mut self.pending_readbacks) {
            match pending.readback.status() {
                ReadbackStatus::Pending => still_pending.push(pending),
                ReadbackStatus::Failed => {
                    // The buffer was never mapped; return the slot to the pool.
                    pending.readback.release();
                    self.failed.insert(pending.cache_key);
                }
                ReadbackStatus::Ready => {
                    let slot = pending.readback.slot();
                    let image = with_render_resources(rs, |resources, _device, _queue| {
                        Some(resources.renderer.finish_thumbnail_readback(slot))
                    });
                    match image {
                        Some(image) => self.deliver_readback(ctx, &pending, image),
                        // Resources momentarily absent: the slot is still mapped,
                        // so keep tracking this readback (preventing a double-map
                        // on its buffer) and retry next frame.
                        None => still_pending.push(pending),
                    }
                }
            }
        }
        self.pending_readbacks = still_pending;

        if !self.pending_readbacks.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Route a completed readback's pixels to the texture cache and disk.
    ///
    /// Only the on-screen tileset's thumbnails enter the in-memory texture
    /// map; background-precache readbacks still warm the disk cache.
    fn deliver_readback(
        &mut self,
        ctx: &egui::Context,
        pending: &PendingReadback,
        image: egui::ColorImage,
    ) {
        let is_preview = pending
            .cache_key
            .contains(crate::designer::tabs::PREVIEW_ID);
        let display = is_preview
            || (pending.tileset == self.active_tileset
                && self.current_tileset == self.active_tileset);
        let save_dir = if is_preview {
            None
        } else {
            pending.disk_dir.clone()
        };
        // The web build has no disk cache; persist to browser Cache Storage so
        // generated thumbnails survive a reload.
        #[cfg(target_arch = "wasm32")]
        if !is_preview {
            super::web_cache::save_async(&pending.tileset, &pending.cache_key, &image);
        }
        if display {
            let tex = ctx.load_texture(
                format!("thumb_{}", pending.cache_key),
                image.clone(),
                TextureOptions::LINEAR,
            );
            self.textures.insert(pending.cache_key.clone(), tex);
        }
        if let Some(dir) = save_dir {
            Self::save_to_disk_async(dir, pending.cache_key.clone(), image);
        }
    }
}

impl ThumbnailCache {
    /// Resolve and create the versioned disk cache directory.
    ///
    /// Returns the path and whether the directory already existed (i.e. cache
    /// is warm). Wipes stale version directories on first call.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn ensure_disk_cache_dir(&mut self) -> (PathBuf, bool) {
        if let Some(ref dir) = self.disk_cache_dir {
            return (dir.clone(), dir.exists());
        }
        let base = crate::config::thumb_cache_dir();
        let versioned = base.join(format!("v{CACHE_VERSION}"));
        wipe_stale_versions(&base, &versioned);
        let tileset_dir = versioned.join(&self.current_tileset);
        let existed = tileset_dir.exists();
        let _ = std::fs::create_dir_all(&tileset_dir);
        self.disk_cache_dir = Some(tileset_dir.clone());
        (tileset_dir, existed)
    }

    /// Try to load a thumbnail from the disk cache.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn load_from_disk(
        &mut self,
        ctx: &egui::Context,
        cache_key: &str,
    ) -> Option<TextureHandle> {
        let dir = self.disk_cache_dir.as_ref()?;
        let png_path = dir.join(format!("{}.png", sanitize_filename(cache_key)));
        if !png_path.exists() {
            return None;
        }
        let img = image::open(&png_path).ok()?.into_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let pixels = img.into_raw();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
        Some(ctx.load_texture(
            format!("thumb_{cache_key}"),
            color_image,
            TextureOptions::LINEAR,
        ))
    }

    /// No disk cache on the web build; always a cache miss.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::unused_self,
        reason = "matches the native &mut self signature shared call sites expect"
    )]
    pub(super) fn load_from_disk(
        &mut self,
        _ctx: &egui::Context,
        _cache_key: &str,
    ) -> Option<TextureHandle> {
        None
    }

    /// Save a rendered thumbnail to the disk cache synchronously.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn save_to_disk(cache_dir: &Path, cache_key: &str, image: &egui::ColorImage) {
        let png_path = cache_dir.join(format!("{}.png", sanitize_filename(cache_key)));
        let w = image.size[0] as u32;
        let h = image.size[1] as u32;
        let pixels: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(egui::Color32::to_array)
            .collect();
        if let Some(buf) = image::RgbaImage::from_raw(w, h, pixels)
            && let Err(e) = buf.save(&png_path)
        {
            log::warn!("Failed to save thumbnail {}: {}", png_path.display(), e);
        }
    }

    /// Queue a PNG save on the dedicated background worker so the GPU
    /// pipeline can keep feeding renders without waiting on encode + I/O.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn save_to_disk_async(
        cache_dir: PathBuf,
        cache_key: String,
        image: egui::ColorImage,
    ) {
        let _ = save_dispatcher().send(SaveJob {
            dir: cache_dir,
            key: cache_key,
            image,
        });
    }

    /// The web build has no persistent thumbnail cache (no filesystem), so
    /// rendered thumbnails live only in the in-memory texture map.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn save_to_disk_async(
        _cache_dir: PathBuf,
        _cache_key: String,
        _image: egui::ColorImage,
    ) {
    }

    /// Centralised "is this cached anywhere" lookup. Inserts disk-loaded
    /// textures into the in-memory map as a side effect so the caller
    /// can read them with `self.textures.get(cache_key)`.
    pub(super) fn cache_lookup(
        &mut self,
        ctx: &egui::Context,
        cache_key: &str,
        check_disk: bool,
    ) -> CacheLookup {
        if self.textures.contains_key(cache_key) {
            return CacheLookup::Resolved;
        }
        if self.failed.contains(cache_key) {
            return CacheLookup::Failed;
        }
        if check_disk && let Some(tex) = self.load_from_disk(ctx, cache_key) {
            self.textures.insert(cache_key.to_string(), tex);
            return CacheLookup::Resolved;
        }
        CacheLookup::NeedsGenerate
    }

    /// Switch to a different tileset's thumbnail cache.
    ///
    /// Clears in-memory textures (wrong tileset) and resets preload state
    /// so thumbnails reload from the new tileset's disk cache. Disk caches
    /// are preserved per tileset.
    pub fn switch_tileset(&mut self, tileset_name: &str) {
        self.textures.clear();
        self.failed.clear();
        self.preload = super::PreloadState::Idle;
        self.disk_cache_dir = None;
        self.current_tileset = tileset_name.to_lowercase();
        self.active_tileset = self.current_tileset.clone();
        self.pending_tilesets.clear();
        log::info!("Thumbnail cache switched to tileset: {tileset_name}");
    }

    /// Wipe all cached thumbnails (memory and disk) for all tilesets.
    /// Used when the game data directory changes.
    pub fn invalidate_all(&mut self) {
        self.textures.clear();
        self.failed.clear();
        self.preload = super::PreloadState::Idle;
        self.disk_cache_dir = None;
        self.pending_tilesets.clear();
        self.active_tileset = self.current_tileset.clone();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let base = crate::config::thumb_cache_dir();
            let versioned = base.join(format!("v{CACHE_VERSION}"));
            if versioned.exists() {
                let _ = std::fs::remove_dir_all(&versioned);
            }
        }
        log::info!("Thumbnail cache invalidated (all tilesets)");
    }
}

/// Sanitize a cache key for use as a filename. Replaces characters
/// invalid on Windows with underscores.
pub(super) fn sanitize_filename(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

/// Remove sibling cache directories from older `CACHE_VERSION` values.
#[cfg(not(target_arch = "wasm32"))]
fn wipe_stale_versions(base: &Path, current: &Path) {
    if !base.exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() && p != current {
            log::info!("Removing stale thumbnail cache: {}", p.display());
            let _ = std::fs::remove_dir_all(&p);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct SaveJob {
    dir: PathBuf,
    key: String,
    image: egui::ColorImage,
}

/// Lazily-spawned thread that drains queued thumbnail PNG saves. One
/// worker is enough: encode + write is 1-5ms per 256x256 image and the
/// renderer produces them sequentially.
#[cfg(not(target_arch = "wasm32"))]
fn save_dispatcher() -> &'static std::sync::mpsc::Sender<SaveJob> {
    static DISPATCH: std::sync::OnceLock<std::sync::mpsc::Sender<SaveJob>> =
        std::sync::OnceLock::new();
    DISPATCH.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<SaveJob>();
        std::thread::Builder::new()
            .name("thumb-save".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    ThumbnailCache::save_to_disk(&job.dir, &job.key, &job.image);
                }
            })
            .expect("failed to spawn thumbnail save worker");
        tx
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_replaces_invalid_chars() {
        assert_eq!(sanitize_filename("foo/bar"), "foo_bar");
        assert_eq!(sanitize_filename("a:b*c?d"), "a_b_c_d");
        assert_eq!(sanitize_filename("plain_key.123"), "plain_key.123");
    }

    #[test]
    fn cache_lookup_hits_in_memory() {
        let mut cache = ThumbnailCache::default();
        let ctx = egui::Context::default();
        // Forge an in-memory entry by inserting a 1x1 image into the
        // egui texture manager and stashing the handle.
        let pixels = vec![255u8; 4];
        let img = egui::ColorImage::from_rgba_unmultiplied([1, 1], &pixels);
        let handle = ctx.load_texture("test", img, TextureOptions::LINEAR);
        cache.textures.insert("hit_key".to_string(), handle);

        match cache.cache_lookup(&ctx, "hit_key", false) {
            CacheLookup::Resolved => {}
            _ => panic!("expected Resolved"),
        }
    }

    #[test]
    fn cache_lookup_returns_failed() {
        let mut cache = ThumbnailCache::default();
        let ctx = egui::Context::default();
        cache.failed.insert("dead_key".to_string());
        match cache.cache_lookup(&ctx, "dead_key", false) {
            CacheLookup::Failed => {}
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn cache_lookup_needs_generate_when_unknown() {
        let mut cache = ThumbnailCache::default();
        let ctx = egui::Context::default();
        match cache.cache_lookup(&ctx, "unseen", false) {
            CacheLookup::NeedsGenerate => {}
            _ => panic!("expected NeedsGenerate"),
        }
    }

    #[test]
    fn invalidate_all_clears_textures_and_failed() {
        let mut cache = ThumbnailCache::default();
        let ctx = egui::Context::default();
        let pixels = vec![0u8; 4];
        let img = egui::ColorImage::from_rgba_unmultiplied([1, 1], &pixels);
        let handle = ctx.load_texture("t", img, TextureOptions::LINEAR);
        cache.textures.insert("k".to_string(), handle);
        cache.failed.insert("x".to_string());
        cache.disk_cache_dir = Some(PathBuf::from("/dev/null/nope"));
        cache.invalidate_all();
        assert!(cache.textures.is_empty());
        assert!(cache.failed.is_empty());
        assert!(cache.disk_cache_dir.is_none());
    }

    #[test]
    fn wipe_stale_versions_removes_other_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path();
        let stale = base.join("v3");
        let current = base.join("v5");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(stale.join("a.png"), b"x").unwrap();

        wipe_stale_versions(base, &current);

        assert!(!stale.exists(), "stale dir should be removed");
        assert!(current.exists(), "current dir should be preserved");
    }
}
