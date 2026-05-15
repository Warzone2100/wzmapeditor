//! Top toolbar row - quick access to frequent File/Edit actions, Test Map and Settings.

use egui::Ui;

use crate::app::EditorApp;
use crate::keybindings::Action;
use crate::ui::actions;

pub fn show_toolbar(ui: &mut Ui, app: &mut EditorApp) {
    ui.horizontal(|ui| {
        // Flatter, roomier buttons. The default egui button has a hard
        // border on every state which reads as cramped in a dense toolbar.
        // Drop the idle border, keep hover/active fills so the widget still
        // feels like a button when the cursor is over it.
        ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        {
            let v = ui.visuals_mut();
            v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
            v.widgets.active.bg_stroke = egui::Stroke::NONE;
        }

        let has_doc = app.document.is_some();
        let save_shortcut = app.config.keymap.shortcut_text(Action::Save).to_string();

        if ui.button("New").on_hover_text("New Map").clicked() {
            app.new_map_dialog.open = true;
        }
        if ui
            .button("Open\u{2026}")
            .on_hover_text("Open .wz")
            .clicked()
        {
            actions::open_wz_dialog(app);
        }
        if ui
            .add_enabled(has_doc, egui::Button::new("Save"))
            .on_hover_text(format!("Save ({save_shortcut})"))
            .clicked()
        {
            actions::save_current_or_prompt(app);
        }
        if ui
            .add_enabled(has_doc, egui::Button::new("Undo"))
            .on_hover_text("Undo")
            .clicked()
        {
            actions::undo(app);
        }
        if ui
            .add_enabled(has_doc, egui::Button::new("Redo"))
            .on_hover_text("Redo")
            .clicked()
        {
            actions::redo(app);
        }

        ui.separator();

        if ui
            .button("Map Browser")
            .on_hover_text("Open the map browser")
            .clicked()
        {
            actions::browse_maps(app, ui.ctx());
        }

        ui.separator();

        if ui.button("Settings").on_hover_text("Settings").clicked() {
            app.settings_open = true;
        }

        ui.separator();

        let can_test = app.can_test_map();
        let label = if app.test_process.is_some() {
            "Game Running\u{2026}"
        } else {
            "Test Map"
        };
        let tooltip = app.test_map_tooltip();
        if ui
            .add_enabled(can_test, egui::Button::new(label))
            .on_hover_text(tooltip)
            .on_disabled_hover_text(tooltip)
            .clicked()
        {
            app.test_map();
        }

        show_update_button(ui, app);
    });
}

fn show_update_button(ui: &mut Ui, app: &mut EditorApp) {
    let Some(info) = app.update_available.clone() else {
        return;
    };
    ui.separator();
    let btn = egui::Button::new(
        egui::RichText::new(format!("Update available: {}", info.latest))
            .color(egui::Color32::BLACK)
            .strong(),
    )
    .fill(egui::Color32::from_rgb(255, 196, 0));
    let resp = ui
        .add(btn)
        .on_hover_text("Open the release page in your browser");
    if resp.clicked()
        && let Err(e) = open::that(&info.html_url)
    {
        log::warn!("Failed to open release URL {}: {e}", info.html_url);
    }
    resp.context_menu(|ui| {
        if ui.button("Don't show for this version").clicked() {
            app.config.dismissed_update_version = Some(info.latest.clone());
            app.config.save();
            app.update_available = None;
            ui.close();
        }
    });
}
