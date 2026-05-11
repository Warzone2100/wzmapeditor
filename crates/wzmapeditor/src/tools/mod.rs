//! Editor tool definitions and state.

pub mod gateway_tool;
pub mod ground_type_brush;
pub mod height_brush;
pub mod label_tool;
pub mod line_mode;
pub mod mirror;
pub mod object_edit;
pub mod object_tools;
pub mod placement;
pub mod stamp;
pub mod texture_paint;
pub mod trait_def;
pub mod vertex_sculpt;
pub mod wall_tool;

/// Mirror symmetry mode for terrain tools and object placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MirrorMode {
    #[default]
    None,
    /// Reflect across vertical axis (left/right).
    Vertical,
    /// Reflect across horizontal axis (top/bottom).
    Horizontal,
    /// Reflect across both axes (4-way).
    Both,
    /// Reflect across both diagonals (4-way rotational, square maps only).
    Diagonal,
}

/// Tool identity used for keybindings, palette dispatch, and the
/// [`trait_def`] registry. Distinct from the trait object that supplies behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolId {
    HeightBrush,
    TexturePaint,
    GroundTypePaint,
    ObjectSelect,
    ObjectPlace,
    Gateway,
    ScriptLabel,
    Stamp,
    WallPlacement,
    VertexSculpt,
}

impl ToolId {
    /// Gates both the property-panel mirror picker and the mirror-axis overlay
    /// so the lines only appear for tools that can actually mirror.
    pub fn uses_mirror(self) -> bool {
        matches!(
            self,
            Self::HeightBrush
                | Self::TexturePaint
                | Self::ObjectPlace
                | Self::Stamp
                | Self::VertexSculpt
        )
    }
}

/// Operation performed by the unified height brush.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeightBrushMode {
    #[default]
    Raise,
    Lower,
    /// Blend each tile toward its neighbours' average.
    Smooth,
    /// Blend each tile toward `target_height`.
    Set,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StampMode {
    /// Click places the captured pattern once.
    #[default]
    Single,
    /// Drag scatters randomly sampled objects within a circular brush.
    Scatter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCategory {
    Structures,
    Features,
    Droids,
}

/// State shared across all tools.
#[derive(Debug)]
pub struct ToolState {
    pub active_tool: ToolId,
    /// Trait-based tool instances keyed by [`ToolId`].
    pub tools: std::collections::HashMap<ToolId, Box<dyn trait_def::Tool>>,
    /// Lets the viewport defer the shadow/water/lightmap cascade until a stroke ends.
    pub stroke_active: bool,
    /// Shared placement player for Object Place, Wall, and Stamp tools.
    pub placement_player: i8,
    pub asset_search: String,
    /// Pixels per thumbnail in the asset grid.
    pub asset_thumb_size: f32,
    /// True = grid view, false = list view.
    pub asset_grid_view: bool,
    pub asset_category: AssetCategory,
    /// Off by default: campaign-only entries silently fail to spawn in skirmish/multiplayer maps.
    pub asset_show_campaign_only: bool,
    /// Pre-computed tile pools per ground type, rebuilt on tileset load.
    pub tile_pools: Vec<ground_type_brush::TilePool>,
    /// True = ground type mode in the tileset browser; false = tile mode.
    pub ground_type_mode: bool,
    pub new_group_name: String,
    /// `tile_id` -> pool indices. Rebuilt when `tile_pools_dirty` is set.
    pub tile_membership: std::collections::HashMap<u16, Vec<usize>>,
    pub tile_pools_dirty: bool,
    pub mirror_mode: MirrorMode,
}

impl Default for ToolState {
    fn default() -> Self {
        let mut tools: std::collections::HashMap<ToolId, Box<dyn trait_def::Tool>> =
            std::collections::HashMap::new();
        tools.insert(ToolId::WallPlacement, Box::<wall_tool::WallTool>::default());
        tools.insert(ToolId::Stamp, Box::<stamp::StampTool>::default());
        tools.insert(ToolId::Gateway, Box::<gateway_tool::GatewayTool>::default());
        tools.insert(
            ToolId::ScriptLabel,
            Box::<label_tool::ScriptLabelTool>::default(),
        );
        tools.insert(
            ToolId::HeightBrush,
            Box::<height_brush::HeightBrushTool>::default(),
        );
        tools.insert(
            ToolId::TexturePaint,
            Box::<texture_paint::TexturePaintTool>::default(),
        );
        tools.insert(
            ToolId::GroundTypePaint,
            Box::<ground_type_brush::GroundTypeBrushTool>::default(),
        );
        tools.insert(
            ToolId::VertexSculpt,
            Box::<vertex_sculpt::VertexSculptTool>::default(),
        );
        tools.insert(
            ToolId::ObjectSelect,
            Box::<object_tools::ObjectSelectTool>::default(),
        );
        tools.insert(
            ToolId::ObjectPlace,
            Box::<object_tools::ObjectPlaceTool>::default(),
        );
        Self {
            active_tool: ToolId::ObjectSelect,
            tools,
            stroke_active: false,
            placement_player: 0,
            asset_search: String::new(),
            asset_thumb_size: 64.0,
            asset_grid_view: true,
            asset_category: AssetCategory::Structures,
            asset_show_campaign_only: false,
            tile_pools: Vec::new(),
            ground_type_mode: false,
            new_group_name: String::new(),
            tile_membership: std::collections::HashMap::new(),
            tile_pools_dirty: true,
            mirror_mode: MirrorMode::None,
        }
    }
}

impl ToolState {
    /// Borrow the registered [`stamp::StampTool`] for read-only
    /// access. `None` if the registry has not been initialised yet
    /// (only true in tests that construct a custom [`ToolState`]).
    pub fn stamp(&self) -> Option<&stamp::StampTool> {
        self.tools
            .get(&ToolId::Stamp)
            .and_then(|t| t.as_any().downcast_ref::<stamp::StampTool>())
    }

    /// Borrow the registered [`stamp::StampTool`] for read-write
    /// access. See [`ToolState::stamp`].
    pub fn stamp_mut(&mut self) -> Option<&mut stamp::StampTool> {
        self.tools
            .get_mut(&ToolId::Stamp)
            .and_then(|t| t.as_any_mut().downcast_mut::<stamp::StampTool>())
    }

    /// Borrow the registered [`label_tool::ScriptLabelTool`] for
    /// read-only access. Used by the viewport overlay to draw the
    /// in-flight area-label drag rectangle.
    pub fn script_label(&self) -> Option<&label_tool::ScriptLabelTool> {
        self.tools
            .get(&ToolId::ScriptLabel)
            .and_then(|t| t.as_any().downcast_ref::<label_tool::ScriptLabelTool>())
    }

    /// Borrow the registered [`texture_paint::TexturePaintTool`] for
    /// read-only access. Used by the viewport's brush-ring overlay to
    /// read the active radius.
    pub fn texture_paint(&self) -> Option<&texture_paint::TexturePaintTool> {
        self.tools
            .get(&ToolId::TexturePaint)
            .and_then(|t| t.as_any().downcast_ref::<texture_paint::TexturePaintTool>())
    }

    /// Borrow the registered [`texture_paint::TexturePaintTool`] for
    /// read-write access. Used by the tileset browser and Ctrl-pick
    /// path to seed the selected texture / orientation from a sampled tile.
    pub fn texture_paint_mut(&mut self) -> Option<&mut texture_paint::TexturePaintTool> {
        self.tools.get_mut(&ToolId::TexturePaint).and_then(|t| {
            t.as_any_mut()
                .downcast_mut::<texture_paint::TexturePaintTool>()
        })
    }

    /// Borrow the registered [`height_brush::HeightBrushTool`] for
    /// read-only access. The viewport reads `is_continuous_mode` here
    /// to decide whether to schedule a continuous repaint.
    pub fn height_brush(&self) -> Option<&height_brush::HeightBrushTool> {
        self.tools
            .get(&ToolId::HeightBrush)
            .and_then(|t| t.as_any().downcast_ref::<height_brush::HeightBrushTool>())
    }

    /// Borrow the registered [`height_brush::HeightBrushTool`] for
    /// read-write access. Used by the keymap to switch the active
    /// height-brush mode and by the Ctrl-pick path to seed
    /// `target_height` from a sampled tile.
    pub fn height_brush_mut(&mut self) -> Option<&mut height_brush::HeightBrushTool> {
        self.tools.get_mut(&ToolId::HeightBrush).and_then(|t| {
            t.as_any_mut()
                .downcast_mut::<height_brush::HeightBrushTool>()
        })
    }

    /// Borrow the registered [`ground_type_brush::GroundTypeBrushTool`]
    /// for read-only access. The viewport reads the brush radius here
    /// when drawing the brush-ring overlay, and the tileset browser
    /// reads `selected_ground_type` for highlight state.
    pub fn ground_type_brush(&self) -> Option<&ground_type_brush::GroundTypeBrushTool> {
        self.tools.get(&ToolId::GroundTypePaint).and_then(|t| {
            t.as_any()
                .downcast_ref::<ground_type_brush::GroundTypeBrushTool>()
        })
    }

    /// Borrow the registered [`ground_type_brush::GroundTypeBrushTool`]
    /// for read-write access. Used by the tileset browser group-picker
    /// and the Ctrl-pick path to set the active ground type.
    pub fn ground_type_brush_mut(&mut self) -> Option<&mut ground_type_brush::GroundTypeBrushTool> {
        self.tools.get_mut(&ToolId::GroundTypePaint).and_then(|t| {
            t.as_any_mut()
                .downcast_mut::<ground_type_brush::GroundTypeBrushTool>()
        })
    }

    /// Borrow the registered [`vertex_sculpt::VertexSculptTool`] for
    /// read-only access. The viewport overlay reads selection state and
    /// the marquee rectangle here.
    pub fn vertex_sculpt(&self) -> Option<&vertex_sculpt::VertexSculptTool> {
        self.tools
            .get(&ToolId::VertexSculpt)
            .and_then(|t| t.as_any().downcast_ref::<vertex_sculpt::VertexSculptTool>())
    }

    /// Borrow the registered [`vertex_sculpt::VertexSculptTool`] for
    /// read-write access. The viewport calls this when leaving the tool
    /// to wipe the selection and any in-flight drag state.
    pub fn vertex_sculpt_mut(&mut self) -> Option<&mut vertex_sculpt::VertexSculptTool> {
        self.tools.get_mut(&ToolId::VertexSculpt).and_then(|t| {
            t.as_any_mut()
                .downcast_mut::<vertex_sculpt::VertexSculptTool>()
        })
    }

    /// Borrow the registered [`object_tools::ObjectSelectTool`] for
    /// read-only access. The duplicate path checks `dragging_object`
    /// here to know whether to stamp at the dragged position.
    pub fn object_select(&self) -> Option<&object_tools::ObjectSelectTool> {
        self.tools
            .get(&ToolId::ObjectSelect)
            .and_then(|t| t.as_any().downcast_ref::<object_tools::ObjectSelectTool>())
    }

    /// Borrow the registered [`object_tools::ObjectPlaceTool`] for
    /// read-only access. The viewport's ghost preview reads
    /// `placement_object`, `preview_pos`, and `preview_valid` here.
    pub fn object_place(&self) -> Option<&object_tools::ObjectPlaceTool> {
        self.tools
            .get(&ToolId::ObjectPlace)
            .and_then(|t| t.as_any().downcast_ref::<object_tools::ObjectPlaceTool>())
    }

    /// Borrow the registered [`object_tools::ObjectPlaceTool`] for
    /// read-write access. The asset browser writes `placement_object`,
    /// the Ctrl+click eyedropper writes the full set, and the R-key
    /// keymap action rotates `placement_direction`.
    pub fn object_place_mut(&mut self) -> Option<&mut object_tools::ObjectPlaceTool> {
        self.tools.get_mut(&ToolId::ObjectPlace).and_then(|t| {
            t.as_any_mut()
                .downcast_mut::<object_tools::ObjectPlaceTool>()
        })
    }

    /// Borrow the registered [`wall_tool::WallTool`] for read-only access.
    /// The renderer's wall-ghost path reads the active family, the
    /// cross-corners toggle, and the hovered tile from here.
    pub fn wall_tool(&self) -> Option<&wall_tool::WallTool> {
        self.tools
            .get(&ToolId::WallPlacement)
            .and_then(|t| t.as_any().downcast_ref::<wall_tool::WallTool>())
    }
}

/// Iterate over all map tiles within a square brush of the given `radius`,
/// centered on `(cx, cy)`. Tiles outside `[0, width)` x `[0, height)` are
/// skipped. Calls `f(tile_x, tile_y)` for each valid tile.
pub fn for_each_tile_in_radius(
    cx: u32,
    cy: u32,
    radius: u32,
    width: u32,
    height: u32,
    mut f: impl FnMut(u32, u32),
) {
    let r = radius as i64;
    let cx_i = cx as i64;
    let cy_i = cy as i64;

    for dy in -r..=r {
        for dx in -r..=r {
            let tx = cx_i + dx;
            let ty = cy_i + dy;
            if tx < 0 || ty < 0 {
                continue;
            }
            let tx = tx as u32;
            let ty = ty as u32;
            if tx >= width || ty >= height {
                continue;
            }
            f(tx, ty);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_each_tile_radius_zero() {
        let mut tiles = Vec::new();
        for_each_tile_in_radius(5, 5, 0, 10, 10, |x, y| tiles.push((x, y)));
        assert_eq!(tiles, vec![(5, 5)]);
    }

    #[test]
    fn for_each_tile_radius_one() {
        let mut tiles = Vec::new();
        for_each_tile_in_radius(5, 5, 1, 10, 10, |x, y| tiles.push((x, y)));
        assert_eq!(tiles.len(), 9);
        assert!(tiles.contains(&(4, 4)));
        assert!(tiles.contains(&(5, 5)));
        assert!(tiles.contains(&(6, 6)));
    }

    #[test]
    fn for_each_tile_clamps_to_origin() {
        let mut tiles = Vec::new();
        for_each_tile_in_radius(0, 0, 1, 10, 10, |x, y| tiles.push((x, y)));
        assert_eq!(tiles.len(), 4);
        assert!(tiles.contains(&(0, 0)));
        assert!(tiles.contains(&(1, 0)));
        assert!(tiles.contains(&(0, 1)));
        assert!(tiles.contains(&(1, 1)));
    }

    #[test]
    fn for_each_tile_clamps_to_map_edge() {
        let mut tiles = Vec::new();
        for_each_tile_in_radius(3, 3, 2, 4, 4, |x, y| tiles.push((x, y)));
        for &(x, y) in &tiles {
            assert!(x < 4 && y < 4, "tile ({x},{y}) out of bounds");
        }
        assert_eq!(tiles.len(), 9);
    }

    #[test]
    fn for_each_tile_full_map_coverage() {
        let mut count = 0;
        for_each_tile_in_radius(1, 1, 5, 3, 3, |_, _| count += 1);
        assert_eq!(count, 9);
    }
}
