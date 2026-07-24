use egui::{Color32, Painter, Pos2, Rect, Stroke, Ui};
use glam::{Mat4, Vec3};

use crate::app::{EditorApp, SelectedObject};
use crate::viewport::camera::Camera;
use crate::viewport::picking;

use super::project_world_to_screen;

const AABB_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (1, 3),
    (3, 2),
    (2, 0),
    (4, 5),
    (5, 7),
    (7, 6),
    (6, 4),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// Draw a 12-edge wireframe for an axis-aligned box centered at `center`
/// with half-extents `half`. Edges with both endpoints behind the camera,
/// or both endpoints fully outside `rect`, are skipped.
fn draw_aabb_wireframe(
    painter: &Painter,
    vp: &Mat4,
    rect: Rect,
    center: Vec3,
    half: Vec3,
    stroke: Stroke,
) {
    let min = center - half;
    let max = center + half;
    let corners = [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ];
    let screen: [Option<Pos2>; 8] = [
        project_world_to_screen(vp, corners[0], rect),
        project_world_to_screen(vp, corners[1], rect),
        project_world_to_screen(vp, corners[2], rect),
        project_world_to_screen(vp, corners[3], rect),
        project_world_to_screen(vp, corners[4], rect),
        project_world_to_screen(vp, corners[5], rect),
        project_world_to_screen(vp, corners[6], rect),
        project_world_to_screen(vp, corners[7], rect),
    ];
    for (a, b) in AABB_EDGES {
        if let (Some(sa), Some(sb)) = (screen[a], screen[b])
            && (rect.contains(sa) || rect.contains(sb))
        {
            painter.line_segment([sa, sb], stroke);
        }
    }
}

pub(super) fn draw(ui: &mut Ui, app: &mut EditorApp, camera: Option<&Camera>, rect: Rect) {
    // "Show all" supersedes "show selection"; the selection is a subset, so
    // no de-dup needed.
    let show_all = app.show_all_hitboxes;
    let show_sel = app.show_selection_hitboxes;
    if !(show_all || show_sel) {
        return;
    }
    let Some(camera) = camera else {
        return;
    };
    let Some(doc) = app.document.as_ref() else {
        return;
    };

    let vp = camera.view_projection_matrix();
    let painter = ui.painter_at(rect);
    let map_data = &doc.map.map_data;

    let render_state = app.wgpu_render_state.clone();
    let boxes_opt = render_state.as_ref().and_then(|rs| {
        let r = rs.renderer.read();
        r.callback_resources
            .get::<crate::viewport::ViewportResources>()
            .map(|res| {
                let mut boxes: Vec<(glam::Vec3, glam::Vec3)> = Vec::new();
                let push_structure =
                    |boxes: &mut Vec<(glam::Vec3, glam::Vec3)>,
                     s: &wz_maplib::objects::Structure| {
                        let (half, center_y) = picking::structure_half_extents(
                            &s.name,
                            s.modules,
                            app.stats.as_ref(),
                            app.model_loader.as_ref(),
                            &res.renderer,
                        );
                        let center = picking::object_world_center_with_offset(
                            s.position.x,
                            s.position.y,
                            map_data,
                            center_y,
                        );
                        boxes.push((center, half));
                    };
                let push_named =
                    |boxes: &mut Vec<(glam::Vec3, glam::Vec3)>,
                     name: &str,
                     pos: wz_maplib::objects::WorldPos| {
                        let (half, center_y) = picking::object_half_extents_with_center(
                            name,
                            app.model_loader.as_ref(),
                            &res.renderer,
                        );
                        let center = picking::object_world_center_with_offset(
                            pos.x, pos.y, map_data, center_y,
                        );
                        boxes.push((center, half));
                    };

                if show_all {
                    for s in &doc.map.structures {
                        push_structure(&mut boxes, s);
                    }
                    for d in &doc.map.droids {
                        push_named(&mut boxes, &d.name, d.position);
                    }
                    for f in &doc.map.features {
                        push_named(&mut boxes, &f.name, f.position);
                    }
                } else {
                    for obj in &app.selection.objects {
                        match *obj {
                            SelectedObject::Structure(i) => {
                                if let Some(s) = doc.map.structures.get(i) {
                                    push_structure(&mut boxes, s);
                                }
                            }
                            SelectedObject::Droid(i) => {
                                if let Some(d) = doc.map.droids.get(i) {
                                    push_named(&mut boxes, &d.name, d.position);
                                }
                            }
                            SelectedObject::Feature(i) => {
                                if let Some(f) = doc.map.features.get(i) {
                                    push_named(&mut boxes, &f.name, f.position);
                                }
                            }
                            SelectedObject::Label(_) | SelectedObject::Gateway(_) => {}
                        }
                    }
                }
                boxes
            })
    });

    if let Some(boxes) = boxes_opt {
        let stroke = Stroke::new(1.0_f32, Color32::from_rgb(0, 255, 0));
        for (center, half) in &boxes {
            draw_aabb_wireframe(&painter, &vp, rect, *center, *half, stroke);
        }
    }
}
