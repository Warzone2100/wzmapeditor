//! Central stats database loaded from data/base/stats/.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::StatsError;
use crate::bodies::BodyStats;
use crate::features::FeatureStats;
use crate::propulsion::PropulsionStats;
use crate::structures::StructureStats;
use crate::templates::TemplateStats;
use crate::terrain_table::TerrainTable;
use crate::turrets::{BrainStats, ConstructStats, EcmStats, RepairStats, SensorStats};
use crate::weapons::WeaponStats;

/// Central database of all game stats, loaded from the data/base/stats/ directory.
#[derive(Debug, Default, Clone)]
pub struct StatsDatabase {
    pub structures: HashMap<String, StructureStats>,
    pub features: HashMap<String, FeatureStats>,
    pub bodies: HashMap<String, BodyStats>,
    pub propulsion: HashMap<String, PropulsionStats>,
    pub weapons: HashMap<String, WeaponStats>,
    pub templates: HashMap<String, TemplateStats>,
    pub construct: HashMap<String, ConstructStats>,
    pub sensor: HashMap<String, SensorStats>,
    pub ecm: HashMap<String, EcmStats>,
    pub repair: HashMap<String, RepairStats>,
    pub brain: HashMap<String, BrainStats>,
    pub terrain_table: Option<TerrainTable>,
    /// Template ids present in `mp/stats/templates.json`. Templates outside
    /// this set work only in campaign maps and silently fail to spawn in
    /// skirmish. Empty when `mp/stats/` was never merged.
    pub mp_template_ids: HashSet<String>,
    /// Same idea for structures (e.g. `GuardTower1MG` is base-only).
    pub mp_structure_ids: HashSet<String>,
}

const STRUCTURES_FILE: &str = "structure.json";
const FEATURES_FILE: &str = "features.json";
const BODIES_FILE: &str = "body.json";
const PROPULSION_FILE: &str = "propulsion.json";
const WEAPONS_FILE: &str = "weapons.json";
const TEMPLATES_FILE: &str = "templates.json";
const CONSTRUCT_FILE: &str = "construction.json";
const SENSOR_FILE: &str = "sensor.json";
const ECM_FILE: &str = "ecm.json";
const REPAIR_FILE: &str = "repair.json";
const BRAIN_FILE: &str = "brain.json";
const TERRAIN_TABLE_FILE: &str = "terraintable.json";

impl StatsDatabase {
    /// Load all stats, reading each JSON file's contents through `read`.
    ///
    /// `read(name)` returns the named file's UTF-8 contents, `Ok(None)` when it
    /// is absent, or an error when it exists but cannot be read. This is the
    /// source-agnostic core of [`load_from_dir`](Self::load_from_dir): the web
    /// build supplies a closure backed by an in-memory archive instead of disk.
    pub fn load_from_source<F>(read: F) -> Result<Self, StatsError>
    where
        F: Fn(&str) -> Result<Option<String>, StatsError>,
    {
        let mut db = StatsDatabase::default();
        let load_json = read;

        if let Some(content) = load_json(STRUCTURES_FILE)? {
            db.structures = crate::structures::load_structures(&content)?;
            log::info!("Loaded {} structure stats", db.structures.len());
        }

        if let Some(content) = load_json(FEATURES_FILE)? {
            db.features = crate::features::load_features(&content)?;
            log::info!("Loaded {} feature stats", db.features.len());
        }

        if let Some(content) = load_json(BODIES_FILE)? {
            db.bodies = crate::bodies::load_bodies(&content)?;
            log::info!("Loaded {} body stats", db.bodies.len());
        }

        if let Some(content) = load_json(PROPULSION_FILE)? {
            db.propulsion = crate::propulsion::load_propulsion(&content)?;
            log::info!("Loaded {} propulsion stats", db.propulsion.len());
        }

        if let Some(content) = load_json(WEAPONS_FILE)? {
            db.weapons = crate::weapons::load_weapons(&content)?;
            log::info!("Loaded {} weapon stats", db.weapons.len());
        }

        if let Some(content) = load_json(TEMPLATES_FILE)? {
            db.templates = crate::templates::load_templates(&content)?;
            log::info!("Loaded {} template stats", db.templates.len());
        }

        if let Some(content) = load_json(CONSTRUCT_FILE)? {
            db.construct = crate::turrets::load_construct(&content)?;
            log::info!("Loaded {} construction stats", db.construct.len());
        }

        if let Some(content) = load_json(SENSOR_FILE)? {
            db.sensor = crate::turrets::load_sensor(&content)?;
            log::info!("Loaded {} sensor stats", db.sensor.len());
        }

        if let Some(content) = load_json(ECM_FILE)? {
            db.ecm = crate::turrets::load_ecm(&content)?;
            log::info!("Loaded {} ECM stats", db.ecm.len());
        }

        if let Some(content) = load_json(REPAIR_FILE)? {
            db.repair = crate::turrets::load_repair(&content)?;
            log::info!("Loaded {} repair stats", db.repair.len());
        }

        if let Some(content) = load_json(BRAIN_FILE)? {
            db.brain = crate::turrets::load_brain(&content)?;
            log::info!("Loaded {} brain stats", db.brain.len());
        }

        if let Some(content) = load_json(TERRAIN_TABLE_FILE)? {
            db.terrain_table = Some(crate::terrain_table::load_terrain_table(&content)?);
        }

        Ok(db)
    }

    /// Load all stats from the given stats directory.
    pub fn load_from_dir(stats_dir: impl AsRef<Path>) -> Result<Self, StatsError> {
        let stats_dir = stats_dir.as_ref();
        Self::load_from_source(|name| read_stats_file(stats_dir, name))
    }

    /// Merge stats read through `read` on top of the current database.
    ///
    /// Source-agnostic core of [`merge_from_dir`](Self::merge_from_dir); see it
    /// for the overriding and id-tracking semantics.
    pub fn merge_from_source<F>(&mut self, read: F) -> Result<(), StatsError>
    where
        F: Fn(&str) -> Result<Option<String>, StatsError>,
    {
        let load_json = read;

        let mut merged = 0usize;
        if let Some(content) = load_json(STRUCTURES_FILE)? {
            let extra = crate::structures::load_structures(&content)?;
            merged += extra.len();
            self.mp_structure_ids.extend(extra.keys().cloned());
            self.structures.extend(extra);
        }
        if let Some(content) = load_json(FEATURES_FILE)? {
            let extra = crate::features::load_features(&content)?;
            merged += extra.len();
            self.features.extend(extra);
        }
        if let Some(content) = load_json(BODIES_FILE)? {
            let extra = crate::bodies::load_bodies(&content)?;
            merged += extra.len();
            self.bodies.extend(extra);
        }
        if let Some(content) = load_json(PROPULSION_FILE)? {
            let extra = crate::propulsion::load_propulsion(&content)?;
            merged += extra.len();
            self.propulsion.extend(extra);
        }
        if let Some(content) = load_json(WEAPONS_FILE)? {
            let extra = crate::weapons::load_weapons(&content)?;
            merged += extra.len();
            self.weapons.extend(extra);
        }
        if let Some(content) = load_json(TEMPLATES_FILE)? {
            let extra = crate::templates::load_templates(&content)?;
            merged += extra.len();
            self.mp_template_ids.extend(extra.keys().cloned());
            self.templates.extend(extra);
        }
        if let Some(content) = load_json(CONSTRUCT_FILE)? {
            let extra = crate::turrets::load_construct(&content)?;
            merged += extra.len();
            self.construct.extend(extra);
        }
        if let Some(content) = load_json(SENSOR_FILE)? {
            let extra = crate::turrets::load_sensor(&content)?;
            merged += extra.len();
            self.sensor.extend(extra);
        }
        if let Some(content) = load_json(ECM_FILE)? {
            let extra = crate::turrets::load_ecm(&content)?;
            merged += extra.len();
            self.ecm.extend(extra);
        }
        if let Some(content) = load_json(REPAIR_FILE)? {
            let extra = crate::turrets::load_repair(&content)?;
            merged += extra.len();
            self.repair.extend(extra);
        }
        if let Some(content) = load_json(BRAIN_FILE)? {
            let extra = crate::turrets::load_brain(&content)?;
            merged += extra.len();
            self.brain.extend(extra);
        }
        if merged > 0 {
            log::info!("Merged {merged} stat entries");
        }
        Ok(())
    }

    /// Merge an additional stats directory on top, overriding entries with
    /// matching keys. Used to layer `mp/stats/` on top of `base/stats/` so
    /// multiplayer-only components (e.g. Dragon, Wyvern bodies) are
    /// available. Template and structure ids seen here are recorded in
    /// `mp_template_ids` / `mp_structure_ids` so callers can distinguish
    /// skirmish-allowed entries from campaign-only ones.
    pub fn merge_from_dir(&mut self, stats_dir: impl AsRef<Path>) -> Result<(), StatsError> {
        let stats_dir = stats_dir.as_ref();
        self.merge_from_source(|name| read_stats_file(stats_dir, name))
    }

    /// True when `mp/stats/` has been merged. When false, callers cannot
    /// distinguish campaign-only entries from skirmish-allowed ones.
    pub fn has_mp_overlay(&self) -> bool {
        !self.mp_template_ids.is_empty() || !self.mp_structure_ids.is_empty()
    }

    /// True when `template_id` will spawn in a skirmish/multiplayer map.
    /// Falls back to true when no mp overlay is loaded, to avoid
    /// over-filtering in setups that lack `mp/stats/`.
    pub fn template_allowed_in_mp(&self, template_id: &str) -> bool {
        !self.has_mp_overlay() || self.mp_template_ids.contains(template_id)
    }

    /// True when `structure_id` is loaded by the game in skirmish/multiplayer.
    pub fn structure_allowed_in_mp(&self, structure_id: &str) -> bool {
        !self.has_mp_overlay() || self.mp_structure_ids.contains(structure_id)
    }

    /// Primary weapon stats for a structure. `None` for weapon-less
    /// structures (research labs, power generators) or when the referenced
    /// weapon id is missing from the loaded weapons table.
    pub fn weapon_for_structure(&self, structure_id: &str) -> Option<&WeaponStats> {
        self.structures
            .get(structure_id)
            .and_then(|s| s.weapons.first())
            .and_then(|w| self.weapons.get(w))
    }
}

/// Read `name` from `dir` as text, `Ok(None)` when the file is absent.
fn read_stats_file(dir: &Path, name: &str) -> Result<Option<String>, StatsError> {
    let path = dir.join(name);
    if path.exists() {
        let content =
            std::fs::read_to_string(&path).map_err(|e| StatsError::Io { path, source: e })?;
        Ok(Some(content))
    } else {
        Ok(None)
    }
}
