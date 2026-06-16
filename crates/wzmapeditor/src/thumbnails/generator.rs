//! On-demand thumbnail generation.
//!
//! Wraps the renderer's [`render_thumbnail`] / [`render_preview_thumbnail`]
//! entry points behind the request_* public API. Knows how to translate
//! a stats key into the list of model parts that make up a composite
//! (structure + base + weapons, body + propulsion + turrets, etc).
//!
//! [`render_thumbnail`]: crate::viewport::renderer::EditorRenderer::render_thumbnail
//! [`render_preview_thumbnail`]: crate::viewport::renderer::EditorRenderer::render_preview_thumbnail

use eframe::egui_wgpu;
use egui::TextureHandle;

use super::cache::{CacheLookup, Kickoff};
use super::{
    MODEL_LOADS_PER_FRAME, THUMBNAILS_PER_FRAME, ThumbnailCache, with_render_resources_mut,
};
use crate::viewport::model_loader::ModelLoader;

impl ThumbnailCache {
    /// Public wrapper around the composite droid generator for callers
    /// outside the asset browser (e.g. the droid designer's live preview).
    pub fn request_droid_thumbnail(
        &mut self,
        ctx: &egui::Context,
        template_key: &str,
        stats: &wz_stats::StatsDatabase,
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Option<&TextureHandle> {
        self.get_or_generate_droid(ctx, template_key, stats, model_loader, render_state)
    }

    /// Public wrapper around the single-model thumbnail generator, used
    /// by the droid designer to render individual component icons.
    pub fn request_model_thumbnail(
        &mut self,
        ctx: &egui::Context,
        imd_name: &str,
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Option<&TextureHandle> {
        self.get_or_generate(ctx, imd_name, model_loader, render_state)
    }

    /// Render an arbitrary composite (e.g. turret = mount + weapon) and
    /// cache the result under `cache_key`. Used by the designer's
    /// component grid so a weapon tile shows the full mount+turret the
    /// same way the in-game design screen does, rather than a detached
    /// weapon PIE floating in space.
    pub fn request_composite_thumbnail(
        &mut self,
        ctx: &egui::Context,
        cache_key: &str,
        parts: &[(String, glam::Vec3)],
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Option<&TextureHandle> {
        match self.cache_lookup(ctx, cache_key, true) {
            CacheLookup::Resolved => return self.textures.get(cache_key),
            CacheLookup::Failed => return None,
            CacheLookup::NeedsGenerate => {}
        }
        if self.frame_budget >= THUMBNAILS_PER_FRAME {
            return None;
        }
        self.frame_budget += 1;
        let parts_vec = parts.to_vec();
        if self
            .render_gpu_composite(cache_key, &parts_vec, model_loader, render_state)
            .is_err()
        {
            self.failed.insert(cache_key.to_string());
        }
        None
    }

    /// Get a cached structure thumbnail, generating on demand.
    pub fn request_structure_thumbnail(
        &mut self,
        ctx: &egui::Context,
        structure_key: &str,
        stats: &wz_stats::StatsDatabase,
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Option<&TextureHandle> {
        self.get_or_generate_structure(ctx, structure_key, stats, model_loader, render_state)
    }

    /// Try to get or generate a thumbnail. Returns the texture if available.
    ///
    /// Checks: memory cache, then disk cache, then GPU render (with budget limits).
    fn get_or_generate(
        &mut self,
        ctx: &egui::Context,
        imd_name: &str,
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Option<&TextureHandle> {
        match self.cache_lookup(ctx, imd_name, true) {
            CacheLookup::Resolved => return self.textures.get(imd_name),
            CacheLookup::Failed => return None,
            CacheLookup::NeedsGenerate => {}
        }

        if self.frame_budget >= THUMBNAILS_PER_FRAME {
            return None;
        }

        self.frame_budget += 1;
        if self
            .try_generate_gpu(imd_name, model_loader, render_state)
            .is_err()
        {
            self.failed.insert(imd_name.to_string());
        }
        None
    }

    /// Dispatch a single-model thumbnail render on the GPU pipeline.
    ///
    /// Returns `Ok(())` once the readback is dispatched or deferred (the
    /// texture lands later via [`ThumbnailCache::tick_readbacks`]), or
    /// `Err(())` if the model genuinely failed (don't retry).
    ///
    /// [`ThumbnailCache::tick_readbacks`]: super::ThumbnailCache::tick_readbacks
    fn try_generate_gpu(
        &mut self,
        imd_name: &str,
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Result<(), ()> {
        let rs = render_state.ok_or(())?;
        let loader = model_loader.as_mut().ok_or(())?;

        if !loader.is_uploaded(imd_name) {
            if self.load_budget >= MODEL_LOADS_PER_FRAME {
                return Ok(());
            }
            self.load_budget += 1;
            let uploaded = with_render_resources_mut(rs, |resources, device, queue| {
                Some(loader.ensure_model(imd_name, &mut resources.renderer, device, queue))
            });
            match uploaded {
                Some(true) => {}
                Some(false) | None => return Err(()),
            }
        }

        let entry = crate::viewport::renderer::ThumbnailEntry {
            model_key: imd_name,
            offset: glam::Vec3::ZERO,
            team_color: crate::viewport::pie_mesh::TEAM_COLORS[0],
        };

        match self.kickoff_thumbnail(rs, &[entry], imd_name.to_string()) {
            Kickoff::Dispatched | Kickoff::Busy => Ok(()),
            Kickoff::Failed => Err(()),
        }
    }

    /// Generate a composite droid thumbnail (body + propulsion + weapons) via GPU.
    fn get_or_generate_droid(
        &mut self,
        ctx: &egui::Context,
        template_key: &str,
        stats: &wz_stats::StatsDatabase,
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Option<&TextureHandle> {
        let cache_key = format!("droid_{template_key}");
        // The designer's live preview is volatile: never cache to disk and
        // always allow a re-render. The buffer hash invalidates the
        // in-memory texture elsewhere.
        let is_preview = template_key == crate::designer::tabs::PREVIEW_ID;
        match self.cache_lookup(ctx, &cache_key, !is_preview) {
            CacheLookup::Resolved => return self.textures.get(&cache_key),
            CacheLookup::Failed => return None,
            CacheLookup::NeedsGenerate => {}
        }
        if self.frame_budget >= THUMBNAILS_PER_FRAME {
            return None;
        }

        self.frame_budget += 1;
        let Some(parts) = build_gpu_composite(template_key, Some(stats), model_loader) else {
            self.failed.insert(cache_key);
            return None;
        };
        if self
            .render_gpu_composite(&cache_key, &parts, model_loader, render_state)
            .is_err()
        {
            self.failed.insert(cache_key);
        }
        None
    }

    /// Generate a composite structure thumbnail (structure + base + weapons) via GPU.
    fn get_or_generate_structure(
        &mut self,
        ctx: &egui::Context,
        structure_key: &str,
        stats: &wz_stats::StatsDatabase,
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Option<&TextureHandle> {
        let cache_key = format!("struct_{structure_key}");
        match self.cache_lookup(ctx, &cache_key, true) {
            CacheLookup::Resolved => return self.textures.get(&cache_key),
            CacheLookup::Failed => return None,
            CacheLookup::NeedsGenerate => {}
        }
        if self.frame_budget >= THUMBNAILS_PER_FRAME {
            return None;
        }

        self.frame_budget += 1;
        let Some(parts) = build_gpu_composite(structure_key, Some(stats), model_loader) else {
            self.failed.insert(cache_key);
            return None;
        };
        if self
            .render_gpu_composite(&cache_key, &parts, model_loader, render_state)
            .is_err()
        {
            self.failed.insert(cache_key);
        }
        None
    }

    /// Dispatch a composite-model thumbnail render + readback under `cache_key`.
    ///
    /// Returns `Ok(())` once dispatched or deferred (the texture lands via
    /// [`ThumbnailCache::tick_readbacks`]), or `Err(())` if nothing could be
    /// rendered. Used by the asset browser / splash preload path.
    ///
    /// [`ThumbnailCache::tick_readbacks`]: super::ThumbnailCache::tick_readbacks
    pub(super) fn render_gpu_composite(
        &mut self,
        cache_key: &str,
        parts: &[(String, glam::Vec3)],
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Result<(), ()> {
        let rs = render_state.ok_or(())?;

        if !self.ensure_parts_uploaded(parts, model_loader, rs)? {
            return Ok(());
        }

        let entries = thumbnail_entries(parts);
        match self.kickoff_thumbnail(rs, &entries, cache_key.to_string()) {
            Kickoff::Dispatched | Kickoff::Busy => Ok(()),
            Kickoff::Failed => Err(()),
        }
    }

    /// Ensure every part PIE is uploaded to the renderer; returns
    /// `Ok(false)` when the per-frame upload budget is already spent so
    /// the caller can defer rendering until next frame.
    pub(super) fn ensure_parts_uploaded(
        &mut self,
        parts: &[(String, glam::Vec3)],
        model_loader: &mut Option<ModelLoader>,
        rs: &egui_wgpu::RenderState,
    ) -> Result<bool, ()> {
        let loader = model_loader.as_mut().ok_or(())?;
        let needs_load: Vec<&str> = parts
            .iter()
            .map(|(k, _)| k.as_str())
            .filter(|k| !loader.is_uploaded(k))
            .collect();

        if needs_load.is_empty() {
            return Ok(true);
        }
        if self.load_budget >= MODEL_LOADS_PER_FRAME {
            return Ok(false);
        }

        let load_budget = &mut self.load_budget;
        let ok = with_render_resources_mut(rs, |resources, device, queue| {
            for key in needs_load {
                *load_budget += 1;
                loader.ensure_model(key, &mut resources.renderer, device, queue);
            }
            Some(())
        })
        .is_some();
        if ok { Ok(true) } else { Err(()) }
    }

    /// Re-render the droid designer's live preview directly into the
    /// renderer's `preview_thumb` GPU texture and return the egui
    /// [`TextureId`] that references it. No pixels are copied back to
    /// the CPU, egui samples the rendered texture in its own pass.
    ///
    /// On the first successful render the preview target's view is
    /// registered with `egui_wgpu` via `register_native_texture`; the
    /// resulting id is cached for the lifetime of the cache. Freeing it
    /// at modal-close time is unsafe because the texture may still be
    /// bound by the current frame's render pass (freeing it triggers the
    /// `egui_texid_Managed(N) label has been destroyed` shutdown panic),
    /// so we keep the registration.
    pub fn update_droid_preview(
        &mut self,
        template_key: &str,
        y_rotation: f32,
        stats: &wz_stats::StatsDatabase,
        model_loader: &mut Option<ModelLoader>,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> Option<egui::TextureId> {
        let parts = build_gpu_composite(template_key, Some(stats), model_loader)?;
        let rs = render_state?;

        if !self.ensure_parts_uploaded(&parts, model_loader, rs).ok()? {
            // Upload budget spent; show the last good frame if any.
            return self.preview_texture_id;
        }

        let device = &rs.device;
        let queue = &rs.queue;
        let entries = thumbnail_entries(&parts);

        // Avoid holding a borrow of `callback_resources` at the same time
        // as `&mut egui_rdr`:
        //   1. Render into the preview target under a shared borrow.
        //   2. On first-ever success, create an owned view of the preview
        //      texture so the shared borrow can be released.
        //   3. Take `&mut egui_rdr` to register the view natively.
        let mut egui_rdr = rs.renderer.write();
        let needs_registration = self.preview_texture_id.is_none();
        let preview_view = {
            let resources = egui_rdr
                .callback_resources
                .get::<crate::viewport::ViewportResources>()?;
            let rendered = resources
                .renderer
                .render_preview_thumbnail(device, queue, &entries, y_rotation);
            if !rendered {
                return self.preview_texture_id;
            }
            needs_registration.then(|| resources.renderer.preview_color_view_fresh())
        };

        if let Some(view) = preview_view {
            let tex_id = egui_rdr.register_native_texture(device, &view, wgpu::FilterMode::Linear);
            self.preview_texture_id = Some(tex_id);
        }

        self.preview_texture_id
    }
}

pub(super) fn thumbnail_entries(
    parts: &[(String, glam::Vec3)],
) -> Vec<crate::viewport::renderer::ThumbnailEntry<'_>> {
    let team_color = crate::viewport::pie_mesh::TEAM_COLORS[0];
    parts
        .iter()
        .map(|(key, offset)| crate::viewport::renderer::ThumbnailEntry {
            model_key: key.as_str(),
            offset: *offset,
            team_color,
        })
        .collect()
}

/// Resolve the mount and turret model names for a template's system turret
/// (sensor / ECM / repair / construct / brain). Returns `(mount, model)`.
fn resolve_system_turret(
    tmpl: &wz_stats::templates::TemplateStats,
    stats: &wz_stats::StatsDatabase,
) -> (Option<String>, Option<String>) {
    if let Some(ref id) = tmpl.sensor
        && let Some(s) = stats.sensor.get(id)
    {
        return (s.mount_model.clone(), s.sensor_model.clone());
    }
    if let Some(ref id) = tmpl.ecm
        && let Some(e) = stats.ecm.get(id)
    {
        return (e.mount_model.clone(), e.sensor_model.clone());
    }
    if let Some(ref id) = tmpl.repair
        && let Some(r) = stats.repair.get(id)
    {
        return (r.mount_model.clone(), r.model.clone());
    }
    if let Some(ref id) = tmpl.construct
        && let Some(c) = stats.construct.get(id)
    {
        return (c.mount_model.clone(), c.sensor_model.clone());
    }
    if let Some(ref id) = tmpl.brain
        && let Some(b) = stats.brain.get(id)
    {
        // Brain uses its associated weapon's mount + model.
        if let Some(w) = b.turret.as_deref().and_then(|wid| stats.weapons.get(wid)) {
            return (w.mount_model.clone(), w.model.clone());
        }
    }
    (None, None)
}

/// Build a list of (`model_key`, offset) for GPU composite thumbnail rendering.
///
/// Resolves structure components (base pad + weapons) or droid components
/// (body + propulsion + weapons + system turrets) into model keys with connector offsets.
pub(super) fn build_gpu_composite(
    entry_id: &str,
    stats: Option<&wz_stats::StatsDatabase>,
    model_loader: &mut Option<ModelLoader>,
) -> Option<Vec<(String, glam::Vec3)>> {
    let stats = stats?;
    let loader = model_loader.as_mut()?;
    let mut parts: Vec<(String, glam::Vec3)> = Vec::new();

    if let Some(ss) = stats.structures.get(entry_id) {
        let struct_imd = ss.pie_model()?.to_string();
        parts.push((struct_imd.clone(), glam::Vec3::ZERO));

        if let Some(base) = ss.base_model.as_deref().or(ss.base_imd.as_deref()) {
            parts.push((base.to_string(), glam::Vec3::ZERO));
        }

        let connectors = loader.get_connectors(&struct_imd);
        for (wi, weapon_name) in ss.weapons.iter().enumerate() {
            if let Some(ws) = stats.weapons.get(weapon_name) {
                let connector = connectors.get(wi).copied().unwrap_or(glam::Vec3::ZERO);
                if let Some(ref mount) = ws.mount_model {
                    let mount_conn = loader
                        .get_connectors(mount)
                        .first()
                        .copied()
                        .unwrap_or(glam::Vec3::ZERO);
                    parts.push((mount.clone(), connector));
                    if let Some(ref wep) = ws.model {
                        parts.push((wep.clone(), connector + mount_conn));
                    }
                } else if let Some(ref wep) = ws.model {
                    parts.push((wep.clone(), connector));
                }
            }
        }

        // Fall back to ECM then sensor when no weapons are mounted, matching
        // WZ2100 renderStructureTurrets priority (display3d.cpp:2718). Walls,
        // corner walls and gates point at sentinel sensors but never render
        // a turret in game. Mirrors model_loader::resolve_structure_weapons.
        // ZNULLECM exists in ecm.json with no sensorModel, so we must check
        // the inner sensor_model rather than just stats.ecm.contains_key.
        if ss.weapons.is_empty() {
            let suppress = matches!(
                ss.structure_type.as_deref(),
                Some("WALL" | "CORNER WALL" | "GATE")
            );
            let mut head: Option<String> = None;
            let mut mount: Option<String> = None;
            if !suppress {
                if let Some(id) = ss.ecm_id.as_deref()
                    && let Some(es) = stats.ecm.get(id)
                    && let Some(ref model) = es.sensor_model
                {
                    head = Some(model.clone());
                    mount.clone_from(&es.mount_model);
                }
                if head.is_none()
                    && let Some(id) = ss.sensor_id.as_deref()
                    && let Some(sn) = stats.sensor.get(id)
                    && let Some(ref model) = sn.sensor_model
                {
                    head = Some(model.clone());
                    mount.clone_from(&sn.mount_model);
                }
            }

            if head.is_some() || mount.is_some() {
                let connector = connectors.first().copied().unwrap_or(glam::Vec3::ZERO);
                if let Some(mount_imd) = mount {
                    let mount_conn = loader
                        .get_connectors(&mount_imd)
                        .first()
                        .copied()
                        .unwrap_or(glam::Vec3::ZERO);
                    parts.push((mount_imd, connector));
                    if let Some(head_imd) = head {
                        parts.push((head_imd, connector + mount_conn));
                    }
                } else if let Some(head_imd) = head {
                    parts.push((head_imd, connector));
                }
            }
        }
        return Some(parts);
    }

    if let Some(tmpl) = stats.templates.get(entry_id) {
        let body_id = &tmpl.body;
        if !body_id.is_empty()
            && let Some(body_stat) = stats.bodies.get(body_id.as_str())
            && let Some(ref body_imd) = body_stat.model
        {
            // VTOL droids mount weapons at connectors 5+ (skipping
            // the first 5 ground-unit connectors). Ground droids
            // use connectors 0+. See WZ2100 src/component.cpp
            // VTOL_CONNECTOR_START.
            const VTOL_CONNECTOR_START: usize = 5;

            parts.push((body_imd.clone(), glam::Vec3::ZERO));
            let body_connectors = loader.get_connectors(body_imd);

            let prop_id = &tmpl.propulsion;
            if !prop_id.is_empty()
                && let Some(prop_stat) = stats.propulsion.get(prop_id.as_str())
                && let Some(ref prop_imd) = prop_stat.model
                && prop_imd != body_imd
            {
                parts.push((prop_imd.clone(), glam::Vec3::ZERO));
            }
            let is_vtol = stats
                .propulsion
                .get(tmpl.propulsion.as_str())
                .is_some_and(|p| {
                    p.propulsion_type
                        .as_deref()
                        .is_some_and(|t| t.eq_ignore_ascii_case("Lift"))
                });
            let connector_offset: usize = if is_vtol { VTOL_CONNECTOR_START } else { 0 };

            for (wi, weapon_name) in tmpl.weapons.iter().enumerate() {
                if let Some(ws) = stats.weapons.get(weapon_name) {
                    let connector = body_connectors
                        .get(connector_offset + wi)
                        .copied()
                        .unwrap_or(glam::Vec3::ZERO);
                    if let Some(ref mount) = ws.mount_model {
                        let mount_conn = loader
                            .get_connectors(mount)
                            .first()
                            .copied()
                            .unwrap_or(glam::Vec3::ZERO);
                        parts.push((mount.clone(), connector));
                        if let Some(ref wep) = ws.model {
                            parts.push((wep.clone(), connector + mount_conn));
                        }
                    } else if let Some(ref wep) = ws.model {
                        parts.push((wep.clone(), connector));
                    }
                }
            }

            // System turrets (sensor/ECM/repair/construct/brain)
            // mount at body connector 0.
            let sys_connector = body_connectors.first().copied().unwrap_or(glam::Vec3::ZERO);
            let (sys_mount, sys_model) = resolve_system_turret(tmpl, stats);
            if let Some(ref mount) = sys_mount {
                let mount_conn = loader
                    .get_connectors(mount)
                    .first()
                    .copied()
                    .unwrap_or(glam::Vec3::ZERO);
                parts.push((mount.clone(), sys_connector));
                if let Some(ref model) = sys_model {
                    parts.push((model.clone(), sys_connector + mount_conn));
                }
            } else if let Some(ref model) = sys_model {
                parts.push((model.clone(), sys_connector));
            }

            return Some(parts);
        }
    }

    None
}
