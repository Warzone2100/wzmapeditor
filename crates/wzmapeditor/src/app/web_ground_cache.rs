//! Browser Cache Storage persistence for decoded Remastered (HQ) ground and
//! decal texture layers (web build).
//!
//! Transcoding the HQ KTX2 textures (`ruzstd` + UASTC->RGBA) is the slowest
//! part of enabling Remastered terrain in the browser, and there is no worker
//! thread to hide it behind. To avoid re-running it on every reload, each
//! decoded RGBA layer is stored in Cache Storage keyed by tileset and source
//! filename -- the web analogue of the native `ground-cache` `.bin` files. A
//! later session reads the layers back and skips the transcode entirely. Cache
//! Storage is only available in secure contexts (HTTPS / localhost), which the
//! hosted build always is; on an insecure origin these helpers degrade to
//! no-ops, so the terrain still decodes -- just without persistence.

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use wasm_bindgen_futures::spawn_local;

use crate::config::Tileset;
use crate::web::cache;

/// Cache bucket for decoded ground layers. The `v1` suffix is the format
/// version: bump it when the decode/resize output changes so superseded layers
/// are never matched.
const CACHE_NAME: &str = "wz-ground-cache-v1";

/// Synthetic same-origin URL one decoded layer is cached under. `name` is the
/// source texture filename (e.g. `page-9.png`, `tile-03_nm.png`),
/// percent-encoded so it round-trips back via `decode_uri_component`.
fn layer_url(tileset: Tileset, name: &str) -> String {
    format!(
        "/__ground__/{}/{}",
        tileset.as_str(),
        cache::encode_component(name)
    )
}

/// Prefix shared by every layer of one tileset, used to filter enumeration.
fn tileset_prefix(tileset: Tileset) -> String {
    format!("/__ground__/{}/", tileset.as_str())
}

/// Persist one decoded RGBA layer. Fire-and-forget; any failure (including a
/// storage-quota overrun) is non-fatal -- it only costs a re-decode next
/// session. A re-uploaded pack overwrites layers in place: the key is stable
/// per (tileset, filename), so `put` replaces stale content.
pub(crate) fn save(tileset: Tileset, name: &str, bytes: Vec<u8>) {
    let url = layer_url(tileset, name);
    spawn_local(async move {
        if let Some(cache) = cache::open(CACHE_NAME).await {
            let _ = cache::put_bytes(&cache, &url, bytes).await;
        }
    });
}

/// Load every cached layer for `tileset` into a `name -> RGBA bytes` map and
/// send it over `tx` exactly once. Sends an empty map if the cache is
/// unavailable or holds nothing for the tileset, so the caller always receives
/// a single value to react to.
pub(crate) fn start_prefetch(tileset: Tileset, tx: Sender<HashMap<String, Vec<u8>>>) {
    spawn_local(async move {
        let _ = tx.send(prefetch(tileset).await);
    });
}

async fn prefetch(tileset: Tileset) -> HashMap<String, Vec<u8>> {
    let mut out = HashMap::new();
    let Some(cache) = cache::open(CACHE_NAME).await else {
        return out;
    };
    for (name, request) in cache::keys_with_prefix(&cache, &tileset_prefix(tileset)).await {
        let Some(response) = cache::match_request(&cache, &request).await else {
            continue;
        };
        if let Some(bytes) = cache::read_response_bytes(&response).await {
            out.insert(name, bytes);
        }
    }
    out
}
