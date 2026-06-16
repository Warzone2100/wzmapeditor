//! 3D viewport panel with tool interaction.

mod ctrl_pick;
mod object_rendering;
mod overlays;

pub(crate) use object_rendering::collect_unloaded_models;

use eframe::egui_wgpu;
use egui::Ui;

use crate::app::EditorApp;
use crate::tools::{self, ToolId};
use crate::viewport::camera::Camera;
use crate::viewport::{TerrainPaintCallback, ViewportResources};

use wz_maplib::constants::TILE_UNITS_F32 as TILE_UNITS;

/// Sentinel value for `ModelInstance::team_color[3]` that signals the shader
/// to render a selection highlight (rim glow). The shader tests `team_color.a > 1.5`.
const SELECTION_ALPHA: f32 = 2.0;

pub fn show_viewport(ui: &mut Ui, app: &mut EditorApp) {
    if app.document.is_none() {
        ui.centered_and_justified(|ui| {
            ui.heading("wzmapeditor: open or create a map to begin");
        });
        return;
    }

    let available = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());

    let dt = ui.ctx().input(|i| i.stable_dt);

    // Picking uses a camera clone so we don't hold the renderer lock during input handling.
    let mut pick_camera: Option<Camera> = None;
    let mut brush_highlight = [0.0_f32; 4]; // [world_x, world_z, radius, active]
    let mut brush_highlight_extra = [[0.0_f32; 4]; 3]; // up to 3 mirrored highlights

    let render_state = app.wgpu_render_state.clone();

    if let Some(ref render_state) = render_state {
        let device = &render_state.device;
        let queue = &render_state.queue;

        {
            let mut egui_renderer = render_state.renderer.write();
            let resources = egui_renderer
                .callback_resources
                .get_mut::<ViewportResources>();

            if let Some(resources) = resources {
                if rect.width() > 0.0 && rect.height() > 0.0 {
                    resources.camera.aspect = rect.width() / rect.height();
                }
                resources.camera.fov = app.render_settings.fov_degrees.to_radians();

                resources.camera.process_input(&response, ui.ctx(), dt);

                // Shadow updates are cheap so they run live; lightmap recompute
                // is debounced (~6 frames = 100 ms at 60 fps) so slider-scrubbing
                // doesn't trigger a full-map raycast every frame.
                let sun_changed = resources
                    .renderer
                    .settings
                    .sun_direction
                    .iter()
                    .zip(app.render_settings.sun_direction.iter())
                    .any(|(a, b)| (a - b).abs() > f32::EPSILON);
                if sun_changed {
                    app.shadow_dirty = true;
                    app.sun_change_cooldown = 6;
                    // Keep frames flowing so the cooldown decrements after the
                    // user releases the slider with no further interaction.
                    ui.ctx().request_repaint();
                } else if app.sun_change_cooldown > 0 {
                    app.sun_change_cooldown -= 1;
                    if app.sun_change_cooldown == 0 {
                        app.lightmap_dirty = true;
                    } else {
                        ui.ctx().request_repaint();
                    }
                }
                resources.renderer.settings = app.render_settings;

                // Geometry changes invalidate the shadow map; a terrain rebuild
                // also invalidates the water mesh (depth blur uses neighbour
                // heights). Skip the cascade during an active brush stroke:
                // shadow re-render, water rebuild, lightmap recompute, and
                // heatmap upload are too expensive per tile crossing, and
                // finalize_stroke restores them on mouse release.
                let stroke_active = app.tool_state.stroke_active;
                if !stroke_active
                    && (app.terrain_dirty
                        || !app.terrain_dirty_tiles.is_empty()
                        || app.objects_dirty)
                {
                    app.shadow_dirty = true;
                }
                if app.terrain_dirty && !stroke_active {
                    app.water_dirty = true;
                }

                if let Some(ref doc) = app.document {
                    if app.terrain_dirty {
                        let map = &doc.map.map_data;
                        resources.renderer.upload_terrain_with_ground(
                            device,
                            map,
                            app.ground_data.as_ref(),
                            doc.map.terrain_types.as_ref(),
                        );

                        // Reset camera only when map dimensions change (new map
                        // load), not on edit-driven re-uploads.
                        if resources.renderer.terrain_gpu.is_some() {
                            let cam_needs_reset = {
                                let expected_extent =
                                    (map.width.max(map.height) as f32) * TILE_UNITS;
                                (resources.camera.far - expected_extent * 4.0).abs() > 1.0
                            };
                            if cam_needs_reset {
                                resources.camera = Camera::for_map(map.width, map.height);
                                resources.camera.aspect = rect.width() / rect.height();
                            }
                        }
                        app.terrain_dirty = false;
                        app.terrain_dirty_tiles.clear();
                        if !stroke_active {
                            app.lightmap_dirty = true;
                            if app.show_heatmap {
                                app.heatmap_dirty = true;
                            }
                        }
                        if !app.viewshed.is_idle() {
                            app.viewshed_dirty = true;
                        }
                    } else if !app.terrain_dirty_tiles.is_empty() {
                        // Partial update: write only the touched-tile vertex
                        // range via queue.write_buffer, vs rebuilding the full
                        // ~22 MB mesh per tile crossing.
                        let map = &doc.map.map_data;
                        let (mut min_x, mut min_y, mut max_x, mut max_y) =
                            (u32::MAX, u32::MAX, 0u32, 0u32);
                        for &(x, y) in &app.terrain_dirty_tiles {
                            min_x = min_x.min(x);
                            min_y = min_y.min(y);
                            max_x = max_x.max(x);
                            max_y = max_y.max(y);
                        }
                        if min_x <= max_x && min_y <= max_y {
                            resources.renderer.update_terrain_tile_rect(
                                queue,
                                map,
                                app.ground_data.as_ref(),
                                min_x,
                                min_y,
                                max_x,
                                max_y,
                            );
                        }
                        app.terrain_dirty_tiles.clear();
                        if !app.viewshed.is_idle() {
                            app.viewshed_dirty = true;
                        }
                    }

                    // Separate flag so non-water brush strokes skip the 500 KB
                    // double-clone inside `build_water_vertex_depths`.
                    if app.water_dirty {
                        let map = &doc.map.map_data;
                        if let Some(ref ttp) = doc.map.terrain_types {
                            resources.renderer.upload_water(device, map, ttp);
                            if let Some(ref assets) = app.assets {
                                resources.renderer.load_and_upload_water_textures(
                                    device,
                                    queue,
                                    assets.as_ref(),
                                    std::path::Path::new("base/texpages"),
                                );
                            }
                        } else {
                            log::info!("No terrain types (TTP) data, water detection unavailable");
                        }
                        app.water_dirty = false;
                    }

                    if app.lightmap_dirty {
                        let map = &doc.map.map_data;
                        let sun =
                            glam::Vec3::from_slice(&resources.renderer.settings.sun_direction);
                        let lightmap = crate::viewport::lightmap::compute_lightmap(map, sun);
                        resources.renderer.upload_lightmap(device, queue, &lightmap);
                        app.lightmap_dirty = false;
                    }

                    if app.show_heatmap && app.heatmap_dirty {
                        if let (Some(ttp), Some(stats)) = (&doc.map.terrain_types, &app.stats)
                            && let Some(tt) = &stats.terrain_table
                        {
                            let mut lut = [0_u32; 512];
                            for (i, terrain_type) in ttp.terrain_types.iter().enumerate().take(512)
                            {
                                lut[i] = *terrain_type as u32;
                            }
                            let speeds = tt.speed_column(app.heatmap_propulsion);
                            resources
                                .renderer
                                .upload_heatmap_data(device, queue, &lut, &speeds);
                        }
                        app.heatmap_dirty = false;
                    }

                    if app.viewshed.show_range_on_select
                        && let Some(ref stats) = app.stats
                    {
                        let sources = crate::viewshed::collect_sources(
                            &doc.map.structures,
                            &app.selection,
                            stats,
                        );
                        let sig = crate::viewshed::selection_sig(&sources);
                        if sig != app.viewshed.last_selection_sig {
                            app.viewshed_dirty = true;
                            app.viewshed.last_selection_sig = sig;
                        }
                        if app.viewshed_dirty {
                            let frame = crate::viewshed::compute_viewshed(
                                &doc.map.map_data,
                                &doc.map.structures,
                                &sources,
                                stats,
                            );
                            resources.renderer.upload_viewshed_frame(device, &frame);
                            app.viewshed_dirty = false;
                        }
                    }
                }

                // Disk uploads on first map load are deferred to
                // `show_map_model_loading` (with a progress bar). The path here
                // only runs the fast instance-building pass when models are
                // cached.
                if app.objects_dirty && app.rt.map_model_load.is_none() {
                    // Sync the tileset BEFORE checking which models need uploading:
                    // set_tileset() clears the uploaded set on tileset change, so
                    // collect_unloaded_models() will then correctly detect models
                    // that need re-uploading with the new texture pages.
                    if let Some(loader) = app.model_loader.as_mut() {
                        loader.set_tileset(app.current_tileset.texture_index());
                    }

                    let unloaded = collect_unloaded_models(app);
                    if unloaded.is_empty() {
                        object_rendering::prepare_object_rendering(
                            app,
                            &mut resources.renderer,
                            device,
                            queue,
                        );
                        app.objects_dirty = false;
                        // Object additions/moves change cast shadows.
                        app.shadow_dirty = true;
                    } else {
                        let total = unloaded.len();
                        log::info!(
                            "Map needs {total} new models, starting parallel background load"
                        );
                        // Background threads handle disk I/O, parse, and mesh
                        // build; the main thread polls the receiver and does
                        // GPU uploads.
                        let receiver = app
                            .model_loader
                            .as_ref()
                            .expect("model_loader required for model loading")
                            .prepare_models_background(unloaded);
                        log::info!("[splash] Caching object models: started ({total} to upload)");
                        app.rt.map_model_load = Some(crate::app::MapModelLoadState {
                            receiver,
                            total,
                            uploaded: 0,
                        });
                        // Leave objects_dirty set; it gets re-cleared when loading completes.
                    }
                }

                if let Some((tx, ty)) = app.hovered_tile {
                    let brush_size = app
                        .tool_state
                        .tools
                        .get(&app.tool_state.active_tool)
                        .and_then(|t| t.brush_radius_tiles());
                    if let Some(brush_size) = brush_size {
                        let radius = (brush_size as f32 + 0.5) * TILE_UNITS;
                        if let Some(ref doc) = app.document {
                            let map_w = doc.map.map_data.width;
                            let map_h = doc.map.map_data.height;
                            let pts = tools::mirror::mirror_points(
                                tx,
                                ty,
                                map_w,
                                map_h,
                                app.tool_state.mirror_mode,
                            );
                            for (i, &(px, py)) in pts.iter().enumerate() {
                                let wx = px as f32 * TILE_UNITS + TILE_UNITS * 0.5;
                                let wz = py as f32 * TILE_UNITS + TILE_UNITS * 0.5;
                                if i == 0 {
                                    brush_highlight = [wx, wz, radius, 1.0];
                                } else {
                                    brush_highlight_extra[i - 1] = [wx, wz, radius, 1.0];
                                }
                            }
                        } else {
                            let world_x = tx as f32 * TILE_UNITS + TILE_UNITS * 0.5;
                            let world_z = ty as f32 * TILE_UNITS + TILE_UNITS * 0.5;
                            brush_highlight = [world_x, world_z, radius, 1.0];
                        }
                    }
                }

                if let Some((wx, wz)) = app.focus_request.take() {
                    let target_y = app
                        .document
                        .as_ref()
                        .and_then(|doc| {
                            let tx = (wx as u32) >> 7;
                            let tz = (wz as u32) >> 7;
                            doc.map.map_data.tile(tx, tz).map(|t| t.height as f32)
                        })
                        .unwrap_or(0.0);
                    resources.camera.focus_on(wx, wz, target_y);
                }

                {
                    resources.particle_system.set_weather(app.view_weather);
                    let (map_ref, ttp_ref) = match app.document.as_ref() {
                        Some(doc) => (Some(&doc.map.map_data), doc.map.terrain_types.as_ref()),
                        None => (None, None),
                    };
                    resources.particle_system.update(
                        dt,
                        resources.camera.position,
                        map_ref,
                        ttp_ref,
                    );

                    let (right, up) = resources.camera.billboard_axes();
                    let (verts, idxs) = resources.particle_system.build_mesh_into(right, up);
                    resources.renderer.upload_particles(device, verts, idxs);
                }

                pick_camera = Some(resources.camera.clone());
            }
        }

        // Drive continuous repaints so water/particle animation stays smooth
        // without user input. Skip while unfocused to avoid burning background
        // frames; egui wakes us on the next focus event.
        let has_doc = app.document.is_some();
        let water_on = app.render_settings.water_enabled;
        let particles_on = !matches!(
            app.view_weather,
            wz_maplib::Weather::Default | wz_maplib::Weather::Clear
        );
        if app.window_focused && has_doc && (water_on || particles_on) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }

        // Uniform buffer is written in prepare() so the time value is fresh
        // when paint() runs. Consuming `shadow_dirty` here lets the callback
        // skip the shadow depth pass on idle or camera-only frames and reuse
        // the previous depth texture, the biggest frame-time win when nothing
        // has changed.
        let run_shadow = app.shadow_dirty;
        app.shadow_dirty = false;
        let callback = egui_wgpu::Callback::new_paint_callback(
            rect,
            TerrainPaintCallback {
                show_grid: app.show_grid,
                show_border: app.show_border,
                show_heatmap: app.show_heatmap,
                show_viewshed: !app.viewshed.is_idle(),
                camera: pick_camera.clone().unwrap_or_else(|| Camera::for_map(1, 1)),
                brush_highlight,
                brush_highlight_extra,
                run_shadow,
            },
        );
        ui.painter().add(egui::Shape::Callback(callback));
    } else {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(40, 44, 52));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "wgpu not available",
            egui::FontId::proportional(16.0),
            egui::Color32::WHITE,
        );
    }

    let rmb_held = ui
        .ctx()
        .input(|i| i.pointer.button_down(egui::PointerButton::Secondary));

    // Ctrl+Click samples properties from the map instead of applying the tool.
    // Computed up front to avoid borrow conflicts with `doc` below.
    let ctrl_held = response.ctx.input(|i| i.modifiers.command);
    let picker_handled = !rmb_held
        && ctrl_held
        && response.clicked_by(egui::PointerButton::Primary)
        && pick_camera.as_ref().is_some_and(|camera| {
            response
                .hover_pos()
                .is_some_and(|hp| ctrl_pick::handle_ctrl_click_pick(app, hp, rect, camera))
        });

    // Flush in-flight strokes from tools the user switched away from so
    // accumulated already-applied edits land as a single undo step.
    crate::ui::tool_dispatch::flush_inactive_tool_strokes(app);

    if app.tool_state.active_tool != ToolId::ObjectSelect && !app.selection.is_empty() {
        app.selection.clear();
        app.objects_dirty = true;
    }

    if let (Some(camera), Some(_doc)) = (&pick_camera, &app.document) {
        if !rmb_held && !picker_handled {
            crate::ui::tool_dispatch::dispatch_pointer_to_active_tool(app, &response, rect, camera);
        } else if rmb_held {
            app.hovered_tile = None;
        }
    }

    overlays::draw_all(ui, app, pick_camera.as_ref(), rect);
}
