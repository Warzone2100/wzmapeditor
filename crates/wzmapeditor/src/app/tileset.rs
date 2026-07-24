//! Tileset loading and tile pool management.

use super::EditorApp;
use crate::tools::ground_type_brush::{CustomTileGroup, random_group_color};
use crate::ui::tileset_browser::TilesetData;

/// Load tileset textures from the configured data directory.
pub(super) fn try_load_tileset(app: &mut EditorApp, ctx: &egui::Context) {
    let Some(assets) = app.assets.clone() else {
        return;
    };
    let tileset_rel = std::path::PathBuf::from(app.current_tileset.subpath());
    log::info!(
        "Loading tileset {:?} from {}",
        app.current_tileset,
        tileset_rel.display()
    );

    if let Some(ts) = TilesetData::load(ctx, assets.as_ref(), &tileset_rel) {
        app.log(format!(
            "Loaded {} {:?} tileset tiles",
            ts.tile_count(),
            app.current_tileset
        ));
        app.tileset = Some(ts);
    }

    let mut atlas_msg = None;
    if let Some(ref render_state) = app.wgpu_render_state
        && let Some(atlas) = crate::viewport::atlas::TileAtlas::build(assets.as_ref(), &tileset_rel)
    {
        let device = &render_state.device;
        let queue = &render_state.queue;
        let mut egui_renderer = render_state.renderer.write();
        if let Some(resources) = egui_renderer
            .callback_resources
            .get_mut::<crate::viewport::ViewportResources>()
        {
            resources.renderer.upload_atlas(
                device,
                queue,
                &atlas.data,
                atlas.tile_size,
                atlas.layers,
            );
            atlas_msg = Some(format!(
                "Uploaded tileset atlas: {} tiles ({} layers x {}px)",
                atlas.tile_count, atlas.layers, atlas.tile_size
            ));
        }
    }
    if let Some(msg) = atlas_msg {
        app.log(msg);
    }

    // Ground data loads separately after the editor window is visible so it
    // doesn't block the first paint.
}

/// Rebuild tile pools for the ground type brush from current ground and
/// terrain data. If the loaded ground data is for the wrong tileset (a race
/// between the startup background load and `load_map`'s tileset detection),
/// clear it so the correct tileset's data reloads next frame.
pub(super) fn rebuild_tile_pools(app: &mut EditorApp) {
    // Check tileset match before borrowing ground_data.
    let expected_prefix = match app.current_tileset {
        crate::config::Tileset::Arizona => "a_",
        crate::config::Tileset::Urban => "u_",
        crate::config::Tileset::Rockies => "r_",
    };

    let gd_matches = app
        .ground_data
        .as_ref()
        .and_then(|gd| gd.ground_types.first())
        .is_some_and(|gt| gt.name.starts_with(expected_prefix));

    if app.ground_data.is_some() && !gd_matches {
        log::warn!(
            "Ground data tileset mismatch: expected {expected_prefix}*, \
             clearing ground data so correct tileset loads"
        );
        app.tool_state.tile_pools.clear();
        app.ground_data = None;
        return;
    }

    let tileset_key = app.current_tileset.as_str().to_string();
    if let Some(custom_groups) = app.config.custom_tile_groups.get(&tileset_key)
        && !custom_groups.is_empty()
    {
        app.tool_state.tile_pools = custom_groups
            .iter()
            .enumerate()
            .map(|(i, g)| g.to_tile_pool(i as u8))
            .collect();
        app.tool_state.tile_pools_dirty = true;
        log::info!(
            "Loaded {} custom tile groups for {tileset_key}",
            app.tool_state.tile_pools.len(),
        );
        return;
    }

    let default_group = CustomTileGroup {
        name: "Group 1".to_string(),
        color: random_group_color(),
        tiles: Vec::new(),
    };
    app.tool_state.tile_pools = vec![default_group.to_tile_pool(0)];
    app.tool_state.tile_pools_dirty = true;
    app.config
        .custom_tile_groups
        .insert(tileset_key, vec![default_group]);
    app.config.save();
    log::info!("Created default empty tile group");
}

/// Save the current tile pools as custom groups in config.
pub(super) fn save_custom_tile_groups(app: &mut EditorApp) {
    let tileset_key = app.current_tileset.as_str().to_string();
    let custom =
        crate::tools::ground_type_brush::pools_to_custom_groups(&app.tool_state.tile_pools);
    app.config.custom_tile_groups.insert(tileset_key, custom);
    app.config.save();
}

/// Add a new empty tile group with a random color.
pub(super) fn add_new_tile_group(app: &mut EditorApp) {
    let name = app.tool_state.new_group_name.trim().to_string();
    let name = if name.is_empty() {
        format!("Group {}", app.tool_state.tile_pools.len() + 1)
    } else {
        name
    };
    let group = CustomTileGroup {
        name: name.clone(),
        color: random_group_color(),
        tiles: Vec::new(),
    };
    let new_idx = app.tool_state.tile_pools.len() as u8;
    app.tool_state.tile_pools.push(group.to_tile_pool(new_idx));
    if let Some(brush) = app.tool_state.ground_type_brush_mut() {
        brush.selected_ground_type = new_idx;
    }
    app.tool_state.new_group_name.clear();
    app.tool_state.tile_pools_dirty = true;
    save_custom_tile_groups(app);
    log::info!("Added new tile group '{name}'");
}

/// Delete the currently selected tile group.
pub(super) fn delete_selected_tile_group(app: &mut EditorApp) {
    let selected = app
        .tool_state
        .ground_type_brush()
        .map_or(0, |b| b.selected_ground_type);
    let idx = selected as usize;
    if app.tool_state.tile_pools.len() <= 1 {
        // Always keep at least one group.
        return;
    }
    if idx < app.tool_state.tile_pools.len() {
        app.tool_state.tile_pools.remove(idx);
        for (i, pool) in app.tool_state.tile_pools.iter_mut().enumerate() {
            pool.ground_type_index = i as u8;
        }
        let new_len = app.tool_state.tile_pools.len();
        if let Some(brush) = app.tool_state.ground_type_brush_mut()
            && brush.selected_ground_type as usize >= new_len
        {
            brush.selected_ground_type = (new_len - 1) as u8;
        }
        app.tool_state.tile_pools_dirty = true;
        save_custom_tile_groups(app);
        log::info!("Deleted tile group at index {idx}");
    }
}

/// Lazily rebuild pools when ground data becomes available or when the
/// tileset changes. Called each frame; cheap when up to date.
pub(super) fn ensure_tile_pools(app: &mut EditorApp) {
    if app.tool_state.tile_pools.is_empty() {
        rebuild_tile_pools(app);
    }
}
