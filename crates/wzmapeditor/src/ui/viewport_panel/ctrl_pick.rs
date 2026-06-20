//! Ctrl+Click property sampling for terrain and object pickers.

use crate::app::EditorApp;
use crate::tools::{self, ToolId};
use crate::viewport::ViewportResources;
use crate::viewport::camera::Camera;
use crate::viewport::picking;

/// Returns `true` if a picker action was taken (suppresses normal tool application).
pub(super) fn handle_ctrl_click_pick(
    app: &mut EditorApp,
    hover_pos: egui::Pos2,
    rect: egui::Rect,
    camera: &Camera,
) -> bool {
    match app.tool_state.active_tool {
        ToolId::TexturePaint => pick_texture(app, hover_pos, rect, camera),
        ToolId::GroundTypePaint => pick_ground_type(app, hover_pos, rect, camera),
        ToolId::HeightBrush => pick_height(app, hover_pos, rect, camera),
        ToolId::ObjectSelect | ToolId::ObjectPlace => {
            pick_object_for_placement(app, hover_pos, rect, camera)
        }
        ToolId::Gateway
        | ToolId::ScriptLabel
        | ToolId::Stamp
        | ToolId::WallPlacement
        | ToolId::VertexSculpt => false,
    }
}

/// Resolve a screen position to a map tile, passing it to a closure.
///
/// The closure runs while `app.document` is borrowed; returning extracted
/// data lets the caller mutate `app` afterward without borrow conflicts.
fn with_picked_tile<T>(
    app: &EditorApp,
    hover_pos: egui::Pos2,
    rect: egui::Rect,
    camera: &Camera,
    f: impl FnOnce(&wz_maplib::map_data::MapTile) -> T,
) -> Option<T> {
    let doc = app.document.as_ref()?;
    let (tx, ty) = picking::screen_to_tile(hover_pos, rect, camera, &doc.map.map_data)?;
    let tile = doc.map.map_data.tile(tx, ty)?;
    Some(f(tile))
}

fn pick_texture(
    app: &mut EditorApp,
    hover_pos: egui::Pos2,
    rect: egui::Rect,
    camera: &Camera,
) -> bool {
    let Some((tex_id, rot, xf, yf)) = with_picked_tile(app, hover_pos, rect, camera, |tile| {
        (
            tile.texture_id(),
            tile.rotation(),
            tile.x_flip(),
            tile.y_flip(),
        )
    }) else {
        return false;
    };

    if let Some(t) = app.tool_state.texture_paint_mut() {
        t.selected_texture = tex_id;
        t.tile_rotation = rot;
        t.tile_x_flip = xf;
        t.tile_y_flip = yf;
    }

    app.log(format!(
        "Picked texture {tex_id} (rot={rot}, xflip={xf}, yflip={yf})"
    ));
    true
}

fn pick_ground_type(
    app: &mut EditorApp,
    hover_pos: egui::Pos2,
    rect: egui::Rect,
    camera: &Camera,
) -> bool {
    let Some(tex_id) = with_picked_tile(
        app,
        hover_pos,
        rect,
        camera,
        wz_maplib::map_data::MapTile::texture_id,
    ) else {
        return false;
    };

    if let Some(pools) = app.tool_state.tile_membership.get(&tex_id)
        && let Some(&pool_idx) = pools.first()
        && let Some(pool) = app.tool_state.tile_pools.get(pool_idx)
    {
        let gt = pool.ground_type_index;
        let name = pool.name.clone();
        if let Some(brush) = app.tool_state.ground_type_brush_mut() {
            brush.selected_ground_type = gt;
        }
        app.log(format!("Picked ground type: {name} ({gt})"));
        return true;
    }

    // Older maps without a tile-membership entry: fall back to corner types.
    if let Some(ref gd) = app.ground_data
        && let Some(corners) = gd.tile_grounds.get(tex_id as usize)
    {
        let gt = corners[0];
        if let Some(pool) = app
            .tool_state
            .tile_pools
            .iter()
            .find(|p| p.ground_type_index == gt)
        {
            let name = pool.name.clone();
            if let Some(brush) = app.tool_state.ground_type_brush_mut() {
                brush.selected_ground_type = gt;
            }
            app.log(format!("Picked ground type: {name} ({gt})"));
            return true;
        }
    }

    app.log(format!("No ground type found for texture {tex_id}"));
    false
}

fn pick_height(
    app: &mut EditorApp,
    hover_pos: egui::Pos2,
    rect: egui::Rect,
    camera: &Camera,
) -> bool {
    let Some(height) = with_picked_tile(app, hover_pos, rect, camera, |tile| tile.height) else {
        return false;
    };

    if let Some(brush) = app.tool_state.height_brush_mut() {
        brush.target_height = height;
    }
    app.log(format!("Picked height: {height}"));
    true
}

fn pick_object_for_placement(
    app: &mut EditorApp,
    hover_pos: egui::Pos2,
    rect: egui::Rect,
    camera: &Camera,
) -> bool {
    let pick_result = {
        let Some(doc) = app.document.as_ref() else {
            return false;
        };
        let render_state = app.wgpu_render_state.clone();
        let Some(ref rs) = render_state else {
            return false;
        };
        let egui_renderer = rs.renderer.read();
        let Some(resources) = egui_renderer.callback_resources.get::<ViewportResources>() else {
            return false;
        };
        picking::pick_object(
            hover_pos,
            rect,
            camera,
            &doc.map,
            &resources.renderer,
            app.model_loader.as_ref(),
            app.stats.as_ref(),
            picking::PickVisibility {
                labels: app.labels_visible(),
                gateways: app.gateways_visible(),
            },
        )
    };

    let Some(result) = pick_result else {
        return false;
    };

    let (name, category, player, direction) = {
        let Some(doc) = app.document.as_ref() else {
            return false;
        };
        match result.kind {
            crate::app::SelectedObject::Structure(i) => {
                let Some(s) = doc.map.structures.get(i) else {
                    return false;
                };
                (
                    s.name.clone(),
                    tools::AssetCategory::Structures,
                    s.player,
                    s.direction,
                )
            }
            crate::app::SelectedObject::Feature(i) => {
                let Some(f) = doc.map.features.get(i) else {
                    return false;
                };
                (
                    f.name.clone(),
                    tools::AssetCategory::Features,
                    f.player.unwrap_or(0),
                    f.direction,
                )
            }
            crate::app::SelectedObject::Droid(i) => {
                let Some(d) = doc.map.droids.get(i) else {
                    return false;
                };
                (
                    d.name.clone(),
                    tools::AssetCategory::Droids,
                    d.player,
                    d.direction,
                )
            }
            crate::app::SelectedObject::Label(_) | crate::app::SelectedObject::Gateway(_) => {
                return false;
            }
        }
    };

    let category_label = match category {
        tools::AssetCategory::Structures => "structure",
        tools::AssetCategory::Features => "feature",
        tools::AssetCategory::Droids => "droid",
    };
    app.log(format!("Picked {category_label}: {name}"));

    app.tool_state.placement_player = player;
    if let Some(place) = app.tool_state.object_place_mut() {
        place.placement_object = Some(name);
        place.placement_player = player;
        place.placement_direction = direction;
    }
    app.tool_state.asset_category = category;
    app.tool_state.active_tool = ToolId::ObjectPlace;
    // Rebuild the ghost preview with the new object model on the next frame.
    app.objects_dirty = true;
    true
}
