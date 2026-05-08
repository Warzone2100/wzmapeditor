//! Text-format PIE model parser.

use glam::{Vec2, Vec3};

use crate::PieError;
use crate::constants::*;
use crate::types::{PieLevel, PieModel, PiePolygon};

/// Parse a PIE model from its text content.
#[expect(
    clippy::too_many_lines,
    reason = "sequential parsing steps do not benefit from splitting"
)]
pub fn parse_pie(content: &str) -> Result<PieModel, PieError> {
    let mut lines = content.lines().peekable();

    let header = lines
        .next()
        .ok_or_else(|| PieError::InvalidHeader("empty PIE file".to_string()))?;
    let version = parse_pie_header(header)?;

    let mut model_type: u32 = 0;
    let mut texture_page = String::new();
    let mut texture_width: u32 = 256;
    let mut texture_height: u32 = 256;
    let mut texture_pages: Vec<String> = Vec::new();
    let mut tcmask_pages: Vec<String> = Vec::new();
    let mut normal_page: Option<String> = None;
    let mut specular_page: Option<String> = None;
    let mut event_page: Option<String> = None;
    let mut num_levels: u32 = 0;
    let mut levels: Vec<PieLevel> = Vec::new();

    while let Some(line) = lines.peek() {
        let line = line.trim();
        if line.is_empty() {
            lines.next();
            continue;
        }

        if line.starts_with("TYPE") {
            let line = lines.next().expect("just peeked");
            // TYPE is hex in PIE files (upstream uses sscanf("%x")).
            model_type = parse_hex_after(line, "TYPE")?;
        } else if line.starts_with("TEXTURE") {
            let line = lines.next().expect("just peeked");
            let parts: Vec<&str> = line.split_whitespace().collect();
            // TEXTURE <directive> <filename> [<width> <height>]; v4 allows
            // multiple lines per tileset (0=Arizona, 1=Urban, 2=Rockies).
            let directive: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            if parts.len() >= 4 {
                let page = parts[2].to_string();
                if directive == 0 {
                    texture_page.clone_from(&page);
                    texture_width = parts[3].parse().unwrap_or(256);
                    texture_height = parts
                        .get(4)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(texture_width);
                }
                if texture_pages.len() <= directive as usize {
                    texture_pages.resize(directive as usize + 1, String::new());
                }
                texture_pages[directive as usize] = page;
            } else if parts.len() >= 3 {
                let page = parts[2].to_string();
                if directive == 0 {
                    texture_page.clone_from(&page);
                }
                if texture_pages.len() <= directive as usize {
                    texture_pages.resize(directive as usize + 1, String::new());
                }
                texture_pages[directive as usize] = page;
            }
        } else if line.starts_with("TCMASK") {
            // TCMASK <directive> <filename>: v4 team color mask texture.
            let line = lines.next().expect("just peeked");
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let directive: u32 = parts[1].parse().unwrap_or(0);
                let page = parts[2].to_string();
                if tcmask_pages.len() <= directive as usize {
                    tcmask_pages.resize(directive as usize + 1, String::new());
                }
                tcmask_pages[directive as usize] = page;
            }
        } else if line.starts_with("NORMALMAP") {
            let line = lines.next().expect("just peeked");
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                normal_page = Some(parts[2].to_string());
            }
        } else if line.starts_with("SPECULARMAP") {
            let line = lines.next().expect("just peeked");
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                specular_page = Some(parts[2].to_string());
            }
        } else if line.starts_with("EVENT") {
            let line = lines.next().expect("just peeked");
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                event_page = Some(parts[2].to_string());
            }
        } else if line.starts_with("LEVELS") {
            let line = lines.next().expect("just peeked");
            num_levels = parse_value_after(line, "LEVELS")?;
        } else if line.starts_with("LEVEL") {
            break;
        } else {
            lines.next();
        }
    }

    for _ in 0..num_levels {
        let level = parse_level(&mut lines, version)?;
        levels.push(level);
    }

    Ok(PieModel {
        version,
        model_type,
        texture_page,
        texture_width,
        texture_height,
        texture_pages,
        tcmask_pages,
        normal_page,
        specular_page,
        event_page,
        levels,
    })
}

fn parse_pie_header(line: &str) -> Result<u32, PieError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 || parts[0] != "PIE" {
        return Err(PieError::InvalidHeader(line.to_string()));
    }
    let version: u32 = parts[1]
        .parse()
        .map_err(|e: std::num::ParseIntError| PieError::InvalidHeader(e.to_string()))?;
    if !(PIE_MIN_VER..=PIE_MAX_VER).contains(&version) {
        return Err(PieError::UnsupportedVersion {
            version,
            min: PIE_MIN_VER,
            max: PIE_MAX_VER,
        });
    }
    Ok(version)
}

fn parse_level<'a, I: Iterator<Item = &'a str>>(
    lines: &mut std::iter::Peekable<I>,
    version: u32,
) -> Result<PieLevel, PieError> {
    if let Some(line) = lines.peek()
        && line.trim().starts_with("LEVEL")
    {
        lines.next();
    }

    let mut vertices: Vec<Vec3> = Vec::new();
    let mut polygons: Vec<PiePolygon> = Vec::new();
    let mut connectors: Vec<Vec3> = Vec::new();

    while let Some(line) = lines.peek() {
        let line = line.trim();
        if line.is_empty() {
            lines.next();
            continue;
        }

        if line.starts_with("LEVEL") && !line.starts_with("LEVELS") {
            break;
        }

        if line.starts_with("POINTS") {
            let line = lines.next().expect("just peeked");
            let count: usize = parse_value_after(line.trim(), "POINTS")?;
            vertices.reserve(count);
            for _ in 0..count {
                let vline = lines.next().ok_or_else(|| PieError::UnexpectedEof {
                    section: "POINTS".to_string(),
                })?;
                let v = parse_vertex(vline.trim(), version)?;
                vertices.push(v);
            }
        } else if line.starts_with("POLYGONS") {
            let line = lines.next().expect("just peeked");
            let count: usize = parse_value_after(line.trim(), "POLYGONS")?;
            polygons.reserve(count);
            for _ in 0..count {
                let pline = lines.next().ok_or_else(|| PieError::UnexpectedEof {
                    section: "POLYGONS".to_string(),
                })?;
                let poly = parse_polygon(pline.trim(), version)?;
                polygons.push(poly);
            }
        } else if line.starts_with("CONNECTORS") {
            let line = lines.next().expect("just peeked");
            let count: usize = parse_value_after(line.trim(), "CONNECTORS")?;
            connectors.reserve(count);
            for _ in 0..count {
                let cline = lines.next().ok_or_else(|| PieError::UnexpectedEof {
                    section: "CONNECTORS".to_string(),
                })?;
                let c = parse_vertex(cline.trim(), version)?;
                connectors.push(c);
            }
        } else {
            lines.next();
        }
    }

    Ok(PieLevel {
        vertices,
        polygons,
        connectors,
    })
}

fn parse_vertex(line: &str, version: u32) -> Result<Vec3, PieError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(PieError::InvalidVertex(line.to_string()));
    }

    if version >= PIE_VER_3 {
        let x: f32 = parts[0]
            .parse()
            .map_err(|e: std::num::ParseFloatError| PieError::InvalidVertex(e.to_string()))?;
        let y: f32 = parts[1]
            .parse()
            .map_err(|e: std::num::ParseFloatError| PieError::InvalidVertex(e.to_string()))?;
        let z: f32 = parts[2]
            .parse()
            .map_err(|e: std::num::ParseFloatError| PieError::InvalidVertex(e.to_string()))?;
        Ok(Vec3::new(x, y, z))
    } else {
        let x: i32 = parts[0]
            .parse()
            .map_err(|e: std::num::ParseIntError| PieError::InvalidVertex(e.to_string()))?;
        let y: i32 = parts[1]
            .parse()
            .map_err(|e: std::num::ParseIntError| PieError::InvalidVertex(e.to_string()))?;
        let z: i32 = parts[2]
            .parse()
            .map_err(|e: std::num::ParseIntError| PieError::InvalidVertex(e.to_string()))?;
        Ok(Vec3::new(x as f32, y as f32, z as f32))
    }
}

fn parse_polygon(line: &str, _version: u32) -> Result<PiePolygon, PieError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(PieError::InvalidPolygon(line.to_string()));
    }

    // Polygon flags are hex in PIE files (upstream uses sscanf("%x")); fall
    // back to decimal for older or hand-edited files.
    let flags: u32 = u32::from_str_radix(parts[0], 16)
        .or_else(|_| parts[0].parse::<u32>())
        .map_err(|e| PieError::InvalidPolygon(format!("invalid polygon flags: {e}")))?;
    let num_verts: usize = parts[1]
        .parse()
        .map_err(|e| PieError::InvalidPolygon(format!("invalid vertex count: {e}")))?;

    if parts.len() < 2 + num_verts {
        return Err(PieError::InvalidPolygon(line.to_string()));
    }

    let mut indices = Vec::with_capacity(num_verts);
    for i in 0..num_verts {
        let idx: u16 = parts[2 + i]
            .parse()
            .map_err(|e: std::num::ParseIntError| PieError::InvalidPolygon(e.to_string()))?;
        indices.push(idx);
    }

    let has_tex = flags & PIE_TEX != 0;
    let has_texanim = flags & PIE_TEXANIM != 0;

    let mut tex_coords = Vec::new();
    let mut anim_frames = None;
    let mut anim_rate = None;
    let mut anim_width = None;
    let mut anim_height = None;

    let mut offset = 2 + num_verts;

    if has_texanim && has_tex && offset + 4 <= parts.len() {
        anim_frames = Some(parts[offset].parse().unwrap_or(1));
        anim_rate = Some(parts[offset + 1].parse().unwrap_or(1));
        anim_width = Some(parts[offset + 2].parse().unwrap_or(0.0));
        anim_height = Some(parts[offset + 3].parse().unwrap_or(0.0));
        offset += 4;
    }

    if has_tex {
        for _ in 0..num_verts {
            if offset + 1 < parts.len() {
                let u: f32 = parts[offset].parse().unwrap_or(0.0);
                let v: f32 = parts[offset + 1].parse().unwrap_or(0.0);
                tex_coords.push(Vec2::new(u, v));
                offset += 2;
            }
        }
    }

    Ok(PiePolygon {
        flags,
        indices,
        tex_coords,
        anim_frames,
        anim_rate,
        anim_width,
        anim_height,
    })
}

fn parse_value_after<T: std::str::FromStr>(line: &str, prefix: &str) -> Result<T, PieError>
where
    T::Err: std::fmt::Display,
{
    let rest = line
        .strip_prefix(prefix)
        .ok_or_else(|| PieError::Parse(format!("expected '{prefix}' prefix")))?
        .trim();
    rest.parse::<T>()
        .map_err(|e| PieError::Parse(format!("failed to parse value after '{prefix}': {e}")))
}

/// Parse a hex u32 value after a keyword prefix (e.g. `TYPE 10200` → 0x10200).
fn parse_hex_after(line: &str, prefix: &str) -> Result<u32, PieError> {
    let rest = line
        .strip_prefix(prefix)
        .ok_or_else(|| PieError::Parse(format!("expected '{prefix}' prefix")))?
        .trim();
    u32::from_str_radix(rest, 16)
        .or_else(|_| rest.parse::<u32>())
        .map_err(|e| PieError::Parse(format!("failed to parse hex after '{prefix}': {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_pie() {
        let content = r"PIE 3
TYPE 10200
TEXTURE 0 page-11-player-buildings.png 256 256
LEVELS 1
LEVEL 1
POINTS 4
	-21.0 0.0 -21.0
	21.0 0.0 -21.0
	21.0 0.0 21.0
	-21.0 0.0 21.0
POLYGONS 2
	200 3 0 1 2 0.0 0.0 1.0 0.0 1.0 1.0
	200 3 0 2 3 0.0 0.0 1.0 1.0 0.0 1.0
CONNECTORS 1
	0.0 10.0 0.0
";
        let model = parse_pie(content).unwrap();
        assert_eq!(model.version, 3);
        assert_eq!(model.model_type, 0x10200);
        assert!(model.has_tcmask());
        assert_eq!(model.texture_page, "page-11-player-buildings.png");
        assert_eq!(model.levels.len(), 1);
        assert_eq!(model.levels[0].vertices.len(), 4);
        assert_eq!(model.levels[0].polygons.len(), 2);
        assert_eq!(model.levels[0].connectors.len(), 1);
        assert_eq!(model.levels[0].polygons[0].indices.len(), 3);
        assert_eq!(model.levels[0].polygons[0].tex_coords.len(), 3);
        assert!(model.tcmask_pages.is_empty());
    }

    #[test]
    fn test_parse_pie4_with_tcmask() {
        let content = r"PIE 4
TYPE 200
TEXTURE 0 page-34-buildings.png
TCMASK 0 page-34_tcmask.png
LEVELS 1
LEVEL 1
POINTS 3
	0 0 0
	100 0 0
	50 100 0
POLYGONS 1
	200 3 0 1 2 0.0 0.0 1.0 0.0 0.5 1.0
";
        let model = parse_pie(content).unwrap();
        assert_eq!(model.version, 4);
        assert_eq!(model.model_type, 0x200);
        assert!(!model.has_tcmask());
        assert_eq!(model.texture_page, "page-34-buildings.png");
        assert_eq!(model.tcmask_pages.len(), 1);
        assert_eq!(model.tcmask_pages[0], "page-34_tcmask.png");
    }

    #[test]
    fn test_type_hex_parsing_tcmask_flag() {
        // 0x10200 = iV_IMD_TEX | iV_IMD_TCMASK, the common structure case.
        let content = "PIE 3\nTYPE 10200\nTEXTURE 0 page-11-player-buildings.png 256 256\nLEVELS 1\nLEVEL 1\nPOINTS 3\n0 0 0\n1 0 0\n0 1 0\nPOLYGONS 1\n200 3 0 1 2 0.0 0.0 1.0 0.0 0.5 1.0\n";
        let model = parse_pie(content).unwrap();
        assert_eq!(model.model_type, 0x10200);
        assert!(model.has_tcmask());
        assert_ne!(model.model_type & PIE_TCMASK, 0);
        assert_ne!(model.model_type & PIE_TEX, 0);
    }

    #[test]
    fn test_type_hex_plain_tex_only() {
        let content = "PIE 3\nTYPE 200\nTEXTURE 0 page-8-ground.png 256 256\nLEVELS 1\nLEVEL 1\nPOINTS 3\n0 0 0\n1 0 0\n0 1 0\nPOLYGONS 1\n200 3 0 1 2 0.0 0.0 1.0 0.0 0.5 1.0\n";
        let model = parse_pie(content).unwrap();
        assert_eq!(model.model_type, 0x200);
        assert!(!model.has_tcmask());
    }

    #[test]
    fn test_type_zero() {
        let content = "PIE 3\nTYPE 0\nTEXTURE 0 page-1.png 256 256\nLEVELS 1\nLEVEL 1\nPOINTS 3\n0 0 0\n1 0 0\n0 1 0\nPOLYGONS 1\n200 3 0 1 2 0.0 0.0 1.0 0.0 0.5 1.0\n";
        let model = parse_pie(content).unwrap();
        assert_eq!(model.model_type, 0);
        assert!(!model.has_tcmask());
    }

    #[test]
    fn test_type_with_additive_and_tcmask() {
        // 0x10202 = TCMASK | TEX | ADDITIVE.
        let content = "PIE 3\nTYPE 10202\nTEXTURE 0 page-7-effects.png 256 256\nLEVELS 1\nLEVEL 1\nPOINTS 3\n0 0 0\n1 0 0\n0 1 0\nPOLYGONS 1\n200 3 0 1 2 0.0 0.0 1.0 0.0 0.5 1.0\n";
        let model = parse_pie(content).unwrap();
        assert_eq!(model.model_type, 0x10202);
        assert!(model.has_tcmask());
        assert_ne!(model.model_type & PIE_ADDITIVE, 0);
    }

    #[test]
    fn test_polygon_flags_are_hex() {
        // 0x200 = PIE_TEX, the standard textured polygon flag.
        let content = "PIE 3\nTYPE 200\nTEXTURE 0 page-1.png 256 256\nLEVELS 1\nLEVEL 1\nPOINTS 3\n0 0 0\n1 0 0\n0 1 0\nPOLYGONS 1\n200 3 0 1 2 0.0 0.0 1.0 0.0 0.5 1.0\n";
        let model = parse_pie(content).unwrap();
        let poly = &model.levels[0].polygons[0];
        assert_eq!(poly.flags, 0x200);
        assert!(poly.has_texture());
        assert!(!poly.has_tex_anim());
    }

    #[test]
    fn test_polygon_flags_texanim() {
        // 0x4200 = PIE_TEX | PIE_TEXANIM.
        let content = "PIE 3\nTYPE 200\nTEXTURE 0 page-1.png 256 256\nLEVELS 1\nLEVEL 1\nPOINTS 3\n0 0 0\n1 0 0\n0 1 0\nPOLYGONS 1\n4200 3 0 1 2 4 1 0.25 0.25 0.0 0.0 1.0 0.0 0.5 1.0\n";
        let model = parse_pie(content).unwrap();
        let poly = &model.levels[0].polygons[0];
        assert_eq!(poly.flags, 0x4200);
        assert!(poly.has_texture());
        assert!(poly.has_tex_anim());
        assert_eq!(poly.anim_frames, Some(4));
        assert_eq!(poly.anim_rate, Some(1));
    }

    #[test]
    fn test_pie4_multi_tileset_textures() {
        let content = r"PIE 4
TYPE 10200
TEXTURE 0 page-34-arizona.png
TEXTURE 1 page-34-urban.png
TEXTURE 2 page-34-rockies.png
TCMASK 0 page-34_tcmask.png
TCMASK 1 page-34_tcmask.png
TCMASK 2 page-34_tcmask.png
LEVELS 1
LEVEL 1
POINTS 3
0 0 0
1 0 0
0 1 0
POLYGONS 1
200 3 0 1 2 0.0 0.0 1.0 0.0 0.5 1.0
";
        let model = parse_pie(content).unwrap();
        assert_eq!(model.texture_pages.len(), 3);
        assert_eq!(model.texture_pages[0], "page-34-arizona.png");
        assert_eq!(model.texture_pages[1], "page-34-urban.png");
        assert_eq!(model.texture_pages[2], "page-34-rockies.png");
        assert_eq!(model.tcmask_pages.len(), 3);
        // texture_page mirrors directive 0.
        assert_eq!(model.texture_page, "page-34-arizona.png");
    }

    #[test]
    fn test_connectors_parsed() {
        let content = "PIE 3\nTYPE 200\nTEXTURE 0 page-1.png 256 256\nLEVELS 1\nLEVEL 1\nPOINTS 3\n0 0 0\n1 0 0\n0 1 0\nPOLYGONS 1\n200 3 0 1 2 0.0 0.0 1.0 0.0 0.5 1.0\nCONNECTORS 2\n0.0 10.0 5.0\n-3.0 12.0 7.0\n";
        let model = parse_pie(content).unwrap();
        assert_eq!(model.levels[0].connectors.len(), 2);
        let c0 = model.levels[0].connectors[0];
        assert_eq!(c0.x, 0.0);
        assert_eq!(c0.y, 10.0);
        assert_eq!(c0.z, 5.0);
        let c1 = model.levels[0].connectors[1];
        assert_eq!(c1.x, -3.0);
        assert_eq!(c1.y, 12.0);
        assert_eq!(c1.z, 7.0);
    }

    #[test]
    fn test_parse_hex_after_various_values() {
        assert_eq!(parse_hex_after("TYPE 0", "TYPE").unwrap(), 0);
        assert_eq!(parse_hex_after("TYPE 200", "TYPE").unwrap(), 0x200);
        assert_eq!(parse_hex_after("TYPE 10200", "TYPE").unwrap(), 0x10200);
        assert_eq!(parse_hex_after("TYPE FF", "TYPE").unwrap(), 0xFF);
        assert_eq!(parse_hex_after("TYPE 10202", "TYPE").unwrap(), 0x10202);
        assert!(parse_hex_after("TYPE notanumber", "TYPE").is_err());
    }

    #[test]
    fn test_pie_version_2_integer_coords() {
        let content = "PIE 2\nTYPE 200\nTEXTURE 0 page-1.png 256 256\nLEVELS 1\nLEVEL 1\nPOINTS 3\n0 0 0\n128 0 0\n0 128 0\nPOLYGONS 1\n200 3 0 1 2 0 0 256 0 0 256\n";
        let model = parse_pie(content).unwrap();
        assert_eq!(model.version, 2);
        assert_eq!(model.levels[0].vertices[1].x, 128.0);
    }

    #[test]
    fn test_unsupported_version_rejected() {
        let content = "PIE 1\nTYPE 200\nLEVELS 0\n";
        let err = parse_pie(content).unwrap_err();
        assert!(matches!(
            err,
            PieError::UnsupportedVersion { version: 1, .. }
        ));

        let content5 = "PIE 5\nTYPE 200\nLEVELS 0\n";
        let err5 = parse_pie(content5).unwrap_err();
        assert!(matches!(
            err5,
            PieError::UnsupportedVersion { version: 5, .. }
        ));
    }
}
