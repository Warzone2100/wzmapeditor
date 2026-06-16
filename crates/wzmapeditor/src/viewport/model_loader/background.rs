//! Background-thread PIE preparation: parse, texture decode, mesh build.
//! Only the fast GPU buffer creation remains for the main thread via
//! `ModelLoader::upload_prepared`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use wz_stats::StatsDatabase;

use super::file_finder::{build_pie_file_index, lookup_in_index};
use super::pie_loader::extract_connectors;
use super::texture_loader::{
    TexturePageData, load_texture_offline, resolve_normal_specular_offline, resolve_tcmask_offline,
};
use crate::viewport::pie_mesh;

/// Model fully prepared on a background thread: built mesh, decoded
/// textures, and connector positions. Ready for GPU upload.
pub struct PreparedModel {
    pub imd_name: String,
    pub mesh: Option<pie_mesh::ModelMesh>,
    pub(crate) texture_data: Option<TexturePageData>,
    pub(crate) tcmask_data: Option<TexturePageData>,
    pub(crate) normal_data: Option<TexturePageData>,
    pub(crate) specular_data: Option<TexturePageData>,
    pub connectors: Vec<glam::Vec3>,
}

/// Spawn background workers producing a `PreparedModel` for each
/// requested IMD. Fans out across `min(cores, 8)` threads sharing a
/// single prebuilt file index.
pub(crate) fn prepare_models_background(
    assets: Arc<dyn crate::assets::AssetSource>,
    tileset_index: usize,
    names: Vec<String>,
) -> mpsc::Receiver<PreparedModel> {
    let (tx, rx) = mpsc::channel();

    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        let file_index = Arc::new(build_pie_file_index(assets.as_ref()));
        log::info!("PIE file index: {} files found", file_index.len());

        let num_threads = std::thread::available_parallelism()
            .map_or(4, std::num::NonZero::get)
            .min(8);
        let chunk_size = names.len().div_ceil(num_threads.max(1));

        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for chunk in names.chunks(chunk_size.max(1)) {
                let tx = tx.clone();
                let file_index = Arc::clone(&file_index);
                let assets = Arc::clone(&assets);
                let chunk: Vec<String> = chunk.to_vec();

                handles.push(s.spawn(move || {
                    for imd_name in chunk {
                        let prepared = prepare_model_offline(
                            &imd_name,
                            &file_index,
                            assets.as_ref(),
                            tileset_index,
                        );
                        if tx.send(prepared).is_err() {
                            break;
                        }
                    }
                }));
            }
            for h in handles {
                let _ = h.join();
            }
        });
    });

    rx
}

/// Spawn a background thread that pre-parses every PIE referenced by stats
/// and ships back a connector-only map for the main thread to merge.
pub(crate) fn precache_connectors_background(
    data_dir: &Path,
    stats: &StatsDatabase,
    already_cached: &HashSet<String>,
) -> mpsc::Receiver<HashMap<String, Vec<glam::Vec3>>> {
    let (tx, rx) = mpsc::channel();

    let mut pie_names: HashSet<String> = HashSet::new();

    for ss in stats.structures.values() {
        if let Some(imd) = ss.pie_model() {
            pie_names.insert(imd.to_string());
        }
        for weapon_name in &ss.weapons {
            if let Some(ws) = stats.weapons.get(weapon_name)
                && let Some(ref mount) = ws.mount_model
            {
                pie_names.insert(mount.clone());
            }
        }
    }

    for tmpl in stats.templates.values() {
        if let Some(body_stat) = stats.bodies.get(&tmpl.body)
            && let Some(ref body_imd) = body_stat.model
        {
            pie_names.insert(body_imd.clone());
        }
        for weapon_name in &tmpl.weapons {
            if let Some(ws) = stats.weapons.get(weapon_name)
                && let Some(ref mount) = ws.mount_model
            {
                pie_names.insert(mount.clone());
            }
        }
    }

    let names: Vec<String> = pie_names
        .into_iter()
        .filter(|n| !already_cached.contains(n))
        .collect();

    let data_dir = data_dir.to_path_buf();

    std::thread::spawn(move || {
        let file_index = build_pie_file_index(&data_dir);
        log::info!(
            "Connector precache: parsing {} PIE files on background thread",
            names.len()
        );

        let mut connectors = HashMap::new();
        for name in &names {
            let pie_path = lookup_in_index(&file_index, name);

            let Some(pie_path) = pie_path else {
                connectors.insert(name.clone(), Vec::new());
                continue;
            };

            let Some(content) = assets.text(pie_path) else {
                connectors.insert(name.clone(), Vec::new());
                continue;
            };

            match wz_pie::parse_pie(&content) {
                Ok(pie) => {
                    connectors.insert(name.clone(), extract_connectors(&pie));
                }
                Err(_) => {
                    connectors.insert(name.clone(), Vec::new());
                }
            }
        }

        log::info!("Connector precache complete: {} entries", connectors.len());
        let _ = tx.send(connectors);
    });

    rx
}

/// Prepare a single model on a background thread (no GPU access).
/// Steps: file lookup, read PIE, parse, load textures, build mesh,
/// extract connectors. Result is ready for `upload_prepared`.
fn prepare_model_offline(
    imd_name: &str,
    file_index: &HashMap<String, PathBuf>,
    assets: &dyn crate::assets::AssetSource,
    tileset_index: usize,
) -> PreparedModel {
    let Some(pie_path) = lookup_in_index(file_index, imd_name) else {
        log::debug!("PIE file not found: {imd_name}");
        return empty_prepared(imd_name);
    };

    let Some(content) = assets.text(pie_path) else {
        log::warn!("Failed to read PIE file {}", pie_path.display());
        return empty_prepared(imd_name);
    };

    let pie = match wz_pie::parse_pie(&content) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Failed to parse PIE file {}: {}", pie_path.display(), e);
            return empty_prepared(imd_name);
        }
    };

    let tex_page = pie
        .texture_pages
        .get(tileset_index)
        .filter(|s| !s.is_empty())
        .unwrap_or(&pie.texture_page);

    let texture_data = load_texture_offline(assets, tex_page, file_index);
    let tcmask_data = resolve_tcmask_offline(&pie, tex_page, assets, tileset_index, file_index);

    // HQ normal/specular maps live in terrain_overrides/high.wz; check
    // explicit PIE directives first, then auto-detect by suffix.
    let normal_data = resolve_normal_specular_offline(
        pie.normal_page.as_ref(),
        tex_page,
        "_nm",
        assets,
        file_index,
    );
    let specular_data = resolve_normal_specular_offline(
        pie.specular_page.as_ref(),
        tex_page,
        "_sm",
        assets,
        file_index,
    );

    let connectors = extract_connectors(&pie);
    let mesh = pie_mesh::build_mesh(&pie);

    PreparedModel {
        imd_name: imd_name.to_string(),
        mesh,
        texture_data,
        tcmask_data,
        normal_data,
        specular_data,
        connectors,
    }
}

fn empty_prepared(imd_name: &str) -> PreparedModel {
    PreparedModel {
        imd_name: imd_name.to_string(),
        mesh: None,
        texture_data: None,
        tcmask_data: None,
        normal_data: None,
        specular_data: None,
        connectors: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use wz_pie::PieModel;

    fn fs(dir: &Path) -> crate::assets::FsAssetSource {
        crate::assets::FsAssetSource::new(dir.to_path_buf())
    }

    fn write_test_png(path: &Path) {
        let img = image::RgbaImage::from_raw(2, 2, vec![255u8; 16]).unwrap();
        img.save(path).unwrap();
    }

    #[test]
    fn resolve_tcmask_offline_finds_explicit_tcmask() {
        let dir = tempfile::tempdir().unwrap();
        let texpages = dir.path().join("base/texpages");
        std::fs::create_dir_all(&texpages).unwrap();
        write_test_png(&texpages.join("page-11_tcmask.png"));

        let pie = PieModel {
            version: 4,
            model_type: 0x200,
            texture_page: "page-11-player-buildings.png".to_string(),
            texture_width: 256,
            texture_height: 256,
            texture_pages: vec!["page-11-player-buildings.png".to_string()],
            tcmask_pages: vec!["page-11_tcmask.png".to_string()],
            normal_page: None,
            specular_page: None,
            event_page: None,
            levels: vec![],
        };

        let file_index = HashMap::new();
        let result = resolve_tcmask_offline(
            &pie,
            "page-11-player-buildings.png",
            &fs(dir.path()),
            0,
            &file_index,
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap().page_name, "page-11_tcmask.png");
    }

    #[test]
    fn resolve_tcmask_offline_auto_detects_from_page_name() {
        let dir = tempfile::tempdir().unwrap();
        let texpages = dir.path().join("base/texpages");
        std::fs::create_dir_all(&texpages).unwrap();
        write_test_png(&texpages.join("page-11_tcmask.png"));

        let pie = PieModel {
            version: 3,
            model_type: 0x200,
            texture_page: "page-11-player-buildings.png".to_string(),
            texture_width: 256,
            texture_height: 256,
            texture_pages: vec![],
            tcmask_pages: vec![],
            normal_page: None,
            specular_page: None,
            event_page: None,
            levels: vec![],
        };

        let file_index = HashMap::new();
        let result = resolve_tcmask_offline(
            &pie,
            "page-11-player-buildings.png",
            &fs(dir.path()),
            0,
            &file_index,
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap().page_name, "page-11_tcmask.png");
    }

    #[test]
    fn resolve_tcmask_offline_returns_none_when_not_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("base/texpages")).unwrap();

        let pie = PieModel {
            version: 3,
            model_type: 0x200,
            texture_page: "page-99-nothing.png".to_string(),
            texture_width: 256,
            texture_height: 256,
            texture_pages: vec![],
            tcmask_pages: vec![],
            normal_page: None,
            specular_page: None,
            event_page: None,
            levels: vec![],
        };

        let file_index = HashMap::new();
        let result =
            resolve_tcmask_offline(&pie, "page-99-nothing.png", &fs(dir.path()), 0, &file_index);
        assert!(result.is_none());
    }

    #[test]
    fn prepare_model_offline_carries_texture_page_names() {
        let dir = tempfile::tempdir().unwrap();
        let texpages = dir.path().join("base/texpages");
        let structs = dir.path().join("base/structs");
        std::fs::create_dir_all(&texpages).unwrap();
        std::fs::create_dir_all(&structs).unwrap();

        let pie_content = "\
PIE 3
TYPE 200
TEXTURE 0 page-11-player-buildings.png 256 256
LEVELS 1
LEVEL 1
POINTS 3
\t0 0 0
\t10 0 0
\t0 0 10
POLYGONS 1
\t200 3 0 1 2 0.0 0.0 1.0 0.0 0.0 1.0
CONNECTORS 0
";
        std::fs::write(structs.join("test.pie"), pie_content).unwrap();
        write_test_png(&texpages.join("page-11-player-buildings.png"));
        write_test_png(&texpages.join("page-11_tcmask.png"));

        let mut file_index = HashMap::new();
        file_index.insert(
            "test.pie".to_string(),
            PathBuf::from("base/structs/test.pie"),
        );

        let prepared = prepare_model_offline("test.pie", &file_index, &fs(dir.path()), 0);
        assert_eq!(prepared.imd_name, "test.pie");
        assert!(prepared.mesh.is_some());

        let tex = prepared
            .texture_data
            .as_ref()
            .expect("diffuse texture should be loaded");
        assert_eq!(tex.page_name, "page-11-player-buildings.png");

        let tcmask = prepared
            .tcmask_data
            .as_ref()
            .expect("tcmask should be auto-detected");
        assert_eq!(tcmask.page_name, "page-11_tcmask.png");
    }

    #[test]
    fn prepare_model_offline_missing_pie_returns_empty() {
        let file_index = HashMap::new();
        let dir = tempfile::tempdir().unwrap();
        let prepared = prepare_model_offline("nonexistent.pie", &file_index, &fs(dir.path()), 0);
        assert_eq!(prepared.imd_name, "nonexistent.pie");
        assert!(prepared.mesh.is_none());
        assert!(prepared.texture_data.is_none());
    }

    #[test]
    fn texture_page_data_different_tilesets_have_different_keys() {
        let dir = tempfile::tempdir().unwrap();
        let texpages = dir.path().join("base/texpages");
        let structs = dir.path().join("base/structs");
        std::fs::create_dir_all(&texpages).unwrap();
        std::fs::create_dir_all(&structs).unwrap();

        let pie_content = "\
PIE 4
TYPE 200
TEXTURE 0 page-9-bases.png
TEXTURE 1 page-9-bases-urban.png
TEXTURE 2 page-9-bases-rockies.png
LEVELS 1
LEVEL 1
POINTS 3
\t0 0 0
\t10 0 0
\t0 0 10
POLYGONS 1
\t200 3 0 1 2 0.0 0.0 1.0 0.0 0.0 1.0
CONNECTORS 0
";
        std::fs::write(structs.join("multi.pie"), pie_content).unwrap();
        write_test_png(&texpages.join("page-9-bases.png"));
        write_test_png(&texpages.join("page-9-bases-urban.png"));
        write_test_png(&texpages.join("page-9-bases-rockies.png"));

        let mut file_index = HashMap::new();
        file_index.insert(
            "multi.pie".to_string(),
            PathBuf::from("base/structs/multi.pie"),
        );
        let assets = fs(dir.path());

        let arizona = prepare_model_offline("multi.pie", &file_index, &assets, 0);
        let az_page = arizona.texture_data.as_ref().unwrap().page_name.clone();

        let urban = prepare_model_offline("multi.pie", &file_index, &assets, 1);
        let ur_page = urban.texture_data.as_ref().unwrap().page_name.clone();

        let rockies = prepare_model_offline("multi.pie", &file_index, &assets, 2);
        let rk_page = rockies.texture_data.as_ref().unwrap().page_name.clone();

        assert_eq!(az_page, "page-9-bases.png");
        assert_eq!(ur_page, "page-9-bases-urban.png");
        assert_eq!(rk_page, "page-9-bases-rockies.png");
        assert_ne!(az_page, ur_page);
        assert_ne!(ur_page, rk_page);
    }
}
