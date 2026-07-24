use egui::{Color32, FontId, Rect, Stroke, StrokeKind, Ui, Vec2};

use crate::app::EditorApp;

use wz_stats::terrain_table::PropulsionClass;

const HEATMAP_TOGGLE_SIZE: Vec2 = egui::vec2(24.0, 24.0);
const HEATMAP_PROP_BTN_SIZE: Vec2 = egui::vec2(62.0, 22.0);
const HEATMAP_LEGEND_BAR_SIZE: Vec2 = egui::vec2(10.0, 80.0);

pub(super) fn draw(ui: &mut Ui, app: &mut EditorApp, rect: Rect) {
    if app.document.is_none() {
        return;
    }
    let has_terrain_table = app
        .stats
        .as_ref()
        .and_then(|s| s.terrain_table.as_ref())
        .is_some();
    if !has_terrain_table {
        return;
    }

    let margin = 8.0;
    let spacing = 4.0;
    let painter = ui.painter_at(rect);

    let hm_rect = Rect::from_min_size(
        egui::pos2(
            rect.right() - margin - HEATMAP_TOGGLE_SIZE.x,
            rect.bottom() - margin - HEATMAP_TOGGLE_SIZE.y,
        ),
        HEATMAP_TOGGLE_SIZE,
    );
    let is_active = app.show_heatmap;
    let bg = if is_active {
        Color32::from_rgba_unmultiplied(80, 140, 220, 200)
    } else {
        Color32::from_rgba_unmultiplied(30, 30, 30, 160)
    };
    let hm_response = ui.allocate_rect(hm_rect, egui::Sense::click());
    let bg = if hm_response.hovered() && !is_active {
        Color32::from_rgba_unmultiplied(60, 60, 60, 190)
    } else {
        bg
    };
    painter.rect_filled(hm_rect, 4.0, bg);
    painter.rect_stroke(
        hm_rect,
        4.0,
        Stroke::new(
            1.0_f32,
            if is_active {
                Color32::from_rgb(130, 180, 255)
            } else {
                Color32::from_rgba_unmultiplied(100, 100, 100, 120)
            },
        ),
        StrokeKind::Inside,
    );
    let text_color = if is_active {
        Color32::WHITE
    } else {
        Color32::from_rgba_unmultiplied(200, 200, 200, 200)
    };
    painter.text(
        hm_rect.center(),
        egui::Align2::CENTER_CENTER,
        "H",
        FontId::proportional(13.0),
        text_color,
    );
    if hm_response.clicked() {
        app.show_heatmap = !app.show_heatmap;
        if app.show_heatmap {
            app.heatmap_dirty = true;
        }
    }
    hm_response.on_hover_text("Propulsion Speed Heatmap (H)");

    if !app.show_heatmap {
        return;
    }

    let propulsion_types: &[(PropulsionClass, &str)] = &[
        (PropulsionClass::Wheeled, "Wheeled"),
        (PropulsionClass::Tracked, "Tracked"),
        (PropulsionClass::HalfTracked, "Half-Track"),
        (PropulsionClass::Hover, "Hover"),
        (PropulsionClass::Legged, "Legs"),
    ];

    let prop_font = FontId::proportional(11.0);
    let hotbar_right = hm_rect.left() - spacing;
    let hotbar_y = hm_rect.center().y - HEATMAP_PROP_BTN_SIZE.y / 2.0;

    for (i, &(prop, label)) in propulsion_types.iter().enumerate() {
        let x = hotbar_right - (HEATMAP_PROP_BTN_SIZE.x + spacing) * (i as f32 + 1.0) + spacing;
        let btn_rect = Rect::from_min_size(egui::pos2(x, hotbar_y), HEATMAP_PROP_BTN_SIZE);

        let prop_active = app.heatmap_propulsion == prop;
        let bg = if prop_active {
            Color32::from_rgba_unmultiplied(80, 140, 220, 200)
        } else {
            Color32::from_rgba_unmultiplied(30, 30, 30, 160)
        };
        let btn_response = ui.allocate_rect(btn_rect, egui::Sense::click());
        let bg = if btn_response.hovered() && !prop_active {
            Color32::from_rgba_unmultiplied(60, 60, 60, 190)
        } else {
            bg
        };

        painter.rect_filled(btn_rect, 4.0, bg);
        painter.rect_stroke(
            btn_rect,
            4.0,
            Stroke::new(
                1.0_f32,
                if prop_active {
                    Color32::from_rgb(130, 180, 255)
                } else {
                    Color32::from_rgba_unmultiplied(100, 100, 100, 120)
                },
            ),
            StrokeKind::Inside,
        );
        let text_color = if prop_active {
            Color32::WHITE
        } else {
            Color32::from_rgba_unmultiplied(200, 200, 200, 200)
        };
        painter.text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            prop_font.clone(),
            text_color,
        );
        if btn_response.clicked() {
            app.heatmap_propulsion = prop;
            app.heatmap_dirty = true;
        }
        btn_response.on_hover_text(prop.display_name());
    }

    let bar_bottom = hm_rect.top() - spacing;
    let bar_rect = Rect::from_min_size(
        egui::pos2(
            rect.right() - margin - HEATMAP_LEGEND_BAR_SIZE.x,
            bar_bottom - HEATMAP_LEGEND_BAR_SIZE.y,
        ),
        HEATMAP_LEGEND_BAR_SIZE,
    );

    let steps = 24;
    let step_h = HEATMAP_LEGEND_BAR_SIZE.y / steps as f32;
    for s in 0..steps {
        let t = 1.0 - (s as f32 / (steps - 1) as f32);
        let speed = t * 1.5;
        let (r, g, b) = if speed < 1.0 {
            let s_val = speed.clamp(0.0, 1.0);
            (
                lerp_u8(230, 242, s_val),
                lerp_u8(38, 217, s_val),
                lerp_u8(25, 25, s_val),
            )
        } else {
            let s_val = ((speed - 1.0) / 0.5).clamp(0.0, 1.0);
            (
                lerp_u8(242, 25, s_val),
                lerp_u8(217, 204, s_val),
                lerp_u8(25, 51, s_val),
            )
        };
        let slice_rect = Rect::from_min_size(
            egui::pos2(bar_rect.left(), bar_rect.top() + s as f32 * step_h),
            egui::vec2(HEATMAP_LEGEND_BAR_SIZE.x, step_h + 0.5),
        );
        painter.rect_filled(
            slice_rect,
            0.0,
            Color32::from_rgba_unmultiplied(r, g, b, 200),
        );
    }
    painter.rect_stroke(
        bar_rect,
        2.0,
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(100, 100, 100, 120)),
        StrokeKind::Inside,
    );

    let label_font = FontId::proportional(9.0);
    let label_color = Color32::from_rgba_unmultiplied(220, 220, 220, 220);
    let label_x = bar_rect.left() - 3.0;
    let bar_h = HEATMAP_LEGEND_BAR_SIZE.y;
    painter.text(
        egui::pos2(label_x, bar_rect.top()),
        egui::Align2::RIGHT_TOP,
        "150%",
        label_font.clone(),
        label_color,
    );
    painter.text(
        egui::pos2(label_x, bar_rect.top() + bar_h / 3.0),
        egui::Align2::RIGHT_CENTER,
        "100%",
        label_font.clone(),
        label_color,
    );
    painter.text(
        egui::pos2(label_x, bar_rect.top() + bar_h * 2.0 / 3.0),
        egui::Align2::RIGHT_CENTER,
        "50%",
        label_font.clone(),
        label_color,
    );
    painter.text(
        egui::pos2(label_x, bar_rect.bottom()),
        egui::Align2::RIGHT_BOTTOM,
        "0%",
        label_font,
        label_color,
    );
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let v = a as f32 + (b as f32 - a as f32) * t;
    v.round().clamp(0.0, 255.0) as u8
}
