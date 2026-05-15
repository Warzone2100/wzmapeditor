//! Main application state and egui integration.

mod actions;
mod data_loading;
mod designer;
pub(crate) mod dialogs;
mod dock_viewer;
mod duplicate;
mod map_io;
pub mod output_log;
mod testing;
mod tileset;
mod types;
pub use types::*;

use output_log::{LogEntry, LogSeverity, LogSource, OutputLog};

use eframe::egui_wgpu;
use egui_dock::{DockArea, DockState, Style};

use crate::config::EditorConfig;
use crate::map::document::MapDocument;
use crate::tools::ToolState;
use crate::ui;
use crate::ui::map_browser::MapBrowserDialog;
use crate::ui::minimap::MinimapState;
use crate::ui::tileset_browser::TilesetData;
use crate::viewport::model_loader::ModelLoader;

/// Main application state.
pub struct EditorApp {
    /// The currently loaded map document (if any).
    pub document: Option<MapDocument>,
    /// Current tool state.
    pub tool_state: ToolState,
    /// Dockable panel layout (viewport + tool/browser tabs).
    pub dock: DockState<DockTab>,
    /// Structured Output-panel log (editor-curated + captured internal-crate warnings/errors).
    pub output_log: OutputLog,
    /// Persistent configuration.
    pub config: EditorConfig,
    /// Loaded tileset textures for the terrain painter.
    pub tileset: Option<TilesetData>,
    /// wgpu render state (cloned from eframe's creation context).
    pub wgpu_render_state: Option<egui_wgpu::RenderState>,
    /// Whether the terrain needs a full re-upload to GPU.
    pub terrain_dirty: bool,
    /// Tiles that have been edited and need incremental re-upload.
    ///
    /// Ignored while `terrain_dirty` is true - a full rebuild covers them.
    /// A non-empty set with `terrain_dirty = false` triggers the fast
    /// partial-update path in `EditorRenderer::update_terrain_tiles`.
    pub terrain_dirty_tiles: rustc_hash::FxHashSet<(u32, u32)>,
    /// Whether the lightmap needs recomputation.
    pub lightmap_dirty: bool,
    /// Whether the shadow map must be re-rendered next frame.
    ///
    /// Set whenever terrain, objects, or sun direction change. The shadow
    /// pass is expensive (2048² depth render); skipping it when nothing
    /// moved is the single biggest frame-time win.
    pub shadow_dirty: bool,
    /// Countdown frames before lightmap recomputes after a sun-direction change.
    ///
    /// Drag-scrubbing the sun slider was firing full-map raycasts every
    /// frame. Shadow still updates live (cheap); lightmap waits for this
    /// cooldown to hit zero - ~100 ms of sun stability - before running.
    pub(crate) sun_change_cooldown: u8,
    /// Whether the water mesh needs rebuilding.
    ///
    /// Separated from `terrain_dirty` so camera-only frames don't trigger
    /// vertex-buffer reuploads for water.
    pub water_dirty: bool,
    /// The tile currently under the mouse cursor (for brush preview).
    pub hovered_tile: Option<(u32, u32)>,
    /// Weather effect displayed in the viewport (view-only, not saved to map).
    pub view_weather: wz_maplib::Weather,
    /// Whether to show the grid overlay.
    pub show_grid: bool,
    /// Whether to show the build margin border overlay (4-tile unbuildable zone).
    pub show_border: bool,
    /// Whether to show script label overlays in the viewport.
    pub show_labels: bool,
    /// Show picking AABB wireframes for every object on the map (View menu).
    pub show_all_hitboxes: bool,
    /// Show picking AABB wireframes only for currently selected objects
    /// (Settings → Viewport).
    pub show_selection_hitboxes: bool,
    /// Whether to show gateway overlays in the viewport.
    pub show_gateways: bool,
    /// New map dialog state.
    pub new_map_dialog: NewMapDialog,
    /// Resize-map dialog state.
    pub resize_map_dialog: ResizeMapDialog,
    /// Save As metadata dialog state.
    pub save_as_metadata_dialog: SaveAsMetadataDialog,
    /// Map Properties dialog state (`Map > Properties`).
    pub map_properties_dialog: MapPropertiesDialog,
    /// Post-publish drag-in instruction modal.
    pub publish_instructions_dialog: PublishInstructionsDialog,
    /// Map generator dialog state.
    pub generator_dialog: crate::generator::dialog::GeneratorDialog,
    /// Game stats database (structures, features).
    pub stats: Option<wz_stats::StatsDatabase>,
    /// PIE model loader and cache.
    pub model_loader: Option<ModelLoader>,
    /// Whether object models need to be re-uploaded / instances rebuilt.
    pub objects_dirty: bool,
    /// Current tileset (detected from loaded map).
    pub current_tileset: crate::config::Tileset,
    /// Currently selected objects (structures/droids/features/labels).
    pub selection: Selection,
    /// All in-flight background loading tasks (extraction, ground textures, models, etc.).
    pub rt: RuntimeTasks,
    /// Map browser dialog state.
    pub map_browser: MapBrowserDialog,
    /// Rendering settings (fog, shadows, water, sky, sun direction).
    pub render_settings: crate::viewport::renderer::RenderSettings,
    /// Minimap overlay state.
    pub minimap: MinimapState,
    /// Cached 3D model preview thumbnails and generation state.
    pub model_thumbnails: crate::thumbnails::ThumbnailCache,
    /// Path where the current map was last saved or loaded from.
    ///
    /// Used by "Save" (Ctrl+S) for quick-save without a file dialog.
    /// `None` for newly created maps that haven't been saved yet.
    pub save_path: Option<std::path::PathBuf>,
    /// Deferred Ctrl+S flag - set inside the input closure, handled after it.
    ctrl_s_pressed: bool,
    /// Player count for the current map (used in the "Nc-" filename prefix).
    pub map_players: u8,
    /// Loaded ground type data for the current tileset (Medium terrain quality).
    pub ground_data: Option<crate::viewport::ground_types::GroundData>,
    /// Pending camera focus request (`world_x`, `world_z`) from hierarchy double-click.
    pub focus_request: Option<(f32, f32)>,
    /// Startup phase - gates editor UI while critical data loads in the background.
    pub startup_phase: StartupPhase,
    /// Running WZ2100 test game process (launched via "Test Map").
    pub test_process: Option<std::process::Child>,
    /// Temp files to clean up after the test game exits.
    test_temp_files: Vec<std::path::PathBuf>,
    /// Modal shown when copying the test map to the WZ2100 maps directory
    /// fails with `PermissionDenied`.
    pub permission_error_dialog: PermissionErrorDialog,
    /// Modal shown when a user-initiated `.wz` open or drag-drop fails.
    pub load_error_dialog: LoadErrorDialog,
    /// Text-edit buffer for the install-directory field on the Settings → Game page.
    pub settings_install_dir_text: String,
    /// Text-edit buffer for the test-game executable field on Settings → Game.
    pub settings_wz_exe_text: String,
    /// Text-edit buffer for the WZ configuration directory field on Settings → Game.
    pub settings_wz_config_dir_text: String,
    /// Most recent map validation results (`None` if never validated).
    pub validation_results: Option<wz_maplib::validate::ValidationResults>,
    /// Whether validation needs to be re-run (set when map content changes).
    pub(crate) validation_dirty: bool,
    /// Cooldown frame counter to batch rapid edits before re-validating.
    validation_cooldown: u8,
    /// Whether the Settings window is open.
    pub settings_open: bool,
    /// Which page is selected in the Settings window sidebar.
    pub settings_page: ui::settings_window::SettingsPage,
    /// Lazily-decoded editor icon used by the splash launcher and the About
    /// settings page. Decoded the first frame either UI shows.
    pub editor_icon: Option<egui::TextureHandle>,
    /// Set once we've attempted to decode the icon, so a corrupt PNG is
    /// reported once instead of every frame the splash or About page is open.
    pub editor_icon_tried: bool,
    /// Graphics backend the app was actually launched with.
    ///
    /// Captured from config at startup so the Rendering settings page can
    /// detect whether the user has changed `config.graphics_backend` since
    /// launch and needs a restart for the change to take effect.
    pub launched_graphics_backend: crate::config::GraphicsBackend,
    /// Present mode the swapchain was actually created with (cross-platform).
    /// Used by the Rendering settings page to show a "restart required"
    /// hint after the user changes the preference.
    pub launched_present_mode: crate::config::PresentMode,
    /// Action currently being rebound in the keybindings settings (waiting for key press).
    pub keybinding_capture: Option<crate::keybindings::Action>,
    /// Auto-save subsystem state.
    pub autosave: crate::autosave::AutoSaveState,
    /// Recoverable auto-save entries found on startup.
    pub recovery_entries: Vec<crate::autosave::AutoSaveEntry>,
    /// Droid Designer modal state.
    pub designer: crate::designer::Designer,
    /// Per-map custom droid templates (round-trip through templates.json).
    pub custom_templates: crate::designer::CustomTemplateStore,
    /// Whether the propulsion speed heatmap overlay is active.
    pub show_heatmap: bool,
    /// Selected propulsion class for the heatmap overlay.
    pub heatmap_propulsion: wz_stats::terrain_table::PropulsionClass,
    /// Whether heatmap GPU data needs re-uploading.
    pub heatmap_dirty: bool,
    /// Whether the FPS / frame-time readout is shown in the viewport corner.
    pub show_fps: bool,
    /// Rolling window of per-frame deltas (seconds), filled in `update()`.
    pub fps_samples: [f32; 120],
    /// Next write slot in the ring buffer.
    pub fps_idx: usize,
    /// True once the ring has been filled at least once (so unused slots
    /// don't drag the avg toward zero on the first ~2 seconds).
    pub fps_filled: bool,
    /// Cached "Backend | GPU name" label shown next to the FPS readout.
    pub gpu_info_label: String,
    /// Line-of-sight viewshed toggles (per-structure + show-all).
    pub viewshed: ViewshedSettings,
    /// True when the visibility texture or range-ring buffers need a rebuild.
    pub viewshed_dirty: bool,
    /// Per-player starting-balance summary (lazy-computed, cached).
    pub balance: crate::balance::BalanceState,
    /// Whether the editor's viewport window has OS focus this frame.
    /// Used to suppress animation-driven `request_repaint_after` calls so
    /// the editor goes idle in the background instead of churning frames.
    pub window_focused: bool,
    /// Wall-clock timestamp of the last frame painted under an FPS cap,
    /// used to compute how long to sleep when enforcing `fps_limit`.
    pub last_paint_at: Option<std::time::Instant>,
    /// `update()` call count, used to detect the first surviving frame
    /// so the launch sentinel can be cleared.
    pub update_count: u32,
    /// Whether HQ terrain textures (`high.wz` `hw-256` decals) are present
    /// on disk. Drives the "Remastered (HQ)" radio's enabled state.
    pub has_hq_textures: bool,
    /// One-shot receiver fed by the background update-check worker.
    /// Dropped once a value lands in `update_available` (or the worker exits).
    pub update_check_rx: Option<std::sync::mpsc::Receiver<crate::update_check::UpdateInfo>>,
    /// Newer release the user could upgrade to, surfaced as a toolbar button.
    pub update_available: Option<crate::update_check::UpdateInfo>,
}

impl std::fmt::Debug for EditorApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorApp")
            .field("has_document", &self.document.is_some())
            .field("active_tool", &self.tool_state.active_tool)
            .field("show_grid", &self.show_grid)
            .field("show_border", &self.show_border)
            .field("show_labels", &self.show_labels)
            .field("log_count", &self.output_log.len())
            .field("has_stats", &self.stats.is_some())
            .field("has_model_loader", &self.model_loader.is_some())
            .field("selection", &self.selection)
            .field("extracting", &self.rt.extraction_progress.is_some())
            .field("generator_dialog", &self.generator_dialog)
            .field("map_browser_open", &self.map_browser.open)
            .field("minimap", &self.minimap)
            .field("save_path", &self.save_path)
            .finish_non_exhaustive()
    }
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, output_log: OutputLog) -> Self {
        // egui 0.34 raised the default text sizes (Body/Button 12.5 -> 13.0,
        // Monospace 12.0 -> 13.0) which makes the whole UI feel zoomed in
        // versus the previous build. Restore the previous defaults.
        cc.egui_ctx.global_style_mut(|style| {
            use egui::{FontFamily::Proportional, FontId, TextStyle};
            style
                .text_styles
                .insert(TextStyle::Body, FontId::new(12.5, Proportional));
            style
                .text_styles
                .insert(TextStyle::Button, FontId::new(12.5, Proportional));
            style.text_styles.insert(
                TextStyle::Monospace,
                FontId::new(12.0, egui::FontFamily::Monospace),
            );
            // Anchor scroll bars to the panel edge instead of overlaying the
            // content. Use the full `solid()` preset rather than patching just
            // the `floating` boolean; the floating preset's other widths are
            // tuned for the overlay layout and produce a flicker on resize
            // when forced into solid mode.
            style.spacing.scroll = egui::style::ScrollStyle::solid();
            // egui 0.34's new Skrifa hinter snaps glyph advance widths to
            // pixels per frame, which makes measured text width jitter as
            // content scrolls, layout passes, or pixel offsets change. The
            // jitter pushes scroll-area content over the "needs scrollbar"
            // threshold and back, and every flip calls request_repaint
            // (scroll_area.rs:1482). At 400 fps that registers as constant
            // scrollbar flashing. Turning hinting off pins the metrics.
            style.visuals.text_options.font_hinting = false;
        });

        let wgpu_render_state = cc.wgpu_render_state.clone();
        let mut gpu_info_label = String::new();
        if let Some(ref render_state) = wgpu_render_state {
            let adapter_info = render_state.adapter.get_info();
            log::info!(
                "GPU: {} ({:?}, {:?})",
                adapter_info.name,
                adapter_info.backend,
                adapter_info.device_type
            );
            gpu_info_label = format!("{:?} | {}", adapter_info.backend, adapter_info.name);
            crate::viewport::init_viewport_resources(render_state);
            log::info!("wgpu 3D viewport initialized");
        }

        let config = EditorConfig::load();

        cc.egui_ctx.set_theme(config.theme_preference);

        let startup_init = crate::startup::pipeline::create_startup(&config);
        let startup_phase = startup_init.phase;
        let extraction_progress_arc = startup_init.extraction_progress;
        let extraction_rx_channel = startup_init.extraction_rx;
        let cli_path = startup_init.cli_path;

        // Use the remembered tileset so the startup tileset-match check doesn't
        // immediately discard and reload the background thread's output.
        let current_tileset = config
            .last_tileset
            .as_deref()
            .and_then(crate::config::Tileset::from_name)
            .unwrap_or(crate::config::Tileset::Arizona);

        let mut tool_state = ToolState::default();
        if let Some(grid) = config.asset_grid_view {
            tool_state.asset_grid_view = grid;
        }
        if let Some(size) = config.asset_thumb_size {
            tool_state.asset_thumb_size = size;
        }
        if let Some(show) = config.asset_show_campaign_only {
            tool_state.asset_show_campaign_only = show;
        }

        let render_settings =
            config
                .render_settings
                .unwrap_or_else(|| crate::viewport::renderer::RenderSettings {
                    fog_color: crate::viewport::renderer::FOG_ARIZONA,
                    ..Default::default()
                });

        // Fall back to the default layout when the saved one is missing new tabs.
        let dock = match config.dock_layout.clone() {
            Some(saved) => {
                let has_all = [
                    DockTab::Properties,
                    DockTab::Minimap,
                    DockTab::Hierarchy,
                    DockTab::Validation,
                    DockTab::Balance,
                ]
                .iter()
                .all(|tab| saved.iter_all_tabs().any(|(_, t)| t == tab));
                if has_all {
                    saved
                } else {
                    default_dock_layout()
                }
            }
            None => default_dock_layout(),
        };

        let show_grid = config.show_grid.unwrap_or(true);
        let show_border = config.show_border.unwrap_or(true);
        let show_labels = config.show_labels.unwrap_or(true);
        let show_gateways = config.show_gateways.unwrap_or(false);
        let show_zone_lines = config.show_zone_lines.unwrap_or(false);
        let show_zone_fill = config.show_zone_fill.unwrap_or(false);
        let show_all_hitboxes = config.show_all_hitboxes.unwrap_or(false);
        let show_selection_hitboxes = config.show_selection_hitboxes.unwrap_or(false);
        let show_fps = config.show_fps.unwrap_or(false);
        let view_weather = config.view_weather.unwrap_or_default();
        let launched_graphics_backend = config.graphics_backend;
        let launched_present_mode = config.present_mode;
        let mut minimap = MinimapState::default();
        if let Some(vis) = config.minimap_visible {
            minimap.visible = vis;
        }
        let settings_install_dir_text = config
            .game_install_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let settings_wz_exe_text = config
            .wz_executable
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let settings_wz_config_dir_text = config
            .wz_config_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        let update_check_rx = config
            .check_for_updates_on_startup
            .then(crate::update_check::spawn_check);

        Self {
            document: None,
            tool_state,
            dock,
            output_log: {
                let mut l = output_log;
                l.push(LogEntry::new(
                    LogSeverity::Info,
                    LogSource::Editor,
                    "wzmapeditor initialized.".to_string(),
                ));
                l
            },
            config,
            tileset: None,
            wgpu_render_state,
            terrain_dirty: false,
            terrain_dirty_tiles: rustc_hash::FxHashSet::default(),
            lightmap_dirty: false,
            shadow_dirty: true,
            sun_change_cooldown: 0,
            water_dirty: false,
            hovered_tile: None,
            show_grid,
            show_border,
            show_labels,
            show_all_hitboxes,
            show_selection_hitboxes,
            show_gateways,
            new_map_dialog: NewMapDialog::default(),
            resize_map_dialog: ResizeMapDialog::default(),
            save_as_metadata_dialog: SaveAsMetadataDialog::default(),
            map_properties_dialog: MapPropertiesDialog::default(),
            publish_instructions_dialog: PublishInstructionsDialog::default(),
            generator_dialog: crate::generator::dialog::GeneratorDialog::default(),
            stats: None,
            model_loader: None,
            objects_dirty: false,
            current_tileset,
            selection: Selection::default(),
            rt: {
                let mut rt = RuntimeTasks::new();
                rt.extraction_progress = extraction_progress_arc;
                rt.extraction_rx = extraction_rx_channel;
                rt
            },
            map_browser: MapBrowserDialog::default(),
            render_settings,
            minimap,
            model_thumbnails: crate::thumbnails::ThumbnailCache::default(),
            save_path: cli_path,
            ctrl_s_pressed: false,
            map_players: 2,
            ground_data: None,
            focus_request: None,
            startup_phase,
            test_process: None,
            test_temp_files: Vec::new(),
            permission_error_dialog: PermissionErrorDialog::default(),
            load_error_dialog: LoadErrorDialog::default(),
            settings_install_dir_text,
            settings_wz_exe_text,
            settings_wz_config_dir_text,
            validation_results: None,
            validation_dirty: false,
            validation_cooldown: 0,
            view_weather,
            settings_open: false,
            settings_page: ui::settings_window::SettingsPage::default(),
            editor_icon: None,
            editor_icon_tried: false,
            launched_graphics_backend,
            launched_present_mode,
            keybinding_capture: None,
            autosave: crate::autosave::AutoSaveState::new(),
            recovery_entries: crate::autosave::scan_for_recovery(),
            designer: crate::designer::Designer::default(),
            custom_templates: crate::designer::CustomTemplateStore::default(),
            show_heatmap: false,
            heatmap_propulsion: wz_stats::terrain_table::PropulsionClass::default(),
            heatmap_dirty: false,
            show_fps,
            fps_samples: [0.0; 120],
            fps_idx: 0,
            fps_filled: false,
            gpu_info_label,
            viewshed: ViewshedSettings::default(),
            viewshed_dirty: false,
            balance: crate::balance::BalanceState {
                show_voronoi: show_zone_lines,
                show_voronoi_tint: show_zone_fill,
                ..crate::balance::BalanceState::default()
            },
            window_focused: true,
            last_paint_at: None,
            update_count: 0,
            has_hq_textures: false,
            update_check_rx,
            update_available: None,
        }
    }

    /// Drop the update-check receiver after its single message arrives,
    /// filtering out versions the user has already dismissed.
    fn poll_update_check(&mut self) {
        let Some(rx) = self.update_check_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(info) => {
                if self.config.dismissed_update_version.as_deref() != Some(info.latest.as_str()) {
                    log::info!("Editor update available: {}", info.latest);
                    self.update_available = Some(info);
                }
                self.update_check_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_check_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Push the latest frame delta into the rolling FPS window.
    fn record_frame_time(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.unstable_dt).max(0.000_1);
        self.fps_samples[self.fps_idx] = dt;
        self.fps_idx = (self.fps_idx + 1) % self.fps_samples.len();
        if self.fps_idx == 0 {
            self.fps_filled = true;
        }
    }

    /// Returns `(avg_dt, min_dt, max_dt)` in seconds across the window so
    /// far, or `None` if no samples have been recorded yet.
    pub fn fps_stats(&self) -> Option<(f32, f32, f32)> {
        let n = if self.fps_filled {
            self.fps_samples.len()
        } else {
            self.fps_idx
        };
        if n == 0 {
            return None;
        }
        let slice = &self.fps_samples[..n];
        let sum: f32 = slice.iter().sum();
        let avg = sum / n as f32;
        let min = slice.iter().copied().fold(f32::INFINITY, f32::min);
        let max = slice.iter().copied().fold(0.0_f32, f32::max);
        Some((avg, min, max))
    }

    /// Emit an Info entry to the Output panel and the disk log.
    pub fn log(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::info!("{msg}");
        self.output_log
            .push(LogEntry::new(LogSeverity::Info, LogSource::Editor, msg));
    }

    /// Emit a Warn entry to the Output panel and the disk log.
    pub fn log_warn(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::warn!("{msg}");
        self.output_log
            .push(LogEntry::new(LogSeverity::Warn, LogSource::Editor, msg));
    }

    /// Emit an Error entry to the Output panel and the disk log.
    pub fn log_error(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::error!("{msg}");
        self.output_log
            .push(LogEntry::new(LogSeverity::Error, LogSource::Editor, msg));
    }

    /// Report a failed user-initiated `.wz` load via the modal dialog and
    /// log the technical detail. The dialog text is keyed off the archive's
    /// classification, so e.g. script maps get a specific explanation.
    pub fn report_wz_load_error(&mut self, path: &std::path::Path, error: &wz_maplib::MapError) {
        let kind = wz_maplib::io_wz::classify_wz_archive(path);
        let raw = error.to_string();
        let (title, message) = dialogs::wz_load_error_copy(&kind, &raw);
        self.log_error(format!("Failed to load {}: {raw}", path.display()));
        self.load_error_dialog = LoadErrorDialog {
            open: true,
            title,
            message,
            details: raw,
        };
    }

    /// Run all validation checks on the current map. Returns `true` if problems were found.
    pub fn run_validation(&mut self) -> bool {
        actions::run_validation(self)
    }

    /// Delete all currently selected objects with undo support.
    pub fn delete_selected_objects(&mut self) {
        actions::delete_selected_objects(self);
    }

    /// Duplicate the current selection. While a drag-move is active, stamps
    /// copies at the current drag position; otherwise offsets by one tile.
    pub fn duplicate_selection(&mut self) {
        duplicate::duplicate_selection(self);
    }

    /// Whether a test game can be launched right now.
    pub fn can_test_map(&self) -> bool {
        testing::can_test_map(self)
    }

    /// Tooltip explaining why the test map button is disabled.
    pub fn test_map_tooltip(&self) -> &'static str {
        testing::test_map_tooltip(self)
    }

    /// Launch the current map in WZ2100 as a skirmish test game.
    pub fn test_map(&mut self) {
        testing::test_map(self);
    }

    /// Poll the test game process and clean up temp files when it exits.
    fn poll_test_process(&mut self) {
        testing::poll_test_process(self);
    }

    /// Load a new map document and mark terrain for re-upload.
    pub fn load_map(
        &mut self,
        map: wz_maplib::WzMap,
        source_path: Option<std::path::PathBuf>,
        save_path: Option<std::path::PathBuf>,
        archive_prefix: Option<String>,
    ) {
        map_io::load_map(self, map, source_path, save_path, archive_prefix);
    }

    /// Try to load tileset from the configured data directory.
    pub fn try_load_tileset(&mut self, ctx: &egui::Context) {
        tileset::try_load_tileset(self, ctx);
    }

    /// Save the current tile pools as custom groups in config.
    pub fn save_custom_tile_groups(&mut self) {
        tileset::save_custom_tile_groups(self);
    }

    /// Add a new empty tile group with a random color.
    pub fn add_new_tile_group(&mut self) {
        tileset::add_new_tile_group(self);
    }

    /// Delete the currently selected tile group.
    pub fn delete_selected_tile_group(&mut self) {
        tileset::delete_selected_tile_group(self);
    }

    /// Ensure tile pools match the current tileset, rebuilding if stale.
    pub fn ensure_tile_pools(&mut self) {
        tileset::ensure_tile_pools(self);
    }

    /// Load ground texture data in the background.
    pub fn start_ground_data_load(&mut self) {
        data_loading::start_ground_data_load(self);
    }

    /// Pre-decode and cache ground textures for all tilesets in the background.
    pub fn start_ground_precache(&mut self) {
        data_loading::start_ground_precache(self);
    }

    /// Set the resolved asset root, save config, and schedule tileset + stats reload.
    pub fn set_data_dir(&mut self, dir: std::path::PathBuf, ctx: &egui::Context) {
        data_loading::set_data_dir(self, dir, ctx);
    }

    /// Begin background extraction of `base.wz` into the persistent cache directory.
    pub fn start_base_wz_extraction(&mut self, wz_path: std::path::PathBuf, ctx: &egui::Context) {
        data_loading::start_base_wz_extraction(self, wz_path, ctx);
    }

    /// Try to load game stats from the configured data directory.
    pub fn try_load_stats(&mut self, ctx: &egui::Context) {
        data_loading::try_load_stats(self, ctx);
    }

    /// Save to the remembered path (quick save). Returns false if no path is set.
    pub fn save_to_current(&mut self) -> bool {
        map_io::save_to_current(self)
    }

    /// Return the current tileset as a lowercase string for `level.json`.
    fn current_tileset_name(&self) -> String {
        map_io::current_tileset_name(self)
    }

    /// Build a suggested `.wz` filename from the map name and player count.
    pub fn suggested_wz_filename(&self) -> String {
        map_io::suggested_wz_filename(self)
    }

    /// Save the current map as a `.wz` archive.
    pub fn save_to_wz(&mut self, path: &std::path::Path) {
        map_io::save_to_wz(self, path);
    }

    /// Save the current map to a directory on disk.
    pub fn save_to_directory(&mut self, path: &std::path::Path) {
        map_io::save_to_directory(self, path);
    }

    /// Poll background auto-save completion.
    fn poll_autosave(&mut self) {
        map_io::poll_autosave(self);
    }

    /// Check whether it's time to auto-save and kick one off if needed.
    fn tick_autosave(&mut self) {
        map_io::tick_autosave(self);
    }
}

impl eframe::App for EditorApp {
    // eframe 0.34 calls both `update` and `ui` each frame. Pre-render logic
    // (polling, dialogs, dirty checks) stays in `update`; panel rendering
    // moved here so we can call `Panel::show_inside(ui, ...)` instead of the
    // deprecated `Panel::show(ctx, ...)`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !matches!(self.startup_phase, StartupPhase::Ready) {
            crate::startup::splash_ui::show_launcher(ui, self);
            return;
        }
        show_ui_panels(ui, self);
    }

    fn on_exit(&mut self) {
        // Wait for the GPU to finish all in-flight work before any of our
        // textures get dropped. This avoids the
        // `egui_texid_Managed(N) label has been destroyed` panic that
        // wgpu raises when a queue submit references a texture that has
        // already been destroyed by an `egui` `TexturesDelta::Free` from
        // the same teardown.
        if let Some(rs) = &self.wgpu_render_state {
            let _ = rs.device.poll(wgpu::PollType::wait_indefinitely());
        }

        self.config.render_settings = Some(self.render_settings);
        self.config.show_grid = Some(self.show_grid);
        self.config.show_border = Some(self.show_border);
        self.config.show_labels = Some(self.show_labels);
        self.config.show_gateways = Some(self.show_gateways);
        self.config.show_zone_lines = Some(self.balance.show_voronoi);
        self.config.show_zone_fill = Some(self.balance.show_voronoi_tint);
        self.config.show_all_hitboxes = Some(self.show_all_hitboxes);
        self.config.show_selection_hitboxes = Some(self.show_selection_hitboxes);
        self.config.show_fps = Some(self.show_fps);
        self.config.view_weather = Some(self.view_weather);
        self.config.asset_grid_view = Some(self.tool_state.asset_grid_view);
        self.config.asset_thumb_size = Some(self.tool_state.asset_thumb_size);
        self.config.asset_show_campaign_only = Some(self.tool_state.asset_show_campaign_only);
        self.config.minimap_visible = Some(self.minimap.visible);

        self.config.dock_layout = Some(self.dock.clone());

        self.config.save();
        log::info!("Settings saved on exit");

        // Synchronous auto-save on exit if the document has unsaved changes.
        if self.config.autosave_enabled
            && let Some(ref doc) = self.document
            && doc.dirty
        {
            crate::autosave::save_sync(
                &doc.map,
                self.save_path.as_deref(),
                self.map_players,
                self.autosave.session_id(),
            );
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_count = self.update_count.saturating_add(1);
        // Reaching the second update means the first frame rendered, so
        // GPU init and egui's first shader compile succeeded.
        if self.update_count == 2 {
            crate::launch_sentinel::disarm();
        }
        self.poll_update_check();
        self.record_frame_time(ctx);
        // Snapshot OS-level window focus so animation-driven repaint
        // schedulers can skip when we're in the background. egui already
        // wakes update() on focus changes and on background-thread
        // request_repaint, so we don't need a fallback slow tick.
        self.window_focused = ctx.input(|i| i.focused);

        if !matches!(self.startup_phase, StartupPhase::Ready) {
            if matches!(self.startup_phase, StartupPhase::Loading { .. }) {
                crate::startup::workers::poll_startup_loads(ctx, self);
            }
            return;
        }

        // Drain any logger-sourced entries into the Output panel every frame
        // so internal-crate warnings/errors surface even when the tab is hidden.
        self.output_log.pump();

        poll_background_tasks(ctx, self);
        auto_load_assets(ctx, self);
        tick_validation_cooldown(self);
        show_dialogs(ctx, self);
        dispatch_keyboard_actions(ctx, self);
        handle_drag_and_drop(ctx, self);
        handle_deferred_save(self);
        enforce_fps_cap(self);
    }
}

/// Sleep at the end of `update()` so consecutive frames are at least
/// `1/fps_limit` seconds apart. Driven unconditionally when a cap is
/// set: skipping the sleep when nothing currently requests a repaint
/// lets steady-state callers like the viewport's water animation
/// (`request_repaint_after(16ms)`) and idle internal egui repaints slip
/// past the cap, which is exactly the "stop moving and fps spikes back
/// to 600" symptom. If the editor is truly idle, eframe doesn't call
/// `update()` and the sleep below never runs, so always-sleeping when
/// active is harmless.
fn enforce_fps_cap(app: &mut EditorApp) {
    let Some(fps) = app.config.fps_limit else {
        app.last_paint_at = None;
        return;
    };
    if fps == 0 {
        return;
    }
    let target_dt = std::time::Duration::from_secs_f32(1.0 / fps as f32);
    let now = std::time::Instant::now();
    if let Some(last) = app.last_paint_at
        && let Some(remaining) = target_dt.checked_sub(now.duration_since(last))
    {
        std::thread::sleep(remaining);
    }
    app.last_paint_at = Some(std::time::Instant::now());
}

/// Poll background extraction and loading progress bars.
fn poll_background_tasks(ctx: &egui::Context, app: &mut EditorApp) {
    crate::startup::loading_ui::show_extraction_progress(ctx, app);

    // Auto re-extract base.wz if data_dir is configured but the cache is
    // missing or stale (overlay marker version mismatch).
    if app.rt.extraction_progress.is_none()
        && let Some(ref data_dir) = app.config.data_dir
    {
        let files_missing = !data_dir.join("base").join("texpages").exists()
            && !data_dir.join("base").join("stats").exists();
        let marker_stale = !data_dir.join(".overlays_v9").exists()
            && (data_dir.join("base").join("texpages").exists()
                || data_dir.join("base").join("stats").exists());
        if (files_missing || marker_stale)
            && let Some(ref install_dir) = app.config.game_install_dir
        {
            let base_wz = install_dir.join("base.wz");
            if base_wz.exists() {
                if marker_stale {
                    log::info!("Overlay marker stale, re-extracting base.wz...");
                    let _ = std::fs::remove_dir_all(data_dir);
                    let ground_cache = crate::config::ground_cache_dir();
                    let _ = std::fs::remove_dir_all(&ground_cache);
                } else {
                    log::info!("Data cache missing, re-extracting base.wz...");
                }
                app.start_base_wz_extraction(base_wz, ctx);
            }
        }
    }
}

/// Auto-load tileset, stats, ground textures, and thumbnails once extraction finishes.
fn auto_load_assets(ctx: &egui::Context, app: &mut EditorApp) {
    let extracting = app.rt.extraction_progress.is_some();

    if !extracting && app.config.data_dir.is_some() {
        if app.tileset.is_none() && !app.rt.tileset_load_attempted {
            app.rt.tileset_load_attempted = true;
            app.try_load_tileset(ctx);
        }

        if app.config.data_dir.is_some() && !app.rt.ground_precache_attempted {
            app.rt.ground_precache_attempted = true;
            app.start_ground_precache();
        }

        if app.stats.is_none() && !app.rt.stats_load_attempted {
            app.try_load_stats(ctx);
        }

        if app.tileset.is_some()
            && app.ground_data.is_none()
            && app.rt.ground_texture_load.is_none()
            && app.rt.ground_precache_rx.is_none()
        {
            app.start_ground_data_load();
        }
    }

    crate::startup::loading_ui::show_ground_precache_progress(ctx, app);
    crate::startup::loading_ui::poll_ground_texture_load(ctx, app);
    app.ensure_tile_pools();

    if let Some(stats) = app.stats.as_ref()
        && matches!(
            app.model_thumbnails.preload,
            crate::thumbnails::PreloadState::Idle
        )
    {
        app.model_thumbnails
            .start_preload(ctx, stats, &mut app.model_loader);
    }

    crate::startup::loading_ui::show_thumbnail_preload(ctx, app, false);
    crate::startup::loading_ui::show_map_model_loading(ctx, app);
    crate::startup::loading_ui::show_loading_screen(ctx, app);
}

/// Auto-revalidation cooldown: debounce rapid edits before re-running validation.
fn tick_validation_cooldown(app: &mut EditorApp) {
    if app.terrain_dirty || app.objects_dirty {
        app.validation_dirty = true;
        app.validation_cooldown = 30; // ~0.5s at 60fps
    }
    if app.validation_dirty {
        if app.validation_cooldown > 0 {
            app.validation_cooldown -= 1;
        } else {
            app.validation_dirty = false;
            if app.validation_results.is_some() {
                app.run_validation();
            }
        }
    }
}

/// Show modal dialogs, poll test process, and handle auto-save.
fn show_dialogs(ctx: &egui::Context, app: &mut EditorApp) {
    if app.new_map_dialog.open {
        dialogs::show_new_map_dialog(ctx, app);
    }
    if app.resize_map_dialog.open {
        dialogs::show_resize_map_dialog(ctx, app);
    }
    if app.save_as_metadata_dialog.open {
        dialogs::show_save_as_metadata_dialog(ctx, app);
    }
    if app.map_properties_dialog.open {
        dialogs::show_map_properties_dialog(ctx, app);
    }
    if app.generator_dialog.open || app.generator_dialog.gen_rx.is_some() {
        crate::generator::dialog::show_generator_dialog(ctx, app);
    }
    if app.map_browser.open {
        ui::map_browser::show_map_browser(ctx, app);
    }
    if app.settings_open {
        ui::settings_window::show_settings_window(ctx, app);
    }
    if app.permission_error_dialog.open {
        dialogs::show_permission_error_dialog(ctx, app);
    }
    if app.load_error_dialog.open {
        dialogs::show_load_error_dialog(ctx, app);
    }
    if app.publish_instructions_dialog.open {
        dialogs::show_publish_instructions_dialog(ctx, app);
    }

    app.poll_test_process();

    if app.config.autosave_enabled && app.document.as_ref().is_some_and(|d| d.dirty) {
        let elapsed = app.autosave.elapsed_secs();
        let interval = app.config.autosave_interval_secs;
        let remaining = interval.saturating_sub(elapsed);
        ctx.request_repaint_after(std::time::Duration::from_secs(remaining.saturating_add(1)));
    }
    app.poll_autosave();
    app.tick_autosave();

    if !app.recovery_entries.is_empty() && matches!(app.startup_phase, StartupPhase::Ready) {
        dialogs::show_recovery_dialog(ctx, app);
    }
}

/// Centralized keyboard shortcut dispatch via the keymap.
fn dispatch_keyboard_actions(ctx: &egui::Context, app: &mut EditorApp) {
    use crate::keybindings::Action;

    let rmb_held = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Secondary));
    let fired = if app.keybinding_capture.is_some() {
        None
    } else {
        app.config.keymap.poll_action(ctx, rmb_held)
    };

    let Some(action) = fired else { return };

    match action {
        Action::Undo => {
            if let Some(ref mut doc) = app.document {
                doc.undo();
                app.terrain_dirty = true;
                app.terrain_dirty_tiles.clear();
                app.lightmap_dirty = true;
                app.objects_dirty = true;
                app.shadow_dirty = true;
                app.water_dirty = true;
                app.minimap.dirty = true;
                app.heatmap_dirty = true;
                app.selection.clear();
            }
        }
        Action::Redo => {
            if let Some(ref mut doc) = app.document {
                doc.redo();
                app.terrain_dirty = true;
                app.terrain_dirty_tiles.clear();
                app.lightmap_dirty = true;
                app.objects_dirty = true;
                app.shadow_dirty = true;
                app.water_dirty = true;
                app.minimap.dirty = true;
                app.heatmap_dirty = true;
                app.selection.clear();
            }
        }
        Action::Save => {
            app.ctrl_s_pressed = true;
        }
        Action::TestMap => {
            if app.can_test_map() {
                app.test_map();
            }
        }
        Action::DeleteSelected => {
            app.delete_selected_objects();
        }
        Action::Duplicate => {
            app.duplicate_selection();
        }
        Action::RotatePlacement => {
            // Prefer rotating the selection; fall back to nudging the
            // placement ghost when nothing is selected.
            if !actions::rotate_selected_objects(app)
                && app.tool_state.active_tool == crate::tools::ToolId::ObjectPlace
                && let Some(place) = app.tool_state.object_place_mut()
            {
                // 0x4000 = 90 degrees in WZ2100 direction units.
                place.placement_direction = place.placement_direction.wrapping_add(0x4000);
                app.objects_dirty = true;
            }
        }
        Action::EscapeTool => {
            if app.tool_state.active_tool == crate::tools::ToolId::VertexSculpt {
                if let Some(tool) = app.tool_state.vertex_sculpt_mut() {
                    tool.clear();
                }
            } else if matches!(
                app.tool_state.active_tool,
                crate::tools::ToolId::ScriptLabel | crate::tools::ToolId::Gateway
            ) {
                app.tool_state.active_tool = crate::tools::ToolId::ObjectSelect;
            }
        }
        Action::ToggleHeatmap => {
            app.show_heatmap = !app.show_heatmap;
            if app.show_heatmap {
                app.heatmap_dirty = true;
            }
        }
        _ => {
            if let Some(tool) = action.as_tool() {
                app.tool_state.active_tool = tool;
            }
            if let Some(mode) = action.as_height_mode()
                && let Some(brush) = app.tool_state.height_brush_mut()
            {
                brush.mode = mode;
            }
        }
    }
}

/// Drag-and-drop: show overlay while files hover, open .wz on drop.
fn handle_drag_and_drop(ctx: &egui::Context, app: &mut EditorApp) {
    let has_hovered_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
    let dropped_path = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .filter_map(|f| f.path.as_ref())
            .find(|p| {
                p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("wz"))
            })
            .cloned()
    });

    if has_hovered_files {
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_overlay"),
        ));
        painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
        painter.text(
            screen.center(),
            egui::Align2::CENTER_CENTER,
            "Drop .wz file to open",
            egui::FontId::proportional(28.0),
            egui::Color32::WHITE,
        );
    }

    if let Some(path) = dropped_path {
        match wz_maplib::io_wz::load_from_wz_archive(&path) {
            Ok(map) => {
                let save = Some(path.clone());
                app.load_map(map, Some(path), save, None);
            }
            Err(e) => app.report_wz_load_error(&path, &e),
        }
    }
}

/// Handle deferred Ctrl+S save.
fn handle_deferred_save(app: &mut EditorApp) {
    if app.ctrl_s_pressed {
        app.ctrl_s_pressed = false;
        if app.document.is_some() {
            ui::actions::save_current_or_prompt(app);
        }
    }
}

/// Render menu bar, toolbar, dock area, and designer modal.
fn show_ui_panels(ui: &mut egui::Ui, app: &mut EditorApp) {
    egui::Panel::top("menu_bar").show_inside(ui, |ui| {
        ui::main_menu::show_menu_bar(ui, app);
    });

    egui::Panel::top("toolbar").show_inside(ui, |ui| {
        ui::toolbar::show_toolbar(ui, app);
    });

    let mut dock = std::mem::replace(&mut app.dock, DockState::new(vec![DockTab::Viewport]));
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| {
            let mut tab_viewer = dock_viewer::DockTabViewer { app };
            let mut dock_style = Style::from_egui(ui.style().as_ref());
            dock_style.dock_area_padding = Some(egui::Margin::same(0));
            dock_style.main_surface_border_stroke = egui::Stroke::NONE;
            dock_style.tab.tab_body.stroke = egui::Stroke::NONE;
            dock_style.tab.tab_body.inner_margin = egui::Margin {
                left: 8,
                right: 4,
                top: 4,
                bottom: 4,
            };
            DockArea::new(&mut dock)
                .style(dock_style)
                .show_close_buttons(false)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .show_inside(ui, &mut tab_viewer);
        });
    app.dock = dock;

    let ctx = ui.ctx().clone();
    designer::update_designer(app, &ctx);
}

#[cfg(test)]
mod tests;
