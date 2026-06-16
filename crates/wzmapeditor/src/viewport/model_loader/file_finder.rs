//! Filesystem search for PIE files and texture pages.
//!
//! PIE files live across many subdirectories under `base/` and `mp/`.
//! The sync loader walks a known-dirs list and falls back to a recursive
//! search; the background loader prebuilds a case-insensitive index for
//! O(1) lookups per parse.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::assets::AssetSource;

/// Subdirectories under `data_dir` searched by the synchronous PIE loader.
pub(crate) const PIE_SEARCH_DIRS: &[&str] = &[
    "base/components/prop",
    "base/components/bodies",
    "base/components/weapons",
    "base/components/turrets",
    "base/structs",
    "base/features",
    "base/misc",
    "base/effects",
    "base/components",
    "mp/components/bodies",
    "mp/components/weapons",
    "mp/components/turrets",
    "mp/structs",
    "mp/effects",
    "mp/components",
];

/// Find a PIE file by name under `data_dir`. Walks the known-dirs list
/// first, then falls back to a depth-limited recursive search rooted at
/// `base/` and `mp/`.
pub(crate) fn find_pie_file(data_dir: &Path, imd_name: &str) -> Option<PathBuf> {
    for dir in PIE_SEARCH_DIRS {
        let path = data_dir.join(dir).join(imd_name);
        if path.exists() {
            return Some(path);
        }
    }

    for subdir in ["base", "mp"] {
        let search_root = data_dir.join(subdir);
        if let Some(found) = find_file_recursive(&search_root, imd_name) {
            return Some(found);
        }
    }

    None
}

/// Recursive file search rooted at `dir_rel`. First match wins; depth is
/// capped to avoid scanning the entire data tree.
fn find_file_recursive(
    assets: &dyn AssetSource,
    dir_rel: &Path,
    filename: &str,
) -> Option<PathBuf> {
    find_file_recursive_depth(assets, dir_rel, filename, 5)
}

fn find_file_recursive_depth(
    assets: &dyn AssetSource,
    dir_rel: &Path,
    filename: &str,
    max_depth: u32,
) -> Option<PathBuf> {
    if max_depth == 0 {
        return None;
    }

    for child in assets.read_dir(dir_rel) {
        if !assets.is_dir(&child)
            && child
                .file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(filename))
        {
            return Some(child);
        }
        if assets.is_dir(&child)
            && let Some(found) = find_file_recursive_depth(assets, &child, filename, max_depth - 1)
        {
            return Some(found);
        }
    }

    None
}

/// Case-insensitive filename to path index for all `.pie`, `.png`, and
/// `.ktx2` files under `data_dir/base/` and `data_dir/mp/`. Scanning
/// once avoids per-model recursive searches, which otherwise dominate
/// load time on large maps.
pub(crate) fn build_pie_file_index(assets: &dyn AssetSource) -> HashMap<String, PathBuf> {
    let mut index = HashMap::new();
    for subdir in ["base", "mp"] {
        let root = Path::new(subdir);
        if assets.is_dir(root) {
            index_directory_recursive(assets, root, &mut index, 6);
        }
    }
    let tcmask_count = index.keys().filter(|k| k.contains("tcmask")).count();
    log::info!(
        "File index: {} entries ({} tcmask files)",
        index.len(),
        tcmask_count
    );
    index
}

fn index_directory_recursive(
    assets: &dyn AssetSource,
    dir_rel: &Path,
    index: &mut HashMap<String, PathBuf>,
    depth: u32,
) {
    if depth == 0 {
        return;
    }
    for child in assets.read_dir(dir_rel) {
        if assets.is_dir(&child) {
            index_directory_recursive(assets, &child, index, depth - 1);
        } else if let Some(name) = child.file_name() {
            let name_str = name.to_string_lossy();
            let ext = child.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext.eq_ignore_ascii_case("pie")
                || ext.eq_ignore_ascii_case("png")
                || ext.eq_ignore_ascii_case("ktx2")
            {
                index
                    .entry(name_str.to_string())
                    .or_insert_with(|| child.clone());
                let lower = name_str.to_lowercase();
                if lower != name_str.as_ref() {
                    index.entry(lower).or_insert(child);
                }
            }
        }
    }
}

/// Look up a PIE file in the prebuilt index. Tries exact case first,
/// then lowercase.
pub(crate) fn lookup_in_index<'a>(
    file_index: &'a HashMap<String, PathBuf>,
    imd_name: &str,
) -> Option<&'a PathBuf> {
    file_index
        .get(imd_name)
        .or_else(|| file_index.get(&imd_name.to_lowercase()))
}
