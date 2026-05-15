//! Right sidebar editor for the current selection. Tool settings live in
//! the Terrain tool palette.

use egui::Ui;

use wz_maplib::constants::{MAX_PLAYERS, PLAYER_SCAVENGERS, TILE_SHIFT};

use crate::app::EditorApp;
use crate::map::history::EditCommand;

/// 90 degrees in WZ2100 internal units (0-65535 maps to 0-360).
const DIRECTION_QUARTER: u16 = 16384;

/// Snap a raw direction value to the nearest 90 degree step (0..=3).
fn direction_to_step(dir: u16) -> u8 {
    (((u32::from(dir) + u32::from(DIRECTION_QUARTER) / 2) / u32::from(DIRECTION_QUARTER)) % 4) as u8
}

/// Render the 4 rotation buttons (0/90/180/270). Pass `None` for the
/// multi-select "varies" case.
fn rotation_buttons(ui: &mut Ui, current: Option<u8>) -> Option<u8> {
    let mut clicked = None;
    for s in 0u8..4 {
        let label = format!("{}°", u32::from(s) * 90);
        let btn = egui::Button::new(label)
            .small()
            .selected(current == Some(s));
        if ui.add(btn).clicked() {
            clicked = Some(s);
        }
    }
    clicked
}

fn rotation_widget(ui: &mut Ui, dir: &mut u16) -> bool {
    let cur = direction_to_step(*dir);
    match rotation_buttons(ui, Some(cur)) {
        Some(s) if s != cur => {
            *dir = u16::from(s) * DIRECTION_QUARTER;
            true
        }
        _ => false,
    }
}

fn player_label(p: i8) -> String {
    if p == PLAYER_SCAVENGERS {
        "Scavenger".to_string()
    } else {
        format!("Player {p}")
    }
}

fn player_widget(ui: &mut Ui, p: &mut i8, salt: impl std::hash::Hash) -> bool {
    let original = *p;
    egui::ComboBox::from_id_salt(salt)
        .selected_text(player_label(*p))
        .show_ui(ui, |ui| {
            ui.selectable_value(p, PLAYER_SCAVENGERS, "Scavenger");
            for n in 0..MAX_PLAYERS as i8 {
                ui.selectable_value(p, n, format!("Player {n}"));
            }
        });
    *p != original
}

fn show_tile_coords(ui: &mut Ui, world_x: u32, world_y: u32) {
    let tx = world_x >> TILE_SHIFT;
    let ty = world_y >> TILE_SHIFT;
    ui.label(
        egui::RichText::new(format!("Tile: ({tx}, {ty})"))
            .small()
            .weak(),
    );
}

fn show_id_field(ui: &mut Ui, id: Option<u32>) {
    let text = match id {
        Some(v) => format!("ID: {v}"),
        None => "ID: (unassigned)".to_string(),
    };
    ui.label(egui::RichText::new(text).small().weak());
}

pub fn show_property_panel(ui: &mut Ui, app: &mut EditorApp) {
    ui.heading("Selection");
    ui.separator();

    if app.document.is_none() {
        ui.label("No map loaded.");
        ui.label("Use File > New Map or Open to get started.");
        return;
    }

    show_selected_object_props(ui, app);

    if matches!(
        app.tool_state.active_tool,
        crate::tools::ToolId::Gateway | crate::tools::ToolId::ScriptLabel
    ) {
        ui.separator();
        match app.tool_state.active_tool {
            crate::tools::ToolId::Gateway => show_gateway_list(ui, app),
            crate::tools::ToolId::ScriptLabel => show_label_list(ui, app),
            _ => {}
        }
    }
}

fn show_selected_object_props(ui: &mut Ui, app: &mut EditorApp) {
    if app.selection.len() > 1 {
        show_multi_selection_props(ui, app);
        return;
    }
    let Some(sel) = app.selection.single() else {
        ui.label("Click objects in the viewport to select them.");
        return;
    };

    let Some(doc) = app.document.as_mut() else {
        return;
    };
    if doc.is_read_only() {
        ui.label("Script map: read-only.");
        return;
    }

    match sel {
        crate::app::SelectedObject::Structure(i) => {
            if let Some(s) = doc.map.structures.get_mut(i) {
                ui.label(format!("Structure: {}", s.name));
                show_id_field(ui, s.id);

                let mut changed = false;

                ui.horizontal(|ui| {
                    ui.label("X:");
                    let mut x = s.position.x as i32;
                    if ui.add(egui::DragValue::new(&mut x).speed(16)).changed() {
                        s.position.x = x.max(0) as u32;
                        changed = true;
                    }
                    ui.label("Y:");
                    let mut y = s.position.y as i32;
                    if ui.add(egui::DragValue::new(&mut y).speed(16)).changed() {
                        s.position.y = y.max(0) as u32;
                        changed = true;
                    }
                });
                show_tile_coords(ui, s.position.x, s.position.y);

                ui.horizontal(|ui| {
                    ui.label("Rotation:");
                    if rotation_widget(ui, &mut s.direction) {
                        changed = true;
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Player:");
                    if player_widget(ui, &mut s.player, ("struct_player", i)) {
                        changed = true;
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Modules:");
                    let mut m = s.modules as i32;
                    if ui.add(egui::DragValue::new(&mut m).range(0..=4)).changed() {
                        s.modules = m as u8;
                        changed = true;
                    }
                });

                if changed {
                    doc.dirty = true;
                    app.objects_dirty = true;
                }
            }
        }
        crate::app::SelectedObject::Droid(i) => {
            if let Some(d) = doc.map.droids.get_mut(i) {
                let droid_name = d.name.clone();
                ui.label(format!("Droid: {droid_name}"));
                show_id_field(ui, d.id);

                let mut changed = false;

                ui.horizontal(|ui| {
                    ui.label("X:");
                    let mut x = d.position.x as i32;
                    if ui.add(egui::DragValue::new(&mut x).speed(16)).changed() {
                        d.position.x = x.max(0) as u32;
                        changed = true;
                    }
                    ui.label("Y:");
                    let mut y = d.position.y as i32;
                    if ui.add(egui::DragValue::new(&mut y).speed(16)).changed() {
                        d.position.y = y.max(0) as u32;
                        changed = true;
                    }
                });
                show_tile_coords(ui, d.position.x, d.position.y);

                ui.horizontal(|ui| {
                    ui.label("Rotation:");
                    if rotation_widget(ui, &mut d.direction) {
                        changed = true;
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Player:");
                    if player_widget(ui, &mut d.player, ("droid_player", i)) {
                        changed = true;
                    }
                });

                if changed {
                    doc.dirty = true;
                    app.objects_dirty = true;
                }
            }
        }
        crate::app::SelectedObject::Feature(i) => {
            if let Some(f) = doc.map.features.get_mut(i) {
                ui.label(format!("Feature: {}", f.name));
                show_id_field(ui, f.id);

                let mut changed = false;

                ui.horizontal(|ui| {
                    ui.label("X:");
                    let mut x = f.position.x as i32;
                    if ui.add(egui::DragValue::new(&mut x).speed(16)).changed() {
                        f.position.x = x.max(0) as u32;
                        changed = true;
                    }
                    ui.label("Y:");
                    let mut y = f.position.y as i32;
                    if ui.add(egui::DragValue::new(&mut y).speed(16)).changed() {
                        f.position.y = y.max(0) as u32;
                        changed = true;
                    }
                });
                show_tile_coords(ui, f.position.x, f.position.y);

                ui.horizontal(|ui| {
                    ui.label("Rotation:");
                    if rotation_widget(ui, &mut f.direction) {
                        changed = true;
                    }
                });

                if changed {
                    doc.dirty = true;
                    app.objects_dirty = true;
                }
            }
        }
        crate::app::SelectedObject::Label(i) => {
            if let Some((key, label)) = doc.map.labels.get_mut(i) {
                let type_name = match label {
                    wz_maplib::labels::ScriptLabel::Position { .. } => "Position",
                    wz_maplib::labels::ScriptLabel::Area { .. } => "Area",
                };
                ui.label(format!("Label: {} ({type_name})", label.label()));
                ui.label(format!("Key: {key}"));

                let mut changed = false;

                match label {
                    wz_maplib::labels::ScriptLabel::Position { pos, label: name } => {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            if ui.text_edit_singleline(name).changed() {
                                changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("X:");
                            let mut x = pos[0] as i32;
                            if ui.add(egui::DragValue::new(&mut x).speed(16)).changed() {
                                pos[0] = x.max(0) as u32;
                                changed = true;
                            }
                            ui.label("Y:");
                            let mut y = pos[1] as i32;
                            if ui.add(egui::DragValue::new(&mut y).speed(16)).changed() {
                                pos[1] = y.max(0) as u32;
                                changed = true;
                            }
                        });
                    }
                    wz_maplib::labels::ScriptLabel::Area {
                        pos1,
                        pos2,
                        label: name,
                    } => {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            if ui.text_edit_singleline(name).changed() {
                                changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("X1:");
                            let mut x = pos1[0] as i32;
                            if ui.add(egui::DragValue::new(&mut x).speed(16)).changed() {
                                pos1[0] = x.max(0) as u32;
                                changed = true;
                            }
                            ui.label("Y1:");
                            let mut y = pos1[1] as i32;
                            if ui.add(egui::DragValue::new(&mut y).speed(16)).changed() {
                                pos1[1] = y.max(0) as u32;
                                changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("X2:");
                            let mut x = pos2[0] as i32;
                            if ui.add(egui::DragValue::new(&mut x).speed(16)).changed() {
                                pos2[0] = x.max(0) as u32;
                                changed = true;
                            }
                            ui.label("Y2:");
                            let mut y = pos2[1] as i32;
                            if ui.add(egui::DragValue::new(&mut y).speed(16)).changed() {
                                pos2[1] = y.max(0) as u32;
                                changed = true;
                            }
                        });
                    }
                }

                if changed {
                    doc.dirty = true;
                    app.validation_dirty = true;
                }
            }
        }
        crate::app::SelectedObject::Gateway(i) => {
            if let Some(gw) = doc.map.map_data.gateways.get_mut(i) {
                ui.label(format!("Gateway #{i}"));

                let mut changed = false;
                let max_x = doc.map.map_data.width.saturating_sub(1) as i32;
                let max_y = doc.map.map_data.height.saturating_sub(1) as i32;

                ui.horizontal(|ui| {
                    ui.label("X1:");
                    let mut x = i32::from(gw.x1);
                    if ui
                        .add(egui::DragValue::new(&mut x).range(0..=max_x))
                        .changed()
                    {
                        gw.x1 = x.clamp(0, max_x) as u8;
                        changed = true;
                    }
                    ui.label("Y1:");
                    let mut y = i32::from(gw.y1);
                    if ui
                        .add(egui::DragValue::new(&mut y).range(0..=max_y))
                        .changed()
                    {
                        gw.y1 = y.clamp(0, max_y) as u8;
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("X2:");
                    let mut x = i32::from(gw.x2);
                    if ui
                        .add(egui::DragValue::new(&mut x).range(0..=max_x))
                        .changed()
                    {
                        gw.x2 = x.clamp(0, max_x) as u8;
                        changed = true;
                    }
                    ui.label("Y2:");
                    let mut y = i32::from(gw.y2);
                    if ui
                        .add(egui::DragValue::new(&mut y).range(0..=max_y))
                        .changed()
                    {
                        gw.y2 = y.clamp(0, max_y) as u8;
                        changed = true;
                    }
                });

                if changed {
                    doc.dirty = true;
                    app.validation_dirty = true;
                }
            }
        }
    }
}

/// Merge a per-object value into a running "common across selection" tracker.
/// Once values disagree, `varies` latches true and `common` clears.
fn merge_common<T: Eq + Copy>(common: &mut Option<T>, varies: &mut bool, value: T) {
    if *varies {
        return;
    }
    match *common {
        Some(v) if v != value => {
            *varies = true;
            *common = None;
        }
        Some(_) => {}
        None => *common = Some(value),
    }
}

/// Property editor shown when more than one object is selected.
fn show_multi_selection_props(ui: &mut Ui, app: &mut EditorApp) {
    use crate::app::SelectedObject;

    let Some(doc) = app.document.as_mut() else {
        return;
    };
    if doc.is_read_only() {
        ui.label("Script map: read-only.");
        return;
    }

    let mut struct_count = 0usize;
    let mut droid_count = 0usize;
    let mut feat_count = 0usize;
    let mut player_targets = 0usize;
    let mut rot_targets = 0usize;
    let mut common_player: Option<i8> = None;
    let mut player_varies = false;
    let mut common_rot: Option<u8> = None;
    let mut rot_varies = false;

    for obj in &app.selection.objects {
        match obj {
            SelectedObject::Structure(i) => {
                if let Some(s) = doc.map.structures.get(*i) {
                    struct_count += 1;
                    player_targets += 1;
                    rot_targets += 1;
                    merge_common(&mut common_player, &mut player_varies, s.player);
                    merge_common(
                        &mut common_rot,
                        &mut rot_varies,
                        direction_to_step(s.direction),
                    );
                }
            }
            SelectedObject::Droid(i) => {
                if let Some(d) = doc.map.droids.get(*i) {
                    droid_count += 1;
                    player_targets += 1;
                    rot_targets += 1;
                    merge_common(&mut common_player, &mut player_varies, d.player);
                    merge_common(
                        &mut common_rot,
                        &mut rot_varies,
                        direction_to_step(d.direction),
                    );
                }
            }
            SelectedObject::Feature(i) => {
                if let Some(f) = doc.map.features.get(*i) {
                    feat_count += 1;
                    rot_targets += 1;
                    merge_common(
                        &mut common_rot,
                        &mut rot_varies,
                        direction_to_step(f.direction),
                    );
                    if let Some(p) = f.player {
                        player_targets += 1;
                        merge_common(&mut common_player, &mut player_varies, p);
                    }
                }
            }
            SelectedObject::Label(_) | SelectedObject::Gateway(_) => {}
        }
    }

    let total = struct_count + droid_count + feat_count;
    ui.label(format!("{total} objects selected"));
    ui.label(
        egui::RichText::new(format!(
            "({struct_count} structs, {droid_count} droids, {feat_count} feats)"
        ))
        .small()
        .weak(),
    );

    if total == 0 {
        return;
    }

    let mut new_rot_step: Option<u8> = None;
    let mut new_player: Option<i8> = None;

    if rot_targets > 0 {
        ui.horizontal(|ui| {
            ui.label("Rotation:");
            if rot_varies {
                ui.label(egui::RichText::new("(varies)").weak());
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            let highlight = if rot_varies { None } else { common_rot };
            if let Some(s) = rotation_buttons(ui, highlight) {
                new_rot_step = Some(s);
            }
        });
    }

    if player_targets > 0 {
        ui.horizontal(|ui| {
            ui.label("Player:");
            let display = if player_varies {
                "(varies)".to_string()
            } else {
                common_player.map_or_else(String::new, player_label)
            };
            egui::ComboBox::from_id_salt("multi_player")
                .selected_text(display)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(false, "Scavenger").clicked() {
                        new_player = Some(PLAYER_SCAVENGERS);
                    }
                    for n in 0..MAX_PLAYERS as i8 {
                        if ui.selectable_label(false, format!("Player {n}")).clicked() {
                            new_player = Some(n);
                        }
                    }
                });
        });
    }

    if new_rot_step.is_none() && new_player.is_none() {
        return;
    }

    let mut changed = false;
    for obj in app.selection.objects.clone() {
        match obj {
            SelectedObject::Structure(i) => {
                if let Some(s) = doc.map.structures.get_mut(i) {
                    if let Some(step) = new_rot_step {
                        s.direction = u16::from(step) * DIRECTION_QUARTER;
                        changed = true;
                    }
                    if let Some(p) = new_player {
                        s.player = p;
                        changed = true;
                    }
                }
            }
            SelectedObject::Droid(i) => {
                if let Some(d) = doc.map.droids.get_mut(i) {
                    if let Some(step) = new_rot_step {
                        d.direction = u16::from(step) * DIRECTION_QUARTER;
                        changed = true;
                    }
                    if let Some(p) = new_player {
                        d.player = p;
                        changed = true;
                    }
                }
            }
            SelectedObject::Feature(i) => {
                if let Some(f) = doc.map.features.get_mut(i) {
                    if let Some(step) = new_rot_step {
                        f.direction = u16::from(step) * DIRECTION_QUARTER;
                        changed = true;
                    }
                    if let Some(p) = new_player
                        && f.player.is_some()
                    {
                        f.player = Some(p);
                        changed = true;
                    }
                }
            }
            SelectedObject::Label(_) | SelectedObject::Gateway(_) => {}
        }
    }

    if changed {
        doc.dirty = true;
        app.objects_dirty = true;
    }
}

fn show_gateway_list(ui: &mut Ui, app: &mut EditorApp) {
    let Some(doc) = app.document.as_ref() else {
        return;
    };

    if doc.map.map_data.gateways.is_empty() {
        ui.label("No gateways.");
        return;
    }

    ui.label(format!("{} gateways:", doc.map.map_data.gateways.len()));

    let mut delete_idx = None;

    egui::ScrollArea::vertical()
        .max_height(150.0)
        .show(ui, |ui| {
            for (i, gw) in doc.map.map_data.gateways.iter().enumerate() {
                ui.horizontal(|ui| {
                    let label = format!("#{}: ({},{}) - ({},{})", i, gw.x1, gw.y1, gw.x2, gw.y2);
                    ui.label(&label);
                    if ui.small_button("X").clicked() {
                        delete_idx = Some(i);
                    }
                });
            }
        });

    if let Some(idx) = delete_idx {
        let Some(doc) = app.document.as_mut() else {
            return;
        };
        if doc.is_read_only() {
            return;
        }
        let gw = doc.map.map_data.gateways[idx];
        let cmd = crate::tools::gateway_tool::DeleteGatewayCommand {
            index: idx,
            saved: gw,
        };
        cmd.execute(&mut doc.map);
        doc.history.push_already_applied(Box::new(cmd));
        doc.dirty = true;
        app.validation_dirty = true;
    }
}

fn show_label_list(ui: &mut Ui, app: &mut EditorApp) {
    let Some(doc) = app.document.as_ref() else {
        return;
    };

    if doc.map.labels.is_empty() {
        ui.label("No labels.");
        return;
    }

    ui.label(format!("{} labels:", doc.map.labels.len()));

    let mut delete_idx = None;

    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            for (i, (key, label)) in doc.map.labels.iter().enumerate() {
                ui.horizontal(|ui| {
                    let info = match label {
                        wz_maplib::labels::ScriptLabel::Position { label: name, pos } => {
                            format!("{key}: \"{name}\" ({}, {})", pos[0], pos[1])
                        }
                        wz_maplib::labels::ScriptLabel::Area {
                            label: name,
                            pos1,
                            pos2,
                        } => {
                            format!(
                                "{key}: \"{name}\" ({},{})..({},{})",
                                pos1[0], pos1[1], pos2[0], pos2[1]
                            )
                        }
                    };
                    ui.label(egui::RichText::new(&info).small());
                    if ui.small_button("X").clicked() {
                        delete_idx = Some(i);
                    }
                });
            }
        });

    if let Some(idx) = delete_idx {
        let Some(doc) = app.document.as_mut() else {
            return;
        };
        if doc.is_read_only() {
            return;
        }
        let (saved_key, saved_label) = doc.map.labels[idx].clone();
        let cmd = crate::tools::label_tool::DeleteLabelCommand {
            index: idx,
            saved_key,
            saved_label,
        };
        cmd.execute(&mut doc.map);
        doc.history.push_already_applied(Box::new(cmd));
        doc.dirty = true;
        app.validation_dirty = true;
    }
}
