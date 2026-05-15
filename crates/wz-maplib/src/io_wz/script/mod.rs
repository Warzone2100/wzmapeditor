//! Read-only import of Warzone 2100 "script maps".
//!
//! Script maps ship a `game.js` and let Warzone procedurally generate the
//! terrain at load time, seeded by a single 32-bit value. We embed the
//! same `QuickJS` fork (`quickjs-wz`) the game uses, expose the same four
//! host functions, and reuse the same seed-to-PRNG mapping
//! (`std::mt19937` ↔ `rand_mt::Mt`) so the output is byte-for-byte
//! identical to what Warzone would generate.
//!
//! Ported from `lib/wzmaplib/src/map_script.cpp` in the upstream Warzone
//! 2100 source tree.

use std::path::Path;

use crate::io_ttp;
use crate::io_wz::WzMap;
use crate::io_wz::archive::WzArchiveReader;
use crate::io_wz::common::detect_tileset_from_ttp;
use crate::io_wz::level_json::parse_level_json_bytes;

mod host_fns;
mod runtime;

pub use runtime::{ScriptError, run_script_source};

/// Load a script map from a `.wz` archive, generating its terrain
/// deterministically for the given seed.
///
/// The resulting map is intended to be opened read-only — the caller is
/// expected to refuse edits, since any edit would diverge from the
/// engine-side projection of `game.js` for the chosen seed.
pub fn run_script_map(wz_path: &Path, seed: u32) -> Result<WzMap, ScriptError> {
    let mut archive = WzArchiveReader::open(wz_path)
        .ok_or_else(|| ScriptError::Io(format!("failed to open {}", wz_path.display())))?;

    let script_bytes = archive
        .read_entry("game.js")
        .ok_or_else(|| ScriptError::MissingEntry("game.js".to_owned()))?;
    let script = String::from_utf8(script_bytes)
        .map_err(|e| ScriptError::Io(format!("game.js is not UTF-8: {e}")))?;

    let stem = wz_path.file_stem().map_or_else(
        || "scripted".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    );

    let mut map = run_script_source(&stem, &script, seed)?;

    // Pull metadata + terrain types from the archive, mirroring the
    // static-map loader's behaviour.
    let level_meta = archive
        .read_entry("level.json")
        .as_deref()
        .and_then(parse_level_json_bytes);
    let terrain_types = archive
        .read_entry("ttypes.ttp")
        .and_then(|bytes| io_ttp::read_ttp(&bytes).ok());

    map.terrain_types = terrain_types;
    if let Some(meta) = level_meta {
        map.map_name = meta.name;
        map.players = meta.players;
        map.tileset = meta.tileset;
        map.author = meta.author;
        map.additional_authors = meta.additional_authors;
        map.license = meta.license;
    } else {
        map.tileset = detect_tileset_from_ttp(map.terrain_types.as_ref());
    }

    Ok(map)
}
