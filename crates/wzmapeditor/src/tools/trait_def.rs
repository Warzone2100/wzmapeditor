//! Tool trait + per-stroke context.
//!
//! Replaces the `match active_tool` dispatch that used to live across
//! `ui/viewport_panel/`. Each tool owns its own per-stroke state and
//! per-tool settings; the dispatcher routes pointer input through the
//! trait surface below.

use std::any::Any;

use wz_maplib::WzMap;
use wz_maplib::objects::WorldPos;

use crate::map::history::{EditCommand, EditHistory};

use super::MirrorMode;
use super::line_mode::LineModeState;

/// Behaviour shared by every editor tool.
///
/// Each tool owns its per-stroke state (e.g. the captured stamp pattern,
/// the active wall-drag, the height-brush snapshot) and exposes the
/// hooks the viewport calls in response to pointer input. The trait
/// returns an [`EditCommand`] from `on_mouse_release` when a stroke
/// produced an undoable mutation; the viewport pushes it onto the
/// history stack via [`EditHistory::push_already_applied`].
///
/// # Examples
///
/// A tool implementation skeleton:
///
/// ```ignore
/// use wzmapeditor::tools::trait_def::{Tool, ToolCtx};
/// use wzmapeditor::map::history::EditCommand;
/// use wz_maplib::objects::WorldPos;
///
/// #[derive(Debug, Default)]
/// struct NoopTool;
///
/// impl Tool for NoopTool {
///     fn as_any(&self) -> &dyn std::any::Any { self }
///     fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
///     fn on_mouse_press(&mut self, _: &mut ToolCtx, _: WorldPos) {}
///     fn on_mouse_drag(&mut self, _: &mut ToolCtx, _: WorldPos) {}
///     fn on_mouse_release(&mut self, _: &mut ToolCtx, _: Option<WorldPos>) -> Option<Box<dyn EditCommand>> { None }
///     fn properties_ui(&mut self, _: &mut egui::Ui, _: &mut ToolCtx) {}
/// }
/// ```
pub trait Tool: std::fmt::Debug + Send + Sync + Any {
    /// Cast to `&dyn Any` so callers can downcast to a concrete tool
    /// type for read-only access (overlays reading captured stamp
    /// patterns, etc.). Implementations always return `self`.
    fn as_any(&self) -> &dyn Any;
    /// Mutable counterpart of [`Tool::as_any`]. Implementations always
    /// return `self`.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Pointer pressed at `pos` over the map (left mouse button).
    /// Tile-based tools override this; screen-input tools that handle
    /// pointer events directly through [`Tool::on_pointer_input`] leave
    /// it as the no-op default.
    fn on_mouse_press(&mut self, ctx: &mut ToolCtx<'_>, pos: WorldPos) {
        let _ = (ctx, pos);
    }

    /// Pointer dragged to `pos` while the left mouse button is held.
    fn on_mouse_drag(&mut self, ctx: &mut ToolCtx<'_>, pos: WorldPos) {
        let _ = (ctx, pos);
    }

    /// Cursor moved with no button held. `pos` is the tile-snapped world
    /// position when the cursor is over the map, or `None` when it's off
    /// the map or off-screen. Used by tools that draw a hover-time ghost
    /// or footprint preview (stamp, wall placer) to keep the preview
    /// tracking the cursor between clicks and to clear it on exit.
    fn on_mouse_hover(&mut self, ctx: &mut ToolCtx<'_>, pos: Option<WorldPos>) {
        let _ = (ctx, pos);
    }

    /// Pointer released. `pos` is the cursor position at release time
    /// when the cursor is over the map, or `None` if the drag ended
    /// off-terrain. Returns the stroke's compound edit command if the
    /// tool produced one. Callers push it onto the undo stack.
    fn on_mouse_release(
        &mut self,
        ctx: &mut ToolCtx<'_>,
        pos: Option<WorldPos>,
    ) -> Option<Box<dyn EditCommand>> {
        let _ = (ctx, pos);
        None
    }

    /// Called when the user switches away from this tool. A tool with
    /// an in-flight stroke flushes it here so the accumulated edits land
    /// as one undo step instead of being dropped on the floor.
    fn on_deactivated(&mut self, ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        let _ = ctx;
        None
    }

    /// Right-mouse-button click. Tools that use it for cancel-style
    /// affordances (Stamp's escape-from-pattern, etc.) override this.
    /// Returns `true` if the click was consumed by the tool.
    fn on_secondary_click(&mut self, ctx: &mut ToolCtx<'_>) -> bool {
        let _ = ctx;
        false
    }

    /// Single pointer entry point called by the viewport every frame
    /// the cursor is over the map. The default implementation translates
    /// egui pointer events into [`Tool::on_mouse_press`] /
    /// [`Tool::on_mouse_drag`] / [`Tool::on_mouse_release`] using the
    /// cursor's world-space tile, which is what every tile-based tool
    /// (height brush, texture paint, walls, stamp, etc.) wants.
    ///
    /// Screen-input tools (object pickers, marquee selectors, vertex
    /// gizmos) override this to read the raw [`egui::Response`] and
    /// borrow the renderer through [`PointerInput::picker`] instead of
    /// going through the `WorldPos` hooks.
    ///
    /// Returns `Some(cmd)` when the interaction produced an undoable
    /// edit this frame.
    fn on_pointer_input(
        &mut self,
        ctx: &mut ToolCtx<'_>,
        input: PointerInput<'_>,
    ) -> Option<Box<dyn EditCommand>> {
        default_world_pos_dispatch(self, ctx, input)
    }

    /// Brush footprint in tile units, when the tool draws a circular
    /// hover ring. Returning `None` (the default) means no ring.
    fn brush_radius_tiles(&self) -> Option<u32> {
        None
    }

    /// Cancel any in-flight modal state (e.g. an armed Shift+click line
    /// awaiting its second click). Triggered by the Esc key. Default is
    /// a no-op; tools with modal state override it.
    fn on_cancel(&mut self, ctx: &mut ToolCtx<'_>) {
        let _ = ctx;
    }

    /// Read-only view of the tool's Shift+click line-draw state, if any.
    /// The viewport renders the preview overlay from this. Tools without
    /// line mode return `None` (the default).
    fn line_mode_state(&self) -> Option<&LineModeState> {
        None
    }

    /// One-line shortcut hint shown at the bottom of the viewport.
    /// Returning `None` (the default) falls back to the generic camera
    /// help string the overlay layer renders.
    fn help_text(&self, keymap: &crate::keybindings::Keymap) -> Option<String> {
        let _ = keymap;
        None
    }

    /// Render the tool's settings into the property panel.
    fn properties_ui(&mut self, ui: &mut egui::Ui, ctx: &mut ToolCtx<'_>);
}

/// Borrowed context handed to every [`Tool`] callback.
///
/// Holds short-lived references the tool needs to read or mutate:
/// the open document, undo history, dirty-flag setters that schedule
/// GPU re-uploads, and read-only access to shared state (stats, the
/// editor's current placement player).
pub struct ToolCtx<'a> {
    /// The map being edited.
    pub map: &'a mut WzMap,
    /// Undo/redo stack, for tools that prefer to push their own
    /// compound commands mid-stroke instead of returning them.
    pub history: &'a mut EditHistory,
    /// Flags describing what GPU state needs to be rebuilt this frame.
    /// Tool implementations call the corresponding `mark_*_dirty`
    /// helper after mutating the map.
    pub dirty: &'a mut DirtyFlags,
    /// Stats database, when loaded. Tools that need stat lookups
    /// (wall connectors, structure footprints) read it through here.
    pub stats: Option<&'a wz_stats::StatsDatabase>,
    /// Player index for newly-placed objects. Shared across placement
    /// tools; the property panel writes it.
    pub placement_player: i8,
    /// Active mirror symmetry mode for tools that fan out their edits
    /// across reflection axes (terrain brushes, stamp, object placement).
    pub mirror_mode: MirrorMode,
    /// Per-tile partial dirty list for the terrain mesh. Tools that
    /// mutate a bounded brush rect record the touched tiles here so
    /// the renderer can `update_terrain_tile_rect` instead of doing a
    /// full ~22 MB mesh rebuild every cursor crossing.
    pub terrain_dirty_tiles: &'a mut rustc_hash::FxHashSet<(u32, u32)>,
    /// True while a tool is mid-stroke. The viewport defers the
    /// shadow / water / lightmap cascade until the stroke ends so a
    /// drag does not pay for those rebuilds on every tile crossing.
    pub stroke_active: &'a mut bool,
    /// Pre-computed ground-type tile pools. Each entry holds the
    /// buckets the ground-type brush samples from. Read-only on the
    /// tool side; the asset browser owns the build path.
    pub tile_pools: &'a [super::ground_type_brush::TilePool],
    /// Append a line to the editor's log panel. Tools call this for
    /// transient feedback ("Captured pattern: ..."). Implementations
    /// route the message into the same buffer that `EditorApp::log`
    /// writes to. Use [`ToolCtx::log`] rather than calling the field
    /// directly.
    pub log_sink: &'a mut dyn FnMut(String),
    /// Editor-wide hovered tile under the cursor. Screen-input tools
    /// (vertex sculpt) compute their own pick from the cursor position
    /// and write it back here so overlays and the info bar stay in sync.
    pub hovered_tile: &'a mut Option<(u32, u32)>,
}

/// Borrowed input bundle for screen-space pointer tools.
///
/// Tools that opt in via [`Tool::uses_screen_input`] receive this
/// instead of the WorldPos-based hooks so they can reach the raw
/// [`egui::Response`] (modifier keys, drag start/stop edges) and the
/// camera projection needed to pick vertices or marquee-select objects.
///
/// The object editor tools also need access to renderer-side AABBs, the
/// editor's [`Selection`](crate::app::Selection), and a few app-level
/// dirty flags. Those live in the optional [`PickerCtx`] plus the
/// borrowed mutable references below; tools that don't need them
/// simply ignore the extra fields.
pub struct PointerInput<'a> {
    pub response: &'a egui::Response,
    pub rect: egui::Rect,
    pub camera: &'a crate::viewport::camera::Camera,
    /// Renderer + model loader, when the wgpu callback resources are
    /// available. Tools that pick objects via screen-space AABB
    /// intersection borrow this.
    pub picker: Option<PickerCtx<'a>>,
    /// Editor-wide multi-selection. Object tools mutate it on click,
    /// drag-select, and Ctrl+click eyedropper.
    pub selection: &'a mut crate::app::Selection,
    /// Set when an object's position changes so the renderer rebuilds
    /// the per-object instance buffers.
    pub objects_dirty: &'a mut bool,
    /// Set when an object move invalidates the cached viewshed (ranges
    /// drawn around weaponised structures).
    pub viewshed_dirty: &'a mut bool,
    /// Tools that want to switch the active tool from inside their
    /// pointer handler write the request here. The dispatcher applies
    /// it after the trait call returns; this avoids reborrowing the
    /// tool registry while a tool is mid-call. `ObjectSelect` uses it
    /// for the Ctrl+drag stamp-capture handoff.
    pub requested_tool_switch: &'a mut Option<ToolSwitchRequest>,
}

/// Cross-tool handoff produced by a screen-input tool's pointer handler.
///
/// The dispatcher reads this after the trait call returns and applies
/// the side effects. Today the only producer is `ObjectSelect`'s
/// Ctrl+drag stamp-capture path; the variant carries the captured
/// pattern so the dispatcher can install it on the Stamp tool before
/// flipping `active_tool`.
#[derive(Debug)]
pub enum ToolSwitchRequest {
    /// Install `pattern` on the Stamp tool and switch to it.
    StampWithPattern(Box<super::stamp::StampPattern>),
}

impl std::fmt::Debug for PointerInput<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PointerInput")
            .field("rect", &self.rect)
            .field("has_picker", &self.picker.is_some())
            .field("requested_tool_switch", &*self.requested_tool_switch)
            .finish_non_exhaustive()
    }
}

/// Renderer plus model loader access for object picking.
///
/// Tools that hit-test objects with screen-space AABBs borrow this
/// bundle; absent when the wgpu renderer is not yet available (e.g.
/// the first frame after startup, or in egui-only test runs).
#[derive(Debug)]
pub struct PickerCtx<'a> {
    pub renderer: &'a crate::viewport::renderer::EditorRenderer,
    pub model_loader: Option<&'a crate::viewport::model_loader::ModelLoader>,
    pub show_labels: bool,
    pub show_gateways: bool,
}

impl ToolCtx<'_> {
    /// Mark terrain mesh data as needing a GPU re-upload.
    pub fn mark_terrain_dirty(&mut self) {
        self.dirty.terrain = true;
    }

    /// Mark per-tile texture indices as needing a GPU re-upload.
    #[expect(
        dead_code,
        reason = "texture/ground brushes route through mark_terrain_dirty until the renderer grows a tile-only fast path"
    )]
    pub fn mark_tile_textures_dirty(&mut self) {
        self.dirty.tile_textures = true;
    }

    /// Mark object instance buffers as needing a rebuild.
    pub fn mark_objects_dirty(&mut self) {
        self.dirty.objects = true;
    }

    /// Mark the minimap thumbnail as needing a redraw.
    pub fn mark_minimap_dirty(&mut self) {
        self.dirty.minimap = true;
    }

    /// Add the inclusive tile bounding box `[min_x..=max_x] x [min_y..=max_y]`
    /// to the partial terrain dirty set. Tools with bounded edit footprints
    /// call this after mutating tile data so the renderer can patch only the
    /// affected vertex range.
    pub fn mark_terrain_rect_dirty(&mut self, min_x: u32, min_y: u32, max_x: u32, max_y: u32) {
        if min_x > max_x || min_y > max_y {
            return;
        }
        for ty in min_y..=max_y {
            for tx in min_x..=max_x {
                self.terrain_dirty_tiles.insert((tx, ty));
            }
        }
    }

    /// Append a line to the editor's log panel.
    pub fn log(&mut self, msg: impl Into<String>) {
        (self.log_sink)(msg.into());
    }
}

impl std::fmt::Debug for ToolCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolCtx")
            .field("placement_player", &self.placement_player)
            .field("dirty", &self.dirty)
            .field("stats", &self.stats.is_some())
            .field("terrain_dirty_tile_count", &self.terrain_dirty_tiles.len())
            .field("stroke_active", &*self.stroke_active)
            .field("tile_pool_count", &self.tile_pools.len())
            .finish_non_exhaustive()
    }
}

/// Coarse-grained dirty flags consumed by the renderer at frame end.
///
/// Mirrors the booleans currently scattered on `EditorApp`. Kept
/// behind this trait-side handle so tools talk to a stable surface
/// even as the owning struct's field layout evolves.
#[derive(Debug, Default, Clone, Copy)]
pub struct DirtyFlags {
    pub terrain: bool,
    pub tile_textures: bool,
    pub objects: bool,
    pub minimap: bool,
}

/// Default [`Tool::on_pointer_input`] body.
///
/// Translates the egui drag/click edges in `input.response` into
/// [`Tool::on_mouse_press`] / [`Tool::on_mouse_drag`] /
/// [`Tool::on_mouse_release`] using the cursor's tile-aligned
/// [`WorldPos`]. Tools that override `on_pointer_input` skip this path.
#[expect(
    clippy::needless_pass_by_value,
    reason = "trait method passes PointerInput by value; this helper mirrors that signature so impls can forward `input` straight through"
)]
pub fn default_world_pos_dispatch<T: Tool + ?Sized>(
    tool: &mut T,
    ctx: &mut ToolCtx<'_>,
    input: PointerInput<'_>,
) -> Option<Box<dyn EditCommand>> {
    use wz_maplib::constants::TILE_UNITS;

    let response = input.response;
    let cursor_tile = response.hover_pos().and_then(|p| {
        crate::viewport::picking::screen_to_tile(p, input.rect, input.camera, &ctx.map.map_data)
    });

    let pos_for = |tile: (u32, u32)| WorldPos {
        x: tile.0 * TILE_UNITS + TILE_UNITS / 2,
        y: tile.1 * TILE_UNITS + TILE_UNITS / 2,
    };

    let drag_started = response.drag_started_by(egui::PointerButton::Primary);
    let dragging = response.dragged_by(egui::PointerButton::Primary);
    let drag_stopped = response.drag_stopped_by(egui::PointerButton::Primary);
    let clicked = response.clicked_by(egui::PointerButton::Primary);

    let on_tile_pos = cursor_tile.map(pos_for);
    if let Some(pos) = on_tile_pos {
        if drag_started || clicked {
            tool.on_mouse_press(ctx, pos);
        } else if dragging {
            tool.on_mouse_drag(ctx, pos);
        } else {
            tool.on_mouse_hover(ctx, Some(pos));
        }
    } else if !dragging {
        tool.on_mouse_hover(ctx, None);
    }
    if drag_stopped || clicked {
        return tool.on_mouse_release(ctx, on_tile_pos);
    }
    None
}
