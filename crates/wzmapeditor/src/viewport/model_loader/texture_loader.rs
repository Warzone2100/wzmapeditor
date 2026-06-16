//! Texture decode and resolution for PIE models.
//!
//! Prefers KTX2 from high.wz when present, falling back to PNG via
//! `image`. tcmask, normal, and specular page names come from explicit
//! PIE directives or are auto-detected by appending the conventional
//! suffix to the diffuse page name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use wz_pie::PieModel;

use super::super::renderer;
use crate::assets::AssetSource;

/// Subdirectory under the data root holding model texture pages.
const TEXPAGES_REL: &str = "base/texpages";

/// Decoded texture page plus its source page name (used as cache key).
pub(crate) struct TexturePageData {
    /// Cache key (e.g. "page-11-player-buildings.png").
    pub(crate) page_name: String,
    pub(crate) rgba: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl TexturePageData {
    pub(crate) fn as_page_ref(&self) -> renderer::TexturePageRef<'_> {
        renderer::TexturePageRef {
            page_name: &self.page_name,
            rgba: &self.rgba,
            width: self.width,
            height: self.height,
        }
    }
}

/// Decode image bytes as RGBA8 with `page_name` as the cache key.
pub(crate) fn load_image_rgba(bytes: &[u8], page_name: &str) -> Option<TexturePageData> {
    match image::load_from_memory(bytes) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let width = rgba.width();
            let height = rgba.height();
            Some(TexturePageData {
                page_name: page_name.to_string(),
                rgba: rgba.into_raw(),
                width,
                height,
            })
        }
        Err(e) => {
            log::warn!("Failed to decode texture {page_name}: {e}");
            None
        }
    }
}

/// Load a texture PNG by its `texture_page` name. Sync, no file index.
pub(crate) fn load_texture_simple(
    assets: &dyn AssetSource,
    texture_page: &str,
) -> Option<TexturePageData> {
    let rel = Path::new(TEXPAGES_REL).join(texture_page);
    if let Some(bytes) = assets.bytes(&rel) {
        return load_image_rgba(&bytes, texture_page);
    }

    let with_png = rel.with_extension("png");
    if let Some(bytes) = assets.bytes(&with_png) {
        let normalized = with_png
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string);
        return load_image_rgba(&bytes, normalized.as_deref().unwrap_or(texture_page));
    }

    log::debug!("Texture not found: {TEXPAGES_REL}/{texture_page}");
    None
}

/// Load a texture by name. Tries KTX2 (HQ) first, then PNG, then a
/// prebuilt file index for textures that live outside `base/texpages/`.
pub(crate) fn load_texture_offline(
    assets: &dyn AssetSource,
    texture_page: &str,
    file_index: &HashMap<String, PathBuf>,
) -> Option<TexturePageData> {
    let texpages_rel = Path::new(TEXPAGES_REL);

    // Diffuse KTX2 from high.wz is linear; convert to sRGB so the
    // Rgba8UnormSrgb GPU format round-trips. Normal/specular maps stay
    // linear and upload as Rgba8Unorm.
    let is_diffuse = !texture_page.contains("_nm")
        && !texture_page.contains("_sm")
        && !texture_page.contains("_tcmask");
    let ktx2_name = texture_page.replace(".png", ".ktx2");
    if let Some(bytes) = assets.bytes(&texpages_rel.join(&ktx2_name)) {
        match renderer::load_ktx2_as_rgba_bytes(&bytes) {
            Ok(mut rgba) => {
                if is_diffuse {
                    renderer::linear_to_srgb(&mut rgba);
                }
                log::info!(
                    "Loaded KTX2 model texture: {ktx2_name} ({}x{}, srgb={is_diffuse})",
                    rgba.width(),
                    rgba.height(),
                );
                return Some(TexturePageData {
                    page_name: texture_page.to_string(),
                    width: rgba.width(),
                    height: rgba.height(),
                    rgba: rgba.into_raw(),
                });
            }
            Err(e) => {
                log::warn!("KTX2 decode failed for {ktx2_name}: {e}");
            }
        }
    }

    if let Some(bytes) = assets.bytes(&texpages_rel.join(texture_page)) {
        return load_image_rgba(&bytes, texture_page);
    }
    let with_png = texpages_rel.join(texture_page).with_extension("png");
    if let Some(bytes) = assets.bytes(&with_png) {
        let normalized = with_png
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string);
        return load_image_rgba(&bytes, normalized.as_deref().unwrap_or(texture_page));
    }
    let indexed = file_index
        .get(texture_page)
        .or_else(|| file_index.get(&texture_page.to_lowercase()));
    if let Some(rel) = indexed
        && let Some(bytes) = assets.bytes(rel)
    {
        return load_image_rgba(&bytes, texture_page);
    }
    log::debug!("Texture not found anywhere: '{texture_page}' (tried KTX2, PNG, file_index)");
    None
}

/// Resolve the tcmask texture for a model parsed in the foreground path.
pub(crate) fn resolve_tcmask_simple(
    assets: &dyn AssetSource,
    pie: &PieModel,
    tex_page: &str,
    tileset_index: usize,
) -> Option<TexturePageData> {
    let explicit = pie
        .tcmask_pages
        .get(tileset_index)
        .filter(|s| !s.is_empty())
        .or_else(|| pie.tcmask_pages.first().filter(|s| !s.is_empty()));
    if let Some(tcmask_name) = explicit {
        let mut result = load_texture_simple(assets, tcmask_name);
        if let Some(ref mut data) = result {
            normalize_tcmask_channels(data);
        }
        return result;
    }

    if let Some(auto_name) = tcmask_name_from_texture(tex_page) {
        let auto_rel = Path::new(TEXPAGES_REL).join(&auto_name);
        if let Some(bytes) = assets.bytes(&auto_rel) {
            let mut result = load_image_rgba(&bytes, &auto_name);
            if let Some(ref mut data) = result {
                normalize_tcmask_channels(data);
            }
            return result;
        }
    }

    None
}

/// Resolve and load a tcmask texture during background preparation.
pub(crate) fn resolve_tcmask_offline(
    pie: &PieModel,
    tex_page: &str,
    assets: &dyn AssetSource,
    tileset_index: usize,
    file_index: &HashMap<String, PathBuf>,
) -> Option<TexturePageData> {
    let explicit = pie
        .tcmask_pages
        .get(tileset_index)
        .filter(|s| !s.is_empty())
        .or_else(|| pie.tcmask_pages.first().filter(|s| !s.is_empty()));
    if let Some(tcmask_name) = explicit {
        let mut result = load_texture_offline(assets, tcmask_name, file_index);
        if let Some(ref mut data) = result {
            normalize_tcmask_channels(data);
        } else {
            log::debug!("TCMask explicit '{tcmask_name}' not found (tex_page={tex_page})");
        }
        return result;
    }

    if let Some(auto_name) = tcmask_name_from_texture(tex_page) {
        let mut result = load_texture_offline(assets, &auto_name, file_index);
        if let Some(ref mut data) = result {
            normalize_tcmask_channels(data);
        } else {
            log::debug!("TCMask auto-detect '{auto_name}' not found for tex_page={tex_page}");
        }
        return result;
    }

    None
}

/// Resolve and load a normal or specular map.
///
/// Checks the explicit PIE directive first, then auto-detects by
/// appending `_nm` or `_sm` to the diffuse texture page name.
pub(crate) fn resolve_normal_specular_offline(
    explicit_page: Option<&String>,
    tex_page: &str,
    suffix: &str,
    assets: &dyn AssetSource,
    file_index: &HashMap<String, PathBuf>,
) -> Option<TexturePageData> {
    if let Some(page) = explicit_page
        && !page.is_empty()
    {
        return load_texture_offline(assets, page, file_index);
    }

    let auto_name = append_suffix_to_filename(tex_page, suffix);
    load_texture_offline(assets, &auto_name, file_index)
}

/// Append a suffix before the file extension: "page-11.png" + "_nm" becomes "page-11_nm.png".
pub(crate) fn append_suffix_to_filename(filename: &str, suffix: &str) -> String {
    if let Some(dot_pos) = filename.rfind('.') {
        format!("{}{suffix}{}", &filename[..dot_pos], &filename[dot_pos..])
    } else {
        format!("{filename}{suffix}")
    }
}

/// Move the tcmask from alpha into the red channel.
///
/// WZ2100 stores the mask in alpha; indexed PNGs with tRNS transparency
/// can carry garbage RGB behind alpha=0 pixels, so copy alpha into red
/// rather than using `max(R, A)`.
pub(crate) fn normalize_tcmask_channels(data: &mut TexturePageData) {
    for pixel in data.rgba.chunks_exact_mut(4) {
        pixel[0] = pixel[3];
    }
}

/// Auto-detected tcmask filename for a texture page.
///
/// `page-NN-description.png` becomes `page-NN_tcmask.png`. Returns `None`
/// if the page doesn't match the `page-NN` pattern.
pub(crate) fn tcmask_name_from_texture(tex_page: &str) -> Option<String> {
    if !tex_page.starts_with("page-") {
        return None;
    }
    let digit_end = tex_page[5..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(tex_page.len(), |i| i + 5);
    if digit_end <= 5 {
        return None;
    }
    Some(format!("{}_tcmask.png", &tex_page[..digit_end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(dir: &Path) -> crate::assets::FsAssetSource {
        crate::assets::FsAssetSource::new(dir.to_path_buf())
    }

    fn write_test_png(path: &Path) {
        let img = image::RgbaImage::from_raw(2, 2, vec![255u8; 16]).unwrap();
        img.save(path).unwrap();
    }

    #[test]
    fn tcmask_name_standard_texture_page() {
        assert_eq!(
            tcmask_name_from_texture("page-11-player-buildings.png"),
            Some("page-11_tcmask.png".to_string()),
        );
    }

    #[test]
    fn tcmask_name_two_digit_page() {
        assert_eq!(
            tcmask_name_from_texture("page-34-buildings.png"),
            Some("page-34_tcmask.png".to_string()),
        );
    }

    #[test]
    fn tcmask_name_single_digit_page() {
        assert_eq!(
            tcmask_name_from_texture("page-8-ground.png"),
            Some("page-8_tcmask.png".to_string()),
        );
    }

    #[test]
    fn tcmask_name_bare_page_no_suffix() {
        assert_eq!(
            tcmask_name_from_texture("page-11.png"),
            Some("page-11_tcmask.png".to_string()),
        );
    }

    #[test]
    fn tcmask_name_bare_page_no_extension() {
        assert_eq!(
            tcmask_name_from_texture("page-11"),
            Some("page-11_tcmask.png".to_string()),
        );
    }

    #[test]
    fn tcmask_name_non_page_texture_returns_none() {
        assert_eq!(tcmask_name_from_texture("terrain.png"), None);
        assert_eq!(tcmask_name_from_texture("effect-glow.png"), None);
        assert_eq!(tcmask_name_from_texture(""), None);
    }

    #[test]
    fn tcmask_name_page_no_digits_returns_none() {
        assert_eq!(tcmask_name_from_texture("page-abc.png"), None);
    }

    #[test]
    fn load_image_rgba_carries_page_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page-99-test.png");
        write_test_png(&path);

        let bytes = std::fs::read(&path).unwrap();
        let result = load_image_rgba(&bytes, "page-99-test.png");
        assert!(result.is_some());
        let data = result.unwrap();
        assert_eq!(data.page_name, "page-99-test.png");
        assert_eq!(data.width, 2);
        assert_eq!(data.height, 2);
        assert_eq!(data.rgba.len(), 2 * 2 * 4);
    }

    #[test]
    fn load_texture_offline_resolves_direct_path() {
        let dir = tempfile::tempdir().unwrap();
        let texpages = dir.path().join("base/texpages");
        std::fs::create_dir_all(&texpages).unwrap();
        write_test_png(&texpages.join("page-7-barbarians.png"));

        let file_index = HashMap::new();
        let result = load_texture_offline(&fs(dir.path()), "page-7-barbarians.png", &file_index);
        assert!(result.is_some());
        let data = result.unwrap();
        assert_eq!(data.page_name, "page-7-barbarians.png");
    }

    #[test]
    fn load_texture_offline_resolves_with_png_extension() {
        let dir = tempfile::tempdir().unwrap();
        let texpages = dir.path().join("base/texpages");
        std::fs::create_dir_all(&texpages).unwrap();
        write_test_png(&texpages.join("page-7.png"));

        let file_index = HashMap::new();
        let result = load_texture_offline(&fs(dir.path()), "page-7", &file_index);
        assert!(result.is_some());
        assert_eq!(result.unwrap().page_name, "page-7.png");
    }

    #[test]
    fn load_texture_offline_falls_back_to_file_index() {
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("custom");
        std::fs::create_dir_all(&custom).unwrap();
        write_test_png(&custom.join("page-42-special.png"));

        let mut file_index = HashMap::new();
        file_index.insert(
            "page-42-special.png".to_string(),
            PathBuf::from("custom/page-42-special.png"),
        );

        let result = load_texture_offline(&fs(dir.path()), "page-42-special.png", &file_index);
        assert!(result.is_some());
        assert_eq!(result.unwrap().page_name, "page-42-special.png");
    }

    #[test]
    fn load_texture_offline_returns_none_for_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("base/texpages")).unwrap();
        let file_index = HashMap::new();

        let result = load_texture_offline(&fs(dir.path()), "nonexistent.png", &file_index);
        assert!(result.is_none());
    }

    #[test]
    fn append_suffix_inserts_before_extension() {
        assert_eq!(
            append_suffix_to_filename("page-11-player-buildings.png", "_nm"),
            "page-11-player-buildings_nm.png"
        );
        assert_eq!(
            append_suffix_to_filename("page-82.png", "_sm"),
            "page-82_sm.png"
        );
    }

    #[test]
    fn append_suffix_handles_ktx2_extension() {
        assert_eq!(
            append_suffix_to_filename("page-11.ktx2", "_nm"),
            "page-11_nm.ktx2"
        );
    }

    #[test]
    fn append_suffix_no_extension() {
        assert_eq!(append_suffix_to_filename("texture", "_nm"), "texture_nm");
    }
}
