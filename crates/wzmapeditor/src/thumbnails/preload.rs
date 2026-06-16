//! Bulk preload of every structure, feature, and droid thumbnail.
//!
//! The work list is built once when stats finish loading, then drained
//! across many frames in [`ThumbnailCache::tick_preload`]. Two cadences
//! are supported: an aggressive splash-screen cadence (whole frame is
//! ours) and a conservative editor cadence (must coexist with the
//! viewport).

use eframe::egui_wgpu;

#[cfg(not(target_arch = "wasm32"))]
use super::PreloadItem;
#[cfg(not(target_arch = "wasm32"))]
use super::cache::sanitize_filename;
#[cfg(not(target_arch = "wasm32"))]
use super::generator::build_gpu_composite;
use super::{
    PRELOAD_FRAME_BUDGET_MS_EDITOR, PRELOAD_FRAME_BUDGET_MS_SPLASH, PRELOAD_PER_FRAME_EDITOR,
    PRELOAD_PER_FRAME_SPLASH, PreloadState, ThumbnailCache, with_render_resources_mut,
};
use crate::viewport::model_loader::ModelLoader;

impl ThumbnailCache {
    /// Start the preload process: build the work list from stats.
    ///
    /// Called once from `app.rs` after stats and model loader are ready.
    /// Only the active tileset is preloaded eagerly; other tileset disk
    /// caches populate lazily when the user switches tilesets.
    ///
    /// The web build skips the eager pass entirely: it has no thumbnail disk
    /// cache, so warming every structure/feature/template would parse and
    /// decode hundreds of PIE models inline on the single browser thread and
    /// freeze the tab. The asset browser renders thumbnails on demand (under a
    /// per-frame budget) instead.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            unused_variables,
            reason = "web skips the eager preload; thumbnails render on demand"
        )
    )]
    pub fn start_preload(
        &mut self,
        ctx: &egui::Context,
        stats: &wz_stats::StatsDatabase,
        model_loader: &mut Option<ModelLoader>,
    ) {
        self.active_tileset = self.current_tileset.clone();
        self.pending_tilesets.clear();
        #[cfg(target_arch = "wasm32")]
        {
            self.preload = PreloadState::Done;
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.start_preload_for_current(ctx, stats, model_loader);
    }

    /// Build the work list for the current tileset and start rendering.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_preload_for_current(
        &mut self,
        ctx: &egui::Context,
        stats: &wz_stats::StatsDatabase,
        model_loader: &mut Option<ModelLoader>,
    ) {
        let (_cache_dir, cache_warm) = self.ensure_disk_cache_dir();
        let is_active = self.current_tileset == self.active_tileset;

        let mut work: Vec<PreloadItem> = Vec::new();
        let mut loaded_from_disk = 0usize;

        for (key, ss) in &stats.structures {
            let cache_key = format!("struct_{key}");
            if self.try_warm_load(
                ctx,
                &cache_key,
                cache_warm,
                is_active,
                &mut loaded_from_disk,
            ) {
                continue;
            }
            if let Some(parts) = build_gpu_composite(key, Some(stats), model_loader) {
                work.push(PreloadItem { cache_key, parts });
            } else if let Some(imd) = ss.pie_model() {
                work.push(PreloadItem {
                    cache_key,
                    parts: vec![(imd.to_string(), glam::Vec3::ZERO)],
                });
            }
        }

        // Features keyed by imd_name to match `get_or_generate` lookups.
        for fs in stats.features.values() {
            if let Some(imd) = fs.pie_model() {
                let cache_key = imd.to_string();
                if is_active && self.textures.contains_key(&cache_key) {
                    continue;
                }
                if self.try_warm_load(
                    ctx,
                    &cache_key,
                    cache_warm,
                    is_active,
                    &mut loaded_from_disk,
                ) {
                    continue;
                }
                work.push(PreloadItem {
                    cache_key,
                    parts: vec![(imd.to_string(), glam::Vec3::ZERO)],
                });
            }
        }

        // The GPU model bind group uses a single SRV descriptor (no
        // per-model sampler), so caching every template doesn't fragment
        // the DX12 descriptor heap.
        for key in stats.templates.keys() {
            let cache_key = format!("droid_{key}");
            if self.try_warm_load(
                ctx,
                &cache_key,
                cache_warm,
                is_active,
                &mut loaded_from_disk,
            ) {
                continue;
            }
            if let Some(parts) = build_gpu_composite(key, Some(stats), model_loader) {
                work.push(PreloadItem { cache_key, parts });
            }
        }

        let total = work.len();
        log::info!(
            "Thumbnail preload ({}): {loaded_from_disk} from disk cache, {total} to render",
            self.current_tileset,
        );

        if total == 0 {
            self.preload = PreloadState::Complete;
        } else {
            self.preload = PreloadState::Rendering {
                work,
                done: 0,
                total,
            };
        }
    }

    /// Try to satisfy a preload entry from the warm disk cache. Returns
    /// `true` when the entry should be skipped (already resolved or PNG
    /// already on disk for an inactive tileset).
    #[cfg(not(target_arch = "wasm32"))]
    fn try_warm_load(
        &mut self,
        ctx: &egui::Context,
        cache_key: &str,
        cache_warm: bool,
        is_active: bool,
        loaded_from_disk: &mut usize,
    ) -> bool {
        if !cache_warm {
            return false;
        }
        if is_active {
            if let Some(tex) = self.load_from_disk(ctx, cache_key) {
                self.textures.insert(cache_key.to_string(), tex);
                *loaded_from_disk += 1;
                return true;
            }
            return false;
        }
        let Some(dir) = self.disk_cache_dir.as_ref() else {
            log::warn!("thumbnail preload: cache_warm without disk_cache_dir set");
            return false;
        };
        let png = dir.join(format!("{}.png", sanitize_filename(cache_key)));
        if png.exists() {
            *loaded_from_disk += 1;
            return true;
        }
        false
    }

    /// Advance the preload by rendering thumbnails up to the per-frame cap.
    ///
    /// `splash=true` uses the aggressive cap (idle splash screen),
    /// `splash=false` uses the editor cap (keeps live UI responsive).
    /// Returns `(done, total)` for the progress bar, or `None` if complete.
    pub fn tick_preload(
        &mut self,
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
        splash: bool,
    ) -> Option<(usize, usize)> {
        let total = match &self.preload {
            PreloadState::Rendering { total, .. } => *total,
            _ => return None,
        };

        let rs = render_state?;
        let (per_frame, budget_ms) = if splash {
            (PRELOAD_PER_FRAME_SPLASH, PRELOAD_FRAME_BUDGET_MS_SPLASH)
        } else {
            (PRELOAD_PER_FRAME_EDITOR, PRELOAD_FRAME_BUDGET_MS_EDITOR)
        };

        let mut rendered_this_frame = 0usize;
        let frame_start = web_time::Instant::now();

        while rendered_this_frame < per_frame {
            if rendered_this_frame > 0 && frame_start.elapsed().as_millis() >= budget_ms {
                break;
            }
            // The staging-buffer pool bounds how many readbacks can be in
            // flight; once it is full, wait for tick_readbacks to drain some
            // before dispatching more. Items that need no render (skipped
            // below) do not occupy a slot, so they keep flowing.
            if self.pending_readbacks.len() >= crate::viewport::renderer::READBACK_POOL_SIZE {
                break;
            }
            let item = match &mut self.preload {
                PreloadState::Rendering { work, .. } => match work.pop() {
                    Some(item) => item,
                    None => break,
                },
                _ => break,
            };

            if !ensure_parts_uploaded_for_preload(&item.parts, model_loader, rs) {
                if let PreloadState::Rendering { done, .. } = &mut self.preload {
                    *done += 1;
                }
                rendered_this_frame += 1;
                continue;
            }

            let entries = super::generator::thumbnail_entries(&item.parts);
            let _ = self.kickoff_thumbnail(rs, &entries, item.cache_key.clone());

            // Count the item as processed at dispatch; the texture and disk
            // write land later via tick_readbacks.
            if let PreloadState::Rendering { done, .. } = &mut self.preload {
                *done += 1;
            }
            rendered_this_frame += 1;
        }

        let (done, work_empty) = match &self.preload {
            PreloadState::Rendering { done, work, .. } => (*done, work.is_empty()),
            _ => return None,
        };

        // Defer completion until the final readbacks drain, so a tileset
        // switch cannot discard in-flight thumbnails.
        if work_empty && self.pending_readbacks.is_empty() {
            log::info!(
                "Thumbnail preload complete ({}): {done} rendered",
                self.current_tileset
            );
            self.preload = PreloadState::Complete;
            None
        } else {
            Some((done, total))
        }
    }
}

/// Upload every component PIE for an item before it gets rendered.
///
/// Preload runs in bulk and ignores the per-frame upload budget: the
/// splash screen has nothing else to do, and skipping uploads would
/// strand items in the queue forever. Returns `false` only when the
/// model loader or renderer resources are unavailable.
fn ensure_parts_uploaded_for_preload(
    parts: &[(String, glam::Vec3)],
    model_loader: &mut Option<ModelLoader>,
    rs: &egui_wgpu::RenderState,
) -> bool {
    let Some(loader) = model_loader.as_mut() else {
        return false;
    };
    with_render_resources_mut(rs, |resources, device, queue| {
        for (key, _) in parts {
            loader.ensure_model(key, &mut resources.renderer, device, queue);
        }
        Some(())
    })
    .is_some()
}
