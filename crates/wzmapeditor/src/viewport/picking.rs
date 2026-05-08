//! Ray-terrain and ray-object intersection for mouse picking.

use glam::{Vec3, Vec4};
use wz_maplib::MapData;

use super::camera::Camera;

use wz_maplib::constants::TILE_UNITS_F32 as TILE_UNITS;

/// Bounding box half-extents for objects without a loaded model.
const DEFAULT_OBJECT_HALF_EXTENTS: Vec3 = Vec3::new(64.0, 80.0, 64.0);

/// Floor on per-axis half-extent so small models (walls, lights, fences)
/// stay clickable.
const MIN_PICK_HALF_EXTENT: f32 = 40.0;

/// Generous click area around the visual model AABB.
const PICK_AABB_SCALE: f32 = 1.3;

const MAX_RAY_DISTANCE: f32 = 50000.0;

const BINARY_REFINE_ITERATIONS: u32 = 10;

/// Tile under the cursor, or `None` if the ray misses the terrain.
pub fn screen_to_tile(
    screen_pos: egui::Pos2,
    viewport_rect: egui::Rect,
    camera: &Camera,
    map: &MapData,
) -> Option<(u32, u32)> {
    let ray = screen_to_ray(screen_pos, viewport_rect, camera)?;

    ray_terrain_intersect(ray.0, ray.1, map)
}

/// Unproject a screen position to a world-space ray (origin, direction).
fn screen_to_ray(
    screen_pos: egui::Pos2,
    viewport_rect: egui::Rect,
    camera: &Camera,
) -> Option<(Vec3, Vec3)> {
    let ndc_x = (screen_pos.x - viewport_rect.left()) / viewport_rect.width() * 2.0 - 1.0;
    let ndc_y = 1.0 - (screen_pos.y - viewport_rect.top()) / viewport_rect.height() * 2.0;

    let inv_vp = camera.view_projection_matrix().inverse();

    let near_ndc = Vec4::new(ndc_x, ndc_y, -1.0, 1.0);
    let far_ndc = Vec4::new(ndc_x, ndc_y, 1.0, 1.0);

    let near_world = inv_vp * near_ndc;
    let far_world = inv_vp * far_ndc;

    if near_world.w.abs() < 1e-6 || far_world.w.abs() < 1e-6 {
        return None;
    }

    let near = Vec3::new(
        near_world.x / near_world.w,
        near_world.y / near_world.w,
        near_world.z / near_world.w,
    );
    let far = Vec3::new(
        far_world.x / far_world.w,
        far_world.y / far_world.w,
        far_world.z / far_world.w,
    );

    let direction = (far - near).normalize_or_zero();
    if direction.length_squared() < 0.5 {
        return None;
    }

    Some((near, direction))
}

/// March against the terrain heightfield with coarse stepping then binary refinement.
fn ray_terrain_intersect(origin: Vec3, direction: Vec3, map: &MapData) -> Option<(u32, u32)> {
    let max_dist = MAX_RAY_DISTANCE;
    let step_size = TILE_UNITS * 0.5;
    let max_steps = (max_dist / step_size) as usize;

    let mut prev_pos = origin;
    let mut prev_above = true;
    let mut was_in_bounds = false;

    for i in 1..=max_steps {
        let t = i as f32 * step_size;
        let pos = origin + direction * t;

        let tile_x_f = pos.x / TILE_UNITS;
        let tile_y_f = pos.z / TILE_UNITS;

        if tile_x_f < -1.0
            || tile_y_f < -1.0
            || tile_x_f > (map.width + 1) as f32
            || tile_y_f > (map.height + 1) as f32
        {
            // Once the ray has entered the map and left, stop marching.
            if was_in_bounds {
                return None;
            }
            continue;
        }
        was_in_bounds = true;

        let terrain_h = sample_terrain_height(map, pos.x, pos.z);
        let above = pos.y > terrain_h;

        if !above && prev_above {
            return binary_refine(prev_pos, pos, map, BINARY_REFINE_ITERATIONS);
        }

        prev_pos = pos;
        prev_above = above;
    }

    None
}

fn binary_refine(above: Vec3, below: Vec3, map: &MapData, iterations: u32) -> Option<(u32, u32)> {
    let mut a = above;
    let mut b = below;

    for _ in 0..iterations {
        let mid = (a + b) * 0.5;
        let h = sample_terrain_height(map, mid.x, mid.z);
        if mid.y > h {
            a = mid;
        } else {
            b = mid;
        }
    }

    let hit = (a + b) * 0.5;
    let tx = (hit.x / TILE_UNITS).floor();
    let ty = (hit.z / TILE_UNITS).floor();

    if tx >= 0.0 && ty >= 0.0 && (tx as u32) < map.width && (ty as u32) < map.height {
        Some((tx as u32, ty as u32))
    } else {
        None
    }
}

/// Same as [`ray_terrain_intersect`] but returns the precise world XZ hit.
fn ray_terrain_intersect_precise(
    origin: Vec3,
    direction: Vec3,
    map: &MapData,
) -> Option<(f32, f32)> {
    let step_size = TILE_UNITS * 0.5;
    let max_steps = (MAX_RAY_DISTANCE / step_size) as usize;

    let mut prev_pos = origin;
    let mut prev_above = true;
    let mut was_in_bounds = false;

    for i in 1..=max_steps {
        let t = i as f32 * step_size;
        let pos = origin + direction * t;

        let tile_x_f = pos.x / TILE_UNITS;
        let tile_y_f = pos.z / TILE_UNITS;

        if tile_x_f < -1.0
            || tile_y_f < -1.0
            || tile_x_f > (map.width + 1) as f32
            || tile_y_f > (map.height + 1) as f32
        {
            if was_in_bounds {
                return None;
            }
            continue;
        }
        was_in_bounds = true;

        let terrain_h = sample_terrain_height(map, pos.x, pos.z);
        let above = pos.y > terrain_h;

        if !above && prev_above {
            let hit = binary_refine_precise(prev_pos, pos, map, BINARY_REFINE_ITERATIONS);
            let max_x = (map.width as f32) * TILE_UNITS;
            let max_z = (map.height as f32) * TILE_UNITS;
            if hit.x >= 0.0 && hit.z >= 0.0 && hit.x < max_x && hit.z < max_z {
                return Some((hit.x, hit.z));
            }
            return None;
        }

        prev_pos = pos;
        prev_above = above;
    }

    None
}

fn binary_refine_precise(above: Vec3, below: Vec3, map: &MapData, iterations: u32) -> Vec3 {
    let mut a = above;
    let mut b = below;

    for _ in 0..iterations {
        let mid = (a + b) * 0.5;
        let h = sample_terrain_height(map, mid.x, mid.z);
        if mid.y > h {
            a = mid;
        } else {
            b = mid;
        }
    }

    (a + b) * 0.5
}

/// Public terrain height sampling for object placement.
pub fn sample_terrain_height_pub(map: &MapData, world_x: f32, world_z: f32) -> f32 {
    sample_terrain_height(map, world_x, world_z)
}

/// Sub-tile-accurate terrain hit `(world_x, world_z)` for object placement.
pub fn screen_to_world_pos(
    screen_pos: egui::Pos2,
    viewport_rect: egui::Rect,
    camera: &Camera,
    map: &MapData,
) -> Option<(f32, f32)> {
    let ray = screen_to_ray(screen_pos, viewport_rect, camera)?;
    ray_terrain_intersect_precise(ray.0, ray.1, map)
}

/// Result of picking an object.
#[derive(Debug, Clone, Copy)]
pub struct ObjectPickResult {
    pub kind: crate::app::SelectedObject,
    pub distance: f32,
}

/// Per-overlay toggle gating which categories are eligible for picking.
///
/// Mirrors View-menu toggles so hidden overlays don't steal clicks.
/// Categories without a hide toggle stay always-on.
#[derive(Debug, Clone, Copy)]
pub struct PickVisibility {
    pub labels: bool,
    pub gateways: bool,
}

impl Default for PickVisibility {
    fn default() -> Self {
        Self {
            labels: true,
            gateways: true,
        }
    }
}

pub fn pick_object(
    screen_pos: egui::Pos2,
    viewport_rect: egui::Rect,
    camera: &Camera,
    map: &wz_maplib::WzMap,
    renderer: &super::renderer::EditorRenderer,
    model_loader: Option<&super::model_loader::ModelLoader>,
    stats: Option<&wz_stats::StatsDatabase>,
    visibility: PickVisibility,
) -> Option<ObjectPickResult> {
    let (origin, direction) = screen_to_ray(screen_pos, viewport_rect, camera)?;

    let mut best: Option<ObjectPickResult> = None;

    // Structures use module-aware model lookup.
    for (i, s) in map.structures.iter().enumerate() {
        let (half, center_y) =
            structure_half_extents(&s.name, s.modules, stats, model_loader, renderer);
        let center =
            object_world_center_with_offset(s.position.x, s.position.y, &map.map_data, center_y);
        if let Some(t) = ray_aabb_intersect(origin, direction, center - half, center + half)
            && best.as_ref().is_none_or(|b| t < b.distance)
        {
            best = Some(ObjectPickResult {
                kind: crate::app::SelectedObject::Structure(i),
                distance: t,
            });
        }
    }

    for (i, d) in map.droids.iter().enumerate() {
        let (half, center_y) = object_half_extents_with_center(&d.name, model_loader, renderer);
        let center =
            object_world_center_with_offset(d.position.x, d.position.y, &map.map_data, center_y);
        if let Some(t) = ray_aabb_intersect(origin, direction, center - half, center + half)
            && best.as_ref().is_none_or(|b| t < b.distance)
        {
            best = Some(ObjectPickResult {
                kind: crate::app::SelectedObject::Droid(i),
                distance: t,
            });
        }
    }

    for (i, f) in map.features.iter().enumerate() {
        let (half, center_y) = object_half_extents_with_center(&f.name, model_loader, renderer);
        let center =
            object_world_center_with_offset(f.position.x, f.position.y, &map.map_data, center_y);
        if let Some(t) = ray_aabb_intersect(origin, direction, center - half, center + half)
            && best.as_ref().is_none_or(|b| t < b.distance)
        {
            best = Some(ObjectPickResult {
                kind: crate::app::SelectedObject::Feature(i),
                distance: t,
            });
        }
    }

    // Skip labels when hidden so invisible markers can't steal clicks.
    if visibility.labels {
        for (i, (_key, label)) in map.labels.iter().enumerate() {
            let half = match label {
                wz_maplib::labels::ScriptLabel::Position { .. } => Vec3::new(32.0, 40.0, 32.0),
                wz_maplib::labels::ScriptLabel::Area { pos1, pos2, .. } => {
                    let dx = (pos2[0] as f32 - pos1[0] as f32).abs() * 0.5;
                    let dz = (pos2[1] as f32 - pos1[1] as f32).abs() * 0.5;
                    Vec3::new(dx.max(32.0), 40.0, dz.max(32.0))
                }
            };
            let c = label.center();
            let center = object_world_center_with_offset(
                c.x,
                c.y,
                &map.map_data,
                DEFAULT_OBJECT_HALF_EXTENTS.y,
            );
            if let Some(t) = ray_aabb_intersect(origin, direction, center - half, center + half)
                && best.as_ref().is_none_or(|b| t < b.distance)
            {
                best = Some(ObjectPickResult {
                    kind: crate::app::SelectedObject::Label(i),
                    distance: t,
                });
            }
        }
    }

    // Gateway boxes are lifted a few units off the terrain to match the overlay.
    if visibility.gateways {
        for (i, gw) in map.map_data.gateways.iter().enumerate() {
            let x1 = f32::from(gw.x1) * TILE_UNITS;
            let z1 = f32::from(gw.y1) * TILE_UNITS;
            let x2 = (f32::from(gw.x2) + 1.0) * TILE_UNITS;
            let z2 = (f32::from(gw.y2) + 1.0) * TILE_UNITS;
            let cx = (x1 + x2) * 0.5;
            let cz = (z1 + z2) * 0.5;
            let cy = sample_terrain_height_pub(&map.map_data, cx, cz);
            let min = Vec3::new(x1.min(x2), cy - 8.0, z1.min(z2));
            let max = Vec3::new(x1.max(x2), cy + 40.0, z1.max(z2));
            if let Some(t) = ray_aabb_intersect(origin, direction, min, max)
                && best.as_ref().is_none_or(|b| t < b.distance)
            {
                best = Some(ObjectPickResult {
                    kind: crate::app::SelectedObject::Gateway(i),
                    distance: t,
                });
            }
        }
    }

    best
}

fn project_to_screen(
    world_pos: Vec3,
    vp_matrix: glam::Mat4,
    viewport_rect: egui::Rect,
) -> Option<egui::Pos2> {
    let clip = vp_matrix * Vec4::new(world_pos.x, world_pos.y, world_pos.z, 1.0);
    if clip.w <= 0.0 {
        return None; // Behind camera.
    }
    let ndc = clip.truncate() / clip.w;
    let x = (ndc.x * 0.5 + 0.5) * viewport_rect.width() + viewport_rect.left();
    let y = (0.5 - ndc.y * 0.5) * viewport_rect.height() + viewport_rect.top();
    Some(egui::pos2(x, y))
}

/// Objects whose world-space center projects into the given screen rect.
pub fn objects_in_screen_rect(
    sel_rect: egui::Rect,
    viewport_rect: egui::Rect,
    camera: &Camera,
    map: &wz_maplib::WzMap,
    visibility: PickVisibility,
) -> Vec<crate::app::SelectedObject> {
    let vp = camera.view_projection_matrix();
    let map_data = &map.map_data;
    let mut result = Vec::new();

    for (i, s) in map.structures.iter().enumerate() {
        let center = object_world_center(s.position.x, s.position.y, map_data);
        if let Some(screen_pos) = project_to_screen(center, vp, viewport_rect)
            && sel_rect.contains(screen_pos)
        {
            result.push(crate::app::SelectedObject::Structure(i));
        }
    }

    for (i, d) in map.droids.iter().enumerate() {
        let center = object_world_center(d.position.x, d.position.y, map_data);
        if let Some(screen_pos) = project_to_screen(center, vp, viewport_rect)
            && sel_rect.contains(screen_pos)
        {
            result.push(crate::app::SelectedObject::Droid(i));
        }
    }

    for (i, f) in map.features.iter().enumerate() {
        let center = object_world_center(f.position.x, f.position.y, map_data);
        if let Some(screen_pos) = project_to_screen(center, vp, viewport_rect)
            && sel_rect.contains(screen_pos)
        {
            result.push(crate::app::SelectedObject::Feature(i));
        }
    }

    if visibility.labels {
        for (i, (_key, label)) in map.labels.iter().enumerate() {
            let c = label.center();
            let center = object_world_center(c.x, c.y, map_data);
            if let Some(screen_pos) = project_to_screen(center, vp, viewport_rect)
                && sel_rect.contains(screen_pos)
            {
                result.push(crate::app::SelectedObject::Label(i));
            }
        }
    }

    if visibility.gateways {
        for (i, gw) in map_data.gateways.iter().enumerate() {
            let cx = (f32::from(gw.x1) + f32::from(gw.x2) + 1.0) * 0.5 * TILE_UNITS;
            let cz = (f32::from(gw.y1) + f32::from(gw.y2) + 1.0) * 0.5 * TILE_UNITS;
            let center = Vec3::new(cx, sample_terrain_height_pub(map_data, cx, cz), cz);
            if let Some(screen_pos) = project_to_screen(center, vp, viewport_rect)
                && sel_rect.contains(screen_pos)
            {
                result.push(crate::app::SelectedObject::Gateway(i));
            }
        }
    }

    result
}

/// World-space center of an object on the terrain surface.
pub fn object_world_center(world_x: u32, world_y: u32, map_data: &MapData) -> Vec3 {
    object_world_center_with_offset(world_x, world_y, map_data, DEFAULT_OBJECT_HALF_EXTENTS.y)
}

/// World-space center with a custom vertical offset above the terrain.
pub fn object_world_center_with_offset(
    world_x: u32,
    world_y: u32,
    map_data: &MapData,
    center_y: f32,
) -> Vec3 {
    let wx = world_x as f32;
    let wz = world_y as f32;
    let wy = sample_terrain_height(map_data, wx, wz);
    Vec3::new(wx, wy + center_y, wz)
}

/// Half-extents and AABB vertical center from the cached model.
pub fn object_half_extents_with_center(
    name: &str,
    model_loader: Option<&super::model_loader::ModelLoader>,
    renderer: &super::renderer::EditorRenderer,
) -> (Vec3, f32) {
    if let Some(loader) = model_loader
        && let Some(imd) = loader.imd_for_object(name)
        && let Some(gpu_model) = renderer.models.cache.get(imd)
    {
        return aabb_half_extents_and_center(gpu_model);
    }
    (DEFAULT_OBJECT_HALF_EXTENTS, DEFAULT_OBJECT_HALF_EXTENTS.y)
}

/// Half-extents and AABB vertical center for a structure, accounting for module count.
pub fn structure_half_extents(
    name: &str,
    modules: u8,
    stats: Option<&wz_stats::StatsDatabase>,
    model_loader: Option<&super::model_loader::ModelLoader>,
    renderer: &super::renderer::EditorRenderer,
) -> (Vec3, f32) {
    // Resolve the PIE that matches the structure's installed modules.
    let imd_name: Option<String> = stats
        .and_then(|st| st.structures.get(name))
        .and_then(|ss| ss.pie_model_for_modules(modules))
        .map(ToString::to_string)
        .or_else(|| {
            model_loader
                .and_then(|l| l.imd_for_object(name))
                .map(ToString::to_string)
        });

    if let Some(ref imd) = imd_name
        && let Some(gpu_model) = renderer.models.cache.get(imd)
    {
        return aabb_half_extents_and_center(gpu_model);
    }
    (DEFAULT_OBJECT_HALF_EXTENTS, DEFAULT_OBJECT_HALF_EXTENTS.y)
}

fn aabb_half_extents_and_center(gpu_model: &super::renderer::GpuModel) -> (Vec3, f32) {
    let min = Vec3::from(gpu_model.aabb_min);
    let max = Vec3::from(gpu_model.aabb_max);
    let half = ((max - min) * 0.5 * PICK_AABB_SCALE).max(Vec3::splat(MIN_PICK_HALF_EXTENT));
    let center_y = (min.y + max.y) * 0.5;
    (half, center_y)
}

/// Distance along the ray to the AABB hit, or `None` on miss.
fn ray_aabb_intersect(
    origin: Vec3,
    direction: Vec3,
    aabb_min: Vec3,
    aabb_max: Vec3,
) -> Option<f32> {
    let inv_dir = Vec3::new(
        if direction.x.abs() > 1e-8 {
            1.0 / direction.x
        } else {
            f32::MAX
        },
        if direction.y.abs() > 1e-8 {
            1.0 / direction.y
        } else {
            f32::MAX
        },
        if direction.z.abs() > 1e-8 {
            1.0 / direction.z
        } else {
            f32::MAX
        },
    );

    let t1 = (aabb_min.x - origin.x) * inv_dir.x;
    let t2 = (aabb_max.x - origin.x) * inv_dir.x;
    let t3 = (aabb_min.y - origin.y) * inv_dir.y;
    let t4 = (aabb_max.y - origin.y) * inv_dir.y;
    let t5 = (aabb_min.z - origin.z) * inv_dir.z;
    let t6 = (aabb_max.z - origin.z) * inv_dir.z;

    let tmin = t1.min(t2).max(t3.min(t4)).max(t5.min(t6));
    let tmax = t1.max(t2).min(t3.max(t4)).min(t5.max(t6));

    if tmax < 0.0 || tmin > tmax {
        return None;
    }

    Some(if tmin > 0.0 { tmin } else { tmax })
}

/// Bilinearly interpolated terrain height at world XZ.
fn sample_terrain_height(map: &MapData, world_x: f32, world_z: f32) -> f32 {
    let fx = world_x / TILE_UNITS;
    let fz = world_z / TILE_UNITS;

    let ix = fx.floor() as i32;
    let iz = fz.floor() as i32;

    let frac_x = fx - ix as f32;
    let frac_z = fz - iz as f32;

    let h00 = get_height(map, ix, iz);
    let h10 = get_height(map, ix + 1, iz);
    let h01 = get_height(map, ix, iz + 1);
    let h11 = get_height(map, ix + 1, iz + 1);

    let h_top = h00 + (h10 - h00) * frac_x;
    let h_bot = h01 + (h11 - h01) * frac_x;
    h_top + (h_bot - h_top) * frac_z
}

/// Tile height with bounds clamping.
fn get_height(map: &MapData, x: i32, z: i32) -> f32 {
    let cx = x.max(0).min(map.width as i32 - 1) as u32;
    let cz = z.max(0).min(map.height as i32 - 1) as u32;
    map.tile(cx, cz).map_or(0.0, |t| t.height as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a flat map where every tile has the same height.
    fn flat_map(width: u32, height: u32, tile_height: u16) -> MapData {
        let mut map = MapData::new(width, height);
        for y in 0..height {
            for x in 0..width {
                if let Some(tile) = map.tile_mut(x, y) {
                    tile.height = tile_height;
                }
            }
        }
        map
    }

    #[test]
    fn sample_terrain_height_flat_map_returns_flat_height() {
        let map = flat_map(10, 10, 100);
        // Sample in the middle of the map
        let h = sample_terrain_height(&map, 5.0 * TILE_UNITS, 5.0 * TILE_UNITS);
        assert!(
            (h - 100.0).abs() < 1e-3,
            "Expected ~100.0 on flat map, got {h}"
        );
    }

    #[test]
    fn sample_terrain_height_at_tile_center_returns_tile_height() {
        let mut map = MapData::new(10, 10);
        // Set tile (3, 3) and its neighbors to height 200
        for y in 3..=4 {
            for x in 3..=4 {
                if let Some(tile) = map.tile_mut(x, y) {
                    tile.height = 200;
                }
            }
        }
        // The center of tile (3,3) is at world (3.5 * 128, _, 3.5 * 128).
        // With bilinear interpolation, the center is the average of the four corners.
        // Since all four corners (3,3), (4,3), (3,4), (4,4) are 200, center should be 200.
        let h = sample_terrain_height(&map, 3.5 * TILE_UNITS, 3.5 * TILE_UNITS);
        assert!(
            (h - 200.0).abs() < 1e-3,
            "Expected ~200.0 at tile center, got {h}"
        );
    }

    #[test]
    fn sample_terrain_height_interpolates_between_different_heights() {
        let mut map = MapData::new(10, 10);
        // Set tile (2,2) to height 0, tile (3,2) to height 100
        // Tiles (2,3) and (3,3) stay at 0 (default)
        if let Some(tile) = map.tile_mut(2, 2) {
            tile.height = 0;
        }
        if let Some(tile) = map.tile_mut(3, 2) {
            tile.height = 100;
        }
        // Leave (2,3) and (3,3) at 0

        // At the left edge of tile (2,2): world_x = 2.0 * 128 = 256, world_z = 2.0 * 128 = 256
        // Corners: h00 = tile(2,2) = 0, h10 = tile(3,2) = 100, h01 = tile(2,3) = 0, h11 = tile(3,3) = 0
        // At frac_x = 0.5, frac_z = 0.0:
        //   h_top = 0 + (100 - 0) * 0.5 = 50
        //   h_bot = 0 + (0 - 0) * 0.5 = 0
        //   result = 50 + (0 - 50) * 0.0 = 50
        let h = sample_terrain_height(&map, 2.5 * TILE_UNITS, 2.0 * TILE_UNITS);
        assert!(
            (h - 50.0).abs() < 1e-3,
            "Expected ~50.0 from interpolation, got {h}"
        );
    }

    #[test]
    fn sample_terrain_height_clamps_out_of_range_coordinates() {
        let map = flat_map(10, 10, 50);

        // Negative coordinates should clamp to edge
        let h_neg = sample_terrain_height(&map, -100.0, -100.0);
        assert!(
            (h_neg - 50.0).abs() < 1e-3,
            "Expected ~50.0 for negative coords (clamped), got {h_neg}"
        );

        // Coordinates beyond map bounds should clamp to edge
        let h_over = sample_terrain_height(&map, 20.0 * TILE_UNITS, 20.0 * TILE_UNITS);
        assert!(
            (h_over - 50.0).abs() < 1e-3,
            "Expected ~50.0 for out-of-bounds coords (clamped), got {h_over}"
        );
    }

    #[test]
    fn screen_to_tile_camera_looking_straight_down() {
        // Create a flat map large enough for tile (5,5)
        let map = flat_map(16, 16, 0);

        // Camera positioned directly above tile (5,5) looking almost straight down
        let camera = Camera {
            position: Vec3::new(5.0 * 128.0 + 64.0, 1000.0, 5.0 * 128.0 + 64.0),
            yaw: 0.0,
            pitch: -std::f32::consts::FRAC_PI_2 + 0.01,
            fov: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
            near: 1.0,
            far: 10000.0,
            move_speed: 1.0,
            look_sensitivity: 1.0,
            map_extent: 8192.0,
        };

        // Screen center of a 800x800 viewport
        let viewport_rect =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 800.0));
        let screen_center = egui::pos2(400.0, 400.0);

        let result = screen_to_tile(screen_center, viewport_rect, &camera, &map);
        assert!(result.is_some(), "Expected screen_to_tile to return Some");
        let (tx, ty) = result.unwrap();
        assert_eq!(
            (tx, ty),
            (5, 5),
            "Expected tile (5, 5) but got ({tx}, {ty})"
        );
    }

    #[test]
    fn ray_aabb_hit_along_x_axis() {
        let origin = Vec3::new(-10.0, 0.0, 0.0);
        let direction = Vec3::new(1.0, 0.0, 0.0);
        let aabb_min = Vec3::new(-1.0, -1.0, -1.0);
        let aabb_max = Vec3::new(1.0, 1.0, 1.0);
        let t = ray_aabb_intersect(origin, direction, aabb_min, aabb_max);
        assert!(t.is_some());
        let t = t.unwrap();
        assert!((t - 9.0).abs() < 1e-3, "expected t~9, got {t}");
    }

    #[test]
    fn ray_aabb_miss() {
        let origin = Vec3::new(-10.0, 5.0, 0.0); // above the box
        let direction = Vec3::new(1.0, 0.0, 0.0); // shooting parallel past it
        let aabb_min = Vec3::new(-1.0, -1.0, -1.0);
        let aabb_max = Vec3::new(1.0, 1.0, 1.0);
        assert!(ray_aabb_intersect(origin, direction, aabb_min, aabb_max).is_none());
    }

    #[test]
    fn ray_aabb_origin_inside() {
        let origin = Vec3::new(0.0, 0.0, 0.0); // inside the box
        let direction = Vec3::new(1.0, 0.0, 0.0);
        let aabb_min = Vec3::new(-1.0, -1.0, -1.0);
        let aabb_max = Vec3::new(1.0, 1.0, 1.0);
        let t = ray_aabb_intersect(origin, direction, aabb_min, aabb_max);
        assert!(t.is_some(), "ray starting inside AABB should hit");
        assert!(t.unwrap() >= 0.0);
    }

    #[test]
    fn ray_aabb_behind_ray() {
        // Box is behind the ray origin
        let origin = Vec3::new(10.0, 0.0, 0.0);
        let direction = Vec3::new(1.0, 0.0, 0.0); // shooting away
        let aabb_min = Vec3::new(-1.0, -1.0, -1.0);
        let aabb_max = Vec3::new(1.0, 1.0, 1.0);
        assert!(ray_aabb_intersect(origin, direction, aabb_min, aabb_max).is_none());
    }

    #[test]
    fn ray_aabb_diagonal_hit() {
        let origin = Vec3::new(-10.0, -10.0, -10.0);
        let direction = Vec3::new(1.0, 1.0, 1.0).normalize();
        let aabb_min = Vec3::new(-1.0, -1.0, -1.0);
        let aabb_max = Vec3::new(1.0, 1.0, 1.0);
        assert!(ray_aabb_intersect(origin, direction, aabb_min, aabb_max).is_some());
    }

    #[test]
    fn ray_aabb_parallel_to_face() {
        // Ray parallel to one axis, grazing past box edge
        let origin = Vec3::new(0.0, 1.5, 0.0); // just above
        let direction = Vec3::new(1.0, 0.0, 0.0);
        let aabb_min = Vec3::new(-1.0, -1.0, -1.0);
        let aabb_max = Vec3::new(1.0, 1.0, 1.0);
        assert!(ray_aabb_intersect(origin, direction, aabb_min, aabb_max).is_none());
    }

    #[test]
    fn project_to_screen_center_of_viewport() {
        // yaw=0,pitch=0 → forward is +Z. Camera at z=-10 looking at origin.
        let camera = Camera {
            position: Vec3::new(0.0, 0.0, -10.0),
            yaw: 0.0,
            pitch: 0.0,
            fov: std::f32::consts::FRAC_PI_2,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
            move_speed: 1.0,
            look_sensitivity: 1.0,
            map_extent: 8192.0,
        };
        let vp = camera.view_projection_matrix();
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 800.0));

        // Object at origin, directly in front of camera
        let result = project_to_screen(Vec3::new(0.0, 0.0, 0.0), vp, viewport);
        assert!(result.is_some(), "object in front should project");
        let pos = result.unwrap();
        // Should be near center of viewport
        assert!(
            (pos.x - 400.0).abs() < 50.0,
            "x={} expected near 400",
            pos.x
        );
        assert!(
            (pos.y - 400.0).abs() < 50.0,
            "y={} expected near 400",
            pos.y
        );
    }

    #[test]
    fn project_to_screen_behind_camera_returns_none() {
        // yaw=0,pitch=0 → forward is +Z. Camera at origin.
        let camera = Camera {
            position: Vec3::new(0.0, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            fov: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
            move_speed: 1.0,
            look_sensitivity: 1.0,
            map_extent: 8192.0,
        };
        let vp = camera.view_projection_matrix();
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 800.0));

        // Object behind the camera (negative Z, camera looks +Z)
        let result = project_to_screen(Vec3::new(0.0, 0.0, -20.0), vp, viewport);
        assert!(result.is_none(), "object behind camera should return None");
    }

    #[test]
    fn objects_in_screen_rect_finds_objects_in_view() {
        let map_data = flat_map(16, 16, 0);
        let map = wz_maplib::WzMap {
            map_data,
            structures: vec![wz_maplib::objects::Structure {
                name: "test".to_string(),
                position: wz_maplib::objects::WorldPos { x: 640, y: 640 },
                direction: 0,
                player: 0,
                modules: 0,
                id: None,
            }],
            droids: Vec::new(),
            features: vec![wz_maplib::objects::Feature {
                name: "tree".to_string(),
                position: wz_maplib::objects::WorldPos { x: 256, y: 256 },
                direction: 0,
                id: None,
                player: None,
            }],
            terrain_types: None,
            labels: Vec::new(),
            map_name: String::new(),
            players: 1,
            tileset: String::new(),
            custom_templates_json: None,
            author: None,
            additional_authors: Vec::new(),
            license: None,
        };

        // Camera looking straight down at the map center
        let camera = Camera {
            position: Vec3::new(1024.0, 2000.0, 1024.0),
            yaw: 0.0,
            pitch: -std::f32::consts::FRAC_PI_2 + 0.01,
            fov: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
            near: 1.0,
            far: 10000.0,
            move_speed: 1.0,
            look_sensitivity: 1.0,
            map_extent: 8192.0,
        };

        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 800.0));

        // Select the entire viewport - should find both objects
        let all =
            objects_in_screen_rect(viewport, viewport, &camera, &map, PickVisibility::default());
        assert!(
            all.len() >= 2,
            "full-viewport rect should find at least 2 objects, got {}",
            all.len()
        );

        // A tiny rect in the corner should find nothing (or fewer objects)
        let tiny = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(5.0, 5.0));
        let few = objects_in_screen_rect(tiny, viewport, &camera, &map, PickVisibility::default());
        assert!(few.len() < all.len(), "tiny rect should find fewer objects");
    }
}
