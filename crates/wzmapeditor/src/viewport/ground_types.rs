//! Ground type data for terrain texture splatting (Medium/High quality modes).
//!
//! Parses WZ2100 tileset data files to extract per-tile corner ground types,
//! ground type texture filenames and scales, and decal tile lists.
//! Reference: `warzone2100/src/map.cpp` - `SetGroundForTile`, `groundFromMapTile`.

use std::collections::HashMap;
use std::path::Path;

/// Ground type texture metadata for one ground type (e.g. `a_yellow`).
#[derive(Debug, Clone)]
pub struct GroundTexture {
    /// Ground type name (e.g. `a_yellow`).
    pub name: String,
    /// Diffuse texture filename (e.g. "page-82-yellow-sand-arizona.png").
    pub filename: String,
    /// Texture tiling scale (world units per texture repeat).
    pub scale: f32,
    /// Normal map filename (e.g. "page-82-yellow-sand-arizona_nm.png").
    pub normal_filename: Option<String>,
    /// Specular map filename (e.g. "page-82-yellow-sand-arizona_sm.png").
    pub specular_filename: Option<String>,
}

/// Complete ground type data for one tileset.
#[derive(Debug, Clone)]
pub struct GroundData {
    /// Ordered list of ground types with texture info.
    pub ground_types: Vec<GroundTexture>,
    /// Per tile index: 4 corner ground type indices.
    ///
    /// Corner layout matches WZ2100 `map[tile][j][k]`:
    /// `[0]` = `[0][0]`, `[1]` = `[0][1]`, `[2]` = `[1][0]`, `[3]` = `[1][1]`
    pub tile_grounds: Vec<[u8; 4]>,
    /// Set of tile indices that are decals (drawn over ground splatting).
    pub decal_tiles: Vec<bool>,
    /// Ground texture scale values indexed by ground type, packed for the shader.
    ///
    /// Stored as 16 floats (enough for up to 16 ground types per tileset).
    pub ground_scales: [f32; 16],
}

impl GroundData {
    /// Load ground data for a tileset from the WZ2100 data directory.
    ///
    /// Reads three files from `<data_dir>/base/tileset/`:
    /// - `tertilesc{N}hwGtype.txt` - ground type names, textures, scales
    /// - `{tileset}ground.txt` - per-tile corner ground types
    /// - `{tileset}decals.txt` - decal tile indices
    pub fn load(assets: &dyn crate::assets::AssetSource, tileset: &str) -> Option<Self> {
        let tileset_rel = Path::new("base").join("tileset");

        let (gtype_file, ground_file, decals_file) = match tileset {
            "arizona" => (
                "tertilesc1hwGtype.txt",
                "arizonaground.txt",
                "arizonadecals.txt",
            ),
            "urban" => (
                "tertilesc2hwGtype.txt",
                "urbanground.txt",
                "urbandecals.txt",
            ),
            "rockies" => (
                "tertilesc3hwGtype.txt",
                "rockieground.txt",
                "rockiedecals.txt",
            ),
            _ => {
                log::warn!("Unknown tileset {tileset:?}, cannot load ground data");
                return None;
            }
        };

        let texpages_rel = Path::new("base").join("texpages");
        let Some(gtype_content) = assets.text(&tileset_rel.join(gtype_file)) else {
            log::warn!("Failed to read ground type file {gtype_file}");
            return None;
        };
        let mut ground_types = parse_gtype_content(&gtype_content)?;

        // WZ2100 ships these as optional files; most installs don't have them.
        probe_high_quality_textures(&mut ground_types, assets, &texpages_rel);

        let name_to_index = build_name_index(&ground_types);
        let ground_content = assets.text(&tileset_rel.join(ground_file))?;
        let tile_grounds = parse_ground_content(&ground_content, &name_to_index)?;
        let decal_tiles = if let Some(content) = assets.text(&tileset_rel.join(decals_file)) {
            parse_decals_content(&content, tile_grounds.len())
        } else {
            log::warn!("Failed to read {decals_file} (decals will be disabled)");
            vec![false; tile_grounds.len()]
        };

        let mut ground_scales = [1.0f32; 16];
        for (i, gt) in ground_types.iter().enumerate() {
            if i < 16 {
                ground_scales[i] = gt.scale;
            }
        }

        let has_high_quality = ground_types
            .iter()
            .any(|gt| gt.normal_filename.is_some() || gt.specular_filename.is_some());

        log::info!(
            "Loaded ground data for {tileset}: {} ground types, {} tile entries, {} decals, high_quality={has_high_quality}",
            ground_types.len(),
            tile_grounds.len(),
            decal_tiles.iter().filter(|&&d| d).count(),
        );

        Some(Self {
            ground_types,
            tile_grounds,
            decal_tiles,
            ground_scales,
        })
    }

    /// Whether a tile index is a decal (should be overlaid on ground splatting).
    pub fn is_decal(&self, tile_index: u32) -> bool {
        let idx = tile_index as usize;
        idx < self.decal_tiles.len() && self.decal_tiles[idx]
    }

    /// Build a per-vertex ground type grid using WZ2100's voting algorithm.
    ///
    /// Each vertex on the `(w+1)×(h+1)` grid looks at the 4 surrounding tiles
    /// and picks the majority-voted ground type, so adjacent tiles agree on
    /// shared edges and visible seams don't appear.
    ///
    /// Reference: `warzone2100/src/map.cpp` - `determineGroundType()`.
    pub fn build_ground_grid(&self, map: &wz_maplib::MapData) -> Vec<u8> {
        let w = map.width as i32;
        let h = map.height as i32;
        let vw = (w + 1) as usize;
        let vh = (h + 1) as usize;
        let mut grid = vec![0u8; vw * vh];

        for vy in 0..vh {
            for vx in 0..vw {
                grid[vy * vw + vx] = self.determine_ground_type(map, vx as i32, vy as i32, w, h);
            }
        }
        grid
    }

    /// Per-vertex ground type via the same voting algorithm as
    /// `build_ground_grid`, exposed for partial mesh rebuilds. Called
    /// once per affected vertex during a brush-stroke incremental
    /// update so the ground splatting reflects the just-painted tile
    /// instead of a stale cached vote.
    pub fn vertex_ground_type(&self, map: &wz_maplib::MapData, vx: u32, vy: u32) -> u8 {
        self.determine_ground_type(
            map,
            vx as i32,
            vy as i32,
            map.width as i32,
            map.height as i32,
        )
    }

    /// Determine the ground type at vertex (vx, vy) by voting across
    /// the 4 surrounding tiles.
    fn determine_ground_type(
        &self,
        map: &wz_maplib::MapData,
        vx: i32,
        vy: i32,
        w: i32,
        h: i32,
    ) -> u8 {
        // Tile (vx+i-1, vy+j-1) contributes its corner (i, j) after rotFlip.
        let mut ground = [0u8; 4];
        let mut weight = [0u32; 4];

        for j in 0..2i32 {
            for i in 0..2i32 {
                let tx = vx + i - 1;
                let ty = vy + j - 1;
                let idx = (j * 2 + i) as usize;

                if tx < 0 || ty < 0 || tx >= w || ty >= h {
                    ground[idx] = 0;
                    weight[idx] = 0;
                    continue;
                }

                let tile = if let Some(t) = map.tile(tx as u32, ty as u32) {
                    *t
                } else {
                    ground[idx] = 0;
                    weight[idx] = 0;
                    continue;
                };

                let (ri, rj) = rot_flip_corner(
                    i as u8,
                    j as u8,
                    tile.rotation(),
                    tile.x_flip(),
                    tile.y_flip(),
                );
                let tile_idx = tile.texture_id() as usize;
                ground[idx] = if tile_idx < self.tile_grounds.len() {
                    self.tile_grounds[tile_idx][(ri as usize) * 2 + (rj as usize)]
                } else {
                    0
                };
                weight[idx] = 10; // Default weight; cliff/water weights skipped for simplicity
            }
        }

        let mut best_idx = 0usize;
        let mut best_score = 0u32;
        for a in 0..4 {
            let mut score = 0u32;
            for b in 0..4 {
                if ground[a] == ground[b] {
                    score += weight[b];
                }
            }
            if score > best_score || (score == best_score && ground[a] < ground[best_idx]) {
                best_score = score;
                best_idx = a;
            }
        }
        ground[best_idx]
    }
}

/// Apply WZ2100's `rotFlip` transform to a corner (j,k) coordinate.
///
/// Reference: `warzone2100/src/map.cpp:527` - `rotFlip()`.
fn rot_flip_corner(mut j: u8, mut k: u8, rotation: u8, x_flip: bool, y_flip: bool) -> (u8, u8) {
    if x_flip {
        j = 1 - j;
    }
    if y_flip {
        k = 1 - k;
    }

    // Rotation lookup table matching WZ2100's tmpMap/invmap approach.
    // tmpMap[j][k] gives corner index 0..3, subtract rotation, then invmap back.
    let tmp_map = [[0u8, 3], [1, 2]]; // [j][k] -> corner number
    let inv_map: [(u8, u8); 4] = [(0, 0), (1, 0), (1, 1), (0, 1)]; // corner -> (j, k)

    let mut corner = tmp_map[j as usize][k as usize] as i8;
    corner -= rotation as i8;
    while corner < 0 {
        corner += 4;
    }
    let (rj, rk) = inv_map[corner as usize];
    (rj, rk)
}

/// Append a suffix before the file extension (e.g. "page-82.png" + "_nm" -> "page-82_nm.png").
fn append_to_filename(filename: &str, suffix: &str) -> String {
    if let Some(dot_pos) = filename.rfind('.') {
        format!("{}{suffix}{}", &filename[..dot_pos], &filename[dot_pos..])
    } else {
        format!("{filename}{suffix}")
    }
}

/// Parse `tertilesc{N}hwGtype.txt` - ground type names, textures, and scales.
fn parse_gtype_content(content: &str) -> Option<Vec<GroundTexture>> {
    let mut lines = content.lines();
    // First line: "tertilesc1hw,8" (tileset name, count)
    let header = lines.next()?;
    let count: usize = header.split(',').nth(1)?.trim().parse().ok()?;

    let mut types = Vec::with_capacity(count);
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 3 {
            continue;
        }
        types.push(GroundTexture {
            name: parts[0].trim().to_string(),
            filename: parts[1].trim().to_string(),
            scale: parts[2].trim().parse().unwrap_or(5.0),
            normal_filename: None,
            specular_filename: None,
        });
    }

    Some(types)
}

/// Check whether `_nm` / `_sm` texture variants exist on disk.
///
/// Probes for `.png`, `.ktx2`, or cached `.bin` files. Only sets the
/// filename fields when at least one variant form is found, so
/// `has_high_quality` remains `false` for installs that lack them
/// (which is the norm - WZ2100 ships the hooks but not the assets).
fn probe_high_quality_textures(
    ground_types: &mut [GroundTexture],
    assets: &dyn crate::assets::AssetSource,
    texpages_rel: &Path,
) {
    let cache_dir = crate::config::ground_cache_dir();
    let mut found_any = false;

    for gt in ground_types.iter_mut() {
        let nm = append_to_filename(&gt.filename, "_nm");
        if texture_exists(assets, texpages_rel, &cache_dir, &nm) {
            gt.normal_filename = Some(nm);
            found_any = true;
        }

        let sm = append_to_filename(&gt.filename, "_sm");
        if texture_exists(assets, texpages_rel, &cache_dir, &sm) {
            gt.specular_filename = Some(sm);
            found_any = true;
        }
    }

    if found_any {
        let nm_count = ground_types
            .iter()
            .filter(|gt| gt.normal_filename.is_some())
            .count();
        let sm_count = ground_types
            .iter()
            .filter(|gt| gt.specular_filename.is_some())
            .count();
        log::info!("Found high-quality textures: {nm_count} normal maps, {sm_count} specular maps");
    } else {
        log::debug!("No _nm/_sm texture variants found, high quality mode unavailable");
    }
}

/// Check if a ground texture exists in any supported form.
///
/// The cached `.bin` lives under the config dir, not the data root, so it is
/// probed directly rather than through the asset source (native-only).
fn texture_exists(
    assets: &dyn crate::assets::AssetSource,
    texpages_rel: &Path,
    cache_dir: &Path,
    filename: &str,
) -> bool {
    // PNG source
    if assets.exists(&texpages_rel.join(filename)) {
        return true;
    }
    // KTX2 (installed game)
    let ktx2 = filename.replace(".png", ".ktx2");
    if assets.exists(&texpages_rel.join(&ktx2)) {
        return true;
    }
    // Cached raw .bin
    let bin = filename.replace(".png", ".bin");
    if cache_dir.join(&bin).exists() {
        return true;
    }
    false
}

/// Build a name → index map from ground types.
fn build_name_index(types: &[GroundTexture]) -> HashMap<String, u8> {
    types
        .iter()
        .enumerate()
        .map(|(i, gt)| (gt.name.clone(), i as u8))
        .collect()
}

/// Parse `{tileset}ground.txt` - per-tile 4 corner ground types.
///
/// File format: `val1,val2,val3,val4` per line. WZ2100 mapping:
/// - `map[i][0][0]` = val4, `map[i][0][1]` = val2
/// - `map[i][1][0]` = val3, `map[i][1][1]` = val1
fn parse_ground_content(content: &str, name_index: &HashMap<String, u8>) -> Option<Vec<[u8; 4]>> {
    let mut lines = content.lines();
    // Header: "arizona_ground,78"
    let header = lines.next()?;
    let count: usize = header.split(',').nth(1)?.trim().parse().ok()?;

    let mut tile_grounds = Vec::with_capacity(count);
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 4 {
            continue;
        }
        let val1 = parts[0].trim();
        let val2 = parts[1].trim();
        let val3 = parts[2].trim();
        let val4 = parts[3].trim();

        // WZ2100 mapping: map[i][j][k] stored as flat array index j*2+k
        // [0][0]=val4, [0][1]=val2, [1][0]=val3, [1][1]=val1
        let g00 = *name_index.get(val4).unwrap_or(&0);
        let g01 = *name_index.get(val2).unwrap_or(&0);
        let g10 = *name_index.get(val3).unwrap_or(&0);
        let g11 = *name_index.get(val1).unwrap_or(&0);

        tile_grounds.push([g00, g01, g10, g11]);
    }

    Some(tile_grounds)
}

/// Parse `{tileset}decals.txt` - tile indices that are decals.
fn parse_decals_content(content: &str, num_tiles: usize) -> Vec<bool> {
    let mut decals = vec![false; num_tiles];

    let mut lines = content.lines();
    // Header: "arizona_decals,29"
    let _header = lines.next();

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(idx) = line.parse::<usize>()
            && idx < decals.len()
        {
            decals[idx] = true;
        }
    }

    decals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(dir: &std::path::Path) -> crate::assets::FsAssetSource {
        crate::assets::FsAssetSource::new(dir.to_path_buf())
    }

    #[test]
    fn rot_flip_identity() {
        // No rotation, no flip - corners unchanged.
        assert_eq!(rot_flip_corner(0, 0, 0, false, false), (0, 0));
        assert_eq!(rot_flip_corner(0, 1, 0, false, false), (0, 1));
        assert_eq!(rot_flip_corner(1, 0, 0, false, false), (1, 0));
        assert_eq!(rot_flip_corner(1, 1, 0, false, false), (1, 1));
    }

    #[test]
    fn rot_flip_x_flip() {
        // X flip mirrors j axis.
        assert_eq!(rot_flip_corner(0, 0, 0, true, false), (1, 0));
        assert_eq!(rot_flip_corner(1, 0, 0, true, false), (0, 0));
    }

    #[test]
    fn rot_flip_y_flip() {
        // Y flip mirrors k axis.
        assert_eq!(rot_flip_corner(0, 0, 0, false, true), (0, 1));
        assert_eq!(rot_flip_corner(0, 1, 0, false, true), (0, 0));
    }

    #[test]
    fn rot_flip_rotation_1() {
        // 90° rotation.
        let r0 = rot_flip_corner(0, 0, 1, false, false);
        let r1 = rot_flip_corner(1, 1, 1, false, false);
        // Rotation should produce different corners than identity.
        assert_ne!(r0, (0, 0));
        assert_ne!(r1, (1, 1));
    }

    #[test]
    fn parse_gtype_roundtrip() {
        let dir = std::env::temp_dir().join("wz_ground_test_gtype");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("test_gtype.txt"),
            "test_tileset,3\na_sand,page-1-sand.png,6.4\na_rock,page-2-rock.png,5.0\na_water,page-3-water.png,3.2\n",
        ).unwrap();

        let content = std::fs::read_to_string(dir.join("test_gtype.txt")).unwrap();
        let types = parse_gtype_content(&content).unwrap();
        assert_eq!(types.len(), 3);
        assert_eq!(types[0].name, "a_sand");
        assert_eq!(types[0].filename, "page-1-sand.png");
        assert!((types[0].scale - 6.4).abs() < 0.01);
        assert_eq!(types[2].name, "a_water");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_ground_file_maps_corners() {
        let dir = std::env::temp_dir().join("wz_ground_test_ground");
        let _ = std::fs::create_dir_all(&dir);

        let mut name_index = HashMap::new();
        name_index.insert("a_sand".to_string(), 0u8);
        name_index.insert("a_rock".to_string(), 1u8);

        // Line format: val1,val2,val3,val4
        // WZ2100 mapping: [0][0]=val4, [0][1]=val2, [1][0]=val3, [1][1]=val1
        std::fs::write(
            dir.join("test_ground.txt"),
            "test_ground,1\na_rock,a_sand,a_rock,a_sand\n",
        )
        .unwrap();

        let content = std::fs::read_to_string(dir.join("test_ground.txt")).unwrap();
        let grounds = parse_ground_content(&content, &name_index).unwrap();
        assert_eq!(grounds.len(), 1);
        // val1=a_rock, val2=a_sand, val3=a_rock, val4=a_sand
        // [0][0]=val4=a_sand=0, [0][1]=val2=a_sand=0, [1][0]=val3=a_rock=1, [1][1]=val1=a_rock=1
        assert_eq!(grounds[0], [0, 0, 1, 1]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_decals_file_marks_indices() {
        let dir = std::env::temp_dir().join("wz_ground_test_decals");
        let _ = std::fs::create_dir_all(&dir);

        std::fs::write(dir.join("test_decals.txt"), "test_decals,2\n05\n10\n").unwrap();

        let content = std::fs::read_to_string(dir.join("test_decals.txt")).unwrap();
        let decals = parse_decals_content(&content, 20);
        assert!(!decals[0]);
        assert!(decals[5]);
        assert!(decals[10]);
        assert!(!decals[11]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_decal_returns_correct_values() {
        let gd = GroundData {
            ground_types: vec![],
            tile_grounds: vec![[0; 4]; 5],
            decal_tiles: vec![false, true, false, false, true],
            ground_scales: [1.0; 16],
        };
        assert!(!gd.is_decal(0));
        assert!(gd.is_decal(1));
        assert!(!gd.is_decal(2));
        assert!(gd.is_decal(4));
        // Out of bounds returns false.
        assert!(!gd.is_decal(100));
    }

    #[test]
    fn append_to_filename_inserts_suffix_before_extension() {
        assert_eq!(
            append_to_filename("page-82-sand.png", "_nm"),
            "page-82-sand_nm.png"
        );
        assert_eq!(append_to_filename("texture.ktx2", "_sm"), "texture_sm.ktx2");
    }

    #[test]
    fn append_to_filename_no_extension() {
        assert_eq!(append_to_filename("texture", "_nm"), "texture_nm");
    }

    #[test]
    fn texture_exists_finds_png() {
        let dir = std::env::temp_dir().join("wz_test_tex_exists_png");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("page-1.png"), b"fake").unwrap();

        let cache_dir = std::env::temp_dir().join("wz_test_tex_exists_cache_empty");
        let _ = std::fs::create_dir_all(&cache_dir);

        assert!(texture_exists(
            &fs(&dir),
            std::path::Path::new(""),
            &cache_dir,
            "page-1.png"
        ));
        assert!(!texture_exists(
            &fs(&dir),
            std::path::Path::new(""),
            &cache_dir,
            "page-99.png"
        ));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&cache_dir);
    }

    #[test]
    fn texture_exists_finds_ktx2() {
        let dir = std::env::temp_dir().join("wz_test_tex_exists_ktx2");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("page-1.ktx2"), b"fake").unwrap();

        let cache_dir = std::env::temp_dir().join("wz_test_tex_exists_ktx2_cache");
        let _ = std::fs::create_dir_all(&cache_dir);

        assert!(texture_exists(
            &fs(&dir),
            std::path::Path::new(""),
            &cache_dir,
            "page-1.png"
        ));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&cache_dir);
    }

    #[test]
    fn texture_exists_finds_cached_bin() {
        let texpages = std::env::temp_dir().join("wz_test_tex_exists_bin_tp");
        let cache_dir = std::env::temp_dir().join("wz_test_tex_exists_bin_cache");
        let _ = std::fs::create_dir_all(&texpages);
        let _ = std::fs::create_dir_all(&cache_dir);
        std::fs::write(cache_dir.join("page-1.bin"), b"raw").unwrap();

        assert!(texture_exists(
            &fs(&texpages),
            std::path::Path::new(""),
            &cache_dir,
            "page-1.png"
        ));
        assert!(!texture_exists(
            &fs(&texpages),
            std::path::Path::new(""),
            &cache_dir,
            "page-2.png"
        ));

        let _ = std::fs::remove_dir_all(&texpages);
        let _ = std::fs::remove_dir_all(&cache_dir);
    }

    #[test]
    fn probe_high_quality_sets_filenames_when_present() {
        let dir = std::env::temp_dir().join("wz_test_probe_hq");
        let _ = std::fs::create_dir_all(&dir);
        // Create a _nm and _sm file for one ground type.
        std::fs::write(dir.join("page-1-sand_nm.png"), b"nm").unwrap();
        std::fs::write(dir.join("page-1-sand_sm.png"), b"sm").unwrap();

        let mut types = vec![
            GroundTexture {
                name: "a_sand".into(),
                filename: "page-1-sand.png".into(),
                scale: 1.0,
                normal_filename: None,
                specular_filename: None,
            },
            GroundTexture {
                name: "a_rock".into(),
                filename: "page-2-rock.png".into(),
                scale: 1.0,
                normal_filename: None,
                specular_filename: None,
            },
        ];

        probe_high_quality_textures(&mut types, &fs(&dir), std::path::Path::new(""));

        assert_eq!(
            types[0].normal_filename.as_deref(),
            Some("page-1-sand_nm.png")
        );
        assert_eq!(
            types[0].specular_filename.as_deref(),
            Some("page-1-sand_sm.png")
        );
        // Rock has no _nm/_sm files on disk.
        assert!(types[1].normal_filename.is_none());
        assert!(types[1].specular_filename.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_high_quality_no_files_leaves_none() {
        let dir = std::env::temp_dir().join("wz_test_probe_hq_empty");
        let _ = std::fs::create_dir_all(&dir);

        let mut types = vec![GroundTexture {
            name: "a_sand".into(),
            filename: "page-1-sand.png".into(),
            scale: 1.0,
            normal_filename: None,
            specular_filename: None,
        }];

        probe_high_quality_textures(&mut types, &fs(&dir), std::path::Path::new(""));
        assert!(types[0].normal_filename.is_none());
        assert!(types[0].specular_filename.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
