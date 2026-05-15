//! Editor document wrapping a `WzMap` with undo history.

use wz_maplib::WzMap;

use crate::map::history::EditHistory;

/// Wraps a `WzMap` with editor state (undo history, selection, dirty flag).
pub struct MapDocument {
    pub map: WzMap,
    pub history: EditHistory,
    pub dirty: bool,
    /// `true` when the map was produced by `run_script_map` and edits must
    /// be refused.
    pub read_only: bool,
    /// Seed used to generate this map, when `read_only` is set.
    pub script_seed: Option<u32>,
    /// Source `.wz` path of the script map; needed by the Re-roll Seed
    /// toolbar action.
    pub script_source: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for MapDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapDocument")
            .field("map_name", &self.map.map_name)
            .field(
                "map_size",
                &format_args!("{}x{}", self.map.map_data.width, self.map.map_data.height),
            )
            .finish_non_exhaustive()
    }
}

impl MapDocument {
    pub fn new(map: WzMap) -> Self {
        Self {
            map,
            history: EditHistory::new(),
            dirty: false,
            read_only: false,
            script_seed: None,
            script_source: None,
        }
    }

    /// Returns whether the replayed command dirties object instance buffers.
    pub fn undo(&mut self) -> bool {
        let dirties_objects = self.history.undo(&mut self.map);
        self.dirty = true;
        dirties_objects
    }

    /// Returns whether the replayed command dirties object instance buffers.
    pub fn redo(&mut self) -> bool {
        let dirties_objects = self.history.redo(&mut self.map);
        self.dirty = true;
        dirties_objects
    }

    /// Mark the document as cleanly saved. Auto-save preserves the dirty
    /// state so the user still sees unsaved changes.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Whether mutations to the document must be refused. Set by the
    /// script-map import path; cleared on every other code path.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_map() -> WzMap {
        WzMap::new("test", 4, 4)
    }

    #[test]
    fn new_document_is_clean() {
        let doc = MapDocument::new(test_map());
        assert!(!doc.dirty);
    }

    #[test]
    fn mark_clean_clears_dirty() {
        let mut doc = MapDocument::new(test_map());
        doc.dirty = true;
        doc.mark_clean();
        assert!(!doc.dirty);
    }

    #[test]
    fn undo_sets_dirty() {
        let mut doc = MapDocument::new(test_map());
        doc.undo();
        assert!(doc.dirty);
    }

    #[test]
    fn redo_sets_dirty() {
        let mut doc = MapDocument::new(test_map());
        doc.redo();
        assert!(doc.dirty);
    }

    #[test]
    fn mark_clean_then_undo_is_dirty() {
        let mut doc = MapDocument::new(test_map());
        doc.dirty = true;
        doc.mark_clean();
        assert!(!doc.dirty);
        doc.undo();
        assert!(doc.dirty);
    }
}
