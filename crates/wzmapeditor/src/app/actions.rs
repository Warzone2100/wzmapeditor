//! User-triggered actions: validation, object deletion, and object rotation.

use super::{EditorApp, SelectedObject};

/// 90 degrees in WZ2100 internal direction units (0..65535 == 0..360 deg).
const QUARTER_TURN: u16 = 0x4000;

/// Run all validation checks on the current map. Returns `true` if problems were found.
pub(super) fn run_validation(app: &mut EditorApp) -> bool {
    let Some(ref doc) = app.document else {
        return false;
    };
    let stats_bridge = app
        .stats
        .as_ref()
        .map(crate::map::stats_bridge::StatsBridge);
    let stats_ref: Option<&dyn wz_maplib::validate::StatsLookup> = stats_bridge
        .as_ref()
        .map(|b| b as &dyn wz_maplib::validate::StatsLookup);
    let results =
        wz_maplib::validate::validate_map(&doc.map, stats_ref, &app.config.validation_config);
    let has_problems = results.has_problems();
    app.validation_results = Some(results);
    has_problems
}

/// Delete all currently selected objects with undo support.
pub(super) fn delete_selected_objects(app: &mut EditorApp) {
    if app.selection.is_empty() {
        return;
    }

    let selected = app.selection.objects.clone();
    app.selection.clear();

    let Some(doc) = app.document.as_mut() else {
        return;
    };

    let mut commands: Vec<Box<dyn crate::map::history::EditCommand>> = Vec::new();

    // Sort indices descending so earlier deletions don't shift later ones.
    let mut struct_indices: Vec<usize> = selected
        .iter()
        .filter_map(|s| match s {
            SelectedObject::Structure(i) => Some(*i),
            _ => None,
        })
        .collect();
    struct_indices.sort_unstable_by(|a, b| b.cmp(a));

    let mut droid_indices: Vec<usize> = selected
        .iter()
        .filter_map(|s| match s {
            SelectedObject::Droid(i) => Some(*i),
            _ => None,
        })
        .collect();
    droid_indices.sort_unstable_by(|a, b| b.cmp(a));

    let mut feature_indices: Vec<usize> = selected
        .iter()
        .filter_map(|s| match s {
            SelectedObject::Feature(i) => Some(*i),
            _ => None,
        })
        .collect();
    feature_indices.sort_unstable_by(|a, b| b.cmp(a));

    let mut label_indices: Vec<usize> = selected
        .iter()
        .filter_map(|s| match s {
            SelectedObject::Label(i) => Some(*i),
            _ => None,
        })
        .collect();
    label_indices.sort_unstable_by(|a, b| b.cmp(a));

    let mut gateway_indices: Vec<usize> = selected
        .iter()
        .filter_map(|s| match s {
            SelectedObject::Gateway(i) => Some(*i),
            _ => None,
        })
        .collect();
    gateway_indices.sort_unstable_by(|a, b| b.cmp(a));

    for i in struct_indices {
        if i < doc.map.structures.len() {
            let obj = doc.map.structures[i].clone();
            commands.push(Box::new(
                crate::tools::object_edit::DeleteObjectCommand::structure(i, obj),
            ));
        }
    }
    for i in droid_indices {
        if i < doc.map.droids.len() {
            let obj = doc.map.droids[i].clone();
            commands.push(Box::new(
                crate::tools::object_edit::DeleteObjectCommand::droid(i, obj),
            ));
        }
    }
    for i in feature_indices {
        if i < doc.map.features.len() {
            let obj = doc.map.features[i].clone();
            commands.push(Box::new(
                crate::tools::object_edit::DeleteObjectCommand::feature(i, obj),
            ));
        }
    }
    for i in label_indices {
        if i < doc.map.labels.len() {
            let (key, label) = doc.map.labels[i].clone();
            commands.push(Box::new(crate::tools::label_tool::DeleteLabelCommand {
                index: i,
                saved_key: key,
                saved_label: label,
            }));
        }
    }
    for i in gateway_indices {
        if i < doc.map.map_data.gateways.len() {
            let saved = doc.map.map_data.gateways[i];
            commands.push(Box::new(crate::tools::gateway_tool::DeleteGatewayCommand {
                index: i,
                saved,
            }));
        }
    }

    if commands.is_empty() {
        return;
    }

    let cmd: Box<dyn crate::map::history::EditCommand> =
        Box::new(crate::map::history::CompoundCommand::new(commands));

    cmd.execute(&mut doc.map);
    doc.history.push_already_applied(cmd);
    doc.dirty = true;
    app.objects_dirty = true;
    app.log("Deleted object");
}

/// Rotate every selected structure / droid / feature by a quarter turn.
/// Returns true when at least one object was rotated.
pub(super) fn rotate_selected_objects(app: &mut EditorApp) -> bool {
    use crate::tools::object_edit::{ObjectKind, RotateObjectCommand};

    if app.selection.is_empty() {
        return false;
    }
    let Some(doc) = app.document.as_mut() else {
        return false;
    };

    let mut commands: Vec<Box<dyn crate::map::history::EditCommand>> = Vec::new();

    for sel in &app.selection.objects {
        let (kind, index, current) = match *sel {
            SelectedObject::Structure(i) => match doc.map.structures.get(i) {
                Some(s) => (ObjectKind::Structure, i, s.direction),
                None => continue,
            },
            SelectedObject::Droid(i) => match doc.map.droids.get(i) {
                Some(d) => (ObjectKind::Droid, i, d.direction),
                None => continue,
            },
            SelectedObject::Feature(i) => match doc.map.features.get(i) {
                Some(f) => (ObjectKind::Feature, i, f.direction),
                None => continue,
            },
            SelectedObject::Label(_) | SelectedObject::Gateway(_) => continue,
        };
        commands.push(Box::new(RotateObjectCommand {
            kind,
            index,
            old_direction: current,
            new_direction: current.wrapping_add(QUARTER_TURN),
        }));
    }

    if commands.is_empty() {
        return false;
    }

    let count = commands.len();
    let cmd: Box<dyn crate::map::history::EditCommand> =
        Box::new(crate::map::history::CompoundCommand::new(commands));
    cmd.execute(&mut doc.map);
    doc.history.push_already_applied(cmd);
    doc.dirty = true;
    app.objects_dirty = true;
    app.shadow_dirty = true;
    app.log(format!("Rotated {count} object(s) 90\u{00b0}"));
    true
}
