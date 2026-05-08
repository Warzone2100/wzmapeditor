//! Settings/Preferences window with IntelliJ-style sidebar navigation.

use egui::{Color32, RichText, Ui};
use wz_maplib::validate::{ValidationCategory, WarningRule};

use crate::app::EditorApp;
use crate::config::{GraphicsBackend, PresentMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsPage {
    #[default]
    Viewport,
    Rendering,
    Problems,
    AutoSave,
    Keybindings,
    About,
}

impl SettingsPage {
    pub const ALL: [Self; 6] = [
        Self::Viewport,
        Self::Rendering,
        Self::Problems,
        Self::AutoSave,
        Self::Keybindings,
        Self::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Viewport => "Viewport",
            Self::Rendering => "Rendering",
            Self::Problems => "Problems",
            Self::AutoSave => "Auto-Save",
            Self::Keybindings => "Keybindings",
            Self::About => "About",
        }
    }
}

pub fn show_settings_window(ctx: &egui::Context, app: &mut EditorApp) {
    let mut open = app.settings_open;

    let screen = ctx.content_rect();
    let default_pos = egui::pos2(
        (screen.width() - 560.0) * 0.5,
        (screen.height() - 480.0) * 0.5,
    );

    egui::Window::new("Settings")
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .default_size([560.0, 480.0])
        .min_size([400.0, 300.0])
        .default_pos(default_pos)
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(120.0);
                    ui.set_max_width(120.0);
                    for page in SettingsPage::ALL {
                        let selected = app.settings_page == page;
                        if ui.selectable_label(selected, page.label()).clicked() {
                            app.settings_page = page;
                        }
                    }
                });

                ui.separator();

                ui.vertical(|ui| match app.settings_page {
                    SettingsPage::Viewport => show_viewport_settings(ui, app),
                    SettingsPage::Rendering => show_rendering_settings(ui, app),
                    SettingsPage::Problems => show_problems_settings(ui, app),
                    SettingsPage::AutoSave => show_autosave_settings(ui, app),
                    SettingsPage::Keybindings => show_keybindings_settings(ui, ctx, app),
                    SettingsPage::About => show_about_settings(ui, ctx, app),
                });
            });
        });

    app.settings_open = open;
}

fn show_viewport_settings(ui: &mut Ui, app: &mut EditorApp) {
    ui.heading("Viewport");
    ui.label(RichText::new("Configure viewport overlays and display options.").weak());
    ui.add_space(8.0);

    ui.checkbox(
        &mut app.show_selection_hitboxes,
        "Show hitboxes on selected objects",
    );
    if ui
        .checkbox(
            &mut app.viewshed.show_range_on_select,
            "Show range on selected towers",
        )
        .changed()
    {
        app.viewshed_dirty = true;
        app.viewshed.last_selection_sig = 0;
    }
}

fn show_rendering_settings(ui: &mut Ui, app: &mut EditorApp) {
    ui.heading("Rendering");
    ui.add_space(8.0);

    ui.checkbox(&mut app.render_settings.sky_enabled, "Sky");
    ui.checkbox(&mut app.render_settings.fog_enabled, "Fog");
    if app.render_settings.fog_enabled {
        ui.add(
            egui::Slider::new(&mut app.render_settings.fog_start, 500.0..=20000.0)
                .text("Fog Start"),
        );
        ui.add(
            egui::Slider::new(&mut app.render_settings.fog_end, 1000.0..=30000.0).text("Fog End"),
        );
    }
    ui.checkbox(&mut app.render_settings.shadows_enabled, "Shadows");
    ui.checkbox(&mut app.render_settings.water_enabled, "Water");

    ui.separator();
    ui.label("Sun Direction");
    ui.add(egui::Slider::new(&mut app.render_settings.sun_direction[0], -1.0..=1.0).text("X"));
    ui.add(egui::Slider::new(&mut app.render_settings.sun_direction[1], 0.0..=1.0).text("Y (up)"));
    ui.add(egui::Slider::new(&mut app.render_settings.sun_direction[2], -1.0..=1.0).text("Z"));

    ui.separator();
    ui.add(
        egui::Slider::new(&mut app.render_settings.fov_degrees, 20.0..=120.0)
            .text("FOV")
            .suffix("\u{00b0}"),
    );

    let backends = GraphicsBackend::available_for_platform();
    if backends.len() > 1 {
        ui.separator();
        ui.label(RichText::new("Graphics Backend").strong());
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Backend:");
            let current = app.config.graphics_backend;
            egui::ComboBox::from_id_salt("graphics_backend_combo")
                .selected_text(current.label())
                .show_ui(ui, |ui| {
                    for backend in backends.iter().copied() {
                        if ui
                            .selectable_label(current == backend, backend.label())
                            .clicked()
                            && app.config.graphics_backend != backend
                        {
                            app.config.graphics_backend = backend;
                            changed = true;
                        }
                    }
                });
        });

        if changed {
            app.config.save();
        }

        if app.config.graphics_backend != app.launched_graphics_backend {
            restart_required_row(
                ui,
                &format!(
                    "\u{26a0} Restart required. Currently running on {}.",
                    app.launched_graphics_backend.label()
                ),
            );
        }
    }

    ui.separator();
    let mut vsync_on = app.config.present_mode.is_vsynced();
    if ui
        .checkbox(&mut vsync_on, "Vsync")
        .on_hover_text(
            "Off: lowest input latency, may tear. \
             On: cap frame rate to monitor refresh (AutoVsync / Fifo). \
             Takes effect after restart.",
        )
        .changed()
    {
        app.config.present_mode = if vsync_on {
            PresentMode::SmartVsync
        } else {
            PresentMode::AutoNoVsync
        };
        app.config.save();
    }

    if app.config.present_mode != app.launched_present_mode {
        restart_required_row(
            ui,
            &format!(
                "\u{26a0} Restart required. Currently running with vsync {}.",
                if app.launched_present_mode.is_vsynced() {
                    "on"
                } else {
                    "off"
                }
            ),
        );
    }

    ui.add_space(4.0);
    let mut limit_on = app.config.fps_limit.is_some();
    let mut fps_value = app.config.fps_limit.unwrap_or(60);
    let mut changed = ui
        .checkbox(&mut limit_on, "Limit FPS")
        .on_hover_text(
            "Cap the editor's frame rate independently of vsync. Sleeps \
             at the end of each frame instead of blocking the swapchain, \
             so input is sampled fresh each capped frame.",
        )
        .changed();
    if limit_on {
        let slider = egui::Slider::new(&mut fps_value, 15..=240)
            .suffix(" fps")
            .clamping(egui::SliderClamping::Always);
        if ui.add(slider).changed() {
            changed = true;
        }
    }
    if changed {
        app.config.fps_limit = if limit_on { Some(fps_value) } else { None };
        app.config.save();
    }
}

/// "Restart required" line with an in-place re-exec button. egui-wgpu bakes
/// `present_mode` and the backend into the surface at startup, so a runtime
/// toggle takes effect only after restart.
fn restart_required_row(ui: &mut Ui, message: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.colored_label(Color32::YELLOW, message);
        if ui.button("Restart now").clicked() {
            relaunch_editor(ui.ctx());
        }
    });
}

/// Spawn a fresh copy of the editor with the same args, then close the
/// current viewport via `ViewportCommand::Close` so `EditorApp::on_exit`
/// still flushes view flags, dock layout, and any dirty autosave.
fn relaunch_editor(ctx: &egui::Context) {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            log::error!("Cannot find current_exe for restart: {e}");
            return;
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    log::info!("Relaunching {} {:?}", exe.display(), args);
    match std::process::Command::new(&exe).args(&args).spawn() {
        Ok(_child) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        Err(e) => log::error!("Failed to relaunch editor: {e}"),
    }
}

fn show_problems_settings(ui: &mut Ui, app: &mut EditorApp) {
    ui.heading("Problems");
    ui.label(
        RichText::new(
            "Configure which validation warnings are reported. Errors cannot be disabled.",
        )
        .weak(),
    );
    ui.add_space(8.0);

    let mut changed = false;

    ui.horizontal(|ui| {
        if ui.button("Enable All").clicked() {
            app.config.validation_config.disabled.clear();
            changed = true;
        }
        if ui.button("Disable All").clicked() {
            for rule in WarningRule::ALL {
                app.config.validation_config.disabled.insert(rule);
            }
            changed = true;
        }
    });

    ui.add_space(4.0);
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .show(ui, |ui| {
            let categories = [
                ValidationCategory::Map,
                ValidationCategory::Terrain,
                ValidationCategory::ObjectPositions,
                ValidationCategory::ObjectData,
                ValidationCategory::Multiplayer,
                ValidationCategory::Gateways,
                ValidationCategory::Labels,
            ];

            for cat in categories {
                let rules: Vec<WarningRule> = WarningRule::ALL
                    .iter()
                    .copied()
                    .filter(|r| r.category() == cat)
                    .collect();
                if rules.is_empty() {
                    continue;
                }

                ui.add_space(4.0);
                ui.label(RichText::new(cat.label()).strong().color(Color32::WHITE));
                ui.indent(cat.label(), |ui| {
                    for rule in rules {
                        let mut enabled = app.config.validation_config.is_enabled(rule);
                        if ui.checkbox(&mut enabled, rule.label()).changed() {
                            if enabled {
                                app.config.validation_config.disabled.remove(&rule);
                            } else {
                                app.config.validation_config.disabled.insert(rule);
                            }
                            changed = true;
                        }
                    }
                });
            }
        });

    if changed {
        app.config.save();
        app.validation_dirty = true;
    }
}

fn show_autosave_settings(ui: &mut Ui, app: &mut EditorApp) {
    ui.heading("Auto-Save");
    ui.label(
        RichText::new(
            "Periodically saves a temporary copy of your map for crash recovery. \
             Temporary files are cleaned up after a manual save.",
        )
        .weak(),
    );
    ui.add_space(8.0);

    let mut changed = false;

    if ui
        .checkbox(&mut app.config.autosave_enabled, "Enable auto-save")
        .changed()
    {
        changed = true;
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Interval:");
        let mut secs = app.config.autosave_interval_secs;
        let slider = egui::Slider::new(&mut secs, 30..=600)
            .suffix(" sec")
            .step_by(10.0)
            .clamping(egui::SliderClamping::Always);
        if ui
            .add_enabled(app.config.autosave_enabled, slider)
            .changed()
        {
            app.config.autosave_interval_secs = secs;
            changed = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Presets:");
        for (label, secs) in [
            ("1 min", 60),
            ("2 min", 120),
            ("5 min", 300),
            ("10 min", 600),
        ] {
            let is_current = app.config.autosave_interval_secs == secs;
            if ui
                .add_enabled(
                    app.config.autosave_enabled && !is_current,
                    egui::Button::new(label),
                )
                .clicked()
            {
                app.config.autosave_interval_secs = secs;
                changed = true;
            }
        }
    });

    if changed {
        app.config.save();
    }
}

fn show_keybindings_settings(ui: &mut Ui, ctx: &egui::Context, app: &mut EditorApp) {
    use crate::keybindings::Action;

    ui.heading("Keybindings");
    ui.label(
        RichText::new(
            "Click a binding to rebind it. Press a key (with optional modifiers) to assign. \
             Press Escape to clear a binding.",
        )
        .weak(),
    );
    ui.add_space(8.0);

    let mut keymap_changed = false;
    ui.horizontal(|ui| {
        if ui.button("Reset All to Defaults").clicked() {
            app.config.keymap = crate::keybindings::Keymap::default_keymap();
            app.keybinding_capture = None;
            keymap_changed = true;
        }
    });

    ui.add_space(4.0);

    ui.separator();

    if let Some(capturing_action) = app.keybinding_capture {
        let captured = ctx.input_mut(|input| {
            for &key in egui::Key::ALL {
                let mods = input.modifiers;
                if input.consume_key(mods, key) {
                    if key == egui::Key::Escape && mods == egui::Modifiers::NONE {
                        // Bare Escape clears the binding.
                        return Some(None);
                    }
                    return Some(Some(crate::keybindings::KeyCombo {
                        key,
                        ctrl: mods.command,
                        shift: mods.shift,
                        alt: mods.alt,
                    }));
                }
            }
            None
        });

        if let Some(result) = captured {
            match result {
                Some(combo) => {
                    app.config.keymap.rebind(capturing_action, vec![combo]);
                }
                None => {
                    app.config.keymap.rebind(capturing_action, vec![]);
                }
            }
            app.keybinding_capture = None;
            keymap_changed = true;
        }
    }

    let conflicts = app.config.keymap.conflicts();
    if !conflicts.is_empty() {
        ui.colored_label(
            Color32::YELLOW,
            format!("\u{26a0} {} conflict(s) detected", conflicts.len()),
        );
        ui.add_space(4.0);
    }

    if keymap_changed {
        app.config.save();
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("keybindings_grid")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    for &action in Action::ALL {
                        ui.label(action.display_name());

                        let is_capturing = app.keybinding_capture == Some(action);
                        let button_text = if is_capturing {
                            RichText::new("Press a key (+ modifiers)...")
                                .italics()
                                .color(Color32::YELLOW)
                        } else {
                            let text = app.config.keymap.shortcut_text(action);
                            if text.is_empty() {
                                RichText::new("(unbound)").weak()
                            } else {
                                RichText::new(text)
                            }
                        };

                        let btn = egui::Button::new(button_text).min_size(egui::vec2(120.0, 0.0));
                        if ui.add(btn).clicked() {
                            app.keybinding_capture = if is_capturing { None } else { Some(action) };
                        }

                        ui.end_row();
                    }
                });
        });
}

fn show_about_settings(ui: &mut Ui, ctx: &egui::Context, app: &mut EditorApp) {
    if !app.editor_icon_tried {
        app.editor_icon = crate::icon::for_egui(ctx, 256);
        app.editor_icon_tried = true;
    }

    ui.vertical_centered(|ui| {
        ui.add_space(20.0);

        if let Some(handle) = app.editor_icon.as_ref() {
            ui.image((handle.id(), egui::vec2(128.0, 128.0)));
        }

        ui.add_space(10.0);
        ui.heading("wzmapeditor");
        ui.label(RichText::new(format!("version {}", env!("CARGO_PKG_VERSION"))).weak());

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(14.0);
    });

    // ui.vertical_centered doesn't center multi-item rows (egui packs them
    // left-to-right, ignoring the row block's alignment). Measure the text
    // width via the font cache and pad manually.
    let body_id = egui::TextStyle::Body.resolve(ui.style());
    let small_id = egui::TextStyle::Small.resolve(ui.style());
    let item_spacing = ui.spacing().item_spacing.x;
    let (link_row_w, credit_row_w) = ui.ctx().fonts_mut(|f| {
        let mut w = |text: &str, font: &egui::FontId| -> f32 {
            f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
                .rect
                .width()
        };
        let bullet = w("\u{2022}", &body_id);
        let link = w("Homepage", &body_id)
            + w("Report an Issue", &body_id)
            + w(env!("CARGO_PKG_LICENSE"), &body_id)
            + bullet * 2.0
            + item_spacing * 4.0;
        let credit = w("Created by", &small_id) + w("phetrommer", &small_id) + item_spacing;
        (link, credit)
    });

    ui.horizontal(|ui| {
        let pad = ((ui.available_width() - link_row_w) * 0.5).max(0.0);
        ui.add_space(pad);
        ui.hyperlink_to("Homepage", env!("CARGO_PKG_HOMEPAGE"));
        ui.label(RichText::new("\u{2022}").weak());
        ui.hyperlink_to(
            "Report an Issue",
            concat!(env!("CARGO_PKG_HOMEPAGE"), "/issues"),
        );
        ui.label(RichText::new("\u{2022}").weak());
        ui.hyperlink_to(
            env!("CARGO_PKG_LICENSE"),
            concat!(env!("CARGO_PKG_HOMEPAGE"), "/blob/main/LICENSE"),
        );
    });

    ui.add_space(20.0);

    ui.horizontal(|ui| {
        let pad = ((ui.available_width() - credit_row_w) * 0.5).max(0.0);
        ui.add_space(pad);
        ui.label(RichText::new("Created by").weak().small());
        ui.hyperlink_to(
            RichText::new("phetrommer").small(),
            "https://github.com/phetrommer",
        );
    });

    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new("Built with Rust \u{2022} egui \u{2022} wgpu")
                .weak()
                .small(),
        );
    });
}
