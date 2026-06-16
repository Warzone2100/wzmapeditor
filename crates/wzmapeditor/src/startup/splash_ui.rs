//! Launcher UI: setup card before a data dir is picked, task list during loading.
//!
//! Both phases share the same centered card chrome so first launch feels
//! like a single coherent launcher; the editor stays hidden from frame
//! one until critical assets finish loading.

use crate::app::{EditorApp, StartupPhase};

const SPLASH_WIDTH: f32 = 360.0;

/// Icon column width, kept consistent for task-list alignment.
const SPLASH_ICON_WIDTH: f32 = 20.0;

const SPLASH_ICON_DISPLAY_PX: f32 = 80.0;

/// Render the launcher card. No-op when phase is `Ready`.
pub fn show_launcher(ui: &mut egui::Ui, app: &mut EditorApp) {
    if matches!(
        app.startup_phase,
        StartupPhase::Setup { .. } | StartupPhase::Loading { .. }
    ) && !app.editor_icon_tried
    {
        app.editor_icon = crate::icon::for_egui(ui.ctx(), 256);
        app.editor_icon_tried = true;
    }

    match &app.startup_phase {
        StartupPhase::Setup { .. } => show_setup_card(ui, app),
        StartupPhase::Loading { .. } => show_loading_card(ui, app),
        StartupPhase::Ready => {}
    }
}

/// Render the fullscreen background and centered card with the shared
/// title block. The body closure draws everything below the separator.
fn splash_card(
    ui: &mut egui::Ui,
    icon: Option<&egui::TextureHandle>,
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::CentralPanel::default().show_inside(ui, |_| {});

    egui::Window::new("splash")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([SPLASH_WIDTH, 0.0])
        .show(ui.ctx(), |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                if let Some(handle) = icon {
                    ui.image((
                        handle.id(),
                        egui::vec2(SPLASH_ICON_DISPLAY_PX, SPLASH_ICON_DISPLAY_PX),
                    ));
                    ui.add_space(8.0);
                }
                ui.label(egui::RichText::new("wzmapeditor").size(28.0).strong());
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Warzone 2100 Map Editor")
                        .size(12.0)
                        .weak(),
                );
                ui.add_space(20.0);
            });

            ui.separator();
            ui.add_space(8.0);

            body(ui);

            ui.add_space(12.0);
        });
}

/// First-run welcome card. Browse opens a folder picker; valid picks
/// transition to `Loading`, invalid picks stash an error on Setup.
fn show_setup_card(ui: &mut egui::Ui, app: &mut EditorApp) {
    // Snapshot the error so the closure doesn't borrow `app`.
    let error_msg = match &app.startup_phase {
        StartupPhase::Setup { error, .. } => error.clone(),
        _ => return,
    };

    let mut browse_clicked = false;
    let ctx = ui.ctx().clone();

    splash_card(ui, app.editor_icon.as_ref(), |ui| {
        ui.label("Select the 'data' folder inside your Warzone 2100 install.");
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("It's the folder that contains base.wz.")
                .weak()
                .size(12.0),
        );
        ui.add_space(14.0);

        ui.vertical_centered(|ui| {
            if ui.button("Browse...").clicked() {
                browse_clicked = true;
            }
        });

        if let Some(msg) = error_msg.as_deref() {
            ui.add_space(10.0);
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), msg);
        }
    });

    if browse_clicked {
        handle_setup_browse(app, &ctx);
    }
}

/// Validate the user's directory pick and either transition into Loading
/// or stash an error on the Setup phase.
fn handle_setup_browse(app: &mut EditorApp, ctx: &egui::Context) {
    let Some(dir) = rfd::FileDialog::new()
        .set_title("Select WZ2100 Data Directory")
        .pick_folder()
    else {
        return;
    };

    handle_picked_data_dir(app, ctx, dir);
}

/// Validate a picked data directory and transition into Loading or stash an
/// error on the Setup phase.
#[cfg(not(target_arch = "wasm32"))]
fn handle_picked_data_dir(app: &mut EditorApp, ctx: &egui::Context, dir: std::path::PathBuf) {
    let has_base_dir =
        dir.join("base").join("stats").exists() || dir.join("base").join("texpages").exists();
    let base_wz = dir.join("base.wz");

    if has_base_dir {
        app.config.game_install_dir = Some(dir.clone());
        app.set_data_dir(dir, ctx);
        crate::startup::pipeline::transition_setup_to_loading(app, false);
    } else if base_wz.exists() {
        app.config.game_install_dir = Some(dir);
        app.config.save();
        app.start_base_wz_extraction(base_wz, ctx);
        crate::startup::pipeline::transition_setup_to_loading(app, true);
    } else {
        let msg = format!("No base.wz or base/ tree found in:\n{}", dir.display());
        app.log(msg.clone());
        if let StartupPhase::Setup { error, .. } = &mut app.startup_phase {
            *error = Some(msg);
        }
    }
}

fn show_loading_card(ui: &mut egui::Ui, app: &EditorApp) {
    let StartupPhase::Loading {
        ref map_done,
        ref tileset_done,
        ref stats_done,
        ref map_rx,
        ref tileset_rx,
        ref stats_rx,
        ref extracting,
        ref extraction_started,
        ref post_load_started,
    } = app.startup_phase
    else {
        return;
    };

    let ctx = ui.ctx().clone();
    splash_card(ui, app.editor_icon.as_ref(), |ui| {
        let extraction_done = !*extracting;
        if *extraction_started {
            if extraction_done {
                splash_task_done(ui, "Extracting game data");
            } else {
                splash_task_progress(ui, "Extracting game data...", app.rt.extraction_fraction());
            }
        }

        // Cascade checkmarks top-to-bottom for sequential visual progress.
        let map_show_done = *map_done && extraction_done;
        let tileset_show_done = *tileset_done && map_show_done;
        let stats_show_done = *stats_done && tileset_show_done;

        if map_rx.is_some() || *map_done {
            if map_show_done {
                splash_task_done(ui, "Loading map");
            } else if extraction_done {
                splash_task_progress(ui, "Loading map...", None);
            } else {
                splash_task_pending(ui, "Loading map");
            }
        }
        if tileset_rx.is_some() || *tileset_done {
            if tileset_show_done {
                splash_task_done(ui, "Loading tileset");
            } else if map_show_done {
                splash_task_progress(ui, "Loading tileset...", None);
            } else {
                splash_task_pending(ui, "Loading tileset");
            }
        }
        if stats_rx.is_some() || *stats_done {
            if stats_show_done {
                splash_task_done(ui, "Loading stats");
            } else if tileset_show_done {
                splash_task_progress(ui, "Loading stats...", None);
            } else {
                splash_task_pending(ui, "Loading stats");
            }
        }

        if *post_load_started {
            ui.add_space(4.0);

            let ground_precache_done =
                app.rt.ground_precache_rx.is_none() && app.rt.ground_precache_attempted;
            let ground_load_done = app.rt.ground_texture_load.is_none()
                && (app.ground_data.is_some() || ground_precache_done);
            let ground_all_done = ground_precache_done && ground_load_done;

            if ground_all_done {
                splash_task_done(ui, "Loaded ground textures");
            } else if app.rt.ground_precache_rx.is_some() {
                splash_task_progress(
                    ui,
                    "Decoding ground textures...",
                    app.rt.ground_precache_fraction(),
                );
            } else if let Some(ref state) = app.rt.ground_texture_load {
                let raw = state.progress.load(std::sync::atomic::Ordering::Relaxed);
                if raw > 1000 {
                    let frac = ((raw - 1000) as f32 / 1000.0).min(1.0);
                    splash_task_progress(ui, "Uploading ground textures...", Some(frac));
                } else {
                    let frac = raw as f32 / 1000.0;
                    splash_task_progress(ui, "Loading ground textures...", Some(frac));
                }
            } else if !app.rt.ground_precache_attempted {
                splash_task_pending(ui, "Loading ground textures");
            } else {
                splash_task_done(ui, "Caching ground textures");
            }

            let connectors_done = app.rt.connectors_done();
            if connectors_done && app.model_loader.is_some() {
                splash_task_done(ui, "Caching model connectors");
            } else if app.rt.connector_precache_rx.is_some() {
                splash_task_progress(ui, "Caching model connectors...", None);
            } else if app.stats.is_some() {
                splash_task_pending(ui, "Caching model connectors");
            }

            let models_done = app.rt.models_done();
            if models_done && app.document.is_some() {
                splash_task_done(ui, "Caching object models");
            } else if app.rt.map_model_load.is_some() {
                splash_task_progress(ui, "Caching object models...", app.rt.model_fraction());
            } else if app.document.is_some() && !connectors_done {
                splash_task_pending(ui, "Caching object models");
            }

            let above_thumbnails_done = models_done && connectors_done;
            show_thumbnail_tasks(ui, app, above_thumbnails_done);
        }
    });

    ctx.request_repaint();
}

fn show_thumbnail_tasks(ui: &mut egui::Ui, app: &EditorApp, above_thumbnails_done: bool) {
    let current_ts = &app.model_thumbnails.current_tileset;

    let mut display_order: Vec<String> = Vec::new();
    display_order.push(app.model_thumbnails.active_tileset.clone());
    for ts in app.model_thumbnails.pending_tilesets.iter().rev() {
        if !display_order.contains(ts) {
            display_order.push(ts.clone());
        }
    }
    if !display_order.contains(current_ts) {
        display_order.insert(1, current_ts.clone());
    }
    for &ts in &["arizona", "urban", "rockies"] {
        if !display_order.iter().any(|s| s == ts) {
            display_order.push(ts.to_string());
        }
    }

    let mut prev_ts_done = above_thumbnails_done;
    for ts_name in &display_order {
        let label = format!(
            "Caching {} unit & structure previews",
            capitalize_first(ts_name)
        );
        let is_current = ts_name == current_ts;

        let ts_actually_done = if is_current {
            matches!(
                app.model_thumbnails.preload,
                crate::thumbnails::PreloadState::Complete | crate::thumbnails::PreloadState::Done
            )
        } else {
            !app.model_thumbnails
                .pending_tilesets
                .iter()
                .any(|p| p == ts_name.as_str())
                && !is_current
        };

        let show_done = ts_actually_done && prev_ts_done;

        if show_done {
            splash_task_done(ui, &label);
        } else if !prev_ts_done {
            splash_task_pending(ui, &label);
        } else if is_current {
            match app.model_thumbnails.preload {
                crate::thumbnails::PreloadState::Rendering { done, total, .. } => {
                    let frac = done as f32 / total.max(1) as f32;
                    splash_task_progress(ui, &format!("{label}..."), Some(frac));
                }
                _ => {
                    splash_task_progress(ui, &format!("{label}..."), None);
                }
            }
        } else {
            splash_task_progress(ui, &format!("{label}..."), None);
        }

        prev_ts_done = show_done;
    }
}

fn splash_task_done(ui: &mut egui::Ui, label: &str) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(SPLASH_ICON_WIDTH, ui.spacing().interact_size.y),
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                ui.label(
                    egui::RichText::new("\u{2714}").color(egui::Color32::from_rgb(100, 200, 100)),
                );
            },
        );
        ui.label(egui::RichText::new(label).weak());
    });
}

pub(crate) fn splash_task_progress(ui: &mut egui::Ui, label: &str, frac: Option<f32>) {
    ui.horizontal(|ui| {
        ui.allocate_space(egui::vec2(SPLASH_ICON_WIDTH, ui.spacing().interact_size.y));
        ui.label(label);
    });
    ui.horizontal(|ui| {
        ui.add_space(SPLASH_ICON_WIDTH + ui.spacing().item_spacing.x);
        let bar = if let Some(f) = frac {
            egui::ProgressBar::new(f).show_percentage()
        } else {
            egui::ProgressBar::new(0.0)
        };
        ui.add(bar);
    });
}

fn splash_task_pending(ui: &mut egui::Ui, label: &str) {
    ui.horizontal(|ui| {
        ui.allocate_space(egui::vec2(SPLASH_ICON_WIDTH, ui.spacing().interact_size.y));
        ui.label(label);
    });
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}
