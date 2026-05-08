//! Trait-based `ObjectSelect` and `ObjectPlace` tools.
//!
//! Both rely on screen-space pointer input (object picking, hover ghost,
//! drag-select rect) and live behind `Tool::on_pointer_input`. Per-stroke
//! state (drag-select rectangles, drag-move position snapshots, placement
//! preview) lives on the per-tool struct so switching tools cleans up
//! through `on_deactivated` instead of leaving stale flags on `ToolState`.

use wz_maplib::constants::TILE_UNITS_F32 as TILE_UNITS;
use wz_maplib::objects::WorldPos;

use crate::app::SelectedObject;
use crate::map::history::{CompoundCommand, EditCommand};
use crate::tools;
use crate::tools::trait_def::{PointerInput, Tool, ToolCtx, ToolSwitchRequest};
use crate::viewport::picking;

/// Click + drag-select + drag-move tool.
#[derive(Debug, Default)]
pub(crate) struct ObjectSelectTool {
    drag_select_active: bool,
    drag_select_start: Option<egui::Pos2>,
    dragging_object: bool,
    drag_start_world: Option<(f32, f32)>,
    drag_start_positions: Vec<(SelectedObject, WorldPos)>,
}

impl ObjectSelectTool {
    /// True while a multi-object move drag is in progress. Read by the
    /// duplicate shortcut so Ctrl+D mid-drag stamps copies in place.
    pub(crate) fn dragging_object(&self) -> bool {
        self.dragging_object
    }

    fn reset_transient(&mut self) {
        self.drag_select_active = false;
        self.drag_select_start = None;
        self.dragging_object = false;
        self.drag_start_world = None;
        self.drag_start_positions.clear();
    }
}

/// Hover-ghost + click-to-place tool.
#[derive(Debug, Default)]
pub(crate) struct ObjectPlaceTool {
    /// Asset name (structure / feature / droid template) to place.
    pub(crate) placement_object: Option<String>,
    /// Player slot for newly placed objects.
    pub(crate) placement_player: i8,
    /// Direction for newly placed objects (0-65535 WZ2100 units).
    pub(crate) placement_direction: u16,
    /// Last hovered snapped world position used for the ghost preview.
    pub(crate) preview_pos: Option<(u32, u32)>,
    /// Whether the cursor sits over a valid placement spot.
    pub(crate) preview_valid: bool,
}

impl ObjectPlaceTool {
    fn clear_preview(&mut self) {
        self.preview_pos = None;
        self.preview_valid = false;
    }
}

impl Tool for ObjectSelectTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn on_deactivated(&mut self, _ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        self.reset_transient();
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
            picker,
            selection,
            objects_dirty,
            viewshed_dirty,
            requested_tool_switch,
        } = input;

        let clicked = response.clicked_by(egui::PointerButton::Primary);
        let dragging = response.dragged_by(egui::PointerButton::Primary);
        let drag_released = response.drag_stopped_by(egui::PointerButton::Primary);

        let (shift_held, ctrl_held) = response
            .ctx
            .input(|i| (i.modifiers.shift, i.modifiers.command));

        // `hover_pos` goes `None` once the cursor moves over a UI panel, which
        // would silently drop a box-select released there. Fall back to the
        // egui-wide latest pointer position so the gesture still resolves.
        let cursor_pos = response
            .hover_pos()
            .or_else(|| response.ctx.input(|i| i.pointer.latest_pos()));

        if clicked && let Some(hover_pos) = response.hover_pos() {
            let pick_result = picker.as_ref().map(|p| {
                picking::pick_object(
                    hover_pos,
                    rect,
                    camera,
                    ctx.map,
                    p.renderer,
                    p.model_loader,
                    ctx.stats,
                    picking::PickVisibility {
                        labels: p.show_labels,
                        gateways: p.show_gateways,
                    },
                )
            });
            if let Some(Some(result)) = pick_result {
                if shift_held {
                    selection.toggle(result.kind);
                    selection.enforce_group();
                } else {
                    selection.set_single(result.kind);
                }
                *objects_dirty = true;
            } else if !shift_held {
                selection.clear();
                *objects_dirty = true;
            }
        }

        // Drag opens a rect-select or a drag-move depending on what's
        // under the cursor. Ctrl+drag always opens a rect for the
        // stamp-capture gesture.
        if dragging && !self.dragging_object && !self.drag_select_active {
            let mut start_move = false;

            if !selection.is_empty()
                && !shift_held
                && !ctrl_held
                && let Some(hover_pos) = response.hover_pos()
                && let Some(p) = picker.as_ref()
            {
                let over_selected = picking::pick_object(
                    hover_pos,
                    rect,
                    camera,
                    ctx.map,
                    p.renderer,
                    p.model_loader,
                    ctx.stats,
                    picking::PickVisibility {
                        labels: p.show_labels,
                        gateways: p.show_gateways,
                    },
                )
                .is_some_and(|r| selection.contains(&r.kind));

                if over_selected {
                    if let Some((wx, wz)) =
                        picking::screen_to_world_pos(hover_pos, rect, camera, &ctx.map.map_data)
                    {
                        self.drag_start_world = Some((wx, wz));
                    }
                    self.drag_start_positions.clear();
                    for obj in &selection.objects {
                        if let Some(pos) = get_object_pos(ctx.map, *obj) {
                            self.drag_start_positions.push((*obj, pos));
                        }
                    }
                    self.dragging_object = true;
                    start_move = true;
                }
            }

            if !start_move && let Some(hover_pos) = response.hover_pos() {
                self.drag_select_active = true;
                self.drag_select_start = Some(hover_pos);
            }
        }

        // Draw the in-flight drag-select rectangle here, alongside the
        // gesture that owns it. Orange while Ctrl is held to signal
        // stamp-capture mode.
        if self.drag_select_active
            && let (Some(start), Some(current)) = (self.drag_select_start, cursor_pos)
        {
            let sel_rect = egui::Rect::from_two_pos(start, current);
            let painter = response.ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drag_select_rect"),
            ));
            let (fill, stroke_color) = if ctrl_held {
                (
                    egui::Color32::from_rgba_unmultiplied(255, 200, 50, 40),
                    egui::Color32::from_rgb(255, 200, 50),
                )
            } else {
                (
                    egui::Color32::from_rgba_unmultiplied(50, 100, 200, 40),
                    egui::Color32::from_rgb(80, 140, 255),
                )
            };
            painter.rect_filled(sel_rect, 0.0, fill);
            painter.rect_stroke(
                sel_rect,
                0.0,
                egui::Stroke::new(1.5, stroke_color),
                egui::StrokeKind::Inside,
            );
        }

        if dragging
            && self.dragging_object
            && let Some(hover_pos) = response.hover_pos()
            && let Some((wx, wz)) =
                picking::screen_to_world_pos(hover_pos, rect, camera, &ctx.map.map_data)
            && let Some((start_wx, start_wz)) = self.drag_start_world
        {
            apply_drag_move(
                ctx.map,
                &self.drag_start_positions,
                wx - start_wx,
                wz - start_wz,
            );
            *objects_dirty = true;
            *viewshed_dirty = true;
        }

        if drag_released {
            if self.dragging_object {
                self.dragging_object = false;
                self.drag_start_world = None;
                let start_positions = std::mem::take(&mut self.drag_start_positions);
                let cmd = build_multi_move_command(ctx.map, &start_positions);
                *objects_dirty = true;
                return cmd;
            }

            if self.drag_select_active {
                if let (Some(start), Some(end)) = (self.drag_select_start.take(), cursor_pos) {
                    if ctrl_held {
                        let t1 = picking::screen_to_tile(start, rect, camera, &ctx.map.map_data);
                        let t2 = picking::screen_to_tile(end, rect, camera, &ctx.map.map_data);
                        if let (Some((sx, sy)), Some((ex, ey))) = (t1, t2) {
                            let pattern = tools::stamp::capture_pattern(ctx.map, sx, sy, ex, ey);
                            let tile_count = pattern.tiles.len();
                            let obj_count = pattern.objects.len();
                            ctx.log(format!(
                                "Captured stamp pattern: {tile_count} tiles, {obj_count} objects"
                            ));
                            // Defer the registry mutation: the dispatcher
                            // already holds a borrow on this tool, so it
                            // installs the pattern after we return.
                            *requested_tool_switch =
                                Some(ToolSwitchRequest::StampWithPattern(Box::new(pattern)));
                        }
                    } else {
                        let sel_rect = egui::Rect::from_two_pos(start, end);
                        let visibility =
                            picker
                                .as_ref()
                                .map_or_else(picking::PickVisibility::default, |p| {
                                    picking::PickVisibility {
                                        labels: p.show_labels,
                                        gateways: p.show_gateways,
                                    }
                                });
                        let picked = picking::objects_in_screen_rect(
                            sel_rect, rect, camera, ctx.map, visibility,
                        );
                        if shift_held {
                            for obj in picked {
                                selection.add(obj);
                            }
                        } else {
                            selection.objects = picked;
                        }
                        selection.enforce_group();
                        *objects_dirty = true;
                    }
                }
                self.drag_select_active = false;
            }
        }

        None
    }

    fn help_text(&self, keymap: &crate::keybindings::Keymap) -> Option<String> {
        let del_key = keymap.shortcut_text(crate::keybindings::Action::DeleteSelected);
        Some(format!(
            "RMB+WASD=fly | LMB=select | Shift+LMB=multi | Drag=box select | {del_key}=delete"
        ))
    }

    fn properties_ui(&mut self, ui: &mut egui::Ui, _ctx: &mut ToolCtx<'_>) {
        ui.heading("Select");
        ui.label("Click objects in the viewport to select them.");
    }
}

impl Tool for ObjectPlaceTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn on_deactivated(&mut self, _ctx: &mut ToolCtx<'_>) -> Option<Box<dyn EditCommand>> {
        self.clear_preview();
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
            objects_dirty,
            ..
        } = input;

        let clicked = response.clicked_by(egui::PointerButton::Primary);
        let shift_held = response.ctx.input(|i| i.modifiers.shift);

        if let Some(hover_pos) = response.hover_pos() {
            if let Some((wx, wz)) =
                picking::screen_to_world_pos(hover_pos, rect, camera, &ctx.map.map_data)
            {
                let (sx, sz) = super::placement::snap_placement_pos(
                    ctx.stats,
                    self.placement_object.as_deref(),
                    self.placement_direction,
                    wx as u32,
                    wz as u32,
                );
                let new_pos = Some((sx, sz));
                if self.preview_pos != new_pos {
                    self.preview_pos = new_pos;
                    self.preview_valid = super::placement::validate_placement(
                        ctx.map,
                        ctx.stats,
                        self.placement_object.as_deref(),
                        self.placement_direction,
                        sx,
                        sz,
                        shift_held,
                    );
                    *objects_dirty = true;
                }
            } else {
                self.clear_preview();
            }
        } else {
            self.clear_preview();
        }

        if clicked
            && self.preview_valid
            && let Some((sx, sz)) = self.preview_pos
        {
            let cmd = place_object_command(
                ctx,
                self.placement_object.as_deref(),
                self.placement_player,
                self.placement_direction,
                sx,
                sz,
            );
            *objects_dirty = true;
            // Re-validate immediately: the freshly placed object may now
            // block this tile, so the preview flips to red without waiting
            // for the cursor to move.
            self.preview_valid = super::placement::validate_placement(
                ctx.map,
                ctx.stats,
                self.placement_object.as_deref(),
                self.placement_direction,
                sx,
                sz,
                shift_held,
            );
            return cmd;
        }
        None
    }

    fn help_text(&self, keymap: &crate::keybindings::Keymap) -> Option<String> {
        let rot_key = keymap.shortcut_text(crate::keybindings::Action::RotatePlacement);
        Some(format!(
            "RMB+WASD=fly | LMB=place | {rot_key}=rotate 90\u{00b0}"
        ))
    }

    fn properties_ui(&mut self, ui: &mut egui::Ui, _ctx: &mut ToolCtx<'_>) {
        ui.heading("Place Object");
        if let Some(ref name) = self.placement_object {
            ui.label(format!("Placing: {name}"));
            ui.label(format!("Player: {}", self.placement_player));
            let deg = wz_maplib::constants::direction_to_degrees(self.placement_direction);
            ui.label(format!("Direction: {deg:.0}\u{00b0}"));
            ui.label("Press R to rotate 90\u{00b0}");
        } else {
            ui.label("Select an object in the Asset browser, then click to place.");
        }
    }
}

fn get_object_pos(map: &wz_maplib::WzMap, sel: SelectedObject) -> Option<WorldPos> {
    match sel {
        SelectedObject::Structure(i) => map.structures.get(i).map(|s| s.position),
        SelectedObject::Droid(i) => map.droids.get(i).map(|d| d.position),
        SelectedObject::Feature(i) => map.features.get(i).map(|f| f.position),
        SelectedObject::Label(i) => map.labels.get(i).map(|(_, l)| l.center()),
        SelectedObject::Gateway(_) => None,
    }
}

fn set_object_pos(map: &mut wz_maplib::WzMap, sel: SelectedObject, pos: WorldPos) {
    match sel {
        SelectedObject::Structure(i) => {
            if let Some(s) = map.structures.get_mut(i) {
                s.position = pos;
            }
        }
        SelectedObject::Droid(i) => {
            if let Some(d) = map.droids.get_mut(i) {
                d.position = pos;
            }
        }
        SelectedObject::Feature(i) => {
            if let Some(f) = map.features.get_mut(i) {
                f.position = pos;
            }
        }
        SelectedObject::Label(i) => {
            if let Some((_, label)) = map.labels.get_mut(i) {
                let old_center = label.center();
                let dx = pos.x as i64 - old_center.x as i64;
                let dy = pos.y as i64 - old_center.y as i64;
                match label {
                    wz_maplib::labels::ScriptLabel::Position { pos: p, .. } => {
                        p[0] = pos.x;
                        p[1] = pos.y;
                    }
                    wz_maplib::labels::ScriptLabel::Area { pos1, pos2, .. } => {
                        pos1[0] = (pos1[0] as i64 + dx).max(0) as u32;
                        pos1[1] = (pos1[1] as i64 + dy).max(0) as u32;
                        pos2[0] = (pos2[0] as i64 + dx).max(0) as u32;
                        pos2[1] = (pos2[1] as i64 + dy).max(0) as u32;
                    }
                }
            }
        }
        // Gateway drag-move isn't wired up; edit via the property panel.
        SelectedObject::Gateway(_) => {}
    }
}

fn apply_drag_move(
    map: &mut wz_maplib::WzMap,
    start_positions: &[(SelectedObject, WorldPos)],
    raw_dx: f32,
    raw_dz: f32,
) {
    // Snap to the coarsest grid in the selection so the group moves in
    // lockstep: structures = full tile, features = half tile,
    // droids/labels = free.
    let has_structures = start_positions
        .iter()
        .any(|(o, _)| matches!(o, SelectedObject::Structure(_)));
    let has_features = start_positions
        .iter()
        .any(|(o, _)| matches!(o, SelectedObject::Feature(_)));

    let (dx, dz) = if has_structures {
        let dx = (raw_dx / TILE_UNITS).round() * TILE_UNITS;
        let dz = (raw_dz / TILE_UNITS).round() * TILE_UNITS;
        (dx, dz)
    } else if has_features {
        let half = TILE_UNITS * 0.5;
        let dx = (raw_dx / half).round() * half;
        let dz = (raw_dz / half).round() * half;
        (dx, dz)
    } else {
        (raw_dx, raw_dz)
    };

    for (obj, orig_pos) in start_positions {
        let new_x = (orig_pos.x as f32 + dx).max(0.0) as u32;
        let new_y = (orig_pos.y as f32 + dz).max(0.0) as u32;
        set_object_pos(map, *obj, WorldPos { x: new_x, y: new_y });
    }
}

/// Build a compound move command from `start_positions` -> the map's
/// current positions. Returns `None` if no object actually moved.
fn build_multi_move_command(
    map: &wz_maplib::WzMap,
    start_positions: &[(SelectedObject, WorldPos)],
) -> Option<Box<dyn EditCommand>> {
    let mut commands: Vec<Box<dyn EditCommand>> = Vec::new();

    for (obj, old_pos) in start_positions {
        let new_pos = match obj {
            SelectedObject::Structure(i) => map.structures.get(*i).map(|s| s.position),
            SelectedObject::Droid(i) => map.droids.get(*i).map(|d| d.position),
            SelectedObject::Feature(i) => map.features.get(*i).map(|f| f.position),
            SelectedObject::Label(i) => map.labels.get(*i).map(|(_, l)| l.center()),
            SelectedObject::Gateway(_) => None,
        };
        let Some(new_pos) = new_pos else {
            continue;
        };
        if old_pos.x == new_pos.x && old_pos.y == new_pos.y {
            continue;
        }

        let cmd: Box<dyn EditCommand> = if let SelectedObject::Label(i) = obj {
            let Some((_, new_label)) = map.labels.get(*i) else {
                continue;
            };
            let dx = new_pos.x as i64 - old_pos.x as i64;
            let dy = new_pos.y as i64 - old_pos.y as i64;
            let old_label = revert_label(new_label, dx, dy);
            Box::new(super::label_tool::MoveLabelCommand {
                index: *i,
                old_label,
                new_label: new_label.clone(),
            })
        } else {
            let (kind, index) = match obj {
                SelectedObject::Structure(i) => (super::object_edit::ObjectKind::Structure, *i),
                SelectedObject::Droid(i) => (super::object_edit::ObjectKind::Droid, *i),
                SelectedObject::Feature(i) => (super::object_edit::ObjectKind::Feature, *i),
                SelectedObject::Label(_) | SelectedObject::Gateway(_) => unreachable!(),
            };
            Box::new(super::object_edit::MoveObjectCommand {
                kind,
                index,
                old_pos: *old_pos,
                new_pos,
            })
        };
        commands.push(cmd);
    }

    if commands.is_empty() {
        None
    } else {
        Some(Box::new(CompoundCommand::new(commands)))
    }
}

/// Reconstruct the old label state by reversing the move delta.
fn revert_label(
    current: &wz_maplib::labels::ScriptLabel,
    dx: i64,
    dy: i64,
) -> wz_maplib::labels::ScriptLabel {
    match current {
        wz_maplib::labels::ScriptLabel::Position { label, pos } => {
            wz_maplib::labels::ScriptLabel::Position {
                label: label.clone(),
                pos: [
                    (pos[0] as i64 - dx).max(0) as u32,
                    (pos[1] as i64 - dy).max(0) as u32,
                ],
            }
        }
        wz_maplib::labels::ScriptLabel::Area {
            label, pos1, pos2, ..
        } => wz_maplib::labels::ScriptLabel::Area {
            label: label.clone(),
            pos1: [
                (pos1[0] as i64 - dx).max(0) as u32,
                (pos1[1] as i64 - dy).max(0) as u32,
            ],
            pos2: [
                (pos2[0] as i64 - dx).max(0) as u32,
                (pos2[1] as i64 - dy).max(0) as u32,
            ],
        },
    }
}

/// Place a new object at the snapped world position using the current
/// placement settings. Returns one compound command (one entry per
/// mirror reflection) so undo lands the whole placement in one step.
fn place_object_command(
    ctx: &mut ToolCtx<'_>,
    placement_object: Option<&str>,
    placement_player: i8,
    placement_direction: u16,
    world_x: u32,
    world_z: u32,
) -> Option<Box<dyn EditCommand>> {
    let Some(obj_name) = placement_object else {
        ctx.log("No object selected for placement. Use the Asset Browser to select one.");
        return None;
    };

    let stats = ctx.stats;
    let is_structure = stats.is_some_and(|s| s.structures.contains_key(obj_name));
    let is_feature = stats.is_some_and(|s| s.features.contains_key(obj_name));
    let is_template = stats.is_some_and(|s| s.templates.contains_key(obj_name));

    if !is_structure && !is_feature && !is_template {
        log::warn!("place_object: '{obj_name}' not found in structures, features, or templates");
    }

    let map_w = ctx.map.map_data.width;
    let map_h = ctx.map.map_data.height;
    let mirror_pts =
        tools::mirror::mirror_world_points(world_x, world_z, map_w, map_h, ctx.mirror_mode);

    let mut commands: Vec<Box<dyn EditCommand>> = Vec::with_capacity(mirror_pts.len());
    for (point_idx, &(mx, mz)) in mirror_pts.iter().enumerate() {
        let m_dir =
            tools::mirror::mirror_direction(placement_direction, ctx.mirror_mode, point_idx);
        let pos = WorldPos { x: mx, y: mz };

        let cmd: Box<dyn EditCommand> = if is_structure {
            let structure = wz_maplib::objects::Structure {
                name: obj_name.to_owned(),
                position: pos,
                direction: m_dir,
                player: placement_player,
                modules: 0,
                id: None,
            };
            super::placement::build_placement_with_wall_replace(ctx.map, stats, structure)
        } else if is_feature {
            let feature = wz_maplib::objects::Feature {
                name: obj_name.to_owned(),
                position: pos,
                direction: m_dir,
                id: None,
                player: Some(placement_player),
            };
            Box::new(super::object_edit::PlaceFeatureCommand { feature })
        } else {
            let droid = wz_maplib::objects::Droid {
                name: obj_name.to_owned(),
                position: pos,
                direction: m_dir,
                player: placement_player,
                id: None,
            };
            Box::new(super::object_edit::PlaceDroidCommand { droid })
        };
        cmd.execute(ctx.map);
        commands.push(cmd);
    }

    if commands.is_empty() {
        None
    } else {
        Some(Box::new(CompoundCommand::new(commands)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_tool_defaults() {
        let tool = ObjectSelectTool::default();
        assert!(!tool.drag_select_active);
        assert!(!tool.dragging_object());
    }

    #[test]
    fn select_tool_reset_clears_drag_state() {
        let mut tool = ObjectSelectTool {
            drag_select_active: true,
            drag_select_start: Some(egui::Pos2::new(10.0, 20.0)),
            dragging_object: true,
            drag_start_world: Some((5.0, 6.0)),
            drag_start_positions: vec![(SelectedObject::Structure(0), WorldPos { x: 1, y: 2 })],
        };
        tool.reset_transient();
        assert!(!tool.drag_select_active);
        assert!(tool.drag_select_start.is_none());
        assert!(!tool.dragging_object);
        assert!(tool.drag_start_world.is_none());
        assert!(tool.drag_start_positions.is_empty());
    }

    #[test]
    fn place_tool_clear_preview_resets_state() {
        let mut tool = ObjectPlaceTool {
            placement_object: Some("Tower".into()),
            placement_player: 3,
            placement_direction: 0x4000,
            preview_pos: Some((10, 20)),
            preview_valid: true,
        };
        tool.clear_preview();
        assert!(tool.preview_pos.is_none());
        assert!(!tool.preview_valid);
        // Asset selection must survive the preview reset; switching tools
        // shouldn't lose the user's chosen asset.
        assert_eq!(tool.placement_object.as_deref(), Some("Tower"));
        assert_eq!(tool.placement_player, 3);
    }

    #[test]
    fn apply_drag_move_snaps_to_full_tile_for_structures() {
        let mut map = wz_maplib::WzMap::new("test", 32, 32);
        map.structures.push(wz_maplib::objects::Structure {
            name: "S".into(),
            position: WorldPos { x: 256, y: 256 },
            direction: 0,
            player: 0,
            modules: 0,
            id: None,
        });
        let starts = vec![(SelectedObject::Structure(0), WorldPos { x: 256, y: 256 })];
        // Drag 70 units east (just over half a tile of 128); expect snap to 128.
        apply_drag_move(&mut map, &starts, 70.0, 0.0);
        assert_eq!(map.structures[0].position.x, 384);
        assert_eq!(map.structures[0].position.y, 256);
    }
}
