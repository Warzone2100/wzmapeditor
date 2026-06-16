//! [`AssetSource`] backed by a directory on the native filesystem.

use std::path::{Path, PathBuf};

use super::AssetSource;

/// Reads assets from a directory tree rooted at `root`.
#[derive(Debug, Clone)]
pub(crate) struct FsAssetSource {
    root: PathBuf,
}

impl FsAssetSource {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl AssetSource for FsAssetSource {
    fn bytes(&self, rel: &Path) -> Option<Vec<u8>> {
        std::fs::read(self.root.join(rel)).ok()
    }

    fn exists(&self, rel: &Path) -> bool {
        self.root.join(rel).exists()
    }

    fn is_dir(&self, rel: &Path) -> bool {
        self.root.join(rel).is_dir()
    }

    fn read_dir(&self, rel: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.root.join(rel)) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                e.path()
                    .strip_prefix(&self.root)
                    .ok()
                    .map(Path::to_path_buf)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_text_and_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("base/tileset")).unwrap();
        std::fs::write(dir.path().join("base/tileset/g.txt"), "hello").unwrap();

        let src = FsAssetSource::new(dir.path().to_path_buf());
        assert_eq!(
            src.bytes(Path::new("base/tileset/g.txt")).as_deref(),
            Some(&b"hello"[..])
        );
        assert_eq!(
            src.text(Path::new("base/tileset/g.txt")).as_deref(),
            Some("hello")
        );
        assert!(src.exists(Path::new("base/tileset/g.txt")));
        assert!(!src.exists(Path::new("base/tileset/missing.txt")));
        assert!(src.bytes(Path::new("missing")).is_none());
    }

    #[test]
    fn is_dir_distinguishes_files_and_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("base/components")).unwrap();
        std::fs::write(dir.path().join("base/components/a.pie"), "x").unwrap();

        let src = FsAssetSource::new(dir.path().to_path_buf());
        assert!(src.is_dir(Path::new("base/components")));
        assert!(!src.is_dir(Path::new("base/components/a.pie")));
        assert!(!src.is_dir(Path::new("base/components/missing")));
    }

    #[test]
    fn read_dir_returns_relative_children() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("base/components/sub")).unwrap();
        std::fs::write(dir.path().join("base/components/a.pie"), "x").unwrap();

        let src = FsAssetSource::new(dir.path().to_path_buf());
        let mut children = src.read_dir(Path::new("base/components"));
        children.sort();
        assert_eq!(
            children,
            vec![
                PathBuf::from("base/components/a.pie"),
                PathBuf::from("base/components/sub"),
            ]
        );
        assert!(src.read_dir(Path::new("does/not/exist")).is_empty());
    }
}
