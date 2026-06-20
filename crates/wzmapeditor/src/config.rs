//! Persistent editor configuration.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// User-selected graphics backend. Changing requires an app restart.
/// See `available_for_platform` for the per-OS choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphicsBackend {
    Vulkan,
    Dx12,
    Metal,
    OpenGl,
}

impl Default for GraphicsBackend {
    fn default() -> Self {
        #[cfg(target_os = "windows")]
        return Self::Vulkan;
        #[cfg(target_os = "macos")]
        return Self::Metal;
        #[cfg(all(unix, not(target_os = "macos")))]
        return Self::Vulkan;
        // In-browser wgpu auto-selects WebGPU or the WebGL2 fallback; this
        // value is only a placeholder for the (unused) backend config on web.
        #[cfg(target_arch = "wasm32")]
        return Self::OpenGl;
    }
}

impl GraphicsBackend {
    /// Human-readable label for UI surfaces.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dx12 => "Direct3D 12",
            Self::Vulkan => "Vulkan",
            Self::Metal => "Metal",
            Self::OpenGl => "OpenGL",
        }
    }

    /// Backends supported on this build target, in preferred order.
    /// The first entry is the platform default.
    pub fn available_for_platform() -> &'static [Self] {
        #[cfg(target_os = "windows")]
        {
            &[Self::Vulkan, Self::Dx12, Self::OpenGl]
        }
        #[cfg(target_os = "macos")]
        {
            // wgpu's GL backend has no surface impl for macOS NSViews.
            &[Self::Metal]
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            &[Self::Vulkan, Self::OpenGl]
        }
        #[cfg(target_arch = "wasm32")]
        {
            &[Self::OpenGl]
        }
    }
}

/// User-selected UI theme. `System` follows the OS dark/light preference
/// via egui's built-in detection; `Light` and `Dark` force a fixed theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    /// Human-readable label for UI surfaces.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// All variants in display order.
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];
}

impl From<ThemePreference> for egui::ThemePreference {
    fn from(value: ThemePreference) -> Self {
        match value {
            ThemePreference::System => Self::System,
            ThemePreference::Light => Self::Light,
            ThemePreference::Dark => Self::Dark,
        }
    }
}

/// Swapchain present mode preference.
///
/// `SmartVsync` is the user-facing "Vsync ON" setting and resolves to
/// `AutoVsync` (Fifo / `FifoRelaxed`) on every platform. Fifo blocks the
/// producer at vblank so the rendered frame rate matches the monitor
/// refresh rate; combined with `desired_maximum_frame_latency: Some(1)`
/// the queue depth stays at one frame, keeping input latency low. The
/// remaining variants are explicit overrides (e.g. `Mailbox` for
/// vsynced-but-uncapped rendering, or `Immediate` for tearing).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresentMode {
    /// Smart vsynced mode: resolves to `AutoVsync` (Fifo / `FifoRelaxed`) on every backend.
    #[default]
    SmartVsync,
    /// Pick a vsynced mode the platform supports (Fifo on Vulkan, flip-model on DX12).
    AutoVsync,
    /// Pick a non-vsynced mode the platform supports. Lower input latency at
    /// the cost of possible tearing.
    AutoNoVsync,
    /// Block until vblank, queue is bounded. Always supported.
    Fifo,
    /// Replace queued frame on each present; vsync without blocking. Lower
    /// latency than Fifo but may not be supported on every adapter.
    Mailbox,
    /// Present immediately, no vsync. Tears, but minimum latency.
    Immediate,
}

impl PresentMode {
    /// Human-readable label for UI surfaces.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn label(self) -> &'static str {
        match self {
            Self::SmartVsync => "Auto (recommended)",
            Self::AutoVsync => "Auto Vsync",
            Self::AutoNoVsync => "Auto (no vsync)",
            Self::Fifo => "Fifo (vsync, blocks)",
            Self::Mailbox => "Mailbox (vsync, low latency)",
            Self::Immediate => "Immediate (no vsync, tears)",
        }
    }

    /// True if the mode is intended to vsync (used to derive the simple
    /// "Vsync" checkbox state from the underlying enum).
    pub fn is_vsynced(self) -> bool {
        matches!(
            self,
            Self::SmartVsync | Self::AutoVsync | Self::Fifo | Self::Mailbox
        )
    }
}

/// SPDX license expressions offered by the Save As dialog. `CC0-1.0` is
/// first because it is the default for new maps.
pub const LICENSE_OPTIONS: &[&str] = &[
    "CC0-1.0",
    "GPL-2.0-or-later",
    "CC-BY-3.0 OR GPL-2.0-or-later",
    "CC-BY-SA-3.0 OR GPL-2.0-or-later",
];

/// Default SPDX license written when a map has no prior license set.
pub const DEFAULT_LICENSE: &str = "CC0-1.0";

/// Persistent configuration saved to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorConfig {
    /// The WZ2100 data directory chosen by the user. For installed game
    /// copies this is the `data/` folder that contains `base.wz`. For
    /// source checkouts it is the `data/` folder with an extracted `base/`
    /// subtree.
    pub game_install_dir: Option<PathBuf>,
    /// Resolved asset root used by all loaders, has `base/texpages` etc.
    /// Equal to `game_install_dir` for source checkouts; points to the
    /// extraction cache directory for installed game copies.
    pub data_dir: Option<PathBuf>,
    /// Whether first-run setup has been completed.
    pub setup_complete: bool,
    /// Last opened map path (directory or .wz file) for auto-reload on startup.
    pub last_opened_map: Option<PathBuf>,
    /// Archive prefix for multi-map `.wz` files (e.g. `"multiplay/maps/2c-Roughness/"`).
    /// Empty for single-map archives or directory loads.
    #[serde(default)]
    pub last_opened_map_prefix: Option<String>,
    /// Persisted rendering settings (fog, shadows, water, etc.).
    #[serde(default)]
    pub render_settings: Option<crate::viewport::renderer::RenderSettings>,
    /// Whether to show the grid overlay.
    #[serde(default)]
    pub show_grid: Option<bool>,
    /// Whether to show the build margin border overlay.
    #[serde(default)]
    pub show_border: Option<bool>,
    /// Whether to show script label overlays in the viewport.
    #[serde(default)]
    pub show_labels: Option<bool>,
    /// Whether to show gateway overlays in the viewport.
    #[serde(default)]
    pub show_gateways: Option<bool>,
    /// Balance panel: draw the per-tile nearest-player partition outline.
    #[serde(default)]
    pub show_zone_lines: Option<bool>,
    /// Balance panel: tint each Voronoi cell with its owning player's color.
    #[serde(default)]
    pub show_zone_fill: Option<bool>,
    /// Show hitbox wireframes on every map object (View menu toggle).
    #[serde(default)]
    pub show_all_hitboxes: Option<bool>,
    /// Whether the FPS / frame-time readout overlay is shown.
    #[serde(default)]
    pub show_fps: Option<bool>,
    /// Persisted weather override from the View > Weather submenu.
    #[serde(default)]
    pub view_weather: Option<wz_maplib::Weather>,
    /// Show hitbox wireframes only on selected objects (Settings toggle).
    /// Aliased to the old `show_hitboxes` key so configs from earlier
    /// builds, where that flag meant "on selected", keep their value.
    #[serde(default, alias = "show_hitboxes")]
    pub show_selection_hitboxes: Option<bool>,
    /// Asset browser: grid view (true) or list view (false).
    #[serde(default)]
    pub asset_grid_view: Option<bool>,
    /// Asset browser: thumbnail size.
    #[serde(default)]
    pub asset_thumb_size: Option<f32>,
    /// Asset browser: show campaign-only droid templates and structures
    /// (false hides them since they don't spawn in skirmish maps).
    #[serde(default)]
    pub asset_show_campaign_only: Option<bool>,
    /// Whether the minimap is visible.
    #[serde(default)]
    pub minimap_visible: Option<bool>,
    /// Active bottom dock tab name (legacy, kept for backwards compat on load).
    #[serde(default)]
    pub active_tab: Option<String>,
    /// Serialized dock layout (viewport + tool/browser tab positions and splits).
    /// Deserialized leniently: a layout saved by an older `egui_dock` whose
    /// internal schema has since changed is silently dropped instead of failing
    /// the whole config load.
    #[serde(default, deserialize_with = "deserialize_dock_layout_lenient")]
    pub dock_layout: Option<egui_dock::DockState<crate::app::DockTab>>,
    /// User-defined tile groups per tileset (keyed by "arizona", "urban", "rockies").
    #[serde(default)]
    pub custom_tile_groups:
        std::collections::HashMap<String, Vec<crate::tools::ground_type_brush::CustomTileGroup>>,
    /// User-customizable keyboard shortcuts.
    #[serde(default = "crate::keybindings::Keymap::default_keymap")]
    pub keymap: crate::keybindings::Keymap,
    /// Validation warning configuration (which warnings are disabled).
    #[serde(default)]
    pub validation_config: wz_maplib::ValidationConfig,
    /// Whether periodic auto-save is enabled.
    #[serde(default = "default_true")]
    pub autosave_enabled: bool,
    /// Auto-save interval in seconds.
    #[serde(default = "crate::autosave::default_interval")]
    pub autosave_interval_secs: u64,
    /// Last-used tileset name (e.g. "rockies"), used to pre-load the right
    /// tileset on startup instead of always defaulting to Arizona.
    #[serde(default)]
    pub last_tileset: Option<String>,
    /// Graphics backend preference. Per-platform set; see
    /// `GraphicsBackend::available_for_platform`. Changing requires restart.
    #[serde(default)]
    pub graphics_backend: GraphicsBackend,
    /// Swapchain present mode. Changing this requires an app restart.
    #[serde(default)]
    pub present_mode: PresentMode,
    /// Optional frame-rate cap, independent of vsync. `None` is uncapped;
    /// `Some(n)` throttles `update()` so at least `1/n` seconds elapse
    /// between frames. Applied at the eframe layer rather than the
    /// swapchain, so input is sampled fresh each capped frame instead of
    /// waiting for the presentation queue to drain.
    #[serde(default)]
    pub fps_limit: Option<u32>,
    /// UI theme preference. `System` follows the OS dark/light setting.
    #[serde(default)]
    pub theme_preference: ThemePreference,
    /// Explicit path to the Warzone 2100 executable used by Test Map.
    /// Overrides the auto-detection via `game_install_dir` when set.
    #[serde(default)]
    pub wz_executable: Option<PathBuf>,
    /// Override for the WZ2100 user configuration directory. When unset,
    /// the editor falls back to `wz2100_config_dir()`.
    #[serde(default)]
    pub wz_config_dir: Option<PathBuf>,
    /// Default `level.json` author name used when saving a new map.
    #[serde(default)]
    pub default_author_name: Option<String>,
    /// Whether to query GitHub for a newer release on startup.
    #[serde(default = "default_true")]
    pub check_for_updates_on_startup: bool,
    /// Version the user dismissed via the toolbar update button; the
    /// notification stays hidden until a strictly newer version exists.
    #[serde(default)]
    pub dismissed_update_version: Option<String>,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            game_install_dir: None,
            data_dir: None,
            setup_complete: false,
            last_opened_map: None,
            last_opened_map_prefix: None,
            render_settings: None,
            show_grid: None,
            show_border: None,
            show_labels: None,
            show_gateways: None,
            show_zone_lines: None,
            show_zone_fill: None,
            show_all_hitboxes: None,
            show_fps: None,
            view_weather: None,
            show_selection_hitboxes: None,
            asset_grid_view: None,
            asset_thumb_size: None,
            asset_show_campaign_only: None,
            minimap_visible: None,
            active_tab: None,
            dock_layout: None,
            custom_tile_groups: std::collections::HashMap::new(),
            keymap: crate::keybindings::Keymap::default_keymap(),
            validation_config: wz_maplib::ValidationConfig::default(),
            autosave_enabled: true,
            autosave_interval_secs: crate::autosave::DEFAULT_INTERVAL_SECS,
            last_tileset: None,
            graphics_backend: GraphicsBackend::default(),
            present_mode: PresentMode::default(),
            fps_limit: None,
            theme_preference: ThemePreference::default(),
            wz_executable: None,
            wz_config_dir: None,
            default_author_name: None,
            check_for_updates_on_startup: true,
            dismissed_update_version: None,
        }
    }
}

/// localStorage key the web build persists the serialized config under.
#[cfg(target_arch = "wasm32")]
const WEB_CONFIG_KEY: &str = "wzmapeditor.config";

#[cfg(target_arch = "wasm32")]
fn web_local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

impl EditorConfig {
    /// Path to the config file.
    #[cfg(not(target_arch = "wasm32"))]
    fn config_path() -> PathBuf {
        let base = dirs_next().unwrap_or_else(|| PathBuf::from("."));
        base.join("wzmapeditor.json")
    }

    /// Load config, or return defaults if none is stored.
    pub fn load() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            Self::load_web()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = Self::config_path();
            if path.exists() {
                match std::fs::read_to_string(&path) {
                    Ok(content) => match serde_json::from_str(&content) {
                        Ok(config) => {
                            log::info!("Loaded config from {}", path.display());
                            return config;
                        }
                        Err(e) => log::warn!("Failed to parse config: {e}"),
                    },
                    Err(e) => log::warn!("Failed to read config: {e}"),
                }
            }
            log::info!("No config found, using defaults");
            Self::default()
        }
    }

    /// Persist config.
    pub fn save(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            self.save_web();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = Self::config_path();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match serde_json::to_string_pretty(self) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(&path, json) {
                        log::error!("Failed to write config to {}: {}", path.display(), e);
                    } else {
                        log::info!("Saved config to {}", path.display());
                    }
                }
                Err(e) => log::error!("Failed to serialize config: {e}"),
            }
        }
    }

    /// Read the config from `localStorage`; defaults when absent or invalid.
    #[cfg(target_arch = "wasm32")]
    fn load_web() -> Self {
        let Some(storage) = web_local_storage() else {
            log::warn!("localStorage unavailable; using default config");
            return Self::default();
        };
        let Ok(Some(json)) = storage.get_item(WEB_CONFIG_KEY) else {
            log::info!("No stored config, using defaults");
            return Self::default();
        };
        match serde_json::from_str(&json) {
            Ok(config) => {
                log::info!("Loaded config from localStorage");
                config
            }
            Err(e) => {
                log::warn!("Failed to parse stored config: {e}");
                Self::default()
            }
        }
    }

    /// Write the config to `localStorage`.
    #[cfg(target_arch = "wasm32")]
    fn save_web(&self) {
        let json = match serde_json::to_string(self) {
            Ok(json) => json,
            Err(e) => {
                log::error!("Failed to serialize config: {e}");
                return;
            }
        };
        let Some(storage) = web_local_storage() else {
            log::warn!("localStorage unavailable; config not persisted");
            return;
        };
        match storage.set_item(WEB_CONFIG_KEY, &json) {
            Ok(()) => log::info!("Saved config to localStorage"),
            Err(e) => log::warn!("Failed to store config: {e:?}"),
        }
    }

    /// Get the tileset directory path from `data_dir` for a given tileset.
    pub fn tileset_dir_for(&self, tileset: Tileset) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join(tileset.subpath()))
    }
}

/// WZ2100 terrain tilesets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tileset {
    Arizona,
    Urban,
    Rockies,
}

impl Tileset {
    /// All available tilesets in display order.
    pub const ALL: [Self; 3] = [Self::Arizona, Self::Urban, Self::Rockies];

    /// Lowercase name matching WZ2100 conventions (e.g. `"arizona"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Arizona => "arizona",
            Self::Urban => "urban",
            Self::Rockies => "rockies",
        }
    }

    /// Parse a tileset from its lowercase name (e.g. `"rockies"` → `Rockies`).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "arizona" => Some(Self::Arizona),
            "urban" => Some(Self::Urban),
            "rockies" => Some(Self::Rockies),
            _ => None,
        }
    }

    /// PIE texture page index for multi-texture models (0=Arizona, 1=Urban, 2=Rockies).
    pub fn texture_index(self) -> usize {
        match self {
            Self::Arizona => 0,
            Self::Urban => 1,
            Self::Rockies => 2,
        }
    }

    /// Subdirectory path for 128x128 tile textures within the WZ2100 data directory.
    pub fn subpath(self) -> &'static str {
        match self {
            Tileset::Arizona => "base/texpages/tertilesc1hw-128",
            Tileset::Urban => "base/texpages/tertilesc2hw-128",
            Tileset::Rockies => "base/texpages/tertilesc3hw-128",
        }
    }

    /// Subdirectory for 256x256 normal/specular tile maps (decal `_nm`/`_sm` files).
    pub fn subpath_256(self) -> &'static str {
        match self {
            Tileset::Arizona => "base/texpages/tertilesc1hw-256",
            Tileset::Urban => "base/texpages/tertilesc2hw-256",
            Tileset::Rockies => "base/texpages/tertilesc3hw-256",
        }
    }

    /// Detect tileset from the first three terrain type values in the TTP
    /// data, which form a unique per-tileset signature (matches the
    /// maptools-cli approach).
    pub fn from_terrain_types(types: &[u16]) -> Self {
        if types.len() >= 3 {
            match (types[0], types[1], types[2]) {
                // Urban: Bakedearth, Bakedearth, Bakedearth.
                (2, 2, 2) => Tileset::Urban,
                // Rockies: Sand, Sand, Bakedearth.
                (0, 0, 2) => Tileset::Rockies,
                // Arizona (SandYellow, Sand, Bakedearth) and default.
                _ => Tileset::Arizona,
            }
        } else {
            Tileset::Arizona
        }
    }

    /// Default TTP terrain type signature for this tileset (first 3 entries).
    pub fn default_terrain_types(self) -> Vec<wz_maplib::TerrainType> {
        use wz_maplib::TerrainType;
        match self {
            Self::Arizona => vec![
                TerrainType::SandYellow,
                TerrainType::Sand,
                TerrainType::Bakedearth,
            ],
            Self::Urban => vec![
                TerrainType::Bakedearth,
                TerrainType::Bakedearth,
                TerrainType::Bakedearth,
            ],
            Self::Rockies => vec![
                TerrainType::Sand,
                TerrainType::Sand,
                TerrainType::Bakedearth,
            ],
        }
    }

    /// Full canonical per-texture terrain-type table for the tileset, matching
    /// the standard WZ2100 stock maps (Arizona/Urban have 96 textures, Rockies
    /// 90). New maps embed this so every cliff and water texture is typed
    /// correctly for in-game pathfinding and the tileset browser — unlike
    /// [`Self::default_terrain_types`], which only covers the first few.
    pub fn full_terrain_types(self) -> Vec<wz_maplib::TerrainType> {
        #[rustfmt::skip]
        let ids: &[u16] = match self {
            Self::Arizona => &[
                1, 0, 2, 2, 0, 2, 2, 2, 2, 1, 1, 1, 0, 7, 7, 7, 7, 7, 8, 6,
                4, 4, 6, 3, 3, 3, 2, 4, 1, 4, 7, 7, 7, 7, 4, 4, 2, 2, 2, 2,
                1, 4, 0, 4, 4, 8, 8, 2, 4, 4, 4, 4, 4, 4, 4, 9, 9, 6, 9, 6,
                4, 4, 9, 9, 9, 9, 9, 9, 9, 9, 9, 8, 4, 4, 4, 8, 5, 6, 2, 2,
                2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0,
            ],
            Self::Urban => &[
                2, 2, 2, 2, 1, 2, 2, 1, 1, 1, 1, 1, 1, 7, 7, 7, 7, 7, 1, 8,
                4, 4, 0, 7, 7, 7, 7, 4, 4, 2, 4, 0, 2, 0, 0, 2, 4, 4, 0, 4,
                6, 2, 6, 6, 6, 6, 6, 6, 4, 6, 3, 4, 4, 2, 2, 9, 9, 9, 2, 4,
                2, 4, 9, 9, 9, 9, 9, 8, 8, 8, 8, 4, 2, 0, 4, 4, 2, 2, 2, 3,
                2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0,
            ],
            Self::Rockies => &[
                0, 0, 2, 2, 2, 2, 2, 2, 1, 8, 11, 2, 11, 6, 7, 7, 7, 7, 8, 6,
                1, 2, 6, 11, 11, 0, 11, 1, 1, 8, 8, 7, 7, 7, 0, 0, 1, 6, 0, 4,
                5, 11, 8, 5, 8, 8, 8, 11, 11, 1, 1, 1, 1, 1, 8, 9, 9, 5, 2, 6,
                6, 8, 9, 8, 10, 10, 11, 11, 8, 8, 10, 8, 1, 10, 0, 10, 8, 8, 8, 6,
                2, 2, 2, 2, 2, 2, 2, 2, 2, 0,
            ],
        };
        ids.iter()
            .copied()
            .map(wz_maplib::TerrainType::from)
            .collect()
    }
}

impl std::fmt::Display for Tileset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arizona => write!(f, "Arizona"),
            Self::Urban => write!(f, "Urban"),
            Self::Rockies => write!(f, "Rockies"),
        }
    }
}

fn default_true() -> bool {
    true
}

/// On any deserialize error fall through to the programmatic default
/// layout. Older configs from a previous `egui_dock` version may be missing
/// fields the current version requires.
fn deserialize_dock_layout_lenient<'de, D>(
    deserializer: D,
) -> Result<Option<egui_dock::DockState<crate::app::DockTab>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// `<config_dir>/wzmapeditor.log`. Overwritten each launch.
#[cfg(not(target_arch = "wasm32"))]
pub fn log_file_path() -> PathBuf {
    dirs_next()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wzmapeditor.log")
}

/// `<config_dir>/base-cache`. Content extracted from `base.wz` lives here
/// so it survives restarts without re-extraction.
pub fn extraction_cache_dir() -> PathBuf {
    dirs_next()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("base-cache")
}

/// `<config_dir>/ground-cache-v5`. Decoded RGBA ground textures (resized to
/// 1024x1024) are stored as raw `.bin` files for instant loading without
/// PNG/KTX2 decode overhead.
pub fn ground_cache_dir() -> PathBuf {
    dirs_next()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ground-cache-v5")
}

/// `<config_dir>/autosave`. Temporary `.wz` archives are written here
/// periodically and cleaned up after explicit user saves.
pub fn autosave_dir() -> PathBuf {
    dirs_next()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("autosave")
}

/// `<config_dir>/thumb-cache`. 128x128 PNG model thumbnails that persist
/// across restarts; invalidated when the cache version changes.
#[cfg(not(target_arch = "wasm32"))]
pub fn thumb_cache_dir() -> PathBuf {
    dirs_next()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("thumb-cache")
}

/// `%APPDATA%\wzmapeditor` on Windows, `~/.config/wzmapeditor` on Unix.
pub fn config_dir() -> PathBuf {
    dirs_next().unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve the WZ2100 user config directory, preferring an explicit
/// override from the editor's settings over auto-detection.
pub fn resolve_wz_config_dir(config: &EditorConfig) -> Option<PathBuf> {
    if let Some(ref dir) = config.wz_config_dir {
        return Some(dir.clone());
    }
    wz2100_config_dir()
}

/// Platform-appropriate path where WZ2100 stores user data (maps, tests,
/// replays). Different WZ2100 builds use different base directories
/// (`%APPDATA%` vs `%LOCALAPPDATA%` on Windows), so this checks both and
/// picks whichever contains an existing WZ2100 profile.
pub fn wz2100_config_dir() -> Option<PathBuf> {
    let bases = wz2100_config_bases();
    let suffixes = ["4.5", "4.4", "4.3", "4.2"];

    // First pass: find an existing versioned or bare directory.
    for base in &bases {
        for suffix in &suffixes {
            let versioned = base.join(format!("Warzone 2100-{suffix}"));
            if versioned.exists() {
                log_detected_config_dir(&versioned);
                return Some(versioned);
            }
        }
        let bare = base.join("Warzone 2100");
        if bare.exists() {
            log_detected_config_dir(&bare);
            return Some(bare);
        }
    }

    // Nothing found, fall back to the first base with a reasonable default.
    bases.first().map(|b| b.join("Warzone 2100-4.5"))
}

/// Log the detected WZ2100 config dir, suppressing repeats of the same path.
///
/// `wz2100_config_dir` is re-resolved every frame the Game settings tab is
/// shown, so logging unconditionally floods the log.
fn log_detected_config_dir(dir: &Path) {
    static LAST_LOGGED: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
    if let Ok(mut last) = LAST_LOGGED.lock()
        && last.as_deref() != Some(dir)
    {
        log::info!("Detected WZ2100 config dir: {}", dir.display());
        *last = Some(dir.to_path_buf());
    }
}

/// Candidate base directories for WZ2100 user profiles. Different WZ2100
/// builds (installer, portable, Steam) may use `%APPDATA%` (Roaming) or
/// `%LOCALAPPDATA%` (Local) on Windows; callers probe each for existence.
fn wz2100_config_bases() -> Vec<PathBuf> {
    let mut bases = Vec::new();

    #[cfg(target_os = "windows")]
    {
        // Roaming first, most common for official WZ2100 builds.
        if let Ok(appdata) = std::env::var("APPDATA") {
            bases.push(Path::new(&appdata).join("Warzone 2100 Project"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            bases.push(Path::new(&local).join("Warzone 2100 Project"));
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            bases.push(Path::new(&home).join(".local/share/Warzone 2100 Project"));
        }
    }

    bases
}

/// Locate the WZ2100 executable relative to the game install directory.
/// `game_install_dir` from config may point to the `data/` subfolder rather
/// than the game root, so this checks the dir itself, its parent, and
/// common `bin/` subdirectories.
pub fn wz2100_executable(game_install_dir: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let name = "warzone2100.exe";
    #[cfg(not(target_os = "windows"))]
    let name = "warzone2100";

    let mut candidates: Vec<PathBuf> = vec![game_install_dir.to_path_buf()];
    if let Some(parent) = game_install_dir.parent() {
        // game_install_dir is likely `<root>/data/`, try the game root and bin/.
        candidates.push(parent.to_path_buf());
        candidates.push(parent.join("bin"));
    }
    candidates.push(game_install_dir.join("bin"));

    for dir in &candidates {
        let exe = dir.join(name);
        if exe.exists() {
            return Some(exe);
        }
    }
    None
}

/// Platform-appropriate config directory: `%APPDATA%\wzmapeditor` on
/// Windows, `~/.config/wzmapeditor` on Unix/macOS. Falls back to the
/// current working directory when no home variable is set.
fn dirs_next() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(Path::new(&appdata).join("wzmapeditor"));
    }

    #[cfg(not(target_os = "windows"))]
    if let Ok(home) = std::env::var("HOME") {
        return Some(Path::new(&home).join(".config").join("wzmapeditor"));
    }

    // USERPROFILE covers Windows when APPDATA is absent.
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return Some(Path::new(&profile).join(".config").join("wzmapeditor"));
    }

    std::env::current_dir().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn full_terrain_types_match_stock_maps() {
        use wz_maplib::TerrainType;

        let indices_of = |ts: Tileset, want: TerrainType| -> Vec<usize> {
            ts.full_terrain_types()
                .into_iter()
                .enumerate()
                .filter(|&(_, t)| t == want)
                .map(|(i, _)| i)
                .collect()
        };
        let cliffs = |ts: Tileset| indices_of(ts, TerrainType::Cliffface);
        let water = |ts: Tileset| indices_of(ts, TerrainType::Water);

        // Lengths and cliff/water indices taken from the canonical stock
        // ttypes.ttp shipped with WZ2100 (the plurality per tileset).
        assert_eq!(Tileset::Arizona.full_terrain_types().len(), 96);
        assert_eq!(Tileset::Urban.full_terrain_types().len(), 96);
        assert_eq!(Tileset::Rockies.full_terrain_types().len(), 90);
        assert_eq!(cliffs(Tileset::Arizona), vec![18, 45, 46, 71, 75]);
        assert_eq!(
            water(Tileset::Arizona),
            vec![13, 14, 15, 16, 17, 30, 31, 32, 33]
        );
        assert_eq!(cliffs(Tileset::Urban), vec![19, 67, 68, 69, 70]);

        // The first three entries are the tileset-detection signature, so the
        // full table must agree with the short default there.
        for ts in [Tileset::Arizona, Tileset::Urban, Tileset::Rockies] {
            assert_eq!(ts.full_terrain_types()[..3], ts.default_terrain_types()[..]);
        }
    }

    #[test]
    fn wz2100_executable_finds_exe_in_same_dir() {
        let dir = tempfile::tempdir().expect("temp dir");
        #[cfg(target_os = "windows")]
        let name = "warzone2100.exe";
        #[cfg(not(target_os = "windows"))]
        let name = "warzone2100";

        let exe_path = dir.path().join(name);
        fs::write(&exe_path, b"fake").expect("write");

        let result = wz2100_executable(dir.path());
        assert_eq!(result, Some(exe_path));
    }

    #[test]
    fn wz2100_executable_finds_exe_in_parent() {
        // Simulates game_install_dir = <root>/data/
        let root = tempfile::tempdir().expect("temp dir");
        let data_dir = root.path().join("data");
        fs::create_dir(&data_dir).expect("mkdir");

        #[cfg(target_os = "windows")]
        let name = "warzone2100.exe";
        #[cfg(not(target_os = "windows"))]
        let name = "warzone2100";

        let exe_path = root.path().join(name);
        fs::write(&exe_path, b"fake").expect("write");

        let result = wz2100_executable(&data_dir);
        assert_eq!(result, Some(exe_path));
    }

    #[test]
    fn wz2100_executable_finds_exe_in_parent_bin() {
        // Simulates game_install_dir = <root>/data/, exe at <root>/bin/
        let root = tempfile::tempdir().expect("temp dir");
        let data_dir = root.path().join("data");
        let bin_dir = root.path().join("bin");
        fs::create_dir(&data_dir).expect("mkdir data");
        fs::create_dir(&bin_dir).expect("mkdir bin");

        #[cfg(target_os = "windows")]
        let name = "warzone2100.exe";
        #[cfg(not(target_os = "windows"))]
        let name = "warzone2100";

        let exe_path = bin_dir.join(name);
        fs::write(&exe_path, b"fake").expect("write");

        let result = wz2100_executable(&data_dir);
        assert_eq!(result, Some(exe_path));
    }

    #[test]
    fn fps_limit_defaults_to_uncapped() {
        let cfg = EditorConfig::default();
        assert_eq!(cfg.fps_limit, None);
    }

    #[test]
    fn fps_limit_round_trips_through_json() {
        let cfg = EditorConfig {
            fps_limit: Some(72),
            ..EditorConfig::default()
        };
        let json = serde_json::to_string(&cfg).expect("serialize");
        let parsed: EditorConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.fps_limit, Some(72));
    }

    #[test]
    fn fps_limit_missing_field_loads_as_none() {
        // Older configs predate fps_limit; serde(default) keeps them loadable.
        let json = r#"{
            "game_install_dir": null,
            "data_dir": null,
            "setup_complete": false,
            "last_opened_map": null
        }"#;
        let cfg: EditorConfig = serde_json::from_str(json).expect("deserialize legacy");
        assert_eq!(cfg.fps_limit, None);
    }

    #[test]
    fn wz2100_executable_returns_none_when_missing() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(wz2100_executable(dir.path()), None);
    }
}
