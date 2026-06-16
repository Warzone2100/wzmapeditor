//!
//! The native build points [`AssetSource`](crate::assets::AssetSource) at a
//!


use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::app::{EditorApp, StartupPhase};
use crate::assets::{WebDataArchives, WebVfsAssetSource};

const BASE_WZ: &str = "base.wz";
const MP_WZ: &str = "mp.wz";
const CLASSIC_WZ: &str = "terrain_overrides/classic.wz";

///
    let (tx, rx) = mpsc::channel();
    app.rt.web_data_rx = Some(rx);
}

    };
    match outcome {
        Err(msg) => set_setup_error(app, &msg),
    }
}

fn apply(app: &mut EditorApp, archives: WebDataArchives) {
    let Some(vfs) = WebVfsAssetSource::from_archives(archives) else {
        return;
    };

    app.assets = Some(assets);
    // Sentinel root: only its `is_some()`-ness gates the per-frame auto-loads;
    // the bytes come from the VFS, and the path is never read on the web build.
    app.config.data_dir = Some(std::path::PathBuf::from("/web-data"));
    app.config.setup_complete = true;
    app.has_hq_textures = false;

    app.rt.tileset_load_attempted = false;
    app.rt.stats_load_attempted = false;
    app.rt.ground_precache_attempted = false;
    app.stats = None;
    app.tileset = None;
    app.ground_data = None;
    app.model_loader = None;

    app.startup_phase = StartupPhase::Ready;
}

fn set_setup_error(app: &mut EditorApp, msg: &str) {
    log::warn!("{msg}");
    app.log(msg.to_string());
    if let StartupPhase::Setup { error, .. } = &mut app.startup_phase {
        *error = Some(msg.to_string());
    }
}

    };
        .ok()
        return;
    };
        spawn_local(async move {
        });
    }

}

/// Read a `File`'s bytes via the asynchronous `Blob.arrayBuffer()` API.
    let buffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(|e: JsValue| format!("{e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
