//! Web map open/save: file-picker open and browser-download save.
//!
//! The native build opens and saves maps through `rfd` dialogs backed by the
//! filesystem. The browser has neither, so this module opens a `.wz` with a
//! single-file `<input>` (read asynchronously into memory, then parsed by
//! [`wz_maplib::io_wz::load_from_wz_reader`]) and "saves" by serializing the
//! map to an in-memory buffer and triggering a browser download.
//!
//! Like [`web_data`](crate::web_data), the asynchronous read delivers its
//! bytes over a channel that [`poll`] drains each frame; the download path is
//! synchronous and runs inline from the save flow.

use std::io::Cursor;
use std::sync::mpsc;

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::app::EditorApp;
use crate::web::{Drain, dom, drain_once};
use crate::web_data::read_file_bytes;

/// A picked `.wz` file: its name stem and raw bytes.
pub(crate) struct PickedMap {
    name_hint: String,
    bytes: Vec<u8>,
}

/// Open a single-file picker and route the chosen `.wz` into `app`.
pub(crate) fn begin_open(app: &mut EditorApp, ctx: &egui::Context) {
    let (tx, rx) = mpsc::channel();
    app.rt.web_open_map_rx = Some(rx);
    let ctx = ctx.clone();
    dom::pick_file(".wz", move |file| match file {
        Some(file) => spawn_local(async move {
            let _ = tx.send(read_picked(file).await);
            // Wake egui so `poll` runs; nothing else repaints here.
            ctx.request_repaint();
        }),
        None => {
            // Picker dismissed: drop `tx` so `poll` clears the latch next frame.
            ctx.request_repaint();
        }
    });
}

/// Drain the open-map channel; load the map or report the error.
pub(crate) fn poll(app: &mut EditorApp, ctx: &egui::Context) {
    // The picker's own callback repaints when it has a result, so this poller
    // does not need to keep the frame loop awake.
    let outcome = match drain_once(&mut app.rt.web_open_map_rx, ctx, false) {
        Drain::Ready(outcome) => outcome,
        Drain::Pending | Drain::Closed => return,
    };
    match outcome {
        Ok(picked) => load(app, picked),
        Err(msg) => {
            log::warn!("{msg}");
            app.log(msg);
        }
    }
}

fn load(app: &mut EditorApp, picked: PickedMap) {
    let PickedMap { name_hint, bytes } = picked;
    // Parse from a borrowed slice so the owned bytes survive for caching.
    match wz_maplib::io_wz::load_from_wz_reader(Cursor::new(bytes.as_slice()), &name_hint) {
        Ok(map) => {
            // No filesystem path exists on the web; synthesize a save path from
            // the map name so a later Ctrl+S re-downloads without re-prompting.
            let save = std::path::PathBuf::from(format!("{name_hint}.wz"));
            app.load_map(map, None, Some(save), None);
            crate::web_data::cache_last_map(&name_hint, bytes);
        }
        Err(e) => {
            let path = std::path::PathBuf::from(format!("{name_hint}.wz"));
            app.report_wz_load_error(&path, &e);
        }
    }
}

/// Reopen the last map, restored from Cache Storage at startup.
///
/// Unlike [`load`], a parse failure is swallowed — a stale or corrupt cache
/// entry must not block boot — and the bytes are not written back to the cache.
pub(crate) fn restore(app: &mut EditorApp, name_hint: &str, bytes: &[u8]) {
    match wz_maplib::io_wz::load_from_wz_reader(Cursor::new(bytes), name_hint) {
        Ok(map) => {
            let save = std::path::PathBuf::from(format!("{name_hint}.wz"));
            app.load_map(map, None, Some(save), None);
            app.log(format!("Restored last map: {name_hint}"));
        }
        Err(e) => log::warn!("Discarding unreadable cached map '{name_hint}': {e}"),
    }
}

/// Read the picked file's bytes and derive its name stem.
async fn read_picked(file: web_sys::File) -> Result<PickedMap, String> {
    let name = file.name();
    let name_hint = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _ext)| stem)
        .to_string();
    let bytes = read_file_bytes(&file).await?;
    Ok(PickedMap { name_hint, bytes })
}

/// Trigger a browser download of `bytes` under `filename`.
pub(crate) fn download(filename: &str, bytes: &[u8]) -> Result<(), String> {
    let window = web_sys::window().ok_or("No browser window available.")?;
    let document = window.document().ok_or("No browser document available.")?;

    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).map_err(|e| format!("{e:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))?;

    let anchor = document
        .create_element("a")
        .ok()
        .and_then(|el| el.dyn_into::<web_sys::HtmlAnchorElement>().ok())
        .ok_or("Could not create a download anchor.")?;
    anchor.set_href(&url);
    anchor.set_download(filename);

    // Firefox only fires the download for an anchor attached to the document.
    if let Some(body) = document.body() {
        let _ = body.append_child(&anchor);
        anchor.click();
        let _ = body.remove_child(&anchor);
    } else {
        anchor.click();
    }

    // The object URL is intentionally not revoked: revoking before the browser
    // reads the blob cancels the download, and saves are infrequent enough that
    // each per-save URL lives harmlessly until the page unloads.
    Ok(())
}
