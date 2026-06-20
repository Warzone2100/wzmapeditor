//! Asset browser panel with grid/list views and 3D model thumbnails.
//!
//! Thumbnail rendering and caching lives in [`crate::thumbnails`]. This
//! module owns only the panel UI that consumes the cache.

use eframe::egui_wgpu;
use egui::{Ui, Vec2};

use crate::thumbnails::ThumbnailCache;
use crate::tools::{AssetCategory, ToolId, ToolState};
use crate::viewport::model_loader::ModelLoader;

struct AssetEntry {
    id: String,
    label: String,
    imd_name: Option<String>,
    /// Which tab the entry belongs to; selects its thumbnail kind. Needed
    /// because a search mixes entries from every category in one list.
    kind: AssetCategory,
    /// Drives the divider between skirmish-spawnable and campaign-only
    /// blocks in the Droids and Structures tabs.
    is_campaign: bool,
}

/// Pseudo-stats and upgrade modules that aren't placeable structures.
/// Modules attach to factory / power / research hosts via the host's
/// `modules` field. `A0ADemolishStructure` is the demolition-cursor stat,
/// rejected by WZ2100's own build path (`wzapi.cpp:380`).
fn is_hidden_structure(s: &wz_stats::structures::StructureStats) -> bool {
    matches!(
        s.structure_type.as_deref(),
        Some("DEMOLISH" | "FACTORY MODULE" | "POWER MODULE" | "RESEARCH MODULE")
    )
}

pub fn show_asset_browser_inner(
    ui: &mut Ui,
    stats: Option<&wz_stats::StatsDatabase>,
    tool_state: &mut ToolState,
    thumbnails: &mut ThumbnailCache,
    model_loader: &mut Option<ModelLoader>,
    render_state: Option<&egui_wgpu::RenderState>,
    custom_templates: Option<&crate::designer::CustomTemplateStore>,
) {
    let Some(stats) = stats else {
        ui.label("No stats loaded. Set the WZ2100 install directory in Settings \u{2192} Game.");
        return;
    };

    thumbnails.reset_frame_budget();

    let has_mp_overlay = stats.has_mp_overlay();
    let show_campaign = tool_state.asset_show_campaign_only;
    let droid_visible = if has_mp_overlay && !show_campaign {
        stats
            .templates
            .keys()
            .filter(|k| stats.mp_template_ids.contains(k.as_str()))
            .count()
    } else {
        stats.templates.len()
    };
    let structure_visible = if has_mp_overlay && !show_campaign {
        stats
            .structures
            .iter()
            .filter(|(k, s)| !is_hidden_structure(s) && stats.mp_structure_ids.contains(k.as_str()))
            .count()
    } else {
        stats
            .structures
            .iter()
            .filter(|(_, s)| !is_hidden_structure(s))
            .count()
    };

    // `horizontal_wrapped` lets the toolbar break onto a second line in a
    // narrow pane instead of clipping the size slider off the right edge.
    ui.horizontal_wrapped(|ui| {
        let search_resp = ui.add(
            egui::TextEdit::singleline(&mut tool_state.asset_search)
                .hint_text("Search")
                .desired_width(90.0)
                // Reserve room on the right so typed text never slides under
                // the clear glyph.
                .margin(egui::Margin {
                    left: 4,
                    right: 18,
                    top: 2,
                    bottom: 2,
                }),
        );
        // A search spans every category, so it overrides the category
        // selection: remember the picked category on the first keystroke and
        // restore it when the box is emptied again.
        if search_resp.changed() {
            if tool_state.asset_search.trim().is_empty() {
                if let Some(prev) = tool_state.asset_search_prev_category.take() {
                    tool_state.asset_category = prev;
                }
            } else if tool_state.asset_search_prev_category.is_none() {
                tool_state.asset_search_prev_category = Some(tool_state.asset_category);
            }
        }
        // Clear (✕) glyph painted inside the right edge of the field; clicking
        // it empties the search and restores the category the search overrode.
        if !tool_state.asset_search.is_empty() {
            let center = egui::pos2(search_resp.rect.right() - 9.0, search_resp.rect.center().y);
            let hit = egui::Rect::from_center_size(center, egui::Vec2::splat(16.0));
            let clear = ui.interact(
                hit,
                ui.id().with("asset_search_clear"),
                egui::Sense::click(),
            );
            let color = if clear.hovered() {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                "\u{2715}",
                egui::FontId::proportional(12.0),
                color,
            );
            if clear.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if clear.clicked() {
                tool_state.asset_search.clear();
                if let Some(prev) = tool_state.asset_search_prev_category.take() {
                    tool_state.asset_category = prev;
                }
            }
        }
        crate::ui::property_panel::player_widget(
            ui,
            &mut tool_state.placement_player,
            "asset_placement_player",
        );

        ui.separator();

        let searching = !tool_state.asset_search.trim().is_empty();
        let category_tabs = [
            (
                AssetCategory::Structures,
                format!("Structures ({structure_visible})"),
            ),
            (
                AssetCategory::Features,
                format!("Features ({})", stats.features.len()),
            ),
            (AssetCategory::Droids, format!("Droids ({droid_visible})")),
        ];
        for (cat, label) in category_tabs {
            // No tab is highlighted while a search is active.
            let selected = !searching && tool_state.asset_category == cat;
            if ui.selectable_label(selected, label).clicked() {
                tool_state.asset_category = cat;
                tool_state.asset_search.clear();
                tool_state.asset_search_prev_category = None;
            }
        }

        if has_mp_overlay {
            ui.add(egui::Checkbox::new(
                &mut tool_state.asset_show_campaign_only,
                "Campaign",
            ))
            .on_hover_text(
                "Include campaign-only droid templates and structures \
                 (CO-*, NP-*, NX-*, GuardTowerN, etc). These don't spawn \
                 in skirmish or multiplayer maps.",
            );
        }

        ui.separator();

        // Show the *other* mode's label so a click switches into it.
        let next_view_label = if tool_state.asset_grid_view {
            "List"
        } else {
            "Grid"
        };
        if ui.button(next_view_label).clicked() {
            tool_state.asset_grid_view = !tool_state.asset_grid_view;
        }

        if tool_state.asset_grid_view {
            ui.add(
                egui::Slider::new(&mut tool_state.asset_thumb_size, 32.0..=128.0).show_value(false),
            );
        }
    });

    ui.separator();

    let search = tool_state.asset_search.trim().to_lowercase();
    let searching = !search.is_empty();
    let filter_to_mp = has_mp_overlay && !show_campaign;
    // A search pulls matches from every category; otherwise only the
    // selected tab's entries are shown.
    let active_categories: &[AssetCategory] = if searching {
        &[
            AssetCategory::Structures,
            AssetCategory::Features,
            AssetCategory::Droids,
        ]
    } else {
        std::slice::from_ref(&tool_state.asset_category)
    };
    let mut entries: Vec<AssetEntry> = active_categories
        .iter()
        .flat_map(|&cat| {
            build_category_entries(
                cat,
                stats,
                &search,
                has_mp_overlay,
                filter_to_mp,
                custom_templates,
            )
        })
        .collect();
    // Campaign-only entries fall to the bottom, divided from the
    // skirmish-usable block by the section header rendered in the grid/list.
    entries.sort_by(|a, b| a.is_campaign.cmp(&b.is_campaign).then(a.id.cmp(&b.id)));

    let selected = tool_state
        .object_place()
        .and_then(|t| t.placement_object.clone());
    let mut clicked_id: Option<String> = None;
    let mut hovered_entry: Option<(String, String)> = None;

    if tool_state.asset_grid_view {
        show_grid_view(
            ui,
            &entries,
            selected.as_deref(),
            tool_state.asset_thumb_size,
            thumbnails,
            model_loader,
            &mut clicked_id,
            &mut hovered_entry,
            stats,
            render_state,
        );
    } else {
        show_list_view(
            ui,
            &entries,
            selected.as_deref(),
            thumbnails,
            model_loader,
            &mut clicked_id,
            stats,
            render_state,
        );
    }

    // Only request another paint while the window has focus. Regaining
    // focus re-runs update() and resumes the loop.
    if thumbnails.has_pending() && ui.ctx().input(|i| i.focused) {
        ui.ctx().request_repaint();
    }

    if let Some(id) = clicked_id {
        if let Some(place) = tool_state.object_place_mut() {
            place.placement_object = Some(id);
        }
        tool_state.active_tool = ToolId::ObjectPlace;
    }
}

/// Render the thumbnail grid view, splitting into mp and campaign sections
/// with a divider between them when both are present.
/// Build the asset entries for a single category, applying the search and
/// multiplayer/campaign filters. Each entry is tagged with its `kind` so a
/// cross-category search can render the right thumbnail per item.
fn build_category_entries(
    category: AssetCategory,
    stats: &wz_stats::StatsDatabase,
    search: &str,
    has_mp_overlay: bool,
    filter_to_mp: bool,
    custom_templates: Option<&crate::designer::CustomTemplateStore>,
) -> Vec<AssetEntry> {
    match category {
        AssetCategory::Structures => stats
            .structures
            .iter()
            .filter(|(_, s)| !is_hidden_structure(s))
            .filter(|(key, _)| !filter_to_mp || stats.mp_structure_ids.contains(key.as_str()))
            .filter(|(_, s)| search.is_empty() || s.id.to_lowercase().contains(search))
            .map(|(key, s)| {
                let is_campaign = has_mp_overlay && !stats.mp_structure_ids.contains(key.as_str());
                AssetEntry {
                    id: key.clone(),
                    label: s
                        .structure_type
                        .as_ref()
                        .map_or_else(|| key.clone(), |t| format!("{key} ({t})")),
                    imd_name: s.pie_model().map(ToString::to_string),
                    kind: AssetCategory::Structures,
                    is_campaign,
                }
            })
            .collect(),
        AssetCategory::Features => stats
            .features
            .iter()
            .filter(|(_, f)| search.is_empty() || f.id.to_lowercase().contains(search))
            .map(|(key, f)| AssetEntry {
                id: key.clone(),
                label: f
                    .feature_type
                    .as_ref()
                    .map_or_else(|| key.clone(), |t| format!("{key} ({t})")),
                imd_name: f.pie_model().map(ToString::to_string),
                kind: AssetCategory::Features,
                is_campaign: false,
            })
            .collect(),
        AssetCategory::Droids => stats
            .templates
            .iter()
            .filter(|(key, _)| {
                // Custom templates always show. Built-in templates only show
                // mp-allowed entries unless the user opts into campaign-only.
                let is_custom = custom_templates.is_some_and(|s| s.owns(key));
                is_custom || !filter_to_mp || stats.mp_template_ids.contains(key.as_str())
            })
            .filter(|(_, t)| search.is_empty() || t.id.to_lowercase().contains(search))
            .map(|(key, t)| {
                let is_custom = custom_templates.is_some_and(|s| s.owns(key));
                let badge = if is_custom { "\u{2605} " } else { "" };
                let is_campaign =
                    has_mp_overlay && !is_custom && !stats.mp_template_ids.contains(key.as_str());
                AssetEntry {
                    id: key.clone(),
                    label: t.droid_type.as_ref().map_or_else(
                        || format!("{badge}{key}"),
                        |dt| format!("{badge}{key} ({dt})"),
                    ),
                    imd_name: stats
                        .bodies
                        .get(&t.body)
                        .and_then(|b| b.pie_model())
                        .map(ToString::to_string),
                    kind: AssetCategory::Droids,
                    is_campaign,
                }
            })
            .collect(),
    }
}

fn show_grid_view(
    ui: &mut Ui,
    entries: &[AssetEntry],
    selected: Option<&str>,
    thumb_size: f32,
    thumbnails: &mut ThumbnailCache,
    model_loader: &mut Option<ModelLoader>,
    clicked_id: &mut Option<String>,
    hovered_entry: &mut Option<(String, String)>,
    stats: &wz_stats::StatsDatabase,
    render_state: Option<&egui_wgpu::RenderState>,
) {
    // Cards are at most `thumb_size` wide with 4px grid spacing. When the
    // pane is narrower than one thumbnail, shrink the column to fit so the
    // inner ScrollArea's vertical scrollbar stays on-screen.
    let spacing = 4.0_f32;
    let avail = ui.available_width();
    let card_width = thumb_size.min((avail - spacing).max(48.0));
    let cols = ((avail / (card_width + spacing)).floor() as usize).max(1);
    let split = entries.partition_point(|e| !e.is_campaign);
    let mp = &entries[..split];
    let campaign = &entries[split..];

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .show(ui, |ui| {
            render_grid_section(
                ui,
                "asset_grid_mp",
                mp,
                cols,
                card_width,
                selected,
                thumbnails,
                model_loader,
                clicked_id,
                hovered_entry,
                stats,
                render_state,
            );

            if !campaign.is_empty() {
                if !mp.is_empty() {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Campaign-only")
                            .small()
                            .color(egui::Color32::from_gray(170)),
                    );
                    ui.separator();
                }
                render_grid_section(
                    ui,
                    "asset_grid_campaign",
                    campaign,
                    cols,
                    card_width,
                    selected,
                    thumbnails,
                    model_loader,
                    clicked_id,
                    hovered_entry,
                    stats,
                    render_state,
                );
            }
        });
}

#[expect(
    clippy::too_many_arguments,
    reason = "egui rendering helper threading egui state, layout config, and async handles"
)]
fn render_grid_section(
    ui: &mut Ui,
    grid_id: &str,
    entries: &[AssetEntry],
    cols: usize,
    thumb_size: f32,
    selected: Option<&str>,
    thumbnails: &mut ThumbnailCache,
    model_loader: &mut Option<ModelLoader>,
    clicked_id: &mut Option<String>,
    hovered_entry: &mut Option<(String, String)>,
    stats: &wz_stats::StatsDatabase,
    render_state: Option<&egui_wgpu::RenderState>,
) {
    if entries.is_empty() {
        return;
    }
    egui::Grid::new(grid_id)
        .num_columns(cols)
        .min_col_width(thumb_size)
        .max_col_width(thumb_size)
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for chunk in entries.chunks(cols) {
                for entry in chunk {
                    let is_selected = selected == Some(entry.id.as_str());
                    let (click, hover) = show_grid_card(
                        ui,
                        entry,
                        is_selected,
                        thumb_size,
                        thumbnails,
                        model_loader,
                        stats,
                        render_state,
                    );
                    if click {
                        *clicked_id = Some(entry.id.clone());
                    }
                    if hover && let Some(ref imd) = entry.imd_name {
                        *hovered_entry = Some((entry.id.clone(), imd.clone()));
                    }
                }
                ui.end_row();
            }
        });
}

fn show_grid_card(
    ui: &mut Ui,
    entry: &AssetEntry,
    selected: bool,
    thumb_size: f32,
    thumbnails: &mut ThumbnailCache,
    model_loader: &mut Option<ModelLoader>,
    stats: &wz_stats::StatsDatabase,
    render_state: Option<&egui_wgpu::RenderState>,
) -> (bool, bool) {
    let mut clicked = false;
    let mut hovered = false;

    let resp = ui.vertical(|ui| {
        ui.set_width(thumb_size);
        let thumb_vec = Vec2::splat(thumb_size);

        let tex_id = match entry.kind {
            AssetCategory::Droids => thumbnails
                .request_droid_thumbnail(ui.ctx(), &entry.id, stats, model_loader, render_state)
                .map(egui::TextureHandle::id),
            AssetCategory::Structures => thumbnails
                .request_structure_thumbnail(ui.ctx(), &entry.id, stats, model_loader, render_state)
                .map(egui::TextureHandle::id),
            AssetCategory::Features => entry
                .imd_name
                .as_deref()
                .and_then(|imd| {
                    thumbnails.request_model_thumbnail(ui.ctx(), imd, model_loader, render_state)
                })
                .map(egui::TextureHandle::id),
        };

        if let Some(tex_id) = tex_id {
            let r = ui.add(
                egui::Image::new((tex_id, thumb_vec))
                    .corner_radius(3)
                    .sense(egui::Sense::click()),
            );
            if r.clicked() {
                clicked = true;
            }
            if r.hovered() {
                hovered = true;
            }
            if selected {
                ui.painter().rect_stroke(
                    r.rect,
                    3.0,
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 180, 255)),
                    egui::StrokeKind::Inside,
                );
            }
        } else {
            let (rect, r) = ui.allocate_exact_size(thumb_vec, egui::Sense::click());
            ui.painter()
                .rect_filled(rect, 3.0, egui::Color32::from_gray(50));
            if r.clicked() {
                clicked = true;
            }
        }

        let max_chars = (thumb_size / 6.0) as usize;
        let display = if entry.id.len() > max_chars && max_chars > 3 {
            format!("{}...", &entry.id[..max_chars - 3])
        } else {
            entry.id.clone()
        };
        let label_r = ui.add(
            egui::Label::new(egui::RichText::new(&display).small())
                .truncate()
                .sense(egui::Sense::click()),
        );
        if label_r.clicked() {
            clicked = true;
        }
        label_r.on_hover_text(&entry.label);
    });

    if resp.response.clicked() {
        clicked = true;
    }

    (clicked, hovered)
}

/// Render the multi-column list view, splitting into mp and campaign sections
/// with a divider between them when both are present.
fn show_list_view(
    ui: &mut Ui,
    entries: &[AssetEntry],
    selected: Option<&str>,
    thumbnails: &mut ThumbnailCache,
    model_loader: &mut Option<ModelLoader>,
    clicked_id: &mut Option<String>,
    stats: &wz_stats::StatsDatabase,
    render_state: Option<&egui_wgpu::RenderState>,
) {
    // Shrink list-row columns to fit a narrow pane so the inner
    // ScrollArea's vertical scrollbar stays on-screen.
    let preferred = 280.0_f32;
    let avail = ui.available_width();
    let col_width = preferred.min(avail.max(120.0));
    let cols = ((avail / col_width).floor() as usize).max(1);
    let split = entries.partition_point(|e| !e.is_campaign);
    let mp = &entries[..split];
    let campaign = &entries[split..];

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .show(ui, |ui| {
            render_list_section(
                ui,
                "asset_list_grid_mp",
                mp,
                cols,
                col_width,
                selected,
                thumbnails,
                model_loader,
                clicked_id,
                stats,
                render_state,
            );
            if !campaign.is_empty() {
                if !mp.is_empty() {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Campaign-only")
                            .small()
                            .color(egui::Color32::from_gray(170)),
                    );
                    ui.separator();
                }
                render_list_section(
                    ui,
                    "asset_list_grid_campaign",
                    campaign,
                    cols,
                    col_width,
                    selected,
                    thumbnails,
                    model_loader,
                    clicked_id,
                    stats,
                    render_state,
                );
            }
        });
}

#[expect(
    clippy::too_many_arguments,
    reason = "egui rendering helper threading egui state, layout config, and async handles"
)]
fn render_list_section(
    ui: &mut Ui,
    grid_id: &str,
    entries: &[AssetEntry],
    cols: usize,
    col_width: f32,
    selected: Option<&str>,
    thumbnails: &mut ThumbnailCache,
    model_loader: &mut Option<ModelLoader>,
    clicked_id: &mut Option<String>,
    stats: &wz_stats::StatsDatabase,
    render_state: Option<&egui_wgpu::RenderState>,
) {
    if entries.is_empty() {
        return;
    }
    let row_height = 26.0;
    egui::Grid::new(grid_id)
        .num_columns(cols)
        .spacing([12.0, 2.0])
        .min_col_width(col_width)
        .max_col_width(col_width)
        .show(ui, |ui| {
            for chunk in entries.chunks(cols) {
                for entry in chunk {
                    let is_selected = selected == Some(entry.id.as_str());

                    let r = ui.horizontal(|ui| {
                        ui.set_height(row_height);

                        let small_thumb = 20.0;
                        let tex = match entry.kind {
                            AssetCategory::Droids => thumbnails.request_droid_thumbnail(
                                ui.ctx(),
                                &entry.id,
                                stats,
                                model_loader,
                                render_state,
                            ),
                            AssetCategory::Structures => thumbnails.request_structure_thumbnail(
                                ui.ctx(),
                                &entry.id,
                                stats,
                                model_loader,
                                render_state,
                            ),
                            AssetCategory::Features => entry.imd_name.as_deref().and_then(|imd| {
                                thumbnails.request_model_thumbnail(
                                    ui.ctx(),
                                    imd,
                                    model_loader,
                                    render_state,
                                )
                            }),
                        };

                        if let Some(tex) = tex {
                            let tex_id = tex.id();
                            ui.add(
                                egui::Image::new((tex_id, Vec2::splat(small_thumb)))
                                    .corner_radius(2),
                            );
                        } else {
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::splat(small_thumb),
                                egui::Sense::hover(),
                            );
                            ui.painter()
                                .rect_filled(rect, 2.0, egui::Color32::from_gray(50));
                        }

                        ui.selectable_label(is_selected, &entry.label)
                    });

                    if r.inner.clicked() {
                        *clicked_id = Some(entry.id.clone());
                    }
                }
                ui.end_row();
            }
        });
}
