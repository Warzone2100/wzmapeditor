// Medium quality terrain: 4-way ground type splatting with classic atlas decal overlay.
// Reference: warzone2100/data/base/shaders/vk/terrain_combined_medium.frag

struct Uniforms {
    mvp: mat4x4<f32>,
    sun_direction: vec4<f32>,
    brush_highlight: vec4<f32>,
    brush_highlight_extra: array<vec4<f32>, 3>,
    camera_pos: vec4<f32>,
    fog_color: vec4<f32>,      // rgb = fog color, a = fog enabled (>0.5)
    fog_params: vec4<f32>,     // x = fog start, y = fog end, z = time, w = unused
    shadow_mvp: mat4x4<f32>,
    map_world_size: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var lightmap_texture: texture_2d<f32>;
@group(0) @binding(2) var lightmap_sampler: sampler;

// Classic tile atlas (used for decal overlay)
@group(1) @binding(0)
var atlas_texture: texture_2d<f32>;
@group(1) @binding(1)
var atlas_sampler: sampler;

// Shadow map
@group(2) @binding(0)
var shadow_map: texture_depth_2d;
@group(2) @binding(1)
var shadow_sampler: sampler_comparison;

// Ground type texture array (one layer per ground type)
@group(3) @binding(0)
var ground_texture: texture_2d_array<f32>;
@group(3) @binding(1)
var ground_sampler: sampler;
// Ground scales packed as 4 vec4s (up to 16 ground types)
@group(3) @binding(2)
var<uniform> ground_scales: array<vec4<f32>, 4>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coord: vec2<f32>,
    @location(3) height_color: f32,
    @location(4) tile_index: f32,
    @location(5) ground_indices: vec4<u32>,
    @location(6) ground_weights: vec4<f32>,
    @location(7) tile_no: i32,
    @location(8) decal_tangent: vec4<f32>,  // unused here; layout matches High shader
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) tex_coord: vec2<f32>,
    @location(2) height_color: f32,
    @location(3) world_xz: vec2<f32>,
    @location(4) @interpolate(flat) tile_index: u32,
    @location(5) world_pos: vec3<f32>,
    @location(6) @interpolate(flat) ground_indices: vec4<u32>,
    @location(7) ground_weights: vec4<f32>,
    @location(8) @interpolate(flat) tile_no: i32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.mvp * vec4<f32>(in.position, 1.0);
    out.world_normal = in.normal;
    out.tex_coord = in.tex_coord;
    out.height_color = in.height_color;
    out.world_xz = in.position.xz;
    out.tile_index = u32(in.tile_index);
    out.world_pos = in.position;
    out.ground_indices = in.ground_indices;
    out.ground_weights = in.ground_weights;
    out.tile_no = in.tile_no;
    return out;
}

fn get_ground_scale(ground_no: u32) -> f32 {
    let vec_idx = ground_no / 4u;
    let comp_idx = ground_no % 4u;
    return ground_scales[vec_idx][comp_idx];
}

fn sample_ground(ground_no: u32, world_xz: vec2<f32>) -> vec3<f32> {
    let scale = get_ground_scale(ground_no);
    // WZ2100 ground UV is (-vertex.z, vertex.x); world_xz=(x,z) maps to (-z, x)/scale.
    let uv = vec2<f32>(-world_xz.y, world_xz.x) / (scale * 128.0);
    return textureSample(ground_texture, ground_sampler, uv, ground_no).rgb;
}

// 3x3 PCF shadow with depth bias to mask acne.
fn compute_shadow(world_pos: vec3<f32>) -> f32 {
    let shadow_pos = uniforms.shadow_mvp * vec4<f32>(world_pos, 1.0);
    let shadow_ndc = shadow_pos.xyz / shadow_pos.w;
    let shadow_uv = vec2<f32>(
        shadow_ndc.x * 0.5 + 0.5,
        -shadow_ndc.y * 0.5 + 0.5,
    );
    let shadow_depth = shadow_ndc.z;

    // WebGPU bans textureSampleCompare in non-uniform control flow, so the
    // out-of-bounds case folds into select() rather than an early return.
    let in_bounds = shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0
        && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0;

    let texel_size = 1.0 / f32(textureDimensions(shadow_map).x);
    var visibility = 0.0;
    let bias = 0.003;
    for (var y = -1i; y <= 1i; y++) {
        for (var x = -1i; x <= 1i; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            visibility += textureSampleCompare(
                shadow_map,
                shadow_sampler,
                shadow_uv + offset,
                shadow_depth - bias,
            );
        }
    }
    return select(1.0, visibility / 9.0, in_bounds);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let g0 = sample_ground(in.ground_indices.x, in.world_xz) * in.ground_weights.x;
    let g1 = sample_ground(in.ground_indices.y, in.world_xz) * in.ground_weights.y;
    let g2 = sample_ground(in.ground_indices.z, in.world_xz) * in.ground_weights.z;
    let g3 = sample_ground(in.ground_indices.w, in.world_xz) * in.ground_weights.w;
    var ground_color = g0 + g1 + g2 + g3;

    // The decal atlas sample is hoisted out of the `tile_no >= 0` branch:
    // WebGPU forbids implicit-LOD sampling in non-uniform control flow, and
    // tile_no is a per-fragment (flat) varying. The atlas UV is always valid.
    let atlas_cols = 16u;
    let tile_col = in.tile_index % atlas_cols;
    let tile_row = in.tile_index / atlas_cols;
    let tile_size = 1.0 / f32(atlas_cols);

    let half_pixel = 0.5 / 256.0;
    let uv_clamped = clamp(in.tex_coord, vec2<f32>(half_pixel), vec2<f32>(1.0 - half_pixel));
    let atlas_uv = vec2<f32>(
        (f32(tile_col) + uv_clamped.x) * tile_size,
        (f32(tile_row) + uv_clamped.y) * tile_size,
    );
    let decal = textureSample(atlas_texture, atlas_sampler, atlas_uv);

    var base_color = ground_color;
    if in.tile_no >= 0 {
        base_color = mix(ground_color, decal.rgb, decal.a);
    }

    let sun_dir = normalize(uniforms.sun_direction.xyz);
    let normal = normalize(in.world_normal);
    let ndotl = max(dot(normal, sun_dir), 0.0);

    let ambient = 0.6;
    let diffuse = 0.8 * ndotl;

    let shadow = compute_shadow(in.world_pos);
    let lit_diffuse = diffuse * mix(0.3, 1.0, shadow);

    // Per-tile AO from R8Unorm lightmap. WZ2100 adaptive gamma: darker tiles get stronger correction.
    let lm_uv = in.world_xz / uniforms.map_world_size.xy;
    let lm_value = textureSample(lightmap_texture, lightmap_sampler, lm_uv).r;
    let tile_brightness = pow(lm_value, 2.0 - lm_value);

    var lit_color = base_color * (ambient * tile_brightness + lit_diffuse);

    // Blinn-Phong specular, shininess=16, strength=0.08.
    let view_dir = normalize(uniforms.camera_pos.xyz - in.world_pos);
    let half_dir = normalize(sun_dir + view_dir);
    let spec = pow(max(dot(normal, half_dir), 0.0), 16.0);
    lit_color += vec3<f32>(1.0, 0.95, 0.85) * spec * 0.08 * shadow;

    if uniforms.brush_highlight.w > 0.5 {
        let brush_center = uniforms.brush_highlight.xy;
        let brush_radius = uniforms.brush_highlight.z;
        let delta = abs(in.world_xz - brush_center);
        let dist = max(delta.x, delta.y);

        if dist < brush_radius {
            let edge = 1.0 - smoothstep(brush_radius * 0.7, brush_radius, dist);
            lit_color = mix(lit_color, vec3<f32>(1.0, 1.0, 1.0), edge * 0.2);
            let ring_dist = abs(dist - brush_radius);
            let ring = 1.0 - smoothstep(0.0, brush_radius * 0.08, ring_dist);
            lit_color = mix(lit_color, vec3<f32>(1.0, 1.0, 0.5), ring * 0.6);
        }
    }
    for (var mi = 0u; mi < 3u; mi = mi + 1u) {
        let bh = uniforms.brush_highlight_extra[mi];
        if bh.w > 0.5 {
            let delta = abs(in.world_xz - bh.xy);
            let dist = max(delta.x, delta.y);
            if dist < bh.z {
                let edge = 1.0 - smoothstep(bh.z * 0.7, bh.z, dist);
                lit_color = mix(lit_color, vec3<f32>(1.0, 1.0, 1.0), edge * 0.2);
                let ring_dist = abs(dist - bh.z);
                let ring = 1.0 - smoothstep(0.0, bh.z * 0.08, ring_dist);
                lit_color = mix(lit_color, vec3<f32>(1.0, 1.0, 0.5), ring * 0.6);
            }
        }
    }

    if uniforms.fog_color.a > 0.5 {
        let dist = distance(in.world_pos, uniforms.camera_pos.xyz);
        let fog_start = uniforms.fog_params.x;
        let fog_end = uniforms.fog_params.y;
        let fog_factor = clamp((fog_end - dist) / (fog_end - fog_start), 0.0, 1.0);
        lit_color = mix(uniforms.fog_color.rgb, lit_color, fog_factor);
    }

    return vec4<f32>(lit_color, 1.0);
}
