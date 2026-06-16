//! Online map browser. Fetches and downloads maps from the WZ2100 maps database.
//!
//! API: <https://github.com/Warzone2100/maps-database/blob/main/docs/API.md>

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc;

use egui::{Color32, TextureHandle, TextureOptions, Vec2};

#[cfg(not(target_arch = "wasm32"))]
const API_BASE: &str = "https://maps.wz2100.net";
#[cfg(not(target_arch = "wasm32"))]
const API_FULL: &str = "https://maps.wz2100.net/api/v1/full.json";

/// Replace `{hash}` with the map's download hash.
const PREVIEW_URL_TEMPLATE: &str = "https://maps-assets.wz2100.net/v1/maps/{hash}/preview.png";

/// Replace `{repo}` and `{path}`.
const DOWNLOAD_URL_TEMPLATE: &str =
    "https://github.com/Warzone2100/maps-{repo}/releases/download/{path}";

const THUMB_SIZE: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchState {
    Idle,
    Fetching,
    Done,
    Error,
}

pub struct OnlineMapEntry {
    pub name: String,
    pub players: u8,
    pub tileset: String,
    pub author: String,
    pub width: u32,
    pub height: u32,
    pub oil_wells: u32,
    pub scavengers: u32,
    pub download_hash: String,
    pub download_repo: String,
    pub download_path: String,
    pub file_size: u64,
    pub created: String,
    /// Loaded asynchronously.
    pub thumbnail: Option<TextureHandle>,
    pub thumbnail_requested: bool,
    /// True when the .wz file is present in the user's maps directory.
    pub downloaded: bool,
}

enum FetchResult {
    MapList(Result<Vec<OnlineMapEntry>, String>),
    Preview(usize, Result<Vec<u8>, String>),
    Download(usize, Result<PathBuf, String>),
}

pub struct OnlineMapsState {
    pub fetch_state: FetchState,
    pub maps: Vec<OnlineMapEntry>,
    pub error_message: Option<String>,
    pub filter: String,
    pub player_filter: Option<u8>,
    pub tileset_filter: Option<String>,
    pub min_oil: Option<u32>,
    rx: Option<mpsc::Receiver<FetchResult>>,
    tx: Option<mpsc::Sender<FetchResult>>,
    /// Set when a map is downloaded; signals the map browser to rescan My Maps.
    pub maps_changed: bool,
    previews_in_flight: u32,
    /// 0-indexed page for paginated display.
    pub page: usize,
    /// Indices visible on the last rendered page, used for texture eviction.
    visible_indices: Vec<usize>,
    /// Recently-evicted thumbnail handles, held for a few frames so an
    /// in-flight GPU command buffer that still references them can finish
    /// before `egui_wgpu` destroys the backing texture. Without this
    /// cooldown, freeing during rapid filter changes races the previous
    /// frame's queue submit and trips a wgpu validation error.
    pending_evictions: VecDeque<Vec<TextureHandle>>,
}

/// Three frames is enough to outlive any command buffer that was in flight
/// when the texture left the visible set, while keeping extra GPU memory
/// bounded.
const EVICTION_COOLDOWN_FRAMES: usize = 3;

const PAGE_SIZE: usize = 50;

impl std::fmt::Debug for OnlineMapsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnlineMapsState")
            .field("fetch_state", &self.fetch_state)
            .field("map_count", &self.maps.len())
            .field("filter", &self.filter)
            .finish_non_exhaustive()
    }
}

impl Default for OnlineMapsState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            fetch_state: FetchState::Idle,
            maps: Vec::new(),
            error_message: None,
            filter: String::new(),
            player_filter: None,
            tileset_filter: None,
            min_oil: None,
            rx: Some(rx),
            tx: Some(tx),
            maps_changed: false,
            previews_in_flight: 0,
            page: 0,
            visible_indices: Vec::new(),
            pending_evictions: VecDeque::new(),
        }
    }
}

impl OnlineMapsState {
    /// Start fetching the map list in a background thread.
    pub fn start_fetch(&mut self) {
        if self.fetch_state == FetchState::Fetching {
            return;
        }
        self.fetch_state = FetchState::Fetching;
        self.error_message = None;
        self.maps.clear();

        let tx = self.tx.as_ref().expect("sender available").clone();
        let user_dirs = super::map_browser::wz2100_user_map_dirs();

        let work = move || {
            let result = fetch_map_list(&user_dirs);
            let _ = tx.send(FetchResult::MapList(result));
        };
        // No usable OS threads in the browser; the web `fetch_*` helpers are
        // stubs, so run them inline and let `poll` drain the result.
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(work);
        #[cfg(target_arch = "wasm32")]
        work();
    }

    const MAX_PREVIEW_IN_FLIGHT: u32 = 8;

    /// Request a preview thumbnail if under the concurrency limit.
    /// Returns true if the request was dispatched.
    pub fn try_request_preview(&mut self, index: usize) -> bool {
        if self.previews_in_flight >= Self::MAX_PREVIEW_IN_FLIGHT {
            return false;
        }
        let Some(entry) = self.maps.get_mut(index) else {
            return false;
        };
        if entry.thumbnail_requested || entry.thumbnail.is_some() || entry.download_hash.is_empty()
        {
            return false;
        }
        entry.thumbnail_requested = true;
        self.previews_in_flight += 1;
        let hash = entry.download_hash.clone();
        self.request_preview(index, &hash);
        true
    }

    fn request_preview(&self, index: usize, hash: &str) {
        let tx = self.tx.as_ref().expect("sender available").clone();
        let url = PREVIEW_URL_TEMPLATE.replace("{hash}", hash);

        let work = move || {
            let result = fetch_bytes(&url);
            let _ = tx.send(FetchResult::Preview(index, result));
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(work);
        #[cfg(target_arch = "wasm32")]
        work();
    }

    pub fn start_download(&self, index: usize, repo: &str, path: &str, filename: &str) {
        let tx = self.tx.as_ref().expect("sender available").clone();
        let url = DOWNLOAD_URL_TEMPLATE
            .replace("{repo}", repo)
            .replace("{path}", path);
        let user_dirs = super::map_browser::wz2100_user_map_dirs();
        let filename = filename.to_string();

        let work = move || {
            let result = download_map(&url, &user_dirs, &filename);
            let _ = tx.send(FetchResult::Download(index, result));
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(work);
        #[cfg(target_arch = "wasm32")]
        work();
    }

    /// Free thumbnails for maps not in the current visible set, preventing
    /// GPU descriptor heap exhaustion when browsing many pages. Evicted
    /// `TextureHandle`s are parked in `pending_evictions` for a few frames
    /// so the previous frame's command buffer (which may still hold a
    /// reference via `egui_wgpu`) can finish on the GPU before the backing
    /// texture is destroyed. Call every frame; the cooldown queue advances
    /// even when nothing was evicted this frame.
    fn evict_offscreen_thumbnails(&mut self, current_indices: &[usize]) {
        let visible: std::collections::HashSet<usize> = current_indices.iter().copied().collect();

        let mut freed_this_frame: Vec<TextureHandle> = Vec::new();
        for &idx in &self.visible_indices {
            if !visible.contains(&idx)
                && let Some(entry) = self.maps.get_mut(idx)
                && let Some(tex) = entry.thumbnail.take()
            {
                freed_this_frame.push(tex);
                entry.thumbnail_requested = false;
            }
        }

        self.pending_evictions.push_back(freed_this_frame);
        while self.pending_evictions.len() > EVICTION_COOLDOWN_FRAMES {
            self.pending_evictions.pop_front();
        }

        self.visible_indices = current_indices.to_vec();
    }

    /// Drain completed background operations. Call each frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(ref rx) = self.rx else { return };

        while let Ok(result) = rx.try_recv() {
            match result {
                FetchResult::MapList(Ok(maps)) => {
                    log::info!("Fetched {} maps from online database", maps.len());
                    self.maps = maps;
                    self.fetch_state = FetchState::Done;
                }
                FetchResult::MapList(Err(e)) => {
                    log::error!("Failed to fetch online maps: {e}");
                    self.error_message = Some(e);
                    self.fetch_state = FetchState::Error;
                }
                FetchResult::Preview(index, Ok(png_data)) => {
                    self.previews_in_flight = self.previews_in_flight.saturating_sub(1);
                    if let Some(entry) = self.maps.get_mut(index)
                        && let Some(tex) = load_preview_texture(ctx, &entry.name, &png_data)
                    {
                        entry.thumbnail = Some(tex);
                    }
                }
                FetchResult::Preview(index, Err(e)) => {
                    self.previews_in_flight = self.previews_in_flight.saturating_sub(1);
                    log::warn!("Failed to load preview for map {index}: {e}");
                }
                FetchResult::Download(index, Ok(path)) => {
                    log::info!("Downloaded map to: {}", path.display());
                    if let Some(entry) = self.maps.get_mut(index) {
                        entry.downloaded = true;
                    }
                    self.maps_changed = true;
                }
                FetchResult::Download(index, Err(e)) => {
                    log::error!("Failed to download map {index}: {e}");
                }
            }
        }
    }
}

/// Fetch the full map list from the API. Blocking; run on a worker thread.
#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "signature must match the native fetch_map_list"
)]
fn fetch_map_list(_user_dirs: &[PathBuf]) -> Result<Vec<OnlineMapEntry>, String> {
    log::warn!("Fetching the online map list is not available in the web build");
    Ok(Vec::new())
}

/// Fetch the full map list from the API. Blocking; run on a worker thread.
#[cfg(not(target_arch = "wasm32"))]
fn fetch_map_list(user_dirs: &[PathBuf]) -> Result<Vec<OnlineMapEntry>, String> {
    let mut all_maps = Vec::new();
    let mut url = API_FULL.to_string();

    loop {
        log::info!("Fetching online maps: {url}");
        let response_body = ureq::get(&url)
            .call()
            .map_err(|e| format!("HTTP request failed: {e}"))?
            .body_mut()
            .read_to_vec()
            .map_err(|e| format!("Read failed: {e}"))?;
        let body: serde_json::Value = serde_json::from_slice(&response_body)
            .map_err(|e| format!("JSON parse failed: {e}"))?;

        let maps = body["maps"]
            .as_array()
            .ok_or("Missing 'maps' array in API response")?;

        for map in maps {
            if let Some(entry) = parse_map_entry(map, user_dirs) {
                all_maps.push(entry);
            }
        }

        if let Some(next) = body["links"]["next"].as_str() {
            if next.is_empty() {
                break;
            }
            url = if next.starts_with("http") {
                next.to_string()
            } else {
                format!("{API_BASE}{next}")
            };
        } else {
            break;
        }
    }

    all_maps.sort_by(|a, b| a.players.cmp(&b.players).then_with(|| a.name.cmp(&b.name)));
    Ok(all_maps)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_map_entry(map: &serde_json::Value, user_dirs: &[PathBuf]) -> Option<OnlineMapEntry> {
    let name = map["name"].as_str()?.to_string();
    let players = map["slots"].as_u64().unwrap_or(0) as u8;
    let tileset = map["tileset"].as_str().unwrap_or("").to_string();
    let author = map["author"].as_str().unwrap_or("Unknown").to_string();
    let width = map["size"]["w"].as_u64().unwrap_or(0) as u32;
    let height = map["size"]["h"].as_u64().unwrap_or(0) as u32;
    let oil_wells = map["oilWells"].as_u64().unwrap_or(0) as u32;
    let scavengers = map["scavs"].as_u64().unwrap_or(0) as u32;
    let created = map["created"].as_str().unwrap_or("").to_string();

    let download = &map["download"];
    let download_hash = download["hash"].as_str().unwrap_or("").to_string();
    let download_repo = download["repo"].as_str().unwrap_or("").to_string();
    let download_path = download["path"].as_str().unwrap_or("").to_string();
    let file_size = download["size"].as_u64().unwrap_or(0);

    let wz_filename = download_path.rsplit('/').next().unwrap_or(&download_path);
    let downloaded = user_dirs.iter().any(|dir| dir.join(wz_filename).exists());

    Some(OnlineMapEntry {
        name,
        players,
        tileset,
        author,
        width,
        height,
        oil_wells,
        scavengers,
        download_hash,
        download_repo,
        download_path,
        file_size,
        created,
        thumbnail: None,
        thumbnail_requested: false,
        downloaded,
    })
}

/// Blocking GET.
#[cfg(target_arch = "wasm32")]
fn fetch_bytes(_url: &str) -> Result<Vec<u8>, String> {
    Err("network fetch is not available in the web build".to_string())
}

/// Blocking GET.
#[cfg(not(target_arch = "wasm32"))]
fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    ureq::get(url)
        .call()
        .map_err(|e| format!("HTTP error: {e}"))?
        .body_mut()
        .read_to_vec()
        .map_err(|e| format!("Read error: {e}"))
}

/// Blocking download into the user maps directory.
#[cfg(target_arch = "wasm32")]
fn download_map(_url: &str, _user_dirs: &[PathBuf], _filename: &str) -> Result<PathBuf, String> {
    log::warn!("Downloading online maps is not available in the web build");
    Err("downloading maps is not available in the web build".to_string())
}

/// Blocking download into the user maps directory.
#[cfg(not(target_arch = "wasm32"))]
fn download_map(url: &str, user_dirs: &[PathBuf], filename: &str) -> Result<PathBuf, String> {
    let dest_dir = user_dirs.first().ok_or("No user maps directory found")?;

    std::fs::create_dir_all(dest_dir).map_err(|e| format!("Cannot create maps directory: {e}"))?;

    let dest_path = dest_dir.join(filename);

    log::info!("Downloading {url} to {}", dest_path.display());
    let data = fetch_bytes(url)?;
    std::fs::write(&dest_path, &data).map_err(|e| format!("Cannot write file: {e}"))?;

    Ok(dest_path)
}

fn load_preview_texture(ctx: &egui::Context, name: &str, png_data: &[u8]) -> Option<TextureHandle> {
    let img = image::load_from_memory(png_data).ok()?.into_rgba8();
    let (w, h) = img.dimensions();

    let pixels: Vec<Color32> = (0..THUMB_SIZE * THUMB_SIZE)
        .map(|i| {
            let tx = i % THUMB_SIZE;
            let ty = i / THUMB_SIZE;
            let sx = (tx * w as usize) / THUMB_SIZE;
            let sy = (ty * h as usize) / THUMB_SIZE;
            let p = img.get_pixel(sx as u32, sy as u32);
            Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])
        })
        .collect();

    let image = egui::ColorImage::new([THUMB_SIZE, THUMB_SIZE], pixels);

    Some(ctx.load_texture(format!("online_{name}"), image, TextureOptions::LINEAR))
}

/// Render the online maps tab. `main_filter` is the search text from the
/// map browser's top-level search bar.
pub fn show_online_tab(
    ui: &mut egui::Ui,
    state: &mut OnlineMapsState,
    ctx: &egui::Context,
    main_filter: &str,
) -> Option<PathBuf> {
    if state.fetch_state == FetchState::Idle {
        state.start_fetch();
    }

    match state.fetch_state {
        FetchState::Idle | FetchState::Fetching => {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            ctx.request_repaint();
            return None;
        }
        FetchState::Error => {
            ui.centered_and_justified(|ui| {
                let msg = state.error_message.as_deref().unwrap_or("Unknown error");
                ui.label(
                    egui::RichText::new(format!("Failed to load maps: {msg}"))
                        .color(Color32::from_rgb(255, 100, 100)),
                );
            });
            if ui.button("Retry").clicked() {
                state.start_fetch();
            }
            return None;
        }
        FetchState::Done => {}
    }

    if state.filter != main_filter {
        state.filter = main_filter.to_string();
        state.page = 0;
    }

    ui.horizontal(|ui| {
        ui.label("Players:");
        egui::ComboBox::from_id_salt("online_player_filter")
            .selected_text(
                state
                    .player_filter
                    .map_or("All".to_string(), |p| format!("{p}")),
            )
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(state.player_filter.is_none(), "All")
                    .clicked()
                {
                    state.player_filter = None;
                    state.page = 0;
                }
                for p in [2, 3, 4, 5, 6, 7, 8, 10] {
                    if ui
                        .selectable_label(state.player_filter == Some(p), format!("{p}"))
                        .clicked()
                    {
                        state.player_filter = Some(p);
                        state.page = 0;
                    }
                }
            });
        ui.label("Tileset:");
        egui::ComboBox::from_id_salt("onlinetileset_filter")
            .selected_text(state.tileset_filter.as_deref().unwrap_or("All"))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(state.tileset_filter.is_none(), "All")
                    .clicked()
                {
                    state.tileset_filter = None;
                    state.page = 0;
                }
                for ts in ["arizona", "urban", "rockies"] {
                    if ui
                        .selectable_label(state.tileset_filter.as_deref() == Some(ts), ts)
                        .clicked()
                    {
                        state.tileset_filter = Some(ts.to_string());
                        state.page = 0;
                    }
                }
            });
        ui.label("Min Oil:");
        egui::ComboBox::from_id_salt("online_oil_filter")
            .selected_text(state.min_oil.map_or("Any".to_string(), |o| format!("{o}+")))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(state.min_oil.is_none(), "Any")
                    .clicked()
                {
                    state.min_oil = None;
                    state.page = 0;
                }
                for o in [4, 8, 12, 16, 24, 32] {
                    if ui
                        .selectable_label(state.min_oil == Some(o), format!("{o}+"))
                        .clicked()
                    {
                        state.min_oil = Some(o);
                        state.page = 0;
                    }
                }
            });
    });

    ui.separator();

    let filter_lower = main_filter.to_lowercase();
    let indices: Vec<usize> = state
        .maps
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            (filter_lower.is_empty()
                || m.name.to_lowercase().contains(&filter_lower)
                || m.author.to_lowercase().contains(&filter_lower))
                && state.player_filter.is_none_or(|p| m.players == p)
                && state
                    .tileset_filter
                    .as_ref()
                    .is_none_or(|ts| m.tileset.eq_ignore_ascii_case(ts))
                && state.min_oil.is_none_or(|min| m.oil_wells >= min)
        })
        .map(|(i, _)| i)
        .collect();

    if indices.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label("No maps match the filter.");
        });
        return None;
    }

    let total = indices.len();
    let total_pages = total.div_ceil(PAGE_SIZE);
    if state.page >= total_pages {
        state.page = total_pages.saturating_sub(1);
    }
    let page_start = state.page * PAGE_SIZE;
    let page_end = (page_start + PAGE_SIZE).min(total);
    let page_indices = &indices[page_start..page_end];

    state.evict_offscreen_thumbnails(page_indices);

    ui.horizontal(|ui| {
        if ui
            .add_enabled(state.page > 0, egui::Button::new("\u{25C0} Prev"))
            .clicked()
        {
            state.page = state.page.saturating_sub(1);
        }
        ui.label(format!(
            "Page {} of {} ({} maps)",
            state.page + 1,
            total_pages,
            total
        ));
        if ui
            .add_enabled(
                state.page + 1 < total_pages,
                egui::Button::new("Next \u{25B6}"),
            )
            .clicked()
        {
            state.page += 1;
        }
    });

    ui.separator();

    let mut load_path: Option<PathBuf> = None;
    let min_card_w = THUMB_SIZE as f32 + 16.0;
    let available = ui.available_width();
    let cols = ((available / min_card_w).floor() as usize).max(1);
    let card_w = available / cols as f32 - 8.0;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut groups: Vec<(u8, Vec<usize>)> = Vec::new();
            for &idx in page_indices {
                let players = state.maps[idx].players;
                if let Some(last) = groups.last_mut()
                    && last.0 == players
                {
                    last.1.push(idx);
                    continue;
                }
                groups.push((players, vec![idx]));
            }

            for (players, group_indices) in &groups {
                ui.add_space(8.0);
                if *players > 0 {
                    ui.heading(format!("{players} Players"));
                } else {
                    ui.heading("Unknown Player Count");
                }
                ui.separator();

                egui::Grid::new(format!("online_grid_{players}"))
                    .num_columns(cols)
                    .min_col_width(card_w)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        for (i, &idx) in group_indices.iter().enumerate() {
                            if i > 0 && i % cols == 0 {
                                ui.end_row();
                            }
                            state.try_request_preview(idx);
                            let action = show_online_map_card(ui, &state.maps[idx], idx);
                            match action {
                                CardAction::None => {}
                                CardAction::Download => {
                                    let entry = &state.maps[idx];
                                    let filename = entry
                                        .download_path
                                        .rsplit('/')
                                        .next()
                                        .unwrap_or(&entry.download_path);
                                    state.start_download(
                                        idx,
                                        &entry.download_repo,
                                        &entry.download_path,
                                        filename,
                                    );
                                }
                                CardAction::Load(path) => {
                                    load_path = Some(path);
                                }
                            }
                        }
                    });
            }
        });

    if state.previews_in_flight > 0 {
        ctx.request_repaint();
    }

    load_path
}

enum CardAction {
    None,
    Download,
    Load(PathBuf),
}

fn show_online_map_card(ui: &mut egui::Ui, entry: &OnlineMapEntry, _index: usize) -> CardAction {
    let thumb_size = Vec2::new(THUMB_SIZE as f32, THUMB_SIZE as f32);
    let mut action = CardAction::None;

    ui.vertical(|ui| {
        let response = if let Some(ref tex) = entry.thumbnail {
            ui.add(
                egui::Image::new(tex)
                    .fit_to_exact_size(thumb_size)
                    .sense(egui::Sense::click()),
            )
        } else {
            let (rect, response) = ui.allocate_exact_size(thumb_size, egui::Sense::click());
            ui.painter().rect_filled(rect, 4.0, Color32::from_gray(40));
            if entry.thumbnail_requested {
                ui.put(rect, egui::Spinner::new());
            }
            response
        };

        if response.hovered() {
            ui.painter().rect_stroke(
                response.rect,
                4.0,
                egui::Stroke::new(2.0, Color32::from_rgb(100, 180, 255)),
                egui::StrokeKind::Inside,
            );
        }

        // Count by `chars()` so non-ASCII community map names don't get
        // sliced mid-UTF-8.
        let display_name = if entry.name.chars().count() > 20 {
            let truncated: String = entry.name.chars().take(17).collect();
            format!("{truncated}...")
        } else {
            entry.name.clone()
        };
        ui.label(egui::RichText::new(&display_name).small().strong());

        ui.label(
            egui::RichText::new(format!(
                "{}x{} | {} | {}",
                entry.width,
                entry.height,
                entry.tileset,
                format_size(entry.file_size),
            ))
            .small()
            .weak(),
        );

        ui.label(egui::RichText::new(&entry.author).small().weak());

        if entry.downloaded {
            if response.clicked() {
                let user_dirs = super::map_browser::wz2100_user_map_dirs();
                let filename = entry
                    .download_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&entry.download_path);
                for dir in &user_dirs {
                    let path = dir.join(filename);
                    if path.exists() {
                        action = CardAction::Load(path);
                        break;
                    }
                }
            }
            ui.label(
                egui::RichText::new("Installed")
                    .small()
                    .color(Color32::from_rgb(100, 200, 100)),
            );
        } else if ui.button(egui::RichText::new("Download").small()).clicked() {
            action = CardAction::Download;
        }

        response.on_hover_text(format!(
            "{}\nby {}\n{}x{} tiles | {} players\n{} oils | {} scavs\n{} | {}\n{}",
            entry.name,
            entry.author,
            entry.width,
            entry.height,
            entry.players,
            entry.oil_wells,
            entry.scavengers,
            entry.tileset,
            entry.created,
            if entry.downloaded {
                "Click to open"
            } else {
                "Click Download to install"
            },
        ));
    });

    action
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `Arena_22` entry from the WZ2100 maps database API.
    fn sample_map_json() -> serde_json::Value {
        serde_json::json!({
            "name": "Arena_22",
            "slots": 2,
            "tileset": "arizona",
            "author": "Olrox",
            "license": "CC-BY-SA-3.0 OR GPL-2.0-or-later",
            "created": "2013-04-26",
            "size": { "w": 134, "h": 70 },
            "scavs": 0,
            "oilWells": 24,
            "player": {
                "units": { "eq": true, "min": 4, "max": 4 },
                "structs": { "eq": true, "min": 63, "max": 63 },
                "resourceExtr": { "eq": true, "min": 6, "max": 6 },
                "pwrGen": { "eq": true, "min": 1, "max": 1 },
                "regFact": { "eq": true, "min": 2, "max": 2 },
                "vtolFact": { "eq": true, "min": 0, "max": 0 },
                "cyborgFact": { "eq": true, "min": 0, "max": 0 },
                "researchCent": { "eq": true, "min": 1, "max": 1 },
                "defStruct": { "eq": true, "min": 34, "max": 34 }
            },
            "hq": [[12, 12], [123, 58]],
            "download": {
                "type": "jsonv2",
                "repo": "2p",
                "path": "v1/2p-Arena_22.wz",
                "uploaded": "2023-08-14",
                "hash": "399ed4da4e250bc63f458cffeb49b90ac0eaf6f63d3ae6a8da5b4b009c44d55e",
                "size": 25374
            }
        })
    }

    #[test]
    fn parse_map_entry_all_fields() {
        let json = sample_map_json();
        let entry = parse_map_entry(&json, &[]).expect("should parse");

        assert_eq!(entry.name, "Arena_22");
        assert_eq!(entry.players, 2);
        assert_eq!(entry.tileset, "arizona");
        assert_eq!(entry.author, "Olrox");
        assert_eq!(entry.width, 134);
        assert_eq!(entry.height, 70);
        assert_eq!(entry.oil_wells, 24);
        assert_eq!(entry.scavengers, 0);
        assert_eq!(entry.created, "2013-04-26");
        assert_eq!(entry.download_repo, "2p");
        assert_eq!(entry.download_path, "v1/2p-Arena_22.wz");
        assert_eq!(entry.file_size, 25374);
        assert_eq!(
            entry.download_hash,
            "399ed4da4e250bc63f458cffeb49b90ac0eaf6f63d3ae6a8da5b4b009c44d55e"
        );
        assert!(!entry.downloaded);
    }

    #[test]
    fn parse_map_entry_missing_name_returns_none() {
        let json = serde_json::json!({ "slots": 4 });
        assert!(parse_map_entry(&json, &[]).is_none());
    }

    #[test]
    fn parse_map_entry_missing_optional_fields_uses_defaults() {
        let json = serde_json::json!({ "name": "Minimal" });
        let entry = parse_map_entry(&json, &[]).expect("should parse with just name");

        assert_eq!(entry.name, "Minimal");
        assert_eq!(entry.players, 0);
        assert_eq!(entry.tileset, "");
        assert_eq!(entry.author, "Unknown");
        assert_eq!(entry.width, 0);
        assert_eq!(entry.height, 0);
        assert_eq!(entry.oil_wells, 0);
        assert_eq!(entry.file_size, 0);
    }

    #[test]
    fn download_url_construction() {
        let entry = parse_map_entry(&sample_map_json(), &[]).unwrap();
        let url = DOWNLOAD_URL_TEMPLATE
            .replace("{repo}", &entry.download_repo)
            .replace("{path}", &entry.download_path);
        assert_eq!(
            url,
            "https://github.com/Warzone2100/maps-2p/releases/download/v1/2p-Arena_22.wz"
        );
    }

    #[test]
    fn preview_url_construction() {
        let entry = parse_map_entry(&sample_map_json(), &[]).unwrap();
        let url = PREVIEW_URL_TEMPLATE.replace("{hash}", &entry.download_hash);
        assert_eq!(
            url,
            "https://maps-assets.wz2100.net/v1/maps/399ed4da4e250bc63f458cffeb49b90ac0eaf6f63d3ae6a8da5b4b009c44d55e/preview.png"
        );
    }

    #[test]
    fn downloaded_detection_with_matching_file() {
        let tmp = std::env::temp_dir().join("wz_test_maps");
        let _ = std::fs::create_dir_all(&tmp);
        let fake_wz = tmp.join("2p-Arena_22.wz");
        std::fs::write(&fake_wz, b"fake").unwrap();

        let entry = parse_map_entry(&sample_map_json(), std::slice::from_ref(&tmp)).unwrap();
        assert!(entry.downloaded);

        let _ = std::fs::remove_file(&fake_wz);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(25374), "24.8 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(1_048_576), "1.0 MB");
        assert_eq!(format_size(5_500_000), "5.2 MB");
    }

    #[test]
    fn filter_by_tileset() {
        let arizona = parse_map_entry(&sample_map_json(), &[]).unwrap();
        let mut urban_json = sample_map_json();
        urban_json["tileset"] = serde_json::json!("urban");
        urban_json["name"] = serde_json::json!("UrbanMap");
        let urban = parse_map_entry(&urban_json, &[]).unwrap();

        let maps = [arizona, urban];

        let filtered: Vec<&OnlineMapEntry> = maps
            .iter()
            .filter(|m| "arizona".eq_ignore_ascii_case(&m.tileset))
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "Arena_22");

        let filtered: Vec<&OnlineMapEntry> = maps
            .iter()
            .filter(|m| "urban".eq_ignore_ascii_case(&m.tileset))
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "UrbanMap");
    }

    #[test]
    fn filter_by_min_oil() {
        let entry = parse_map_entry(&sample_map_json(), &[]).unwrap();
        assert_eq!(entry.oil_wells, 24);

        assert!(entry.oil_wells >= 12); // passes min_oil=12
        assert!(entry.oil_wells >= 24); // passes min_oil=24
        assert!(entry.oil_wells < 32); // fails min_oil=32
    }

    #[test]
    fn filter_by_player_count() {
        let entry = parse_map_entry(&sample_map_json(), &[]).unwrap();
        assert_eq!(entry.players, 2);
    }

    #[test]
    fn api_response_structure() {
        // Verify our parsing handles the expected top-level API structure.
        let api_response = serde_json::json!({
            "type": "wz2100.mapdatabase.full.v1",
            "id": "full-page-1",
            "version": "2026-01-16 17:03:19",
            "links": { "self": "/api/v1/full.json" },
            "maps": [sample_map_json()]
        });

        let maps = api_response["maps"]
            .as_array()
            .expect("maps should be array");
        assert_eq!(maps.len(), 1);
        let entry = parse_map_entry(&maps[0], &[]).expect("should parse from API response");
        assert_eq!(entry.name, "Arena_22");
    }
}
