//! Runtime loading overlay UI and background task polling.
//!
//! Runs during normal editor operation (after the splash screen) to
//! handle mid-session background tasks: re-extraction, tileset changes,
//! ground texture loading, and model uploads.

use crate::app::EditorApp;
use crate::startup::splash_ui::splash_task_progress;

/// Width of the compact mid-session progress overlay. These tasks (e.g. a
/// Remastered-quality switch) run while the editor is already in use, so the
/// overlay tucks into the viewport's bottom-left rather than filling the
/// centre like the startup splash.
const OVERLAY_WIDTH: f32 = 240.0;

/// Cap on prepared-model GPU uploads per frame.
const MAP_MODELS_PER_FRAME: usize = 64;

const GROUND_UPLOAD_STEPS: u32 = 7;

/// Unified loading screen showing all active background tasks.
pub fn show_loading_screen(ctx: &egui::Context, app: &mut EditorApp) {
    // The web build's full-screen launcher splash owns all progress display
    // until the initial load finishes; suppress this compact corner overlay so
    // its bars don't duplicate the splash's in the bottom-left.
    #[cfg(target_arch = "wasm32")]
    if !app.rt.web_initial_load_done {
        return;
    }

    let mut tasks: Vec<(&str, Option<f32>)> = Vec::new();

    if let Some(ref p) = app.generator_dialog.progress {
        let frac = p.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0;
        tasks.push(("Generating map", Some(frac)));
    }

    // Runs once per cache version, decodes KTX2 to cached BIN files.
    if let Some(frac) = app.rt.ground_precache_fraction() {
        tasks.push(("Decoding ground textures", Some(frac)));
    }

    // On web, ground-texture loading is shown by the centered modal overlay
    // (`splash_ui::show_web_loading_overlay`), not this corner bar.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(ref state) = app.rt.ground_texture_load {
        let raw = state.progress.load(std::sync::atomic::Ordering::Relaxed);
        if raw > 1000 {
            let frac = ((raw - 1000) as f32 / 1000.0).min(1.0);
            tasks.push(("Uploading ground textures", Some(frac)));
        } else {
            let frac = raw as f32 / 1000.0;
            tasks.push(("Loading ground textures", Some(frac)));
        }
    }

    if let Some(frac) = app.rt.model_fraction() {
        tasks.push(("Loading object models", Some(frac)));
    }

    if tasks.is_empty() {
        return;
    }

    egui::Window::new("loading_overlay")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
        .fixed_size([OVERLAY_WIDTH, 0.0])
        .show(ctx, |ui| {
            ui.add_space(4.0);
            for (label, progress) in &tasks {
                splash_task_progress(ui, label, *progress);
                ui.add_space(4.0);
            }
        });

    ctx.request_repaint();
}

/// Poll mid-session extraction and handle completion.
pub fn show_extraction_progress(ctx: &egui::Context, app: &mut EditorApp) {
    // Check completion before drawing to avoid a one-frame UI delay.
    let finished = app
        .rt
        .extraction_rx
        .as_ref()
        .and_then(super::task::TaskHandle::try_take);

    if let Some(result) = finished {
        app.rt.extraction_rx = None;
        app.rt.extraction_progress = None;
        match result {
            Ok(data_dir) => {
                app.log(format!(
                    "Extraction complete, data dir: {}",
                    data_dir.display()
                ));
                app.set_data_dir(data_dir, ctx);
            }
            Err(msg) => {
                app.log(format!("Extraction failed: {msg}"));
            }
        }
        return;
    }

    if app.rt.extraction_progress.is_some() {
        ctx.request_repaint();
    }
}

/// Poll the background ground texture load and upload to GPU when ready.
///
/// Two-phase: phase 1 awaits the payload from the worker, phase 2 does
/// chunked uploads (7 steps, one per frame) so the UI thread never stalls.
pub fn poll_ground_texture_load(ctx: &egui::Context, app: &mut EditorApp) {
    if app.rt.ground_texture_load.is_none() {
        return;
    }

    {
        let state = app.rt.ground_texture_load.as_mut().expect("checked above");
        if state.payload.is_none() {
            if let Ok(payload) = state.receiver.try_recv() {
                state.payload = Some(payload);
                state.upload_step = Some(0);
                log::info!("[splash] Ground texture data received, starting chunked GPU upload");
            } else {
                ctx.request_repaint();
                return;
            }
        }
    }

    let Some(step) = app
        .rt
        .ground_texture_load
        .as_ref()
        .and_then(|s| s.upload_step)
    else {
        return;
    };

    // Upload-phase progress lands in 1001..~1858; UI maps it back to 0..1.
    if let Some(ref state) = app.rt.ground_texture_load {
        state.progress.store(
            1000 + step * (1000 / GROUND_UPLOAD_STEPS),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    // Clone so we don't hold a borrow on `app` across upload and finalize.
    // `RenderState` is Arc-wrapped, the clone is cheap.
    let Some(render_state) = app.wgpu_render_state.clone() else {
        return;
    };

    let state = app.rt.ground_texture_load.as_mut().expect("checked above");
    let payload = state.payload.as_ref().expect("populated in phase 1");
    let num_layers = state.ground_data.ground_types.len() as u32;
    let num_decal_tiles = payload.num_decal_tiles;

    let device = &render_state.device;
    let queue = &render_state.queue;

    match step {
        0 => {
            let mut egui_renderer = render_state.renderer.write();
            if let Some(resources) = egui_renderer
                .callback_resources
                .get_mut::<crate::viewport::ViewportResources>()
            {
                resources.renderer.upload_ground_texture_data(
                    device,
                    queue,
                    &payload.diffuse,
                    num_layers,
                    &state.ground_data.ground_scales,
                );
            }
            log::info!("[splash] GPU upload step 0/6: medium diffuse");
        }
        1 => {
            let view = crate::viewport::atlas_gpu::upload_texture_array(
                device,
                queue,
                "ground_diffuse_array",
                &payload.diffuse,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                1024,
                num_layers,
            );
            state.upload_views.high_diffuse = Some(view);
            log::info!("[splash] GPU upload step 1/6: high diffuse");
        }
        2 => {
            let view = crate::viewport::atlas_gpu::upload_texture_array(
                device,
                queue,
                "ground_normal_array",
                &payload.normals,
                wgpu::TextureFormat::Rgba8Unorm,
                1024,
                num_layers,
            );
            state.upload_views.high_normal = Some(view);
            log::info!("[splash] GPU upload step 2/6: high normals");
        }
        3 => {
            let view = crate::viewport::atlas_gpu::upload_texture_array(
                device,
                queue,
                "ground_specular_array",
                &payload.specular,
                wgpu::TextureFormat::Rgba8Unorm,
                1024,
                num_layers,
            );
            state.upload_views.high_specular = Some(view);
            log::info!("[splash] GPU upload step 3/6: high specular");
        }
        4 => {
            let view = crate::viewport::atlas_gpu::upload_texture_array(
                device,
                queue,
                "decal_diffuse_array",
                &payload.decal_diffuse,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                256,
                num_decal_tiles,
            );
            state.upload_views.decal_diffuse = Some(view);
            log::info!("[splash] GPU upload step 4/6: decal diffuse");
        }
        5 => {
            let normal_view = crate::viewport::atlas_gpu::upload_texture_array(
                device,
                queue,
                "decal_normal_array",
                &payload.decal_normal,
                wgpu::TextureFormat::Rgba8Unorm,
                256,
                num_decal_tiles,
            );
            let specular_view = crate::viewport::atlas_gpu::upload_texture_array(
                device,
                queue,
                "decal_specular_array",
                &payload.decal_specular,
                wgpu::TextureFormat::Rgba8Unorm,
                256,
                num_decal_tiles,
            );
            state.upload_views.decal_normal = Some(normal_view);
            state.upload_views.decal_specular = Some(specular_view);
            log::info!("[splash] GPU upload step 5/6: decal normals + specular");
        }
        6 => {
            let views = &state.upload_views;
            let scales = &state.ground_data.ground_scales;
            let mut egui_renderer = render_state.renderer.write();
            if let Some(resources) = egui_renderer
                .callback_resources
                .get_mut::<crate::viewport::ViewportResources>()
            {
                resources.renderer.create_ground_high_bind_group(
                    device,
                    scales,
                    views.high_diffuse.as_ref().expect("step 1 done"),
                    views.high_normal.as_ref().expect("step 2 done"),
                    views.high_specular.as_ref().expect("step 3 done"),
                    views.decal_diffuse.as_ref().expect("step 4 done"),
                    views.decal_normal.as_ref().expect("step 5 done"),
                    views.decal_specular.as_ref().expect("step 5 done"),
                );
            }
            drop(egui_renderer);
            log::info!("[splash] GPU upload step 6/6: bind group created");

            let final_state = app.rt.ground_texture_load.take().expect("checked above");
            app.ground_data = Some(final_state.ground_data);
            log::info!("[splash] Caching ground textures: GPU upload complete");
            if app.rt.ground_precache_rx.is_none() {
                log::info!("[splash] Caching ground textures: done");
            }
            app.log("Loaded ground textures".to_string());
            app.terrain_dirty = true;
            return;
        }
        _ => {}
    }

    if let Some(ref mut s) = app.rt.ground_texture_load
        && let Some(ref mut step) = s.upload_step
    {
        *step += 1;
    }
    ctx.request_repaint();
}

pub fn show_ground_precache_progress(ctx: &egui::Context, app: &mut EditorApp) {
    let finished = if let Some(ref rx) = app.rt.ground_precache_rx {
        rx.try_recv().ok()
    } else {
        None
    };

    if let Some(result) = finished {
        app.rt.ground_precache_rx = None;
        app.rt.ground_precache_progress = None;
        app.rt.precached_ground_data = result.ground_data;
        log::info!("[splash] Caching ground textures: precache complete");
        if app.ground_data.is_some() {
            log::info!("[splash] Caching ground textures: done");
        }
        app.log(result.message);
        return;
    }

    if app.rt.ground_precache_progress.is_some() {
        ctx.request_repaint();
    }
}

/// Drain prepared models from the worker and upload them to the GPU.
pub fn show_map_model_loading(ctx: &egui::Context, app: &mut EditorApp) {
    let Some(load_state) = app.rt.map_model_load.as_mut() else {
        return;
    };

    let Some(rs) = app.wgpu_render_state.as_ref() else {
        return;
    };
    let Some(loader) = app.model_loader.as_mut() else {
        return;
    };
    let device = &rs.device;
    let queue = &rs.queue;

    let mut uploaded_this_frame = 0usize;
    let mut channel_disconnected = false;
    {
        let mut egui_rdr = rs.renderer.write();
        if let Some(resources) = egui_rdr
            .callback_resources
            .get_mut::<crate::viewport::ViewportResources>()
        {
            loop {
                if uploaded_this_frame >= MAP_MODELS_PER_FRAME {
                    break;
                }
                match load_state.receiver.try_recv() {
                    Ok(prepared) => {
                        loader.upload_prepared(prepared, &mut resources.renderer, device, queue);
                        load_state.uploaded += 1;
                        uploaded_this_frame += 1;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        channel_disconnected = true;
                        break;
                    }
                }
            }
        }
    }

    let done = load_state.uploaded;
    let total = load_state.total;

    let all_done = done >= total || channel_disconnected;

    if all_done {
        app.rt.map_model_load = None;
        app.objects_dirty = true;
        ctx.request_repaint();
        log::info!("[splash] Caching object models: done ({done} uploaded)");
    } else {
        ctx.request_repaint();
    }
}

/// Advance thumbnail preload and cycle through tilesets. `splash`
/// controls the per-frame budget: aggressive while the launcher owns the
/// screen, conservative once the editor is interactive.
pub fn show_thumbnail_preload(ctx: &egui::Context, app: &mut EditorApp, splash: bool) {
    // Complete any in-flight GPU thumbnail readbacks before dispatching
    // more, so the shared staging buffer is freed for reuse this frame.
    app.model_thumbnails
        .tick_readbacks(ctx, app.wgpu_render_state.as_ref());

    // Restore (and lazily persist) generated thumbnails via browser Cache
    // Storage so the web build does not re-render them every session.
    #[cfg(target_arch = "wasm32")]
    app.model_thumbnails.web_thumb_tick(ctx);

    // Advance to the next pending tileset once the current one finishes,
    // so each tileset's disk cache gets populated.
    if matches!(
        app.model_thumbnails.preload,
        crate::thumbnails::PreloadState::Complete
    ) {
        if let Some(next_ts) = app.model_thumbnails.pending_tilesets.pop() {
            let prev_ts = app.model_thumbnails.current_tileset.clone();
            log::info!("[splash] Caching {prev_ts} model previews: done");
            log::info!("[splash] Caching {next_ts} model previews: started");
            if let Some(loader) = app.model_loader.as_mut() {
                loader.set_tileset(crate::thumbnails::tileset_index(&next_ts));
            }
            app.model_thumbnails.current_tileset = next_ts;
            app.model_thumbnails.disk_cache_dir = None;
            // Idle triggers start_preload on the next update tick.
            app.model_thumbnails.preload = crate::thumbnails::PreloadState::Idle;
            ctx.request_repaint();
            return;
        }
        // Restore the active tileset on the loader once all caches are warm.
        let last_ts = app.model_thumbnails.current_tileset.clone();
        log::info!("[splash] Caching {last_ts} model previews: done");
        let active = app.model_thumbnails.active_tileset.clone();
        if app.model_thumbnails.current_tileset != active {
            log::info!("All tileset thumbnails cached. Restoring tileset: {active}");
            if let Some(loader) = app.model_loader.as_mut() {
                loader.set_tileset(crate::thumbnails::tileset_index(&active));
            }
            app.model_thumbnails.current_tileset = active;
            app.model_thumbnails.disk_cache_dir = None;
        }
        app.model_thumbnails.preload = crate::thumbnails::PreloadState::Done;
        log::info!("[splash] All model previews: done");
        return;
    }

    if matches!(
        app.model_thumbnails.preload,
        crate::thumbnails::PreloadState::Idle | crate::thumbnails::PreloadState::Done
    ) {
        return;
    }

    let progress = app.model_thumbnails.tick_preload(
        &mut app.model_loader,
        app.wgpu_render_state.as_ref(),
        splash,
    );

    if progress.is_some() {
        ctx.request_repaint();
    }
}
