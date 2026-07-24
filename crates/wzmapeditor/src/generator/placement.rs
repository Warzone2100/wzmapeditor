//! Object placement: player starts, oil resources, scavengers, and decorative features.
//!
//! Uses mirror symmetry to ensure balanced placement across all players.

use wz_maplib::constants::{PLAYER_SCAVENGERS, TILE_UNITS};
use wz_maplib::io_wz::WzMap;
use wz_maplib::objects::{Droid, Feature, Structure, WorldPos};

use crate::config::Tileset;
use crate::tools::mirror::{mirror_direction, mirror_world_points};

use super::GeneratorConfig;
use super::nodes::NodeNetwork;
use super::terrain::LevelAssignment;

/// Place player start objects (HQ, factory, power generator, constructor droids).
pub(crate) fn place_player_starts(
    map: &mut WzMap,
    config: &GeneratorConfig,
    network: &NodeNetwork,
) {
    let tile_units = TILE_UNITS;

    for (player_idx, &node_idx) in network.player_nodes.iter().enumerate() {
        if player_idx >= config.players as usize {
            break;
        }
        let node = &network.nodes[node_idx];
        let player = player_idx as i8;

        let base_x = node.tile_x * tile_units + tile_units / 2;
        let base_y = node.tile_y * tile_units + tile_units / 2;

        let max_x = config.width * tile_units;
        let max_y = config.height * tile_units;
        let clamp_pos = |x: u32, y: u32| -> WorldPos {
            WorldPos {
                x: x.min(max_x - 1),
                y: y.min(max_y - 1),
            }
        };

        map.structures.push(Structure {
            name: "A0CommandCentre".to_string(),
            position: clamp_pos(base_x, base_y),
            direction: 0,
            player,
            modules: 0,
            id: None,
        });

        map.structures.push(Structure {
            name: "A0PowerGenerator".to_string(),
            position: clamp_pos(base_x + tile_units * 3, base_y),
            direction: 0,
            player,
            modules: 0,
            id: None,
        });

        map.structures.push(Structure {
            name: "A0LightFactory".to_string(),
            position: clamp_pos(base_x, base_y + tile_units * 3),
            direction: 0,
            player,
            modules: 0,
            id: None,
        });

        if base_x >= tile_units * 3 {
            map.structures.push(Structure {
                name: "A0ResearchFacility".to_string(),
                position: clamp_pos(base_x - tile_units * 3, base_y),
                direction: 0,
                player,
                modules: 0,
                id: None,
            });
        }

        for truck_idx in 0..config.trucks_per_player {
            let offset_x = ((truck_idx % 4) as u32) * tile_units;
            let offset_y = ((truck_idx / 4) as u32 + 1) * tile_units;
            map.droids.push(Droid {
                name: "ConstructorDroid".to_string(),
                position: clamp_pos(
                    base_x + offset_x,
                    base_y.saturating_sub(tile_units * 2) + offset_y,
                ),
                direction: 0,
                player,
                id: None,
            });
        }
    }
}

/// Place oil resources and decorative features.
pub(crate) fn place_resources(
    map: &mut WzMap,
    config: &GeneratorConfig,
    network: &NodeNetwork,
    levels: &LevelAssignment,
    rng: &mut fastrand::Rng,
) {
    let tile_units = TILE_UNITS;
    let max_x = config.width * tile_units;
    let max_y = config.height * tile_units;

    let oil_candidates: Vec<usize> = (0..network.nodes.len())
        .filter(|&i| !levels.water[i] && !network.nodes[i].is_border && network.nodes[i].player < 0)
        .collect();

    for &node_idx in &network.player_nodes {
        let node = &network.nodes[node_idx];
        let base_x = node.tile_x * tile_units + tile_units / 2;
        let base_y = node.tile_y * tile_units + tile_units / 2;

        for oil_idx in 0..config.base_oil {
            let angle =
                std::f32::consts::TAU * (oil_idx as f32) / (config.base_oil as f32).max(1.0);
            let radius = (tile_units * 5) as f32;
            let ox_f = base_x as f32 + radius * angle.cos();
            let oy_f = base_y as f32 + radius * angle.sin();

            // Reject negative floats before the `as u32` cast: Rust saturates
            // negatives to 0, which would silently dump oil at the map corner
            // and bypass the bounds check below.
            if ox_f < 0.0 || oy_f < 0.0 {
                continue;
            }
            let ox = ox_f as u32;
            let oy = oy_f as u32;

            if ox < max_x && oy < max_y {
                map.features.push(Feature {
                    name: "OilResource".to_string(),
                    position: WorldPos {
                        x: ox.min(max_x - 1),
                        y: oy.min(max_y - 1),
                    },
                    direction: 0,
                    id: None,
                    player: None,
                });
            }
        }
    }

    let mut placed_extra = 0u8;
    let mut attempts = 0u32;
    let max_attempts = config.extra_oil as u32 * 20;

    while placed_extra < config.extra_oil && attempts < max_attempts {
        attempts += 1;

        if oil_candidates.is_empty() {
            break;
        }

        let node_idx = oil_candidates[rng.usize(..oil_candidates.len())];
        let node = &network.nodes[node_idx];

        let jitter_x =
            rng.u32(0..network.node_spacing.max(1)) as i32 - (network.node_spacing / 2) as i32;
        let jitter_y =
            rng.u32(0..network.node_spacing.max(1)) as i32 - (network.node_spacing / 2) as i32;

        let ox = (node.tile_x as i32 * tile_units as i32
            + tile_units as i32 / 2
            + jitter_x * tile_units as i32)
            .clamp(tile_units as i32, (max_x - tile_units) as i32) as u32;
        let oy = (node.tile_y as i32 * tile_units as i32
            + tile_units as i32 / 2
            + jitter_y * tile_units as i32)
            .clamp(tile_units as i32, (max_y - tile_units) as i32) as u32;

        let mirror_pts = mirror_world_points(ox, oy, config.width, config.height, config.symmetry);

        let cluster_size = rng.u8(config.oil_cluster_min..=config.oil_cluster_max);

        for &(mx, my) in &mirror_pts {
            if placed_extra >= config.extra_oil {
                break;
            }
            for c in 0..cluster_size {
                if placed_extra >= config.extra_oil {
                    break;
                }
                let cx = mx as i32 + (c as i32 * tile_units as i32 * 2);
                let cy = my;
                if cx > 0 && (cx as u32) < max_x && cy < max_y {
                    map.features.push(Feature {
                        name: "OilResource".to_string(),
                        position: WorldPos {
                            x: cx as u32,
                            y: cy,
                        },
                        direction: 0,
                        id: None,
                        player: None,
                    });
                    placed_extra += 1;
                }
            }
        }
    }

    if config.scatter_features {
        place_decorative_features(map, config, network, levels, rng);
    }
}

/// Scatter decorative features (trees, boulders, wrecks, ruins) across the map.
fn place_decorative_features(
    map: &mut WzMap,
    config: &GeneratorConfig,
    network: &NodeNetwork,
    levels: &LevelAssignment,
    rng: &mut fastrand::Rng,
) {
    let tile_units = TILE_UNITS;
    let max_x = config.width * tile_units;
    let max_y = config.height * tile_units;

    // Tileset-specific feature pools, names verified against
    // data/base/stats/features.json. Ordered by visual weight: more common,
    // natural features first.
    let feature_names: &[&str] = match config.tileset {
        Tileset::Arizona => &[
            "arizonatree1",
            "arizonatree2",
            "arizonatree3",
            "arizonatree4",
            "arizonatree5",
            "arizonatree6",
            "Tree1",
            "Tree2",
            "Tree3",
            "arizonabush1",
            "arizonabush2",
            "arizonabush3",
            "arizonabush4",
            "Boulder1",
            "Boulder2",
            "Boulder3",
            "Wreck0",
            "Wreck1",
            "Wreck2",
            "Wreck3",
            "Wreck4",
            "Wreck5",
            "Ruin1",
            "Ruin3",
            "Ruin4",
            "Ruin5",
            "LogCabin1",
            "LogCabin2",
            "LogCabin3",
        ],
        Tileset::Urban => &[
            "Wreck0",
            "Wreck1",
            "Wreck2",
            "Wreck3",
            "Wreck4",
            "Wreck5",
            "WreckedBuilding9",
            "WreckedBuilding16",
            "WreckedBuilding17",
            "WreckedDroidHub",
            "WreckedSuzukiJeep",
            "WreckedTankerV",
            "WreckedVertCampVan",
            "Ruin1",
            "Ruin3",
            "Ruin4",
            "Ruin5",
            "Ruin6",
            "Ruin7",
            "Ruin8",
            "Ruin9",
            "Ruin10",
            "BarbTechRuin",
            "BarbWarehouse1",
            "BarbWarehouse2",
            "BarbWarehouse3",
            "BarbHUT",
        ],
        Tileset::Rockies => &[
            "TreeSnow1",
            "TreeSnow2",
            "TreeSnow3",
            "Boulder1",
            "Boulder2",
            "Boulder3",
            "Wreck0",
            "Wreck2",
            "Wreck3",
            "Wreck5",
            "Ruin1",
            "Ruin3",
            "Ruin5",
            "LogCabin1",
            "LogCabin2",
            "LogCabin4",
        ],
    };

    let area = config.width * config.height;
    let target = (area as f32 * config.feature_density * 0.01) as u32;

    let candidates: Vec<usize> = (0..network.nodes.len())
        .filter(|&i| !levels.water[i] && !network.nodes[i].is_border && network.nodes[i].player < 0)
        .collect();

    if candidates.is_empty() {
        return;
    }

    for _ in 0..target {
        let node_idx = candidates[rng.usize(..candidates.len())];
        let node = &network.nodes[node_idx];

        let jx = rng.i32(-(network.node_spacing as i32 / 2)..=(network.node_spacing as i32 / 2));
        let jy = rng.i32(-(network.node_spacing as i32 / 2)..=(network.node_spacing as i32 / 2));

        let fx = ((node.tile_x as i32 + jx) * tile_units as i32 + tile_units as i32 / 2)
            .clamp(tile_units as i32, (max_x - tile_units) as i32) as u32;
        let fy = ((node.tile_y as i32 + jy) * tile_units as i32 + tile_units as i32 / 2)
            .clamp(tile_units as i32, (max_y - tile_units) as i32) as u32;

        let name = feature_names[rng.usize(..feature_names.len())];
        let direction = rng.u16(..=wz_maplib::constants::DIRECTION_MAX);

        let mirror_pts = mirror_world_points(fx, fy, config.width, config.height, config.symmetry);
        for (pi, &(mx, my)) in mirror_pts.iter().enumerate() {
            if mx < max_x && my < max_y {
                map.features.push(Feature {
                    name: name.to_string(),
                    position: WorldPos { x: mx, y: my },
                    direction: mirror_direction(direction, config.symmetry, pi),
                    id: None,
                    player: None,
                });
            }
        }
    }
}

/// Scatter pickup oil drums (instant power when picked up) across the map.
pub(crate) fn place_oil_drums(
    map: &mut WzMap,
    config: &GeneratorConfig,
    network: &NodeNetwork,
    levels: &LevelAssignment,
    rng: &mut fastrand::Rng,
) {
    if config.oil_drums == 0 {
        return;
    }
    let tile_units = TILE_UNITS;
    let max_x = config.width * tile_units;
    let max_y = config.height * tile_units;

    let candidates: Vec<usize> = (0..network.nodes.len())
        .filter(|&i| !levels.water[i] && !network.nodes[i].is_border && network.nodes[i].player < 0)
        .collect();
    if candidates.is_empty() {
        return;
    }

    let mut placed = 0u8;
    let mut attempts = 0u32;
    let max_attempts = u32::from(config.oil_drums) * 10;
    while placed < config.oil_drums && attempts < max_attempts {
        attempts += 1;
        let node = &network.nodes[candidates[rng.usize(..candidates.len())]];
        let jx = rng.i32(-(network.node_spacing as i32 / 2)..=(network.node_spacing as i32 / 2));
        let jy = rng.i32(-(network.node_spacing as i32 / 2)..=(network.node_spacing as i32 / 2));
        let fx = ((node.tile_x as i32 + jx) * tile_units as i32 + tile_units as i32 / 2)
            .clamp(tile_units as i32, (max_x - tile_units) as i32) as u32;
        let fy = ((node.tile_y as i32 + jy) * tile_units as i32 + tile_units as i32 / 2)
            .clamp(tile_units as i32, (max_y - tile_units) as i32) as u32;

        let mirror_pts = mirror_world_points(fx, fy, config.width, config.height, config.symmetry);
        for &(mx, my) in &mirror_pts {
            if placed >= config.oil_drums {
                break;
            }
            if mx < max_x && my < max_y {
                map.features.push(Feature {
                    name: "OilDrum".to_string(),
                    position: WorldPos { x: mx, y: my },
                    direction: 0,
                    id: None,
                    player: None,
                });
                placed += 1;
            }
        }
    }
}

/// Place scavenger base clusters across the map.
///
/// Each base has a small mix of scavenger structures (factory, power, towers,
/// bunkers) plus a handful of scavenger droids. All objects use `player = -1`
/// (`PLAYER_SCAVENGERS`), which serializes as `"scavenger"` in JSON.
///
/// Bases are placed at non-player, non-water, non-border nodes, with
/// symmetry applied so each human player faces roughly the same threat.
pub(crate) fn place_scavengers(
    map: &mut WzMap,
    config: &GeneratorConfig,
    network: &NodeNetwork,
    levels: &LevelAssignment,
    rng: &mut fastrand::Rng,
) {
    if !config.scavengers || config.scavenger_bases == 0 {
        return;
    }
    let tile_units = TILE_UNITS;
    let max_x = config.width * tile_units;
    let max_y = config.height * tile_units;

    // Keep scavengers at least 3 nodes from any player base; also skip water,
    // border, and existing player nodes.
    let min_dist_sq: i32 = 9;
    let candidates: Vec<usize> = (0..network.nodes.len())
        .filter(|&i| {
            if levels.water[i] || network.nodes[i].is_border || network.nodes[i].player >= 0 {
                return false;
            }
            let n = &network.nodes[i];
            network.player_nodes.iter().all(|&p| {
                let pn = &network.nodes[p];
                let dx = n.gx as i32 - pn.gx as i32;
                let dy = n.gy as i32 - pn.gy as i32;
                dx * dx + dy * dy >= min_dist_sq
            })
        })
        .collect();

    if candidates.is_empty() {
        return;
    }

    // Mirroring multiplies the placed count, so divide the requested base
    // count by the symmetry factor to get the number of unique centers.
    let sym_count =
        mirror_world_points(0, 0, config.width, config.height, config.symmetry).len() as u8;
    let unique_bases = (config.scavenger_bases / sym_count.max(1)).max(1);

    let mut used: Vec<usize> = Vec::new();
    for _ in 0..unique_bases {
        // Reject candidates too close to previously placed bases; bail after
        // a few tries so a crowded map doesn't loop forever.
        let mut chosen: Option<usize> = None;
        for _ in 0..16 {
            let idx = candidates[rng.usize(..candidates.len())];
            let node = &network.nodes[idx];
            let too_close = used.iter().any(|&u| {
                let un = &network.nodes[u];
                let dx = node.gx as i32 - un.gx as i32;
                let dy = node.gy as i32 - un.gy as i32;
                dx * dx + dy * dy < 16
            });
            if !too_close {
                chosen = Some(idx);
                break;
            }
        }
        let Some(idx) = chosen else { continue };
        used.push(idx);

        let node = &network.nodes[idx];
        let base_x = node.tile_x * tile_units + tile_units / 2;
        let base_y = node.tile_y * tile_units + tile_units / 2;

        let mirror_pts =
            mirror_world_points(base_x, base_y, config.width, config.height, config.symmetry);
        for (pi, &(mx, my)) in mirror_pts.iter().enumerate() {
            spawn_scavenger_cluster(map, mx, my, max_x, max_y, config.symmetry, pi, rng);
        }
    }
}

/// Spawn one scavenger base at the given world position.
fn spawn_scavenger_cluster(
    map: &mut WzMap,
    cx: u32,
    cy: u32,
    max_x: u32,
    max_y: u32,
    symmetry: crate::tools::MirrorMode,
    mirror_idx: usize,
    rng: &mut fastrand::Rng,
) {
    let t = TILE_UNITS;

    let clamp = |x: i32, y: i32| -> Option<WorldPos> {
        if x < t as i32 || y < t as i32 || x >= (max_x - t) as i32 || y >= (max_y - t) as i32 {
            return None;
        }
        Some(WorldPos {
            x: x as u32,
            y: y as u32,
        })
    };

    let rand_dir = |rng: &mut fastrand::Rng| -> u16 {
        let d = rng.u16(..=wz_maplib::constants::DIRECTION_MAX);
        mirror_direction(d, symmetry, mirror_idx)
    };

    if let Some(pos) = clamp(cx as i32, cy as i32) {
        map.structures.push(Structure {
            name: "A0BaBaFactory".to_string(),
            position: pos,
            direction: rand_dir(rng),
            player: PLAYER_SCAVENGERS,
            modules: 0,
            id: None,
        });
    }
    if let Some(pos) = clamp(cx as i32 + t as i32 * 2, cy as i32) {
        map.structures.push(Structure {
            name: "A0BaBaPowerGenerator".to_string(),
            position: pos,
            direction: rand_dir(rng),
            player: PLAYER_SCAVENGERS,
            modules: 0,
            id: None,
        });
    }

    let tower_pool: &[&str] = &[
        "A0BaBaGunTower",
        "A0BaBaRocketPit",
        "A0BaBaMortarPit",
        "A0BaBaFlameTower",
        "A0CannonTower",
    ];
    let perimeter = [(-2i32, -2i32), (2, -2), (-2, 2), (3, 2), (0, -3)];
    let num_towers = rng.u32(2..=4) as usize;
    for (i, &(dx, dy)) in perimeter.iter().take(num_towers).enumerate() {
        if let Some(pos) = clamp(cx as i32 + dx * t as i32, cy as i32 + dy * t as i32) {
            let name = if i == 0 {
                "A0BaBaBunker"
            } else {
                tower_pool[rng.usize(..tower_pool.len())]
            };
            map.structures.push(Structure {
                name: name.to_string(),
                position: pos,
                direction: rand_dir(rng),
                player: PLAYER_SCAVENGERS,
                modules: 0,
                id: None,
            });
        }
    }

    let combat_pool: &[&str] = &[
        "BabaJeep",
        "BabaBusCan",
        "BabaFireCan",
        "BabaFireTruck",
        "BabaRKJeep",
        "BarbarianBuggy",
        "BarbarianRKBuggy",
        "BarbarianTrike",
    ];
    let num_droids = rng.u32(2..=4) as usize;
    for _ in 0..num_droids {
        let dx = rng.i32(-3..=3);
        let dy = rng.i32(-3..=3);
        if let Some(pos) = clamp(cx as i32 + dx * t as i32, cy as i32 + dy * t as i32) {
            let name = combat_pool[rng.usize(..combat_pool.len())];
            map.droids.push(Droid {
                name: name.to_string(),
                position: pos,
                direction: rand_dir(rng),
                player: PLAYER_SCAVENGERS,
                id: None,
            });
        }
    }

    if rng.bool()
        && let Some(pos) = clamp(cx as i32 + t as i32, cy as i32)
    {
        map.droids.push(Droid {
            name: "BabaPickUp".to_string(),
            position: pos,
            direction: rand_dir(rng),
            player: PLAYER_SCAVENGERS,
            id: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::nodes::build_node_network;
    use crate::generator::terrain::assign_height_levels;
    use crate::tools::MirrorMode;

    fn default_config() -> GeneratorConfig {
        GeneratorConfig {
            seed: 42,
            ..GeneratorConfig::default()
        }
    }

    fn setup(config: &GeneratorConfig) -> (WzMap, NodeNetwork, LevelAssignment) {
        let mut rng = fastrand::Rng::with_seed(config.seed);
        let net = build_node_network(config, &mut rng);
        let la = assign_height_levels(&net, config, &mut rng);
        let mut map = WzMap::new(&config.map_name, config.width, config.height);
        map.players = config.players;
        map.tileset = config.tileset.as_str().to_string();
        (map, net, la)
    }

    #[test]
    fn test_player_starts_per_player() {
        let config = GeneratorConfig {
            players: 4,
            symmetry: MirrorMode::Both,
            trucks_per_player: 3,
            ..default_config()
        };
        let (mut map, net, _la) = setup(&config);
        place_player_starts(&mut map, &config, &net);

        for player in 0..config.players as i8 {
            let structs: Vec<_> = map
                .structures
                .iter()
                .filter(|s| s.player == player)
                .collect();
            assert!(
                structs.len() >= 3,
                "Player {player} has {} structures, expected >=3",
                structs.len()
            );

            let has_hq = structs.iter().any(|s| s.name == "A0CommandCentre");
            assert!(has_hq, "Player {player} missing HQ");

            let has_factory = structs.iter().any(|s| s.name == "A0LightFactory");
            assert!(has_factory, "Player {player} missing factory");

            let has_power = structs.iter().any(|s| s.name == "A0PowerGenerator");
            assert!(has_power, "Player {player} missing power generator");
        }
    }

    #[test]
    fn test_truck_count() {
        let config = GeneratorConfig {
            players: 2,
            trucks_per_player: 5,
            ..default_config()
        };
        let (mut map, net, _la) = setup(&config);
        place_player_starts(&mut map, &config, &net);

        for player in 0..config.players as i8 {
            let droids: Vec<_> = map.droids.iter().filter(|d| d.player == player).collect();
            assert_eq!(
                droids.len(),
                config.trucks_per_player as usize,
                "Player {player} has {} droids, expected {}",
                droids.len(),
                config.trucks_per_player
            );
        }
    }

    #[test]
    fn test_oil_resources_placed() {
        let config = GeneratorConfig {
            base_oil: 4,
            extra_oil: 10,
            ..default_config()
        };
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_resources(&mut map, &config, &net, &la, &mut rng);

        let oil_count = map
            .features
            .iter()
            .filter(|f| f.name == "OilResource")
            .count();

        let min_oil = config.base_oil as usize * config.players as usize;
        assert!(
            oil_count >= min_oil,
            "Expected at least {min_oil} oil, got {oil_count}"
        );
    }

    #[test]
    fn test_extra_oil_without_base_oil() {
        // Regression: extra oil must still place when base_oil is 0.
        let config = GeneratorConfig {
            base_oil: 0,
            extra_oil: 12,
            ..default_config()
        };
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_resources(&mut map, &config, &net, &la, &mut rng);

        let oil_count = map
            .features
            .iter()
            .filter(|f| f.name == "OilResource")
            .count();

        assert!(
            oil_count > 0,
            "Expected extra oil to be placed even with base_oil=0, got {oil_count}"
        );
    }

    #[test]
    fn test_objects_within_bounds() {
        let config = default_config();
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_player_starts(&mut map, &config, &net);
        place_resources(&mut map, &config, &net, &la, &mut rng);

        let max_x = config.width * TILE_UNITS;
        let max_y = config.height * TILE_UNITS;

        for s in &map.structures {
            assert!(
                s.position.x < max_x && s.position.y < max_y,
                "Structure '{}' at ({},{}) out of bounds",
                s.name,
                s.position.x,
                s.position.y
            );
        }
        for d in &map.droids {
            assert!(
                d.position.x < max_x && d.position.y < max_y,
                "Droid '{}' at ({},{}) out of bounds",
                d.name,
                d.position.x,
                d.position.y
            );
        }
        for f in &map.features {
            assert!(
                f.position.x < max_x && f.position.y < max_y,
                "Feature '{}' at ({},{}) out of bounds",
                f.name,
                f.position.x,
                f.position.y
            );
        }
    }

    #[test]
    fn test_decorative_features_placed() {
        let config = GeneratorConfig {
            scatter_features: true,
            feature_density: 0.5,
            ..default_config()
        };
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_resources(&mut map, &config, &net, &la, &mut rng);

        let non_oil = map
            .features
            .iter()
            .filter(|f| f.name != "OilResource")
            .count();
        assert!(non_oil > 0, "Expected decorative features to be placed");
    }

    #[test]
    fn test_no_features_when_disabled() {
        let config = GeneratorConfig {
            scatter_features: false,
            base_oil: 0,
            extra_oil: 0,
            ..default_config()
        };
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_resources(&mut map, &config, &net, &la, &mut rng);

        assert!(
            map.features.is_empty(),
            "Expected no features, got {}",
            map.features.len()
        );
    }

    #[test]
    fn test_scavengers_placed_with_player_minus_one() {
        let config = GeneratorConfig {
            scavengers: true,
            scavenger_bases: 3,
            ..default_config()
        };
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_scavengers(&mut map, &config, &net, &la, &mut rng);

        let scav_structs = map
            .structures
            .iter()
            .filter(|s| s.player == PLAYER_SCAVENGERS)
            .count();
        let scav_droids = map
            .droids
            .iter()
            .filter(|d| d.player == PLAYER_SCAVENGERS)
            .count();

        assert!(
            scav_structs > 0,
            "Expected scavenger structures, got {scav_structs}"
        );
        assert!(
            scav_droids > 0,
            "Expected scavenger droids, got {scav_droids}"
        );

        let max_x = config.width * TILE_UNITS;
        let max_y = config.height * TILE_UNITS;
        for s in map
            .structures
            .iter()
            .filter(|s| s.player == PLAYER_SCAVENGERS)
        {
            assert!(
                s.position.x < max_x && s.position.y < max_y,
                "Scavenger structure '{}' out of bounds at ({},{})",
                s.name,
                s.position.x,
                s.position.y,
            );
        }
    }

    #[test]
    fn test_scavengers_disabled() {
        let config = GeneratorConfig {
            scavengers: false,
            scavenger_bases: 4,
            ..default_config()
        };
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_scavengers(&mut map, &config, &net, &la, &mut rng);

        let any_scav = map.structures.iter().any(|s| s.player == PLAYER_SCAVENGERS)
            || map.droids.iter().any(|d| d.player == PLAYER_SCAVENGERS);
        assert!(!any_scav, "Scavengers placed even though disabled");
    }

    #[test]
    fn test_oil_drums_placed() {
        let config = GeneratorConfig {
            oil_drums: 8,
            ..default_config()
        };
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_oil_drums(&mut map, &config, &net, &la, &mut rng);

        let drums = map.features.iter().filter(|f| f.name == "OilDrum").count();
        assert!(drums > 0, "Expected oil drums, got {drums}");
    }

    #[test]
    fn test_zero_oil_drums() {
        let config = GeneratorConfig {
            oil_drums: 0,
            ..default_config()
        };
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_oil_drums(&mut map, &config, &net, &la, &mut rng);

        let drums = map.features.iter().filter(|f| f.name == "OilDrum").count();
        assert_eq!(drums, 0);
    }

    #[test]
    fn test_scavenger_droid_names_are_valid() {
        let valid: std::collections::HashSet<&str> = [
            "BabaJeep",
            "BabaBusCan",
            "BabaFireCan",
            "BabaFireTruck",
            "BabaRKJeep",
            "BarbarianBuggy",
            "BarbarianRKBuggy",
            "BarbarianTrike",
            "BabaPickUp",
        ]
        .into_iter()
        .collect();

        let config = GeneratorConfig {
            scavengers: true,
            scavenger_bases: 5,
            ..default_config()
        };
        let (mut map, net, la) = setup(&config);
        let mut rng = fastrand::Rng::with_seed(config.seed);
        place_scavengers(&mut map, &config, &net, &la, &mut rng);

        for d in map.droids.iter().filter(|d| d.player == PLAYER_SCAVENGERS) {
            assert!(
                valid.contains(d.name.as_str()),
                "Unknown scavenger droid template: '{}'",
                d.name,
            );
        }
    }

    #[test]
    fn test_zero_trucks() {
        let config = GeneratorConfig {
            trucks_per_player: 0,
            ..default_config()
        };
        let (mut map, net, _la) = setup(&config);
        place_player_starts(&mut map, &config, &net);

        assert!(map.droids.is_empty(), "Expected no droids with 0 trucks");
    }
}
