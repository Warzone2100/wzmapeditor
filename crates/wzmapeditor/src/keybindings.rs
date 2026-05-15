//! Centralized keybinding system.
//!
//! Defines all bindable [`Action`]s, a serializable [`KeyCombo`] type, and
//! the [`Keymap`] that maps actions to key combinations. The keymap is
//! persisted in `EditorConfig` and drives a single dispatch point in the
//! main `update()` loop.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::tools::{HeightBrushMode, ToolId};

/// A key press with optional modifier keys.
///
/// Serialized as a human-readable string such as `"Ctrl+Shift+Z"` or `"F5"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub key: egui::Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyCombo {
    const fn bare(key: egui::Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    const fn ctrl(key: egui::Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: false,
            alt: false,
        }
    }

    const fn ctrl_shift(key: egui::Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: true,
            alt: false,
        }
    }

    /// Convert to egui modifier flags for use with `consume_key`.
    pub fn to_egui_modifiers(&self) -> egui::Modifiers {
        // `command` is the platform-abstract modifier (Cmd on macOS, Ctrl
        // elsewhere). Setting both `ctrl` and `command` would force egui's
        // `cmd_ctrl_matches` to require the raw Ctrl flag on macOS, where a
        // Cmd press only sets `mac_cmd` + `command`.
        egui::Modifiers {
            alt: self.alt,
            ctrl: false,
            shift: self.shift,
            mac_cmd: false,
            command: self.ctrl,
        }
    }

    pub fn has_modifiers(&self) -> bool {
        self.ctrl || self.shift || self.alt
    }

    /// Whether this is a WASD camera fly key that conflicts with RMB movement.
    pub fn is_camera_fly_key(&self) -> bool {
        matches!(
            self.key,
            egui::Key::W | egui::Key::A | egui::Key::S | egui::Key::D
        )
    }

    fn modifier_count(&self) -> u8 {
        u8::from(self.ctrl) + u8::from(self.shift) + u8::from(self.alt)
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl+")?;
        }
        if self.shift {
            write!(f, "Shift+")?;
        }
        if self.alt {
            write!(f, "Alt+")?;
        }
        write!(f, "{}", self.key.name())
    }
}

impl Serialize for KeyCombo {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for KeyCombo {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        parse_key_combo(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid key combo: {s}")))
    }
}

/// Parse a string like `"Ctrl+Shift+Z"` into a [`KeyCombo`].
fn parse_key_combo(s: &str) -> Option<KeyCombo> {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;

    let parts: Vec<&str> = s.split('+').collect();
    let key_name = parts.last()?;
    for &part in &parts[..parts.len() - 1] {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => ctrl = true,
            "shift" => shift = true,
            "alt" => alt = true,
            _ => return None,
        }
    }

    let key = egui::Key::from_name(key_name)?;
    Some(KeyCombo {
        key,
        ctrl,
        shift,
        alt,
    })
}

/// All bindable editor actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    // Global
    Undo,
    Redo,
    Save,
    TestMap,

    // Object / viewport context
    DeleteSelected,
    RotatePlacement,
    EscapeTool,
    Duplicate,

    // Tool switching
    ToolHeightRaise,
    ToolHeightLower,
    ToolHeightSmooth,
    ToolHeightSet,
    ToolTexturePaint,
    ToolGroundTypePaint,
    ToolObjectPlace,
    ToolObjectSelect,
    ToolScriptLabel,
    ToolGateway,
    ToolStamp,
    ToolWallPlacement,
    ToolVertexSculpt,

    // Overlay toggles
    ToggleHeatmap,
}

/// Adding a variant without updating `ALL` causes a build failure via the
/// const array sizing below.
const ACTION_VARIANT_COUNT: usize = 22;

impl Action {
    pub const ALL: &'static [Self] = {
        const LIST: [Action; ACTION_VARIANT_COUNT] = [
            Action::Undo,
            Action::Redo,
            Action::Save,
            Action::TestMap,
            Action::DeleteSelected,
            Action::RotatePlacement,
            Action::EscapeTool,
            Action::Duplicate,
            Action::ToolHeightRaise,
            Action::ToolHeightLower,
            Action::ToolHeightSmooth,
            Action::ToolHeightSet,
            Action::ToolTexturePaint,
            Action::ToolGroundTypePaint,
            Action::ToolObjectPlace,
            Action::ToolObjectSelect,
            Action::ToolScriptLabel,
            Action::ToolGateway,
            Action::ToolStamp,
            Action::ToolWallPlacement,
            Action::ToolVertexSculpt,
            Action::ToggleHeatmap,
        ];
        &LIST
    };

    /// Human-readable display name for the settings UI.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Save => "Save",
            Self::TestMap => "Test Map",
            Self::DeleteSelected => "Delete Selected",
            Self::RotatePlacement => "Rotate Placement",
            Self::EscapeTool => "Escape Tool",
            Self::Duplicate => "Duplicate",
            Self::ToolHeightRaise => "Tool: Height Raise",
            Self::ToolHeightLower => "Tool: Height Lower",
            Self::ToolHeightSmooth => "Tool: Height Smooth",
            Self::ToolHeightSet => "Tool: Height Set",
            Self::ToolTexturePaint => "Tool: Texture Paint",
            Self::ToolGroundTypePaint => "Tool: Ground Type",
            Self::ToolObjectPlace => "Tool: Object Place",
            Self::ToolObjectSelect => "Tool: Object Select",
            Self::ToolScriptLabel => "Tool: Script Label",
            Self::ToolGateway => "Tool: Gateway",
            Self::ToolStamp => "Tool: Stamp",
            Self::ToolWallPlacement => "Tool: Wall Placement",
            Self::ToolVertexSculpt => "Tool: Vertex Sculpt",
            Self::ToggleHeatmap => "Toggle Heatmap",
        }
    }

    /// Map tool-switching actions to their `ToolId` variant. The four
    /// `ToolHeight*` actions all map to `ToolId::HeightBrush`; their specific
    /// mode is exposed separately via [`Self::as_height_mode`].
    pub fn as_tool(self) -> Option<ToolId> {
        match self {
            Self::ToolHeightRaise
            | Self::ToolHeightLower
            | Self::ToolHeightSmooth
            | Self::ToolHeightSet => Some(ToolId::HeightBrush),
            Self::ToolTexturePaint => Some(ToolId::TexturePaint),
            Self::ToolGroundTypePaint => Some(ToolId::GroundTypePaint),
            Self::ToolObjectPlace => Some(ToolId::ObjectPlace),
            Self::ToolObjectSelect => Some(ToolId::ObjectSelect),
            Self::ToolScriptLabel => Some(ToolId::ScriptLabel),
            Self::ToolGateway => Some(ToolId::Gateway),
            Self::ToolStamp => Some(ToolId::Stamp),
            Self::ToolWallPlacement => Some(ToolId::WallPlacement),
            Self::ToolVertexSculpt => Some(ToolId::VertexSculpt),
            _ => None,
        }
    }

    /// For the four height-brush actions, return the mode they switch to.
    /// Other actions return `None`.
    pub fn as_height_mode(self) -> Option<HeightBrushMode> {
        match self {
            Self::ToolHeightRaise => Some(HeightBrushMode::Raise),
            Self::ToolHeightLower => Some(HeightBrushMode::Lower),
            Self::ToolHeightSmooth => Some(HeightBrushMode::Smooth),
            Self::ToolHeightSet => Some(HeightBrushMode::Set),
            _ => None,
        }
    }

    /// Actions that must not fire while a text widget has focus, because
    /// they share modifiers with standard text-editing shortcuts.
    pub fn yields_to_text_focus(self) -> bool {
        matches!(self, Self::Duplicate)
    }

    /// Map a `ToolId` back to its primary switching action. For the unified
    /// height brush, the canonical action is `ToolHeightRaise`; the brush's
    /// active mode is tracked separately on `ToolState`.
    pub fn from_tool(tool: ToolId) -> Self {
        match tool {
            ToolId::HeightBrush => Self::ToolHeightRaise,
            ToolId::TexturePaint => Self::ToolTexturePaint,
            ToolId::GroundTypePaint => Self::ToolGroundTypePaint,
            ToolId::ObjectPlace => Self::ToolObjectPlace,
            ToolId::ObjectSelect => Self::ToolObjectSelect,
            ToolId::ScriptLabel => Self::ToolScriptLabel,
            ToolId::Gateway => Self::ToolGateway,
            ToolId::Stamp => Self::ToolStamp,
            ToolId::WallPlacement => Self::ToolWallPlacement,
            ToolId::VertexSculpt => Self::ToolVertexSculpt,
        }
    }
}

/// Maps [`Action`]s to one or more [`KeyCombo`]s.
///
/// Serialized as `{ "Undo": ["Ctrl+Z"], "Redo": ["Ctrl+Y", "Ctrl+Shift+Z"] }`.
/// Caches a sort order for per-frame dispatch and shortcut display text so
/// the hot path does no allocations.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<Action, Vec<KeyCombo>>,
    /// Pre-sorted (action, combo) pairs for dispatch. Sorted by modifier
    /// count descending so Ctrl+Shift+Z is tested before Ctrl+Z.
    sorted_cache: Vec<(Action, KeyCombo)>,
    /// Cached shortcut display text per action (first binding only).
    text_cache: HashMap<Action, String>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::default_keymap()
    }
}

impl Keymap {
    pub fn default_keymap() -> Self {
        use egui::Key;

        let mut m = HashMap::new();

        m.insert(Action::Undo, vec![KeyCombo::ctrl(Key::Z)]);
        m.insert(
            Action::Redo,
            vec![KeyCombo::ctrl(Key::Y), KeyCombo::ctrl_shift(Key::Z)],
        );
        m.insert(Action::Save, vec![KeyCombo::ctrl(Key::S)]);
        m.insert(Action::TestMap, vec![KeyCombo::bare(Key::F5)]);
        m.insert(
            Action::DeleteSelected,
            vec![KeyCombo::bare(Key::Delete), KeyCombo::bare(Key::Backspace)],
        );
        m.insert(Action::RotatePlacement, vec![KeyCombo::bare(Key::R)]);
        m.insert(Action::EscapeTool, vec![KeyCombo::bare(Key::Escape)]);
        m.insert(Action::Duplicate, vec![KeyCombo::ctrl(Key::D)]);

        m.insert(Action::ToolHeightRaise, vec![KeyCombo::bare(Key::Num1)]);
        m.insert(Action::ToolHeightLower, vec![KeyCombo::bare(Key::Num2)]);
        m.insert(Action::ToolHeightSmooth, vec![KeyCombo::bare(Key::Num3)]);
        m.insert(Action::ToolHeightSet, vec![KeyCombo::bare(Key::Num4)]);
        m.insert(Action::ToolTexturePaint, vec![KeyCombo::bare(Key::Num5)]);
        m.insert(Action::ToolGroundTypePaint, vec![KeyCombo::bare(Key::Num6)]);
        m.insert(Action::ToolStamp, vec![KeyCombo::bare(Key::Num7)]);
        m.insert(Action::ToolWallPlacement, vec![KeyCombo::bare(Key::Num8)]);
        // ObjectPlace stays rebindable but ships unbound: the asset browser
        // and Ctrl+click eyedropper are the canonical entry points.
        m.insert(Action::ToolObjectPlace, vec![]);

        // S is suppressed during RMB fly mode (WASD camera conflict).
        m.insert(Action::ToolObjectSelect, vec![KeyCombo::bare(Key::S)]);
        m.insert(Action::ToolScriptLabel, vec![KeyCombo::bare(Key::L)]);
        m.insert(Action::ToolGateway, vec![KeyCombo::bare(Key::G)]);
        m.insert(Action::ToolVertexSculpt, vec![KeyCombo::bare(Key::V)]);

        m.insert(Action::ToggleHeatmap, vec![KeyCombo::bare(Key::H)]);

        let mut km = Self {
            bindings: m,
            sorted_cache: Vec::new(),
            text_cache: HashMap::new(),
        };
        km.rebuild_caches();
        km
    }

    fn rebuild_caches(&mut self) {
        self.sorted_cache = self
            .bindings
            .iter()
            .flat_map(|(&action, combos)| combos.iter().map(move |c| (action, c.clone())))
            .collect();
        self.sorted_cache
            .sort_by_key(|b| std::cmp::Reverse(b.1.modifier_count()));

        self.text_cache.clear();
        for (&action, combos) in &self.bindings {
            let text = combos.first().map_or_else(String::new, KeyCombo::to_string);
            self.text_cache.insert(action, text);
        }
    }

    pub fn rebind(&mut self, action: Action, combos: Vec<KeyCombo>) {
        self.bindings.insert(action, combos);
        self.rebuild_caches();
    }

    /// Get the cached display string for an action's first binding.
    ///
    /// Returns an empty string if the action is unbound. Zero allocations.
    pub fn shortcut_text(&self, action: Action) -> &str {
        self.text_cache.get(&action).map_or("", |s| s.as_str())
    }

    /// Find the first action whose key combo was pressed this frame.
    ///
    /// Uses `consume_key` so matched keys are not handled by text widgets.
    /// Bare-key bindings are skipped when a text field has focus or when
    /// the right mouse button is held (camera fly mode). Returns at most
    /// one action per frame.
    pub fn poll_action(&self, ctx: &egui::Context, rmb_held: bool) -> Option<Action> {
        let wants_kb = ctx.egui_wants_keyboard_input();

        ctx.input_mut(|input| {
            for (action, combo) in &self.sorted_cache {
                let is_bare = !combo.has_modifiers();
                if is_bare && wants_kb {
                    continue;
                }
                // Yield Ctrl+D (Duplicate) etc. while a text field has focus.
                if wants_kb && action.yields_to_text_focus() {
                    continue;
                }
                // During RMB fly mode only suppress WASD camera keys.
                if is_bare && rmb_held && combo.is_camera_fly_key() {
                    continue;
                }

                if input.consume_key(combo.to_egui_modifiers(), combo.key) {
                    return Some(*action);
                }
            }
            None
        })
    }

    /// Detect conflicting bindings (same combo bound to multiple actions).
    pub fn conflicts(&self) -> Vec<(Action, Action, KeyCombo)> {
        let mut seen: HashMap<&KeyCombo, Action> = HashMap::new();
        let mut conflicts = Vec::new();

        for (&action, combos) in &self.bindings {
            for combo in combos {
                if let Some(&prev_action) = seen.get(combo) {
                    conflicts.push((prev_action, action, combo.clone()));
                } else {
                    seen.insert(combo, action);
                }
            }
        }

        conflicts
    }
}

impl Serialize for Keymap {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // HashMap<Action, Vec<String>> keeps the JSON human-readable.
        let string_map: HashMap<Action, Vec<String>> = self
            .bindings
            .iter()
            .map(|(action, combos)| (*action, combos.iter().map(KeyCombo::to_string).collect()))
            .collect();
        string_map.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize keys as strings first so a removed or renamed `Action`
        // variant (e.g. a shortcut saved from a previous build) is skipped
        // rather than failing the whole config load.
        let raw_map: HashMap<String, Vec<String>> = HashMap::deserialize(deserializer)?;
        let mut bindings = HashMap::new();

        for (action_key, combo_strings) in raw_map {
            let Ok(action) =
                serde_json::from_value::<Action>(serde_json::Value::String(action_key.clone()))
            else {
                log::warn!("Ignoring unknown action in config: \"{action_key}\"");
                continue;
            };
            let mut combos = Vec::new();
            for s in &combo_strings {
                match parse_key_combo(s) {
                    Some(combo) => combos.push(combo),
                    None => {
                        // Drop malformed combos but keep the action around so
                        // the default-merge pass below can fill it back in.
                        log::warn!("Ignoring invalid key combo in config: \"{s}\"");
                    }
                }
            }
            if !combos.is_empty() {
                bindings.insert(action, combos);
            }
        }

        // Merge defaults for any actions missing from the saved config so
        // newly added actions always have a binding.
        let defaults = Self::default_keymap();
        for (action, combos) in defaults.bindings {
            bindings.entry(action).or_insert(combos);
        }

        let mut km = Self {
            bindings,
            sorted_cache: Vec::new(),
            text_cache: HashMap::new(),
        };
        km.rebuild_caches();
        Ok(km)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_combo_display_roundtrip() {
        let combo = KeyCombo::ctrl_shift(egui::Key::Z);
        let s = combo.to_string();
        assert_eq!(s, "Ctrl+Shift+Z");
        let parsed = parse_key_combo(&s).expect("should parse");
        assert_eq!(parsed, combo);
    }

    #[test]
    fn key_combo_bare_key_roundtrip() {
        let combo = KeyCombo::bare(egui::Key::F5);
        let s = combo.to_string();
        assert_eq!(s, "F5");
        let parsed = parse_key_combo(&s).expect("should parse");
        assert_eq!(parsed, combo);
    }

    #[test]
    fn key_combo_number_key_roundtrip() {
        let combo = KeyCombo::bare(egui::Key::Num1);
        let s = combo.to_string();
        assert_eq!(s, "1");
        let parsed = parse_key_combo(&s).expect("should parse");
        assert_eq!(parsed, combo);
    }

    #[test]
    fn default_keymap_has_all_actions() {
        let km = Keymap::default_keymap();
        for action in Action::ALL {
            assert!(
                km.bindings.contains_key(action),
                "missing default binding for {action:?}"
            );
        }
    }

    #[test]
    fn default_keymap_no_conflicts() {
        let km = Keymap::default_keymap();
        let conflicts = km.conflicts();
        assert!(
            conflicts.is_empty(),
            "default keymap has conflicts: {conflicts:?}"
        );
    }

    #[test]
    fn shortcut_text_returns_first_binding() {
        let km = Keymap::default_keymap();
        assert_eq!(km.shortcut_text(Action::Save), "Ctrl+S");
        assert_eq!(km.shortcut_text(Action::Redo), "Ctrl+Y");
    }

    #[test]
    fn keymap_serde_roundtrip() {
        let km = Keymap::default_keymap();
        let json = serde_json::to_string_pretty(&km).expect("serialize");
        let km2: Keymap = serde_json::from_str(&json).expect("deserialize");
        for action in Action::ALL {
            let orig = km.bindings.get(action).expect("orig");
            let loaded = km2.bindings.get(action).expect("loaded");
            assert_eq!(orig, loaded, "mismatch for {action:?}");
        }
    }

    #[test]
    fn action_tool_roundtrip() {
        // Multiple actions can map to the same ToolId (the four ToolHeight*
        // actions all share ToolId::HeightBrush), so the round-trip we can
        // assert is from_tool(tool) -> action -> as_tool() == Some(tool).
        for action in Action::ALL {
            if let Some(tool) = action.as_tool() {
                assert_eq!(Action::from_tool(tool).as_tool(), Some(tool));
            }
        }
    }

    #[test]
    fn height_actions_share_tool_and_have_modes() {
        let height_actions = [
            (Action::ToolHeightRaise, HeightBrushMode::Raise),
            (Action::ToolHeightLower, HeightBrushMode::Lower),
            (Action::ToolHeightSmooth, HeightBrushMode::Smooth),
            (Action::ToolHeightSet, HeightBrushMode::Set),
        ];
        for (action, mode) in height_actions {
            assert_eq!(action.as_tool(), Some(ToolId::HeightBrush));
            assert_eq!(action.as_height_mode(), Some(mode));
        }
        assert!(Action::ToolTexturePaint.as_height_mode().is_none());
    }

    #[test]
    fn parse_invalid_combo_returns_none() {
        assert!(parse_key_combo("").is_none());
        assert!(parse_key_combo("Ctrl+").is_none());
        assert!(parse_key_combo("NotAKey").is_none());
    }

    #[test]
    fn key_combo_alt_modifier_roundtrip() {
        let combo = KeyCombo {
            key: egui::Key::F1,
            ctrl: false,
            shift: false,
            alt: true,
        };
        let s = combo.to_string();
        assert_eq!(s, "Alt+F1");
        let parsed = parse_key_combo(&s).expect("should parse");
        assert_eq!(parsed, combo);
    }

    #[test]
    fn key_combo_all_modifiers_roundtrip() {
        let combo = KeyCombo {
            key: egui::Key::A,
            ctrl: true,
            shift: true,
            alt: true,
        };
        let s = combo.to_string();
        assert_eq!(s, "Ctrl+Shift+Alt+A");
        let parsed = parse_key_combo(&s).expect("should parse");
        assert_eq!(parsed, combo);
    }

    #[test]
    fn has_modifiers_reports_correctly() {
        assert!(!KeyCombo::bare(egui::Key::A).has_modifiers());
        assert!(KeyCombo::ctrl(egui::Key::A).has_modifiers());
        assert!(KeyCombo::ctrl_shift(egui::Key::A).has_modifiers());
    }

    #[test]
    fn modifier_count_ordering() {
        let bare = KeyCombo::bare(egui::Key::Z);
        let ctrl = KeyCombo::ctrl(egui::Key::Z);
        let ctrl_shift = KeyCombo::ctrl_shift(egui::Key::Z);
        assert!(ctrl_shift.modifier_count() > ctrl.modifier_count());
        assert!(ctrl.modifier_count() > bare.modifier_count());
    }

    #[test]
    fn to_egui_modifiers_maps_correctly() {
        let combo = KeyCombo::ctrl_shift(egui::Key::Z);
        let mods = combo.to_egui_modifiers();
        assert!(mods.command);
        assert!(mods.shift);
        assert!(!mods.alt);

        let bare = KeyCombo::bare(egui::Key::A);
        let mods = bare.to_egui_modifiers();
        assert!(!mods.command);
        assert!(!mods.shift);
        assert!(!mods.alt);
    }

    #[test]
    fn parse_case_insensitive_modifiers() {
        let combo = parse_key_combo("ctrl+shift+Z").expect("should parse");
        assert!(combo.ctrl);
        assert!(combo.shift);
        assert_eq!(combo.key, egui::Key::Z);

        let combo2 = parse_key_combo("CMD+S").expect("should parse");
        assert!(combo2.ctrl);
        assert_eq!(combo2.key, egui::Key::S);
    }

    #[test]
    fn deserialize_merges_defaults_for_missing_actions() {
        // Simulate a saved config with only one binding.
        let json = r#"{"Save": ["Ctrl+S"]}"#;
        let km: Keymap = serde_json::from_str(json).expect("deserialize");

        // The explicitly saved binding should be present.
        assert_eq!(km.shortcut_text(Action::Save), "Ctrl+S");

        // Actions missing from the JSON should get defaults.
        assert_eq!(km.shortcut_text(Action::Undo), "Ctrl+Z");
        assert_eq!(km.shortcut_text(Action::TestMap), "F5");
        assert_eq!(km.shortcut_text(Action::ToolHeightRaise), "1");
    }

    #[test]
    fn shortcut_text_empty_for_unbound_action() {
        let mut km = Keymap::default_keymap();
        km.rebind(Action::Save, vec![]);
        assert_eq!(km.shortcut_text(Action::Save), "");
    }

    #[test]
    fn conflicts_detects_duplicate_binding() {
        let mut km = Keymap::default_keymap();
        km.rebind(Action::Undo, vec![KeyCombo::ctrl(egui::Key::S)]);
        let conflicts = km.conflicts();
        assert!(
            !conflicts.is_empty(),
            "should detect Ctrl+S bound to both Undo and Save"
        );
    }

    #[test]
    fn non_tool_actions_return_none_from_as_tool() {
        assert!(Action::Undo.as_tool().is_none());
        assert!(Action::Save.as_tool().is_none());
        assert!(Action::DeleteSelected.as_tool().is_none());
    }

    #[test]
    fn sorted_cache_has_modifiers_first() {
        let km = Keymap::default_keymap();
        // Ctrl+Shift+Z (Redo) should come before Ctrl+Z (Undo) in the cache.
        let ctrl_shift_z_pos = km
            .sorted_cache
            .iter()
            .position(|(_, c)| c.ctrl && c.shift && c.key == egui::Key::Z);
        let ctrl_z_pos = km
            .sorted_cache
            .iter()
            .position(|(_, c)| c.ctrl && !c.shift && c.key == egui::Key::Z);
        assert!(
            ctrl_shift_z_pos.unwrap() < ctrl_z_pos.unwrap(),
            "Ctrl+Shift+Z must be tested before Ctrl+Z to avoid Undo stealing Redo"
        );
    }

    #[test]
    fn rebind_updates_caches() {
        let mut km = Keymap::default_keymap();
        km.rebind(Action::Save, vec![KeyCombo::ctrl(egui::Key::W)]);
        assert_eq!(km.shortcut_text(Action::Save), "Ctrl+W");
        // Sorted cache should contain the new binding.
        assert!(
            km.sorted_cache
                .iter()
                .any(|(a, c)| *a == Action::Save && c.key == egui::Key::W)
        );
    }

    #[test]
    fn deserialize_skips_malformed_combos() {
        // One valid, one invalid combo for Save. The invalid one is skipped.
        let json = r#"{"Save": ["Ctrl+S", "NotAKey"]}"#;
        let km: Keymap = serde_json::from_str(json).expect("should not fail");
        assert_eq!(km.shortcut_text(Action::Save), "Ctrl+S");
    }

    #[test]
    fn deserialize_all_invalid_combos_falls_back_to_default() {
        // All combos for Save are invalid, so it should get the default binding.
        let json = r#"{"Save": ["BadKey"]}"#;
        let km: Keymap = serde_json::from_str(json).expect("should not fail");
        // Falls back to default Ctrl+S.
        assert_eq!(km.shortcut_text(Action::Save), "Ctrl+S");
    }

    #[test]
    fn deserialize_skips_removed_action_variant() {
        // Configs may carry action names that no longer exist as variants.
        // The loader must skip them rather than fail the whole keymap.
        let json = r#"{"Save": ["Ctrl+S"], "Paste": ["Ctrl+V"]}"#;
        let km: Keymap = serde_json::from_str(json).expect("should not fail");
        assert_eq!(km.shortcut_text(Action::Save), "Ctrl+S");
        assert_eq!(km.shortcut_text(Action::Undo), "Ctrl+Z");
    }

    #[test]
    fn action_all_count_matches_variant_count() {
        assert_eq!(Action::ALL.len(), ACTION_VARIANT_COUNT);
    }
}
