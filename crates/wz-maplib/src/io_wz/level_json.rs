//! Build and parse the `level.json` and `gam.json` sidecars that WZ2100
//! expects alongside `game.map` inside a `.wz` archive.

use crate::io_wz::WzMap;
use crate::map_data::MapData;

use super::common::read_zip_file;

/// Identifies the editor in `level.json`'s `generator` field.
const GENERATOR: &str = concat!("wzmapeditor ", env!("CARGO_PKG_VERSION"));

/// Build the `level.json` content that WZ2100 requires to recognize a map.
pub(super) fn build_level_json(map: &WzMap) -> String {
    let mut author_obj = serde_json::Map::new();
    let author_name = map.author.as_deref().unwrap_or("wzmapeditor");
    author_obj.insert("name".to_string(), serde_json::json!(author_name));
    if !map.additional_authors.is_empty() {
        author_obj.insert(
            "additional-authors".to_string(),
            serde_json::json!(map.additional_authors),
        );
    }

    let mut value = serde_json::json!({
        "name": map.map_name,
        "type": "skirmish",
        "players": map.players,
        "tileset": map.tileset,
        "author": serde_json::Value::Object(author_obj),
        "generator": GENERATOR,
    });
    if let Some(license) = &map.license {
        value["license"] = serde_json::json!(license);
    }
    serde_json::to_string_pretty(&value).expect("level.json serialization cannot fail")
}

/// Metadata extracted from `level.json` inside a `.wz` archive.
pub(crate) struct LevelMeta {
    pub name: String,
    pub players: u8,
    pub tileset: String,
    pub author: Option<String>,
    pub additional_authors: Vec<String>,
    pub license: Option<String>,
}

/// Try to read `level.json` from a zip archive and extract metadata.
pub(super) fn read_level_json<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    prefix: &str,
) -> Option<LevelMeta> {
    let bytes = read_zip_file(archive, &format!("{prefix}level.json")).ok()?;
    parse_level_json_bytes(&bytes)
}

/// Parse a `level.json` payload into a [`LevelMeta`].
pub(crate) fn parse_level_json_bytes(bytes: &[u8]) -> Option<LevelMeta> {
    let text = String::from_utf8_lossy(bytes);
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = json.get("name")?.as_str()?.to_string();
    let players = json.get("players")?.as_u64()? as u8;
    let tileset = json.get("tileset")?.as_str()?.to_string();
    let (author, additional_authors) = parse_author(json.get("author"));
    let license = json
        .get("license")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(LevelMeta {
        name,
        players,
        tileset,
        author,
        additional_authors,
        license,
    })
}

/// Parse the `author` field, accepting either a bare string or an object with
/// `name` and optional `additional-authors`.
fn parse_author(value: Option<&serde_json::Value>) -> (Option<String>, Vec<String>) {
    let Some(value) = value else {
        return (None, Vec::new());
    };
    if let Some(s) = value.as_str() {
        return (Some(s.to_string()), Vec::new());
    }
    let Some(obj) = value.as_object() else {
        return (None, Vec::new());
    };
    let name = obj.get("name").and_then(|v| v.as_str()).map(str::to_string);
    let additional = obj
        .get("additional-authors")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    (name, additional)
}

/// Build a `gam.json` file with scroll/viewport bounds.
pub(super) fn build_gam_json(map_data: &MapData) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "version": 7,
        "gameTime": 0,
        "GameType": 0,
        "ScrollMinX": 0,
        "ScrollMinY": 0,
        "ScrollMaxX": map_data.width,
        "ScrollMaxY": map_data.height,
        "levelName": ""
    }))
    .expect("gam.json serialization cannot fail")
}
