//! Map browser dialog with minimap thumbnail previews.

use std::path::{Path, PathBuf};

use egui::{Color32, ColorImage, TextureHandle, TextureOptions, Vec2};

use crate::app::EditorApp;

const THUMB_SIZE: usize = 128;

/// Category grouping for the map browser tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapCategory {
    Multiplayer,
    MyMaps,
    Community,
    Campaign,
}

impl std::fmt::Display for MapCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapCategory::Multiplayer => write!(f, "Multiplayer"),
            MapCategory::MyMaps => write!(f, "My Maps"),
            MapCategory::Community => write!(f, "Community"),
            MapCategory::Campaign => write!(f, "Campaign"),
        }
    }
}

/// State for the map browser dialog.
pub struct MapBrowserDialog {
    pub open: bool,
    pub maps: Vec<(MapCategory, Vec<MapEntry>)>,
    pub active_tab: MapCategory,
    pub filter: String,
    pub scanned_dirs: Vec<PathBuf>,
    pub online: super::online_maps::OnlineMapsState,
}

pub struct MapEntry {
    pub preview: wz_maplib::MapPreview,
    pub thumbnail: Option<TextureHandle>,
    /// Campaign group key (e.g. "cam1", "cam2", "cam3", "tutorial").
    pub campaign_group: Option<String>,
    /// Level name from `gamedesc.lev`. When set, load via
    /// `load_campaign_level_by_name` rather than the raw prefix loader.
    pub campaign_level_name: Option<String>,
    /// For `expand` overlays, the base mission whose terrain it reuses.
    pub overlay_of: Option<String>,
}

impl Default for MapBrowserDialog {
    fn default() -> Self {
        Self {
            open: false,
            maps: Vec::new(),
            active_tab: MapCategory::Multiplayer,
            filter: String::new(),
            scanned_dirs: Vec::new(),
            online: super::online_maps::OnlineMapsState::default(),
        }
    }
}

impl MapBrowserDialog {
    /// Scan `mp.wz`, the loose `mp/` directory, and `base.wz` (campaign).
    pub fn scan_game_dirs(
        &mut self,
        game_install_dir: &Path,
        _data_dir: Option<&Path>,
        ctx: &egui::Context,
    ) {
        self.maps.clear();
        self.scanned_dirs.clear();

        let mut mp_entries: Vec<MapEntry> = Vec::new();

        // Installed games bundle MP maps in mp.wz.
        let mp_wz = game_install_dir.join("mp.wz");
        if mp_wz.exists() {
            log::info!("Scanning mp.wz for multiplayer maps: {}", mp_wz.display());
            let previews = wz_maplib::io_wz::scan_wz_archive_maps(&mp_wz);
            mp_entries.extend(previews.into_iter().map(|p| make_plain_entry(ctx, p)));
            self.scanned_dirs.push(mp_wz);
        }

        // User-added loose .wz files live in mp/.
        let mp_dir = game_install_dir.join("mp");
        if mp_dir.exists() {
            log::info!("Scanning mp/ for loose map archives: {}", mp_dir.display());
            let previews = wz_maplib::io_wz::scan_map_directory(&mp_dir);
            mp_entries.extend(previews.into_iter().map(|p| make_plain_entry(ctx, p)));
            self.scanned_dirs.push(mp_dir);
        }

        if !mp_entries.is_empty() {
            log::info!("Found {} multiplayer maps", mp_entries.len());
            self.maps.push((MapCategory::Multiplayer, mp_entries));
        }

        let mut user_entries: Vec<MapEntry> = Vec::new();
        for user_maps_dir in wz2100_user_map_dirs() {
            if user_maps_dir.exists() {
                log::info!("Scanning user maps directory: {}", user_maps_dir.display());
                let previews = wz_maplib::io_wz::scan_map_directory(&user_maps_dir);
                user_entries.extend(previews.into_iter().map(|p| make_plain_entry(ctx, p)));
                self.scanned_dirs.push(user_maps_dir);
            }
        }
        if !user_entries.is_empty() {
            log::info!("Found {} user maps", user_entries.len());
            self.maps.push((MapCategory::MyMaps, user_entries));
        }

        // Campaign maps come from `gamedesc.lev`, which lists every playable
        // level including `expand` overlays that reuse the prior mission's
        // terrain. Falls back silently when the manifest is missing.
        let base_wz = game_install_dir.join("base.wz");
        if base_wz.exists() {
            log::info!("Scanning base.wz for campaign maps: {}", base_wz.display());
            let campaign_entries = scan_campaign_entries(ctx, &base_wz);
            if !campaign_entries.is_empty() {
                log::info!("Found {} campaign maps in base.wz", campaign_entries.len());
                self.maps.push((MapCategory::Campaign, campaign_entries));
                self.scanned_dirs.push(base_wz);
            }
        }
    }

    /// Scan an arbitrary directory and append the results to the multiplayer tab.
    pub fn scan_custom_dir(&mut self, dir: &Path, ctx: &egui::Context) {
        log::info!("Scanning custom map directory: {}", dir.display());
        let previews = wz_maplib::io_wz::scan_map_directory(dir);
        let entries: Vec<MapEntry> = previews
            .into_iter()
            .map(|p| make_plain_entry(ctx, p))
            .collect();
        log::info!("Found {} maps in custom directory", entries.len());
        let mp = self
            .maps
            .iter_mut()
            .find(|(cat, _)| *cat == MapCategory::Multiplayer)
            .map(|(_, v)| v);
        if let Some(mp) = mp {
            mp.extend(entries);
        } else {
            self.maps.push((MapCategory::Multiplayer, entries));
        }
        self.scanned_dirs.push(dir.to_path_buf());
    }
}

fn rescan_user_maps(browser: &mut MapBrowserDialog, ctx: &egui::Context) {
    let mut user_entries: Vec<MapEntry> = Vec::new();
    for user_maps_dir in wz2100_user_map_dirs() {
        if user_maps_dir.exists() {
            let previews = wz_maplib::io_wz::scan_map_directory(&user_maps_dir);
            user_entries.extend(previews.into_iter().map(|p| make_plain_entry(ctx, p)));
        }
    }
    if let Some(pos) = browser
        .maps
        .iter()
        .position(|(c, _)| *c == MapCategory::MyMaps)
    {
        browser.maps[pos].1 = user_entries;
    } else if !user_entries.is_empty() {
        browser.maps.push((MapCategory::MyMaps, user_entries));
    }
}

/// Locate the WZ2100 user data `maps/` directories.
///
/// - Windows: `%APPDATA%\Warzone 2100 Project\Warzone 2100\maps`
/// - Linux: `~/.local/share/Warzone 2100/maps`
/// - macOS: `~/Library/Application Support/Warzone 2100/maps`
pub fn wz2100_user_map_dirs() -> Vec<PathBuf> {
    // The web build has no local user data directory; only the native
    // per-OS branches below ever populate `dirs`, so gate the `mut`.
    #[cfg(not(target_arch = "wasm32"))]
    let mut dirs = Vec::new();
    #[cfg(target_arch = "wasm32")]
    let dirs = Vec::new();

    #[cfg(target_os = "windows")]
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(
            PathBuf::from(appdata)
                .join("Warzone 2100 Project")
                .join("Warzone 2100")
                .join("maps"),
        );
    }

    #[cfg(target_os = "linux")]
    if let Ok(home) = std::env::var("HOME") {
        let data_dir =
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{home}/.local/share"));
        dirs.push(PathBuf::from(data_dir).join("Warzone 2100").join("maps"));
    }

    #[cfg(target_os = "macos")]
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Warzone 2100")
                .join("maps"),
        );
    }

    dirs
}

fn campaign_group_from_dataset(dataset: &str) -> String {
    match dataset {
        "CAM_1" => "Alpha Campaign",
        "CAM_2" => "Beta Campaign",
        "CAM_3" => "Gamma Campaign",
        "CAM_TUT3" => "Tutorial",
        _ if dataset.starts_with("MULTI_") => "Multiplayer",
        _ => "Other",
    }
    .to_string()
}

fn make_plain_entry(ctx: &egui::Context, preview: wz_maplib::MapPreview) -> MapEntry {
    let thumbnail = generate_thumbnail(ctx, &preview);
    MapEntry {
        preview,
        thumbnail: Some(thumbnail),
        campaign_group: None,
        campaign_level_name: None,
        overlay_of: None,
    }
}

/// Walk `gamedesc.lev` and emit one `MapEntry` per playable level. Overlay
/// entries reuse the base mission's terrain preview for their thumbnail.
fn scan_campaign_entries(ctx: &egui::Context, base_wz: &Path) -> Vec<MapEntry> {
    use std::collections::HashMap;

    let index = match wz_maplib::io_wz::read_campaign_index(base_wz) {
        Ok(i) => i,
        Err(e) => {
            log::warn!("Cannot read gamedesc.lev from {}: {}", base_wz.display(), e);
            return Vec::new();
        }
    };

    // One preview per folder that has a game.map; reused as the terrain
    // source for both full missions and their overlays.
    let previews_by_prefix: HashMap<String, wz_maplib::MapPreview> =
        wz_maplib::io_wz::scan_wz_archive_maps(base_wz)
            .into_iter()
            .map(|p| (p.archive_prefix.clone(), p))
            .collect();

    let mut entries = Vec::new();
    for level in &index.levels {
        let terrain_prefix = match level.kind {
            wz_maplib::io_lev::LevelKind::Expand => level
                .base_folder
                .as_deref()
                .unwrap_or(level.folder.as_str()),
            _ => level.folder.as_str(),
        };

        let Some(base_preview) = previews_by_prefix.get(terrain_prefix) else {
            log::warn!(
                "Campaign level {}: terrain source {} not found in {}",
                level.name,
                terrain_prefix,
                base_wz.display()
            );
            continue;
        };

        let mut preview = base_preview.clone();
        preview.name.clone_from(&level.name);

        let overlay_of = if matches!(level.kind, wz_maplib::io_lev::LevelKind::Expand) {
            index
                .levels
                .iter()
                .find(|l| Some(l.folder.as_str()) == level.base_folder.as_deref())
                .map(|l| l.name.clone())
        } else {
            None
        };

        let thumbnail = generate_thumbnail(ctx, &preview);
        entries.push(MapEntry {
            preview,
            thumbnail: Some(thumbnail),
            campaign_group: Some(campaign_group_from_dataset(&level.dataset)),
            campaign_level_name: Some(level.name.clone()),
            overlay_of,
        });
    }

    entries
}

/// Per-tileset minimap color pairs interpolated by tile height.
struct TilesetColorScheme {
    cliff_low: [u8; 3],
    cliff_high: [u8; 3],
    water: [u8; 3],
    road_low: [u8; 3],
    road_high: [u8; 3],
    ground_low: [u8; 3],
    ground_high: [u8; 3],
}

const ARIZONA_COLORS: TilesetColorScheme = TilesetColorScheme {
    cliff_low: [0x68, 0x3C, 0x24],
    cliff_high: [0xE8, 0x84, 0x5C],
    water: [0x3F, 0x68, 0x9A],
    road_low: [0x24, 0x1F, 0x16],
    road_high: [0xB2, 0x9A, 0x66],
    ground_low: [0x24, 0x1F, 0x16],
    ground_high: [0xCC, 0xB2, 0x80],
};

const URBAN_COLORS: TilesetColorScheme = TilesetColorScheme {
    cliff_low: [0x3C, 0x3C, 0x3C],
    cliff_high: [0x84, 0x84, 0x84],
    water: [0x3F, 0x68, 0x9A],
    road_low: [0x00, 0x00, 0x00],
    road_high: [0x24, 0x1F, 0x16],
    ground_low: [0x1F, 0x1F, 0x1F],
    ground_high: [0xB2, 0xB2, 0xB2],
};

const ROCKIES_COLORS: TilesetColorScheme = TilesetColorScheme {
    cliff_low: [0x3C, 0x3C, 0x3C],
    cliff_high: [0xFF, 0xFF, 0xFF],
    water: [0x3F, 0x68, 0x9A],
    road_low: [0x24, 0x1F, 0x16],
    road_high: [0x3D, 0x21, 0x0A],
    ground_low: [0x00, 0x1C, 0x0E],
    ground_high: [0xFF, 0xFF, 0xFF],
};

const TER_WATER: u16 = 7;
const TER_CLIFFFACE: u16 = 8;
const TER_ROAD: u16 = 6;

fn color_scheme_for_preview(preview: &wz_maplib::MapPreview) -> &'static TilesetColorScheme {
    use crate::config::Tileset;
    let tileset = Tileset::from_terrain_types(&preview.terrain_types);
    match tileset {
        Tileset::Arizona => &ARIZONA_COLORS,
        Tileset::Urban => &URBAN_COLORS,
        Tileset::Rockies => &ROCKIES_COLORS,
    }
}

/// Minimap pixel color, matching the engine's `generate2DMapPreview`.
fn terrain_type_color(scheme: &TilesetColorScheme, terrain_type: u16, height: u16) -> Color32 {
    // Old maps store heights up to 510; squash to 0-255.
    let col = (height / 2).min(255) as f32;
    let t = col / 256.0;

    let (low, high) = match terrain_type {
        TER_CLIFFFACE => (scheme.cliff_low, scheme.cliff_high),
        TER_WATER => return Color32::from_rgb(scheme.water[0], scheme.water[1], scheme.water[2]),
        TER_ROAD => (scheme.road_low, scheme.road_high),
        _ => (scheme.ground_low, scheme.ground_high),
    };

    Color32::from_rgb(
        (low[0] as f32 + (high[0] as f32 - low[0] as f32) * t) as u8,
        (low[1] as f32 + (high[1] as f32 - low[1] as f32) * t) as u8,
        (low[2] as f32 + (high[2] as f32 - low[2] as f32) * t) as u8,
    )
}

fn generate_thumbnail(ctx: &egui::Context, preview: &wz_maplib::MapPreview) -> TextureHandle {
    let w = preview.width as usize;
    let h = preview.height as usize;

    let scheme = color_scheme_for_preview(preview);
    let has_terrain_types = !preview.terrain_types.is_empty();

    let pixels: Vec<Color32> = preview
        .textures
        .iter()
        .zip(preview.heights.iter())
        .map(|(&tex_id, &height)| {
            let terrain_type = if has_terrain_types {
                let idx = tex_id as usize;
                if idx < preview.terrain_types.len() {
                    preview.terrain_types[idx]
                } else {
                    0
                }
            } else {
                0
            };
            terrain_type_color(scheme, terrain_type, height)
        })
        .collect();

    let mut thumb_pixels = Vec::with_capacity(THUMB_SIZE * THUMB_SIZE);
    for ty in 0..THUMB_SIZE {
        let sy = (ty * h) / THUMB_SIZE;
        for tx in 0..THUMB_SIZE {
            let sx = (tx * w) / THUMB_SIZE;
            thumb_pixels.push(pixels[sy * w + sx]);
        }
    }

    let image = ColorImage::new([THUMB_SIZE, THUMB_SIZE], thumb_pixels);

    ctx.load_texture(
        format!("minimap_{}", preview.name),
        image,
        TextureOptions::NEAREST,
    )
}

pub fn show_map_browser(ctx: &egui::Context, app: &mut EditorApp) {
    let mut open = app.map_browser.open;
    let mut load_path: Option<PathBuf> = None;
    let mut load_prefix: Option<String> = None;
    let mut load_level_name: Option<String> = None;

    egui::Window::new("Map Browser")
        .collapsible(true)
        .resizable(true)
        .open(&mut open)
        .default_size([820.0, 600.0])
        .max_size([1200.0, 800.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut app.map_browser.filter);

                if ui.button("Browse Folder...").clicked() {
                    #[cfg(not(target_arch = "wasm32"))]
                    let picked_dir: Option<PathBuf> = rfd::FileDialog::new()
                        .set_title("Select Map Directory")
                        .pick_folder();
                    #[cfg(target_arch = "wasm32")]
                    let picked_dir: Option<PathBuf> = {
                        log::warn!("Browse Folder is not available in the web build");
                        None
                    };

                    if let Some(dir) = picked_dir {
                        app.map_browser.scan_custom_dir(&dir, ctx);
                    }
                }

                if ui.button("Rescan").clicked() {
                    let game_dir = app.config.game_install_dir.clone();
                    let data_dir = app.config.data_dir.clone();
                    if let Some(ref gd) = game_dir {
                        app.map_browser.scan_game_dirs(gd, data_dir.as_deref(), ctx);
                    }
                }
            });

            ui.separator();

            ui.horizontal(|ui| {
                for cat in &[
                    MapCategory::Multiplayer,
                    MapCategory::MyMaps,
                    MapCategory::Community,
                    MapCategory::Campaign,
                ] {
                    let count = if *cat == MapCategory::Community {
                        app.map_browser.online.maps.len()
                    } else {
                        app.map_browser
                            .maps
                            .iter()
                            .find(|(c, _)| c == cat)
                            .map_or(0, |(_, v)| v.len())
                    };
                    let label = if count > 0 {
                        format!("{cat} ({count})")
                    } else {
                        format!("{cat}")
                    };
                    if ui
                        .selectable_label(app.map_browser.active_tab == *cat, label)
                        .clicked()
                    {
                        app.map_browser.active_tab = *cat;
                    }
                }
            });

            ui.separator();

            app.map_browser.online.poll(ctx);
            if app.map_browser.online.maps_changed {
                app.map_browser.online.maps_changed = false;
                rescan_user_maps(&mut app.map_browser, ctx);
            }

            let active_tab = app.map_browser.active_tab;

            if active_tab == MapCategory::Community {
                let filter = app.map_browser.filter.clone();
                let online_load = super::online_maps::show_online_tab(
                    ui,
                    &mut app.map_browser.online,
                    ctx,
                    &filter,
                );
                if let Some(path) = online_load {
                    load_path = Some(path);
                }
            } else {
                let filter_lower = app.map_browser.filter.to_lowercase();
                let entries_opt = app
                    .map_browser
                    .maps
                    .iter()
                    .find(|(c, _)| *c == active_tab)
                    .map(|(_, v)| v);
                if let Some(entries) = entries_opt {
                    let filtered: Vec<&MapEntry> = entries
                        .iter()
                        .filter(|e| {
                            filter_lower.is_empty()
                                || e.preview.name.to_lowercase().contains(&filter_lower)
                        })
                        .collect();

                    if filtered.is_empty() {
                        ui.centered_and_justified(|ui| {
                            if entries.is_empty() {
                                ui.label(
                                    "No maps found. Use 'Browse Folder...' to add a map directory.",
                                );
                            } else {
                                ui.label("No maps match the filter.");
                            }
                        });
                    } else {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if active_tab == MapCategory::Campaign {
                                    render_campaign_grouped(
                                        ui,
                                        &filtered,
                                        &mut load_path,
                                        &mut load_prefix,
                                        &mut load_level_name,
                                    );
                                } else {
                                    render_mp_grouped(
                                        ui,
                                        &filtered,
                                        &mut load_path,
                                        &mut load_prefix,
                                        &mut load_level_name,
                                    );
                                }
                            });
                    }
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label("No maps found. Use 'Browse Folder...' to add a map directory.");
                    });
                }
            }
        });

    app.map_browser.open = open;

    if let Some(path) = load_path {
        let prefix = load_prefix.unwrap_or_default();
        let result = if let Some(ref name) = load_level_name {
            wz_maplib::io_wz::load_campaign_level_by_name(&path, name)
        } else if prefix.is_empty() {
            wz_maplib::io_wz::load_from_wz_archive(&path)
        } else {
            wz_maplib::io_wz::load_map_from_archive_prefix(&path, &prefix)
        };

        match result {
            Ok(map) => {
                // Campaign levels re-derive their source from `gamedesc.lev`
                // on reload, so there's no stable prefix to keep.
                let prefix_opt = if load_level_name.is_some() || prefix.is_empty() {
                    None
                } else {
                    Some(prefix.clone())
                };
                app.load_map(map, Some(path), None, prefix_opt);
                app.map_browser.open = false;
            }
            Err(e) => {
                app.log_error(format!("Failed to load map: {e}"));
            }
        }
    }
}

fn render_mp_grouped(
    ui: &mut egui::Ui,
    filtered: &[&MapEntry],
    load_path: &mut Option<PathBuf>,
    load_prefix: &mut Option<String>,
    load_level_name: &mut Option<String>,
) {
    let mut current_players: Option<u8> = None;
    let mut group: Vec<&MapEntry> = Vec::new();

    for entry in filtered {
        let players = entry.preview.players;
        if current_players != Some(players) {
            if !group.is_empty() {
                render_map_grid(ui, &group, load_path, load_prefix, load_level_name);
            }
            group.clear();
            current_players = Some(players);
            ui.add_space(8.0);
            if players > 0 {
                ui.heading(format!("{players} Players"));
            } else {
                ui.heading("Unknown Player Count");
            }
            ui.separator();
        }
        group.push(entry);
    }
    if !group.is_empty() {
        render_map_grid(ui, &group, load_path, load_prefix, load_level_name);
    }
}

fn render_campaign_grouped(
    ui: &mut egui::Ui,
    filtered: &[&MapEntry],
    load_path: &mut Option<PathBuf>,
    load_prefix: &mut Option<String>,
    load_level_name: &mut Option<String>,
) {
    let group_order = [
        "Alpha Campaign",
        "Beta Campaign",
        "Gamma Campaign",
        "Tutorial",
        "Multiplayer",
        "Other",
    ];

    for group_name in &group_order {
        let group: Vec<&MapEntry> = filtered
            .iter()
            .filter(|e| e.campaign_group.as_deref() == Some(*group_name))
            .copied()
            .collect();

        if group.is_empty() {
            continue;
        }

        ui.add_space(8.0);
        ui.heading(*group_name);
        ui.separator();
        render_map_grid(ui, &group, load_path, load_prefix, load_level_name);
    }
}

fn render_map_grid(
    ui: &mut egui::Ui,
    group: &[&MapEntry],
    load_path: &mut Option<PathBuf>,
    load_prefix: &mut Option<String>,
    load_level_name: &mut Option<String>,
) {
    let card_w = THUMB_SIZE as f32 + 16.0;
    let cols = ((ui.available_width() / card_w).floor() as usize).max(1);

    egui::Grid::new(ui.next_auto_id())
        .num_columns(cols)
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            for (i, entry) in group.iter().enumerate() {
                if i > 0 && i % cols == 0 {
                    ui.end_row();
                }
                if show_map_card(ui, entry) {
                    *load_path = Some(entry.preview.path.clone());
                    *load_prefix = Some(entry.preview.archive_prefix.clone());
                    load_level_name.clone_from(&entry.campaign_level_name);
                }
            }
        });
}

/// Render a single map card. Returns true when clicked.
fn show_map_card(ui: &mut egui::Ui, entry: &MapEntry) -> bool {
    let thumb_size = Vec2::new(THUMB_SIZE as f32, THUMB_SIZE as f32);
    let mut clicked = false;

    ui.vertical(|ui| {
        ui.set_width(THUMB_SIZE as f32 + 8.0);

        let response = if let Some(ref tex) = entry.thumbnail {
            ui.add(
                egui::Image::new(tex)
                    .fit_to_exact_size(thumb_size)
                    .sense(egui::Sense::click()),
            )
        } else {
            let (rect, response) = ui.allocate_exact_size(thumb_size, egui::Sense::click());
            ui.painter().rect_filled(rect, 4.0, Color32::from_gray(60));
            response
        };

        if response.clicked() {
            clicked = true;
        }

        if response.hovered() {
            ui.painter().rect_stroke(
                response.rect,
                4.0,
                egui::Stroke::new(2.0_f32, Color32::from_rgb(100, 180, 255)),
                egui::StrokeKind::Inside,
            );
        }

        // Count chars to avoid slicing mid-UTF-8 on community map names.
        let display_name = if entry.preview.name.chars().count() > 20 {
            let truncated: String = entry.preview.name.chars().take(17).collect();
            format!("{truncated}...")
        } else {
            entry.preview.name.clone()
        };
        ui.label(egui::RichText::new(&display_name).small().strong());

        if let Some(ref base) = entry.overlay_of {
            ui.label(
                egui::RichText::new(format!("overlay of {base}"))
                    .small()
                    .italics()
                    .weak(),
            );
        } else {
            ui.label(
                egui::RichText::new(format!("{}x{}", entry.preview.width, entry.preview.height))
                    .small()
                    .weak(),
            );
        }

        let overlay_line = entry
            .overlay_of
            .as_ref()
            .map(|b| format!("\noverlay of {b}"))
            .unwrap_or_default();
        response.on_hover_text(format!(
            "{}\n{}x{} tiles{}{}",
            entry.preview.name,
            entry.preview.width,
            entry.preview.height,
            if entry.preview.players > 0 {
                format!("\n{} players", entry.preview.players)
            } else {
                String::new()
            },
            overlay_line,
        ));
    });

    clicked
}
