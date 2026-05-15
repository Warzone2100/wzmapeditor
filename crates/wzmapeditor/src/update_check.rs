//! Background check for newer wzmapeditor releases on GitHub.
//!
//! Spawns a worker thread on app launch that consults a 24h on-disk cache
//! and, if stale, queries the public GitHub Releases API. If a strictly
//! newer semver tag is found, an [`UpdateInfo`] is posted back through an
//! `mpsc` channel for the UI thread to surface. Failures are silent: a
//! flaky network does not block the editor or notify the user.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const RELEASES_URL: &str = "https://api.github.com/repos/Warzone2100/wzmapeditor/releases/latest";

/// Skip the network call if we already checked within this window.
const CACHE_TTL: Duration = Duration::from_hours(24);

/// Newer release the user could upgrade to.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub latest: String,
    pub html_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Cache {
    checked_at_unix: u64,
    latest: String,
    html_url: String,
}

/// Kick off the check on a background thread and return the receiver.
///
/// The channel yields at most one message and then closes when the worker
/// exits. The caller should drop the receiver once it has produced a value.
pub fn spawn_check() -> mpsc::Receiver<UpdateInfo> {
    let (tx, rx) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            if let Some(info) = check() {
                let _ = tx.send(info);
            }
        });
    if let Err(e) = spawned {
        log::warn!("Failed to spawn update-check thread: {e}");
    }
    rx
}

fn check() -> Option<UpdateInfo> {
    let (latest, html_url) = if let Some(c) = fresh_cache() {
        (c.latest, c.html_url)
    } else {
        let (latest, html_url) = fetch_latest()?;
        write_cache(&latest, &html_url);
        (latest, html_url)
    };

    let current = semver::Version::parse(env!("CARGO_PKG_VERSION")).ok()?;
    let parsed = semver::Version::parse(latest.trim_start_matches('v')).ok()?;
    (parsed > current).then_some(UpdateInfo { latest, html_url })
}

fn fresh_cache() -> Option<Cache> {
    let cache = read_cache()?;
    let checked_at = UNIX_EPOCH + Duration::from_secs(cache.checked_at_unix);
    let age = SystemTime::now().duration_since(checked_at).ok()?;
    (age < CACHE_TTL).then_some(cache)
}

fn fetch_latest() -> Option<(String, String)> {
    log::info!("Checking for editor updates: {RELEASES_URL}");
    let user_agent = format!("wzmapeditor/{}", env!("CARGO_PKG_VERSION"));
    let body = match ureq::get(RELEASES_URL)
        .header("User-Agent", &user_agent)
        .call()
    {
        Ok(mut response) => match response.body_mut().read_to_vec() {
            Ok(b) => b,
            Err(e) => {
                log::warn!("Update check: failed to read response body: {e}");
                return None;
            }
        },
        Err(e) => {
            log::warn!("Update check: HTTP request failed: {e}");
            return None;
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Update check: JSON parse failed: {e}");
            return None;
        }
    };
    let latest = value.get("tag_name")?.as_str()?.to_owned();
    let html_url = value.get("html_url")?.as_str()?.to_owned();
    Some((latest, html_url))
}

fn cache_path() -> PathBuf {
    crate::config::config_dir().join("update-cache.json")
}

fn read_cache() -> Option<Cache> {
    let bytes = std::fs::read(cache_path()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_cache(latest: &str, html_url: &str) {
    let checked_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let cache = Cache {
        checked_at_unix,
        latest: latest.to_owned(),
        html_url: html_url.to_owned(),
    };
    let Ok(bytes) = serde_json::to_vec_pretty(&cache) else {
        return;
    };
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, bytes) {
        log::warn!("Failed to write update cache: {e}");
    }
}
