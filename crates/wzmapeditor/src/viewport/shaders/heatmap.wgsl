// Propulsion speed heatmap overlay: red (slow) to yellow (100%) to green (fast).

struct Uniforms {
    mvp: mat4x4<f32>,
    sun_direction: vec4<f32>,
    brush_highlight: vec4<f32>,
    brush_highlight_extra: array<vec4<f32>, 3>,
    camera_pos: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    shadow_mvp: mat4x4<f32>,
    map_world_size: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

// terrain_type_lut[texture_id] = terrain_type (0-11). Packed as vec4<u32>
// to keep this a uniform buffer; wgpu's GL backend exposes no fragment-stage SSBOs.
@group(1) @binding(0)
var<uniform> terrain_type_lut: array<vec4<u32>, 128>;

// Per-propulsion speed factors packed 12 floats into 3 vec4s, normalized (1.0 = 100%).
@group(1) @binding(1)
var<uniform> speed_factors: array<vec4<f32>, 3>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coord: vec2<f32>,
    @location(3) height_color: f32,
    @location(4) tile_index: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
    @location(1) @interpolate(flat) tile_index: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // Sit above the grid overlay (which offsets by 0.5).
    let offset_pos = in.position + in.normal * 1.5;
    out.clip_position = uniforms.mvp * vec4<f32>(offset_pos, 1.0);
    out.tex_coord = in.tex_coord;
    out.tile_index = in.tile_index;
    return out;
}

fn get_speed(terrain_type: u32) -> f32 {
    let idx = min(terrain_type, 11u);
    let vec_idx = idx / 4u;
    let comp_idx = idx % 4u;
    return speed_factors[vec_idx][comp_idx];
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tile_idx = min(u32(in.tile_index), 511u);
    let terrain_type = terrain_type_lut[tile_idx / 4u][tile_idx % 4u];
    let speed = get_speed(terrain_type);

    // Impassable terrain (cliff faces for all ground units; water for non-hover)
    // is uploaded as speed 0. Show it as a distinct dark "blocked" tile rather
    // than the slow-end red, which would read as merely slow.
    if speed <= 0.0 {
        return vec4<f32>(0.06, 0.06, 0.08, 0.6);
    }

    // 50% maps to t=0 (red), 100% to t=0.5 (yellow), 150% to t=1 (green).
    let t = clamp((speed - 0.5) / 1.0, 0.0, 1.0);

    var color: vec3<f32>;
    if t < 0.5 {
        let s = t * 2.0;
        color = mix(vec3<f32>(0.9, 0.15, 0.1), vec3<f32>(0.95, 0.85, 0.1), s);
    } else {
        let s = (t - 0.5) * 2.0;
        color = mix(vec3<f32>(0.95, 0.85, 0.1), vec3<f32>(0.1, 0.8, 0.2), s);
    }

    let uv = in.tex_coord;
    let near_edge_x = min(uv.x, 1.0 - uv.x);
    let near_edge_y = min(uv.y, 1.0 - uv.y);
    let near_edge = min(near_edge_x, near_edge_y);
    let edge_darken = smoothstep(0.0, 0.06, near_edge);
    color = color * mix(0.7, 1.0, edge_darken);

    return vec4<f32>(color, 0.45);
}
