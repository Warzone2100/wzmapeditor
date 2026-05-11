//! Ground type brush with weighted random tile selection.

use std::collections::HashMap;

use wz_maplib::constants::TILE_UNITS;
use wz_maplib::map_data::{MapData, MapTile};
use wz_maplib::objects::WorldPos;

use crate::map::history::EditCommand;
use crate::tools::line_mode::{self, LineModeState};
use crate::tools::mirror;
use crate::tools::texture_paint::TexturePaintCommand;
use crate::tools::trait_def::{PointerInput, Tool, ToolCtx, default_world_pos_dispatch};

/// A pool of tile indices for one ground type, grouped by terrain type.
#[derive(Debug, Clone)]
pub struct TilePool {
    /// Ground type index into `GroundData::ground_types`.
    pub ground_type_index: u8,
    pub name: String,
    pub color: [u8; 3],
    pub buckets: Vec<TileBucket>,
}

impl TilePool {
    pub fn total_weight(&self) -> u32 {
        self.buckets.iter().map(|b| b.weight).sum()
    }
}

/// Tiles of a single terrain type within a ground type pool.
#[derive(Debug, Clone)]
pub struct TileBucket {
    /// Tile indices that are solid (all 4 corners = this ground type) with this terrain type.
    pub tile_ids: Vec<u16>,
    /// Weight for random selection. 0 disables the bucket.
    pub weight: u32,
}

/// Select a random tile from the pool based on bucket weights.
///
/// `rng` and `total_weight` are passed in to avoid re-creation and
/// re-computation on every call during a brush stroke. Returns
/// `(tile_id, rotation, x_flip, y_flip)`, or `None` if `total_weight` is
/// zero or the pool has no tiles.
pub fn pick_random_tile(
    pool: &TilePool,
    rng: &mut fastrand::Rng,
    total_weight: u32,
) -> Option<(u16, u8, bool, bool)> {
    if total_weight == 0 {
        return None;
    }

    let mut roll = rng.u32(0..total_weight);

    for bucket in &pool.buckets {
        if bucket.weight == 0 || bucket.tile_ids.is_empty() {
            continue;
        }
        if roll < bucket.weight {
            let idx = rng.usize(..bucket.tile_ids.len());
            let tile_id = bucket.tile_ids[idx];
            let rot = rng.u8(..4);
            let xf = rng.bool();
            let yf = rng.bool();
            return Some((tile_id, rot, xf, yf));
        }
        roll -= bucket.weight;
    }

    // Floating-point rounding can leave `roll` past every weighted bucket;
    // fall back to the first non-empty bucket.
    for bucket in &pool.buckets {
        if !bucket.tile_ids.is_empty() {
            let idx = rng.usize(..bucket.tile_ids.len());
            return Some((bucket.tile_ids[idx], rng.u8(..4), rng.bool(), rng.bool()));
        }
    }

    None
}

/// Paint terrain using weighted random tile selection from a ground type pool.
/// Each tile in the brush radius picks independently with a random orientation.
pub fn apply_ground_type_paint(
    map: &mut MapData,
    center_x: u32,
    center_y: u32,
    radius: u32,
    pool: &TilePool,
) {
    let mut rng = fastrand::Rng::new();
    let total_weight = pool.total_weight();

    super::for_each_tile_in_radius(
        center_x,
        center_y,
        radius,
        map.width,
        map.height,
        |tx, ty| {
            if let Some(tile) = map.tile_mut(tx, ty)
                && let Some((tile_id, rot, xf, yf)) = pick_random_tile(pool, &mut rng, total_weight)
            {
                tile.texture = MapTile::make_texture(tile_id, xf, yf, rot, false);
            }
        },
    );
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TileGroupEntry {
    pub tile_id: u16,
    pub weight: u32,
}

/// User-defined tile group for painting, persisted in config.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CustomTileGroup {
    pub name: String,
    pub color: [u8; 3],
    pub tiles: Vec<TileGroupEntry>,
}

impl CustomTileGroup {
    /// Each tile becomes its own single-tile bucket so per-tile weighting
    /// works with the weighted-walk logic.
    pub fn to_tile_pool(&self, index: u8) -> TilePool {
        let buckets = self
            .tiles
            .iter()
            .map(|entry| TileBucket {
                tile_ids: vec![entry.tile_id],
                weight: entry.weight,
            })
            .collect();
        TilePool {
            ground_type_index: index,
            name: self.name.clone(),
            color: self.color,
            buckets,
        }
    }
}

/// Convert runtime tile pools back into config-shaped custom groups.
pub fn pools_to_custom_groups(pools: &[TilePool]) -> Vec<CustomTileGroup> {
    pools
        .iter()
        .map(|pool| {
            let tiles = pool
                .buckets
                .iter()
                .flat_map(|b| {
                    b.tile_ids.iter().map(move |&tid| TileGroupEntry {
                        tile_id: tid,
                        weight: b.weight,
                    })
                })
                .collect();
            CustomTileGroup {
                name: pool.name.clone(),
                color: pool.color,
                tiles,
            }
        })
        .collect()
}

/// Random saturated RGB suitable for a group indicator swatch.
pub fn random_group_color() -> [u8; 3] {
    let mut rng = fastrand::Rng::new();
    let hue = rng.f32() * 360.0;
    let (r, g, b) = hsl_to_rgb(hue, 0.65, 0.55);
    [r, g, b]
}

#[expect(
    clippy::many_single_char_names,
    reason = "h, s, l, c, x, m, r, g, b are the standard HSL→RGB algorithm names"
)]
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Strips tileset prefixes (`a_`, `u_`, `r_`) and capitalises.
pub fn ground_type_display_name(raw_name: &str) -> String {
    let stripped = if raw_name.len() > 2 && raw_name.as_bytes()[1] == b'_' {
        &raw_name[2..]
    } else {
        raw_name
    };

    let mut chars = stripped.chars();
    match chars.next() {
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            format!("{upper}{rest}", rest = chars.as_str())
        }
        None => raw_name.to_string(),
    }
}

/// Idempotent ground-type brush. Owns brush size + selected pool +
/// per-stroke snapshot.
#[derive(Debug, Default)]
pub(crate) struct GroundTypeBrushTool {
    pub(crate) brush_size: u32,
    pub(crate) selected_ground_type: u8,
    /// First-touch snapshot of original packed `tile.texture` values,
    /// keyed by tile coords. Populated lazily so undo packages the whole
    /// drag as one [`TexturePaintCommand`].
    snapshot: HashMap<(u32, u32), u16>,
    last_tile: Option<(u32, u32)>,
    /// Suppresses the empty-pool log on every press in the same stroke.
    warned_empty_this_stroke: bool,
    line: LineModeState,
}

impl GroundTypeBrushTool {
    /// Lazy first-touch snapshot so undo packages the whole drag as one step.
    fn snapshot_radius(&mut self, map: &MapData, cx: u32, cy: u32) {
        super::for_each_tile_in_radius(cx, cy, self.brush_size, map.width, map.height, |tx, ty| {
            self.snapshot
                .entry((tx, ty))
                .or_insert_with(|| map.tile(tx, ty).map_or(0, |t| t.texture));
        });
    }

    fn fire(&mut self, ctx: &mut ToolCtx<'_>, tx: u32, ty: u32) {
        let Some(pool) = ctx.tile_pools.get(self.selected_ground_type as usize) else {
            return;
        };
        // Clone to drop the borrow on `ctx.tile_pools` before mutating `ctx.map`.
        let pool = pool.clone();

        let map_w = ctx.map.map_data.width;
        let map_h = ctx.map.map_data.height;
        let pts = mirror::mirror_points(tx, ty, map_w, map_h, ctx.mirror_mode);

        for &(mx, my) in &pts {
            self.snapshot_radius(&ctx.map.map_data, mx, my);
            apply_ground_type_paint(&mut ctx.map.map_data, mx, my, self.brush_size, &pool);
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

    /// Build one [`TexturePaintCommand`] from the snapshot delta. `None` if
    /// nothing changed. Forces a full rebuild so the cached water-depth field
    /// and the lightmap/shadow/water cascade catch up in one pass.
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
}

fn world_pos_to_tile(pos: WorldPos) -> (u32, u32) {
    (pos.x / TILE_UNITS, pos.y / TILE_UNITS)
}

impl Tool for GroundTypeBrushTool {
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
        if !self.warned_empty_this_stroke
            && let Some(pool) = ctx.tile_pools.get(self.selected_ground_type as usize)
            && pool.total_weight() == 0
        {
            ctx.log(format!(
                "Ground tool: pool '{}' has no tiles. Click tiles in the Tileset panel to add them.",
                pool.name
            ));
            self.warned_empty_this_stroke = true;
        }
        self.fire(ctx, tx, ty);
    }

    fn on_mouse_drag(&mut self, ctx: &mut ToolCtx<'_>, pos: WorldPos) {
        let (tx, ty) = world_pos_to_tile(pos);
        if tx >= ctx.map.map_data.width || ty >= ctx.map.map_data.height {
            return;
        }
        // Same tile + same pool repaints the same random pattern; skip
        // until the cursor crosses into a new tile.
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
        self.warned_empty_this_stroke = false;
        self.flush(ctx)
    }

    fn on_deactivated(&mut self, ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        self.warned_empty_this_stroke = false;
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
                self.warned_empty_this_stroke = false;
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

    fn properties_ui(&mut self, ui: &mut egui::Ui, ctx: &mut ToolCtx<'_>) {
        ui.heading("Ground (Random Tile)");
        ui.label(
            "Paints a random tile from the selected pool, with a fresh \
             random rotation/flip per tile. Useful for breaking up repeated \
             texture patterns.",
        );
        ui.add(egui::Slider::new(&mut self.brush_size, 0..=20).text("Radius"));

        let idx = self.selected_ground_type as usize;
        let pool = ctx.tile_pools.get(idx);
        if let Some(pool) = pool {
            let display = ground_type_display_name(&pool.name);
            let total: usize = pool.buckets.iter().map(|b| b.tile_ids.len()).sum();
            ui.label(format!("Pool: {display} ({total} tiles)"));
            if total == 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 160, 60),
                    "This pool is empty. Open the Tileset panel below and \
                     click tiles to add them.",
                );
            }
        } else {
            ui.label("No pool selected. Pick one in the Tileset panel.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_strips_prefix() {
        assert_eq!(ground_type_display_name("a_red"), "Red");
        assert_eq!(ground_type_display_name("u_gray"), "Gray");
        assert_eq!(ground_type_display_name("r_snowgrass"), "Snowgrass");
    }

    #[test]
    fn display_name_handles_no_prefix() {
        assert_eq!(ground_type_display_name("yellow"), "Yellow");
        assert_eq!(ground_type_display_name(""), "");
    }

    #[test]
    fn pick_from_empty_pool_returns_none() {
        let pool = TilePool {
            ground_type_index: 0,
            name: "empty".to_string(),
            color: [160, 160, 160],
            buckets: vec![],
        };
        let mut rng = fastrand::Rng::new();
        assert!(pick_random_tile(&pool, &mut rng, 0).is_none());
    }

    #[test]
    fn pick_from_single_bucket() {
        let pool = TilePool {
            ground_type_index: 0,
            name: "test".to_string(),
            color: [160, 160, 160],
            buckets: vec![TileBucket {
                tile_ids: vec![5, 10, 15],
                weight: 3,
            }],
        };
        let mut rng = fastrand::Rng::new();
        let result = pick_random_tile(&pool, &mut rng, pool.total_weight());
        assert!(result.is_some());
        let (tile_id, rot, _, _) = result.unwrap();
        assert!([5, 10, 15].contains(&tile_id));
        assert!(rot < 4);
    }

    #[test]
    fn pick_with_all_zero_weights_returns_none() {
        let pool = TilePool {
            ground_type_index: 0,
            name: "zeroed".to_string(),
            color: [160, 160, 160],
            buckets: vec![
                TileBucket {
                    tile_ids: vec![1, 2],
                    weight: 0,
                },
                TileBucket {
                    tile_ids: vec![3, 4],
                    weight: 0,
                },
            ],
        };
        let mut rng = fastrand::Rng::new();
        assert!(pick_random_tile(&pool, &mut rng, pool.total_weight()).is_none());
    }

    #[test]
    fn pick_never_selects_zero_weight_bucket() {
        let pool = TilePool {
            ground_type_index: 0,
            name: "weighted".to_string(),
            color: [160, 160, 160],
            buckets: vec![
                TileBucket {
                    tile_ids: vec![100],
                    weight: 0, // disabled
                },
                TileBucket {
                    tile_ids: vec![200],
                    weight: 10,
                },
            ],
        };
        let mut rng = fastrand::Rng::new();
        let tw = pool.total_weight();
        // Run many iterations - tile 100 should never appear.
        for _ in 0..100 {
            let (tile_id, _, _, _) = pick_random_tile(&pool, &mut rng, tw).unwrap();
            assert_eq!(tile_id, 200, "Zero-weight bucket should never be selected");
        }
    }

    #[test]
    fn paint_single_tile() {
        let mut map = MapData::new(10, 10);
        let pool = TilePool {
            ground_type_index: 0,
            name: "test".to_string(),
            color: [160, 160, 160],
            buckets: vec![TileBucket {
                tile_ids: vec![42],
                weight: 1,
            }],
        };
        apply_ground_type_paint(&mut map, 5, 5, 0, &pool);
        assert_eq!(map.tile(5, 5).unwrap().texture_id(), 42);
    }

    #[test]
    fn paint_square_radius() {
        let mut map = MapData::new(10, 10);
        let pool = TilePool {
            ground_type_index: 0,
            name: "test".to_string(),
            color: [160, 160, 160],
            buckets: vec![TileBucket {
                tile_ids: vec![7],
                weight: 1,
            }],
        };
        apply_ground_type_paint(&mut map, 5, 5, 1, &pool);
        // Radius 1 = 3x3 = 9 tiles, all should be tile 7.
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let tx = (5 + dx) as u32;
                let ty = (5 + dy) as u32;
                assert_eq!(map.tile(tx, ty).unwrap().texture_id(), 7);
            }
        }
    }

    #[test]
    fn paint_at_map_edge_no_panic() {
        let mut map = MapData::new(5, 5);
        let pool = TilePool {
            ground_type_index: 0,
            name: "test".to_string(),
            color: [160, 160, 160],
            buckets: vec![TileBucket {
                tile_ids: vec![1],
                weight: 1,
            }],
        };
        // Paint at corner with radius extending beyond map bounds.
        apply_ground_type_paint(&mut map, 0, 0, 2, &pool);
        assert_eq!(map.tile(0, 0).unwrap().texture_id(), 1);
    }

    #[test]
    fn paint_with_empty_pool_leaves_tiles_unchanged() {
        let mut map = MapData::new(5, 5);
        let original_tex = map.tile(2, 2).unwrap().texture;
        let pool = TilePool {
            ground_type_index: 0,
            name: "empty".to_string(),
            color: [160, 160, 160],
            buckets: vec![],
        };
        apply_ground_type_paint(&mut map, 2, 2, 0, &pool);
        assert_eq!(map.tile(2, 2).unwrap().texture, original_tex);
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

    fn single_tile_pool(tile_id: u16) -> TilePool {
        TilePool {
            ground_type_index: 0,
            name: "test".to_string(),
            color: [160, 160, 160],
            buckets: vec![TileBucket {
                tile_ids: vec![tile_id],
                weight: 1,
            }],
        }
    }

    #[test]
    fn ground_type_brush_tool_press_release_paints_and_returns_command() {
        let mut map = wz_maplib::WzMap::new("test", 8, 8);
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let pools = vec![single_tile_pool(42)];
        let mut tool = GroundTypeBrushTool {
            selected_ground_type: 0,
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
            tile_pools: &pools,
            log_sink: &mut log_sink,
            hovered_tile: &mut hovered_tile,
        };

        tool.on_mouse_press(&mut ctx, world_pos_for((3, 3)));
        assert!(*ctx.stroke_active, "press should mark stroke active");
        assert_eq!(
            ctx.map.map_data.tile(3, 3).unwrap().texture_id(),
            42,
            "press should paint a tile from the selected pool"
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
    fn ground_type_brush_tool_release_with_no_press_returns_none() {
        let mut map = wz_maplib::WzMap::new("test", 4, 4);
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let pools = vec![single_tile_pool(1)];
        let mut tool = GroundTypeBrushTool::default();
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
            tile_pools: &pools,
            log_sink: &mut log_sink,
            hovered_tile: &mut hovered_tile,
        };
        assert!(tool.on_mouse_release(&mut ctx, None).is_none());
        assert!(tool.on_deactivated(&mut ctx).is_none());
    }

    #[test]
    fn ground_type_brush_tool_with_empty_pools_does_nothing() {
        let mut map = wz_maplib::WzMap::new("test", 4, 4);
        let original_tex = map.map_data.tile(2, 2).unwrap().texture;
        let mut history = EditHistory::new();
        let mut dirty = DirtyFlags::default();
        let mut dirty_tiles = rustc_hash::FxHashSet::default();
        let mut stroke_active = false;
        let mut tool = GroundTypeBrushTool::default();
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
        assert!(
            !*ctx.stroke_active,
            "fire should bail out before marking stroke active"
        );
        assert_eq!(
            ctx.map.map_data.tile(2, 2).unwrap().texture,
            original_tex,
            "no pool means no paint"
        );
        assert!(tool.on_mouse_release(&mut ctx, None).is_none());
    }
}
