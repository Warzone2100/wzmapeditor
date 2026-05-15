//! Shared types for the application module.

use egui_dock::{DockState, NodeIndex};

/// Tab types for the dockable panel system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DockTab {
    /// 3D viewport - always present, not closable.
    Viewport,
    Terrain,
    TilesetBrowser,
    AssetBrowser,
    Properties,
    Minimap,
    Hierarchy,
    Validation,
    OutputLog,
    Balance,
    /// Catch-all for removed or unknown tab variants in saved configs;
    /// keeps deserialization working across version skew.
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for DockTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Viewport => f.write_str("Viewport"),
            Self::Terrain => f.write_str("Terrain"),
            Self::TilesetBrowser => f.write_str("Tileset"),
            Self::AssetBrowser => f.write_str("Assets"),
            Self::Properties => f.write_str("Selection"),
            Self::Minimap => f.write_str("Minimap"),
            Self::Hierarchy => f.write_str("Hierarchy"),
            Self::Validation => f.write_str("Problems"),
            Self::OutputLog => f.write_str("Output"),
            Self::Balance => f.write_str("Balance"),
            Self::Unknown => f.write_str("Unknown"),
        }
    }
}

/// Build the project's standard dock layout. Programmatic construction
/// avoids bit-rot from `egui_dock` version bumps invalidating serialized
/// snapshots.
///
/// Layout (left to right, top to bottom):
/// - Far left column: Terrain on top, Tileset on bottom.
/// - Center top: 3D viewport.
/// - Center bottom: Assets | Output | Problems side-by-side.
/// - Right of center: Hierarchy.
/// - Far right column: Minimap on top, Properties+Balance tabbed below.
pub fn default_dock_layout() -> DockState<DockTab> {
    let mut dock = DockState::new(vec![DockTab::Viewport]);
    let surface = dock.main_surface_mut();

    // Carve the right column off first so all later splits operate on the
    // viewport's own subtree.
    let [center_block, right_column] =
        surface.split_right(NodeIndex::root(), 0.85, vec![DockTab::Minimap]);
    surface.split_below(
        right_column,
        0.31,
        vec![DockTab::Properties, DockTab::Balance],
    );

    // Hierarchy column to the right of the viewport.
    let [center_block, _hierarchy] =
        surface.split_right(center_block, 0.85, vec![DockTab::Hierarchy]);

    // Bottom strip below viewport+hierarchy: Assets | Output | Problems
    // split into three equal columns.
    let [center_block, bottom] =
        surface.split_below(center_block, 0.7, vec![DockTab::AssetBrowser]);
    let [_assets, output_and_problems] =
        surface.split_right(bottom, 1.0 / 3.0, vec![DockTab::OutputLog]);
    surface.split_right(output_and_problems, 0.5, vec![DockTab::Validation]);

    // Far-left tool column: Terrain on top, Tileset below. `split_left`
    // returns `[old, new]`; `new` is the left-side leaf containing Terrain.
    let [_viewport, terrain] = surface.split_left(center_block, 0.18, vec![DockTab::Terrain]);
    surface.split_below(terrain, 0.5, vec![DockTab::TilesetBrowser]);

    dock
}

/// Where existing tiles land in the resized map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl ResizeAnchor {
    /// Returns `(offset_x, offset_y)` in tile units, where `offset` is the
    /// source coordinate that lands at the resized map's `(0, 0)`.
    #[must_use]
    pub fn offset(self, old_size: (u32, u32), new_size: (u32, u32)) -> (i32, i32) {
        let dw = new_size.0 as i32 - old_size.0 as i32;
        let dh = new_size.1 as i32 - old_size.1 as i32;
        let half_dw = dw / 2;
        let half_dh = dh / 2;
        match self {
            Self::TopLeft => (0, 0),
            Self::TopCenter => (-half_dw, 0),
            Self::TopRight => (-dw, 0),
            Self::MiddleLeft => (0, -half_dh),
            Self::MiddleCenter => (-half_dw, -half_dh),
            Self::MiddleRight => (-dw, -half_dh),
            Self::BottomLeft => (0, -dh),
            Self::BottomCenter => (-half_dw, -dh),
            Self::BottomRight => (-dw, -dh),
        }
    }
}

/// State for the "Resize Map" dialog.
#[derive(Debug)]
pub struct ResizeMapDialog {
    pub open: bool,
    pub new_width: u32,
    pub new_height: u32,
    pub anchor: ResizeAnchor,
    /// Map size at the time the dialog was opened, used to derive the offset
    /// for the chosen anchor.
    pub source_size: (u32, u32),
}

impl Default for ResizeMapDialog {
    fn default() -> Self {
        Self {
            open: false,
            new_width: 64,
            new_height: 64,
            anchor: ResizeAnchor::MiddleCenter,
            source_size: (64, 64),
        }
    }
}

impl ResizeMapDialog {
    /// Compute the `(offset_x, offset_y)` in tile units from the current anchor.
    #[must_use]
    pub fn effective_offset(&self) -> (i32, i32) {
        self.anchor
            .offset(self.source_size, (self.new_width, self.new_height))
    }
}

/// State for the "New Map" dialog.
#[derive(Debug)]
pub struct NewMapDialog {
    pub open: bool,
    pub width: u32,
    pub height: u32,
    pub name: String,
    pub initial_height: u16,
    pub tileset: crate::config::Tileset,
}

impl Default for NewMapDialog {
    fn default() -> Self {
        Self {
            open: false,
            width: 64,
            height: 64,
            name: "NewMap".to_string(),
            initial_height: 0,
            tileset: crate::config::Tileset::Arizona,
        }
    }
}

/// State for the "test map cannot be copied" permission-error dialog.
#[derive(Debug, Default)]
pub struct PermissionErrorDialog {
    pub open: bool,
    pub target_path: std::path::PathBuf,
    pub error_message: String,
}

/// State for the modal shown when opening a `.wz` (or dropping one) fails.
///
/// `details` carries the raw loader error so the user can copy it into a
/// bug report; `message` is the human-readable explanation derived from
/// the archive's classification.
#[derive(Debug, Default)]
pub struct LoadErrorDialog {
    pub open: bool,
    pub title: String,
    pub message: String,
    pub details: String,
}

/// State for the Save As metadata dialog. Captures `level.json` fields
/// before invoking the file picker.
#[derive(Debug, Default)]
pub struct SaveAsMetadataDialog {
    pub open: bool,
    pub author: String,
    pub additional_authors: String,
    pub license: String,
    pub original_author: Option<String>,
}

/// State for the Map Properties dialog opened from `Map > Properties`.
///
/// Mirrors the editable `level.json` fields on the loaded map. `name` and
/// `players` shadow `WzMap.map_name` and `EditorApp.map_players` until the
/// user clicks OK so escapes don't dirty the document.
#[derive(Debug, Default)]
pub struct MapPropertiesDialog {
    pub open: bool,
    pub name: String,
    pub players: u8,
    pub author: String,
    pub additional_authors: String,
    pub license: String,
}

#[derive(Debug, Default)]
pub struct PublishInstructionsDialog {
    pub open: bool,
    pub zip_path: std::path::PathBuf,
    pub map_name: String,
    pub submission_url: String,
    pub browser_opened: bool,
}

/// Which type and index of object is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectedObject {
    Structure(usize),
    Droid(usize),
    Feature(usize),
    Label(usize),
    Gateway(usize),
}

/// Multi-selection container for map objects.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub objects: Vec<SelectedObject>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Returns the single selected object, or `None` if zero or multiple are selected.
    pub fn single(&self) -> Option<SelectedObject> {
        if self.objects.len() == 1 {
            Some(self.objects[0])
        } else {
            None
        }
    }

    pub fn contains(&self, obj: &SelectedObject) -> bool {
        self.objects.contains(obj)
    }

    /// Replace the entire selection with a single object.
    pub fn set_single(&mut self, obj: SelectedObject) {
        self.objects.clear();
        self.objects.push(obj);
    }

    pub fn clear(&mut self) {
        self.objects.clear();
    }

    /// Toggle an object in/out of the selection.
    pub fn toggle(&mut self, obj: SelectedObject) {
        if let Some(pos) = self.objects.iter().position(|o| *o == obj) {
            self.objects.remove(pos);
        } else {
            self.objects.push(obj);
        }
    }

    /// Add an object to the selection if not already present.
    pub fn add(&mut self, obj: SelectedObject) {
        if !self.contains(&obj) {
            self.objects.push(obj);
        }
    }

    /// Number of selected objects.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Selection group: structures+droids = 0, features = 1, labels = 2, gateways = 3.
    fn group(obj: &SelectedObject) -> u8 {
        match obj {
            SelectedObject::Structure(_) | SelectedObject::Droid(_) => 0,
            SelectedObject::Feature(_) => 1,
            SelectedObject::Label(_) => 2,
            SelectedObject::Gateway(_) => 3,
        }
    }

    /// Enforce that only one selection group is active.
    /// Keeps the group with the most members; on tie, keeps the group
    /// of the most recently added object (last in the vec).
    pub fn enforce_group(&mut self) {
        if self.objects.len() <= 1 {
            return;
        }
        let mut counts = [0u32; 4];
        for obj in &self.objects {
            counts[Self::group(obj) as usize] += 1;
        }
        let last_group = self.objects.last().map_or(0, Self::group);
        // Pick the unique max; on any tie, keep the group of the most recently added object.
        let max_count = *counts.iter().max().unwrap_or(&0);
        let tied = counts.iter().filter(|c| **c == max_count).count();
        let winning = if tied == 1 {
            counts
                .iter()
                .position(|c| *c == max_count)
                .unwrap_or(last_group as usize) as u8
        } else {
            last_group
        };
        self.objects.retain(|obj| Self::group(obj) == winning);
    }
}

/// State for the line-of-sight viewshed overlay.
///
/// When `show_range_on_select` is enabled, every selected weaponized
/// structure renders its viewshed. `last_selection_sig` is a cached hash
/// of the structure-selection at last compute, used to skip recomputing
/// every frame.
#[derive(Debug, Clone)]
pub struct ViewshedSettings {
    pub show_range_on_select: bool,
    pub last_selection_sig: u64,
}

impl Default for ViewshedSettings {
    fn default() -> Self {
        Self {
            show_range_on_select: true,
            last_selection_sig: 0,
        }
    }
}

impl ViewshedSettings {
    /// True when nothing should be drawn or computed.
    pub fn is_idle(&self) -> bool {
        !self.show_range_on_select
    }
}

pub use crate::startup::{
    GroundTextureLoadState, GroundTexturePayload, GroundUploadViews, MapModelLoadState,
    RuntimeTasks,
};

pub use crate::startup::pipeline::StartupPhase;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dock_layout_contains_all_required_tabs() {
        let dock = default_dock_layout();
        let required = [
            DockTab::Viewport,
            DockTab::Terrain,
            DockTab::TilesetBrowser,
            DockTab::AssetBrowser,
            DockTab::Properties,
            DockTab::Minimap,
            DockTab::Hierarchy,
            DockTab::Validation,
            DockTab::OutputLog,
            DockTab::Balance,
        ];
        for tab in required {
            assert!(
                dock.iter_all_tabs().any(|(_, t)| *t == tab),
                "default dock layout is missing tab {tab:?}",
            );
        }
    }
}
