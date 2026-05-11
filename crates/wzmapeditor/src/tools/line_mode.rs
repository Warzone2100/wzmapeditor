//! Shift+click line-draw modifier shared across tile-based brushes.
//!
//! Three brushes (texture paint, ground-type paint, height-brush Set
//! mode) opt in by composing [`LineModeState`] and rasterizing strokes
//! with [`bresenham_tiles`]. The dispatcher and viewport overlay read
//! the same state through the [`Tool::line_mode_state`](super::trait_def::Tool::line_mode_state)
//! hook so cancel and preview behave uniformly across the three tools.

/// Pending line-draw state held by a brush.
///
/// `start` is `None` when the brush is in normal mode. Once
/// `start` is `Some`, the brush is *armed*: the next primary click
/// commits a Bresenham line from `start` to the clicked tile.
#[derive(Debug, Default, Clone, Copy)]
pub struct LineModeState {
    pub start: Option<(u32, u32)>,
    pub hover: Option<(u32, u32)>,
}

impl LineModeState {
    pub fn armed(&self) -> bool {
        self.start.is_some()
    }

    pub fn clear(&mut self) {
        self.start = None;
        self.hover = None;
    }
}

/// Inclusive tile rasterization between `a` and `b` using Bresenham.
///
/// The first element is always `a` and the last is `b`. `a == b`
/// yields a single-element vector. The result contains no duplicates.
pub fn bresenham_tiles(a: (u32, u32), b: (u32, u32)) -> Vec<(u32, u32)> {
    let (mut x, mut y) = (i64::from(a.0), i64::from(a.1));
    let (x1, y1) = (i64::from(b.0), i64::from(b.1));
    let dx = (x1 - x).abs();
    let dy = -(y1 - y).abs();
    let sx: i64 = if x < x1 { 1 } else { -1 };
    let sy: i64 = if y < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    let len = (dx.max(-dy) + 1) as usize;
    let mut out = Vec::with_capacity(len);
    loop {
        out.push((x as u32, y as u32));
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_tile_when_endpoints_equal() {
        assert_eq!(bresenham_tiles((5, 7), (5, 7)), vec![(5, 7)]);
    }

    #[test]
    fn horizontal_line() {
        assert_eq!(
            bresenham_tiles((2, 4), (5, 4)),
            vec![(2, 4), (3, 4), (4, 4), (5, 4)]
        );
    }

    #[test]
    fn vertical_line() {
        assert_eq!(
            bresenham_tiles((3, 1), (3, 4)),
            vec![(3, 1), (3, 2), (3, 3), (3, 4)]
        );
    }

    #[test]
    fn diagonal_45_degrees() {
        assert_eq!(
            bresenham_tiles((0, 0), (3, 3)),
            vec![(0, 0), (1, 1), (2, 2), (3, 3)]
        );
    }

    #[test]
    fn shallow_line_endpoints_match() {
        let line = bresenham_tiles((0, 0), (5, 2));
        assert_eq!(line.first(), Some(&(0, 0)));
        assert_eq!(line.last(), Some(&(5, 2)));
        assert_eq!(line.len(), 6);
    }

    #[test]
    fn steep_line_endpoints_match() {
        let line = bresenham_tiles((0, 0), (2, 5));
        assert_eq!(line.first(), Some(&(0, 0)));
        assert_eq!(line.last(), Some(&(2, 5)));
        assert_eq!(line.len(), 6);
    }

    #[test]
    fn reversed_direction_works() {
        let line = bresenham_tiles((5, 5), (0, 0));
        assert_eq!(line.first(), Some(&(5, 5)));
        assert_eq!(line.last(), Some(&(0, 0)));
        assert_eq!(line.len(), 6);
    }

    #[test]
    fn no_duplicate_tiles() {
        let line = bresenham_tiles((1, 2), (9, 7));
        let mut sorted = line.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), line.len());
    }

    #[test]
    fn armed_flips_on_start_set() {
        let mut s = LineModeState::default();
        assert!(!s.armed());
        s.start = Some((1, 2));
        assert!(s.armed());
        s.clear();
        assert!(!s.armed());
        assert!(s.hover.is_none());
    }
}
