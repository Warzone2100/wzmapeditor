//! Terrain tools tab content for the bottom dock.
//!
//! Layout: grouped tool buttons (Sculpt / Paint / Objects) at the top, with
//! the active tool's settings rendered inline directly below so picking and
//! tuning a tool happen in the same panel.

use egui::{RichText, Ui};

use crate::app::EditorApp;
use crate::keybindings::{Action, Keymap};
use crate::tools::{ToolId, ToolState};

pub fn show_tool_palette(ui: &mut Ui, app: &mut EditorApp) {
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            show_tool_buttons(ui, &mut app.tool_state, &app.config.keymap);
            ui.separator();
            show_tool_settings(ui, app);
        });
}

fn show_tool_buttons(ui: &mut Ui, tool_state: &mut ToolState, keymap: &Keymap) {
    const SCULPT: &[(ToolId, &str, &str)] = &[
        (
            ToolId::HeightBrush,
            "Sculpt",
            "Height brush: pick Raise / Lower / Smooth / Set in the settings below (1/2/3/4)",
        ),
        (
            ToolId::VertexSculpt,
            "Vertex",
            "Soft-select vertex sculpt: click to select, drag to raise/lower, Ctrl+drag to box-select",
        ),
    ];
    const PAINT: &[(ToolId, &str, &str)] = &[(
        ToolId::TexturePaint,
        "Texture",
        "Paint terrain tiles. Single mode picks one tile, Pool mode samples a weighted group.",
    )];
    // ObjectPlace has no palette entry: clicking an asset in the Asset
    // browser switches to it automatically, and Ctrl+click in the
    // viewport eyedrops an existing object into placement. A dedicated
    // button leads to a dead-end "pick something in the asset browser"
    // empty state.
    const OBJECTS: &[(ToolId, &str, &str)] = &[
        (
            ToolId::WallPlacement,
            "Wall",
            "Drag to place walls; corners snap automatically",
        ),
        (
            ToolId::Stamp,
            "Stamp",
            "Capture and stamp tile/object patterns",
        ),
    ];

    show_tool_group(ui, "Sculpt", SCULPT, tool_state, keymap);
    show_tool_group(ui, "Paint", PAINT, tool_state, keymap);
    show_tool_group(ui, "Objects", OBJECTS, tool_state, keymap);
}

fn show_tool_group(
    ui: &mut Ui,
    title: &str,
    tools: &[(ToolId, &str, &str)],
    tool_state: &mut ToolState,
    keymap: &Keymap,
) {
    ui.label(RichText::new(title).small().weak());
    ui.horizontal_wrapped(|ui| {
        for (tool, label, base_tooltip) in tools {
            // The Texture button stands in for both paint tools, so it
            // counts as selected whenever either is active and clicking
            // it activates the one matching the current Single/Pool mode.
            let selected = if *tool == ToolId::TexturePaint {
                matches!(
                    tool_state.active_tool,
                    ToolId::TexturePaint | ToolId::GroundTypePaint
                )
            } else {
                tool_state.active_tool == *tool
            };
            let btn = egui::Button::new(*label)
                .min_size(egui::vec2(52.0, 32.0))
                .selected(selected);

            let hotkey = keymap.shortcut_text(Action::from_tool(*tool));
            let tooltip = if hotkey.is_empty() {
                (*base_tooltip).to_string()
            } else {
                format!("{base_tooltip} ({hotkey})")
            };

            if ui.add(btn).on_hover_text(tooltip).clicked() {
                tool_state.active_tool =
                    if *tool == ToolId::TexturePaint && tool_state.ground_type_mode {
                        ToolId::GroundTypePaint
                    } else {
                        *tool
                    };
            }
        }
    });
}

fn show_tool_settings(ui: &mut Ui, app: &mut EditorApp) {
    if app.document.is_none() {
        ui.label("No map loaded.");
        return;
    }

    show_texture_mode_toggle(ui, &mut app.tool_state);
    crate::ui::tool_dispatch::render_active_tool_properties(ui, app);

    show_mirror_controls(ui, app);
}

fn show_texture_mode_toggle(ui: &mut Ui, tool_state: &mut ToolState) {
    if !matches!(
        tool_state.active_tool,
        ToolId::TexturePaint | ToolId::GroundTypePaint
    ) {
        return;
    }
    let pool_mode = tool_state.ground_type_mode;
    ui.horizontal(|ui| {
        let single = egui::Button::new("Single").selected(!pool_mode);
        if ui
            .add(single)
            .on_hover_text("Paint with one selected tile.")
            .clicked()
        {
            tool_state.ground_type_mode = false;
            tool_state.active_tool = ToolId::TexturePaint;
        }
        let pool = egui::Button::new("Pool").selected(pool_mode);
        if ui
            .add(pool)
            .on_hover_text("Paint a random tile from the selected ground type pool.")
            .clicked()
        {
            tool_state.ground_type_mode = true;
            tool_state.active_tool = ToolId::GroundTypePaint;
        }
    });
    ui.add_space(2.0);
}

fn show_mirror_controls(ui: &mut Ui, app: &mut EditorApp) {
    if !app.tool_state.active_tool.uses_mirror() {
        return;
    }

    let (map_w, map_h) = app.document.as_ref().map_or((0, 0), |doc| {
        (doc.map.map_data.width, doc.map.map_data.height)
    });

    ui.separator();
    ui.label("Mirror");
    ui.horizontal(|ui| {
        use crate::tools::MirrorMode;
        let modes = [
            (MirrorMode::None, "Off", "No mirroring"),
            (MirrorMode::Vertical, "|", "Vertical axis (left/right)"),
            (
                MirrorMode::Horizontal,
                "\u{2014}",
                "Horizontal axis (top/bottom)",
            ),
            (MirrorMode::Both, "+", "Both axes (4-way)"),
            (MirrorMode::Central, "O", "Central (180° rotation)"),
            (
                MirrorMode::Diagonal,
                "X",
                "Diagonal axes (4-way, square maps only)",
            ),
        ];
        for (mode, label, tooltip) in modes {
            let valid = crate::tools::mirror::is_mirror_valid(map_w, map_h, mode);
            let selected = app.tool_state.mirror_mode == mode;
            let btn = egui::Button::new(label).selected(selected);
            let resp = ui.add_enabled(valid, btn);
            if resp.clicked() {
                app.tool_state.mirror_mode = mode;
            }
            if valid {
                resp.on_hover_text(tooltip);
            } else {
                resp.on_disabled_hover_text("X mode requires a square map");
            }
        }
    });
}
