//! Top menu bar (File, Edit, View, Map, Settings).

use egui::Ui;

use crate::app::EditorApp;
use crate::ui::actions;

pub fn show_menu_bar(ui: &mut Ui, app: &mut EditorApp) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            if ui.button("New Map...").clicked() {
                app.new_map_dialog.open = true;
                ui.close();
            }
            if ui.button("Open .wz...").clicked() {
                actions::open_wz_dialog(app);
                ui.close();
            }
            if ui.button("Browse Maps...").clicked() {
                actions::browse_maps(app, ui.ctx());
                ui.close();
            }
            ui.separator();
            ui.menu_button("Import...", |ui| {
                if ui.button(".wz Archive...").clicked() {
                    actions::import_wz(app);
                    ui.close();
                }
                ui.separator();
                ui.menu_button("Legacy", |ui| {
                    if ui.button("Map Folder...").clicked() {
                        actions::import_map_folder(app);
                        ui.close();
                    }
                    if ui.button("Binary game.map...").clicked() {
                        actions::import_game_map(app);
                        ui.close();
                    }
                });
            });
            ui.separator();
            let has_doc = app.document.is_some();
            if ui
                .add_enabled(
                    has_doc,
                    egui::Button::new("Save").shortcut_text(
                        app.config
                            .keymap
                            .shortcut_text(crate::keybindings::Action::Save),
                    ),
                )
                .clicked()
            {
                actions::save_current_or_prompt(app);
                ui.close();
            }
            if has_doc {
                ui.menu_button("Save As...", |ui| {
                    if ui.button(".wz Archive...").clicked() {
                        actions::save_as_wz(app);
                        ui.close();
                    }
                    if ui.button("Preview PNG...").clicked() {
                        actions::save_as_preview_png(app);
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("Legacy", |ui| {
                        if ui.button("Map Folder...").clicked() {
                            actions::save_as_directory(app);
                            ui.close();
                        }
                        if ui.button("Binary game.map...").clicked() {
                            actions::save_as_game_map(app);
                            ui.close();
                        }
                    });
                });
            } else {
                ui.add_enabled(false, egui::Button::new("Save As..."));
            }
            ui.separator();
            let can_publish = app.save_path.is_some();
            if ui
                .add_enabled(
                    can_publish,
                    egui::Button::new("Publish to Maps Database\u{2026}"),
                )
                .on_disabled_hover_text("Save the map to a .wz file first")
                .clicked()
            {
                actions::publish_to_maps_db(app);
                ui.close();
            }
            ui.separator();
            if ui.button("Open Config Folder").clicked() {
                actions::open_config_dir(app);
                ui.close();
            }
            ui.separator();
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });

        ui.menu_button("Edit", |ui| {
            if ui.button("Undo").clicked() {
                actions::undo(app);
                ui.close();
            }
            if ui.button("Redo").clicked() {
                actions::redo(app);
                ui.close();
            }
            ui.separator();
            let has_selection = !app.selection.is_empty();
            let dup_text = app
                .config
                .keymap
                .shortcut_text(crate::keybindings::Action::Duplicate)
                .to_string();
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new("Duplicate").shortcut_text(dup_text),
                )
                .clicked()
            {
                app.duplicate_selection();
                ui.close();
            }
            ui.separator();
            ui.add_enabled(false, egui::Button::new("Droid Designer..."))
                .on_disabled_hover_text("Droid Designer is temporarily disabled");
        });

        ui.menu_button("View", |ui| {
            ui.checkbox(&mut app.show_grid, "Show Grid");
            ui.checkbox(&mut app.show_border, "Show Border");
            ui.checkbox(&mut app.show_labels, "Show Labels");
            ui.checkbox(&mut app.show_gateways, "Show Gateways");
            ui.checkbox(&mut app.show_all_hitboxes, "Show All Hitboxes");
            ui.checkbox(&mut app.render_settings.sky_enabled, "Sky");
            ui.checkbox(&mut app.render_settings.fog_enabled, "Fog");
            ui.checkbox(&mut app.render_settings.shadows_enabled, "Shadows");
            ui.checkbox(&mut app.render_settings.water_enabled, "Water");
            ui.checkbox(&mut app.show_fps, "Show FPS");
            ui.menu_button("Weather", |ui| {
                for w in wz_maplib::Weather::ALL {
                    if ui
                        .radio_value(&mut app.view_weather, w, w.label())
                        .clicked()
                    {
                        ui.close();
                    }
                }
            });
            ui.separator();
            ui.label("Terrain Quality");
            let current = app.render_settings.terrain_quality;
            for mode in crate::viewport::renderer::TerrainQuality::ALL {
                use crate::viewport::renderer::TerrainQuality;
                let disabled_reason = match mode {
                    TerrainQuality::Medium => Some("Medium quality is not yet supported"),
                    TerrainQuality::High if !app.has_hq_textures => {
                        Some("Install high.wz (terrain_overrides) for Remastered (HQ) textures")
                    }
                    _ => None,
                };
                let resp = ui.add_enabled(
                    disabled_reason.is_none(),
                    egui::RadioButton::new(current == mode, mode.label()),
                );
                let resp = if let Some(reason) = disabled_reason {
                    resp.on_disabled_hover_text(reason)
                } else {
                    resp
                };
                if resp.clicked() {
                    app.render_settings.terrain_quality = mode;
                    app.terrain_dirty = true;
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Reset Layout").clicked() {
                app.dock = crate::app::default_dock_layout();
                ui.close();
            }
        });

        ui.menu_button("Map", |ui| {
            let has_doc = app.document.is_some();
            if ui
                .add_enabled(has_doc, egui::Button::new("Resize..."))
                .on_disabled_hover_text("Open or create a map first")
                .clicked()
            {
                if let Some(ref doc) = app.document {
                    let (w, h) = (doc.map.map_data.width, doc.map.map_data.height);
                    app.resize_map_dialog.source_size = (w, h);
                    app.resize_map_dialog.new_width = w;
                    app.resize_map_dialog.new_height = h;
                    app.resize_map_dialog.anchor = crate::app::ResizeAnchor::MiddleCenter;
                    app.resize_map_dialog.open = true;
                }
                ui.close();
            }
            if ui.button("Generate...").clicked() {
                app.generator_dialog.open = true;
                ui.close();
            }
            ui.separator();
            let can_test = app.can_test_map();
            if ui
                .add_enabled(
                    can_test,
                    egui::Button::new("Test Map").shortcut_text(
                        app.config
                            .keymap
                            .shortcut_text(crate::keybindings::Action::TestMap),
                    ),
                )
                .on_disabled_hover_text(app.test_map_tooltip())
                .clicked()
            {
                app.test_map();
                ui.close();
            }
        });
    });
}
