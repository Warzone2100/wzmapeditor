//! Dockable tab viewer - dispatches UI rendering for each panel tab.

use egui::{Color32, RichText};

use super::{DockTab, EditorApp};
use crate::ui;

const PROBLEM_COLOR: Color32 = Color32::from_rgb(255, 80, 80);
const WARNING_COLOR: Color32 = Color32::from_rgb(255, 200, 60);

/// Tab viewer for the full dockable layout.
pub(super) struct DockTabViewer<'a> {
    pub(super) app: &'a mut EditorApp,
}

impl egui_dock::TabViewer for DockTabViewer<'_> {
    type Tab = DockTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        if matches!(tab, DockTab::Validation)
            && let Some(results) = self.app.validation_results.as_ref()
        {
            let problems = results.problem_count();
            let warnings = results.warning_count();
            let total = problems + warnings;
            if total > 0 {
                let color = if problems > 0 {
                    PROBLEM_COLOR
                } else {
                    WARNING_COLOR
                };
                return RichText::new(format!("Problems ({total})"))
                    .color(color)
                    .strong()
                    .into();
            }
        }
        tab.to_string().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            DockTab::Viewport => {
                ui::viewport_panel::show_viewport(ui, self.app);
            }
            DockTab::Terrain => {
                ui::tool_palette::show_tool_palette(ui, self.app);
            }
            DockTab::TilesetBrowser => {
                let ttp = self
                    .app
                    .document
                    .as_ref()
                    .and_then(|d| d.map.terrain_types.as_ref())
                    .map(|t| t.terrain_types.as_slice());
                let gt_action = ui::tileset_browser::show_tileset_browser(
                    ui,
                    self.app.tileset.as_ref(),
                    ttp,
                    &mut self.app.tool_state,
                );
                match gt_action {
                    ui::tileset_browser::GroundTypeAction::Save => {
                        self.app.save_custom_tile_groups();
                    }
                    ui::tileset_browser::GroundTypeAction::NewGroup => {
                        self.app.add_new_tile_group();
                    }
                    ui::tileset_browser::GroundTypeAction::DeleteGroup => {
                        self.app.delete_selected_tile_group();
                    }
                    ui::tileset_browser::GroundTypeAction::None => {}
                }
            }
            DockTab::AssetBrowser => {
                ui::asset_browser::show_asset_browser_inner(
                    ui,
                    self.app.stats.as_ref(),
                    &mut self.app.tool_state,
                    &mut self.app.model_thumbnails,
                    &mut self.app.model_loader,
                    self.app.wgpu_render_state.as_ref(),
                    Some(&self.app.custom_templates),
                );
            }
            DockTab::Properties => {
                ui::property_panel::show_property_panel(ui, self.app);
            }
            DockTab::Minimap => {
                ui::minimap::show_minimap_tab(ui, self.app);
            }
            DockTab::Hierarchy => {
                ui::hierarchy_panel::show_hierarchy(ui, self.app);
            }
            DockTab::Validation => {
                ui::validation_panel::show_validation_panel(ui, self.app);
            }
            DockTab::OutputLog => {
                self.app.output_log.ui(ui);
            }
            DockTab::Balance => {
                ui::balance_panel::show_balance_panel(ui, self.app);
            }
            DockTab::Unknown => {
                ui.label("This tab is no longer available.");
            }
        }
    }

    fn is_closeable(&self, _tab: &Self::Tab) -> bool {
        false
    }

    fn allowed_in_windows(&self, tab: &mut Self::Tab) -> bool {
        // Viewport cannot be detached into a floating window.
        !matches!(tab, DockTab::Viewport)
    }

    fn scroll_bars(&self, _tab: &Self::Tab) -> [bool; 2] {
        // egui_dock 0.19 wraps each tab body in its own ScrollArea by
        // default. Several of our panels (asset browser, hierarchy, ...)
        // already create an inner ScrollArea. The nested pair oscillates
        // visibility every frame because the outer bar's width affects
        // whether the inner content fits, which feeds back into whether
        // the outer bar should appear, which... -> visible flicker.
        // Each tab manages its own scrolling, so disable the outer one.
        [false, false]
    }
}
