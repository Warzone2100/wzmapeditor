// High quality terrain: 4-way splatting with tangent-space normal maps and
// Blinn-Phong specular from gloss maps. Decals use dedicated diffuse/normal/specular
// arrays instead of the classic atlas.
// Reference: warzone2100/data/base/shaders/vk/terrain_combined_high.frag

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
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var lightmap_texture: texture_2d<f32>;
@group(0) @binding(2) var lightmap_sampler: sampler;

@group(1) @binding(0) var atlas_texture: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;

@group(3) @binding(0) var ground_texture: texture_2d_array<f32>;
@group(3) @binding(1) var ground_sampler: sampler;
@group(3) @binding(2) var<uniform> ground_scales_raw: array<vec4<f32>, 4>;
@group(3) @binding(3) var normal_texture: texture_2d_array<f32>;
@group(3) @binding(4) var specular_texture: texture_2d_array<f32>;
// One array layer per tile index.
@group(3) @binding(5) var decal_texture: texture_2d_array<f32>;
@group(3) @binding(6) var decal_normal_texture: texture_2d_array<f32>;
@group(3) @binding(7) var decal_specular_texture: texture_2d_array<f32>;
@group(3) @binding(8) var decal_sampler: sampler;

fn get_ground_scale(idx: u32) -> f32 {
    return ground_scales_raw[idx / 4u][idx % 4u];
}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coord: vec2<f32>,
    @location(3) height_color: f32,
    @location(4) tile_index: f32,
    @location(5) ground_indices: vec4<u32>,
    @location(6) ground_weights: vec4<f32>,
    @location(7) tile_no: i32,
    @location(8) decal_tangent: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coord: vec2<f32>,
    @location(3) height_color: f32,
    @location(4) tile_index: f32,
    @location(5) @interpolate(flat) ground_indices: vec4<u32>,
    @location(6) ground_weights: vec4<f32>,
    @location(7) @interpolate(flat) tile_no: i32,
    @location(8) shadow_pos: vec4<f32>,
    // TBN-space light + half vector, computed per-vertex like terrain_combined.vert.
    @location(9) ground_light_dir: vec3<f32>,
    @location(10) ground_half_vec: vec3<f32>,
    // mat2 packed (col0.x, col0.y, col1.x, col1.y) mapping decal tangent to ground tangent.
    @location(11) decal2ground: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_pos = u.mvp * vec4<f32>(in.position, 1.0);
    out.world_pos = in.position;
    out.normal = in.normal;
    out.tex_coord = in.tex_coord;
    out.height_color = in.height_color;
    out.tile_index = in.tile_index;
    out.ground_indices = in.ground_indices;
    out.ground_weights = in.ground_weights;
    out.tile_no = in.tile_no;
    out.shadow_pos = u.shadow_mvp * vec4<f32>(in.position, 1.0);

    // Ground TBN per terrain_combined.vert: vaxis=(1,0,0), T=cross(N,vaxis), B=cross(N,T),
    // ModelTangentMatrix = transpose(mat3(T,B,N)) so it transforms world into tangent space.
    let N = normalize(in.normal);
    let vaxis = vec3<f32>(1.0, 0.0, 0.0);
    let T = normalize(cross(N, vaxis));
    let B = cross(N, T);
    let tbn_t = mat3x3<f32>(
        vec3<f32>(T.x, B.x, N.x),
        vec3<f32>(T.y, B.y, N.y),
        vec3<f32>(T.z, B.z, N.z),
    );

    let eye_vec = tbn_t * normalize(u.camera_pos.xyz - in.position);
    out.ground_light_dir = tbn_t * normalize(u.sun_direction.xyz);
    out.ground_half_vec = out.ground_light_dir + eye_vec;

    // Matches frag.decal2groundMat2 in WZ2100.
    if in.tile_no >= 0 {
        let decal_T = in.decal_tangent.xyz;
        let decal_B = -cross(N, decal_T) * in.decal_tangent.w;
        out.decal2ground = vec4<f32>(
            dot(decal_T, T), dot(decal_B, T),
            dot(decal_T, B), dot(decal_B, B),
        );
    } else {
        out.decal2ground = vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }

    return out;
}

fn sample_shadow(shadow_pos: vec4<f32>) -> f32 {
    let proj = shadow_pos.xyz / shadow_pos.w;
    let uv = proj.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    // WebGPU bans textureSampleCompare in non-uniform control flow, so the
    // out-of-bounds case folds into select() rather than an early return.
    let in_bounds = uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0 && proj.z <= 1.0;
    let bias = 0.003;
    let depth = proj.z - bias;
    let tex_size = vec2<f32>(textureDimensions(shadow_map));
    let texel = 1.0 / tex_size;
    var shadow = 0.0;
    for (var y = -1i; y <= 1i; y++) {
        for (var x = -1i; x <= 1i; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            shadow += textureSampleCompare(shadow_map, shadow_sampler, uv + offset, depth);
        }
    }
    return select(1.0, shadow / 9.0, in_bounds);
}

fn ground_uv(ground_no: u32, world_xz: vec2<f32>) -> vec2<f32> {
    let scale = get_ground_scale(ground_no);
    return vec2<f32>(-world_xz.y, world_xz.x) / (scale * 128.0);
}

fn sample_ground_color(ground_no: u32, world_xz: vec2<f32>) -> vec3<f32> {
    let uv = ground_uv(ground_no, world_xz);
    return textureSample(ground_texture, ground_sampler, uv, ground_no).rgb;
}

fn sample_ground_normal(ground_no: u32, world_xz: vec2<f32>) -> vec3<f32> {
    let uv = ground_uv(ground_no, world_xz);
    let n = textureSample(normal_texture, ground_sampler, uv, ground_no).rgb;
    // Decode 0..1 to -1..1; black sample falls back to flat up.
    let decoded = normalize(n * 2.0 - 1.0);
    let is_zero = step(dot(n, n), 0.001);
    return mix(decoded, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(is_zero));
}

fn sample_ground_specular(ground_no: u32, world_xz: vec2<f32>) -> f32 {
    let uv = ground_uv(ground_no, world_xz);
    return textureSample(specular_texture, ground_sampler, uv, ground_no).r;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let world_xz = vec2<f32>(in.world_pos.x, in.world_pos.z);
    let ground_indices = in.ground_indices;
    let w = in.ground_weights;

    var color = vec3<f32>(0.0);
    color += sample_ground_color(ground_indices.x, world_xz) * w.x;
    color += sample_ground_color(ground_indices.y, world_xz) * w.y;
    color += sample_ground_color(ground_indices.z, world_xz) * w.z;
    color += sample_ground_color(ground_indices.w, world_xz) * w.w;

    var ts_normal = vec3<f32>(0.0);
    ts_normal += sample_ground_normal(ground_indices.x, world_xz) * w.x;
    ts_normal += sample_ground_normal(ground_indices.y, world_xz) * w.y;
    ts_normal += sample_ground_normal(ground_indices.z, world_xz) * w.z;
    ts_normal += sample_ground_normal(ground_indices.w, world_xz) * w.w;
    ts_normal = normalize(ts_normal);

    var gloss = 0.0;
    gloss += sample_ground_specular(ground_indices.x, world_xz) * w.x;
    gloss += sample_ground_specular(ground_indices.y, world_xz) * w.y;
    gloss += sample_ground_specular(ground_indices.z, world_xz) * w.z;
    gloss += sample_ground_specular(ground_indices.w, world_xz) * w.w;

    // Matches main_bumpMapping() in terrain_combined_high.frag.
    // Decal array samples are hoisted out of the `tile_no >= 0` branch (WebGPU
    // bans implicit-LOD sampling in non-uniform control flow; tile_no is a
    // per-fragment varying). The index is clamped so the sample is always valid.
    let has_decal = in.tile_no >= 0;
    let decal_idx = max(in.tile_no, 0);
    let uv = in.tex_coord;
    let decal_color = textureSample(decal_texture, decal_sampler, uv, decal_idx);
    let dn_raw = textureSample(decal_normal_texture, decal_sampler, uv, decal_idx).rgb;
    let decal_spec = textureSample(decal_specular_texture, decal_sampler, uv, decal_idx).r;
    if has_decal {
        let a = decal_color.a;
        color = mix(color, decal_color.rgb, a);

        let dn = normalize(dn_raw * 2.0 - 1.0);
        let dn_is_zero = step(dot(dn_raw, dn_raw), 0.001);
        // WZ2100: n_normalized.xy * decal2groundMat2.
        let d2g = mat2x2<f32>(
            in.decal2ground.x, in.decal2ground.z,
            in.decal2ground.y, in.decal2ground.w,
        );
        let decal_n = mix(
            vec3<f32>(d2g * dn.xy, dn.z),
            vec3<f32>(0.0, 0.0, 1.0),
            vec3<f32>(dn_is_zero),
        );
        ts_normal = mix(ts_normal, decal_n, a);

        gloss = mix(gloss, decal_spec, a);
    }

    // Tangent-space lighting per doBumpMapping(). Ambient 0.6 vs WZ2100's 0.5
    // compensates for the missing additive lightmap RGB term in the editor.
    let ambient_light = vec3<f32>(0.6, 0.6, 0.6);
    let diffuse_light = vec3<f32>(1.0, 1.0, 1.0);
    let specular_light = vec3<f32>(1.0, 1.0, 1.0);

    let L = normalize(in.ground_light_dir);
    let diffuse_factor = max(dot(ts_normal, L), 0.0);

    let shadow = sample_shadow(in.shadow_pos);
    let visibility = min(diffuse_factor, shadow * diffuse_factor);

    // Blinn exponent = reflectionValue * (1 - specMap^2), reflectionValue=16 per terrain_combined_high.frag.
    let H = normalize(in.ground_half_vec);
    let blinn_dot = max(dot(ts_normal, H), 0.0);
    let reflection_value = 16.0;
    let spec_exponent = reflection_value * (1.0 - gloss * gloss);
    let spec_factor = pow(blinn_dot, spec_exponent);

    // Per-tile lightmap (R8Unorm) with WZ2100 adaptive gamma.
    let lm_uv = vec2<f32>(in.world_pos.x, in.world_pos.z) / u.map_world_size.xy;
    let lm_value = textureSample(lightmap_texture, lightmap_sampler, lm_uv).r;
    let tile_brightness = pow(lm_value, 2.0 - lm_value);

    // WZ2100: res = color * light + light_spec.
    let light = ambient_light * tile_brightness + diffuse_light * visibility;
    let light_spec = specular_light * spec_factor * visibility * (gloss * gloss);
    var final_color = color * light + light_spec;

    if u.brush_highlight.w > 0.5 {
        let brush_center = u.brush_highlight.xy;
        let brush_radius = u.brush_highlight.z;
        let world_xz = vec2<f32>(in.world_pos.x, in.world_pos.z);
        let delta = abs(world_xz - brush_center);
        let dist = max(delta.x, delta.y);

        if dist < brush_radius {
            let edge = 1.0 - smoothstep(brush_radius * 0.7, brush_radius, dist);
            final_color = mix(final_color, vec3<f32>(1.0, 1.0, 1.0), edge * 0.2);
            let ring_dist = abs(dist - brush_radius);
            let ring = 1.0 - smoothstep(0.0, brush_radius * 0.08, ring_dist);
            final_color = mix(final_color, vec3<f32>(1.0, 1.0, 0.5), ring * 0.6);
        }
    }
    for (var mi = 0u; mi < 3u; mi = mi + 1u) {
        let bh = u.brush_highlight_extra[mi];
        if bh.w > 0.5 {
            let world_xz = vec2<f32>(in.world_pos.x, in.world_pos.z);
            let delta = abs(world_xz - bh.xy);
            let dist = max(delta.x, delta.y);
            if dist < bh.z {
                let edge = 1.0 - smoothstep(bh.z * 0.7, bh.z, dist);
                final_color = mix(final_color, vec3<f32>(1.0, 1.0, 1.0), edge * 0.2);
                let ring_dist = abs(dist - bh.z);
                let ring = 1.0 - smoothstep(0.0, bh.z * 0.08, ring_dist);
                final_color = mix(final_color, vec3<f32>(1.0, 1.0, 0.5), ring * 0.6);
            }
        }
    }

    if u.fog_color.a > 0.5 {
        let cam_dist = length(in.world_pos - u.camera_pos.xyz);
        let fog_start = u.fog_params.x;
        let fog_end = u.fog_params.y;
        let fog_factor = clamp((fog_end - cam_dist) / (fog_end - fog_start), 0.0, 1.0);
        final_color = mix(u.fog_color.rgb, final_color, fog_factor);
    }

    return vec4<f32>(final_color, 1.0);
}
