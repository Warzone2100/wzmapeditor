//! Bottom panel tileset texture browser.

use std::path::Path;

use egui::{ColorImage, TextureHandle, TextureOptions, Ui};

struct TileEntry {
    handle: TextureHandle,
    /// Tile index (e.g. 0 for tile-00.png).
    index: u16,
}

pub struct TilesetData {
    tiles: Vec<TileEntry>,
}

impl std::fmt::Debug for TilesetData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TilesetData")
            .field("tile_count", &self.tiles.len())
            .finish_non_exhaustive()
    }
}

impl TilesetData {
    /// Load `tile-XX.png` files (Arizona has 0..77) from a directory and
    /// upload them as egui textures.
    pub fn load(
        ctx: &egui::Context,
        assets: &dyn crate::assets::AssetSource,
        dir_rel: &Path,
    ) -> Option<Self> {
        let mut tiles = Vec::new();

        for i in 0u16..256 {
            let filename = format!("tile-{i:02}.png");
            let Some(bytes) = assets.bytes(&dir_rel.join(&filename)) else {
                continue;
            };

            match decode_tile_image(&bytes) {
                Ok(color_image) => {
                    let handle =
                        ctx.load_texture(format!("tile_{i}"), color_image, TextureOptions::LINEAR);
                    tiles.push(TileEntry { handle, index: i });
                }
                Err(e) => {
                    log::warn!("Failed to load tile {filename}: {e}");
                }
            }
        }

        if tiles.is_empty() {
            log::warn!("No tile images found in {}", dir_rel.display());
            return None;
        }

        log::info!(
            "Loaded {} tileset tiles from {}",
            tiles.len(),
            dir_rel.display()
        );

        Some(TilesetData { tiles })
    }

    /// Build tileset data from images decoded off-thread; the GPU upload
    /// happens on the main thread.
    pub fn from_preloaded(ctx: &egui::Context, images: Vec<(u16, ColorImage)>) -> Option<Self> {
        if images.is_empty() {
            return None;
        }
        let tiles = images
            .into_iter()
            .map(|(index, color_image)| {
                let handle =
                    ctx.load_texture(format!("tile_{index}"), color_image, TextureOptions::LINEAR);
                TileEntry { handle, index }
            })
            .collect();
        Some(TilesetData { tiles })
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn tile_handle(&self, index: u16) -> Option<&TextureHandle> {
        self.tiles
            .iter()
            .find(|e| e.index == index)
            .map(|e| &e.handle)
    }

    pub fn iter_tiles(&self) -> impl Iterator<Item = (u16, &TextureHandle)> {
        self.tiles.iter().map(|e| (e.index, &e.handle))
    }
}

fn decode_tile_image(bytes: &[u8]) -> Result<ColorImage, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("image decode failed: {e}"))?;
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let pixels = rgba.into_raw();
    Ok(ColorImage::from_rgba_unmultiplied(size, &pixels))
}

/// Load all `tile-NN.png` files from a tileset directory. Safe to call off-thread.
pub(crate) fn load_tile_images_from_dir(
    assets: &dyn crate::assets::AssetSource,
    dir_rel: &Path,
) -> Vec<(u16, ColorImage)> {
    let mut tile_images = Vec::new();
    for i in 0u16..256 {
        let filename = format!("tile-{i:02}.png");
        let Some(bytes) = assets.bytes(&dir_rel.join(&filename)) else {
            continue;
        };
        match decode_tile_image(&bytes) {
            Ok(color_image) => {
                tile_images.push((i, color_image));
            }
            Err(e) => {
                log::warn!("Failed to load tile tile-{i:02}.png: {e}");
            }
        }
    }
    tile_images
}

/// Actions the ground type browser may request from the caller.
pub enum GroundTypeAction {
    None,
    /// Save current tile pools as custom groups to config.
    Save,
    /// Add a new empty group.
    NewGroup,
    /// Delete the currently selected group.
    DeleteGroup,
}

/// Show the tileset browser. In Single mode a click selects one tile; in
/// group mode a click toggles tile membership in the selected pool.
pub fn show_tileset_browser(
    ui: &mut Ui,
    tileset: Option<&TilesetData>,
    terrain_types: Option<&[wz_maplib::TerrainType]>,
    tool_state: &mut crate::tools::ToolState,
) -> GroundTypeAction {
    let Some(ts) = tileset else {
        ui.vertical_centered(|ui| {
            ui.label("No tileset loaded.");
            ui.label("Set the install directory in Settings \u{2192} Game to load tile textures.");
        });
        return GroundTypeAction::None;
    };

    let mut action = GroundTypeAction::None;
    let mut needs_save = false;
    let single_mode = !tool_state.ground_type_mode;
    let selected_ground = tool_state
        .ground_type_brush()
        .map_or(0_u8, |b| b.selected_ground_type);

    let top_id = ui.id().with("tileset_top_panel");
    egui::Panel::top(top_id)
        .resizable(false)
        .show_inside(ui, |ui| {
            ui.add_space(2.0);

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);

                let single_btn = egui::Button::new(egui::RichText::new("Single").small().color(
                    if single_mode {
                        egui::Color32::WHITE
                    } else {
                        ui.visuals().text_color()
                    },
                ))
                .fill(if single_mode {
                    ui.visuals().selection.bg_fill
                } else {
                    egui::Color32::TRANSPARENT
                });

                if ui
                    .add(single_btn)
                    .on_hover_text("Paint with a single tile")
                    .clicked()
                {
                    tool_state.ground_type_mode = false;
                    tool_state.active_tool = crate::tools::ToolId::TexturePaint;
                }

                ui.separator();

                let mut clicked_group: Option<u8> = None;
                for (idx, pool) in tool_state.tile_pools.iter().enumerate() {
                    let is_selected =
                        tool_state.ground_type_mode && selected_ground == pool.ground_type_index;
                    let [r, g, b] = pool.color;
                    let color = egui::Color32::from_rgb(r, g, b);
                    let tile_count: usize = pool.buckets.iter().map(|b| b.tile_ids.len()).sum();

                    let label = format!("{} ({tile_count})", pool.name);
                    let btn = egui::Button::new(egui::RichText::new(&label).small().color(
                        if is_selected {
                            egui::Color32::WHITE
                        } else {
                            ui.visuals().text_color()
                        },
                    ))
                    .fill(if is_selected {
                        color
                    } else {
                        egui::Color32::TRANSPARENT
                    })
                    .stroke(egui::Stroke::new(1.0_f32, color));

                    if ui.add(btn).on_hover_text(&pool.name).clicked() {
                        clicked_group = Some(idx as u8);
                    }
                }
                if let Some(idx) = clicked_group {
                    tool_state.ground_type_mode = true;
                    if let Some(brush) = tool_state.ground_type_brush_mut() {
                        brush.selected_ground_type = idx;
                    }
                    tool_state.active_tool = crate::tools::ToolId::GroundTypePaint;
                }
            });

            if tool_state.ground_type_mode {
                ui.horizontal(|ui| {
                    let text_edit = egui::TextEdit::singleline(&mut tool_state.new_group_name)
                        .desired_width(80.0)
                        .hint_text("Group name");
                    let response = ui.add(text_edit);
                    let enter_pressed =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let name_valid = !tool_state.new_group_name.trim().is_empty();
                    if ui
                        .add_enabled(name_valid, egui::Button::new("+").small())
                        .clicked()
                        || (enter_pressed && name_valid)
                    {
                        action = GroundTypeAction::NewGroup;
                    }
                    let can_delete = tool_state.tile_pools.len() > 1;
                    if ui
                        .add_enabled(can_delete, egui::Button::new("-").small())
                        .clicked()
                    {
                        action = GroundTypeAction::DeleteGroup;
                    }
                });
            }

            ui.add_space(2.0);
        });

    let selected_idx = selected_ground as usize;
    if tool_state.ground_type_mode
        && let Some(pool) = tool_state.tile_pools.get(selected_idx)
    {
        let tile_count: usize = pool.buckets.iter().map(|b| b.tile_ids.len()).sum();
        if tile_count > 0 {
            let weights_id = ui.id().with("ground_type_weights_section");
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("Weights ({tile_count} tiles)")).small(),
            )
            .id_salt(weights_id)
            .default_open(false)
            .show(ui, |ui| {
                let cols = 5;
                let pool = &mut tool_state.tile_pools[selected_idx];
                let entries: Vec<(usize, usize, u16)> = pool
                    .buckets
                    .iter()
                    .enumerate()
                    .flat_map(|(bi, b)| {
                        b.tile_ids
                            .iter()
                            .enumerate()
                            .map(move |(ti, &tid)| (bi, ti, tid))
                    })
                    .collect();

                egui::Grid::new("weight_grid")
                    .num_columns(cols)
                    .spacing(egui::vec2(6.0, 4.0))
                    .show(ui, |ui| {
                        for (col, &(bi, _ti, tid)) in entries.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if let Some(handle) = ts.tile_handle(tid) {
                                    let img = egui::Image::new(handle)
                                        .fit_to_exact_size(egui::vec2(20.0, 20.0));
                                    ui.add(img);
                                }
                                let drag = egui::DragValue::new(&mut pool.buckets[bi].weight)
                                    .range(1..=100)
                                    .speed(0.15)
                                    .suffix("%");
                                if ui.add(drag).changed() {
                                    needs_save = true;
                                }
                            });

                            if (col + 1) % cols == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
            ui.separator();
        }
    }

    let selected_color = if tool_state.ground_type_mode {
        tool_state
            .tile_pools
            .get(selected_idx)
            .map_or(egui::Color32::WHITE, |p| {
                let [r, g, b] = p.color;
                egui::Color32::from_rgb(r, g, b)
            })
    } else {
        ui.visuals().selection.stroke.color
    };

    if tool_state.tile_pools_dirty {
        tool_state.tile_membership.clear();
        for (pool_idx, pool) in tool_state.tile_pools.iter().enumerate() {
            for bucket in &pool.buckets {
                for &tid in &bucket.tile_ids {
                    tool_state
                        .tile_membership
                        .entry(tid)
                        .or_default()
                        .push(pool_idx);
                }
            }
        }
        tool_state.tile_pools_dirty = false;
    }

    let tile_size = 44.0;
    let spacing = 3.0;
    let mut toggle_tile: Option<u16> = None;

    let terrain_of = |tile_id: u16| -> Option<wz_maplib::TerrainType> {
        terrain_types.and_then(|types| types.get(tile_id as usize).copied())
    };

    let mut regular_ids = Vec::new();
    let mut cliff_ids = Vec::new();
    let mut water_ids = Vec::new();
    for (id, _) in ts.iter_tiles() {
        match terrain_of(id) {
            Some(wz_maplib::TerrainType::Cliffface) => cliff_ids.push(id),
            Some(wz_maplib::TerrainType::Water) => water_ids.push(id),
            _ => regular_ids.push(id),
        }
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // egui 0.34's Button::selected only swaps bg_fill / fg_stroke,
            // not bg_stroke, so paint the selection border on top of the
            // response rect. Button::stroke changes the allocated width
            // and makes the wrap layout jitter as tiles flip selected.
            let selected_stroke = egui::Stroke::new(2.0_f32, selected_color);

            render_tile_grid(
                ui,
                &regular_ids,
                ts,
                tile_size,
                spacing,
                single_mode,
                selected_idx,
                selected_stroke,
                tool_state,
                &mut toggle_tile,
            );

            if !cliff_ids.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Cliffs")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(2.0);
                render_tile_grid(
                    ui,
                    &cliff_ids,
                    ts,
                    tile_size,
                    spacing,
                    single_mode,
                    selected_idx,
                    selected_stroke,
                    tool_state,
                    &mut toggle_tile,
                );
            }

            if !water_ids.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Water")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(2.0);
                render_tile_grid(
                    ui,
                    &water_ids,
                    ts,
                    tile_size,
                    spacing,
                    single_mode,
                    selected_idx,
                    selected_stroke,
                    tool_state,
                    &mut toggle_tile,
                );
            }
        });

    if let Some(tile_id) = toggle_tile
        && let Some(pool) = tool_state.tile_pools.get_mut(selected_idx)
    {
        let existing = pool
            .buckets
            .iter()
            .position(|b| b.tile_ids.contains(&tile_id));

        if let Some(bucket_idx) = existing {
            pool.buckets[bucket_idx].tile_ids.retain(|&t| t != tile_id);
            if pool.buckets[bucket_idx].tile_ids.is_empty() {
                pool.buckets.remove(bucket_idx);
            }
            needs_save = true;
        } else {
            pool.buckets
                .push(crate::tools::ground_type_brush::TileBucket {
                    tile_ids: vec![tile_id],
                    weight: 1,
                });
            needs_save = true;
        }
    }

    if needs_save {
        tool_state.tile_pools_dirty = true;
        action = GroundTypeAction::Save;
    }

    action
}

#[expect(
    clippy::too_many_arguments,
    reason = "shared rendering of regular and cliff tile grids; collapsing into a struct just shifts the noise"
)]
fn render_tile_grid(
    ui: &mut Ui,
    tile_ids: &[u16],
    ts: &TilesetData,
    tile_size: f32,
    spacing: f32,
    single_mode: bool,
    selected_idx: usize,
    selected_stroke: egui::Stroke,
    tool_state: &mut crate::tools::ToolState,
    toggle_tile: &mut Option<u16>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
        ui.visuals_mut().selection.bg_fill = egui::Color32::TRANSPARENT;

        for &tile_id in tile_ids {
            let Some(handle) = ts.tile_handle(tile_id) else {
                continue;
            };
            if single_mode {
                let is_selected = tool_state
                    .texture_paint()
                    .is_some_and(|t| t.selected_texture == tile_id);
                let img =
                    egui::Image::new(handle).fit_to_exact_size(egui::vec2(tile_size, tile_size));
                let btn = egui::Button::new(img).selected(is_selected);
                let response = ui.add(btn);
                if is_selected {
                    ui.painter().rect_stroke(
                        response.rect,
                        0.0,
                        selected_stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                let clicked = response.clicked();
                if response.hovered() {
                    response.on_hover_text(format!("Tile {tile_id}"));
                }
                if clicked {
                    if let Some(t) = tool_state.texture_paint_mut() {
                        t.selected_texture = tile_id;
                    }
                    tool_state.active_tool = crate::tools::ToolId::TexturePaint;
                }
            } else {
                let groups = tool_state.tile_membership.get(&tile_id);
                let in_selected = groups.is_some_and(|g| g.contains(&selected_idx));

                let img =
                    egui::Image::new(handle).fit_to_exact_size(egui::vec2(tile_size, tile_size));

                let btn = egui::Button::new(img).selected(in_selected);
                let response = ui.add(btn);
                if in_selected {
                    ui.painter().rect_stroke(
                        response.rect,
                        0.0,
                        selected_stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                let clicked = response.clicked();

                if response.hovered() {
                    let other_groups: Vec<&str> = groups
                        .map(|g| {
                            g.iter()
                                .filter(|&&i| i != selected_idx)
                                .filter_map(|&i| tool_state.tile_pools.get(i).map(|p| &*p.name))
                                .collect()
                        })
                        .unwrap_or_default();
                    let action = if in_selected {
                        "click to remove"
                    } else {
                        "click to add"
                    };
                    let hover_text = if other_groups.is_empty() {
                        format!("Tile {tile_id} ({action})")
                    } else {
                        format!(
                            "Tile {tile_id} ({action}, also in {})",
                            other_groups.join(", ")
                        )
                    };
                    response.on_hover_text(hover_text);
                }

                if clicked {
                    *toggle_tile = Some(tile_id);
                }
            }
        }
    });
}
