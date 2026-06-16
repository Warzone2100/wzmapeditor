//! Browser Cache Storage persistence for generated thumbnails (web build).
//!
//! The web build renders model/structure/feature thumbnails on demand in the
//! asset browser. Each finished thumbnail is encoded to PNG and stored in
//! Cache Storage, keyed by tileset and cache key, so a later session restores
//! it instead of re-rendering. Cache Storage is only available in secure
//! contexts (HTTPS / localhost), which the hosted build always is; on an
//! insecure origin these helpers degrade to no-ops.

use std::cell::Cell;
use std::io::Cursor;
use std::sync::mpsc::Sender;

use wasm_bindgen_futures::spawn_local;

use super::CACHE_VERSION;
use crate::web::cache;

const CACHE_NAME: &str = "wz-thumb-cache";

thread_local! {
    /// Off-version entries are pruned once per session, lazily on first load.
    static PRUNED: Cell<bool> = const { Cell::new(false) };
}

/// Synthetic same-origin URL a thumbnail is cached under. The key is
/// percent-encoded so it round-trips back via `decode_uri_component`.
fn thumb_url(tileset: &str, key: &str) -> String {
    format!(
        "/__thumb__/v{CACHE_VERSION}/{tileset}/{}",
        cache::encode_component(key)
    )
}

/// Prefix shared by every entry of one tileset, used to filter enumeration.
fn tileset_prefix(tileset: &str) -> String {
    format!("/__thumb__/v{CACHE_VERSION}/{tileset}/")
}

/// Encode an egui image to PNG bytes.
fn encode_png(image: &egui::ColorImage) -> Option<Vec<u8>> {
    let w = u32::try_from(image.size[0]).ok()?;
    let h = u32::try_from(image.size[1]).ok()?;
    let pixels: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(egui::Color32::to_array)
        .collect();
    let buf = image::RgbaImage::from_raw(w, h, pixels)?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// Decode PNG bytes into an egui image.
fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        &img.into_raw(),
    ))
}

/// Persist a rendered thumbnail. Fire-and-forget; failure is non-fatal.
pub(super) fn save_async(tileset: &str, key: &str, image: &egui::ColorImage) {
    let Some(png) = encode_png(image) else {
        return;
    };
    let url = thumb_url(tileset, key);
    spawn_local(async move {
        if let Some(cache) = cache::open(CACHE_NAME).await {
            let _ = cache::put_bytes(&cache, &url, png).await;
        }
    });
}

/// Start loading every cached thumbnail for `tileset`, sending `(key, image)`
/// pairs over `tx` as they decode. Runs on the microtask queue.
pub(super) fn start_load(
    tileset: String,
    tx: Sender<(String, egui::ColorImage)>,
    ctx: egui::Context,
) {
    prune_old_versions_once();
    spawn_local(async move {
        load_all(&tileset, &tx, &ctx).await;
    });
}

/// Delete thumbnail entries left behind by a previous [`CACHE_VERSION`]. Runs at
/// most once per session; without it bumping the version orphans the old
/// version's entries forever (the loader only ever reads the current prefix).
fn prune_old_versions_once() {
    if PRUNED.with(|p| p.replace(true)) {
        return;
    }
    spawn_local(async {
        let Some(cache) = cache::open(CACHE_NAME).await else {
            return;
        };
        let keep = format!("{CACHE_VERSION}/");
        // `keys_with_prefix` returns the URL tail after `/__thumb__/v`, i.e.
        // "{version}/{tileset}/{key}"; anything not on the current version goes.
        for (tail, request) in cache::keys_with_prefix(&cache, "/__thumb__/v").await {
            if !tail.starts_with(&keep) {
                cache::delete_request(&cache, &request).await;
            }
        }
    });
}

async fn load_all(tileset: &str, tx: &Sender<(String, egui::ColorImage)>, ctx: &egui::Context) {
    let Some(cache) = cache::open(CACHE_NAME).await else {
        return;
    };
    let mut delivered = 0u32;
    for (key, request) in cache::keys_with_prefix(&cache, &tileset_prefix(tileset)).await {
        let Some(response) = cache::match_request(&cache, &request).await else {
            continue;
        };
        let Some(bytes) = cache::read_response_bytes(&response).await else {
            continue;
        };
        if let Some(image) = decode_png(&bytes)
            && tx.send((key, image)).is_ok()
        {
            delivered += 1;
            // Repaint periodically so restored thumbnails appear without
            // waiting for the whole cache to decode.
            if delivered.is_multiple_of(8) {
                ctx.request_repaint();
            }
        }
    }
    if delivered > 0 {
        ctx.request_repaint();
    }
}
