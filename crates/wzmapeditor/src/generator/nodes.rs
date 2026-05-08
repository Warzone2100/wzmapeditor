//! Passage node network: a graph of terrain zone control points placed on a
//! regular grid. Each node carries a height level, and edges between
//! neighbors drive how terrain transitions between zones.

use crate::tools::MirrorMode;
use crate::tools::mirror::{is_mirror_valid, mirror_points};

use super::GeneratorConfig;

/// A single passage node in the terrain layout graph.
#[derive(Debug, Clone)]
pub struct PassageNode {
    /// Grid position (in node-grid coordinates).
    pub gx: u32,
    pub gy: u32,
    /// Tile-space center of this node.
    pub tile_x: u32,
    pub tile_y: u32,
    /// Player index (-1 = none).
    pub player: i8,
    /// Indices of connected neighbor nodes.
    pub neighbors: Vec<usize>,
    /// Whether this node sits on the map border (outer ring).
    pub is_border: bool,
}

/// The full passage node network.
#[derive(Debug)]
pub struct NodeNetwork {
    /// All nodes in the network.
    pub nodes: Vec<PassageNode>,
    /// Grid dimensions (how many nodes across/down).
    pub grid_w: u32,
    pub grid_h: u32,
    /// Spacing between nodes in tiles.
    pub node_spacing: u32,
    /// Index of each player's base node (one per player).
    pub player_nodes: Vec<usize>,
}

impl NodeNetwork {
    /// Look up a node index by grid coordinates.
    pub fn node_at(&self, gx: u32, gy: u32) -> Option<usize> {
        if gx < self.grid_w && gy < self.grid_h {
            Some((gy * self.grid_w + gx) as usize)
        } else {
            None
        }
    }
}

/// Build the passage node network for the given generator config.
pub(crate) fn build_node_network(config: &GeneratorConfig, rng: &mut fastrand::Rng) -> NodeNetwork {
    // Adaptive node spacing: roughly 10-12 nodes across the map.
    let node_spacing = (config.width / 10).clamp(6, 20);

    let grid_w = config.width / node_spacing;
    let grid_h = config.height / node_spacing;

    let grid_w = grid_w.max(3);
    let grid_h = grid_h.max(3);

    let mut nodes = Vec::with_capacity((grid_w * grid_h) as usize);
    for gy in 0..grid_h {
        for gx in 0..grid_w {
            // Half-spacing offset centers the grid within the map.
            let tile_x = (gx * node_spacing + node_spacing / 2).min(config.width - 1);
            let tile_y = (gy * node_spacing + node_spacing / 2).min(config.height - 1);
            let is_border = gx == 0 || gy == 0 || gx == grid_w - 1 || gy == grid_h - 1;

            nodes.push(PassageNode {
                gx,
                gy,
                tile_x,
                tile_y,
                player: -1,
                neighbors: Vec::with_capacity(4),
                is_border,
            });
        }
    }

    for gy in 0..grid_h {
        for gx in 0..grid_w {
            let idx = (gy * grid_w + gx) as usize;
            if gx + 1 < grid_w {
                let right = idx + 1;
                nodes[idx].neighbors.push(right);
                nodes[right].neighbors.push(idx);
            }
            if gy + 1 < grid_h {
                let below = idx + grid_w as usize;
                nodes[idx].neighbors.push(below);
                nodes[below].neighbors.push(idx);
            }
        }
    }

    // Edges are added from both endpoints, so dedup.
    for node in &mut nodes {
        node.neighbors.sort_unstable();
        node.neighbors.dedup();
    }

    let player_nodes = place_players(config, &mut nodes, grid_w, grid_h, node_spacing, rng);

    NodeNetwork {
        nodes,
        grid_w,
        grid_h,
        node_spacing,
        player_nodes,
    }
}

/// Place player base nodes using the symmetry mode.
///
/// Returns a vec of node indices, one per player.
fn place_players(
    config: &GeneratorConfig,
    nodes: &mut [PassageNode],
    grid_w: u32,
    grid_h: u32,
    _node_spacing: u32,
    rng: &mut fastrand::Rng,
) -> Vec<usize> {
    let symmetry = if is_mirror_valid(config.width, config.height, config.symmetry) {
        config.symmetry
    } else {
        MirrorMode::None
    };

    let mirror_count = match symmetry {
        MirrorMode::None => 1,
        MirrorMode::Vertical | MirrorMode::Horizontal => 2,
        MirrorMode::Both | MirrorMode::Diagonal => 4,
    };

    let groups = (config.players as u32).div_ceil(mirror_count).max(1);

    let mut player_nodes = Vec::with_capacity(config.players as usize);
    let mut used_grid: Vec<bool> = vec![false; nodes.len()];

    for group_idx in 0..groups {
        // Pick in the top-left quadrant so mirroring covers the other sectors.
        let (base_gx, base_gy) =
            pick_base_position(group_idx, groups, grid_w, grid_h, symmetry, &used_grid, rng);

        let mirror_pts = mirror_points(base_gx, base_gy, grid_w, grid_h, symmetry);

        for (i, &(mx, my)) in mirror_pts.iter().enumerate() {
            if player_nodes.len() >= config.players as usize {
                break;
            }
            let node_idx = (my * grid_w + mx) as usize;
            if node_idx < nodes.len() && !used_grid[node_idx] {
                let player_id = player_nodes.len() as i8;
                nodes[node_idx].player = player_id;
                player_nodes.push(node_idx);
                used_grid[node_idx] = true;

                // Reserve neighbors so two players don't end up adjacent.
                for &ni in &nodes[node_idx].neighbors.clone() {
                    used_grid[ni] = true;
                }
            } else if node_idx < nodes.len()
                && let Some(alt) = find_nearest_free(nodes, mx, my, grid_w, grid_h, &used_grid)
            {
                let player_id = player_nodes.len() as i8;
                nodes[alt].player = player_id;
                player_nodes.push(alt);
                used_grid[alt] = true;
            }
            // No-symmetry mode treats each group independently.
            if symmetry == MirrorMode::None && i == 0 {
                break;
            }
        }
    }

    // Edge case: fall back to filling any free node if symmetry left us short.
    while player_nodes.len() < config.players as usize {
        if let Some(idx) = find_any_free(nodes, &used_grid) {
            let player_id = player_nodes.len() as i8;
            nodes[idx].player = player_id;
            player_nodes.push(idx);
            used_grid[idx] = true;
        } else {
            break;
        }
    }

    player_nodes
}

/// Pick a base position in the sector that belongs to the given group index.
fn pick_base_position(
    group_idx: u32,
    total_groups: u32,
    grid_w: u32,
    grid_h: u32,
    symmetry: MirrorMode,
    used: &[bool],
    rng: &mut fastrand::Rng,
) -> (u32, u32) {
    // Symmetric modes anchor the primary base in the top-left sector; mirror
    // points fill the rest.
    let (x_min, x_max, y_min, y_max) = match symmetry {
        MirrorMode::Vertical => (1, grid_w / 2 - 1, 1, grid_h - 2),
        MirrorMode::Horizontal => (1, grid_w - 2, 1, grid_h / 2 - 1),
        MirrorMode::Both | MirrorMode::Diagonal => (1, grid_w / 2 - 1, 1, grid_h / 2 - 1),
        MirrorMode::None => {
            return pick_perimeter_position(group_idx, total_groups, grid_w, grid_h, used, rng);
        }
    };

    let x_min = x_min.min(grid_w.saturating_sub(2)).max(1);
    let x_max = x_max.max(x_min + 1).min(grid_w - 1);
    let y_min = y_min.min(grid_h.saturating_sub(2)).max(1);
    let y_max = y_max.max(y_min + 1).min(grid_h - 1);

    // Multiple groups in a symmetric sector get subdivided vertically so they
    // don't all stack on one row.
    if total_groups > 1 {
        let sector_h = (y_max - y_min) / total_groups.max(1);
        let sy = y_min + group_idx * sector_h;
        let ey = (sy + sector_h).min(y_max);
        let gx = rng.u32(x_min..x_max);
        let gy = rng.u32(sy.max(y_min)..ey.max(sy + 1));
        return (gx, gy);
    }

    let gx = rng.u32(x_min..x_max);
    let gy = rng.u32(y_min..y_max);
    (gx, gy)
}

/// Distribute players evenly around the map perimeter (no-symmetry mode).
fn pick_perimeter_position(
    group_idx: u32,
    total_groups: u32,
    grid_w: u32,
    grid_h: u32,
    used: &[bool],
    _rng: &mut fastrand::Rng,
) -> (u32, u32) {
    let angle = std::f64::consts::TAU * (group_idx as f64) / (total_groups as f64);
    let cx = grid_w as f64 / 2.0;
    let cy = grid_h as f64 / 2.0;
    let radius = (cx.min(cy) - 1.5).max(1.0);

    let gx = (cx + radius * angle.cos()).round() as u32;
    let gy = (cy + radius * angle.sin()).round() as u32;

    let gx = gx.clamp(1, grid_w.saturating_sub(2));
    let gy = gy.clamp(1, grid_h.saturating_sub(2));

    let idx = (gy * grid_w + gx) as usize;
    if idx < used.len() && !used[idx] {
        (gx, gy)
    } else {
        for r in 1..grid_w.max(grid_h) {
            for dy in -(r as i32)..=(r as i32) {
                for dx in -(r as i32)..=(r as i32) {
                    if dx.unsigned_abs() != r && dy.unsigned_abs() != r {
                        continue;
                    }
                    let nx = gx as i32 + dx;
                    let ny = gy as i32 + dy;
                    if nx >= 1 && ny >= 1 && (nx as u32) < grid_w - 1 && (ny as u32) < grid_h - 1 {
                        let ni = (ny as u32 * grid_w + nx as u32) as usize;
                        if ni < used.len() && !used[ni] {
                            return (nx as u32, ny as u32);
                        }
                    }
                }
            }
        }
        (gx, gy)
    }
}

/// Find the nearest free (non-border, non-used) node to the target grid position.
fn find_nearest_free(
    nodes: &[PassageNode],
    target_gx: u32,
    target_gy: u32,
    _grid_w: u32,
    _grid_h: u32,
    used: &[bool],
) -> Option<usize> {
    let mut best = None;
    let mut best_dist = u32::MAX;

    for (i, node) in nodes.iter().enumerate() {
        if used[i] || node.is_border {
            continue;
        }
        let dx = node.gx.abs_diff(target_gx);
        let dy = node.gy.abs_diff(target_gy);
        let dist = dx * dx + dy * dy;
        if dist < best_dist {
            best_dist = dist;
            best = Some(i);
        }
    }

    // Fall back to border nodes if every interior is taken.
    if best.is_none() {
        for (i, _node) in nodes.iter().enumerate() {
            if used[i] {
                continue;
            }
            let dx = nodes[i].gx.abs_diff(target_gx);
            let dy = nodes[i].gy.abs_diff(target_gy);
            let dist = dx * dx + dy * dy;
            if dist < best_dist {
                best_dist = dist;
                best = Some(i);
            }
        }
    }

    best
}

/// Find any free (non-border) node.
fn find_any_free(nodes: &[PassageNode], used: &[bool]) -> Option<usize> {
    for (i, node) in nodes.iter().enumerate() {
        if !used[i] && !node.is_border {
            return Some(i);
        }
    }
    for (i, _node) in nodes.iter().enumerate() {
        if !used[i] {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> GeneratorConfig {
        GeneratorConfig::default()
    }

    #[test]
    fn test_node_grid_dimensions() {
        let config = GeneratorConfig {
            width: 128,
            height: 128,
            ..default_config()
        };
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        assert!(net.grid_w >= 3, "grid_w={}", net.grid_w);
        assert!(net.grid_h >= 3, "grid_h={}", net.grid_h);
        assert_eq!(net.nodes.len(), (net.grid_w * net.grid_h) as usize);
    }

    #[test]
    fn test_node_grid_small_map() {
        let config = GeneratorConfig {
            width: 48,
            height: 48,
            ..default_config()
        };
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        assert!(net.grid_w >= 3);
        assert!(net.grid_h >= 3);
    }

    #[test]
    fn test_node_connectivity_interior() {
        let config = GeneratorConfig {
            width: 128,
            height: 128,
            ..default_config()
        };
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        for node in &net.nodes {
            if !node.is_border {
                assert_eq!(
                    node.neighbors.len(),
                    4,
                    "Interior node ({},{}) has {} neighbors",
                    node.gx,
                    node.gy,
                    node.neighbors.len()
                );
            }
        }
    }

    #[test]
    fn test_node_connectivity_corner() {
        let config = GeneratorConfig {
            width: 128,
            height: 128,
            ..default_config()
        };
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        let corner = &net.nodes[0];
        assert_eq!(corner.gx, 0);
        assert_eq!(corner.gy, 0);
        assert_eq!(corner.neighbors.len(), 2);
    }

    #[test]
    fn test_player_count_matches() {
        for &players in &[2u8, 3, 4, 6, 8, 10] {
            let config = GeneratorConfig {
                width: 128,
                height: 128,
                players,
                symmetry: MirrorMode::None,
                ..default_config()
            };
            let mut rng = fastrand::Rng::with_seed(42);
            let net = build_node_network(&config, &mut rng);

            assert_eq!(
                net.player_nodes.len(),
                players as usize,
                "Expected {players} players but got {}",
                net.player_nodes.len()
            );
        }
    }

    #[test]
    fn test_player_placement_vertical_symmetry() {
        let config = GeneratorConfig {
            width: 128,
            height: 128,
            players: 2,
            symmetry: MirrorMode::Vertical,
            ..default_config()
        };
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        assert_eq!(net.player_nodes.len(), 2);
        let n0 = &net.nodes[net.player_nodes[0]];
        let n1 = &net.nodes[net.player_nodes[1]];
        assert_ne!(n0.gx, n1.gx, "Players should not be at the same X");
    }

    #[test]
    fn test_player_placement_both_symmetry() {
        let config = GeneratorConfig {
            width: 128,
            height: 128,
            players: 4,
            symmetry: MirrorMode::Both,
            ..default_config()
        };
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        assert_eq!(net.player_nodes.len(), 4);
        let positions: Vec<_> = net
            .player_nodes
            .iter()
            .map(|&idx| (net.nodes[idx].gx, net.nodes[idx].gy))
            .collect();
        for i in 0..positions.len() {
            for j in i + 1..positions.len() {
                assert_ne!(
                    positions[i], positions[j],
                    "Players {i} and {j} at same position"
                );
            }
        }
    }

    #[test]
    fn test_player_nodes_not_on_border() {
        let config = GeneratorConfig {
            width: 128,
            height: 128,
            players: 4,
            symmetry: MirrorMode::Both,
            ..default_config()
        };
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        for &idx in &net.player_nodes {
            assert!(
                !net.nodes[idx].is_border,
                "Player node {} at ({},{}) is on border",
                idx, net.nodes[idx].gx, net.nodes[idx].gy
            );
        }
    }

    #[test]
    fn test_node_tile_positions_in_bounds() {
        let config = GeneratorConfig {
            width: 64,
            height: 96,
            ..default_config()
        };
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        for node in &net.nodes {
            assert!(
                node.tile_x < config.width,
                "tile_x {} >= width {}",
                node.tile_x,
                config.width
            );
            assert!(
                node.tile_y < config.height,
                "tile_y {} >= height {}",
                node.tile_y,
                config.height
            );
        }
    }

    #[test]
    fn test_deterministic_with_same_seed() {
        let config = GeneratorConfig {
            width: 128,
            height: 128,
            players: 4,
            symmetry: MirrorMode::Both,
            ..default_config()
        };

        let mut rng1 = fastrand::Rng::with_seed(123);
        let net1 = build_node_network(&config, &mut rng1);

        let mut rng2 = fastrand::Rng::with_seed(123);
        let net2 = build_node_network(&config, &mut rng2);

        assert_eq!(net1.player_nodes, net2.player_nodes);
        assert_eq!(net1.grid_w, net2.grid_w);
        assert_eq!(net1.grid_h, net2.grid_h);
    }

    #[test]
    fn test_node_at_lookup() {
        let config = default_config();
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        let idx = net.node_at(0, 0);
        assert_eq!(idx, Some(0));

        assert!(net.node_at(net.grid_w, 0).is_none());
        assert!(net.node_at(0, net.grid_h).is_none());
    }

    #[test]
    fn test_large_map_node_count() {
        let config = GeneratorConfig {
            width: 250,
            height: 250,
            ..default_config()
        };
        let mut rng = fastrand::Rng::with_seed(42);
        let net = build_node_network(&config, &mut rng);

        assert!(net.nodes.len() >= 9, "Too few nodes: {}", net.nodes.len());
        assert!(
            net.nodes.len() <= 2500,
            "Too many nodes: {}",
            net.nodes.len()
        );
    }
}
