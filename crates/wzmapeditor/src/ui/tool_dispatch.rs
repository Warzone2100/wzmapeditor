//! Trait-based tool dispatch helpers shared by the viewport and the
//! tool-palette property pane.
//!
//! The dispatch boundary funnels short-lived borrows of editor state into
//! a [`ToolCtx`] for the trait impl, then propagates dirty flags back into
//! the per-frame booleans the renderer reads.

use crate::app::EditorApp;
use crate::app::output_log::{LogEntry, LogSeverity, LogSource, OutputLog};
use crate::tools::ToolId;
use crate::tools::trait_def::{
    DirtyFlags, PickerCtx, PointerInput, Tool, ToolCtx, ToolSwitchRequest,
};

/// Translate egui pointer events on the viewport into trait-method calls
/// on the active tool.
///
/// Holds a read lock on the wgpu callback resources across the trait call
/// so screen-input tools (object pickers, marquee selectors) can borrow
/// `&EditorRenderer` for AABB hit-tests. Tile-based tools fall through
/// [`crate::tools::trait_def::default_world_pos_dispatch`] and only touch
/// `ctx`.
pub(crate) fn dispatch_pointer_to_active_tool(
    app: &mut EditorApp,
    response: &egui::Response,
    rect: egui::Rect,
    camera: &crate::viewport::camera::Camera,
) {
    use crate::viewport::picking;

    let cursor_tile = {
        let Some(doc) = app.document.as_ref() else {
            return;
        };
        response
            .hover_pos()
            .and_then(|p| picking::screen_to_tile(p, rect, camera, &doc.map.map_data))
    };
    app.hovered_tile = cursor_tile;

    dispatch_pointer_input(app, response, rect, camera);

    // Continuous brush modes (raise/lower/smooth) keep re-firing while LMB
    // is held even when the cursor is stationary.
    let dragging = response.dragged_by(egui::PointerButton::Primary);
    if app.window_focused
        && dragging
        && app.tool_state.active_tool == ToolId::HeightBrush
        && let Some(brush) = app.tool_state.height_brush()
        && brush.is_continuous_mode()
    {
        response
            .ctx
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn dispatch_pointer_input(
    app: &mut EditorApp,
    response: &egui::Response,
    rect: egui::Rect,
    camera: &crate::viewport::camera::Camera,
) {
    if app
        .document
        .as_ref()
        .is_some_and(crate::map::document::MapDocument::is_read_only)
    {
        return;
    }
    let secondary_clicked = response.clicked_by(egui::PointerButton::Secondary);
    let escape_pressed = response.ctx.input(|i| i.key_pressed(egui::Key::Escape));

    let render_state = app.wgpu_render_state.clone();

    // Disjoint borrows so the tool registry, document, selection, and dirty
    // flags can all be touched at once alongside the renderer read lock.
    let EditorApp {
        ref mut tool_state,
        ref mut selection,
        ref mut objects_dirty,
        ref mut viewshed_dirty,
        ref mut hovered_tile,
        ref mut output_log,
        ref mut document,
        ref mut terrain_dirty,
        ref mut terrain_dirty_tiles,
        ref mut minimap,
        ref stats,
        ref model_loader,
        show_labels,
        show_gateways,
        ..
    } = *app;

    let Some(doc) = document.as_mut() else {
        return;
    };

    let crate::tools::ToolState {
        ref active_tool,
        ref mut tools,
        ref mut stroke_active,
        placement_player,
        mirror_mode,
        ref tile_pools,
        ..
    } = *tool_state;
    let active = *active_tool;
    let Some(tool) = tools.get_mut(&active) else {
        return;
    };

    let mut requested_tool_switch: Option<ToolSwitchRequest> = None;
    let mut dirty = DirtyFlags::default();

    // Hold the renderer lock across the trait call so picker borrows stay
    // valid; egui_wgpu re-acquires it during the next paint pass.
    let renderer_guard = render_state.as_ref().map(|rs| rs.renderer.read());
    let picker = renderer_guard
        .as_ref()
        .and_then(|guard| {
            guard
                .callback_resources
                .get::<crate::viewport::ViewportResources>()
        })
        .map(|res| PickerCtx {
            renderer: &res.renderer,
            model_loader: model_loader.as_ref(),
            show_labels,
            show_gateways,
        });

    let stats_ref: Option<&wz_stats::StatsDatabase> = stats.as_ref();

    {
        let mut log_sink = log_sink(output_log);
        let mut ctx = ToolCtx {
            map: &mut doc.map,
            history: &mut doc.history,
            dirty: &mut dirty,
            stats: stats_ref,
            placement_player,
            mirror_mode,
            terrain_dirty_tiles,
            stroke_active,
            tile_pools,
            log_sink: &mut log_sink,
            hovered_tile,
        };
        let input = PointerInput {
            response,
            rect,
            camera,
            picker,
            selection,
            objects_dirty,
            viewshed_dirty,
            requested_tool_switch: &mut requested_tool_switch,
        };
        if let Some(cmd) = tool.on_pointer_input(&mut ctx, input) {
            ctx.history.push_already_applied(cmd);
        }
        if secondary_clicked {
            tool.on_secondary_click(&mut ctx);
        }
        if escape_pressed {
            tool.on_cancel(&mut ctx);
        }
    }

    drop(renderer_guard);

    apply_dirty_flags(dirty, doc, terrain_dirty, objects_dirty, minimap);

    if let Some(req) = requested_tool_switch {
        apply_tool_switch(tool_state, req);
    }
}

/// Apply a tool switch request raised by a screen-input tool's pointer
/// handler. Runs after the trait call returns so the registry borrow is
/// already released.
fn apply_tool_switch(tool_state: &mut crate::tools::ToolState, req: ToolSwitchRequest) {
    let ToolSwitchRequest::StampWithPattern(pattern) = req;
    if let Some(stamp) = tool_state.stamp_mut() {
        stamp.pattern = Some(*pattern);
        stamp.capture_mode = false;
    }
    tool_state.active_tool = ToolId::Stamp;
}

/// Run a closure against the active trait-based tool. Returns `None` when
/// the active tool has no trait impl or no document is loaded.
///
/// Screen-input tools (selection, marquee) skip this helper because they
/// need wider borrows than the closure signature allows.
pub(crate) fn with_active_tool<R>(
    app: &mut EditorApp,
    f: impl FnOnce(&mut dyn Tool, &mut ToolCtx<'_>) -> R,
) -> Option<R> {
    let crate::tools::ToolState {
        active_tool,
        tools,
        stroke_active,
        placement_player,
        mirror_mode,
        tile_pools,
        ..
    } = &mut app.tool_state;
    let active = *active_tool;
    let placement_player = *placement_player;
    let mirror_mode = *mirror_mode;
    let tool = tools.get_mut(&active)?;
    let doc = app.document.as_mut()?;
    let stats = app.stats.as_ref();
    let mut dirty = DirtyFlags::default();
    let result = {
        let mut log_sink = log_sink(&mut app.output_log);
        let mut ctx = ToolCtx {
            map: &mut doc.map,
            history: &mut doc.history,
            dirty: &mut dirty,
            stats,
            placement_player,
            mirror_mode,
            terrain_dirty_tiles: &mut app.terrain_dirty_tiles,
            stroke_active,
            tile_pools,
            log_sink: &mut log_sink,
            hovered_tile: &mut app.hovered_tile,
        };
        f(tool.as_mut(), &mut ctx)
    };
    apply_dirty_flags(
        dirty,
        doc,
        &mut app.terrain_dirty,
        &mut app.objects_dirty,
        &mut app.minimap,
    );
    Some(result)
}

pub(crate) fn render_active_tool_properties(ui: &mut egui::Ui, app: &mut EditorApp) {
    with_active_tool(app, |tool, ctx| tool.properties_ui(ui, ctx));
}

/// Finalise any in-flight stroke on every non-active trait-based tool by
/// calling `on_deactivated`, producing one undo step per tool.
pub(crate) fn flush_inactive_tool_strokes(app: &mut EditorApp) {
    if app
        .document
        .as_ref()
        .is_some_and(crate::map::document::MapDocument::is_read_only)
    {
        return;
    }
    let active = app.tool_state.active_tool;
    let placement_player = app.tool_state.placement_player;
    let mirror_mode = app.tool_state.mirror_mode;
    let inactive: Vec<ToolId> = app
        .tool_state
        .tools
        .keys()
        .copied()
        .filter(|id| *id != active)
        .collect();
    if inactive.is_empty() {
        return;
    }
    let Some(doc) = app.document.as_mut() else {
        return;
    };
    let stats = app.stats.as_ref();
    let mut dirty = DirtyFlags::default();
    for id in inactive {
        let crate::tools::ToolState {
            tools,
            stroke_active,
            tile_pools,
            ..
        } = &mut app.tool_state;
        let Some(tool) = tools.get_mut(&id) else {
            continue;
        };
        let cmd = {
            let mut log_sink = log_sink(&mut app.output_log);
            let mut ctx = ToolCtx {
                map: &mut doc.map,
                history: &mut doc.history,
                dirty: &mut dirty,
                stats,
                placement_player,
                mirror_mode,
                terrain_dirty_tiles: &mut app.terrain_dirty_tiles,
                stroke_active,
                tile_pools,
                log_sink: &mut log_sink,
                hovered_tile: &mut app.hovered_tile,
            };
            tool.on_deactivated(&mut ctx)
        };
        if let Some(cmd) = cmd {
            doc.history.push_already_applied(cmd);
            doc.dirty = true;
        }
    }
    apply_dirty_flags(
        dirty,
        doc,
        &mut app.terrain_dirty,
        &mut app.objects_dirty,
        &mut app.minimap,
    );
}

fn apply_dirty_flags(
    dirty: DirtyFlags,
    doc: &mut crate::map::document::MapDocument,
    terrain_dirty: &mut bool,
    objects_dirty: &mut bool,
    minimap: &mut crate::ui::minimap::MinimapState,
) {
    if dirty.terrain || dirty.tile_textures || dirty.objects {
        doc.dirty = true;
    }
    if dirty.terrain {
        *terrain_dirty = true;
    }
    if dirty.objects {
        *objects_dirty = true;
    }
    if dirty.minimap {
        minimap.dirty = true;
    }
}

/// Mirrors `EditorApp::log` so trait impls can emit user-visible feedback
/// through `ToolCtx::log`.
fn log_sink(output_log: &mut OutputLog) -> impl FnMut(String) + '_ {
    move |msg: String| {
        log::info!("{msg}");
        output_log.push(LogEntry::new(LogSeverity::Info, LogSource::Editor, msg));
    }
}
