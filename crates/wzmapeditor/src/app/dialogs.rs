//! Application dialog windows: recovery, new map, save-as metadata.
//!
//! First-run setup lives on the launcher card; see `startup::splash_ui`.

use super::{EditorApp, ResizeAnchor};
use crate::config::{DEFAULT_LICENSE, LICENSE_OPTIONS};
use wz_maplib::ResizeReport;
use wz_maplib::constants::{MAP_MAX_WZ_EXPORT, world_coord};

/// Recovery dialog shown on startup when auto-save files from a previous session exist.
pub(super) fn show_recovery_dialog(ctx: &egui::Context, app: &mut EditorApp) {
    egui::Window::new("Recover Auto-Saved Maps")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Auto-saved maps were found from a previous session:");
            ui.add_space(8.0);

            let mut action: Option<(usize, bool)> = None;

            for (i, entry) in app.recovery_entries.iter().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.strong(&entry.map_name);
                        ui.label(format!(
                            "{}x{}, {} players",
                            entry.map_width, entry.map_height, entry.players
                        ));
                        ui.weak(crate::autosave::format_timestamp(entry.timestamp));
                    });
                    if let Some(ref p) = entry.original_path {
                        ui.label(format!("Originally from: {}", p.display()));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Recover").clicked() {
                            action = Some((i, true));
                        }
                        if ui.button("Discard").clicked() {
                            action = Some((i, false));
                        }
                    });
                });
            }

            ui.add_space(8.0);
            if app.recovery_entries.len() > 1 && ui.button("Discard All").clicked() {
                for entry in app.recovery_entries.drain(..) {
                    crate::autosave::discard_entry(&entry);
                }
            }

            if let Some((idx, recover)) = action {
                let entry = app.recovery_entries.remove(idx);
                if recover {
                    match crate::autosave::load_recovery(&entry) {
                        Ok(map) => {
                            app.load_map(map, None, None, None);
                            app.log(format!("Recovered auto-saved map: {}", entry.map_name));
                        }
                        Err(e) => app.log(format!("Recovery failed: {e}")),
                    }
                }
                crate::autosave::discard_entry(&entry);
            }
        });
}

/// New map creation dialog.
pub(super) fn show_new_map_dialog(ctx: &egui::Context, app: &mut EditorApp) {
    let mut open = app.new_map_dialog.open;
    egui::Window::new("New Map")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut app.new_map_dialog.name);
            });

            let sizes: &[u32] = &[32, 64, 128, 192, 250];
            ui.horizontal(|ui| {
                ui.label("Width:");
                egui::ComboBox::from_id_salt("map_width")
                    .selected_text(format!("{}", app.new_map_dialog.width))
                    .show_ui(ui, |ui| {
                        for &s in sizes {
                            ui.selectable_value(&mut app.new_map_dialog.width, s, format!("{s}"));
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Height:");
                egui::ComboBox::from_id_salt("map_height")
                    .selected_text(format!("{}", app.new_map_dialog.height))
                    .show_ui(ui, |ui| {
                        for &s in sizes {
                            ui.selectable_value(&mut app.new_map_dialog.height, s, format!("{s}"));
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Tileset:");
                egui::ComboBox::from_id_salt("map_tileset")
                    .selected_text(app.new_map_dialog.tileset.to_string())
                    .show_ui(ui, |ui| {
                        for ts in crate::config::Tileset::ALL {
                            ui.selectable_value(
                                &mut app.new_map_dialog.tileset,
                                ts,
                                ts.to_string(),
                            );
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Initial Height:");
                ui.add(egui::Slider::new(
                    &mut app.new_map_dialog.initial_height,
                    0..=510,
                ));
            });

            ui.add_space(8.0);
            if ui.button("Create").clicked() {
                let name = app.new_map_dialog.name.clone();
                let w = app.new_map_dialog.width;
                let h = app.new_map_dialog.height;
                let ih = app.new_map_dialog.initial_height;
                let ts = app.new_map_dialog.tileset;
                let mut map = wz_maplib::WzMap::new(&name, w, h);
                for tile in &mut map.map_data.tiles {
                    tile.height = ih;
                }
                map.tileset = ts.as_str().to_string();
                map.terrain_types = Some(wz_maplib::TerrainTypeData {
                    terrain_types: ts.default_terrain_types(),
                });
                app.load_map(map, None, None, None);
                // Signal close - picked up by sync-back below.
                app.new_map_dialog.open = false;
            }
        });
    // Sync close-button state, but don't overwrite an explicit close from Create.
    if app.new_map_dialog.open {
        app.new_map_dialog.open = open;
    }
}

/// Permission-error dialog shown when writing into WZ2100's user-data
/// directory (maps/tests) fails.
pub(super) fn show_permission_error_dialog(ctx: &egui::Context, app: &mut EditorApp) {
    let mut open = app.permission_error_dialog.open;
    let mut dismiss = false;
    egui::Window::new("Cannot Write to Warzone 2100 Directory")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label("Could not write to Warzone 2100's user-data directory:");
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    app.permission_error_dialog
                        .target_path
                        .display()
                        .to_string(),
                )
                .monospace(),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "Reason: {}",
                    app.permission_error_dialog.error_message
                ))
                .weak(),
            );
            ui.add_space(8.0);
            ui.label(
                "Make sure Warzone 2100 has been launched at least once so \
                 its profile directory exists, then check that the folder \
                 above is writable.",
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    dismiss = true;
                }
            });
        });
    if dismiss || !open {
        app.permission_error_dialog.open = false;
    }
}

/// Smallest map dimension allowed in the dialog (matches the WZ2100 engine
/// requirement that both width and height are greater than 1).
const MIN_RESIZE_DIM: u32 = 2;

/// Resize-map dialog. Lets the user grow or shrink the loaded map by picking
/// an anchor on a live preview of the source minimap, and previews how many
/// objects fall outside the new bounds before applying.
pub(super) fn show_resize_map_dialog(ctx: &egui::Context, app: &mut EditorApp) {
    let Some(source_size) = app
        .document
        .as_ref()
        .map(|d| (d.map.map_data.width, d.map.map_data.height))
    else {
        app.resize_map_dialog.open = false;
        return;
    };

    let mut open = app.resize_map_dialog.open;
    let mut apply_clicked = false;
    let mut cancel_clicked = false;

    egui::Window::new("Resize Map")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .default_pos(ctx.content_rect().center() - egui::vec2(180.0, 200.0))
        .fixed_size([360.0, 400.0])
        .show(ctx, |ui| {
            ui.label(format!(
                "Current size: {} x {}",
                source_size.0, source_size.1
            ));
            ui.add_space(4.0);

            let max = MAP_MAX_WZ_EXPORT;
            ui.horizontal(|ui| {
                ui.label("New width:");
                ui.add(
                    egui::DragValue::new(&mut app.resize_map_dialog.new_width)
                        .range(MIN_RESIZE_DIM..=max),
                );
            });
            ui.horizontal(|ui| {
                ui.label("New height:");
                ui.add(
                    egui::DragValue::new(&mut app.resize_map_dialog.new_height)
                        .range(MIN_RESIZE_DIM..=max),
                );
            });
            ui.label(egui::RichText::new(format!("Maximum {max} x {max}.")).weak());

            ui.add_space(8.0);

            // Refresh the cached minimap if the map changed since last view.
            if app.minimap.dirty
                && let Some(ref doc) = app.document
            {
                crate::ui::minimap::regenerate_minimap(
                    ui.ctx(),
                    &mut app.minimap,
                    &doc.map.map_data,
                    doc.map.terrain_types.as_ref(),
                    app.current_tileset,
                );
            }

            let new_dims = (
                app.resize_map_dialog.new_width,
                app.resize_map_dialog.new_height,
            );

            resize_preview_pane(
                ui,
                &mut app.resize_map_dialog.anchor,
                source_size,
                new_dims,
                app.minimap.texture.as_ref(),
            );

            ui.add_space(8.0);

            let new_w = app.resize_map_dialog.new_width;
            let new_h = app.resize_map_dialog.new_height;
            let (ox, oy) = app.resize_map_dialog.effective_offset();
            let valid =
                new_w >= MIN_RESIZE_DIM && new_h >= MIN_RESIZE_DIM && new_w <= max && new_h <= max;
            let unchanged = new_w == source_size.0 && new_h == source_size.1 && (ox, oy) == (0, 0);

            if valid {
                if let Some(report) = app
                    .document
                    .as_ref()
                    .map(|d| d.map.resize_report(new_w, new_h, ox, oy))
                {
                    show_removal_report(ui, &report);
                }
            } else {
                ui.label(
                    egui::RichText::new(format!(
                        "Width and height must be between {MIN_RESIZE_DIM} and {max}."
                    ))
                    .color(egui::Color32::from_rgb(220, 90, 90)),
                );
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let apply_label = if unchanged { "Apply (no-op)" } else { "Apply" };
                if ui
                    .add_enabled(valid, egui::Button::new(apply_label))
                    .clicked()
                {
                    apply_clicked = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel_clicked = true;
                }
            });
        });

    if cancel_clicked {
        app.resize_map_dialog.open = false;
        return;
    }
    if app.resize_map_dialog.open {
        app.resize_map_dialog.open = open;
    }

    if apply_clicked {
        apply_resize(app);
    }
}

const ANCHOR_BY_CELL: [[ResizeAnchor; 3]; 3] = [
    [
        ResizeAnchor::TopLeft,
        ResizeAnchor::TopCenter,
        ResizeAnchor::TopRight,
    ],
    [
        ResizeAnchor::MiddleLeft,
        ResizeAnchor::MiddleCenter,
        ResizeAnchor::MiddleRight,
    ],
    [
        ResizeAnchor::BottomLeft,
        ResizeAnchor::BottomCenter,
        ResizeAnchor::BottomRight,
    ],
];

/// Reverse lookup of the 3x3 cell index from an anchor enum.
fn anchor_cell_index(anchor: ResizeAnchor) -> (usize, usize) {
    for (r, row) in ANCHOR_BY_CELL.iter().enumerate() {
        for (c, a) in row.iter().enumerate() {
            if *a == anchor {
                return (r, c);
            }
        }
    }
    (1, 1)
}

/// Visual preview of the resize: paints the source minimap at the chosen
/// anchor inside the new map's bounds, with red overlays on cropped tiles
/// and green fills where empty tiles will be added. Click any of the nine
/// zones to dock the source at that anchor.
fn resize_preview_pane(
    ui: &mut egui::Ui,
    anchor: &mut ResizeAnchor,
    source_size: (u32, u32),
    new_size: (u32, u32),
    minimap: Option<&egui::TextureHandle>,
) {
    let stage_size = egui::vec2(320.0, 200.0);
    let (rect, response) = ui.allocate_exact_size(stage_size, egui::Sense::click());

    let (sw, sh) = (source_size.0 as i32, source_size.1 as i32);
    let (nw, nh) = (new_size.0 as i32, new_size.1 as i32);
    let (ox, oy) = anchor.offset(source_size, new_size);

    // Union bounding box in source-tile coords. Source occupies [0, sw)x[0, sh);
    // new occupies [ox, ox+nw)x[oy, oy+nh) by the offset convention.
    let umin_x = 0.min(ox) as f32;
    let umin_y = 0.min(oy) as f32;
    let umax_x = sw.max(ox + nw) as f32;
    let umax_y = sh.max(oy + nh) as f32;
    let union_w = (umax_x - umin_x).max(1.0);
    let union_h = (umax_y - umin_y).max(1.0);

    let inner = rect.shrink(8.0);
    let scale = (inner.width() / union_w).min(inner.height() / union_h);
    let stage_origin = inner.center() - egui::vec2(union_w, union_h) * (scale * 0.5);
    let src_origin = stage_origin + egui::vec2(-umin_x, -umin_y) * scale;
    let new_origin = stage_origin + egui::vec2(ox as f32 - umin_x, oy as f32 - umin_y) * scale;
    let src_rect = egui::Rect::from_min_size(src_origin, egui::vec2(sw as f32, sh as f32) * scale);
    let new_rect = egui::Rect::from_min_size(new_origin, egui::vec2(nw as f32, nh as f32) * scale);

    let visuals = ui.visuals().clone();
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );

    // Source surface: paint the live minimap, or a neutral fill if it has
    // not been generated yet (e.g. dialog opened before the first frame).
    if let Some(tex) = minimap {
        painter.image(
            tex.id(),
            src_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        painter.rect_filled(src_rect, 0.0, visuals.widgets.inactive.bg_fill);
    }

    // Added bands: parts of new not covered by source.
    let added = egui::Color32::from_rgba_unmultiplied(80, 180, 110, 110);
    paint_outside(&painter, new_rect, src_rect, added);

    // Cropped bands: parts of source not covered by new (tinted on top of minimap).
    let cropped = egui::Color32::from_rgba_unmultiplied(220, 90, 90, 140);
    paint_outside(&painter, src_rect, new_rect, cropped);

    painter.rect_stroke(
        new_rect,
        0.0,
        egui::Stroke::new(1.5, visuals.widgets.active.fg_stroke.color),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        src_rect,
        0.0,
        egui::Stroke::new(
            1.0,
            visuals.widgets.inactive.fg_stroke.color.gamma_multiply(0.7),
        ),
        egui::StrokeKind::Inside,
    );

    let cell_w = rect.width() / 3.0;
    let cell_h = rect.height() / 3.0;
    let cell_rect = |row: usize, col: usize| {
        egui::Rect::from_min_size(
            egui::pos2(
                rect.left() + col as f32 * cell_w,
                rect.top() + row as f32 * cell_h,
            ),
            egui::vec2(cell_w, cell_h),
        )
    };

    if let Some(pos) = response.hover_pos() {
        let col = ((pos.x - rect.left()) / cell_w).floor().clamp(0.0, 2.0) as usize;
        let row = ((pos.y - rect.top()) / cell_h).floor().clamp(0.0, 2.0) as usize;
        painter.rect_filled(
            cell_rect(row, col),
            0.0,
            visuals.widgets.hovered.bg_fill.gamma_multiply(0.25),
        );
    }

    let (sr, sc) = anchor_cell_index(*anchor);
    painter.rect_stroke(
        cell_rect(sr, sc).shrink(2.0),
        2.0,
        egui::Stroke::new(1.5, visuals.selection.stroke.color),
        egui::StrokeKind::Inside,
    );

    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let col = ((pos.x - rect.left()) / cell_w).floor().clamp(0.0, 2.0) as usize;
        let row = ((pos.y - rect.top()) / cell_h).floor().clamp(0.0, 2.0) as usize;
        *anchor = ANCHOR_BY_CELL[row][col];
    }
}

/// Paint the four bands of `outer` that fall outside `hole`.
/// If they don't overlap at all, the entire `outer` is painted.
fn paint_outside(
    painter: &egui::Painter,
    outer: egui::Rect,
    hole: egui::Rect,
    color: egui::Color32,
) {
    let hole = outer.intersect(hole);
    if hole.width() <= 0.0 || hole.height() <= 0.0 {
        painter.rect_filled(outer, 0.0, color);
        return;
    }
    if hole.top() > outer.top() {
        painter.rect_filled(
            egui::Rect::from_min_max(outer.left_top(), egui::pos2(outer.right(), hole.top())),
            0.0,
            color,
        );
    }
    if hole.bottom() < outer.bottom() {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(outer.left(), hole.bottom()),
                outer.right_bottom(),
            ),
            0.0,
            color,
        );
    }
    if hole.left() > outer.left() {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(outer.left(), hole.top()),
                egui::pos2(hole.left(), hole.bottom()),
            ),
            0.0,
            color,
        );
    }
    if hole.right() < outer.right() {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(hole.right(), hole.top()),
                egui::pos2(outer.right(), hole.bottom()),
            ),
            0.0,
            color,
        );
    }
}

/// Vertical breakdown of objects that will be discarded by the resize.
/// Lists only categories with non-zero counts; collapses to a single
/// muted line when nothing will be removed.
fn show_removal_report(ui: &mut egui::Ui, report: &ResizeReport) {
    let categories: [(usize, &str, &str); 5] = [
        (report.structures_removed, "structure", "structures"),
        (report.droids_removed, "droid", "droids"),
        (report.features_removed, "feature", "features"),
        (report.labels_removed, "label", "labels"),
        (report.gateways_removed, "gateway", "gateways"),
    ];
    let total: usize = categories.iter().map(|(n, _, _)| *n).sum();

    if total == 0 {
        ui.label(
            egui::RichText::new("Nothing will be removed.")
                .color(ui.visuals().widgets.inactive.fg_stroke.color),
        );
        return;
    }

    let warn = egui::Color32::from_rgb(220, 160, 60);
    ui.label(egui::RichText::new("Will remove:").strong().color(warn));
    ui.indent("resize_removal_report", |ui| {
        for (n, single, plural) in categories {
            if n == 0 {
                continue;
            }
            let unit = if n == 1 { single } else { plural };
            ui.label(egui::RichText::new(format!("{n} {unit}")).color(warn));
        }
    });
}

fn apply_resize(app: &mut EditorApp) {
    let new_w = app.resize_map_dialog.new_width;
    let new_h = app.resize_map_dialog.new_height;
    let (ox, oy) = app.resize_map_dialog.effective_offset();

    let report = {
        let Some(doc) = app.document.as_mut() else {
            app.resize_map_dialog.open = false;
            return;
        };
        let (cmd, report) =
            crate::map::resize::ResizeMapCommand::apply(&mut doc.map, new_w, new_h, ox, oy);
        doc.history.push_already_applied(cmd);
        doc.dirty = true;
        report
    };

    mark_post_resize_dirty(app, new_w, new_h);

    app.log(format!(
        "Resized to {new_w}x{new_h} (removed {} structs, {} droids, {} features, {} labels, {} gateways)",
        report.structures_removed,
        report.droids_removed,
        report.features_removed,
        report.labels_removed,
        report.gateways_removed,
    ));
    app.resize_map_dialog.open = false;
}

fn mark_post_resize_dirty(app: &mut EditorApp, new_w: u32, new_h: u32) {
    app.terrain_dirty = true;
    app.terrain_dirty_tiles.clear();
    app.lightmap_dirty = true;
    app.objects_dirty = true;
    app.shadow_dirty = true;
    app.water_dirty = true;
    app.minimap.dirty = true;
    app.heatmap_dirty = true;
    app.selection.clear();
    app.hovered_tile = None;
    app.validation_dirty = true;
    // Recenter so the user is not staring into empty space after a strong shift.
    let cx = world_coord((new_w / 2) as i32) as f32;
    let cy = world_coord((new_h / 2) as i32) as f32;
    app.focus_request = Some((cx, cy));
}

/// Open the Save As metadata dialog with fields prefilled from the loaded
/// map. When the Settings "Default author" differs from the map's existing
/// author, the existing author is pre-shifted into the additional-authors
/// list so the user appears as the primary author.
pub(crate) fn open_save_as_metadata_dialog(app: &mut EditorApp) {
    let Some(ref doc) = app.document else {
        return;
    };
    let map = &doc.map;
    let settings_default = app.config.default_author_name.clone().unwrap_or_default();
    let mut additional = map.additional_authors.clone();

    let author = match (map.author.as_deref(), settings_default.is_empty()) {
        (Some(loaded), false) if loaded != settings_default => {
            if !additional.iter().any(|a| a == loaded) {
                additional.insert(0, loaded.to_string());
            }
            settings_default
        }
        (Some(loaded), _) => loaded.to_string(),
        (None, false) => settings_default,
        (None, true) => String::new(),
    };

    let license = map
        .license
        .clone()
        .unwrap_or_else(|| DEFAULT_LICENSE.to_string());
    app.save_as_metadata_dialog = super::SaveAsMetadataDialog {
        open: true,
        author,
        additional_authors: additional.join(", "),
        license,
        original_author: map.author.clone(),
    };
}

pub(super) fn show_save_as_metadata_dialog(ctx: &egui::Context, app: &mut EditorApp) {
    let mut open = app.save_as_metadata_dialog.open;
    let mut save_clicked = false;

    egui::Window::new("Save As")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Author:");
                ui.add(
                    egui::TextEdit::singleline(&mut app.save_as_metadata_dialog.author)
                        .hint_text("Your name")
                        .desired_width(260.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Additional authors:");
                ui.add(
                    egui::TextEdit::singleline(&mut app.save_as_metadata_dialog.additional_authors)
                        .hint_text("comma-separated")
                        .desired_width(260.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label("License:");
                let current = app.save_as_metadata_dialog.license.clone();
                let preserved_unknown = !LICENSE_OPTIONS.iter().any(|s| *s == current);
                egui::ComboBox::from_id_salt("save_as_license_combo")
                    .selected_text(if current.is_empty() {
                        DEFAULT_LICENSE.to_string()
                    } else {
                        current.clone()
                    })
                    .show_ui(ui, |ui| {
                        if preserved_unknown && !current.is_empty() {
                            ui.selectable_value(
                                &mut app.save_as_metadata_dialog.license,
                                current,
                                "Preserved (unknown SPDX)",
                            );
                        }
                        for opt in LICENSE_OPTIONS {
                            ui.selectable_value(
                                &mut app.save_as_metadata_dialog.license,
                                (*opt).to_string(),
                                *opt,
                            );
                        }
                    });
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save\u{2026}").clicked() {
                    save_clicked = true;
                }
                if ui.button("Cancel").clicked() {
                    app.save_as_metadata_dialog.open = false;
                }
            });
        });

    if app.save_as_metadata_dialog.open {
        app.save_as_metadata_dialog.open = open;
    }

    if save_clicked {
        commit_save_as_metadata(app);
    }
}

fn commit_save_as_metadata(app: &mut EditorApp) {
    let dialog = std::mem::take(&mut app.save_as_metadata_dialog);
    if app.document.is_none() {
        return;
    }

    let filename = app.suggested_wz_filename();
    let Some(path) = rfd::FileDialog::new()
        .set_title("Save As .wz Archive")
        .set_file_name(filename)
        .add_filter("WZ Map", &["wz"])
        .save_file()
    else {
        return;
    };

    let trimmed_author = dialog.author.trim().to_string();
    let mut additional: Vec<String> = dialog
        .additional_authors
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if let Some(prev) = dialog.original_author {
        let prev_trim = prev.trim();
        if !prev_trim.is_empty()
            && prev_trim != trimmed_author
            && !additional.iter().any(|a| a == prev_trim)
        {
            additional.insert(0, prev_trim.to_string());
        }
    }

    let doc = app.document.as_mut().expect("checked above");
    doc.map.author = if trimmed_author.is_empty() {
        None
    } else {
        Some(trimmed_author)
    };
    doc.map.additional_authors = additional;
    doc.map.license = if dialog.license.trim().is_empty() {
        None
    } else {
        Some(dialog.license.trim().to_string())
    };

    app.save_to_wz(&path);
}
