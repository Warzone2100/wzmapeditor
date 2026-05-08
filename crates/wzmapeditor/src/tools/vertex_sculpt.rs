//! Soft-selection vertex sculpt: drag a few selected vertices and have
//! the surrounding ring of vertices follow with Gaussian falloff.
//!
//! Heights live per tile in [`MapData`], where `tile(x, y).height` is the
//! top-left vertex of that tile. A "vertex" here is therefore the integer
//! pair `(vx, vy)` with `0 <= vx < W` and `0 <= vy < H` that maps directly
//! to that tile entry; right and bottom edge vertices share the last
//! interior tile and aren't separately editable.

use std::collections::HashMap;

use egui::{Pos2, Rect};

use wz_maplib::constants::TILE_UNITS_F32 as TILE_UNITS;
use wz_maplib::map_data::MapData;

use crate::map::history::EditCommand;
use crate::tools::MirrorMode;
use crate::tools::height_brush::{HeightEditCommand, clamp_height, gaussian_falloff};
use crate::tools::mirror::mirror_vertex_points;
use crate::tools::trait_def::{PointerInput, Tool, ToolCtx};
use crate::viewport::camera::Camera;
use crate::viewport::picking;

fn expand_with_mirrors(
    selected: &[(u32, u32)],
    map_w: u32,
    map_h: u32,
    mirror_mode: MirrorMode,
) -> Vec<(u32, u32)> {
    let mut out = Vec::with_capacity(selected.len());
    for &(vx, vy) in selected {
        for pt in mirror_vertex_points(vx, vy, map_w, map_h, mirror_mode) {
            if !out.contains(&pt) {
                out.push(pt);
            }
        }
    }
    out
}

/// Capture every vertex within `radius` of any selected vertex into the
/// snapshot, along with the matching tile heights at the start of a drag.
///
/// Selected vertices are always included with `falloff = 1.0`. Neighbours
/// receive a falloff weight based on their distance to the closest
/// selected vertex (taking the maximum across all selected vertices so a
/// vertex midway between two crease points gets the larger of the two
/// influences instead of being doubly weighted).
pub fn build_drag_snapshot(
    map: &MapData,
    selected: &[(u32, u32)],
    radius: u32,
) -> HashMap<(u32, u32), u16> {
    let mut snap = HashMap::new();
    if selected.is_empty() {
        return snap;
    }

    let r = radius as i64;
    for &(sx, sy) in selected {
        let cx = sx as i64;
        let cy = sy as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let vx = cx + dx;
                let vy = cy + dy;
                if vx < 0 || vy < 0 {
                    continue;
                }
                let vx = vx as u32;
                let vy = vy as u32;
                if vx >= map.width || vy >= map.height {
                    continue;
                }
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                if let std::collections::hash_map::Entry::Vacant(slot) = snap.entry((vx, vy))
                    && let Some(tile) = map.tile(vx, vy)
                {
                    slot.insert(tile.height);
                }
            }
        }
    }
    snap
}

/// Apply a soft-falloff height delta to the snapshot vertices.
///
/// Selected vertices receive the full delta. Neighbours receive
/// `delta * max gaussian_falloff(...)` over the selected set, which keeps
/// influence smooth even when several selected vertices sit close
/// together. Heights are written from the snapshot every call so calling
/// repeatedly within one drag stroke is idempotent.
pub fn apply_soft_drag(
    map: &mut MapData,
    selected: &[(u32, u32)],
    radius: u32,
    delta_world_y: f32,
    snapshot: &HashMap<(u32, u32), u16>,
) {
    if selected.is_empty() {
        return;
    }
    let selected_set: std::collections::HashSet<(u32, u32)> = selected.iter().copied().collect();

    for (&(vx, vy), &old_h) in snapshot {
        let weight = if selected_set.contains(&(vx, vy)) {
            1.0
        } else {
            let mut best: f32 = 0.0;
            for &(sx, sy) in selected {
                let dx = vx as i64 - sx as i64;
                let dy = vy as i64 - sy as i64;
                let f = gaussian_falloff(dx, dy, radius);
                if f > best {
                    best = f;
                }
            }
            best
        };

        let new_h = clamp_height(old_h as i32 + (delta_world_y * weight) as i32);
        if let Some(tile) = map.tile_mut(vx, vy) {
            tile.height = new_h;
        }
    }
}

/// Build a single reversible [`HeightEditCommand`] from the drag snapshot
/// and the map's current heights, returning `None` if nothing actually
/// moved (e.g. the user clicked the gizmo without dragging).
pub fn build_finalize_command(
    map: &MapData,
    snapshot: &HashMap<(u32, u32), u16>,
) -> Option<HeightEditCommand> {
    let mut affected = Vec::new();
    let mut old_heights = Vec::new();
    let mut new_heights = Vec::new();
    for (&(vx, vy), &old_h) in snapshot {
        let Some(tile) = map.tile(vx, vy) else {
            continue;
        };
        if tile.height != old_h {
            affected.push((vx, vy));
            old_heights.push(old_h);
            new_heights.push(tile.height);
        }
    }
    if affected.is_empty() {
        None
    } else {
        Some(HeightEditCommand {
            affected_tiles: affected,
            old_heights,
            new_heights,
        })
    }
}

/// Screen-space distance under which the cursor counts as "on" an
/// already-selected vertex. Drag-starts within this radius keep the
/// existing selection instead of replacing it with a fresh single-vertex
/// pick (so a multi-vertex group can be dragged as one).
const VERTEX_GRAB_RADIUS_PX: f32 = 12.0;

/// World-Y delta per pixel of vertical mouse motion. Two height units
/// per pixel keeps full-screen drags within the 0-65535 height range and
/// matches the feel of the height-brush tools.
const WORLD_Y_PER_DRAG_PX: f32 = 2.0;

/// Soft-selection vertex sculpt tool. Owns vertex selection, the soft
/// falloff radius, and the in-flight drag/marquee state that the
/// dispatcher previously kept on `ToolState`.
#[derive(Debug)]
pub(crate) struct VertexSculptTool {
    pub(crate) selected_vertices: Vec<(u32, u32)>,
    pub(crate) soft_select_radius: u32,
    drag_active: bool,
    drag_start_screen: Option<Pos2>,
    drag_height_snapshot: HashMap<(u32, u32), u16>,
    drag_selected_mirrored: Vec<(u32, u32)>,
    pub(crate) marquee_active: bool,
    pub(crate) marquee_start: Option<Pos2>,
    pub(crate) marquee_current: Option<Pos2>,
    marquee_additive: bool,
}

impl Default for VertexSculptTool {
    fn default() -> Self {
        Self {
            selected_vertices: Vec::new(),
            soft_select_radius: 3,
            drag_active: false,
            drag_start_screen: None,
            drag_height_snapshot: HashMap::new(),
            drag_selected_mirrored: Vec::new(),
            marquee_active: false,
            marquee_start: None,
            marquee_current: None,
            marquee_additive: false,
        }
    }
}

impl VertexSculptTool {
    /// Wipe selection plus drag state. Called when leaving the tool or
    /// when the user clicks the property-pane "Clear selection" button.
    pub(crate) fn clear(&mut self) {
        self.selected_vertices.clear();
        self.reset_transient();
    }

    fn reset_transient(&mut self) {
        self.drag_active = false;
        self.drag_start_screen = None;
        self.drag_height_snapshot.clear();
        self.drag_selected_mirrored.clear();
        self.marquee_active = false;
        self.marquee_start = None;
        self.marquee_current = None;
        self.marquee_additive = false;
    }

    fn handle_click(
        &mut self,
        response: &egui::Response,
        rect: Rect,
        camera: &Camera,
        map: &MapData,
    ) {
        let Some(hp) = response.hover_pos() else {
            return;
        };
        let Some(vertex) = pick_vertex(hp, rect, camera, map) else {
            return;
        };
        let shift = response.ctx.input(|i| i.modifiers.shift);
        if shift {
            if let Some(pos) = self.selected_vertices.iter().position(|v| *v == vertex) {
                self.selected_vertices.swap_remove(pos);
            } else {
                self.selected_vertices.push(vertex);
            }
        } else {
            self.selected_vertices.clear();
            self.selected_vertices.push(vertex);
        }
    }

    /// Ctrl-drag is the explicit marquee gesture. Without Ctrl, any drag
    /// is a height drag: it grabs the nearest vertex on the terrain
    /// (replacing or augmenting the selection per shift) and starts
    /// raising/lowering.
    fn try_start_drag_or_marquee(
        &mut self,
        response: &egui::Response,
        rect: Rect,
        camera: &Camera,
        map: &MapData,
        mirror_mode: MirrorMode,
    ) {
        let Some(hp) = response.hover_pos() else {
            return;
        };
        let (ctrl, shift) = response
            .ctx
            .input(|i| (i.modifiers.command, i.modifiers.shift));

        if ctrl {
            self.start_marquee(hp, shift);
            return;
        }

        if self.cursor_over_selected_vertex(hp, rect, camera, map) {
            self.start_height_drag(hp, map, mirror_mode);
            return;
        }

        if let Some(vertex) = pick_vertex(hp, rect, camera, map) {
            if !shift {
                self.selected_vertices.clear();
            }
            if !self.selected_vertices.contains(&vertex) {
                self.selected_vertices.push(vertex);
            }
            self.start_height_drag(hp, map, mirror_mode);
        }
    }

    fn start_height_drag(&mut self, hp: Pos2, map: &MapData, mirror_mode: MirrorMode) {
        let mirrored =
            expand_with_mirrors(&self.selected_vertices, map.width, map.height, mirror_mode);
        let snap = build_drag_snapshot(map, &mirrored, self.soft_select_radius);
        self.drag_active = true;
        self.drag_start_screen = Some(hp);
        self.drag_height_snapshot = snap;
        self.drag_selected_mirrored = mirrored;
    }

    fn start_marquee(&mut self, hp: Pos2, additive: bool) {
        self.marquee_active = true;
        self.marquee_start = Some(hp);
        self.marquee_current = Some(hp);
        self.marquee_additive = additive;
    }

    fn drive_active_drag(
        &mut self,
        ctx: &mut ToolCtx<'_>,
        response: &egui::Response,
    ) -> Option<Box<dyn EditCommand>> {
        let lmb_down = response
            .ctx
            .input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let cursor = response.ctx.input(|i| i.pointer.interact_pos());

        if !lmb_down {
            return self.finalize_drag(ctx);
        }

        let (Some(start), Some(now)) = (self.drag_start_screen, cursor) else {
            return None;
        };

        // Screen Y goes down, world +Y goes up: drag up -> raise.
        let delta_world_y = -(now.y - start.y) * WORLD_Y_PER_DRAG_PX;

        apply_soft_drag(
            &mut ctx.map.map_data,
            &self.drag_selected_mirrored,
            self.soft_select_radius,
            delta_world_y,
            &self.drag_height_snapshot,
        );
        ctx.mark_terrain_dirty();
        ctx.mark_minimap_dirty();
        None
    }

    fn finalize_drag(&mut self, ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        let snapshot = std::mem::take(&mut self.drag_height_snapshot);
        self.drag_selected_mirrored.clear();
        self.drag_active = false;
        self.drag_start_screen = None;

        if snapshot.is_empty() {
            return None;
        }
        build_finalize_command(&ctx.map.map_data, &snapshot)
            .map(|cmd| Box::new(cmd) as Box<dyn EditCommand>)
    }

    fn drive_active_marquee(
        &mut self,
        response: &egui::Response,
        rect: Rect,
        camera: &Camera,
        map: &MapData,
    ) {
        let lmb_down = response
            .ctx
            .input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let cursor = response.ctx.input(|i| i.pointer.interact_pos());

        if let Some(c) = cursor {
            self.marquee_current = Some(c);
        }

        if !lmb_down {
            self.finalize_marquee(rect, camera, map);
        }
    }

    fn finalize_marquee(&mut self, rect: Rect, camera: &Camera, map: &MapData) {
        let start = self.marquee_start.take();
        let end = self.marquee_current.take();
        let additive = self.marquee_additive;
        self.marquee_active = false;
        self.marquee_additive = false;

        let (Some(start), Some(end)) = (start, end) else {
            return;
        };

        let marquee = Rect::from_two_pos(start, end);
        if marquee.width() < 2.0 && marquee.height() < 2.0 {
            return;
        }

        let vp = camera.view_projection_matrix();
        let mut new_picks: Vec<(u32, u32)> = Vec::new();
        for vy in 0..map.height {
            for vx in 0..map.width {
                let h = map.tile(vx, vy).map_or(0.0, |t| f32::from(t.height));
                let world = glam::Vec3::new(vx as f32 * TILE_UNITS, h, vy as f32 * TILE_UNITS);
                let Some(p) = project_world(vp, world, rect) else {
                    continue;
                };
                if marquee.contains(p) {
                    new_picks.push((vx, vy));
                }
            }
        }

        if !additive {
            self.selected_vertices.clear();
        }
        for v in new_picks {
            if !self.selected_vertices.contains(&v) {
                self.selected_vertices.push(v);
            }
        }
    }

    fn cursor_over_selected_vertex(
        &self,
        cursor: Pos2,
        rect: Rect,
        camera: &Camera,
        map: &MapData,
    ) -> bool {
        let vp = camera.view_projection_matrix();
        let r_sq = VERTEX_GRAB_RADIUS_PX * VERTEX_GRAB_RADIUS_PX;
        for &(vx, vy) in &self.selected_vertices {
            let h = map.tile(vx, vy).map_or(0.0, |t| f32::from(t.height));
            let world = glam::Vec3::new(vx as f32 * TILE_UNITS, h, vy as f32 * TILE_UNITS);
            let Some(p) = project_world(vp, world, rect) else {
                continue;
            };
            if (p - cursor).length_sq() <= r_sq {
                return true;
            }
        }
        false
    }
}

/// Convert a cursor position to the nearest editable vertex on the
/// terrain under the cursor. Returns `None` if the ray misses terrain.
fn pick_vertex(cursor: Pos2, rect: Rect, camera: &Camera, map: &MapData) -> Option<(u32, u32)> {
    let (wx, wz) = picking::screen_to_world_pos(cursor, rect, camera, map)?;
    let vx = (wx / TILE_UNITS).round();
    let vy = (wz / TILE_UNITS).round();
    if vx < 0.0 || vy < 0.0 {
        return None;
    }
    let max_x = map.width.saturating_sub(1);
    let max_y = map.height.saturating_sub(1);
    Some(((vx as u32).min(max_x), (vy as u32).min(max_y)))
}

fn project_world(vp: glam::Mat4, world: glam::Vec3, rect: Rect) -> Option<Pos2> {
    let clip = vp * glam::Vec4::new(world.x, world.y, world.z, 1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = glam::Vec3::new(clip.x / clip.w, clip.y / clip.w, clip.z / clip.w);
    Some(Pos2::new(
        rect.left() + (ndc.x * 0.5 + 0.5) * rect.width(),
        rect.top() + (-ndc.y * 0.5 + 0.5) * rect.height(),
    ))
}

impl Tool for VertexSculptTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn on_deactivated(&mut self, _ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        self.clear();
        None
    }

    fn on_pointer_input(
        &mut self,
        ctx: &mut ToolCtx<'_>,
        input: PointerInput<'_>,
    ) -> Option<Box<dyn EditCommand>> {
        let PointerInput {
            response,
            rect,
            camera,
            ..
        } = input;

        // Update hovered tile from the cursor so overlays and the info
        // bar mirror what the dispatcher does for tile-based tools.
        *ctx.hovered_tile = response
            .hover_pos()
            .and_then(|hp| picking::screen_to_tile(hp, rect, camera, &ctx.map.map_data));

        if self.drag_active {
            return self.drive_active_drag(ctx, response);
        }

        if self.marquee_active {
            self.drive_active_marquee(response, rect, camera, &ctx.map.map_data);
            return None;
        }

        if response.drag_started_by(egui::PointerButton::Primary) {
            let mirror_mode = ctx.mirror_mode;
            self.try_start_drag_or_marquee(response, rect, camera, &ctx.map.map_data, mirror_mode);
            return None;
        }

        if response.clicked_by(egui::PointerButton::Primary) {
            self.handle_click(response, rect, camera, &ctx.map.map_data);
        }
        None
    }

    fn properties_ui(&mut self, ui: &mut egui::Ui, _ctx: &mut ToolCtx<'_>) {
        ui.heading("Vertex Sculpt");
        ui.add(
            egui::Slider::new(&mut self.soft_select_radius, 0..=12)
                .text("Soft Radius")
                .suffix(" tiles"),
        );
        ui.label(format!(
            "{} vertices selected",
            self.selected_vertices.len()
        ));
        if ui.button("Clear selection").clicked() {
            self.clear();
        }
        ui.label(
            egui::RichText::new(
                "Click a vertex to select, shift+click to add. Drag to raise/lower. Ctrl+drag to box-select (Ctrl+Shift+drag adds).",
            )
            .small()
            .weak(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_map(w: u32, h: u32, base: u16) -> MapData {
        let mut map = MapData::new(w, h);
        for tile in &mut map.tiles {
            tile.height = base;
        }
        map
    }

    #[test]
    fn snapshot_includes_neighbours_and_clamps_to_map() {
        let map = make_map(10, 10, 100);
        let snap = build_drag_snapshot(&map, &[(0, 0)], 2);
        // Quadrant only: (0..=2, 0..=2) inside the circle of radius 2.
        // Excludes (2,2) since dx^2+dy^2 = 8 > 4.
        assert!(snap.contains_key(&(0, 0)));
        assert!(snap.contains_key(&(2, 0)));
        assert!(snap.contains_key(&(0, 2)));
        assert!(!snap.contains_key(&(2, 2)));
        // No negative coords land in the snapshot.
        for &(vx, vy) in snap.keys() {
            assert!(vx < 10 && vy < 10);
        }
    }

    #[test]
    fn apply_soft_drag_centres_at_full_delta() {
        let mut map = make_map(11, 11, 200);
        let selected = vec![(5, 5)];
        let snap = build_drag_snapshot(&map, &selected, 3);
        apply_soft_drag(&mut map, &selected, 3, 50.0, &snap);

        // Centre vertex moves the full 50.
        assert_eq!(map.tile(5, 5).unwrap().height, 250);
        // Neighbour at distance 1 receives partial weight.
        let one_off = map.tile(6, 5).unwrap().height as i32 - 200;
        assert!(one_off > 0 && one_off < 50, "one_off={one_off}");
        // Neighbour at distance 3 (still in radius) but heavy falloff.
        let three_off = map.tile(8, 5).unwrap().height as i32 - 200;
        assert!(three_off >= 0 && three_off < one_off);
    }

    #[test]
    fn apply_soft_drag_leaves_outside_radius_untouched() {
        let mut map = make_map(11, 11, 200);
        let selected = vec![(5, 5)];
        let snap = build_drag_snapshot(&map, &selected, 2);
        apply_soft_drag(&mut map, &selected, 2, 80.0, &snap);
        // Vertex at (5, 9) is 4 tiles away: outside radius 2.
        assert_eq!(map.tile(5, 9).unwrap().height, 200);
    }

    #[test]
    fn redrag_from_snapshot_is_idempotent() {
        let mut map = make_map(11, 11, 300);
        let selected = vec![(5, 5)];
        let snap = build_drag_snapshot(&map, &selected, 3);
        apply_soft_drag(&mut map, &selected, 3, 40.0, &snap);
        let after_first = map.tile(5, 5).unwrap().height;
        // Re-applying with the same delta from the same snapshot must
        // produce the same result, not stack on top of itself.
        apply_soft_drag(&mut map, &selected, 3, 40.0, &snap);
        assert_eq!(map.tile(5, 5).unwrap().height, after_first);
    }

    #[test]
    fn finalize_command_round_trips_through_undo() {
        let mut wzmap = wz_maplib::WzMap {
            map_data: make_map(8, 8, 150),
            structures: Vec::new(),
            droids: Vec::new(),
            features: Vec::new(),
            terrain_types: None,
            labels: Vec::new(),
            map_name: String::new(),
            players: 0,
            tileset: String::new(),
            custom_templates_json: None,
        };
        let selected = vec![(4, 4)];
        let snap = build_drag_snapshot(&wzmap.map_data, &selected, 2);
        apply_soft_drag(&mut wzmap.map_data, &selected, 2, 60.0, &snap);

        let cmd = build_finalize_command(&wzmap.map_data, &snap)
            .expect("drag must produce non-empty command");
        // Saved heights should match what the map currently shows.
        for (i, &(vx, vy)) in cmd.affected_tiles.iter().enumerate() {
            assert_eq!(
                cmd.new_heights[i],
                wzmap.map_data.tile(vx, vy).unwrap().height
            );
        }

        cmd.undo(&mut wzmap);
        for (&(vx, vy), &h0) in &snap {
            assert_eq!(wzmap.map_data.tile(vx, vy).unwrap().height, h0);
        }

        cmd.execute(&mut wzmap);
        // Centre vertex back at the dragged height.
        assert!(wzmap.map_data.tile(4, 4).unwrap().height > 150);
    }

    #[test]
    fn finalize_returns_none_when_no_change() {
        let map = make_map(5, 5, 100);
        let snap = build_drag_snapshot(&map, &[(2, 2)], 1);
        // Snapshot captured but no drag applied: nothing changed.
        let cmd = build_finalize_command(&map, &snap);
        assert!(cmd.is_none());
    }

    use crate::map::history::EditHistory;
    use crate::tools::ToolId;
    use crate::tools::trait_def::DirtyFlags;

    #[test]
    fn vertex_sculpt_uses_mirror() {
        assert!(ToolId::VertexSculpt.uses_mirror());
    }

    #[test]
    fn expand_with_mirrors_dedups_on_axis() {
        let expanded = expand_with_mirrors(&[(5, 3)], 10, 10, MirrorMode::Vertical);
        assert_eq!(expanded, vec![(5, 3)]);
    }

    #[test]
    fn expand_with_mirrors_vertical_pairs() {
        let expanded = expand_with_mirrors(&[(2, 4)], 10, 10, MirrorMode::Vertical);
        assert_eq!(expanded, vec![(2, 4), (8, 4)]);
    }

    #[test]
    fn vertex_drag_mirrors_symmetrically() {
        let mut map = make_map(11, 11, 200);
        let selected = vec![(2, 5)];
        let expanded = expand_with_mirrors(&selected, map.width, map.height, MirrorMode::Vertical);
        assert_eq!(expanded, vec![(2, 5), (9, 5)]);

        let snap = build_drag_snapshot(&map, &expanded, 2);
        apply_soft_drag(&mut map, &expanded, 2, 60.0, &snap);

        let left = map.tile(2, 5).unwrap().height;
        let right = map.tile(9, 5).unwrap().height;
        assert_eq!(left, 260);
        assert_eq!(right, left, "mirrored vertex must move with the source");
    }

    #[test]
    fn vertex_drag_mirror_both_raises_four_corners() {
        let mut map = make_map(11, 11, 100);
        let expanded = expand_with_mirrors(&[(1, 2)], map.width, map.height, MirrorMode::Both);
        let snap = build_drag_snapshot(&map, &expanded, 1);
        apply_soft_drag(&mut map, &expanded, 1, 50.0, &snap);

        let h = |x, y| map.tile(x, y).unwrap().height;
        assert_eq!(h(1, 2), 150);
        assert_eq!(h(10, 2), 150);
        assert_eq!(h(1, 9), 150);
        assert_eq!(h(10, 9), 150);
    }

    #[test]
    fn vertex_sculpt_default_radius() {
        let tool = VertexSculptTool::default();
        assert_eq!(tool.soft_select_radius, 3);
        assert!(tool.selected_vertices.is_empty());
    }

    #[test]
    fn vertex_sculpt_clear_wipes_drag_and_selection() {
        let mut tool = VertexSculptTool {
            selected_vertices: vec![(1, 1), (2, 2)],
            drag_active: true,
            drag_start_screen: Some(Pos2::new(10.0, 20.0)),
            marquee_active: true,
            marquee_start: Some(Pos2::new(0.0, 0.0)),
            marquee_current: Some(Pos2::new(40.0, 40.0)),
            marquee_additive: true,
            ..Default::default()
        };
        tool.drag_height_snapshot.insert((1, 1), 100);
        tool.clear();
        assert!(tool.selected_vertices.is_empty());
        assert!(!tool.drag_active);
        assert!(tool.drag_start_screen.is_none());
        assert!(tool.drag_height_snapshot.is_empty());
        assert!(!tool.marquee_active);
        assert!(tool.marquee_start.is_none());
        assert!(tool.marquee_current.is_none());
        assert!(!tool.marquee_additive);
    }

    #[test]
    fn vertex_sculpt_drag_finalize_returns_command() {
        let mut wzmap = wz_maplib::WzMap::new("test", 8, 8);
        for tile in &mut wzmap.map_data.tiles {
            tile.height = 200;
        }
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let mut hovered_tile: Option<(u32, u32)> = None;
        let mut log_sink = |_msg: String| {};

        let mut tool = VertexSculptTool {
            selected_vertices: vec![(4, 4)],
            soft_select_radius: 2,
            drag_active: true,
            drag_start_screen: Some(Pos2::new(0.0, 0.0)),
            ..Default::default()
        };
        // Pre-populate the snapshot the same way start_height_drag would.
        tool.drag_height_snapshot =
            build_drag_snapshot(&wzmap.map_data, &tool.selected_vertices, 2);

        // Apply a delta directly so finalize_drag has something to commit.
        apply_soft_drag(
            &mut wzmap.map_data,
            &tool.selected_vertices,
            2,
            50.0,
            &tool.drag_height_snapshot,
        );

        let mut ctx = ToolCtx {
            map: &mut wzmap,
            history: &mut history,
            dirty: &mut dirty,
            stats: None,
            placement_player: 0,
            mirror_mode: MirrorMode::None,
            terrain_dirty_tiles: &mut dirty_tiles,
            stroke_active: &mut stroke_active,
            tile_pools: &[],
            log_sink: &mut log_sink,
            hovered_tile: &mut hovered_tile,
        };
        let cmd = tool.finalize_drag(&mut ctx);
        assert!(cmd.is_some(), "drag with applied delta should finalize");
        assert!(!tool.drag_active);
        assert!(tool.drag_start_screen.is_none());
    }

    #[test]
    fn vertex_sculpt_finalize_with_no_drag_returns_none() {
        let mut wzmap = wz_maplib::WzMap::new("test", 4, 4);
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let mut hovered_tile: Option<(u32, u32)> = None;
        let mut log_sink = |_msg: String| {};

        let mut tool = VertexSculptTool::default();
        let mut ctx = ToolCtx {
            map: &mut wzmap,
            history: &mut history,
            dirty: &mut dirty,
            stats: None,
            placement_player: 0,
            mirror_mode: MirrorMode::None,
            terrain_dirty_tiles: &mut dirty_tiles,
            stroke_active: &mut stroke_active,
            tile_pools: &[],
            log_sink: &mut log_sink,
            hovered_tile: &mut hovered_tile,
        };
        assert!(tool.finalize_drag(&mut ctx).is_none());
        assert!(tool.on_deactivated(&mut ctx).is_none());
    }
}
