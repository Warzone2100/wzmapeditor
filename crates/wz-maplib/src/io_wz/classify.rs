//! Classify a `.wz` archive by inspecting its zip entry list.
//!
//! The `.wz` extension covers editable map archives, script maps whose
//! terrain is generated at runtime by `game.js`, multi-map packs under
//! per-map prefixes, and the occasional unrelated zip. `classify_wz_archive`
//! returns a [`WzArchiveKind`] so callers can render a specific message
//! for each shape without parsing the binary payloads.

use std::path::Path;

/// What kind of `.wz` content the user opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WzArchiveKind {
    /// Editable static map: contains a single `game.map`.
    Map,
    /// Pack containing multiple maps under prefixed folders (e.g. `mp.wz`).
    MultiMapPack { map_count: usize },
    /// Script map: terrain is generated at runtime by `game.js`, so there
    /// is no `game.map` for the editor to load.
    ScriptMap,
    /// Zip opened but contained zero entries.
    Empty,
    /// File could not be opened as a zip (not an archive, or corrupt).
    NotAnArchive,
    /// Zip opened cleanly but doesn't match any known map shape.
    Unknown,
}

impl WzArchiveKind {
    /// Whether wzmapeditor can load this archive as an editable map.
    pub fn is_editable(&self) -> bool {
        matches!(self, Self::Map | Self::MultiMapPack { .. })
    }
}

/// Classify a `.wz` file by peeking at its zip entry list.
///
/// Never returns an error: file-open and zip-parse failures both map to
/// [`WzArchiveKind::NotAnArchive`], so callers can use this on an error
/// path without nested matching.
pub fn classify_wz_archive(path: &Path) -> WzArchiveKind {
    let Ok(file) = std::fs::File::open(path) else {
        return WzArchiveKind::NotAnArchive;
    };
    let Ok(archive) = zip::ZipArchive::new(file) else {
        return WzArchiveKind::NotAnArchive;
    };
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.name_for_index(i).map(str::to_owned))
        .collect();
    classify_entry_names(&names)
}

fn classify_entry_names(names: &[String]) -> WzArchiveKind {
    if names.is_empty() {
        return WzArchiveKind::Empty;
    }
    let game_map_count = names.iter().filter(|n| n.ends_with("game.map")).count();
    if game_map_count > 1 {
        return WzArchiveKind::MultiMapPack {
            map_count: game_map_count,
        };
    }
    if game_map_count == 1 {
        return WzArchiveKind::Map;
    }
    if names.iter().any(|n| n.ends_with("game.js")) {
        return WzArchiveKind::ScriptMap;
    }
    WzArchiveKind::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn empty_archive_is_empty() {
        assert_eq!(classify_entry_names(&[]), WzArchiveKind::Empty);
    }

    #[test]
    fn single_game_map_is_map() {
        let names = s(&["game.map", "level.json", "ttypes.ttp"]);
        assert_eq!(classify_entry_names(&names), WzArchiveKind::Map);
    }

    #[test]
    fn prefixed_single_map_is_still_map() {
        let names = s(&[
            "multiplay/maps/2c-Foo/game.map",
            "multiplay/maps/2c-Foo/level.json",
        ]);
        assert_eq!(classify_entry_names(&names), WzArchiveKind::Map);
    }

    #[test]
    fn multiple_game_maps_is_pack() {
        let names = s(&[
            "multiplay/maps/2c-A/game.map",
            "multiplay/maps/4c-B/game.map",
            "multiplay/maps/8c-C/game.map",
        ]);
        assert_eq!(
            classify_entry_names(&names),
            WzArchiveKind::MultiMapPack { map_count: 3 }
        );
    }

    #[test]
    fn game_js_without_game_map_is_script_map() {
        let names = s(&["game.js", "gam.json", "level.json", "ttypes.ttp"]);
        assert_eq!(classify_entry_names(&names), WzArchiveKind::ScriptMap);
    }

    #[test]
    fn no_known_markers_is_unknown() {
        let names = s(&["texpages/page-0.png", "stats/templates.json"]);
        assert_eq!(classify_entry_names(&names), WzArchiveKind::Unknown);
    }

    #[test]
    fn is_editable_matches_expected_kinds() {
        assert!(WzArchiveKind::Map.is_editable());
        assert!(WzArchiveKind::MultiMapPack { map_count: 2 }.is_editable());
        assert!(!WzArchiveKind::ScriptMap.is_editable());
        assert!(!WzArchiveKind::Empty.is_editable());
        assert!(!WzArchiveKind::NotAnArchive.is_editable());
        assert!(!WzArchiveKind::Unknown.is_editable());
    }
}
