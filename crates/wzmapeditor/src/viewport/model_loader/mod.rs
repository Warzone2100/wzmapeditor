//! PIE model loading, texture resolution, and GPU upload.
//!
//! Object name to stats to `imd_name` to PIE file. Parses, decodes
//! textures, and uploads to the GPU. Bulk loading runs on background
//! threads; only the GPU upload stays on the main thread.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;

use wz_stats::StatsDatabase;

use super::pie_mesh;
use super::renderer::EditorRenderer;

mod background;
mod cache;
mod file_finder;
mod pie_loader;
mod stats_mapper;
mod texture_loader;

use cache::LoaderCache;
use pie_loader::{ParsedModel, extract_connectors, load_pie_sync};
use texture_loader::TexturePageData;

pub use background::PreparedModel;
pub use stats_mapper::{DroidComponents, StructureWeapons};

/// Manages loading and caching of PIE models from the WZ2100 data directory.
pub struct ModelLoader {
    /// `data_dir` root (e.g. `/path/to/warzone2100/data`).
    data_dir: PathBuf,
    cache: LoaderCache,
}

impl std::fmt::Debug for ModelLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelLoader")
            .field("data_dir", &self.data_dir)
            .field("imd_mappings", &self.cache.imd_map_len())
            .field("uploaded", &self.cache.uploaded_len())
            .field("parsed_cache", &self.cache.parsed_cache_len())
            .field("not_found_cache", &self.cache.not_found_cache_len())
            .finish_non_exhaustive()
    }
}

impl ModelLoader {
    /// Create a new model loader from the asset source and stats database.
    pub fn new(assets: Arc<dyn crate::assets::AssetSource>, stats: &StatsDatabase) -> Self {
        Self {
            assets,
            cache: LoaderCache::new(stats),
            pie_file_index: None,
        }
    }

    /// Set the tileset index for multi-texture PIE models.
    ///
    /// 0 = Arizona, 1 = Urban, 2 = Rockies. Uploaded models are cleared
    /// so they reload with the right texture; parsed geometry and
    /// connectors are kept since they don't depend on tileset.
    pub fn set_tileset(&mut self, index: usize) {
        self.cache.set_tileset(index);
    }

    /// Intern a PIE model name to a shared `Arc<str>`.
    ///
    /// First call for a name allocates and stores the `Arc<str>`; later
    /// calls return a ref-bumped clone. Hot-path callers should use the
    /// result as a `HashMap` key instead of cloning a `String`.
    pub fn intern(&mut self, name: &str) -> Arc<str> {
        self.cache.intern(name)
    }

    /// Get the `imd_name` for an object name (structure/feature/droid).
    pub fn imd_for_object(&self, object_name: &str) -> Option<&str> {
        self.cache.imd_for_object(object_name)
    }

    /// Resolve a droid template into its component PIE model names.
    #[expect(
        clippy::unused_self,
        reason = "kept on self for API parity with other resolver methods"
    )]
    pub fn resolve_droid_components(
        &self,
        droid_name: &str,
        stats: &StatsDatabase,
    ) -> Option<DroidComponents> {
        stats_mapper::resolve_droid_components(droid_name, stats)
    }

    /// Resolve a structure name into its weapon/sensor/ECM turret models.
    #[expect(
        clippy::unused_self,
        reason = "kept on self for API parity with other resolver methods"
    )]
    pub fn resolve_structure_weapons(
        &self,
        structure_name: &str,
        stats: &StatsDatabase,
    ) -> Option<StructureWeapons> {
        stats_mapper::resolve_structure_weapons(structure_name, stats)
    }

    /// Get the connector positions from a parsed/cached PIE model.
    pub fn get_connectors(&mut self, imd_name: &str) -> Vec<glam::Vec3> {
        if let Some(cached) = self.cache.get_connectors(imd_name) {
            return cached.to_vec();
        }

        if let Some(parsed) = self.cache.get_parsed(imd_name) {
            let connectors = extract_connectors(&parsed.pie);
            self.cache
                .insert_connectors(imd_name.to_string(), connectors.clone());
            return connectors;
        }

        if let Some(parsed) = self.load_pie(imd_name) {
            let connectors = extract_connectors(&parsed.pie);
            self.cache
                .insert_connectors(imd_name.to_string(), connectors.clone());
            // Keep the parsed entry so a subsequent ensure_model can
            // promote it without re-reading from disk. Single-residency
            // holds because `load_pie` only runs when not yet uploaded.
            self.cache.insert_parsed(imd_name, parsed);
            return connectors;
        }

        Vec::new()
    }

    /// Check whether a model has already been uploaded to the GPU.
    pub fn is_uploaded(&self, imd_name: &str) -> bool {
        self.cache.is_uploaded(imd_name)
    }

    /// Pre-parse PIE files on a background thread to warm the connector cache.
    pub fn precache_connectors_background(
        &self,
        stats: &StatsDatabase,
    ) -> mpsc::Receiver<HashMap<String, Vec<glam::Vec3>>> {
        let already_cached = self.cache.connector_key_snapshot();
        background::precache_connectors_background(self.assets.clone(), stats, &already_cached)
    }

    /// Merge pre-cached connector data into the loader's connector cache.
    pub fn merge_connector_precache(&mut self, cache: HashMap<String, Vec<glam::Vec3>>) {
        let count = cache.len();
        for (name, conns) in cache {
            self.cache.insert_connectors_if_absent(name, conns);
        }
        log::info!("Merged {count} connector precache entries into model loader");
    }

    /// Spawn background threads to prepare models for GPU upload.
    pub fn prepare_models_background(&self, names: Vec<String>) -> mpsc::Receiver<PreparedModel> {
        background::prepare_models_background(
            self.assets.clone(),
            self.cache.tileset_index(),
            names,
        )
    }

    /// Upload a model that was prepared on a background thread.
    pub fn upload_prepared(
        &mut self,
        prepared: PreparedModel,
        renderer: &mut EditorRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let imd_name = &prepared.imd_name;

        if self.cache.is_uploaded(imd_name) {
            return;
        }

        if !prepared.connectors.is_empty() && !self.cache.has_connectors(imd_name) {
            self.cache
                .insert_connectors(imd_name.clone(), prepared.connectors);
        }

        if let Some(ref mesh) = prepared.mesh {
            let has_tcmask = prepared.tcmask_data.is_some();
            let has_normal = prepared.normal_data.is_some();
            let has_specular = prepared.specular_data.is_some();
            let tex = prepared
                .texture_data
                .as_ref()
                .map(TexturePageData::as_page_ref);
            let tcmask = prepared
                .tcmask_data
                .as_ref()
                .map(TexturePageData::as_page_ref);
            let normal = prepared
                .normal_data
                .as_ref()
                .map(TexturePageData::as_page_ref);
            let specular = prepared
                .specular_data
                .as_ref()
                .map(TexturePageData::as_page_ref);
            renderer.upload_model(device, queue, imd_name, mesh, tex, tcmask, normal, specular);
            log::info!(
                "Uploaded model '{}': {} verts, {} indices{}{}{}",
                imd_name,
                mesh.vertices.len(),
                mesh.indices.len(),
                if has_tcmask { " +tcmask" } else { "" },
                if has_normal { " +normal" } else { "" },
                if has_specular { " +specular" } else { "" },
            );
        }

        self.cache.mark_uploaded(imd_name);
    }

    /// Ensure a model is loaded and uploaded to the GPU renderer.
    /// Returns true if the model is (now) available for rendering.
    pub fn ensure_model(
        &mut self,
        imd_name: &str,
        renderer: &mut EditorRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> bool {
        if self.cache.is_uploaded(imd_name) {
            return true;
        }

        if !self.cache.has_parsed(imd_name) {
            if let Some(parsed) = self.load_pie(imd_name) {
                if !self.cache.insert_parsed(imd_name, parsed) {
                    // Another path on this thread already uploaded; discard the parse.
                    return true;
                }
            } else {
                log::warn!("Model '{imd_name}' not found, skipping");
                self.cache.mark_uploaded(imd_name);
                return false;
            }
        }

        if !self.cache.has_connectors(imd_name)
            && let Some(parsed) = self.cache.get_parsed(imd_name)
        {
            let connectors = extract_connectors(&parsed.pie);
            self.cache
                .insert_connectors(imd_name.to_string(), connectors);
        }

        let Some(parsed) = self.cache.promote_to_uploaded(imd_name) else {
            return false;
        };

        if let Some(mesh) = pie_mesh::build_mesh(&parsed.pie) {
            let has_tcmask = parsed.tcmask_data.is_some();
            let tex = parsed
                .texture_data
                .as_ref()
                .map(TexturePageData::as_page_ref);
            let tcmask = parsed
                .tcmask_data
                .as_ref()
                .map(TexturePageData::as_page_ref);
            let normal = parsed
                .normal_data
                .as_ref()
                .map(TexturePageData::as_page_ref);
            let specular = parsed
                .specular_data
                .as_ref()
                .map(TexturePageData::as_page_ref);
            renderer.upload_model(
                device, queue, imd_name, &mesh, tex, tcmask, normal, specular,
            );
            log::info!(
                "Uploaded model '{}': {} verts, {} indices{}",
                imd_name,
                mesh.vertices.len(),
                mesh.indices.len(),
                if has_tcmask { " +tcmask" } else { "" },
            );
            return true;
        }

        log::warn!("Model '{imd_name}' mesh build failed, skipping");
        false
    }

    /// Parse a PIE file synchronously. Marks the name not-found on
    /// failure to short-circuit per-frame rescans.
    fn load_pie(&mut self, imd_name: &str) -> Option<ParsedModel> {
        if self.cache.is_not_found(imd_name) {
            return None;
        }
        let Some(pie_path) = file_finder::find_pie_file(&self.data_dir, imd_name) else {
            log::debug!("PIE file not found: {imd_name}");
            self.cache.mark_not_found(imd_name);
            return None;
        };
        load_pie_sync(&self.data_dir, &pie_path, self.cache.tileset_index())
    }
}
