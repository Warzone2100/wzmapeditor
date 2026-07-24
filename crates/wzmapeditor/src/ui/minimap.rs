//! Minimap overlay rendered in the 3D viewport corner.
//!
//! Generates a terrain-colored texture from the live map data and paints it
//! as an egui overlay with object markers and a camera frustum indicator.

use egui::{Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, Vec2};

use wz_maplib::constants::TILE_UNITS_F32 as TILE_UNITS;
use wz_maplib::map_data::MapData;
use wz_maplib::terrain_types::{TerrainType, TerrainTypeData};

use crate::app::EditorApp;
use crate::config::Tileset;
use crate::viewport::pie_mesh;

fn player_color(player: i8) -> Color32 {
    let c = pie_mesh::team_color(player);
    Color32::from_rgb(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}

const DEFAULT_MINIMAP_SIZE: f32 = 192.0;

struct TilesetColorScheme {
    cliff_low: [u8; 3],
    cliff_high: [u8; 3],
    water: [u8; 3],
    road_low: [u8; 3],
    road_high: [u8; 3],
    ground_low: [u8; 3],
    ground_high: [u8; 3],
}

const ARIZONA_COLORS: TilesetColorScheme = TilesetColorScheme {
    cliff_low: [0x68, 0x3C, 0x24],
    cliff_high: [0xE8, 0x84, 0x5C],
    water: [0x3F, 0x68, 0x9A],
    road_low: [0x24, 0x1F, 0x16],
    road_high: [0xB2, 0x9A, 0x66],
    ground_low: [0x24, 0x1F, 0x16],
    ground_high: [0xCC, 0xB2, 0x80],
};

const URBAN_COLORS: TilesetColorScheme = TilesetColorScheme {
    cliff_low: [0x3C, 0x3C, 0x3C],
    cliff_high: [0x84, 0x84, 0x84],
    water: [0x3F, 0x68, 0x9A],
    road_low: [0x00, 0x00, 0x00],
    road_high: [0x24, 0x1F, 0x16],
    ground_low: [0x1F, 0x1F, 0x1F],
    ground_high: [0xB2, 0xB2, 0xB2],
};

const ROCKIES_COLORS: TilesetColorScheme = TilesetColorScheme {
    cliff_low: [0x3C, 0x3C, 0x3C],
    cliff_high: [0xFF, 0xFF, 0xFF],
    water: [0x3F, 0x68, 0x9A],
    road_low: [0x24, 0x1F, 0x16],
    road_high: [0x3D, 0x21, 0x0A],
    ground_low: [0x00, 0x1C, 0x0E],
    ground_high: [0xFF, 0xFF, 0xFF],
};

fn color_scheme_for(tileset: Tileset) -> &'static TilesetColorScheme {
    match tileset {
        Tileset::Arizona => &ARIZONA_COLORS,
        Tileset::Urban => &URBAN_COLORS,
        Tileset::Rockies => &ROCKIES_COLORS,
    }
}

fn terrain_type_color(
    scheme: &TilesetColorScheme,
    terrain_type: TerrainType,
    height: u16,
) -> Color32 {
    let col = (height / 2).min(255) as f32;
    let t = col / 256.0;

    let (low, high) = match terrain_type {
        TerrainType::Cliffface => (scheme.cliff_low, scheme.cliff_high),
        TerrainType::Water => {
            return Color32::from_rgb(scheme.water[0], scheme.water[1], scheme.water[2]);
        }
        TerrainType::Road => (scheme.road_low, scheme.road_high),
        _ => (scheme.ground_low, scheme.ground_high),
    };

    Color32::from_rgb(
        (low[0] as f32 + (high[0] as f32 - low[0] as f32) * t) as u8,
        (low[1] as f32 + (high[1] as f32 - low[1] as f32) * t) as u8,
        (low[2] as f32 + (high[2] as f32 - low[2] as f32) * t) as u8,
    )
}

pub struct MinimapState {
    pub texture: Option<TextureHandle>,
    pub dirty: bool,
    pub visible: bool,
    /// Display size (longest edge) in logical pixels.
    pub display_size: f32,
}

impl Default for MinimapState {
    fn default() -> Self {
        Self {
            texture: None,
            dirty: true,
            visible: true,
            display_size: DEFAULT_MINIMAP_SIZE,
        }
    }
}

impl std::fmt::Debug for MinimapState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MinimapState")
            .field("has_texture", &self.texture.is_some())
            .field("dirty", &self.dirty)
            .field("visible", &self.visible)
            .field("display_size", &self.display_size)
            .finish()
    }
}

/// Build the minimap pixel buffer. Returns `None` for zero-sized maps.
pub fn build_minimap_image(
    map: &MapData,
    terrain_types: Option<&TerrainTypeData>,
    tileset: Tileset,
) -> Option<ColorImage> {
    let w = map.width as usize;
    let h = map.height as usize;
    if w == 0 || h == 0 {
        return None;
    }

    let scheme = color_scheme_for(tileset);
    let ttp = terrain_types.map(|t| &*t.terrain_types);

    let pixels: Vec<Color32> = (0..h)
        .flat_map(|ty| {
            (0..w).map(move |tx| {
                let tile = map.tile(tx as u32, ty as u32);
                let height = tile.map_or(0, |t| t.height);
                let terrain_type = tile
                    .and_then(|t| ttp.and_then(|types| types.get(t.texture_id() as usize).copied()))
                    .unwrap_or(TerrainType::Sand);
                terrain_type_color(scheme, terrain_type, height)
            })
        })
        .collect();

    Some(ColorImage::new([w, h], pixels))
}

/// (Re)generate the minimap terrain texture.
///
/// Reuses the existing `TextureHandle` (and `TextureId`) when possible so
/// draw commands queued earlier in the frame stay valid across in-frame
/// regeneration. Allocating a fresh texture destroys the old one and
/// crashes the renderer mid-frame.
pub fn regenerate_minimap(
    ctx: &egui::Context,
    state: &mut MinimapState,
    map: &MapData,
    terrain_types: Option<&TerrainTypeData>,
    tileset: Tileset,
) {
    let Some(image) = build_minimap_image(map, terrain_types, tileset) else {
        return;
    };
    if let Some(tex) = state.texture.as_mut() {
        tex.set(image, TextureOptions::NEAREST);
    } else {
        state.texture = Some(ctx.load_texture("viewport_minimap", image, TextureOptions::NEAREST));
    }
    state.dirty = false;
}

pub fn show_minimap_tab(ui: &mut egui::Ui, app: &mut EditorApp) {
    if app.minimap.dirty
        && let Some(ref doc) = app.document
    {
        let ttp = doc.map.terrain_types.as_ref();
        regenerate_minimap(
            ui.ctx(),
            &mut app.minimap,
            &doc.map.map_data,
            ttp,
            app.current_tileset,
        );
    }

    let Some(doc) = app.document.as_ref() else {
        ui.label("No map loaded.");
        return;
    };

    let Some(texture) = app.minimap.texture.as_ref() else {
        return;
    };

    let map_w = doc.map.map_data.width as f32;
    let map_h = doc.map.map_data.height as f32;
    if map_w == 0.0 || map_h == 0.0 {
        return;
    }

    let camera = app.wgpu_render_state.as_ref().and_then(|rs| {
        let renderer = rs.renderer.read();
        renderer
            .callback_resources
            .get::<crate::viewport::ViewportResources>()
            .map(|r| r.camera.clone())
    });

    let avail = ui.available_size();
    let aspect = map_w / map_h;
    let (mm_w, mm_h) = if avail.x / avail.y > aspect {
        (avail.y * aspect, avail.y)
    } else {
        (avail.x, avail.x / aspect)
    };

    let offset_x = (avail.x - mm_w) * 0.5;
    let offset_y = (avail.y - mm_h) * 0.5;
    let (rect, response) = ui.allocate_exact_size(avail, egui::Sense::click());
    let mm_rect = Rect::from_min_size(
        Pos2::new(rect.left() + offset_x, rect.top() + offset_y),
        Vec2::new(mm_w, mm_h),
    );

    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 0.0, Color32::from_gray(30));

    painter.image(
        texture.id(),
        mm_rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );

    let world_to_minimap = |wx: f32, wz: f32| -> Pos2 {
        let map_extent_x = map_w * TILE_UNITS;
        let map_extent_z = map_h * TILE_UNITS;
        let nx = (wx / map_extent_x).clamp(0.0, 1.0);
        let nz = (wz / map_extent_z).clamp(0.0, 1.0);
        Pos2::new(
            mm_rect.left() + nx * mm_rect.width(),
            mm_rect.top() + nz * mm_rect.height(),
        )
    };

    let dot_radius = (mm_w / 100.0).clamp(1.5, 4.0);

    for s in &doc.map.structures {
        let pos = world_to_minimap(s.position.x as f32, s.position.y as f32);
        painter.circle_filled(pos, dot_radius, player_color(s.player));
    }
    for f in &doc.map.features {
        let pos = world_to_minimap(f.position.x as f32, f.position.y as f32);
        let is_oil = f.name == "OilResource";
        let color = if is_oil {
            Color32::from_rgb(255, 255, 0)
        } else {
            f.player
                .map_or(Color32::from_rgb(160, 160, 160), player_color)
        };
        painter.circle_filled(pos, dot_radius * 0.7, color);
    }
    for d in &doc.map.droids {
        let pos = world_to_minimap(d.position.x as f32, d.position.y as f32);
        painter.circle_filled(pos, dot_radius, player_color(d.player));
    }

    if let Some(ref cam) = camera {
        let cam_pos = world_to_minimap(cam.position.x, cam.position.z);
        let (sin_yaw, cos_yaw) = cam.yaw.sin_cos();
        let arrow_len = (mm_w / 20.0).clamp(6.0, 16.0);
        let arrow_half = arrow_len * 0.5;

        let tip = Pos2::new(
            cam_pos.x + sin_yaw * arrow_len,
            cam_pos.y + cos_yaw * arrow_len,
        );
        let left = Pos2::new(
            cam_pos.x - cos_yaw * arrow_half - sin_yaw * 2.0,
            cam_pos.y + sin_yaw * arrow_half - cos_yaw * 2.0,
        );
        let right = Pos2::new(
            cam_pos.x + cos_yaw * arrow_half - sin_yaw * 2.0,
            cam_pos.y - sin_yaw * arrow_half - cos_yaw * 2.0,
        );
        painter.add(egui::Shape::convex_polygon(
            vec![tip, left, right],
            Color32::WHITE,
            egui::Stroke::new(1.0_f32, Color32::BLACK),
        ));
    }

    painter.rect_stroke(
        mm_rect,
        0.0,
        egui::Stroke::new(1.0_f32, Color32::from_gray(100)),
        egui::StrokeKind::Inside,
    );

    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && mm_rect.contains(pos)
    {
        let nx = (pos.x - mm_rect.left()) / mm_rect.width();
        let nz = (pos.y - mm_rect.top()) / mm_rect.height();
        let world_x = nx * map_w * TILE_UNITS;
        let world_z = nz * map_h * TILE_UNITS;

        if let Some(ref render_state) = app.wgpu_render_state {
            let mut renderer = render_state.renderer.write();
            if let Some(resources) = renderer
                .callback_resources
                .get_mut::<crate::viewport::ViewportResources>()
            {
                resources.camera.position.x = world_x;
                resources.camera.position.z = world_z;
            }
        }
    }
}
