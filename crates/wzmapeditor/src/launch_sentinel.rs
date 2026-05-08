//! Sentinel file marking an in-progress launch. Cleared after the first
//! frame is painted; if it survives a launch the previous run died during
//! GPU init (e.g. DXC shader compile with missing `dxil.dll`) and we can
//! fall back to a different backend on the next try.

use crate::config::GraphicsBackend;
use std::path::PathBuf;

fn sentinel_path() -> PathBuf {
    crate::config::config_dir().join(".last_launch_failed")
}

/// Read the sentinel left by a prior launch (if any) and remove it.
pub fn consume() -> Option<GraphicsBackend> {
    let path = sentinel_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    match data.trim() {
        "Dx12" => Some(GraphicsBackend::Dx12),
        "Vulkan" => Some(GraphicsBackend::Vulkan),
        "OpenGl" => Some(GraphicsBackend::OpenGl),
        _ => None,
    }
}

/// Mark the next launch as "in progress with this backend". Cleared by
/// `disarm()` once the first frame is painted.
pub fn arm(backend: GraphicsBackend) {
    let path = sentinel_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let label = match backend {
        GraphicsBackend::Dx12 => "Dx12",
        GraphicsBackend::Vulkan => "Vulkan",
        GraphicsBackend::OpenGl => "OpenGl",
    };
    let _ = std::fs::write(&path, label);
}

/// Clear the sentinel; called after the first successful frame.
pub fn disarm() {
    let _ = std::fs::remove_file(sentinel_path());
}
