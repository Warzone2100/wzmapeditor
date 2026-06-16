//! PIE parse and connector extraction.

use std::path::Path;

use wz_pie::PieModel;

use super::texture_loader::{TexturePageData, load_texture_simple, resolve_tcmask_simple};

/// Cached parsed PIE model that has not yet been uploaded to the GPU.
pub(crate) struct ParsedModel {
    pub(crate) pie: PieModel,
    pub(crate) texture_data: Option<TexturePageData>,
    pub(crate) tcmask_data: Option<TexturePageData>,
    pub(crate) normal_data: Option<TexturePageData>,
    pub(crate) specular_data: Option<TexturePageData>,
}

/// Read a PIE file and resolve its diffuse + tcmask textures.
///
/// Synchronous path. The background path uses `prepare_model_offline` in
/// `background.rs` and also probes for normal and specular maps via the
/// prebuilt file index.
pub(crate) fn load_pie_sync(
    assets: &dyn crate::assets::AssetSource,
    pie_path: &Path,
    tileset_index: usize,
) -> Option<ParsedModel> {
    let Some(content) = assets.text(pie_path) else {
        log::warn!("Failed to read PIE file {}", pie_path.display());
        return None;
    };

    let pie = match wz_pie::parse_pie(&content) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Failed to parse PIE file {}: {}", pie_path.display(), e);
            return None;
        }
    };

    let tex_page = pie
        .texture_pages
        .get(tileset_index)
        .filter(|s| !s.is_empty())
        .unwrap_or(&pie.texture_page);
    let texture_data = load_texture_simple(assets, tex_page);

    if texture_data.is_none() {
        log::warn!(
            "No texture loaded for '{}' (page='{}', tileset_idx={}, pages={:?})",
            pie_path.display(),
            tex_page,
            tileset_index,
            pie.texture_pages,
        );
    }

    let tcmask_data = resolve_tcmask_simple(assets, &pie, tex_page, tileset_index);

    Some(ParsedModel {
        pie,
        texture_data,
        tcmask_data,
        normal_data: None,
        specular_data: None,
    })
}

/// Connector positions from a PIE model's first level.
///
/// PIE connectors are `(x, z_forward, y_height)` in Z-up world space.
/// Vertex space is Y-up, so we swap to `(x, y_height, z_forward)`
/// matching WZ2100's `.xzy()` swizzle, then negate Z to match the
/// mesh-level Z-flip (PIE Z=north, editor Z=south).
pub(crate) fn extract_connectors(pie: &PieModel) -> Vec<glam::Vec3> {
    pie.levels
        .first()
        .map(|level| {
            level
                .connectors
                .iter()
                .map(|c| glam::Vec3::new(c.x, c.z, -c.y))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn extract_connectors_from_pie(pie: &PieModel) -> Vec<glam::Vec3> {
    extract_connectors(pie)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_swizzle_swaps_y_and_z() {
        let pie = PieModel {
            version: 3,
            model_type: 0x200,
            texture_page: String::new(),
            texture_width: 256,
            texture_height: 256,
            texture_pages: Vec::new(),
            tcmask_pages: Vec::new(),
            normal_page: None,
            specular_page: None,
            event_page: None,
            levels: vec![wz_pie::PieLevel {
                vertices: vec![],
                polygons: vec![],
                connectors: vec![
                    glam::Vec3::new(10.0, 20.0, 30.0),
                    glam::Vec3::new(-5.0, 0.0, 15.0),
                ],
            }],
        };

        let connectors = extract_connectors_from_pie(&pie);
        assert_eq!(connectors.len(), 2);
        assert_eq!(connectors[0], glam::Vec3::new(10.0, 30.0, -20.0));
        assert_eq!(connectors[1], glam::Vec3::new(-5.0, 15.0, 0.0));
    }

    #[test]
    fn connector_empty_model_returns_empty() {
        let pie = PieModel {
            version: 3,
            model_type: 0x200,
            texture_page: String::new(),
            texture_width: 256,
            texture_height: 256,
            texture_pages: Vec::new(),
            tcmask_pages: Vec::new(),
            normal_page: None,
            specular_page: None,
            event_page: None,
            levels: vec![],
        };
        assert!(extract_connectors_from_pie(&pie).is_empty());
    }
}
