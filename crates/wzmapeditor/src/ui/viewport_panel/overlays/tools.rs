use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui};
use glam::{Mat4, Vec3, Vec4};

use crate::app::{EditorApp, SelectedObject};
use crate::tools::{self, MirrorMode, ToolId};
use crate::viewport::camera::Camera;
use crate::viewport::picking;

use wz_maplib::constants::TILE_UNITS_F32 as TILE_UNITS;
use wz_maplib::map_data::MapData;

use super::project_world_to_screen;

/// A clip-space `w` smaller than this means the gateway corner sits on
/// the near plane; the divide explodes to huge NDC values that egui
/// would paint as a streak.
const GATEWAY_MIN_CLIP_W: f32 = 1.0;

/// Sanity bound that complements `GATEWAY_MIN_CLIP_W`: corners far
/// outside `[-1, 1]` NDC are dropped instead of drawn as a smear.
const GATEWAY_MAX_NDC: f32 = 4.0;

pub(super) fn draw_gateways(ui: &mut Ui, app: &mut EditorApp, camera: Option<&Camera>, rect: Rect) {
    if !app.show_gateways {
        return;
    }
    let Some(camera) = camera else {
        return;
    };
    let Some(doc) = app.document.as_ref() else {
        return;
    };
    if doc.map.map_data.gateways.is_empty() {
        return;
    }

    let vp = camera.view_projection_matrix();
    let painter = ui.painter_at(rect);
    let map_data = &doc.map.map_data;
    // Open-coded projection: gateway quads need NDC bounds plus the
    // behind-camera reject so a corner grazing the near plane doesn't smear
    // across the viewport.
    let project = |wx: f32, wz: f32| -> Option<Pos2> {
        let wy = picking::sample_terrain_height_pub(map_data, wx, wz) + 5.0;
        let c = vp * Vec4::new(wx, wy, wz, 1.0);
        if c.w < GATEWAY_MIN_CLIP_W {
            return None;
        }
        let nx = c.x / c.w;
        let ny = c.y / c.w;
        if nx.abs() > GATEWAY_MAX_NDC || ny.abs() > GATEWAY_MAX_NDC {
            return None;
        }
        Some(Pos2::new(
            rect.left() + (nx * 0.5 + 0.5) * rect.width(),
            rect.top() + (-ny * 0.5 + 0.5) * rect.height(),
        ))
    };
    let gray = Color32::from_rgb(170, 170, 170);
    let fill = Color32::from_rgba_unmultiplied(170, 170, 170, 50);
    let stroke = Stroke::new(1.5, gray);
    let sel_stroke = Stroke::new(2.5, Color32::from_rgb(255, 180, 40));
    let sel_fill = Color32::from_rgba_unmultiplied(255, 180, 40, 70);
    for (i, gw) in map_data.gateways.iter().enumerate() {
        let x1 = f32::from(gw.x1) * TILE_UNITS;
        let y1 = f32::from(gw.y1) * TILE_UNITS;
        let x2 = (f32::from(gw.x2) + 1.0) * TILE_UNITS;
        let y2 = (f32::from(gw.y2) + 1.0) * TILE_UNITS;
        let corners = [
            project(x1, y1),
            project(x2, y1),
            project(x2, y2),
            project(x1, y2),
        ];
        if let [Some(a), Some(b), Some(c), Some(d)] = corners {
            let selected = app.selection.contains(&SelectedObject::Gateway(i));
            let (f, s) = if selected {
                (sel_fill, sel_stroke)
            } else {
                (fill, stroke)
            };
            painter.add(Shape::convex_polygon(vec![a, b, c, d], f, s));
        }
    }
}

pub(super) fn draw_script_label_drag(
    ui: &mut Ui,
    app: &mut EditorApp,
    camera: Option<&Camera>,
    rect: Rect,
) {
    if app.tool_state.active_tool != ToolId::ScriptLabel {
        return;
    }
    let Some(label_tool) = app.tool_state.script_label() else {
        return;
    };
    let (Some((sx, sy)), Some((tx, ty)), Some(doc), Some(camera)) = (
        label_tool.drag_start(),
        app.hovered_tile,
        app.document.as_ref(),
        camera,
    ) else {
        return;
    };

    let vp = camera.view_projection_matrix();
    let painter = ui.painter_at(rect);

    let min_x = sx.min(tx);
    let min_y = sy.min(ty);
    let max_x = sx.max(tx) + 1;
    let max_y = sy.max(ty) + 1;

    let wx1 = (min_x * TILE_UNITS as u32) as f32;
    let wz1 = (min_y * TILE_UNITS as u32) as f32;
    let wx2 = (max_x * TILE_UNITS as u32) as f32;
    let wz2 = (max_y * TILE_UNITS as u32) as f32;

    let project = |wx: f32, wz: f32| -> Option<Pos2> {
        let wy = picking::sample_terrain_height_pub(&doc.map.map_data, wx, wz) + 5.0;
        project_world_to_screen(&vp, Vec3::new(wx, wy, wz), rect)
    };

    let corners = [
        project(wx1, wz1),
        project(wx2, wz1),
        project(wx2, wz2),
        project(wx1, wz2),
    ];

    if let [Some(a), Some(b), Some(c), Some(d)] = corners {
        let fill = Color32::from_rgba_unmultiplied(100, 200, 255, 30);
        let stroke = Color32::from_rgba_unmultiplied(100, 200, 255, 200);
        painter.add(Shape::convex_polygon(
            vec![a, b, c, d],
            fill,
            Stroke::new(2.0, stroke),
        ));
    }
}

pub(super) fn draw_stamp(ui: &mut Ui, app: &mut EditorApp, camera: Option<&Camera>, rect: Rect) {
    if app.tool_state.active_tool != ToolId::Stamp {
        return;
    }
    let (Some(doc), Some(camera)) = (app.document.as_ref(), camera) else {
        return;
    };

    let vp = camera.view_projection_matrix();
    let painter = ui.painter_at(rect);
    let map_data = &doc.map.map_data;

    let project_tile_rect = |min_x: u32, min_y: u32, max_x: u32, max_y: u32| {
        let wx1 = (min_x * TILE_UNITS as u32) as f32;
        let wz1 = (min_y * TILE_UNITS as u32) as f32;
        let wx2 = ((max_x + 1) * TILE_UNITS as u32) as f32;
        let wz2 = ((max_y + 1) * TILE_UNITS as u32) as f32;

        let proj = |wx: f32, wz: f32| -> Option<Pos2> {
            let wy = picking::sample_terrain_height_pub(map_data, wx, wz) + 5.0;
            project_world_to_screen(&vp, Vec3::new(wx, wy, wz), rect)
        };

        [
            proj(wx1, wz1),
            proj(wx2, wz1),
            proj(wx2, wz2),
            proj(wx1, wz2),
        ]
    };

    let stamp = app.tool_state.stamp();

    if let Some(stamp) = stamp
        && stamp.capture_mode
        && let (Some((sx, sy)), Some((tx, ty))) = (stamp.capture_start, app.hovered_tile)
    {
        let corners = project_tile_rect(sx.min(tx), sy.min(ty), sx.max(tx), sy.max(ty));
        if let [Some(a), Some(b), Some(c), Some(d)] = corners {
            let fill = Color32::from_rgba_unmultiplied(255, 200, 50, 30);
            let stroke = Color32::from_rgba_unmultiplied(255, 200, 50, 200);
            painter.add(Shape::convex_polygon(
                vec![a, b, c, d],
                fill,
                Stroke::new(2.0, stroke),
            ));
        }
    }

    let mode = stamp
        .filter(|s| !s.capture_mode && s.pattern.is_some())
        .map(|s| s.mode);
    if let Some(mode) = mode {
        match mode {
            tools::StampMode::Single => {
                draw_stamp_single_preview(&painter, &project_tile_rect, app, map_data);
            }
            tools::StampMode::Scatter => {
                draw_stamp_scatter_preview(&painter, app, map_data, &vp, rect);
            }
        }
    }
}

fn draw_stamp_single_preview(
    painter: &Painter,
    project_tile_rect: &impl Fn(u32, u32, u32, u32) -> [Option<Pos2>; 4],
    app: &EditorApp,
    map_data: &MapData,
) {
    let Some(stamp) = app.tool_state.stamp() else {
        return;
    };
    let (Some((tx, ty)), Some(pattern)) = (stamp.preview_pos, stamp.pattern.as_ref()) else {
        return;
    };
    let map_w = map_data.width;
    let map_h = map_data.height;
    let end_x = tx + pattern.width - 1;
    let end_y = ty + pattern.height - 1;

    let on_map = end_x < map_w && end_y < map_h;
    let (fill, stroke_color) = if on_map {
        (
            Color32::from_rgba_unmultiplied(50, 255, 100, 30),
            Color32::from_rgba_unmultiplied(50, 255, 100, 200),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(255, 80, 50, 30),
            Color32::from_rgba_unmultiplied(255, 80, 50, 200),
        )
    };

    let corners = project_tile_rect(
        tx,
        ty,
        end_x.min(map_w.saturating_sub(1)),
        end_y.min(map_h.saturating_sub(1)),
    );
    if let [Some(a), Some(b), Some(c), Some(d)] = corners {
        painter.add(Shape::convex_polygon(
            vec![a, b, c, d],
            fill,
            Stroke::new(2.0, stroke_color),
        ));
    }
}

const CIRCLE_SAMPLES: usize = 48;

fn draw_stamp_scatter_preview(
    painter: &Painter,
    app: &EditorApp,
    map_data: &MapData,
    vp: &Mat4,
    rect: Rect,
) {
    let Some(stamp) = app.tool_state.stamp() else {
        return;
    };
    let Some((tx, ty)) = stamp.preview_pos else {
        return;
    };
    let radius_tiles = stamp.scatter_radius;
    if radius_tiles == 0 {
        return;
    }

    let tile_world = TILE_UNITS;
    let cx = (tx as f32) * tile_world + tile_world * 0.5;
    let cz = (ty as f32) * tile_world + tile_world * 0.5;
    let r_world = (radius_tiles as f32) * tile_world;

    let mut points = Vec::with_capacity(CIRCLE_SAMPLES);
    for i in 0..CIRCLE_SAMPLES {
        let theta = (i as f32 / CIRCLE_SAMPLES as f32) * std::f32::consts::TAU;
        let wx = cx + r_world * theta.cos();
        let wz = cz + r_world * theta.sin();
        let wy = picking::sample_terrain_height_pub(map_data, wx, wz) + 5.0;
        // Drop the entire ring if any sample is behind the camera, to avoid
        // a mangled polyline.
        let Some(p) = project_world_to_screen(vp, Vec3::new(wx, wy, wz), rect) else {
            return;
        };
        points.push(p);
    }

    let stroke = Stroke::new(2.0, Color32::from_rgba_unmultiplied(80, 200, 255, 220));
    points.push(points[0]);
    painter.add(Shape::line(points, stroke));
}

pub(super) fn draw_vertex_sculpt(
    ui: &mut Ui,
    app: &mut EditorApp,
    camera: Option<&Camera>,
    rect: Rect,
) {
    if app.tool_state.active_tool != ToolId::VertexSculpt {
        return;
    }
    let (Some(doc), Some(camera)) = (app.document.as_ref(), camera) else {
        return;
    };
    let Some(tool) = app.tool_state.vertex_sculpt() else {
        return;
    };
    let map = &doc.map.map_data;
    let painter = ui.painter_at(rect);
    let vp = camera.view_projection_matrix();
    let project = |world: Vec3| -> Option<Pos2> { project_world_to_screen(&vp, world, rect) };

    let yellow = Color32::from_rgb(255, 220, 60);
    let outline = Color32::BLACK;

    let dragging = tool.marquee_active || tool.marquee_start.is_some();

    let mirror_mode = app.tool_state.mirror_mode;
    let mirror_active = mirror_mode != MirrorMode::None && ToolId::VertexSculpt.uses_mirror();
    let mirrored_selected: Vec<(u32, u32)> = if mirror_active {
        let mut out = Vec::new();
        for &(vx, vy) in &tool.selected_vertices {
            for pt in
                tools::mirror::mirror_vertex_points(vx, vy, map.width, map.height, mirror_mode)
            {
                if pt != (vx, vy) && !tool.selected_vertices.contains(&pt) && !out.contains(&pt) {
                    out.push(pt);
                }
            }
        }
        out
    } else {
        Vec::new()
    };

    // Faint dot field around the cursor; hidden during marquee drag so the
    // terrain reaction stays visible.
    if !dragging && let Some((hx, hy)) = app.hovered_tile {
        const DOT_RADIUS_TILES: i32 = 8;
        const DOT_RADIUS_SQ: f32 = (DOT_RADIUS_TILES * DOT_RADIUS_TILES) as f32;
        let selected: std::collections::HashSet<(u32, u32)> = tool
            .selected_vertices
            .iter()
            .chain(mirrored_selected.iter())
            .copied()
            .collect();
        let inv_r = 1.0 / DOT_RADIUS_TILES as f32;

        let mut centers: Vec<(i32, i32)> = vec![(hx as i32, hy as i32)];
        if mirror_active {
            for (mx, my) in
                tools::mirror::mirror_vertex_points(hx, hy, map.width, map.height, mirror_mode)
            {
                let pt = (mx as i32, my as i32);
                if !centers.contains(&pt) {
                    centers.push(pt);
                }
            }
        }

        let mut drawn: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        for (cx, cy) in centers {
            for dy in -DOT_RADIUS_TILES..=DOT_RADIUS_TILES {
                for dx in -DOT_RADIUS_TILES..=DOT_RADIUS_TILES {
                    let dist_sq = (dx * dx + dy * dy) as f32;
                    if dist_sq > DOT_RADIUS_SQ {
                        continue;
                    }
                    let vx_i = cx + dx;
                    let vy_i = cy + dy;
                    if vx_i < 0 || vy_i < 0 {
                        continue;
                    }
                    let vx = vx_i as u32;
                    let vy = vy_i as u32;
                    if vx >= map.width || vy >= map.height {
                        continue;
                    }
                    if selected.contains(&(vx, vy)) || !drawn.insert((vx, vy)) {
                        continue;
                    }
                    let h = map.tile(vx, vy).map_or(0.0, |t| t.height as f32);
                    let world = Vec3::new(vx as f32 * TILE_UNITS, h + 2.0, vy as f32 * TILE_UNITS);
                    let Some(p) = project(world) else {
                        continue;
                    };
                    let t = (dist_sq.sqrt() * inv_r).clamp(0.0, 1.0);
                    let fade = 1.0 - t * t;
                    let alpha = (fade * 170.0) as u8;
                    if alpha < 12 {
                        continue;
                    }
                    let fill = Color32::from_rgba_unmultiplied(235, 235, 235, alpha);
                    let rim_alpha = (fade * 200.0) as u8;
                    let rim = Color32::from_rgba_unmultiplied(20, 20, 20, rim_alpha);
                    painter.circle_filled(p, 2.0, rim);
                    painter.circle_filled(p, 1.4, fill);
                }
            }
        }
    }

    for &(vx, vy) in &tool.selected_vertices {
        let h = map.tile(vx, vy).map_or(0.0, |t| t.height as f32);
        let world = Vec3::new(vx as f32 * TILE_UNITS, h + 4.0, vy as f32 * TILE_UNITS);
        if let Some(p) = project(world) {
            let half = 4.5;
            let r = Rect::from_center_size(p, egui::vec2(half * 2.0, half * 2.0));
            painter.rect_filled(r, 1.0, yellow);
            painter.rect_stroke(r, 1.0, Stroke::new(1.5, outline), StrokeKind::Inside);
        }
    }

    let mirror_fill = Color32::from_rgba_unmultiplied(255, 220, 60, 70);
    for &(vx, vy) in &mirrored_selected {
        let h = map.tile(vx, vy).map_or(0.0, |t| t.height as f32);
        let world = Vec3::new(vx as f32 * TILE_UNITS, h + 4.0, vy as f32 * TILE_UNITS);
        if let Some(p) = project(world) {
            let half = 4.5;
            let r = Rect::from_center_size(p, egui::vec2(half * 2.0, half * 2.0));
            painter.rect_filled(r, 1.0, mirror_fill);
            painter.rect_stroke(r, 1.0, Stroke::new(1.5, yellow), StrokeKind::Inside);
        }
    }

    if tool.marquee_active
        && let (Some(start), Some(end)) = (tool.marquee_start, tool.marquee_current)
    {
        let r = Rect::from_two_pos(start, end);
        painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(255, 220, 60, 24));
        painter.rect_stroke(r, 0.0, Stroke::new(1.5, yellow), StrokeKind::Inside);
    }
}

pub(super) fn draw_mirror_axis(
    ui: &mut Ui,
    app: &mut EditorApp,
    camera: Option<&Camera>,
    rect: Rect,
) {
    // Hide axes when a non-mirroring tool is active to avoid stale guides.
    if app.tool_state.mirror_mode == MirrorMode::None || !app.tool_state.active_tool.uses_mirror() {
        return;
    }
    let Some(camera) = camera else {
        return;
    };
    let Some(doc) = app.document.as_ref() else {
        return;
    };

    let map_w = doc.map.map_data.width as f32 * TILE_UNITS;
    let map_h = doc.map.map_data.height as f32 * TILE_UNITS;
    let cx = map_w * 0.5;
    let cz = map_h * 0.5;
    let vp = camera.view_projection_matrix();
    let painter = ui.painter_at(rect);
    let axis_color = Color32::from_rgba_unmultiplied(0, 220, 255, 160);
    let axis_stroke = Stroke::new(2.0, axis_color);

    let project_pt = |x: f32, z: f32| -> Option<Pos2> {
        let y = picking::sample_terrain_height_pub(&doc.map.map_data, x, z) + 30.0;
        project_world_to_screen(&vp, Vec3::new(x, y, z), rect)
    };

    // Short segments follow the terrain instead of cutting straight across.
    let draw_line = |x0: f32, z0: f32, x1: f32, z1: f32| {
        let segments = 16;
        for i in 0..segments {
            let t0 = i as f32 / segments as f32;
            let t1 = (i + 1) as f32 / segments as f32;
            let sx0 = x0 + (x1 - x0) * t0;
            let sz0 = z0 + (z1 - z0) * t0;
            let sx1 = x0 + (x1 - x0) * t1;
            let sz1 = z0 + (z1 - z0) * t1;
            if let (Some(a), Some(b)) = (project_pt(sx0, sz0), project_pt(sx1, sz1)) {
                painter.line_segment([a, b], axis_stroke);
            }
        }
    };

    match app.tool_state.mirror_mode {
        MirrorMode::None => {}
        MirrorMode::Vertical => {
            draw_line(cx, 0.0, cx, map_h);
        }
        MirrorMode::Horizontal => {
            draw_line(0.0, cz, map_w, cz);
        }
        MirrorMode::Both => {
            draw_line(cx, 0.0, cx, map_h);
            draw_line(0.0, cz, map_w, cz);
        }
        MirrorMode::Diagonal => {
            draw_line(0.0, 0.0, map_w, map_h);
            draw_line(map_w, 0.0, 0.0, map_h);
        }
    }
}

pub(super) fn draw_tool_buttons(ui: &mut Ui, app: &mut EditorApp, rect: Rect) {
    if app.document.is_none() {
        return;
    }

    let button_size = egui::vec2(24.0, 24.0);
    let margin = 8.0;
    let spacing = 4.0;

    let buttons: &[(&str, ToolId, &str)] = &[
        ("S", ToolId::ObjectSelect, "Select / Move (Esc)"),
        ("L", ToolId::ScriptLabel, "Script Labels"),
        ("G", ToolId::Gateway, "Gateways"),
    ];

    for (i, &(label, tool, tooltip)) in buttons.iter().enumerate() {
        let x_offset = (button_size.x + spacing) * i as f32;
        let btn_rect = Rect::from_min_size(
            egui::pos2(rect.left() + margin + x_offset, rect.top() + margin),
            button_size,
        );

        let is_active = app.tool_state.active_tool == tool;

        let bg = if is_active {
            Color32::from_rgba_unmultiplied(80, 140, 220, 200)
        } else {
            Color32::from_rgba_unmultiplied(30, 30, 30, 160)
        };

        let btn_response = ui.allocate_rect(btn_rect, Sense::click());
        let hovered = btn_response.hovered();

        let bg = if hovered && !is_active {
            Color32::from_rgba_unmultiplied(60, 60, 60, 190)
        } else {
            bg
        };

        let painter = ui.painter_at(rect);
        painter.rect_filled(btn_rect, 4.0, bg);
        painter.rect_stroke(
            btn_rect,
            4.0,
            Stroke::new(
                1.0,
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
            btn_rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(13.0),
            text_color,
        );

        if btn_response.clicked() {
            app.tool_state.active_tool = if is_active {
                ToolId::ObjectSelect
            } else {
                tool
            };
        }
        btn_response.on_hover_text(tooltip);
    }
}
