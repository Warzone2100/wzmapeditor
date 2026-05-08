//! Unified height brush. One apply path covers raise, lower, smooth, and set.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use wz_maplib::WzMap;
use wz_maplib::constants::{TILE_MAX_HEIGHT, TILE_UNITS};
use wz_maplib::map_data::MapData;
use wz_maplib::objects::WorldPos;

use crate::map::history::EditCommand;
use crate::tools::HeightBrushMode;
use crate::tools::mirror;
use crate::tools::trait_def::{Tool, ToolCtx};

/// Multiplier applied to brush strength to produce per-frame height deltas.
const HEIGHT_RAISE_SCALE: f32 = 10.0;

/// Reversible height edit storing before/after heights per affected tile.
pub struct HeightEditCommand {
    pub affected_tiles: Vec<(u32, u32)>,
    pub old_heights: Vec<u16>,
    pub new_heights: Vec<u16>,
}

impl EditCommand for HeightEditCommand {
    fn execute(&self, map: &mut WzMap) {
        for (i, &(x, y)) in self.affected_tiles.iter().enumerate() {
            if let Some(tile) = map.map_data.tile_mut(x, y) {
                tile.height = self.new_heights[i];
            }
        }
    }

    fn undo(&self, map: &mut WzMap) {
        for (i, &(x, y)) in self.affected_tiles.iter().enumerate() {
            if let Some(tile) = map.map_data.tile_mut(x, y) {
                tile.height = self.old_heights[i];
            }
        }
    }

    fn dirties_objects(&self) -> bool {
        true
    }
}

/// Returns `(coords, old_heights)` for in-bounds tiles within a circular radius.
fn collect_tiles_in_radius(
    map: &MapData,
    center_x: u32,
    center_y: u32,
    radius: u32,
) -> (Vec<(u32, u32)>, Vec<u16>) {
    let mut coords = Vec::new();
    let mut old_heights = Vec::new();

    let r = radius as i64;
    let cx = center_x as i64;
    let cy = center_y as i64;
    let radius_sq = (r * r) as f64;

    for dy in -r..=r {
        for dx in -r..=r {
            let tx = cx + dx;
            let ty = cy + dy;
            if tx < 0 || ty < 0 {
                continue;
            }
            let tx = tx as u32;
            let ty = ty as u32;

            let dist_sq = (dx * dx + dy * dy) as f64;
            if dist_sq > radius_sq {
                continue;
            }

            if let Some(tile) = map.tile(tx, ty) {
                coords.push((tx, ty));
                old_heights.push(tile.height);
            }
        }
    }

    (coords, old_heights)
}

pub(crate) fn gaussian_falloff(dx: i64, dy: i64, radius: u32) -> f32 {
    if radius == 0 {
        return 1.0;
    }
    let dist_sq = (dx * dx + dy * dy) as f32;
    let sigma = radius as f32 * 0.5;
    let sigma_sq = sigma * sigma;
    (-dist_sq / (2.0 * sigma_sq)).exp()
}

pub(crate) fn clamp_height(val: i32) -> u16 {
    val.max(0).min(TILE_MAX_HEIGHT as i32) as u16
}

/// Apply the height brush in the given `mode` over a circular area.
///
/// `strength` applies to `Raise`/`Lower` only; `target` applies to `Set`
/// only. `Smooth` ignores both and blends each tile toward the average of
/// its 3x3 neighbourhood with a falloff-weighted ratio.
pub fn apply_height(
    map: &mut MapData,
    center_x: u32,
    center_y: u32,
    radius: u32,
    mode: HeightBrushMode,
    strength: f32,
    target: u16,
) -> HeightEditCommand {
    let (coords, old_heights) = collect_tiles_in_radius(map, center_x, center_y, radius);
    let mut new_heights = Vec::with_capacity(coords.len());

    let cx = center_x as i64;
    let cy = center_y as i64;
    let target_clamped = clamp_height(target as i32) as f32;

    for (i, &(x, y)) in coords.iter().enumerate() {
        let dx = x as i64 - cx;
        let dy = y as i64 - cy;
        let falloff = gaussian_falloff(dx, dy, radius);
        let current = old_heights[i] as f32;

        let new_h = match mode {
            HeightBrushMode::Raise => {
                let delta = (strength * falloff * HEIGHT_RAISE_SCALE) as i32;
                clamp_height(old_heights[i] as i32 + delta)
            }
            HeightBrushMode::Lower => {
                let delta = (strength * falloff * HEIGHT_RAISE_SCALE) as i32;
                clamp_height(old_heights[i] as i32 - delta)
            }
            HeightBrushMode::Smooth => {
                let mut sum: u32 = 0;
                let mut count: u32 = 0;
                for ny in -1i64..=1 {
                    for nx in -1i64..=1 {
                        let tx = x as i64 + nx;
                        let ty = y as i64 + ny;
                        if tx >= 0
                            && ty >= 0
                            && let Some(tile) = map.tile(tx as u32, ty as u32)
                        {
                            sum += tile.height as u32;
                            count += 1;
                        }
                    }
                }
                let avg = sum.checked_div(count).unwrap_or(0) as f32;
                let blended = current + (avg - current) * falloff * 0.5;
                clamp_height(blended as i32)
            }
            HeightBrushMode::Set => {
                let blended = current + (target_clamped - current) * falloff;
                clamp_height(blended as i32)
            }
        };

        new_heights.push(new_h);
    }

    for (i, &(x, y)) in coords.iter().enumerate() {
        if let Some(tile) = map.tile_mut(x, y) {
            tile.height = new_heights[i];
        }
    }

    HeightEditCommand {
        affected_tiles: coords,
        old_heights,
        new_heights,
    }
}

/// 16 ms re-fire keeps continuous-mode strength FPS-independent at ~60 Hz.
const CONTINUOUS_FIRE_INTERVAL: Duration = Duration::from_millis(16);

/// Stateful height brush. Owns mode + slider settings and the per-stroke
/// snapshot so undo packages an entire drag as one [`HeightEditCommand`].
#[derive(Debug)]
pub(crate) struct HeightBrushTool {
    pub(crate) mode: HeightBrushMode,
    pub(crate) brush_size: u32,
    pub(crate) brush_strength: f32,
    pub(crate) target_height: u16,
    /// Lazy first-touch snapshot so undo packages the whole drag as one step.
    snapshot: HashMap<(u32, u32), u16>,
    last_tile: Option<(u32, u32)>,
    last_fire_time: Option<Instant>,
}

impl Default for HeightBrushTool {
    fn default() -> Self {
        Self {
            mode: HeightBrushMode::Raise,
            brush_size: 0,
            brush_strength: 1.0,
            target_height: 128,
            snapshot: HashMap::new(),
            last_tile: None,
            last_fire_time: None,
        }
    }
}

impl HeightBrushTool {
    /// Raise/Lower/Smooth keep firing while the cursor is stationary.
    /// Set is idempotent so it only fires on tile change.
    pub(crate) fn is_continuous_mode(&self) -> bool {
        !matches!(self.mode, HeightBrushMode::Set)
    }

    /// Lazy first-touch snapshot so undo packages the whole drag as one step.
    fn snapshot_radius(&mut self, map: &MapData, cx: u32, cy: u32) {
        super::for_each_tile_in_radius(cx, cy, self.brush_size, map.width, map.height, |tx, ty| {
            self.snapshot
                .entry((tx, ty))
                .or_insert_with(|| map.tile(tx, ty).map_or(0, |t| t.height));
        });
    }

    fn fire(&mut self, ctx: &mut ToolCtx<'_>, tx: u32, ty: u32) {
        let map_w = ctx.map.map_data.width;
        let map_h = ctx.map.map_data.height;
        let pts = mirror::mirror_vertex_points(tx, ty, map_w, map_h, ctx.mirror_mode);

        for &(mx, my) in &pts {
            self.snapshot_radius(&ctx.map.map_data, mx, my);
            apply_height(
                &mut ctx.map.map_data,
                mx,
                my,
                self.brush_size,
                self.mode,
                self.brush_strength,
                self.target_height,
            );
            let r = self.brush_size;
            let xlo = mx.saturating_sub(r);
            let ylo = my.saturating_sub(r);
            let xhi = mx.saturating_add(r).min(map_w.saturating_sub(1));
            let yhi = my.saturating_add(r).min(map_h.saturating_sub(1));
            ctx.mark_terrain_rect_dirty(xlo, ylo, xhi, yhi);
        }

        ctx.mark_minimap_dirty();
        ctx.mark_objects_dirty();
        *ctx.stroke_active = true;
        self.last_tile = Some((tx, ty));
        self.last_fire_time = Some(Instant::now());
    }

    /// Build one [`HeightEditCommand`] from the snapshot delta. `None` if
    /// nothing changed. Forces a full rebuild so the lightmap/water/shadow
    /// cascade catches up in one pass.
    fn flush(&mut self, ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        let snapshot = std::mem::take(&mut self.snapshot);
        self.last_tile = None;
        self.last_fire_time = None;
        *ctx.stroke_active = false;

        if snapshot.is_empty() {
            return None;
        }

        let mut coords = Vec::with_capacity(snapshot.len());
        let mut old_heights = Vec::with_capacity(snapshot.len());
        let mut new_heights = Vec::with_capacity(snapshot.len());
        for ((x, y), old) in &snapshot {
            if let Some(tile) = ctx.map.map_data.tile(*x, *y)
                && tile.height != *old
            {
                coords.push((*x, *y));
                old_heights.push(*old);
                new_heights.push(tile.height);
            }
        }

        if coords.is_empty() {
            return None;
        }

        // Stroke-end cascade: shadow/water/lightmap/heatmap recompute in one pass.
        ctx.mark_terrain_dirty();
        ctx.mark_minimap_dirty();
        ctx.mark_objects_dirty();

        Some(Box::new(HeightEditCommand {
            affected_tiles: coords,
            old_heights,
            new_heights,
        }))
    }
}

fn world_pos_to_tile(pos: WorldPos) -> (u32, u32) {
    (pos.x / TILE_UNITS, pos.y / TILE_UNITS)
}

impl Tool for HeightBrushTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn on_mouse_press(&mut self, ctx: &mut ToolCtx<'_>, pos: WorldPos) {
        let (tx, ty) = world_pos_to_tile(pos);
        if tx >= ctx.map.map_data.width || ty >= ctx.map.map_data.height {
            return;
        }
        self.fire(ctx, tx, ty);
    }

    fn on_mouse_drag(&mut self, ctx: &mut ToolCtx<'_>, pos: WorldPos) {
        let (tx, ty) = world_pos_to_tile(pos);
        if tx >= ctx.map.map_data.width || ty >= ctx.map.map_data.height {
            return;
        }
        let tile_changed = self.last_tile != Some((tx, ty));
        let throttle_elapsed = self
            .last_fire_time
            .is_none_or(|t| t.elapsed() >= CONTINUOUS_FIRE_INTERVAL);
        let should_fire = tile_changed || (self.is_continuous_mode() && throttle_elapsed);
        if should_fire {
            self.fire(ctx, tx, ty);
        }
    }

    fn on_mouse_release(
        &mut self,
        ctx: &mut ToolCtx<'_>,
        _pos: Option<WorldPos>,
    ) -> Option<Box<dyn EditCommand>> {
        self.flush(ctx)
    }

    fn on_deactivated(&mut self, ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        self.flush(ctx)
    }

    fn brush_radius_tiles(&self) -> Option<u32> {
        Some(self.brush_size)
    }

    fn properties_ui(&mut self, ui: &mut egui::Ui, _ctx: &mut ToolCtx<'_>) {
        ui.heading("Sculpt Brush");
        ui.horizontal(|ui| {
            let modes: [(HeightBrushMode, &str, &str); 4] = [
                (HeightBrushMode::Raise, "Raise", "Raise terrain (1)"),
                (HeightBrushMode::Lower, "Lower", "Lower terrain (2)"),
                (HeightBrushMode::Smooth, "Smooth", "Smooth terrain (3)"),
                (HeightBrushMode::Set, "Set", "Set terrain height (4)"),
            ];
            for (mode, label, tooltip) in modes {
                let selected = self.mode == mode;
                let btn = egui::Button::new(label).selected(selected);
                if ui.add(btn).on_hover_text(tooltip).clicked() {
                    self.mode = mode;
                }
            }
        });
        ui.add(egui::Slider::new(&mut self.brush_size, 0..=20).text("Radius"));
        match self.mode {
            HeightBrushMode::Raise | HeightBrushMode::Lower => {
                ui.add(egui::Slider::new(&mut self.brush_strength, 0.1..=5.0).text("Strength"));
            }
            HeightBrushMode::Set => {
                ui.add(egui::Slider::new(&mut self.target_height, 0..=510).text("Target Height"));
            }
            HeightBrushMode::Smooth => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_map(width: u32, height: u32, initial_height: u16) -> MapData {
        let mut map = MapData::new(width, height);
        for tile in &mut map.tiles {
            tile.height = initial_height;
        }
        map
    }

    #[test]
    fn raise_increases_center() {
        let mut map = make_test_map(10, 10, 100);
        let cmd = apply_height(&mut map, 5, 5, 2, HeightBrushMode::Raise, 1.0, 0);
        assert!(!cmd.affected_tiles.is_empty());
        let center = map.tile(5, 5).unwrap();
        assert!(center.height > 100);
    }

    #[test]
    fn lower_decreases_center() {
        let mut map = make_test_map(10, 10, 200);
        let cmd = apply_height(&mut map, 5, 5, 2, HeightBrushMode::Lower, 1.0, 0);
        assert!(!cmd.affected_tiles.is_empty());
        let center = map.tile(5, 5).unwrap();
        assert!(center.height < 200);
    }

    #[test]
    fn raise_clamps_to_max() {
        let mut map = make_test_map(5, 5, TILE_MAX_HEIGHT);
        apply_height(&mut map, 2, 2, 1, HeightBrushMode::Raise, 5.0, 0);
        let center = map.tile(2, 2).unwrap();
        assert_eq!(center.height, TILE_MAX_HEIGHT);
    }

    #[test]
    fn lower_clamps_to_zero() {
        let mut map = make_test_map(5, 5, 0);
        apply_height(&mut map, 2, 2, 1, HeightBrushMode::Lower, 5.0, 0);
        let center = map.tile(2, 2).unwrap();
        assert_eq!(center.height, 0);
    }

    #[test]
    fn set_height() {
        let mut map = make_test_map(10, 10, 100);
        apply_height(&mut map, 5, 5, 0, HeightBrushMode::Set, 0.0, 300);
        let center = map.tile(5, 5).unwrap();
        assert_eq!(center.height, 300);
    }

    #[test]
    fn smooth_reduces_spike() {
        let mut map = make_test_map(10, 10, 100);
        map.tile_mut(5, 5).unwrap().height = 400;
        apply_height(&mut map, 5, 5, 1, HeightBrushMode::Smooth, 0.0, 0);
        let center = map.tile(5, 5).unwrap();
        assert!(center.height < 400);
    }

    #[test]
    fn raise_command_stores_undo_data() {
        let mut map = make_test_map(5, 5, 100);
        let cmd = apply_height(&mut map, 2, 2, 0, HeightBrushMode::Raise, 1.0, 0);
        assert_eq!(cmd.affected_tiles.len(), 1);
        assert_eq!(cmd.old_heights[0], 100);
        assert!(cmd.new_heights[0] > 100);
        assert_eq!(map.tile(2, 2).unwrap().height, cmd.new_heights[0]);
    }

    #[test]
    fn raise_gaussian_falloff_reduces_edges() {
        let mut map = make_test_map(10, 10, 100);
        let cmd = apply_height(&mut map, 5, 5, 2, HeightBrushMode::Raise, 1.0, 0);
        let center_idx = cmd
            .affected_tiles
            .iter()
            .position(|&c| c == (5, 5))
            .unwrap();
        let center_delta = cmd.new_heights[center_idx] as i32 - cmd.old_heights[center_idx] as i32;

        if let Some(edge_idx) = cmd.affected_tiles.iter().position(|&c| c == (5, 3)) {
            let edge_delta = cmd.new_heights[edge_idx] as i32 - cmd.old_heights[edge_idx] as i32;
            assert!(
                center_delta > edge_delta,
                "center delta ({center_delta}) should exceed edge delta ({edge_delta})"
            );
        }
    }

    use crate::map::history::EditHistory;
    use crate::tools::MirrorMode;
    use crate::tools::trait_def::DirtyFlags;

    fn world_pos_for(tile: (u32, u32)) -> WorldPos {
        WorldPos {
            x: tile.0 * TILE_UNITS + TILE_UNITS / 2,
            y: tile.1 * TILE_UNITS + TILE_UNITS / 2,
        }
    }

    #[test]
    fn height_brush_press_release_raises_and_returns_command() {
        let mut map = WzMap::new("test", 10, 10);
        for tile in &mut map.map_data.tiles {
            tile.height = 100;
        }
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let mut tool = HeightBrushTool::default();

        let mut hovered_tile: Option<(u32, u32)> = None;
        let mut log_sink = |_msg: String| {};
        let mut ctx = ToolCtx {
            map: &mut map,
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

        tool.on_mouse_press(&mut ctx, world_pos_for((4, 4)));
        assert!(*ctx.stroke_active, "press should mark stroke active");
        assert!(
            ctx.map.map_data.tile(4, 4).unwrap().height > 100,
            "raise should increase height at the brush center"
        );
        assert!(
            !ctx.terrain_dirty_tiles.is_empty(),
            "fire should mark partial dirty tiles"
        );

        let cmd = tool.on_mouse_release(&mut ctx, None);
        assert!(cmd.is_some(), "release should return a HeightEditCommand");
        assert!(!*ctx.stroke_active, "release should clear stroke active");
        assert!(dirty.terrain, "release should mark terrain dirty");
        assert!(dirty.minimap, "release should mark minimap dirty");
        assert!(
            dirty.objects,
            "height edits must refresh object instance buffers"
        );
        let cmd = cmd.unwrap();
        assert!(
            cmd.dirties_objects(),
            "HeightEditCommand must report it dirties objects so undo/redo can refresh them"
        );
    }

    #[test]
    fn height_brush_release_with_no_press_returns_none() {
        let mut map = WzMap::new("test", 4, 4);
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let mut tool = HeightBrushTool::default();
        let mut hovered_tile: Option<(u32, u32)> = None;
        let mut log_sink = |_msg: String| {};
        let mut ctx = ToolCtx {
            map: &mut map,
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
        assert!(tool.on_mouse_release(&mut ctx, None).is_none());
        assert!(tool.on_deactivated(&mut ctx).is_none());
    }

    #[test]
    fn height_brush_set_mode_writes_target_height() {
        let mut map = WzMap::new("test", 5, 5);
        for tile in &mut map.map_data.tiles {
            tile.height = 50;
        }
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let mut tool = HeightBrushTool {
            mode: HeightBrushMode::Set,
            target_height: 300,
            ..Default::default()
        };
        let mut hovered_tile: Option<(u32, u32)> = None;
        let mut log_sink = |_msg: String| {};
        let mut ctx = ToolCtx {
            map: &mut map,
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
        tool.on_mouse_press(&mut ctx, world_pos_for((2, 2)));
        assert_eq!(
            ctx.map.map_data.tile(2, 2).unwrap().height,
            300,
            "Set mode should write target_height at the brush center"
        );
        let cmd = tool.on_mouse_release(&mut ctx, None);
        assert!(cmd.is_some());
    }
}
