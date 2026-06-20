//! Hierarchy panel showing all map objects in a tree view.

use egui::Ui;

use crate::app::{EditorApp, SelectedObject};

pub fn show_hierarchy(ui: &mut Ui, app: &mut EditorApp) {
    let Some(doc) = app.document.as_ref() else {
        ui.label("No map loaded.");
        return;
    };

    let map = &doc.map;
    let selection = &app.selection;
    let mut new_selection: Option<SelectedObject> = None;
    let mut focus_pos: Option<(f32, f32)> = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .show(ui, |ui| {
            // Pin id_salt to the section name, not the count, so adding
            // or removing an object doesn't cascade new auto-ids onto every
            // child label and trip egui "rect changed" warnings.
            let header = format!("Structures ({})", map.structures.len());
            egui::CollapsingHeader::new(header)
                .id_salt("hierarchy_structures")
                .default_open(true)
                .show(ui, |ui| {
                    for (i, obj) in map.structures.iter().enumerate() {
                        let is_selected = selection.contains(&SelectedObject::Structure(i));
                        let label = format!(
                            "{}: {} ({}, {})",
                            i, obj.name, obj.position.x, obj.position.y
                        );
                        let resp = ui.selectable_label(is_selected, label);
                        if resp.clicked() {
                            new_selection = Some(SelectedObject::Structure(i));
                        }
                        if resp.double_clicked() {
                            focus_pos = Some((obj.position.x as f32, obj.position.y as f32));
                        }
                    }
                });

            let header = format!("Droids ({})", map.droids.len());
            egui::CollapsingHeader::new(header)
                .id_salt("hierarchy_droids")
                .default_open(true)
                .show(ui, |ui| {
                    for (i, obj) in map.droids.iter().enumerate() {
                        let is_selected = selection.contains(&SelectedObject::Droid(i));
                        let label = format!(
                            "{}: {} ({}, {})",
                            i, obj.name, obj.position.x, obj.position.y
                        );
                        let resp = ui.selectable_label(is_selected, label);
                        if resp.clicked() {
                            new_selection = Some(SelectedObject::Droid(i));
                        }
                        if resp.double_clicked() {
                            focus_pos = Some((obj.position.x as f32, obj.position.y as f32));
                        }
                    }
                });

            let header = format!("Features ({})", map.features.len());
            egui::CollapsingHeader::new(header)
                .id_salt("hierarchy_features")
                .default_open(true)
                .show(ui, |ui| {
                    for (i, obj) in map.features.iter().enumerate() {
                        let is_selected = selection.contains(&SelectedObject::Feature(i));
                        let label = format!(
                            "{}: {} ({}, {})",
                            i, obj.name, obj.position.x, obj.position.y
                        );
                        let resp = ui.selectable_label(is_selected, label);
                        if resp.clicked() {
                            new_selection = Some(SelectedObject::Feature(i));
                        }
                        if resp.double_clicked() {
                            focus_pos = Some((obj.position.x as f32, obj.position.y as f32));
                        }
                    }
                });

            let header = format!("Labels ({})", map.labels.len());
            egui::CollapsingHeader::new(header)
                .id_salt("hierarchy_labels")
                .default_open(true)
                .show(ui, |ui| {
                    for (i, (_key, label)) in map.labels.iter().enumerate() {
                        let is_selected = selection.contains(&SelectedObject::Label(i));
                        let center = label.center();
                        let text = format!("{}: {} ({}, {})", i, label.label(), center.x, center.y);
                        let resp = ui.selectable_label(is_selected, text);
                        if resp.clicked() {
                            new_selection = Some(SelectedObject::Label(i));
                        }
                        if resp.double_clicked() {
                            focus_pos = Some((center.x as f32, center.y as f32));
                        }
                    }
                });

            let gateways = &map.map_data.gateways;
            let header = format!("Gateways ({})", gateways.len());
            egui::CollapsingHeader::new(header)
                .id_salt("hierarchy_gateways")
                .default_open(false)
                .show(ui, |ui| {
                    for (i, gw) in gateways.iter().enumerate() {
                        let label = format!("{}: ({},{}) - ({},{})", i, gw.x1, gw.y1, gw.x2, gw.y2);
                        let resp = ui.selectable_label(false, label);
                        if resp.double_clicked() {
                            // Gateway coords are tile-space; convert to world units.
                            let tile_size = 128.0_f32;
                            let cx = (f32::from(gw.x1) + f32::from(gw.x2)) * 0.5 * tile_size;
                            let cz = (f32::from(gw.y1) + f32::from(gw.y2)) * 0.5 * tile_size;
                            focus_pos = Some((cx, cz));
                        }
                    }
                });
        });

    if let Some(sel) = new_selection {
        app.selection.set_single(sel);
        app.objects_dirty = true;
    }

    if let Some(pos) = focus_pos {
        app.focus_request = Some(pos);
    }
}
