use std::fmt::Write as _;

use egui::{Align2, Color32, CornerRadius, FontId, Margin, Rect, RichText, Stroke, Ui};

use crate::app::{EditorApp, SelectedObject};
use crate::viewport::camera::Camera;

pub(super) fn draw_info_bar(ui: &mut Ui, app: &mut EditorApp, rect: Rect) {
    let Some(doc) = app.document.as_ref() else {
        return;
    };
    let map = &doc.map.map_data;
    let painter = ui.painter_at(rect);

    let mut info = format!(
        "Map: {}x{} | {} | {:?}",
        map.width, map.height, app.current_tileset, app.tool_state.active_tool
    );
    if let Some((tx, ty)) = app.hovered_tile {
        write!(info, " | Tile: ({tx}, {ty})").unwrap();
        if let Some(tile) = map.tile(tx, ty) {
            write!(info, " h={} tex={}", tile.height, tile.texture_id()).unwrap();
        }
    }
    if app.selection.len() > 1 {
        write!(info, " | Sel: {} objects", app.selection.len()).unwrap();
    } else if let Some(sel) = app.selection.single() {
        match sel {
            SelectedObject::Structure(i) => {
                if let Some(s) = doc.map.structures.get(i) {
                    write!(info, " | Sel: {} [P{}]", s.name, s.player).unwrap();
                }
            }
            SelectedObject::Droid(i) => {
                if let Some(d) = doc.map.droids.get(i) {
                    write!(info, " | Sel: {} [P{}]", d.name, d.player).unwrap();
                }
            }
            SelectedObject::Feature(i) => {
                if let Some(f) = doc.map.features.get(i) {
                    write!(info, " | Sel: {}", f.name).unwrap();
                }
            }
            SelectedObject::Label(i) => {
                if let Some((key, label)) = doc.map.labels.get(i) {
                    write!(info, " | Label: {} ({})", label.label(), key).unwrap();
                }
            }
            SelectedObject::Gateway(i) => {
                if let Some(gw) = doc.map.map_data.gateways.get(i) {
                    write!(
                        info,
                        " | Gateway #{} ({},{})-({},{})",
                        i, gw.x1, gw.y1, gw.x2, gw.y2
                    )
                    .unwrap();
                }
            }
        }
    }

    painter.text(
        rect.right_top() + egui::vec2(-8.0, 8.0),
        Align2::RIGHT_TOP,
        info,
        FontId::proportional(12.0),
        Color32::WHITE,
    );

    let help = app
        .tool_state
        .tools
        .get(&app.tool_state.active_tool)
        .and_then(|t| t.help_text(&app.config.keymap))
        .unwrap_or_else(|| "RMB+WASD=fly | MMB=pan | Scroll=zoom | LMB=use tool".to_string());
    painter.text(
        rect.left_bottom() + egui::vec2(8.0, -8.0),
        Align2::LEFT_BOTTOM,
        help,
        FontId::proportional(11.0),
        Color32::from_rgba_premultiplied(200, 200, 200, 180),
    );
}

pub(super) fn draw_speed_readout(ui: &Ui, camera: Option<&Camera>, rect: Rect) {
    let Some(cam) = camera else {
        return;
    };
    ui.painter_at(rect).text(
        rect.center_top() + egui::vec2(0.0, 8.0),
        Align2::CENTER_TOP,
        format!("Speed: {:.1}", cam.speed_level()),
        FontId::proportional(12.0),
        Color32::from_rgba_premultiplied(200, 200, 200, 180),
    );
}

pub(super) fn draw_fps_readout(ui: &mut Ui, app: &mut EditorApp, rect: Rect) {
    if !app.show_fps {
        return;
    }
    let Some((avg, min, max)) = app.fps_stats() else {
        return;
    };
    let fps = if avg > 0.0 { 1.0 / avg } else { 0.0 };

    // Fixed widths (up to 999.9 fps, 999.99 ms) keep the label from
    // shifting between frames.
    let stats_line = format!(
        "{fps:>5.1} fps   avg {:>6.2} ms   min {:>6.2}   max {:>6.2}",
        avg * 1000.0,
        min * 1000.0,
        max * 1000.0,
    );

    let font = FontId::monospace(11.5);
    let stats_color = Color32::from_rgb(225, 235, 230);
    let info_color = Color32::from_rgb(160, 175, 175);

    egui::Area::new(egui::Id::new("fps_readout"))
        .fixed_pos(egui::pos2(rect.center().x, rect.top() + 4.0))
        .pivot(Align2::CENTER_TOP)
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(0, 0, 0, 180))
                .stroke(Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                ))
                .corner_radius(CornerRadius::same(5))
                .inner_margin(Margin {
                    left: 12,
                    right: 12,
                    top: 6,
                    bottom: 6,
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(stats_line)
                                .font(font.clone())
                                .color(stats_color),
                        );
                        if !app.gpu_info_label.is_empty() {
                            ui.label(
                                RichText::new(&app.gpu_info_label)
                                    .font(font)
                                    .color(info_color),
                            );
                        }
                    });
                });
        });
}
