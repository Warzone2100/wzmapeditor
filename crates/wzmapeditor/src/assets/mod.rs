//! Read-only access to the WZ2100 data tree.
//!
//! The asset loaders read game-asset bytes (texture pages, PIE models,
//! ground-type metadata, the tile atlas) by path. [`AssetSource`] abstracts
//! where those bytes come from so the same loaders serve the native build
//! (files under the chosen data directory) and the web build (an in-memory
//! archive). Paths are relative to the data root the implementation owns,
//! e.g. `base/texpages/page-11.png`.

use std::path::{Path, PathBuf};

mod native;
pub(crate) use native::FsAssetSource;

#[cfg(any(target_arch = "wasm32", test))]
mod web;
#[cfg(target_arch = "wasm32")]
pub(crate) use web::{WebDataArchives, WebVfsAssetSource};

/// Read-only access to game-asset bytes by data-root-relative path.
///
/// Implementations own the data root; callers pass relative paths and never
/// learn whether the bytes come from disk or an in-memory archive.
pub(crate) trait AssetSource: Send + Sync {
    /// Read a file's raw bytes, or `None` if it is absent or unreadable.
    fn bytes(&self, rel: &Path) -> Option<Vec<u8>>;

    /// Read a file as UTF-8 text. `None` on absence or invalid UTF-8.
    fn text(&self, rel: &Path) -> Option<String> {
        self.bytes(rel).and_then(|b| String::from_utf8(b).ok())
    }

    /// Whether a file or directory exists at `rel`.
    fn exists(&self, rel: &Path) -> bool;

    /// Whether `rel` names a directory.
    fn is_dir(&self, rel: &Path) -> bool;

    /// Immediate children of directory `rel`, as data-root-relative paths.
    ///
    /// Order is unspecified. Returns an empty vec when `rel` is absent or is
    /// not a directory.
    fn read_dir(&self, rel: &Path) -> Vec<PathBuf>;
}
