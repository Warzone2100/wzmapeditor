//! Mirror symmetry utilities for terrain tools and object placement.

use super::MirrorMode;

/// Convert WZ2100 rotation+flip to `FlaME`'s (`SwitchedAxes`, `XFlip`, `YFlip`).
pub(crate) fn rot_to_sa(rot: u8, old_flip_x: bool, old_flip_z: bool) -> (bool, bool, bool) {
    let (sa, mut xf, mut yf) = match rot {
        1 => (true, true, false),
        2 => (false, true, true),
        3 => (true, false, true),
        _ => (false, false, false),
    };
    if old_flip_x {
        if sa {
            yf = !yf;
        } else {
            xf = !xf;
        }
    }
    if old_flip_z {
        if sa {
            xf = !xf;
        } else {
            yf = !yf;
        }
    }
    (sa, xf, yf)
}

/// Convert `FlaME`'s (`SwitchedAxes`, `XFlip`, `YFlip`) back to WZ2100 rotation+flip.
pub(crate) fn sa_to_rot(sa: bool, xf: bool, yf: bool) -> (u8, bool, bool) {
    if sa {
        if xf {
            (1, !(xf ^ yf), false)
        } else {
            (3, !(xf ^ yf), false)
        }
    } else if yf {
        (2, xf ^ yf, false)
    } else {
        (0, xf ^ yf, false)
    }
}

/// Up to 4 unique mirrored tile coords (original first). Tile coords
/// mirror around `map_dim - 1`; on-axis points dedupe so tools don't
/// double-apply.
pub fn mirror_points(
    tx: u32,
    ty: u32,
    map_w: u32,
    map_h: u32,
    mode: MirrorMode,
) -> Vec<(u32, u32)> {
    mirror_coords(
        tx,
        ty,
        map_w.saturating_sub(1),
        map_h.saturating_sub(1),
        mode,
    )
}

/// Up to 4 unique mirrored vertex coords (original first). The vertex grid
/// is `map_dim + 1` wide; vertex `v` mirrors to `map_dim - v` (axis at
/// `map_dim / 2`), distinct from the tile-center mirror.
pub fn mirror_vertex_points(
    vx: u32,
    vy: u32,
    map_w: u32,
    map_h: u32,
    mode: MirrorMode,
) -> Vec<(u32, u32)> {
    mirror_coords(vx, vy, map_w, map_h, mode)
}

/// Mirrored world-unit coords. World coords are continuous (not tile-snapped),
/// so the mirror axis is `map_dim * TILE_UNITS`.
pub fn mirror_world_points(
    wx: u32,
    wz: u32,
    map_w: u32,
    map_h: u32,
    mode: MirrorMode,
) -> Vec<(u32, u32)> {
    let tile_units = wz_maplib::constants::TILE_UNITS;
    mirror_coords(wx, wz, map_w * tile_units, map_h * tile_units, mode)
}

/// `axis_x`/`axis_y` are the mirror axis maximums: `map_dim - 1` for tile
/// coords, `map_dim * TILE_UNITS` for world coords.
fn mirror_coords(x: u32, y: u32, axis_x: u32, axis_y: u32, mode: MirrorMode) -> Vec<(u32, u32)> {
    let mut pts = Vec::with_capacity(4);
    pts.push((x, y));

    match mode {
        MirrorMode::None => {}
        MirrorMode::Vertical => {
            push_unique(&mut pts, (axis_x.saturating_sub(x), y));
        }
        MirrorMode::Horizontal => {
            push_unique(&mut pts, (x, axis_y.saturating_sub(y)));
        }
        MirrorMode::Both => {
            let mx = axis_x.saturating_sub(x);
            let my = axis_y.saturating_sub(y);
            push_unique(&mut pts, (mx, y));
            push_unique(&mut pts, (x, my));
            push_unique(&mut pts, (mx, my));
        }
        MirrorMode::Diagonal => {
            // Only valid for square maps; axis_x == axis_y.
            let s = axis_x;
            push_unique(&mut pts, (y, x));
            push_unique(&mut pts, (s.saturating_sub(y), s.saturating_sub(x)));
            push_unique(&mut pts, (s.saturating_sub(x), s.saturating_sub(y)));
        }
    }

    pts
}

/// Transform texture orientation for a mirror point. `point_index` matches
/// `mirror_points` ordering: 0 = original (no transform), 1+ = reflections.
pub fn mirror_orientation(
    rot: u8,
    x_flip: bool,
    y_flip: bool,
    mode: MirrorMode,
    point_index: usize,
) -> (u8, bool, bool) {
    if point_index == 0 || mode == MirrorMode::None {
        return (rot, x_flip, y_flip);
    }

    let (sa, mut xf, mut yf) = rot_to_sa(rot, x_flip, y_flip);

    match mode {
        MirrorMode::None => {}
        MirrorMode::Vertical => {
            if sa {
                yf = !yf;
            } else {
                xf = !xf;
            }
        }
        MirrorMode::Horizontal => {
            if sa {
                xf = !xf;
            } else {
                yf = !yf;
            }
        }
        MirrorMode::Both => match point_index {
            1 => {
                if sa {
                    yf = !yf;
                } else {
                    xf = !xf;
                }
            }
            2 => {
                if sa {
                    xf = !xf;
                } else {
                    yf = !yf;
                }
            }
            _ => {
                xf = !xf;
                yf = !yf;
            }
        },
        MirrorMode::Diagonal => match point_index {
            1 => {
                return sa_to_rot(!sa, xf, yf);
            }
            2 => {
                let new_sa = !sa;
                return sa_to_rot(new_sa, !xf, !yf);
            }
            _ => {
                xf = !xf;
                yf = !yf;
            }
        },
    }

    sa_to_rot(sa, xf, yf)
}

/// Transform an object direction for a mirror point.
/// WZ2100 directions: 0=north, 0x4000=east, 0x8000=south, 0xC000=west.
pub fn mirror_direction(direction: u16, mode: MirrorMode, point_index: usize) -> u16 {
    if point_index == 0 || mode == MirrorMode::None {
        return direction;
    }

    let dir = direction as u32;

    match mode {
        MirrorMode::None => direction,
        MirrorMode::Vertical => (0x10000_u32.wrapping_sub(dir) & 0xFFFF) as u16,
        MirrorMode::Horizontal => (0x8000_u32.wrapping_sub(dir) & 0xFFFF) as u16,
        MirrorMode::Both => match point_index {
            1 => (0x10000_u32.wrapping_sub(dir) & 0xFFFF) as u16,
            2 => (0x8000_u32.wrapping_sub(dir) & 0xFFFF) as u16,
            _ => (dir.wrapping_add(0x8000) & 0xFFFF) as u16,
        },
        MirrorMode::Diagonal => {
            match point_index {
                // Primary diagonal: reflect across 45°.
                1 => (0x4000_u32.wrapping_sub(dir) & 0xFFFF) as u16,
                // Anti-diagonal: reflect across 135°.
                2 => (0xC000_u32.wrapping_sub(dir) & 0xFFFF) as u16,
                _ => (dir.wrapping_add(0x8000) & 0xFFFF) as u16,
            }
        }
    }
}

/// Diagonal mirroring requires a square map; other modes work on any size.
pub fn is_mirror_valid(map_w: u32, map_h: u32, mode: MirrorMode) -> bool {
    match mode {
        MirrorMode::Diagonal => map_w == map_h,
        _ => true,
    }
}

fn push_unique(pts: &mut Vec<(u32, u32)>, pt: (u32, u32)) {
    if !pts.contains(&pt) {
        pts.push(pt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mirror_none() {
        let pts = mirror_points(3, 5, 10, 10, MirrorMode::None);
        assert_eq!(pts, vec![(3, 5)]);
    }

    #[test]
    fn test_mirror_vertical() {
        let pts = mirror_points(3, 5, 10, 10, MirrorMode::Vertical);
        assert_eq!(pts, vec![(3, 5), (6, 5)]);
    }

    #[test]
    fn test_mirror_horizontal() {
        let pts = mirror_points(3, 5, 10, 10, MirrorMode::Horizontal);
        assert_eq!(pts, vec![(3, 5), (3, 4)]);
    }

    #[test]
    fn test_mirror_both() {
        let pts = mirror_points(3, 5, 10, 10, MirrorMode::Both);
        assert_eq!(pts, vec![(3, 5), (6, 5), (3, 4), (6, 4)]);
    }

    #[test]
    fn test_mirror_diagonal() {
        // (2, 7) on 10x10: primary (7,2), anti-diagonal (2,7) dedupes,
        // 180° (7,2) dedupes against primary.
        let pts = mirror_points(2, 7, 10, 10, MirrorMode::Diagonal);
        assert_eq!(pts, vec![(2, 7), (7, 2)]);
    }

    #[test]
    fn test_mirror_diagonal_4_points() {
        let pts = mirror_points(1, 3, 10, 10, MirrorMode::Diagonal);
        assert_eq!(pts, vec![(1, 3), (3, 1), (6, 8), (8, 6)]);
    }

    #[test]
    fn test_mirror_on_axis_dedup() {
        // x=4 on a 9-wide map mirrors to itself (9-1-4 = 4) so dedup.
        let pts = mirror_points(4, 3, 9, 9, MirrorMode::Vertical);
        assert_eq!(pts, vec![(4, 3)]);
    }

    #[test]
    fn test_mirror_corner() {
        let pts = mirror_points(0, 0, 10, 10, MirrorMode::Both);
        assert_eq!(pts, vec![(0, 0), (9, 0), (0, 9), (9, 9)]);
    }

    #[test]
    fn test_mirror_diagonal_nonsquare_invalid() {
        assert!(!is_mirror_valid(10, 8, MirrorMode::Diagonal));
        assert!(is_mirror_valid(10, 10, MirrorMode::Diagonal));
        assert!(is_mirror_valid(10, 8, MirrorMode::Both));
    }

    #[test]
    fn test_mirror_1x1_map() {
        for mode in [
            MirrorMode::None,
            MirrorMode::Vertical,
            MirrorMode::Horizontal,
            MirrorMode::Both,
            MirrorMode::Diagonal,
        ] {
            let pts = mirror_points(0, 0, 1, 1, mode);
            assert_eq!(
                pts,
                vec![(0, 0)],
                "mode={mode:?} should collapse to 1 point"
            );
        }
    }

    #[test]
    fn test_vertex_mirror_vertical() {
        let pts = mirror_vertex_points(3, 5, 10, 10, MirrorMode::Vertical);
        assert_eq!(pts, vec![(3, 5), (7, 5)]);
    }

    #[test]
    fn test_vertex_mirror_horizontal() {
        let pts = mirror_vertex_points(3, 4, 10, 10, MirrorMode::Horizontal);
        assert_eq!(pts, vec![(3, 4), (3, 6)]);
    }

    #[test]
    fn test_vertex_mirror_central_dedup_even_dim() {
        let pts = mirror_vertex_points(5, 3, 10, 10, MirrorMode::Vertical);
        assert_eq!(pts, vec![(5, 3)]);
    }

    #[test]
    fn test_vertex_mirror_corner_both() {
        let pts = mirror_vertex_points(0, 0, 8, 8, MirrorMode::Both);
        assert_eq!(pts, vec![(0, 0), (8, 0), (0, 8), (8, 8)]);
    }

    #[test]
    fn test_vertex_mirror_diagonal_4_points() {
        let pts = mirror_vertex_points(1, 3, 10, 10, MirrorMode::Diagonal);
        assert_eq!(pts, vec![(1, 3), (3, 1), (7, 9), (9, 7)]);
    }

    #[test]
    fn test_vertex_mirror_axis_is_map_dim() {
        let v = 2u32;
        let map = 10u32;
        let pts = mirror_vertex_points(v, 4, map, map, MirrorMode::Vertical);
        assert_eq!(pts[1].0, map - v);
    }

    #[test]
    fn test_world_points_vertical() {
        let tile_units = wz_maplib::constants::TILE_UNITS;
        let pts = mirror_world_points(200, 500, 10, 10, MirrorMode::Vertical);
        assert_eq!(pts, vec![(200, 500), (10 * tile_units - 200, 500)]);
    }

    #[test]
    fn test_world_points_match_tile_points() {
        let tile_units = wz_maplib::constants::TILE_UNITS;
        let tile_pts = mirror_points(3, 5, 10, 10, MirrorMode::Vertical);
        let world_pts = mirror_world_points(
            3 * tile_units + 64,
            5 * tile_units + 64,
            10,
            10,
            MirrorMode::Vertical,
        );
        assert_eq!(tile_pts[1], (6, 5));
        assert_eq!(world_pts[1], (832, 704));
    }

    #[test]
    fn test_orient_vertical_no_rotation() {
        let (r, xf, yf) = mirror_orientation(0, false, false, MirrorMode::Vertical, 1);
        assert_eq!((r, xf, yf), (0, true, false));
    }

    #[test]
    fn test_orient_horizontal_no_rotation() {
        let (r, xf, yf) = mirror_orientation(0, false, false, MirrorMode::Horizontal, 1);
        assert_eq!((r, xf, yf), (2, true, false));
    }

    #[test]
    fn test_orient_vertical_with_rotation() {
        let (r, xf, yf) = mirror_orientation(1, false, false, MirrorMode::Vertical, 1);
        assert_eq!((r, xf, yf), (1, true, false));
    }

    #[test]
    fn test_orient_both_is_180() {
        // Point 3 in Both is a 180° rotation; from rot=0 no-flips that
        // equals rot=2 no-flips.
        let (r, xf, yf) = mirror_orientation(0, false, false, MirrorMode::Both, 3);
        assert_eq!((r, xf, yf), (2, false, false));
    }

    #[test]
    fn test_orient_roundtrip() {
        // Compare SA states because sa_to_rot normalises yf=false.
        for rot in 0..4u8 {
            for &xf in &[false, true] {
                for &yf in &[false, true] {
                    let orig_sa = rot_to_sa(rot, xf, yf);
                    let (r1, x1, y1) = mirror_orientation(rot, xf, yf, MirrorMode::Vertical, 1);
                    let (r2, x2, y2) = mirror_orientation(r1, x1, y1, MirrorMode::Vertical, 1);
                    let result_sa = rot_to_sa(r2, x2, y2);
                    assert_eq!(
                        result_sa, orig_sa,
                        "Vertical roundtrip failed for rot={rot}, xf={xf}, yf={yf}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_orient_all_rotations_vertical() {
        let expected: [(u8, bool, bool, u8, bool, bool); 16] = [
            // (in_rot, in_xf, in_yf, out_rot, out_xf, out_yf)
            (0, false, false, 0, true, false),
            (0, true, false, 0, false, false),
            (0, false, true, 2, false, false),
            (0, true, true, 2, true, false),
            (1, false, false, 1, true, false),
            (1, true, false, 1, false, false),
            (1, false, true, 3, false, false),
            (1, true, true, 3, true, false),
            (2, false, false, 2, true, false),
            (2, true, false, 2, false, false),
            (2, false, true, 0, false, false),
            (2, true, true, 0, true, false),
            (3, false, false, 3, true, false),
            (3, true, false, 3, false, false),
            (3, false, true, 1, false, false),
            (3, true, true, 1, true, false),
        ];
        for (in_r, in_xf, in_yf, exp_r, exp_xf, exp_yf) in expected {
            let (r, xf, yf) = mirror_orientation(in_r, in_xf, in_yf, MirrorMode::Vertical, 1);
            assert_eq!(
                (r, xf, yf),
                (exp_r, exp_xf, exp_yf),
                "Vertical mirror failed for rot={in_r}, xf={in_xf}, yf={in_yf}"
            );
        }
    }

    #[test]
    fn test_dir_vertical_north() {
        assert_eq!(mirror_direction(0, MirrorMode::Vertical, 1), 0);
    }

    #[test]
    fn test_dir_vertical_east() {
        assert_eq!(mirror_direction(0x4000, MirrorMode::Vertical, 1), 0xC000);
    }

    #[test]
    fn test_dir_horizontal_north() {
        assert_eq!(mirror_direction(0, MirrorMode::Horizontal, 1), 0x8000);
    }

    #[test]
    fn test_dir_both_all_cardinals() {
        assert_eq!(mirror_direction(0x4000, MirrorMode::Both, 1), 0xC000);
        assert_eq!(mirror_direction(0x4000, MirrorMode::Both, 2), 0x4000);
        assert_eq!(mirror_direction(0x4000, MirrorMode::Both, 3), 0xC000);

        assert_eq!(mirror_direction(0, MirrorMode::Both, 1), 0);
        assert_eq!(mirror_direction(0, MirrorMode::Both, 2), 0x8000);
        assert_eq!(mirror_direction(0, MirrorMode::Both, 3), 0x8000);
    }

    #[test]
    fn test_dir_diagonal_90() {
        // Primary diagonal: direction = 0x4000 - dir.
        assert_eq!(mirror_direction(0x4000, MirrorMode::Diagonal, 1), 0);
        assert_eq!(mirror_direction(0, MirrorMode::Diagonal, 1), 0x4000);
    }

    #[test]
    fn test_dir_roundtrip() {
        for dir in [0u16, 0x4000, 0x8000, 0xC000, 0x1234] {
            let d1 = mirror_direction(dir, MirrorMode::Vertical, 1);
            let d2 = mirror_direction(d1, MirrorMode::Vertical, 1);
            assert_eq!(d2, dir, "Vertical roundtrip failed for dir=0x{dir:04X}");

            let d1 = mirror_direction(dir, MirrorMode::Horizontal, 1);
            let d2 = mirror_direction(d1, MirrorMode::Horizontal, 1);
            assert_eq!(d2, dir, "Horizontal roundtrip failed for dir=0x{dir:04X}");
        }
    }

    #[test]
    fn test_sa_roundtrip_all() {
        for rot in 0..4u8 {
            for &xf in &[false, true] {
                for &yf in &[false, true] {
                    let (sa, sxf, syf) = rot_to_sa(rot, xf, yf);
                    let (r2, x2, y2) = sa_to_rot(sa, sxf, syf);
                    let (sa2, sxf2, syf2) = rot_to_sa(r2, x2, y2);
                    assert_eq!(
                        (sa, sxf, syf),
                        (sa2, sxf2, syf2),
                        "SA roundtrip failed for rot={rot}, xf={xf}, yf={yf} → ({r2}, {x2}, {y2})"
                    );
                }
            }
        }
    }
}
