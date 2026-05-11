//! Texture painting tool for terrain tiles.

use std::collections::HashMap;

use wz_maplib::WzMap;
use wz_maplib::constants::TILE_UNITS;
use wz_maplib::map_data::{MapData, MapTile};
use wz_maplib::objects::WorldPos;

use crate::map::history::EditCommand;
use crate::tools::line_mode::{self, LineModeState};
use crate::tools::mirror;
use crate::tools::trait_def::{PointerInput, Tool, ToolCtx, default_world_pos_dispatch};

/// A reversible texture painting command.
pub struct TexturePaintCommand {
    pub affected_tiles: Vec<(u32, u32)>,
    pub old_textures: Vec<u16>,
    pub new_textures: Vec<u16>,
}

impl EditCommand for TexturePaintCommand {
    fn execute(&self, map: &mut WzMap) {
        for (i, &(x, y)) in self.affected_tiles.iter().enumerate() {
            if let Some(tile) = map.map_data.tile_mut(x, y) {
                tile.texture = self.new_textures[i];
            }
        }
    }

    fn undo(&self, map: &mut WzMap) {
        for (i, &(x, y)) in self.affected_tiles.iter().enumerate() {
            if let Some(tile) = map.map_data.tile_mut(x, y) {
                tile.texture = self.old_textures[i];
            }
        }
    }
}

/// Orientation options for texture painting.
pub struct PaintOrientation {
    /// Whether to change the texture index.
    pub set_texture: bool,
    /// Whether to change the tile orientation.
    pub set_orientation: bool,
    /// Rotation (0-3).
    pub rotation: u8,
    /// X flip.
    pub x_flip: bool,
    /// Y flip (WZ2100 flipZ).
    pub y_flip: bool,
    /// Randomize orientation per tile.
    pub randomize: bool,
}

impl Default for PaintOrientation {
    fn default() -> Self {
        Self {
            set_texture: true,
            set_orientation: true,
            rotation: 0,
            x_flip: false,
            y_flip: false,
            randomize: false,
        }
    }
}

/// Paint a texture with default orientation (no rotation/flip).
#[cfg(test)]
pub fn apply_texture_paint(
    map: &mut MapData,
    center_x: u32,
    center_y: u32,
    radius: u32,
    texture_id: u16,
) -> TexturePaintCommand {
    apply_texture_paint_oriented(
        map,
        center_x,
        center_y,
        radius,
        texture_id,
        &PaintOrientation::default(),
    )
}

/// Paint with full orientation control.
pub fn apply_texture_paint_oriented(
    map: &mut MapData,
    center_x: u32,
    center_y: u32,
    radius: u32,
    texture_id: u16,
    orient: &PaintOrientation,
) -> TexturePaintCommand {
    let mut coords = Vec::new();
    let mut old_textures = Vec::new();
    let mut new_textures = Vec::new();

    super::for_each_tile_in_radius(
        center_x,
        center_y,
        radius,
        map.width,
        map.height,
        |tx, ty| {
            if let Some(tile) = map.tile(tx, ty) {
                let old_tex = tile.texture;

                let existing_id = tile.texture_id();
                let existing_rot = tile.rotation();
                let existing_xf = tile.x_flip();
                let existing_yf = tile.y_flip();
                let existing_tri = tile.tri_flip();

                let new_id = if orient.set_texture {
                    texture_id
                } else {
                    existing_id
                };

                let (new_rot, new_xf, new_yf) = if orient.set_orientation {
                    if orient.randomize {
                        random_orientation()
                    } else {
                        (orient.rotation, orient.x_flip, orient.y_flip)
                    }
                } else {
                    (existing_rot, existing_xf, existing_yf)
                };

                let new_tex = MapTile::make_texture(new_id, new_xf, new_yf, new_rot, existing_tri);

                coords.push((tx, ty));
                old_textures.push(old_tex);
                new_textures.push(new_tex);
            }
        },
    );

    for (i, &(x, y)) in coords.iter().enumerate() {
        if let Some(tile) = map.tile_mut(x, y) {
            tile.texture = new_textures[i];
        }
    }

    TexturePaintCommand {
        affected_tiles: coords,
        old_textures,
        new_textures,
    }
}

/// Generate a random orientation (rotation 0-3, random xflip/yflip).
fn random_orientation() -> (u8, bool, bool) {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut hasher);
    let h = hasher.finish();
    let rot = (h & 3) as u8;
    let xf = (h >> 2) & 1 == 1;
    let yf = (h >> 3) & 1 == 1;
    (rot, xf, yf)
}

/// Idempotent texture brush. Owns brush settings + per-stroke snapshot.
#[derive(Debug)]
pub(crate) struct TexturePaintTool {
    pub(crate) brush_size: u32,
    pub(crate) selected_texture: u16,
    pub(crate) set_texture: bool,
    pub(crate) set_orientation: bool,
    pub(crate) tile_rotation: u8,
    pub(crate) tile_x_flip: bool,
    pub(crate) tile_y_flip: bool,
    pub(crate) randomize_orientation: bool,
    /// First-touch snapshot of original packed `tile.texture` values, keyed
    /// by tile coords. Populated lazily so undo packages the whole drag as
    /// one [`TexturePaintCommand`].
    snapshot: HashMap<(u32, u32), u16>,
    last_tile: Option<(u32, u32)>,
    line: LineModeState,
}

impl Default for TexturePaintTool {
    fn default() -> Self {
        Self {
            brush_size: 0,
            selected_texture: 0,
            set_texture: true,
            set_orientation: true,
            tile_rotation: 0,
            tile_x_flip: false,
            tile_y_flip: false,
            randomize_orientation: false,
            snapshot: HashMap::new(),
            last_tile: None,
            line: LineModeState::default(),
        }
    }
}

impl TexturePaintTool {
    /// Snapshot every tile within the brush radius of `(cx, cy)` that
    /// has not been touched yet this stroke.
    fn snapshot_radius(&mut self, map: &MapData, cx: u32, cy: u32) {
        super::for_each_tile_in_radius(cx, cy, self.brush_size, map.width, map.height, |tx, ty| {
            self.snapshot
                .entry((tx, ty))
                .or_insert_with(|| map.tile(tx, ty).map_or(0, |t| t.texture));
        });
    }

    /// Apply the brush at `(tx, ty)` plus every mirrored counterpart.
    /// Snapshots first-touch tiles, runs the oriented paint, marks the
    /// touched rect dirty per mirror point, and updates `last_tile`.
    fn fire(&mut self, ctx: &mut ToolCtx<'_>, tx: u32, ty: u32) {
        let map_w = ctx.map.map_data.width;
        let map_h = ctx.map.map_data.height;
        let pts = mirror::mirror_points(tx, ty, map_w, map_h, ctx.mirror_mode);

        for (point_idx, &(mx, my)) in pts.iter().enumerate() {
            let (m_rot, m_xf, m_yf) = mirror::mirror_orientation(
                self.tile_rotation,
                self.tile_x_flip,
                self.tile_y_flip,
                ctx.mirror_mode,
                point_idx,
            );
            self.snapshot_radius(&ctx.map.map_data, mx, my);
            let orient = PaintOrientation {
                set_texture: self.set_texture,
                set_orientation: self.set_orientation,
                rotation: m_rot,
                x_flip: m_xf,
                y_flip: m_yf,
                randomize: self.randomize_orientation,
            };
            apply_texture_paint_oriented(
                &mut ctx.map.map_data,
                mx,
                my,
                self.brush_size,
                self.selected_texture,
                &orient,
            );
            let r = self.brush_size;
            let xlo = mx.saturating_sub(r);
            let ylo = my.saturating_sub(r);
            let xhi = mx.saturating_add(r).min(map_w.saturating_sub(1));
            let yhi = my.saturating_add(r).min(map_h.saturating_sub(1));
            ctx.mark_terrain_rect_dirty(xlo, ylo, xhi, yhi);
        }

        ctx.mark_minimap_dirty();
        *ctx.stroke_active = true;
        self.last_tile = Some((tx, ty));
    }

    /// Build a single [`TexturePaintCommand`] from the snapshot delta and
    /// reset stroke state. Returns `None` if no tile actually changed.
    /// Forces a full rebuild on release so the cached water-depth field
    /// and the lightmap / shadow / water cascade pick up the final stroke
    /// state in one pass.
    fn flush(&mut self, ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        let snapshot = std::mem::take(&mut self.snapshot);
        self.last_tile = None;
        *ctx.stroke_active = false;

        if snapshot.is_empty() {
            return None;
        }

        let mut coords = Vec::with_capacity(snapshot.len());
        let mut old_textures = Vec::with_capacity(snapshot.len());
        let mut new_textures = Vec::with_capacity(snapshot.len());
        for ((x, y), old) in &snapshot {
            if let Some(tile) = ctx.map.map_data.tile(*x, *y)
                && tile.texture != *old
            {
                coords.push((*x, *y));
                old_textures.push(*old);
                new_textures.push(tile.texture);
            }
        }

        if coords.is_empty() {
            return None;
        }

        // Texture changes can flip a tile's water status; the partial path
        // can't reflect that since the cached blurred water-depth field is
        // global. Force one full rebuild on stroke end.
        ctx.mark_terrain_dirty();
        ctx.mark_minimap_dirty();

        Some(Box::new(TexturePaintCommand {
            affected_tiles: coords,
            old_textures,
            new_textures,
        }))
    }

    /// Rotate the current orientation 90 degrees clockwise.
    pub(crate) fn rotate_cw(&mut self) {
        let (sa, mut xf, mut yf) =
            mirror::rot_to_sa(self.tile_rotation, self.tile_x_flip, self.tile_y_flip);
        let new_sa = !sa;
        if xf ^ yf {
            yf = !yf;
        } else {
            xf = !xf;
        }
        let (r, x, y) = mirror::sa_to_rot(new_sa, xf, yf);
        self.tile_rotation = r;
        self.tile_x_flip = x;
        self.tile_y_flip = y;
    }

    /// Rotate the current orientation 90 degrees counter-clockwise.
    pub(crate) fn rotate_ccw(&mut self) {
        let (sa, mut xf, mut yf) =
            mirror::rot_to_sa(self.tile_rotation, self.tile_x_flip, self.tile_y_flip);
        let new_sa = !sa;
        if xf ^ yf {
            xf = !xf;
        } else {
            yf = !yf;
        }
        let (r, x, y) = mirror::sa_to_rot(new_sa, xf, yf);
        self.tile_rotation = r;
        self.tile_x_flip = x;
        self.tile_y_flip = y;
    }

    fn commit_line_stroke(
        &mut self,
        ctx: &mut ToolCtx<'_>,
        start: (u32, u32),
        end: (u32, u32),
    ) -> Option<Box<dyn EditCommand>> {
        let w = ctx.map.map_data.width;
        let h = ctx.map.map_data.height;
        for (tx, ty) in line_mode::bresenham_tiles(start, end) {
            if tx < w && ty < h {
                self.fire(ctx, tx, ty);
            }
        }
        self.flush(ctx)
    }

    /// Flip the current orientation across the X axis.
    pub(crate) fn flip_x(&mut self) {
        let (sa, mut xf, mut yf) =
            mirror::rot_to_sa(self.tile_rotation, self.tile_x_flip, self.tile_y_flip);
        if sa {
            yf = !yf;
        } else {
            xf = !xf;
        }
        let (r, x, y) = mirror::sa_to_rot(sa, xf, yf);
        self.tile_rotation = r;
        self.tile_x_flip = x;
        self.tile_y_flip = y;
    }
}

fn world_pos_to_tile(pos: WorldPos) -> (u32, u32) {
    (pos.x / TILE_UNITS, pos.y / TILE_UNITS)
}

impl Tool for TexturePaintTool {
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
        // Idempotent brush: same tile + same settings paints the same
        // pixels, so only fire when the cursor crosses into a new tile.
        if self.last_tile == Some((tx, ty)) {
            return;
        }
        self.fire(ctx, tx, ty);
    }

    fn on_mouse_release(
        &mut self,
        ctx: &mut ToolCtx<'_>,
        _pos: Option<WorldPos>,
    ) -> Option<Box<dyn EditCommand>> {
        self.flush(ctx)
    }

    fn on_deactivated(&mut self, ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        self.line.clear();
        self.flush(ctx)
    }

    fn on_pointer_input(
        &mut self,
        ctx: &mut ToolCtx<'_>,
        input: PointerInput<'_>,
    ) -> Option<Box<dyn EditCommand>> {
        let cursor_tile = input.response.hover_pos().and_then(|p| {
            crate::viewport::picking::screen_to_tile(p, input.rect, input.camera, &ctx.map.map_data)
        });

        if self.line.armed() {
            self.line.hover = cursor_tile;
        }

        let press_edge = input.response.drag_started_by(egui::PointerButton::Primary)
            || input.response.clicked_by(egui::PointerButton::Primary);

        if press_edge && let Some(tile) = cursor_tile {
            if let Some(start) = self.line.start {
                self.line.clear();
                return self.commit_line_stroke(ctx, start, tile);
            }
            let shift = input.response.ctx.input(|i| i.modifiers.shift);
            if shift {
                self.line.start = Some(tile);
                self.line.hover = Some(tile);
                return None;
            }
        }

        if self.line.armed() {
            return None;
        }

        default_world_pos_dispatch(self, ctx, input)
    }

    fn on_secondary_click(&mut self, _ctx: &mut ToolCtx<'_>) -> bool {
        if self.line.armed() {
            self.line.clear();
            true
        } else {
            false
        }
    }

    fn on_cancel(&mut self, _ctx: &mut ToolCtx<'_>) {
        self.line.clear();
    }

    fn line_mode_state(&self) -> Option<&LineModeState> {
        Some(&self.line)
    }

    fn brush_radius_tiles(&self) -> Option<u32> {
        Some(self.brush_size)
    }

    fn properties_ui(&mut self, ui: &mut egui::Ui, _ctx: &mut ToolCtx<'_>) {
        ui.heading("Texture Paint");
        ui.add(egui::Slider::new(&mut self.brush_size, 0..=20).text("Radius"));
        ui.label(format!("Selected Texture: {}", self.selected_texture));

        ui.separator();
        ui.checkbox(&mut self.set_texture, "Set Texture");
        ui.checkbox(&mut self.set_orientation, "Set Orientation");

        if self.set_orientation {
            ui.horizontal(|ui| {
                if ui.button("\u{21BA}").on_hover_text("Rotate CCW").clicked() {
                    self.rotate_ccw();
                }
                if ui.button("\u{21BB}").on_hover_text("Rotate CW").clicked() {
                    self.rotate_cw();
                }
                if ui.button("\u{2194}").on_hover_text("Flip X").clicked() {
                    self.flip_x();
                }
            });
            ui.checkbox(&mut self.randomize_orientation, "Randomize");

            let rot_deg = self.tile_rotation as u16 * 90;
            let mut parts = vec![format!("{}°", rot_deg)];
            if self.tile_x_flip {
                parts.push("FlipX".into());
            }
            if self.tile_y_flip {
                parts.push("FlipY".into());
            }
            ui.label(egui::RichText::new(parts.join(" ")).small().weak());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paint_single_tile() {
        let mut map = MapData::new(10, 10);
        let cmd = apply_texture_paint(&mut map, 5, 5, 0, 42);
        assert_eq!(cmd.affected_tiles.len(), 1);
        assert_eq!(map.tile(5, 5).unwrap().texture_id(), 42);
    }

    #[test]
    fn test_paint_square_radius() {
        let mut map = MapData::new(10, 10);
        let cmd = apply_texture_paint(&mut map, 5, 5, 1, 7);
        // Square brush: radius 1 = 3×3 = 9 tiles
        assert_eq!(cmd.affected_tiles.len(), 9);
        for &(x, y) in &cmd.affected_tiles {
            assert_eq!(map.tile(x, y).unwrap().texture_id(), 7);
        }
    }

    #[test]
    fn test_paint_preserves_old_texture() {
        let mut map = MapData::new(10, 10);
        // Set a known texture first
        map.tile_mut(5, 5).unwrap().texture = MapTile::make_texture(10, false, false, 0, false);
        let cmd = apply_texture_paint(&mut map, 5, 5, 0, 20);
        assert_eq!(cmd.old_textures[0] & 0x01ff, 10);
        assert_eq!(cmd.new_textures[0] & 0x01ff, 20);
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
    fn texture_paint_tool_press_release_paints_and_returns_command() {
        let mut map = WzMap::new("test", 8, 8);
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let mut tool = TexturePaintTool {
            selected_texture: 42,
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

        tool.on_mouse_press(&mut ctx, world_pos_for((3, 3)));
        assert!(*ctx.stroke_active, "press should mark stroke active");
        assert_eq!(
            ctx.map.map_data.tile(3, 3).unwrap().texture_id(),
            42,
            "press should paint the selected texture id"
        );
        assert!(
            !ctx.terrain_dirty_tiles.is_empty(),
            "fire should mark partial dirty tiles"
        );

        let cmd = tool.on_mouse_release(&mut ctx, None);
        assert!(cmd.is_some(), "release should return a TexturePaintCommand");
        assert!(!*ctx.stroke_active, "release should clear stroke active");
        assert!(dirty.terrain, "release should mark terrain dirty");
        assert!(dirty.minimap, "release should mark minimap dirty");
    }

    #[test]
    fn texture_paint_tool_release_with_no_press_returns_none() {
        let mut map = WzMap::new("test", 4, 4);
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let mut tool = TexturePaintTool::default();
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
    fn texture_paint_tool_repeat_press_same_tile_only_one_undo_entry() {
        let mut map = WzMap::new("test", 6, 6);
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let mut tool = TexturePaintTool {
            selected_texture: 5,
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
        // Drag to the same tile should be a no-op (idempotent gating);
        // last_tile gates the redundant fire.
        let dirty_before = ctx.terrain_dirty_tiles.len();
        tool.on_mouse_drag(&mut ctx, world_pos_for((2, 2)));
        assert_eq!(
            ctx.terrain_dirty_tiles.len(),
            dirty_before,
            "drag onto same tile should not refire the brush"
        );
        let cmd = tool.on_mouse_release(&mut ctx, None);
        assert!(cmd.is_some(), "stroke produced a command");
        assert_eq!(
            ctx.map.map_data.tile(2, 2).unwrap().texture_id(),
            5,
            "tile should hold the painted texture id"
        );
    }
}
