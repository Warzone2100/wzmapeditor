//! Per-tab navigation state and sub-panel rendering for the designer.
//!
//! `DesignerTabs` owns the stable `egui::Id`s and per-tab state so the
//! orchestrator in `ui.rs` can stay focused on lifecycle dispatch.

use egui::{Color32, RichText, Stroke, Ui, Vec2};

use wz_stats::StatsDatabase;
use wz_stats::templates::TemplateStats;

use crate::designer::state::{Designer, DesignerCtx, SlotTab, TurretKind};
use crate::designer::validation::{self, DroidFamily, Issue, Severity, Slot};
use crate::viewport::model_loader::ModelLoader;

/// Template id used internally for the live preview. The in-progress
/// buffer is registered under this id so droid-composite rendering
/// resolves it like any other template.
pub(crate) const PREVIEW_ID: &str = "__designer_preview__";

/// Auto-rotation rate (rad/s) for the live preview. ~10s per revolution.
const PREVIEW_ROTATION_RATE: f32 = 0.6;

/// Per-tab navigation state.
pub struct DesignerTabs {
    /// Slot whose component grid is visible on the left.
    pub active_slot: SlotTab,
}

impl Default for DesignerTabs {
    fn default() -> Self {
        Self {
            active_slot: SlotTab::Body,
        }
    }
}

/// Lay out a `label | widget` row in an `egui::Grid`. Returns the
/// widget's `changed()` so callers can mark documents dirty.
pub(crate) fn property_row(
    ui: &mut Ui,
    label: &str,
    widget: impl FnOnce(&mut Ui) -> egui::Response,
) -> bool {
    ui.label(label);
    let resp = widget(ui);
    ui.end_row();
    resp.changed()
}

impl DesignerTabs {
    pub fn slot_selector(&mut self, ui: &mut Ui, designer: &mut Designer, dctx: &DesignerCtx<'_>) {
        ui.label(RichText::new("Slots").small().weak());
        ui.add_space(2.0);

        let body = dctx.db.bodies.get(&designer.buffer.body);
        let max_weapons = body.map_or(1_u8, wz_stats::bodies::BodyStats::weapon_slot_count);

        // VTOL droids cannot mount system turrets.
        let is_vtol = dctx
            .db
            .propulsion
            .get(&designer.buffer.propulsion)
            .is_some_and(|p| validation::propulsion_medium(p) == validation::PropulsionMedium::Air);

        // Reset if the active slot is gone (fewer weapon slots, VTOL hides Systems).
        match self.active_slot {
            SlotTab::Weapon(i) if i >= max_weapons => {
                self.active_slot = SlotTab::Weapon(0);
            }
            SlotTab::Turret if is_vtol => {
                self.active_slot = SlotTab::Weapon(0);
            }
            _ => {}
        }

        let mut slots: Vec<SlotTab> = vec![SlotTab::Body, SlotTab::Propulsion, SlotTab::Weapon(0)];
        for i in 1..max_weapons {
            slots.push(SlotTab::Weapon(i));
        }
        if !is_vtol {
            slots.push(SlotTab::Turret);
        }

        for slot in &slots {
            let slot = *slot;
            let selected = self.active_slot == slot;
            let filled = slot_is_filled(slot, &designer.buffer);
            let label_color = if filled {
                Color32::from_rgb(160, 220, 255)
            } else {
                Color32::GRAY
            };
            let equipped = slot_equipped_name(slot, &designer.buffer);

            // Falls back to a unicode glyph when the WZ2100 sprite is
            // missing, so the designer still works without the data dir.
            let icon_tex = designer.icon_for(slot, ui.ctx(), dctx.data_dir);
            let btn_height = if equipped.is_some() { 48.0 } else { 36.0 };
            let resp = ui
                .scope(|ui| {
                    ui.set_width(120.0);
                    let btn = egui::Button::new("")
                        .selected(selected)
                        .min_size(Vec2::new(120.0, btn_height));
                    let r = ui.add(btn);
                    let inner_rect = r.rect;
                    let painter = ui.painter_at(inner_rect);

                    let icon_size = Vec2::splat(20.0);
                    let label_y = if equipped.is_some() {
                        inner_rect.top() + 13.0
                    } else {
                        inner_rect.center().y
                    };
                    let icon_pos = egui::pos2(inner_rect.left() + 6.0, label_y - icon_size.y * 0.5);
                    let icon_rect = egui::Rect::from_min_size(icon_pos, icon_size);
                    if let Some(tex) = icon_tex {
                        painter.image(
                            tex.id(),
                            icon_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    } else {
                        painter.text(
                            icon_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            slot.icon(),
                            egui::FontId::proportional(14.0),
                            label_color,
                        );
                    }

                    let text_pos = egui::pos2(inner_rect.left() + 32.0, label_y);
                    painter.text(
                        text_pos,
                        egui::Align2::LEFT_CENTER,
                        slot.label(),
                        egui::FontId::proportional(12.0),
                        label_color,
                    );

                    if let Some(ref name) = equipped {
                        let name_pos = egui::pos2(inner_rect.left() + 6.0, label_y + 14.0);
                        painter.text(
                            name_pos,
                            egui::Align2::LEFT_CENTER,
                            truncate_name(name, 16),
                            egui::FontId::proportional(10.0),
                            Color32::from_gray(140),
                        );
                    }

                    r
                })
                .inner;
            if resp.on_hover_text(slot.tooltip()).clicked() {
                self.active_slot = slot;
            }
        }
    }

    pub fn component_grid(
        &mut self,
        ui: &mut Ui,
        designer: &mut Designer,
        dctx: &mut DesignerCtx<'_>,
    ) {
        ui.label(
            RichText::new(format!("{} components", self.active_slot.label()))
                .small()
                .weak(),
        );
        ui.separator();

        let is_optional = matches!(self.active_slot, SlotTab::Weapon(i) if i > 0);
        if is_optional && ui.small_button("\u{2716} None").clicked() {
            clear_slot(self.active_slot, &mut designer.buffer);
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let options = component_options(
                    self.active_slot,
                    dctx.db,
                    &designer.buffer,
                    dctx.model_loader,
                );
                let columns = 3;
                let tile_size = Vec2::new(96.0, 112.0);

                egui::Grid::new("designer_comp_grid")
                    .num_columns(columns)
                    .spacing([4.0, 4.0])
                    .show(ui, |ui| {
                        for (i, opt) in options.iter().enumerate() {
                            render_component_tile(
                                ui,
                                tile_size,
                                opt,
                                self.active_slot,
                                designer,
                                dctx,
                            );
                            if (i + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
    }

    // Kept as a method so future per-tab state (scroll offset, zoom)
    // can attach to `self` without changing call sites.
    #[expect(clippy::unused_self, reason = "DesignerTabs method by design")]
    pub fn preview(&self, ui: &mut Ui, designer: &mut Designer, dctx: &mut DesignerCtx<'_>) {
        // Wall-clock dt makes the spin rate frame-rate independent.
        // Cap at 50ms so an unfocused window doesn't jump.
        let dt = ui.ctx().input(|i| i.stable_dt).clamp(0.0, 0.05);
        designer.preview_rotation =
            (designer.preview_rotation + PREVIEW_ROTATION_RATE * dt) % std::f32::consts::TAU;

        egui::Frame::new()
            .fill(Color32::from_rgb(10, 14, 20))
            .stroke(Stroke::new(1.0_f32, Color32::from_gray(40)))
            .inner_margin(8)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Live preview").small().weak());
                    ui.add_space(4.0);

                    let preview_size = Vec2::new(380.0, 280.0);
                    if let Some(tex_id) = dctx.thumbnails.update_droid_preview(
                        PREVIEW_ID,
                        designer.preview_rotation,
                        dctx.db,
                        dctx.model_loader,
                        dctx.render_state,
                    ) {
                        let img = egui::Image::new((tex_id, preview_size))
                            .fit_to_exact_size(preview_size)
                            .maintain_aspect_ratio(true);
                        ui.add(img);
                    } else {
                        ui.add_sized(preview_size, egui::Spinner::new());
                    }

                    ui.ctx().request_repaint();

                    ui.add_space(6.0);
                    stats_readout(ui, &designer.buffer, dctx.db);
                });
            });
    }

    #[expect(clippy::unused_self, reason = "DesignerTabs method by design")]
    pub fn validation_strip(&self, ui: &mut Ui, issues: &[Issue]) {
        if issues.is_empty() {
            ui.colored_label(Color32::from_rgb(120, 220, 120), "\u{2714} Valid design");
            return;
        }
        for issue in issues {
            let color = match issue.severity {
                Severity::Error => Color32::from_rgb(255, 120, 120),
                Severity::Warning => Color32::from_rgb(240, 200, 80),
            };
            let marker = match issue.severity {
                Severity::Error => "\u{2716}",
                Severity::Warning => "\u{26A0}",
            };
            ui.horizontal(|ui| {
                ui.colored_label(color, marker);
                ui.label(RichText::new(&issue.message).color(color));
                ui.label(RichText::new(slot_label(issue.slot)).small().weak());
            });
        }
    }
}

fn slot_equipped_name(slot: SlotTab, t: &TemplateStats) -> Option<String> {
    match slot {
        SlotTab::Body if !t.body.is_empty() => Some(t.body.clone()),
        SlotTab::Propulsion if !t.propulsion.is_empty() => Some(t.propulsion.clone()),
        SlotTab::Weapon(i) => t.weapons.get(i as usize).filter(|w| !w.is_empty()).cloned(),
        SlotTab::Turret => t
            .sensor
            .clone()
            .or_else(|| t.ecm.clone())
            .or_else(|| t.repair.clone())
            .or_else(|| t.construct.clone())
            .or_else(|| t.brain.clone()),
        _ => None,
    }
}

/// Truncate to `max_chars`, appending "...". Uses `char_indices` so
/// multi-byte UTF-8 doesn't panic.
fn truncate_name(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars.saturating_sub(3))
            .map_or(s.len(), |(i, _)| i);
        format!("{}...", &s[..end])
    }
}

fn slot_is_filled(slot: SlotTab, t: &TemplateStats) -> bool {
    match slot {
        SlotTab::Body => !t.body.is_empty(),
        SlotTab::Propulsion => !t.propulsion.is_empty(),
        SlotTab::Weapon(i) => t.weapons.get(i as usize).is_some_and(|w| !w.is_empty()),
        SlotTab::Turret => {
            t.sensor.is_some()
                || t.ecm.is_some()
                || t.repair.is_some()
                || t.construct.is_some()
                || t.brain.is_some()
        }
    }
}

/// A component choice presented in the grid.
struct ComponentOption {
    id: String,
    display_name: String,
    /// Parts in model-local coords. Single PIE for bodies/propulsion;
    /// `[mount, model_at_mount_connector]` for weapons and turrets so
    /// the tile shows the assembled turret. Bare weapon PIEs (especially
    /// VTOL ones) are visually ambiguous without their mount.
    thumbnail_parts: Vec<(String, glam::Vec3)>,
    /// Stable cache key for the composite thumbnail.
    thumbnail_key: String,
    compatible: bool,
    incompat_reason: Option<String>,
    selected: bool,
    /// Sub-category for items in the unified `Turret` slot. `None` for
    /// body / propulsion / weapon tiles.
    turret_kind: Option<TurretKind>,
}

fn component_options(
    slot: SlotTab,
    db: &StatsDatabase,
    buf: &TemplateStats,
    model_loader: &mut Option<ModelLoader>,
) -> Vec<ComponentOption> {
    let family = validation::droid_family(buf.droid_type.as_deref());
    let want_cyborg = matches!(family, DroidFamily::Cyborg | DroidFamily::SuperCyborg);

    match slot {
        SlotTab::Body => {
            let mut items: Vec<ComponentOption> = db
                .bodies
                .iter()
                .filter(|(_, b)| validation::body_selectable(b, family))
                .map(|(id, b)| ComponentOption {
                    id: id.clone(),
                    display_name: friendly_name(id, b.name.as_deref()),
                    thumbnail_parts: single_part(b.model.as_deref()),
                    thumbnail_key: format!("dsn_body_{id}"),
                    compatible: true,
                    incompat_reason: None,
                    selected: id == &buf.body,
                    turret_kind: None,
                })
                .collect();
            items.sort_by(|a, b| a.id.cmp(&b.id));
            items
        }
        SlotTab::Propulsion => {
            let mut items: Vec<ComponentOption> = db
                .propulsion
                .iter()
                .filter(|(_, p)| validation::propulsion_allowed(p, family))
                .map(|(id, p)| ComponentOption {
                    id: id.clone(),
                    display_name: friendly_name(id, p.name.as_deref()),
                    thumbnail_parts: single_part(p.model.as_deref()),
                    thumbnail_key: format!("dsn_prop_{id}"),
                    compatible: true,
                    incompat_reason: None,
                    selected: id == &buf.propulsion,
                    turret_kind: None,
                })
                .collect();
            items.sort_by(|a, b| a.id.cmp(&b.id));
            items
        }
        SlotTab::Weapon(i) => {
            let current = buf.weapons.get(i as usize).cloned().unwrap_or_default();
            let medium = db.propulsion.get(&buf.propulsion).map_or(
                validation::PropulsionMedium::Unknown,
                validation::propulsion_medium,
            );
            let mut items: Vec<ComponentOption> = db
                .weapons
                .iter()
                .filter(|(_, w)| {
                    validation::weapon_allowed(w, family)
                        && validation::weapon_fits_propulsion(w, medium)
                })
                .map(|(id, w)| ComponentOption {
                    id: id.clone(),
                    display_name: friendly_name(id, w.name.as_deref()),
                    thumbnail_parts: turret_on_chassis_parts(
                        w.mount_model.as_deref(),
                        w.model.as_deref(),
                        model_loader,
                    ),
                    thumbnail_key: turret_cache_key(
                        "weapon",
                        w.mount_model.as_deref(),
                        w.model.as_deref(),
                    ),
                    compatible: true,
                    incompat_reason: None,
                    selected: id == &current,
                    turret_kind: None,
                })
                .collect();
            items.sort_by(|a, b| a.id.cmp(&b.id));
            items
        }
        SlotTab::Turret => collect_turret_options(db, buf, want_cyborg, model_loader),
    }
}

fn friendly_name(id: &str, name: Option<&str>) -> String {
    match name {
        Some(n) if !n.is_empty() && !n.starts_with('*') => n.to_string(),
        _ => id.to_string(),
    }
}

fn single_part(model: Option<&str>) -> Vec<(String, glam::Vec3)> {
    match model {
        Some(m) if !m.is_empty() => vec![(m.to_string(), glam::Vec3::ZERO)],
        _ => Vec::new(),
    }
}

/// Build `thumbnail_parts` for a turret (mount + weapon, no chassis).
/// Returns `[mount@origin, weapon@mount_connector]`.
fn turret_on_chassis_parts(
    mount: Option<&str>,
    turret_model: Option<&str>,
    model_loader: &mut Option<ModelLoader>,
) -> Vec<(String, glam::Vec3)> {
    let mut parts = Vec::new();

    let mount_connector = match (mount, model_loader.as_mut()) {
        (Some(m), Some(loader)) if !m.is_empty() => {
            parts.push((m.to_string(), glam::Vec3::ZERO));
            loader
                .get_connectors(m)
                .first()
                .copied()
                .unwrap_or(glam::Vec3::ZERO)
        }
        _ => glam::Vec3::ZERO,
    };

    if let Some(m) = turret_model.filter(|s| !s.is_empty()) {
        parts.push((m.to_string(), mount_connector));
    }
    parts
}

fn turret_cache_key(prefix: &str, mount: Option<&str>, model: Option<&str>) -> String {
    format!(
        "dsn_{prefix}_{}_{}",
        mount.unwrap_or("-"),
        model.unwrap_or("-")
    )
}

fn collect_turret_options(
    db: &StatsDatabase,
    buf: &TemplateStats,
    want_cyborg: bool,
    model_loader: &mut Option<ModelLoader>,
) -> Vec<ComponentOption> {
    let mut out: Vec<ComponentOption> = Vec::new();

    for (id, s) in &db.sensor {
        if !s.designable || s.location.as_deref() == Some("DEFAULT") {
            continue;
        }
        if (s.usage_class.as_deref() == Some("Cyborg")) != want_cyborg {
            continue;
        }
        out.push(ComponentOption {
            id: id.clone(),
            display_name: format!("{} (sensor)", friendly_name(id, s.name.as_deref())),
            thumbnail_parts: turret_on_chassis_parts(
                s.mount_model.as_deref(),
                s.sensor_model.as_deref(),
                model_loader,
            ),
            thumbnail_key: turret_cache_key(
                "sensor",
                s.mount_model.as_deref(),
                s.sensor_model.as_deref(),
            ),
            compatible: true,
            incompat_reason: None,
            selected: buf.sensor.as_deref() == Some(id.as_str()),
            turret_kind: Some(TurretKind::Sensor),
        });
    }
    for (id, e) in &db.ecm {
        if !e.designable || e.location.as_deref() == Some("DEFAULT") {
            continue;
        }
        if (e.usage_class.as_deref() == Some("Cyborg")) != want_cyborg {
            continue;
        }
        out.push(ComponentOption {
            id: id.clone(),
            display_name: format!("{} (ECM)", friendly_name(id, e.name.as_deref())),
            thumbnail_parts: turret_on_chassis_parts(
                e.mount_model.as_deref(),
                e.sensor_model.as_deref(),
                model_loader,
            ),
            thumbnail_key: turret_cache_key(
                "ecm",
                e.mount_model.as_deref(),
                e.sensor_model.as_deref(),
            ),
            compatible: true,
            incompat_reason: None,
            selected: buf.ecm.as_deref() == Some(id.as_str()),
            turret_kind: Some(TurretKind::Ecm),
        });
    }
    for (id, r) in &db.repair {
        if !r.designable || r.location.as_deref() == Some("DEFAULT") {
            continue;
        }
        if (r.usage_class.as_deref() == Some("Cyborg")) != want_cyborg {
            continue;
        }
        out.push(ComponentOption {
            id: id.clone(),
            display_name: format!("{} (repair)", friendly_name(id, r.name.as_deref())),
            thumbnail_parts: turret_on_chassis_parts(
                r.mount_model.as_deref(),
                r.model.as_deref(),
                model_loader,
            ),
            thumbnail_key: turret_cache_key("repair", r.mount_model.as_deref(), r.model.as_deref()),
            compatible: true,
            incompat_reason: None,
            selected: buf.repair.as_deref() == Some(id.as_str()),
            turret_kind: Some(TurretKind::Repair),
        });
    }
    for (id, c) in &db.construct {
        if !c.designable {
            continue;
        }
        if (c.usage_class.as_deref() == Some("Cyborg")) != want_cyborg {
            continue;
        }
        out.push(ComponentOption {
            id: id.clone(),
            display_name: format!("{} (construct)", friendly_name(id, c.name.as_deref())),
            thumbnail_parts: turret_on_chassis_parts(
                c.mount_model.as_deref(),
                c.sensor_model.as_deref(),
                model_loader,
            ),
            thumbnail_key: turret_cache_key(
                "construct",
                c.mount_model.as_deref(),
                c.sensor_model.as_deref(),
            ),
            compatible: true,
            incompat_reason: None,
            selected: buf.construct.as_deref() == Some(id.as_str()),
            turret_kind: Some(TurretKind::Construct),
        });
    }
    for (id, b) in &db.brain {
        if !b.designable {
            continue;
        }
        if (b.usage_class.as_deref() == Some("Cyborg")) != want_cyborg {
            continue;
        }
        // Brains reuse the linked weapon's PIE so the tile looks like
        // a real commander turret rather than an empty placeholder.
        let (mount, model) = b
            .turret
            .as_deref()
            .and_then(|w_id| db.weapons.get(w_id))
            .map(|w| (w.mount_model.clone(), w.model.clone()))
            .unwrap_or_default();
        out.push(ComponentOption {
            id: id.clone(),
            display_name: format!("{} (commander)", friendly_name(id, b.name.as_deref())),
            thumbnail_parts: turret_on_chassis_parts(
                mount.as_deref(),
                model.as_deref(),
                model_loader,
            ),
            thumbnail_key: turret_cache_key("brain", mount.as_deref(), model.as_deref()),
            compatible: true,
            incompat_reason: None,
            selected: buf.brain.as_deref() == Some(id.as_str()),
            turret_kind: Some(TurretKind::Brain),
        });
    }

    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn render_component_tile(
    ui: &mut Ui,
    size: Vec2,
    opt: &ComponentOption,
    active_slot: SlotTab,
    designer: &mut Designer,
    dctx: &mut DesignerCtx<'_>,
) {
    let stroke_color = if opt.selected {
        Color32::from_rgb(90, 200, 255)
    } else if !opt.compatible {
        Color32::from_rgb(180, 60, 60)
    } else {
        Color32::from_gray(80)
    };

    let frame = egui::Frame::group(ui.style())
        .stroke(Stroke::new(
            if opt.selected { 2.0_f32 } else { 1.0_f32 },
            stroke_color,
        ))
        .fill(if opt.selected {
            Color32::from_rgb(24, 40, 56)
        } else {
            Color32::from_gray(20)
        });

    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        frame.show(
            &mut ui.new_child(egui::UiBuilder::new().max_rect(rect)),
            |ui| {
                ui.vertical_centered(|ui| {
                    // Turret tiles composite mount+weapon. Single-part
                    // tiles take the cheaper single-PIE path.
                    let img_size = Vec2::new(size.x - 10.0, 72.0);
                    let tex = if opt.thumbnail_parts.is_empty() {
                        None
                    } else if opt.thumbnail_parts.len() == 1 {
                        dctx.thumbnails.request_model_thumbnail(
                            ui.ctx(),
                            &opt.thumbnail_parts[0].0,
                            dctx.model_loader,
                            dctx.render_state,
                        )
                    } else {
                        dctx.thumbnails.request_composite_thumbnail(
                            ui.ctx(),
                            &opt.thumbnail_key,
                            &opt.thumbnail_parts,
                            dctx.model_loader,
                            dctx.render_state,
                        )
                    };
                    if let Some(tex) = tex {
                        let img = egui::Image::new(tex).fit_to_exact_size(img_size);
                        ui.add(img);
                    } else if opt.thumbnail_parts.is_empty() {
                        ui.add_sized(img_size, egui::Label::new(RichText::new("?").weak()));
                    } else {
                        ui.add_sized(img_size, egui::Spinner::new());
                    }
                    ui.add_space(2.0);
                    let mut text = RichText::new(&opt.display_name).small();
                    if !opt.compatible {
                        text = text.color(Color32::from_rgb(200, 120, 120));
                    }
                    ui.label(text);
                });
            },
        );
    }

    let resp = resp.on_hover_ui(|ui| {
        ui.label(&opt.display_name);
        if let Some(ref reason) = opt.incompat_reason {
            ui.colored_label(Color32::from_rgb(255, 120, 120), reason);
        }
    });

    if resp.clicked() && opt.compatible {
        apply_component_choice(
            active_slot,
            &opt.id,
            opt.turret_kind,
            &mut designer.buffer,
            dctx.db,
        );
    }
}

fn apply_component_choice(
    slot: SlotTab,
    id: &str,
    turret_kind: Option<TurretKind>,
    buf: &mut TemplateStats,
    db: &StatsDatabase,
) {
    match slot {
        SlotTab::Body => buf.body = id.to_string(),
        SlotTab::Propulsion => {
            // Switching ground <-> VTOL invalidates equipped weapons.
            let old_medium = db.propulsion.get(&buf.propulsion).map_or(
                validation::PropulsionMedium::Unknown,
                validation::propulsion_medium,
            );
            let new_medium = db.propulsion.get(id).map_or(
                validation::PropulsionMedium::Unknown,
                validation::propulsion_medium,
            );
            buf.propulsion = id.to_string();
            if old_medium != new_medium {
                buf.weapons.clear();
                if new_medium == validation::PropulsionMedium::Air {
                    clear_all_turrets(buf);
                }
            }
        }
        SlotTab::Weapon(i) => {
            // Weapons and non-brain system turrets are mutually exclusive
            // in WZ2100, so picking a weapon clears any system turret.
            clear_all_turrets(buf);
            while buf.weapons.len() <= i as usize {
                buf.weapons.push(String::new());
            }
            buf.weapons[i as usize] = id.to_string();
        }
        SlotTab::Turret => {
            // Support turrets are mutually exclusive in-game. Non-brain
            // systems also replace weapons; brain/commander keeps them.
            clear_all_turrets(buf);
            let is_brain = turret_kind == Some(TurretKind::Brain);
            if !is_brain {
                buf.weapons.clear();
            }
            match turret_kind {
                Some(TurretKind::Sensor) => buf.sensor = Some(id.to_string()),
                Some(TurretKind::Ecm) => buf.ecm = Some(id.to_string()),
                Some(TurretKind::Repair) => buf.repair = Some(id.to_string()),
                Some(TurretKind::Construct) => buf.construct = Some(id.to_string()),
                Some(TurretKind::Brain) => buf.brain = Some(id.to_string()),
                None => log::warn!("turret option missing TurretKind for id={id}"),
            }
        }
    }
}

fn clear_all_turrets(buf: &mut TemplateStats) {
    buf.sensor = None;
    buf.ecm = None;
    buf.repair = None;
    buf.construct = None;
    buf.brain = None;
}

fn clear_slot(slot: SlotTab, buf: &mut TemplateStats) {
    match slot {
        SlotTab::Weapon(i) if (i as usize) < buf.weapons.len() => {
            buf.weapons.remove(i as usize);
        }
        SlotTab::Turret => clear_all_turrets(buf),
        // Body / Propulsion / Weapon(0) are required.
        _ => {}
    }
}

fn stats_readout(ui: &mut Ui, t: &TemplateStats, db: &StatsDatabase) {
    let body = db.bodies.get(&t.body);
    let prop = db.propulsion.get(&t.propulsion);

    egui::Grid::new("designer_stats")
        .num_columns(2)
        .spacing([16.0, 2.0])
        .show(ui, |ui| {
            property_row(ui, "HP", |ui| {
                ui.label(
                    body.and_then(|b| b.hitpoints)
                        .map_or("-".into(), |v| v.to_string()),
                )
            });

            property_row(ui, "Armour (K / T)", |ui| {
                ui.label(format!(
                    "{} / {}",
                    body.and_then(|b| b.armour_kinetic)
                        .map_or("-".into(), |v| v.to_string()),
                    body.and_then(|b| b.armour_heat)
                        .map_or("-".into(), |v| v.to_string())
                ))
            });

            let weight =
                body.and_then(|b| b.weight).unwrap_or(0) + prop.and_then(|p| p.weight).unwrap_or(0);
            property_row(ui, "Weight", |ui| ui.label(weight.to_string()));

            property_row(ui, "Speed", |ui| {
                ui.label(
                    prop.and_then(|p| p.speed)
                        .map_or("-".into(), |v| v.to_string()),
                )
            });

            property_row(ui, "Power", |ui| {
                ui.label(crate::designer::ui::estimate_power_cost(t, db).to_string())
            });
        });
}

fn slot_label(slot: Slot) -> &'static str {
    match slot {
        Slot::Body => "(Body)",
        Slot::Propulsion => "(Propulsion)",
        Slot::Weapon(_) => "(Weapon)",
        Slot::Brain => "(Brain)",
        Slot::Sensor => "(Sensor)",
        Slot::Ecm => "(ECM)",
        Slot::Repair => "(Repair)",
        Slot::Construct => "(Construct)",
        Slot::General => "",
    }
}
