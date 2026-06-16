//! Periodic auto-save to a temporary location for crash recovery.
//!
//! Saves a `.wz` archive in the background on a configurable timer.
//! Auto-save files live in `<config_dir>/autosave/` and are tracked
//! by a JSON manifest so the editor can offer recovery on next launch.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use web_time::Instant;

use serde::{Deserialize, Serialize};

/// Manifest entries older than seven days are pruned during the startup
/// recovery scan to bound on-disk growth.
const STALE_THRESHOLD_SECS: u64 = 7 * 24 * 3600;

const MANIFEST_FILENAME: &str = "autosave-manifest.json";

/// Default auto-save interval (two minutes).
pub const DEFAULT_INTERVAL_SECS: u64 = 120;

/// Serde default for `EditorConfig::autosave_interval_secs`.
pub fn default_interval() -> u64 {
    DEFAULT_INTERVAL_SECS
}

/// Metadata for one auto-save file, persisted in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoSaveEntry {
    pub filename: String,
    pub map_name: String,
    pub original_path: Option<PathBuf>,
    /// Unix timestamp (seconds since epoch) of the auto-save write.
    pub timestamp: u64,
    pub map_width: u32,
    pub map_height: u32,
    pub players: u8,
}

/// Runtime state for the auto-save subsystem.
pub struct AutoSaveState {
    /// Random hex session id, avoids filename collisions when multiple
    /// editor instances run simultaneously.
    session_id: String,
    last_save: Instant,
    current_file: Option<PathBuf>,
    /// Background save result receiver. The thread returns path + entry so
    /// manifest I/O stays on the main thread, avoiding cross-thread file
    /// access races with `cleanup()`.
    save_rx: Option<mpsc::Receiver<Result<(PathBuf, AutoSaveEntry), String>>>,
}

impl std::fmt::Debug for AutoSaveState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoSaveState")
            .field("session_id", &self.session_id)
            .field("current_file", &self.current_file)
            .field("in_flight", &self.save_rx.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for AutoSaveState {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoSaveState {
    pub fn new() -> Self {
        Self {
            session_id: format!("{:08x}", fastrand::u32(..)),
            last_save: Instant::now(),
            current_file: None,
            save_rx: None,
        }
    }

    /// Check whether enough time has elapsed to trigger an auto-save.
    pub fn should_save(&self, interval_secs: u64) -> bool {
        self.save_rx.is_none() && self.last_save.elapsed().as_secs() >= interval_secs
    }

    /// Spawn a background thread to write a `.wz` archive of `map`.
    pub fn start_save(&mut self, map: &wz_maplib::WzMap, save_path: Option<&Path>, players: u8) {
        if self.save_rx.is_some() {
            return;
        }

        let (wz_path, entry) = prepare_save(map, save_path, players, &self.session_id);
        let map_clone = map.clone();

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = wz_maplib::io_wz::save_to_wz_archive(
                &map_clone,
                &wz_path,
                wz_maplib::OutputFormat::Ver3,
            );
            match result {
                Ok(()) => {
                    let _ = tx.send(Ok((wz_path, entry)));
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("{e}")));
                }
            }
        });

        self.save_rx = Some(rx);
    }

    /// Poll for background save completion. Manifest I/O runs on the main
    /// thread to avoid cross-thread file access races with `cleanup()`.
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        let rx = self.save_rx.as_ref()?;
        match rx.try_recv() {
            Ok(Ok((path, entry))) => {
                self.save_rx = None;
                self.last_save = Instant::now();
                self.current_file = Some(path);
                upsert_manifest_entry(entry);
                Some(Ok(()))
            }
            Ok(Err(msg)) => {
                self.save_rx = None;
                self.last_save = Instant::now();
                Some(Err(msg))
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.save_rx = None;
                self.last_save = Instant::now();
                Some(Err("auto-save thread terminated unexpectedly".to_string()))
            }
        }
    }

    /// Delete the current session's auto-save file and manifest entry,
    /// called after a successful user-initiated save.
    pub fn cleanup(&mut self) {
        if let Some(ref path) = self.current_file {
            if let Err(e) = std::fs::remove_file(path) {
                log::debug!("Could not remove auto-save file: {e}");
            }
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                remove_manifest_entry(filename);
            }
            log::info!("Cleaned up auto-save: {}", path.display());
        }
        self.current_file = None;
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.last_save.elapsed().as_secs()
    }

    /// Reset the timer (e.g. after loading a new map).
    pub fn reset_timer(&mut self) {
        self.last_save = Instant::now();
    }

    /// The session identifier (needed for synchronous on-exit save).
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// Read the manifest from disk. Returns an empty vec on missing or corrupt file.
fn load_manifest() -> Vec<AutoSaveEntry> {
    let path = crate::config::autosave_dir().join(MANIFEST_FILENAME);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Write the manifest atomically: write a `.tmp` then rename. The
/// destination is removed first because `std::fs::rename` fails on Windows
/// if the target already exists.
fn save_manifest(entries: &[AutoSaveEntry]) {
    let dir = crate::config::autosave_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("Failed to create autosave directory: {e}");
        return;
    }
    let tmp = dir.join(format!("{MANIFEST_FILENAME}.tmp"));
    let final_path = dir.join(MANIFEST_FILENAME);

    let Ok(json) = serde_json::to_string_pretty(entries) else {
        log::warn!("Failed to serialize autosave manifest");
        return;
    };
    if let Err(e) = std::fs::write(&tmp, json) {
        log::warn!("Failed to write autosave manifest tmp: {e}");
        return;
    }
    let _ = std::fs::remove_file(&final_path);
    if let Err(e) = std::fs::rename(&tmp, &final_path) {
        log::warn!("Failed to rename autosave manifest: {e}");
    }
}

/// Insert or replace a manifest entry (deduplicates by filename).
fn upsert_manifest_entry(entry: AutoSaveEntry) {
    let mut manifest = load_manifest();
    manifest.retain(|e| e.filename != entry.filename);
    manifest.push(entry);
    save_manifest(&manifest);
}

fn remove_manifest_entry(filename: &str) {
    let mut manifest = load_manifest();
    manifest.retain(|e| e.filename != filename);
    save_manifest(&manifest);
}

/// Scan for recoverable auto-save files on startup. Prunes entries older
/// than `STALE_THRESHOLD_SECS` and entries whose `.wz` files no longer exist.
pub fn scan_for_recovery() -> Vec<AutoSaveEntry> {
    let dir = crate::config::autosave_dir();
    let mut manifest = load_manifest();
    let now = unix_now();

    let before = manifest.len();
    manifest.retain(|e| {
        let age_ok = now.saturating_sub(e.timestamp) < STALE_THRESHOLD_SECS;
        let exists = dir.join(&e.filename).exists();
        age_ok && exists
    });

    if manifest.len() != before {
        save_manifest(&manifest);
    }

    manifest
}

/// Delete an auto-save file and remove its entry from the manifest.
pub fn discard_entry(entry: &AutoSaveEntry) {
    let dir = crate::config::autosave_dir();
    if let Err(e) = std::fs::remove_file(dir.join(&entry.filename)) {
        log::debug!("Could not remove auto-save file during discard: {e}");
    }
    remove_manifest_entry(&entry.filename);
}

/// Load a recovered map from an auto-save `.wz` file.
pub fn load_recovery(entry: &AutoSaveEntry) -> Result<wz_maplib::WzMap, String> {
    let path = crate::config::autosave_dir().join(&entry.filename);
    wz_maplib::io_wz::load_from_wz_archive(&path).map_err(|e| format!("{e}"))
}

/// Synchronous auto-save used on exit, where a background thread would be
/// killed before completing.
pub fn save_sync(map: &wz_maplib::WzMap, save_path: Option<&Path>, players: u8, session_id: &str) {
    let (wz_path, entry) = prepare_save(map, save_path, players, session_id);

    match wz_maplib::io_wz::save_to_wz_archive(map, &wz_path, wz_maplib::OutputFormat::Ver3) {
        Ok(()) => {
            upsert_manifest_entry(entry);
            log::info!("Auto-saved on exit to {}", wz_path.display());
        }
        Err(e) => log::error!("Auto-save on exit failed: {e}"),
    }
}

/// Build the auto-save file path and manifest entry for a map. Shared by
/// `start_save` and `save_sync`.
fn prepare_save(
    map: &wz_maplib::WzMap,
    save_path: Option<&Path>,
    players: u8,
    session_id: &str,
) -> (PathBuf, AutoSaveEntry) {
    let dir = crate::config::autosave_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("Failed to create autosave directory: {e}");
    }

    let sanitized = sanitize_name(&map.map_name);
    let filename = format!("autosave-{sanitized}-{session_id}.wz");
    let wz_path = dir.join(&filename);

    let entry = AutoSaveEntry {
        filename,
        map_name: map.map_name.clone(),
        original_path: save_path.map(Path::to_path_buf),
        timestamp: unix_now(),
        map_width: map.map_data.width,
        map_height: map.map_data.height,
        players,
    };

    (wz_path, entry)
}

/// Filename-safe form of a map name: alphanumeric + hyphen, max 50 chars.
fn sanitize_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(50)
        .collect();
    if s.is_empty() {
        "unnamed".to_string()
    } else {
        s
    }
}

fn unix_now() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Format a unix timestamp for display in the recovery dialog.
pub fn format_timestamp(ts: u64) -> String {
    let now = unix_now();
    let delta = now.saturating_sub(ts);
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        let mins = delta / 60;
        format!("{mins} min ago")
    } else if delta < 86400 {
        let hours = delta / 3600;
        format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" })
    } else {
        let days = delta / 86400;
        format!("{days} day{} ago", if days == 1 { "" } else { "s" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_name_preserves_alphanum_and_hyphens() {
        assert_eq!(sanitize_name("2c-Roughness"), "2c-Roughness");
    }

    #[test]
    fn sanitize_name_replaces_spaces_and_specials() {
        assert_eq!(sanitize_name("My Map (v2)"), "My-Map--v2-");
    }

    #[test]
    fn sanitize_name_truncates_at_50_chars() {
        let long = "a".repeat(100);
        assert_eq!(sanitize_name(&long).len(), 50);
    }

    #[test]
    fn sanitize_name_empty_returns_unnamed() {
        assert_eq!(sanitize_name(""), "unnamed");
    }

    #[test]
    fn sanitize_name_all_special_returns_unnamed() {
        // All characters replaced with '-', so result is non-empty.
        assert_eq!(sanitize_name("!@#"), "---");
    }

    #[test]
    fn format_timestamp_just_now() {
        let ts = unix_now();
        assert_eq!(format_timestamp(ts), "just now");
    }

    #[test]
    fn format_timestamp_minutes() {
        let ts = unix_now().saturating_sub(120);
        assert_eq!(format_timestamp(ts), "2 min ago");
    }

    #[test]
    fn format_timestamp_one_hour() {
        let ts = unix_now().saturating_sub(3600);
        assert_eq!(format_timestamp(ts), "1 hour ago");
    }

    #[test]
    fn format_timestamp_multiple_hours() {
        let ts = unix_now().saturating_sub(7200);
        assert_eq!(format_timestamp(ts), "2 hours ago");
    }

    #[test]
    fn format_timestamp_one_day() {
        let ts = unix_now().saturating_sub(86400);
        assert_eq!(format_timestamp(ts), "1 day ago");
    }

    #[test]
    fn format_timestamp_multiple_days() {
        let ts = unix_now().saturating_sub(3 * 86400);
        assert_eq!(format_timestamp(ts), "3 days ago");
    }

    #[test]
    fn new_state_has_unique_session_id() {
        let a = AutoSaveState::new();
        let b = AutoSaveState::new();
        assert_ne!(a.session_id, b.session_id);
    }

    #[test]
    fn should_save_respects_interval() {
        let state = AutoSaveState::new();
        assert!(!state.should_save(60));
        assert!(state.should_save(0));
    }

    #[test]
    fn reset_timer_prevents_immediate_save() {
        let mut state = AutoSaveState::new();
        state.last_save = Instant::now()
            .checked_sub(std::time::Duration::from_secs(2))
            .expect("Instant - 2s should not underflow");
        assert!(state.should_save(1));
        state.reset_timer();
        assert!(!state.should_save(1));
    }

    #[test]
    fn poll_returns_none_when_idle() {
        let mut state = AutoSaveState::new();
        assert!(state.poll().is_none());
    }

    #[test]
    fn default_matches_new() {
        let a = AutoSaveState::new();
        let b = AutoSaveState::default();
        // Each generates a random ID so equality can't be asserted.
        assert!(!a.session_id.is_empty());
        assert!(!b.session_id.is_empty());
        assert!(a.current_file.is_none());
        assert!(b.current_file.is_none());
    }

    #[test]
    fn manifest_round_trip() {
        let dir = tempfile::tempdir().expect("temp dir");
        let manifest_path = dir.path().join(MANIFEST_FILENAME);

        let entries = vec![AutoSaveEntry {
            filename: "autosave-test-abc12345.wz".to_string(),
            map_name: "TestMap".to_string(),
            original_path: Some(PathBuf::from("/tmp/test.wz")),
            timestamp: 1_700_000_000,
            map_width: 64,
            map_height: 64,
            players: 2,
        }];

        let tmp = dir.path().join(format!("{MANIFEST_FILENAME}.tmp"));
        let json = serde_json::to_string_pretty(&entries).unwrap();
        std::fs::write(&tmp, &json).unwrap();
        std::fs::rename(&tmp, &manifest_path).unwrap();

        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let loaded: Vec<AutoSaveEntry> = serde_json::from_str(&content).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].map_name, "TestMap");
        assert_eq!(loaded[0].map_width, 64);
        assert_eq!(loaded[0].players, 2);
    }

    #[test]
    fn autosave_entry_serialization() {
        let entry = AutoSaveEntry {
            filename: "autosave-map-12345678.wz".to_string(),
            map_name: "4c-Desert".to_string(),
            original_path: None,
            timestamp: 1_700_000_000,
            map_width: 128,
            map_height: 128,
            players: 4,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: AutoSaveEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.filename, entry.filename);
        assert_eq!(back.map_name, entry.map_name);
        assert!(back.original_path.is_none());
        assert_eq!(back.players, 4);
    }

    #[test]
    fn unix_now_returns_plausible_value() {
        // After 2024-01-01 (1_704_067_200).
        assert!(unix_now() > 1_704_067_200);
    }

    #[test]
    fn cleanup_on_empty_state_is_noop() {
        let mut state = AutoSaveState::new();
        assert!(state.current_file.is_none());
        state.cleanup();
    }

    #[test]
    fn default_interval_matches_constant() {
        assert_eq!(default_interval(), DEFAULT_INTERVAL_SECS);
        assert_eq!(DEFAULT_INTERVAL_SECS, 120);
    }

    #[test]
    fn prepare_save_builds_correct_filename() {
        let map = wz_maplib::WzMap::new("TestMap", 4, 4);
        let (path, entry) = prepare_save(&map, None, 2, "abcd1234");
        assert!(
            path.to_string_lossy()
                .contains("autosave-TestMap-abcd1234.wz")
        );
        assert_eq!(entry.map_name, "TestMap");
        assert_eq!(entry.players, 2);
        assert_eq!(entry.map_width, 4);
        assert!(entry.original_path.is_none());
    }

    #[test]
    fn prepare_save_includes_original_path() {
        let map = wz_maplib::WzMap::new("Map", 8, 8);
        let orig = Path::new("/some/path.wz");
        let (_, entry) = prepare_save(&map, Some(orig), 4, "00000000");
        assert_eq!(entry.original_path.as_deref(), Some(orig));
    }
}
