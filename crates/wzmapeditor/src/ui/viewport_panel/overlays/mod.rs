//! 2D overlay rendering for the viewport.

mod balance;
mod debug;
mod heatmap;
mod hitbox;
mod labels;
mod tools;

use egui::{Pos2, Rect, Ui};
use glam::{Mat4, Vec3, Vec4};

use crate::app::EditorApp;
use crate::viewport::camera::Camera;

/// Maps a world-space point to screen-space within `rect`. Returns `None`
/// when the point is behind the camera (clip-space `w <= 0`).
pub(super) fn project_world_to_screen(vp: &Mat4, world: Vec3, rect: Rect) -> Option<Pos2> {
    let c = *vp * Vec4::new(world.x, world.y, world.z, 1.0);
    if c.w <= 0.0 {
        return None;
    }
    let n = Vec3::new(c.x / c.w, c.y / c.w, c.z / c.w);
    Some(Pos2::new(
        rect.left() + (n.x * 0.5 + 0.5) * rect.width(),
        rect.top() + (-n.y * 0.5 + 0.5) * rect.height(),
    ))
}

/// Order matters: painter overlays run first so the click-allocating button
/// overlays paint on top.
pub(super) fn draw_all(ui: &mut Ui, app: &mut EditorApp, camera: Option<&Camera>, rect: Rect) {
    labels::draw(ui, app, camera, rect);
    hitbox::draw(ui, app, camera, rect);
    tools::draw_gateways(ui, app, camera, rect);
    balance::draw(ui, app, camera, rect);
    tools::draw_script_label_drag(ui, app, camera, rect);
    tools::draw_stamp(ui, app, camera, rect);
    tools::draw_line_preview(ui, app, camera, rect);
    debug::draw_info_bar(ui, app, camera, rect);
    tools::draw_vertex_sculpt(ui, app, camera, rect);
    tools::draw_mirror_axis(ui, app, camera, rect);
    tools::draw_tool_buttons(ui, app, rect);
    heatmap::draw(ui, app, rect);
    debug::draw_fps_readout(ui, app, rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis_aligned_camera() -> Camera {
        let mut camera = Camera::for_map(64, 64);
        camera.position = Vec3::new(0.0, 0.0, 100.0);
        camera.yaw = std::f32::consts::PI;
        camera.pitch = 0.0;
        camera.aspect = 1.0;
        camera.fov = std::f32::consts::FRAC_PI_2;
        camera.near = 1.0;
        camera.far = 1000.0;
        camera
    }

    #[test]
    fn project_origin_lands_at_rect_center() {
        let camera = axis_aligned_camera();
        let vp = camera.view_projection_matrix();
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 600.0));
        let p = project_world_to_screen(&vp, Vec3::ZERO, rect)
            .expect("origin sits in front of an axis-aligned camera");
        assert!((p.x - rect.center().x).abs() < 0.01, "x={}", p.x);
        assert!((p.y - rect.center().y).abs() < 0.01, "y={}", p.y);
    }

    #[test]
    fn project_point_behind_camera_returns_none() {
        let camera = axis_aligned_camera();
        let vp = camera.view_projection_matrix();
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 600.0));
        // Camera is at z=100 looking toward -Z. A world point at z=200 is
        // strictly behind it, so the projection must reject it.
        let behind = project_world_to_screen(&vp, Vec3::new(0.0, 0.0, 200.0), rect);
        assert!(behind.is_none());
    }

    #[test]
    fn project_point_offset_right_lands_right_of_center() {
        let camera = axis_aligned_camera();
        let vp = camera.view_projection_matrix();
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 600.0));
        // With yaw=PI the camera looks down -Z and its right vector
        // aligns with world +X, so a world point at +X must land right
        // of screen center.
        let p = project_world_to_screen(&vp, Vec3::new(10.0, 0.0, 0.0), rect)
            .expect("point in front of camera");
        assert!(
            p.x > rect.center().x,
            "x={} center={}",
            p.x,
            rect.center().x
        );
        assert!((p.y - rect.center().y).abs() < 0.01);
    }
}
