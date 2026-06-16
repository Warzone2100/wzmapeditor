//! Per-map custom droid templates.
//!
//! Tracks designer-authored [`TemplateStats`] separately from built-ins
//! loaded from `data/base/stats/templates.json`, so the editor knows
//! which entries to write back into the map archive's `templates.json`.
//! Custom templates are merged into the shared [`StatsDatabase`] at
//! runtime so placement, rendering, and the asset browser resolve them
//! by id transparently.

use std::collections::HashMap;

use wz_stats::StatsDatabase;
use wz_stats::templates::{TemplateStats, load_templates, serialize_templates};

/// User-authored templates for the currently open map.
///
/// `ids_owned` lists template ids the designer created. Ids that overlap
/// with built-ins (e.g. the user copied and renamed a stock design) are
/// also considered owned so they get written out on save.
#[derive(Debug, Default, Clone)]
pub struct CustomTemplateStore {
    ids_owned: std::collections::BTreeSet<String>,
    templates: HashMap<String, TemplateStats>,
}

impl CustomTemplateStore {
    /// True when the id was authored by the designer rather than being a built-in.
    pub fn owns(&self, id: &str) -> bool {
        self.ids_owned.contains(id)
    }

    /// Insert or replace a template (id taken from `stats.id`). Also
    /// registers it into `db` so placement and rendering resolve it.
    pub fn insert(&mut self, stats: TemplateStats, db: &mut StatsDatabase) {
        let id = stats.id.clone();
        assert!(!id.is_empty(), "custom template must have a non-empty id");
        self.ids_owned.insert(id.clone());
        db.templates.insert(id.clone(), stats.clone());
        self.templates.insert(id, stats);
    }

    /// Fresh `Custom_NNN` id (starting at 001) that doesn't collide with
    /// `db` or existing custom entries.
    pub fn fresh_id(&self, db: &StatsDatabase) -> String {
        for n in 1..=9999_u32 {
            let candidate = format!("Custom_{n:03}");
            if !db.templates.contains_key(&candidate) && !self.templates.contains_key(&candidate) {
                return candidate;
            }
        }
        // Astronomically unlikely overflow; timestamp-based fallback.
        format!(
            "Custom_{}",
            web_time::SystemTime::UNIX_EPOCH
                .elapsed()
                .map_or(0, |d| d.as_nanos())
        )
    }

    /// Replace the store's contents with the templates encoded in `json`
    /// and register them into `db`. Parse errors are logged and skipped
    /// rather than aborting the map load.
    pub fn load_from_json(&mut self, json: &str, db: &mut StatsDatabase) {
        match load_templates(json) {
            Ok(parsed) => {
                self.clear(db);
                for (id, mut stats) in parsed {
                    if stats.id.is_empty() {
                        stats.id.clone_from(&id);
                    }
                    self.ids_owned.insert(id.clone());
                    db.templates.insert(id.clone(), stats.clone());
                    self.templates.insert(id, stats);
                }
                log::info!(
                    "Loaded {} custom droid template(s) from map archive",
                    self.templates.len()
                );
            }
            Err(e) => {
                log::warn!("Ignoring map templates.json: {e}");
            }
        }
    }

    /// Serialize tracked templates as a WZ2100-compatible `templates.json`.
    /// Returns `None` when empty so callers can skip writing the file.
    pub fn to_json(&self) -> Option<String> {
        if self.templates.is_empty() {
            return None;
        }
        match serialize_templates(&self.templates) {
            Ok(s) => Some(s),
            Err(e) => {
                log::error!("Failed to serialize custom templates: {e}");
                None
            }
        }
    }

    /// Drop all tracked templates and remove them from `db`.
    pub fn clear(&mut self, db: &mut StatsDatabase) {
        for id in std::mem::take(&mut self.ids_owned) {
            db.templates.remove(&id);
        }
        self.templates.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_template(id: &str) -> TemplateStats {
        TemplateStats {
            id: id.to_string(),
            body: "Body1REC".into(),
            propulsion: "wheeled01".into(),
            weapons: vec!["MG1Mk1".into()],
            name: Some(id.to_string()),
            droid_type: Some("WEAPON".into()),
            construct: None,
            sensor: None,
            repair: None,
            ecm: None,
            brain: None,
        }
    }

    #[test]
    fn insert_registers_into_db() {
        let mut db = StatsDatabase::default();
        let mut store = CustomTemplateStore::default();
        store.insert(make_template("Custom_001"), &mut db);
        assert!(db.templates.contains_key("Custom_001"));
        assert!(store.owns("Custom_001"));
    }

    #[test]
    fn clear_removes_ownership() {
        let mut db = StatsDatabase::default();
        let mut store = CustomTemplateStore::default();
        store.insert(make_template("Custom_001"), &mut db);
        store.clear(&mut db);
        assert!(!db.templates.contains_key("Custom_001"));
        assert!(!store.owns("Custom_001"));
    }

    #[test]
    fn fresh_id_avoids_collisions() {
        let mut db = StatsDatabase::default();
        let mut store = CustomTemplateStore::default();
        store.insert(make_template("Custom_001"), &mut db);
        store.insert(make_template("Custom_002"), &mut db);
        let next = store.fresh_id(&db);
        assert_eq!(next, "Custom_003");

        db.templates
            .insert("Custom_003".into(), make_template("Custom_003"));
        let next = store.fresh_id(&db);
        assert_eq!(next, "Custom_004");
    }

    #[test]
    fn json_round_trip() {
        let mut db = StatsDatabase::default();
        let mut store = CustomTemplateStore::default();
        store.insert(make_template("Alpha"), &mut db);
        store.insert(make_template("Bravo"), &mut db);
        let json = store.to_json().expect("non-empty store serializes");

        let mut db2 = StatsDatabase::default();
        let mut store2 = CustomTemplateStore::default();
        store2.load_from_json(&json, &mut db2);

        assert!(store2.owns("Alpha"));
        assert!(store2.owns("Bravo"));
        assert!(db2.templates.contains_key("Alpha"));
        assert!(db2.templates.contains_key("Bravo"));
    }
}
