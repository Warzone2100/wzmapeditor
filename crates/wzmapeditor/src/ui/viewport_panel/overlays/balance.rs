use std::collections::HashMap;

use egui::{Color32, Mesh, Pos2, Rect, Shape, Stroke, Ui};

use crate::app::EditorApp;
use crate::map::document::MapDocument;
use crate::viewport::camera::Camera;
use crate::viewport::picking;

use wz_maplib::constants::TILE_UNITS_F32 as TILE_UNITS;

use super::project_world_to_screen;

const VORONOI_TINT_ALPHA: u8 = 38;

/// Tint a saturated color through `Rgba` so the alpha multiply happens in
/// linear space. egui 0.34's `Color32::from_rgba_unmultiplied` multiplies
/// gamma-encoded bytes by linear alpha, which collapses a wash like alpha=38
/// to a barely-visible premultiplied byte; the `Rgba` roundtrip preserves
/// the 0.29 brightness.
fn srgb_premultiply(color: Color32, alpha: u8) -> Color32 {
    let factor = f32::from(alpha) / 255.0;
    Color32::from(egui::Rgba::from(color).multiply(factor))
}

pub(super) fn draw(ui: &mut Ui, app: &mut EditorApp, camera: Option<&Camera>, rect: Rect) {
    let Some(camera) = camera else {
        return;
    };
    let Some(doc) = app.document.as_ref() else {
        return;
    };
    if app.balance.show_voronoi || app.balance.show_voronoi_tint {
        draw_voronoi_boundaries(ui, app, doc, camera, rect);
    }
    if !app.balance.highlighted_players.is_empty() {
        draw_balance_highlights(ui, app, doc, camera, rect);
    }
}

/// Per-player accumulator for the Voronoi fill mesh.
///
/// `vmap` is keyed on the linearised grid-corner index with `u32::MAX`
/// meaning "not yet emitted". The flat `Vec<u32>` replaced a `HashMap` keyed
/// on `(u32, u32)` corners that dropped framerate to single digits on large
/// maps.
struct VoronoiPlayerFill {
    mesh: Mesh,
    vmap: Vec<u32>,
    color: Color32,
}

/// Reuse an existing vertex by grid slot or emit a new one. Dedup keeps
/// adjacent tiles from drawing visible seams at the low alpha used here.
fn voronoi_intern_vertex(fill: &mut VoronoiPlayerFill, slot: usize, pos: Pos2) -> u32 {
    if fill.vmap[slot] == u32::MAX {
        let i =
            u32::try_from(fill.mesh.vertices.len()).expect("voronoi mesh vertex count fits in u32");
        fill.mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: egui::epaint::WHITE_UV,
            color: fill.color,
        });
        fill.vmap[slot] = i;
        i
    } else {
        fill.vmap[slot]
    }
}

fn draw_voronoi_boundaries(
    ui: &Ui,
    app: &EditorApp,
    doc: &MapDocument,
    camera: &Camera,
    rect: Rect,
) {
    let Some(report) = app.balance.report.as_ref() else {
        return;
    };
    if report.players.len() < 2 {
        return;
    }

    let map_data = &doc.map.map_data;
    let map_w = map_data.width;
    let map_h = map_data.height;
    if map_w == 0 || map_h == 0 {
        return;
    }

    let starts: Vec<(i8, i32, i32)> = report
        .players
        .iter()
        .map(|p| (p.player, p.start_tile.0 as i32, p.start_tile.1 as i32))
        .collect();

    let nearest = |x: i32, y: i32| -> i8 {
        let mut best: Option<(i32, i8)> = None;
        for &(id, sx, sy) in &starts {
            let dx = x - sx;
            let dy = y - sy;
            let d = dx * dx + dy * dy;
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, id));
            }
        }
        best.map_or(-1, |(_, id)| id)
    };

    let vp = camera.view_projection_matrix();
    let painter = ui.painter_at(rect);
    let project = |wx: f32, wz: f32| -> Option<Pos2> {
        let wy = picking::sample_terrain_height_pub(map_data, wx, wz) + 12.0;
        project_world_to_screen(&vp, glam::Vec3::new(wx, wy, wz), rect)
    };

    let tint = |pid: i8| srgb_premultiply(crate::balance::player_color(pid), VORONOI_TINT_ALPHA);

    // Shared source of truth for the fill and boundary loops.
    let all_rows: Vec<Vec<i8>> = (0..map_h)
        .map(|y| (0..map_w as i32).map(|x| nearest(x, y as i32)).collect())
        .collect();

    let want_fill = app.balance.show_voronoi_tint;
    let want_lines = app.balance.show_voronoi;

    if want_fill {
        let corner_w = (map_w + 1) as usize;
        let total_corners = corner_w * (map_h + 1) as usize;

        let mut grid: Vec<Option<Pos2>> = Vec::with_capacity(total_corners);
        for cy in 0..=map_h {
            for cx in 0..=map_w {
                let wx = cx as f32 * TILE_UNITS;
                let wz = cy as f32 * TILE_UNITS;
                grid.push(project(wx, wz));
            }
        }

        let mut fills: HashMap<i8, VoronoiPlayerFill> = HashMap::new();

        for y in 0..map_h {
            let row = &all_rows[y as usize];
            for x in 0..map_w {
                let pid = row[x as usize];
                let i00 = (y as usize) * corner_w + x as usize;
                let i10 = i00 + 1;
                let i01 = ((y + 1) as usize) * corner_w + x as usize;
                let i11 = i01 + 1;
                let (Some(p00), Some(p10), Some(p11), Some(p01)) =
                    (grid[i00], grid[i10], grid[i11], grid[i01])
                else {
                    continue;
                };
                let fill = fills.entry(pid).or_insert_with(|| VoronoiPlayerFill {
                    mesh: Mesh::default(),
                    vmap: vec![u32::MAX; total_corners],
                    color: tint(pid),
                });
                let v00 = voronoi_intern_vertex(fill, i00, p00);
                let v10 = voronoi_intern_vertex(fill, i10, p10);
                let v11 = voronoi_intern_vertex(fill, i11, p11);
                let v01 = voronoi_intern_vertex(fill, i01, p01);
                fill.mesh
                    .indices
                    .extend_from_slice(&[v00, v10, v11, v00, v11, v01]);
            }
        }

        for fill in fills.into_values() {
            painter.add(Shape::Mesh(fill.mesh.into()));
        }
    }

    if want_lines {
        let stroke = Stroke::new(1.5_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 200));
        for y in 0..map_h {
            let row = &all_rows[y as usize];
            for x in 0..map_w.saturating_sub(1) {
                if row[x as usize] != row[(x + 1) as usize] {
                    let wx = (x + 1) as f32 * TILE_UNITS;
                    let wz1 = y as f32 * TILE_UNITS;
                    let wz2 = (y + 1) as f32 * TILE_UNITS;
                    if let (Some(p1), Some(p2)) = (project(wx, wz1), project(wx, wz2)) {
                        painter.line_segment([p1, p2], stroke);
                    }
                }
            }
            if y + 1 < map_h {
                let row_b = &all_rows[(y + 1) as usize];
                for x in 0..map_w {
                    if row[x as usize] != row_b[x as usize] {
                        let wx1 = x as f32 * TILE_UNITS;
                        let wx2 = (x + 1) as f32 * TILE_UNITS;
                        let wz = (y + 1) as f32 * TILE_UNITS;
                        if let (Some(p1), Some(p2)) = (project(wx1, wz), project(wx2, wz)) {
                            painter.line_segment([p1, p2], stroke);
                        }
                    }
                }
            }
        }
    }
}

fn draw_balance_highlights(
    ui: &Ui,
    app: &EditorApp,
    doc: &MapDocument,
    camera: &Camera,
    rect: Rect,
) {
    let highlighted = &app.balance.highlighted_players;
    let map_data = &doc.map.map_data;
    let vp = camera.view_projection_matrix();
    let painter = ui.painter_at(rect);

    let project = |wx: f32, wz: f32, lift: f32| -> Option<Pos2> {
        let wy = picking::sample_terrain_height_pub(map_data, wx, wz) + lift;
        project_world_to_screen(&vp, glam::Vec3::new(wx, wy, wz), rect)
    };

    let outline = Color32::from_rgba_unmultiplied(0, 0, 0, 200);

    for s in &doc.map.structures {
        if !highlighted.contains(&s.player) {
            continue;
        }
        if let Some(p) = project(s.position.x as f32, s.position.y as f32, 14.0) {
            let color = crate::balance::player_color(s.player);
            painter.circle_filled(p, 4.0, fade(color, 200));
            painter.circle_stroke(p, 4.0, Stroke::new(1.0_f32, outline));
        }
    }

    for d in &doc.map.droids {
        if !highlighted.contains(&d.player) {
            continue;
        }
        if let Some(p) = project(d.position.x as f32, d.position.y as f32, 12.0) {
            let color = crate::balance::player_color(d.player);
            painter.circle_filled(p, 3.0, fade(color, 200));
            painter.circle_stroke(p, 3.0, Stroke::new(0.8_f32, outline));
        }
    }

    if let Some(report) = app.balance.report.as_ref() {
        for &((tx, ty), pid) in &report.oil_assignment {
            if !highlighted.contains(&pid) {
                continue;
            }
            let wx = (tx as f32 + 0.5) * TILE_UNITS;
            let wz = (ty as f32 + 0.5) * TILE_UNITS;
            if let Some(p) = project(wx, wz, 16.0) {
                let color = crate::balance::player_color(pid);
                painter.circle_stroke(p, 5.5, Stroke::new(1.8_f32, fade(color, 230)));
                painter.circle_stroke(p, 5.5, Stroke::new(0.8_f32, outline));
            }
        }
    }
}

fn fade(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}
