//! Shared helper functions for validation and placement logic.

/// Packability score for structures. Two adjacent structures are compatible
/// only if their scores sum to <= 3 (matching WZ2100's canPack logic).
#[must_use]
pub fn structure_packability(structure_type: Option<&str>) -> u8 {
    match structure_type {
        Some("WALL" | "GATE" | "CORNER WALL" | "DEFENSE" | "REARM PAD" | "MISSILE SILO") => 1,
        Some("REPAIR FACILITY") => 3,
        _ => 2, // NORMAL: factories, research, power, HQ, etc.
    }
}

/// Whether a structure type is exempt from cliff/slope restrictions.
///
/// Matches WZ2100's `validLocation()` slope exemption (line 4366 in structure.cpp):
/// `REF_REPAIR_FACILITY`, `REF_DEFENSE`, `REF_GATE`, `REF_WALL`.
/// `CORNER WALL` is NOT exempt in the game.
#[must_use]
pub fn is_wall_or_defense(structure_type: Option<&str>) -> bool {
    matches!(
        structure_type,
        Some("WALL" | "GATE" | "DEFENSE" | "REPAIR FACILITY")
    )
}

/// Strip the player-count prefix from a map name.
///
/// WZ2100 convention: filenames are `{N}c-{BaseName}` or `{N}p-{BaseName}`
/// where N is the player count. E.g. `2c-Roughness` -> `Roughness`,
/// `2p-AValley` -> `AValley`. Returns the full name unchanged if no prefix
/// is found.
pub(super) fn strip_player_prefix(name: &str) -> &str {
    for sep in ["c-", "p-"] {
        if let Some(idx) = name.find(sep)
            && idx > 0
            && name[..idx].chars().all(|c| c.is_ascii_digit())
        {
            return &name[idx + sep.len()..];
        }
    }
    name
}

/// Structure types that accept modules.
///
/// Uses the `type` values from `structure.json`: `FACTORY`, `VTOL FACTORY`,
/// `CYBORG FACTORY`, `RESEARCH` (not "RESEARCH FACILITY"), `POWER GENERATOR`.
pub(super) fn accepts_modules(structure_type: &str) -> bool {
    matches!(
        structure_type,
        "FACTORY" | "VTOL FACTORY" | "CYBORG FACTORY" | "RESEARCH" | "POWER GENERATOR"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_prefix_standard() {
        assert_eq!(strip_player_prefix("2c-Roughness"), "Roughness");
    }

    #[test]
    fn strip_prefix_ten_players() {
        assert_eq!(strip_player_prefix("10c-WaterLoop"), "WaterLoop");
    }

    #[test]
    fn strip_prefix_no_prefix() {
        assert_eq!(strip_player_prefix("MyMap"), "MyMap");
    }

    #[test]
    fn strip_prefix_just_prefix() {
        assert_eq!(strip_player_prefix("4c-"), "");
    }

    #[test]
    fn strip_prefix_non_digit_before_c() {
        assert_eq!(strip_player_prefix("ac-Foo"), "ac-Foo");
    }

    #[test]
    fn strip_prefix_hyphen_in_base_name() {
        assert_eq!(strip_player_prefix("4c-My-Map"), "My-Map");
    }

    #[test]
    fn strip_prefix_p_form() {
        assert_eq!(strip_player_prefix("2p-AValley"), "AValley");
        assert_eq!(strip_player_prefix("10p-WaterLoop"), "WaterLoop");
    }

    #[test]
    fn packability_repair_is_three() {
        // Repair facility is the only type with packability 3
        assert_eq!(structure_packability(Some("REPAIR FACILITY")), 3);
    }

    #[test]
    fn corner_wall_not_exempt_from_slope() {
        // CORNER WALL is NOT exempt from slope checks in WZ2100 (bug fix)
        assert!(!is_wall_or_defense(Some("CORNER WALL")));
    }

    #[test]
    fn accepts_modules_research_not_research_facility() {
        // Game stats use "RESEARCH", not "RESEARCH FACILITY" (bug fix)
        assert!(accepts_modules("RESEARCH"));
        assert!(!accepts_modules("RESEARCH FACILITY"));
    }
}
