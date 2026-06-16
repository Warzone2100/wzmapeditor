//! [`AssetSource`] backed by `.wz` archives held in memory.
//!
//! The web build has no filesystem to root [`FsAssetSource`](super::FsAssetSource)
//! at, so it keeps the user's chosen `.wz` archives in memory and serves asset
//! bytes straight from them. The layout mirrors the native extracted cache so
//! every loader works unchanged:
//!
//! - `base/<path>` resolves `<path>` inside `base.wz`, with `classic.wz`
//!   overlaid on top (an entry present in `classic.wz` wins), exactly as the
//!   native build extracts `terrain_overrides/classic.wz` over `base.wz`.
//! - `mp/<path>` resolves `<path>` inside `mp.wz`.
//!
//! Archive entries carry no `base/` or `mp/` prefix internally (they are
//! `texpages/...`, `stats/...`, and so on), so the leading prefix selects the

use std::collections::BTreeSet;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use wz_maplib::io_wz::WzArchiveReader;

use super::AssetSource;

type MemReader = WzArchiveReader<Cursor<Vec<u8>>>;

/// Raw `.wz` archive bytes the web build loads into a [`WebVfsAssetSource`].
#[derive(Debug, Default)]
pub(crate) struct WebDataArchives {
    /// `base.wz` bytes. Required; the source cannot be built without it.
    pub base: Vec<u8>,
    /// `terrain_overrides/classic.wz` bytes, overlaid on the `base/` prefix.
    pub classic: Option<Vec<u8>>,
    /// `mp.wz` bytes, mapped to the `mp/` prefix.
    pub mp: Option<Vec<u8>>,
}

/// One editor prefix (`base` or `mp`) backed by one or more overlaid archives.
#[derive(Debug)]
struct Layer {
    /// Archives in overlay order: the first one holding an entry wins.
    archives: Vec<Mutex<MemReader>>,
    /// Union of every entry path across the archives, normalized to forward
    /// slashes with no trailing slash. Answers metadata queries without
    /// locking or reading the archives.
    names: BTreeSet<String>,
}

impl Layer {
    /// Build a single-archive layer.
    fn new(reader: MemReader) -> Self {
        let names = normalized_names(&reader).collect();
        Self {
            archives: vec![Mutex::new(reader)],
            names,
        }
    }

    /// Add an archive whose entries take precedence over the existing ones.
    fn overlay(&mut self, reader: MemReader) {
        self.names.extend(normalized_names(&reader));
        self.archives.insert(0, Mutex::new(reader));
    }

    /// Read an entry, trying each archive in overlay order.
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        for archive in &self.archives {
            if let Ok(mut reader) = archive.lock()
                && let Some(bytes) = reader.read_entry(rel)
            {
                return Some(bytes);
            }
        }
        None
    }

    fn exists(&self, rel: &str) -> bool {
        rel.is_empty() || self.names.contains(rel) || self.is_dir(rel)
    }

    fn is_dir(&self, rel: &str) -> bool {
        if rel.is_empty() {
            return true;
        }
        let prefix = format!("{rel}/");
        self.names.iter().any(|n| n.starts_with(&prefix))
    }

    /// Immediate child segment names directly under `rel` (`""` is the root).
    fn children(&self, rel: &str) -> BTreeSet<String> {
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        let mut out = BTreeSet::new();
        for name in self.names.iter().filter(|n| n.starts_with(&prefix)) {
            let remainder = &name[prefix.len()..];
            if let Some(segment) = remainder.split('/').next()
                && !segment.is_empty()
            {
                out.insert(segment.to_owned());
            }
        }
        out
    }
}

/// In-memory, `.wz`-backed [`AssetSource`] for the web build.
///
/// Construct it with [`WebVfsAssetSource::from_archives`] from the bytes the
/// data-source picker reads out of the user's Warzone 2100 folder.
#[derive(Debug)]
pub(crate) struct WebVfsAssetSource {
    base: Layer,
    mp: Option<Layer>,
}

impl WebVfsAssetSource {
    /// Build a source from in-memory `.wz` archive bytes.
    ///
    /// `base` is mandatory; `classic` (when present) overlays the `base/`
    pub(crate) fn from_archives(archives: WebDataArchives) -> Option<Self> {
        let mut base = Layer::new(open_reader(base)?);
        if let Some(classic) = classic.and_then(open_reader) {
            base.overlay(classic);
        }
        let mp = mp.and_then(open_reader).map(Layer::new);
    }

    fn layer(&self, head: &str) -> Option<&Layer> {
        match head {
            "base" => Some(&self.base),
            "mp" => self.mp.as_ref(),
            _ => None,
        }
    }
}

impl AssetSource for WebVfsAssetSource {
    fn bytes(&self, rel: &Path) -> Option<Vec<u8>> {
        let (head, tail) = split_key(rel)?;
        if tail.is_empty() {
            return None;
        }
        self.layer(&head)?.read(&tail)
    }

    fn exists(&self, rel: &Path) -> bool {
    }

    fn is_dir(&self, rel: &Path) -> bool {
    }

    fn read_dir(&self, rel: &Path) -> Vec<PathBuf> {
        let Some((head, tail)) = split_key(rel) else {
            return Vec::new();
        };
        let Some(layer) = self.layer(&head) else {
            return Vec::new();
        };
            .into_iter()
            .map(|child| {
                let mut path = PathBuf::from(&head);
                if !tail.is_empty() {
                    path.push(&tail);
                }
                path.push(child);
                path
            })
            .collect()
    }
}

fn open_reader(bytes: Vec<u8>) -> Option<MemReader> {
    WzArchiveReader::from_reader(Cursor::new(bytes))
}

/// Entry names of `reader`, trailing slashes stripped and empties dropped.
fn normalized_names(reader: &MemReader) -> impl Iterator<Item = String> {
    reader.entry_names().into_iter().filter_map(|name| {
        let trimmed = name.trim_end_matches('/');
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

/// Split a data-root-relative path into its `(prefix, remainder)` parts.
///
/// Normalizes to forward slashes and drops `.` components. Returns `None` for
/// empty, absolute, or parent-escaping paths. The remainder is empty when
/// `rel` names a bare prefix such as `base`.
fn split_key(rel: &Path) -> Option<(String, String)> {
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(s) => parts.push(s.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    let (head, tail) = parts.split_first()?;
    Some(((*head).to_owned(), tail.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in entries {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap();
        }
        buf
    }

    fn sample() -> WebVfsAssetSource {
        let base = zip_with(&[
            ("texpages/page-1.png", b"base-page"),
            ("texpages/tile.png", b"base-tile"),
            ("stats/body.json", b"body"),
            ("components/bodies/viper.pie", b"viper"),
        ]);
        let classic = zip_with(&[("texpages/tile.png", b"classic-tile")]);
        let mp = zip_with(&[("stats/templates.json", b"mp-templates")]);
        WebVfsAssetSource::from_archives(WebDataArchives {
            base,
            classic: Some(classic),
            mp: Some(mp),
        })
        .expect("valid base archive")
    }

    #[test]
    fn from_archives_requires_valid_base() {
        assert!(
            WebVfsAssetSource::from_archives(WebDataArchives {
                base: b"not a zip".to_vec(),
                ..WebDataArchives::default()
            })
            .is_none()
        );
    }

    #[test]
    fn reads_base_entries_by_prefixed_path() {
        let vfs = sample();
        assert_eq!(
            vfs.bytes(Path::new("base/texpages/page-1.png")).as_deref(),
            Some(&b"base-page"[..])
        );
        assert_eq!(
            vfs.bytes(Path::new("base/stats/body.json")).as_deref(),
            Some(&b"body"[..])
        );
        assert!(vfs.bytes(Path::new("base/missing.png")).is_none());
    }

    #[test]
    fn classic_overlay_wins_over_base() {
        let vfs = sample();
        assert_eq!(
            vfs.bytes(Path::new("base/texpages/tile.png")).as_deref(),
            Some(&b"classic-tile"[..])
        );
        // An entry only in base.wz still resolves through the overlay.
        assert_eq!(
            vfs.bytes(Path::new("base/texpages/page-1.png")).as_deref(),
            Some(&b"base-page"[..])
        );
    }

    #[test]
    fn mp_prefix_reads_mp_archive() {
        let vfs = sample();
        assert_eq!(
            vfs.bytes(Path::new("mp/stats/templates.json")).as_deref(),
            Some(&b"mp-templates"[..])
        );
    }

    #[test]
    fn missing_mp_archive_yields_nothing() {
        let vfs = WebVfsAssetSource::from_archives(WebDataArchives {
            base: zip_with(&[("stats/body.json", b"body")]),
            ..WebDataArchives::default()
        })
        .expect("valid base");
        assert!(!vfs.exists(Path::new("mp")));
        assert!(!vfs.exists(Path::new("mp/stats/templates.json")));
        assert!(vfs.bytes(Path::new("mp/stats/templates.json")).is_none());
        assert!(vfs.read_dir(Path::new("mp")).is_empty());
    }

    #[test]
    fn exists_and_is_dir_distinguish_files_and_dirs() {
        let vfs = sample();
        assert!(vfs.exists(Path::new("base")));
        assert!(vfs.is_dir(Path::new("base")));
        assert!(vfs.exists(Path::new("base/texpages")));
        assert!(vfs.is_dir(Path::new("base/texpages")));
        assert!(vfs.exists(Path::new("base/texpages/tile.png")));
        assert!(!vfs.is_dir(Path::new("base/texpages/tile.png")));
        assert!(!vfs.exists(Path::new("base/nope")));
        assert!(!vfs.exists(Path::new("other/thing")));
    }

    #[test]
    fn read_dir_lists_relative_children() {
        let vfs = sample();
        let mut top = vfs.read_dir(Path::new("base"));
        top.sort();
        assert_eq!(
            top,
            vec![
                PathBuf::from("base/components"),
                PathBuf::from("base/stats"),
                PathBuf::from("base/texpages"),
            ]
        );

        let mut texpages = vfs.read_dir(Path::new("base/texpages"));
        texpages.sort();
        assert_eq!(
            texpages,
            vec![
                PathBuf::from("base/texpages/page-1.png"),
                PathBuf::from("base/texpages/tile.png"),
            ]
        );

        assert_eq!(
            vfs.read_dir(Path::new("base/components")),
            vec![PathBuf::from("base/components/bodies")]
        );
        assert!(vfs.read_dir(Path::new("base/does/not/exist")).is_empty());
    }

    #[test]
    fn text_reads_utf8_entries() {
        let vfs = sample();
        assert_eq!(
            vfs.text(Path::new("base/stats/body.json")).as_deref(),
            Some("body")
        );
    }

    #[test]
    fn rejects_absolute_and_parent_paths() {
        let vfs = sample();
        assert!(vfs.bytes(Path::new("/base/stats/body.json")).is_none());
        assert!(vfs.bytes(Path::new("../base/stats/body.json")).is_none());
        assert!(!vfs.exists(Path::new("/base")));
    }
}
