use egui::{Color32, FontId, Pos2, Rect, Shape, Stroke, Ui};
use glam::Vec4;

use crate::app::EditorApp;
use crate::viewport::camera::Camera;
use crate::viewport::picking;

use super::project_world_to_screen;

pub(super) fn draw(ui: &mut Ui, app: &mut EditorApp, camera: Option<&Camera>, rect: Rect) {
    if !app.labels_visible() {
        return;
    }
    let Some(camera) = camera else {
        return;
    };
    let Some(doc) = app.document.as_ref() else {
        return;
    };
    if doc.map.labels.is_empty() {
        return;
    }

    let vp = camera.view_projection_matrix();
    let painter = ui.painter_at(rect);

    for (_key, label) in &doc.map.labels {
        let center = label.center();
        let world_x = center.x as f32;
        let world_z = center.y as f32;
        let world_y =
            picking::sample_terrain_height_pub(&doc.map.map_data, world_x, world_z) + 20.0;

        // NDC clip drops off-screen markers entirely instead of clamping them
        // to the viewport edge.
        let clip = vp * Vec4::new(world_x, world_y, world_z, 1.0);
        if clip.w <= 0.0 {
            continue;
        }
        let ndc_x = clip.x / clip.w;
        let ndc_y = clip.y / clip.w;
        if !(-1.0..=1.0).contains(&ndc_x) || !(-1.0..=1.0).contains(&ndc_y) {
            continue;
        }
        let screen_pos = Pos2::new(
            rect.left() + (ndc_x * 0.5 + 0.5) * rect.width(),
            rect.top() + (-ndc_y * 0.5 + 0.5) * rect.height(),
        );

        match label {
            wz_maplib::labels::ScriptLabel::Position { label: name, .. } => {
                let sz = 6.0_f32;
                let color = Color32::from_rgb(255, 100, 50);
                painter.circle_filled(screen_pos, sz, color);
                painter.circle_stroke(screen_pos, sz, Stroke::new(1.0, Color32::BLACK));
                painter.text(
                    screen_pos + egui::vec2(sz + 4.0, -6.0),
                    egui::Align2::LEFT_TOP,
                    name,
                    FontId::proportional(11.0),
                    Color32::from_rgba_premultiplied(255, 200, 100, 220),
                );
            }
            wz_maplib::labels::ScriptLabel::Area {
                label: name,
                pos1,
                pos2,
            } => {
                let project_area = |wx: f32, wz: f32| -> Option<Pos2> {
                    let wy = picking::sample_terrain_height_pub(&doc.map.map_data, wx, wz) + 5.0;
                    project_world_to_screen(&vp, glam::Vec3::new(wx, wy, wz), rect)
                };
                let corners = [
                    project_area(pos1[0] as f32, pos1[1] as f32),
                    project_area(pos2[0] as f32, pos1[1] as f32),
                    project_area(pos2[0] as f32, pos2[1] as f32),
                    project_area(pos1[0] as f32, pos2[1] as f32),
                ];
                if let [Some(a), Some(b), Some(c), Some(d)] = corners {
                    let fill = Color32::from_rgba_unmultiplied(100, 200, 255, 40);
                    let stroke_color = Color32::from_rgba_unmultiplied(100, 200, 255, 180);
                    painter.add(Shape::convex_polygon(
                        vec![a, b, c, d],
                        fill,
                        Stroke::new(1.5, stroke_color),
                    ));
                    painter.text(
                        screen_pos + egui::vec2(0.0, -12.0),
                        egui::Align2::CENTER_BOTTOM,
                        name,
                        FontId::proportional(11.0),
                        Color32::from_rgba_premultiplied(100, 220, 255, 220),
                    );
                }
            }
        }
    }
}
