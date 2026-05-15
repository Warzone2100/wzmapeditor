//! Duplicate the current selection (Ctrl+D).
//!
//! Two modes:
//!
//! - **Idle**: duplicate selected structures / droids / features at a one-tile
//!   offset and re-select the copies so the next Ctrl+D walks further along.
//! - **Mid-drag**: stamp a copy of every selected object at its current
//!   dragged position without interrupting the drag, so repeated Ctrl+D
//!   leaves a trail of copies while the user moves the originals.
//!
//! Labels are excluded because their keys must stay unique per map.

use wz_maplib::constants::TILE_UNITS;
use wz_maplib::objects::WorldPos;

use crate::map::history::{CompoundCommand, EditCommand};
use crate::tools::object_edit::{PlaceDroidCommand, PlaceFeatureCommand, PlaceStructureCommand};

use super::{EditorApp, SelectedObject, Selection};

pub(super) fn duplicate_selection(app: &mut EditorApp) {
    if app.selection.is_empty() {
        app.log("Duplicate: nothing selected");
        return;
    }

    let mid_drag = app
        .tool_state
        .object_select()
        .is_some_and(crate::tools::object_tools::ObjectSelectTool::dragging_object);
    if mid_drag {
        stamp_at_current_positions(app);
    } else {
        duplicate_with_offset(app, i64::from(TILE_UNITS), 0);
    }
}

/// Snapshot and re-place each selected object at its current in-map position.
/// Leaves the drag state alone so the originals keep moving.
fn stamp_at_current_positions(app: &mut EditorApp) {
    let Some(doc) = app.document.as_mut() else {
        return;
    };
    if doc.is_read_only() {
        return;
    }

    let mut commands: Vec<Box<dyn EditCommand>> = Vec::new();
    let mut count = 0_usize;

    for sel in &app.selection.objects {
        match *sel {
            SelectedObject::Structure(i) => {
                if let Some(s) = doc.map.structures.get(i) {
                    let mut clone = s.clone();
                    clone.id = None;
                    commands.push(Box::new(PlaceStructureCommand { structure: clone }));
                    count += 1;
                }
            }
            SelectedObject::Droid(i) => {
                if let Some(d) = doc.map.droids.get(i) {
                    let mut clone = d.clone();
                    clone.id = None;
                    commands.push(Box::new(PlaceDroidCommand { droid: clone }));
                    count += 1;
                }
            }
            SelectedObject::Feature(i) => {
                if let Some(f) = doc.map.features.get(i) {
                    let mut clone = f.clone();
                    clone.id = None;
                    commands.push(Box::new(PlaceFeatureCommand { feature: clone }));
                    count += 1;
                }
            }
            SelectedObject::Label(_) | SelectedObject::Gateway(_) => {}
        }
    }

    if commands.is_empty() {
        app.log("Duplicate: selection has nothing to stamp");
        return;
    }

    let compound: Box<dyn EditCommand> = Box::new(CompoundCommand::new(commands));
    compound.execute(&mut doc.map);
    doc.history.push_already_applied(compound);
    doc.dirty = true;
    app.objects_dirty = true;
    app.log(format!("Stamped {count} object(s) at drag position"));
}

#[derive(Clone, Copy)]
enum Kind {
    Structure,
    Droid,
    Feature,
}

/// Place a copy of each selected object offset by `(dx, dz)` world units,
/// then move the selection onto the new copies.
fn duplicate_with_offset(app: &mut EditorApp, dx: i64, dz: i64) {
    let Some(doc) = app.document.as_mut() else {
        return;
    };
    if doc.is_read_only() {
        return;
    }

    let max_x = i64::from(doc.map.map_data.width) * i64::from(TILE_UNITS);
    let max_y = i64::from(doc.map.map_data.height) * i64::from(TILE_UNITS);

    let start_structures = doc.map.structures.len();
    let start_droids = doc.map.droids.len();
    let start_features = doc.map.features.len();

    let mut commands: Vec<Box<dyn EditCommand>> = Vec::new();
    let mut kinds: Vec<Kind> = Vec::new();

    for sel in &app.selection.objects {
        match *sel {
            SelectedObject::Structure(i) => {
                if let Some(s) = doc.map.structures.get(i) {
                    let mut clone = s.clone();
                    clone.position = offset_pos(clone.position, dx, dz, max_x, max_y);
                    clone.id = None;
                    commands.push(Box::new(PlaceStructureCommand { structure: clone }));
                    kinds.push(Kind::Structure);
                }
            }
            SelectedObject::Droid(i) => {
                if let Some(d) = doc.map.droids.get(i) {
                    let mut clone = d.clone();
                    clone.position = offset_pos(clone.position, dx, dz, max_x, max_y);
                    clone.id = None;
                    commands.push(Box::new(PlaceDroidCommand { droid: clone }));
                    kinds.push(Kind::Droid);
                }
            }
            SelectedObject::Feature(i) => {
                if let Some(f) = doc.map.features.get(i) {
                    let mut clone = f.clone();
                    clone.position = offset_pos(clone.position, dx, dz, max_x, max_y);
                    clone.id = None;
                    commands.push(Box::new(PlaceFeatureCommand { feature: clone }));
                    kinds.push(Kind::Feature);
                }
            }
            SelectedObject::Label(_) | SelectedObject::Gateway(_) => {}
        }
    }

    if commands.is_empty() {
        app.log("Duplicate: selection has nothing to duplicate");
        return;
    }

    let count = commands.len();
    let compound: Box<dyn EditCommand> = Box::new(CompoundCommand::new(commands));
    compound.execute(&mut doc.map);
    doc.history.push_already_applied(compound);
    doc.dirty = true;

    let mut new_sel = Selection::default();
    let mut struct_i = start_structures;
    let mut droid_i = start_droids;
    let mut feature_i = start_features;
    for kind in &kinds {
        let obj = match kind {
            Kind::Structure => {
                let o = SelectedObject::Structure(struct_i);
                struct_i += 1;
                o
            }
            Kind::Droid => {
                let o = SelectedObject::Droid(droid_i);
                droid_i += 1;
                o
            }
            Kind::Feature => {
                let o = SelectedObject::Feature(feature_i);
                feature_i += 1;
                o
            }
        };
        new_sel.objects.push(obj);
    }
    app.selection = new_sel;
    app.objects_dirty = true;
    app.log(format!("Duplicated {count} object(s)"));
}

fn offset_pos(pos: WorldPos, dx: i64, dz: i64, max_x: i64, max_y: i64) -> WorldPos {
    let new_x = (i64::from(pos.x) + dx).clamp(0, max_x.saturating_sub(1));
    let new_y = (i64::from(pos.y) + dz).clamp(0, max_y.saturating_sub(1));
    WorldPos {
        x: new_x as u32,
        y: new_y as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_pos_clamps_to_bounds() {
        let max = 64 * i64::from(TILE_UNITS);
        let p = offset_pos(WorldPos { x: 10, y: 10 }, -1000, -1000, max, max);
        assert_eq!(p.x, 0);
        assert_eq!(p.y, 0);

        let upper = max as u32 - 5;
        let p = offset_pos(WorldPos { x: upper, y: upper }, 1000, 1000, max, max);
        assert_eq!(i64::from(p.x), max - 1);
        assert_eq!(i64::from(p.y), max - 1);
    }

    #[test]
    fn offset_pos_applies_delta() {
        let max = 64 * i64::from(TILE_UNITS);
        let p = offset_pos(
            WorldPos { x: 100, y: 200 },
            i64::from(TILE_UNITS),
            0,
            max,
            max,
        );
        assert_eq!(i64::from(p.x), 100 + i64::from(TILE_UNITS));
        assert_eq!(p.y, 200);
    }
}
