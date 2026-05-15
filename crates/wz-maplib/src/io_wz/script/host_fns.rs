//! Host functions exposed to `game.js`.
//!
//! Ported from `runMap_*` in `lib/wzmaplib/src/map_script.cpp` (Warzone
//! 2100, GPL-2.0-or-later).

use std::ffi::{CStr, CString};

use quickjs_wz_sys as qjs;

use super::runtime::{ScriptError, ScriptState, jsvalue_to_string};
use crate::map_data::{MapData, MapTile};
use crate::objects::{Droid, Feature, Structure, WorldPos};

// Warzone caps (`map_internal.h`).
const MAX_PLAYERS: i32 = 11;
const MAP_MAXWIDTH: u32 = 256;
const MAP_MAXHEIGHT: u32 = 256;
const MAP_MAXAREA: u64 = (MAP_MAXWIDTH as u64) * (MAP_MAXHEIGHT as u64);
const TILE_MAX_HEIGHT: u32 = 0xFFFF;

/// Pull the per-context `ScriptState` set by `run_script_source`.
///
/// # Safety
///
/// Only callable from a host function whose context was created by
/// `run_script_source`; the opaque pointer must still point at a live
/// `ScriptState`.
unsafe fn state<'a>(ctx: *mut qjs::JSContext) -> Option<&'a mut ScriptState> {
    let opaque = unsafe { qjs::JS_GetContextOpaque(ctx) };
    if opaque.is_null() {
        return None;
    }
    Some(unsafe { &mut *opaque.cast::<ScriptState>() })
}

/// Throw a reference error formatted as a plain string.
unsafe fn throw(ctx: *mut qjs::JSContext, msg: &str) -> qjs::JSValue {
    let fmt = c"%s";
    let bytes = CString::new(msg).unwrap_or_else(|_| c"<bad utf8>".to_owned());
    unsafe { qjs::JS_ThrowReferenceError(ctx, fmt.as_ptr(), bytes.as_ptr()) }
}

// ---------- Argument-reading helpers (host-side, infallible on success). ----------

fn js_is_number(v: qjs::JSValue) -> bool {
    // Mirrors the inline `JS_IsNumber` (tag is INT or FLOAT64).
    let tag = unsafe { qjs::wzqjs_value_get_tag(v) };
    tag == qjs::JS_TAG_INT || tag == qjs::JS_TAG_FLOAT64
}

fn js_is_array(ctx: *mut qjs::JSContext, v: qjs::JSValue) -> bool {
    unsafe { qjs::JS_IsArray(ctx, v) != 0 }
}

fn js_is_function(ctx: *mut qjs::JSContext, v: qjs::JSValue) -> bool {
    unsafe { qjs::JS_IsFunction(ctx, v) != 0 }
}

fn js_array_length(ctx: *mut qjs::JSContext, arr: qjs::JSValue) -> Option<u64> {
    if !js_is_array(ctx, arr) {
        return None;
    }
    let key = c"length";
    let len_val = unsafe { qjs::JS_GetPropertyStr(ctx, arr, key.as_ptr()) };
    if unsafe { qjs::wzqjs_is_exception(len_val) } != 0 {
        return None;
    }
    let mut out: u64 = 0;
    let rc = unsafe { qjs::JS_ToIndex(ctx, &raw mut out, len_val) };
    unsafe { qjs::wzqjs_free_value(ctx, len_val) };
    if rc != 0 { None } else { Some(out) }
}

fn js_to_u32(ctx: *mut qjs::JSContext, v: qjs::JSValue) -> u32 {
    let mut out: u32 = 0;
    let _ = unsafe { qjs::wzqjs_to_uint32(ctx, &raw mut out, v) };
    out
}

fn js_to_i32(ctx: *mut qjs::JSContext, v: qjs::JSValue) -> i32 {
    let mut out: i32 = 0;
    let _ = unsafe { qjs::JS_ToInt32(ctx, &raw mut out, v) };
    out
}

fn obj_get_str(ctx: *mut qjs::JSContext, obj: qjs::JSValue, key: &CStr) -> String {
    let v = unsafe { qjs::JS_GetPropertyStr(ctx, obj, key.as_ptr()) };
    let s = jsvalue_to_string(ctx, v);
    unsafe { qjs::wzqjs_free_value(ctx, v) };
    s
}

fn obj_get_i32(ctx: *mut qjs::JSContext, obj: qjs::JSValue, key: &CStr) -> i32 {
    let v = unsafe { qjs::JS_GetPropertyStr(ctx, obj, key.as_ptr()) };
    let n = js_to_i32(ctx, v);
    unsafe { qjs::wzqjs_free_value(ctx, v) };
    n
}

fn obj_get_u32(ctx: *mut qjs::JSContext, obj: qjs::JSValue, key: &CStr) -> u32 {
    let v = unsafe { qjs::JS_GetPropertyStr(ctx, obj, key.as_ptr()) };
    let n = js_to_u32(ctx, v);
    unsafe { qjs::wzqjs_free_value(ctx, v) };
    n
}

fn obj_get_position(ctx: *mut qjs::JSContext, obj: qjs::JSValue) -> Option<WorldPos> {
    let pos = unsafe { qjs::JS_GetPropertyStr(ctx, obj, c"position".as_ptr()) };
    let result = (|| -> Option<WorldPos> {
        if !js_is_array(ctx, pos) {
            return None;
        }
        let len = js_array_length(ctx, pos)?;
        if len != 2 {
            return None;
        }
        let x_v = unsafe { qjs::JS_GetPropertyUint32(ctx, pos, 0) };
        let y_v = unsafe { qjs::JS_GetPropertyUint32(ctx, pos, 1) };
        let x = js_to_i32(ctx, x_v).max(0) as u32;
        let y = js_to_i32(ctx, y_v).max(0) as u32;
        unsafe {
            qjs::wzqjs_free_value(ctx, x_v);
            qjs::wzqjs_free_value(ctx, y_v);
        }
        Some(WorldPos { x, y })
    })();
    unsafe { qjs::wzqjs_free_value(ctx, pos) };
    result
}

// ---------- gameRand / log ----------

unsafe extern "C" fn game_rand(
    ctx: *mut qjs::JSContext,
    _this: qjs::JSValue,
    argc: i32,
    argv: *const qjs::JSValue,
) -> qjs::JSValue {
    let Some(state) = (unsafe { state(ctx) }) else {
        return unsafe { throw(ctx, "context state missing") };
    };
    let num = state.rng.next_u32();
    let modulus = if argc >= 1 {
        let arg0 = unsafe { *argv };
        let mut m: u32 = 0;
        if unsafe { qjs::wzqjs_to_uint32(ctx, &raw mut m, arg0) } != 0 {
            return unsafe { qjs::wzqjs_exception() };
        }
        m
    } else {
        0
    };
    let result = if modulus == 0 { num } else { num % modulus };
    unsafe { qjs::wzqjs_new_uint32(ctx, result) }
}

unsafe extern "C" fn script_log(
    ctx: *mut qjs::JSContext,
    _this: qjs::JSValue,
    argc: i32,
    argv: *const qjs::JSValue,
) -> qjs::JSValue {
    if argc != 1 {
        return unsafe { throw(ctx, "log() must have exactly one parameter") };
    }
    let arg = unsafe { *argv };
    let msg = jsvalue_to_string(ctx, arg);
    log::info!("game.js: \"{msg}\"");
    unsafe { qjs::wzqjs_undefined() }
}

// ---------- setMapData ----------

unsafe extern "C" fn set_map_data(
    ctx: *mut qjs::JSContext,
    _this: qjs::JSValue,
    argc: i32,
    argv: *const qjs::JSValue,
) -> qjs::JSValue {
    if argc != 7 {
        return unsafe { throw(ctx, "setMapData: must have 7 parameters") };
    }
    let args: &[qjs::JSValue] = unsafe { core::slice::from_raw_parts(argv, 7) };
    let v_w = args[0];
    let v_h = args[1];
    let v_tex = args[2];
    let v_height = args[3];
    let v_structs = args[4];
    let v_droids = args[5];
    let v_features = args[6];

    if !js_is_number(v_w) || !js_is_number(v_h) {
        return unsafe { throw(ctx, "setMapData: width/height must be numbers") };
    }
    for (v, name) in [
        (v_tex, "texture"),
        (v_height, "height"),
        (v_structs, "structures"),
        (v_droids, "droids"),
        (v_features, "features"),
    ] {
        if !js_is_array(ctx, v) {
            return unsafe { throw(ctx, &format!("setMapData: {name} must be array")) };
        }
    }
    let width = js_to_i32(ctx, v_w);
    let height = js_to_i32(ctx, v_h);
    if width <= 1 || height <= 1 {
        return unsafe { throw(ctx, "setMapData: width/height must be > 1") };
    }
    let area = u64::from(width as u32) * u64::from(height as u32);
    if width as u32 > MAP_MAXWIDTH || height as u32 > MAP_MAXHEIGHT || area > MAP_MAXAREA {
        return unsafe { throw(ctx, "setMapData: map size out of bounds") };
    }

    let w = width as u32;
    let h = height as u32;
    let total = (w as usize) * (h as usize);

    // Validate tile-array lengths up front before doing any work.
    let Some(tex_len) = js_array_length(ctx, v_tex) else {
        return unsafe { throw(ctx, "setMapData: texture array length unreadable") };
    };
    let Some(hgt_len) = js_array_length(ctx, v_height) else {
        return unsafe { throw(ctx, "setMapData: height array length unreadable") };
    };
    if tex_len != area || hgt_len != area {
        return unsafe { throw(ctx, "setMapData: texture/height length != width*height") };
    }

    let mut map_data = MapData::new(w, h);
    for n in 0..total {
        let idx = n as u32;
        let tv = unsafe { qjs::JS_GetPropertyUint32(ctx, v_tex, idx) };
        let hv = unsafe { qjs::JS_GetPropertyUint32(ctx, v_height, idx) };
        let tex = js_to_u32(ctx, tv);
        let mut height_v = js_to_u32(ctx, hv);
        unsafe {
            qjs::wzqjs_free_value(ctx, tv);
            qjs::wzqjs_free_value(ctx, hv);
        }
        if tex > u32::from(u16::MAX) {
            return unsafe { throw(ctx, "setMapData: texture exceeds u16::MAX") };
        }
        if height_v > TILE_MAX_HEIGHT {
            log::warn!(
                "game.js: tile height {height_v} exceeds TILE_MAX_HEIGHT ({TILE_MAX_HEIGHT}); capping"
            );
            height_v = TILE_MAX_HEIGHT;
        }
        map_data.tiles[n] = MapTile {
            texture: tex as u16,
            height: height_v as u16,
        };
    }

    // Structures.
    let Some(structs_len) = js_array_length(ctx, v_structs) else {
        return unsafe { throw(ctx, "setMapData: structures length unreadable") };
    };
    if structs_len > u64::from(u16::MAX) {
        return unsafe { throw(ctx, "setMapData: too many structures") };
    }
    let mut structures = Vec::<Structure>::with_capacity(structs_len as usize);
    for i in 0..structs_len as u32 {
        let s = unsafe { qjs::JS_GetPropertyUint32(ctx, v_structs, i) };
        let res = (|| {
            let name = obj_get_str(ctx, s, c"name");
            let position = obj_get_position(ctx, s).unwrap_or_default();
            let direction = obj_get_i32(ctx, s, c"direction");
            if !(0..=i32::from(i16::MAX) * 2 + 1).contains(&direction) {
                return Err("structure direction out of u16 range".to_owned());
            }
            let modules = obj_get_u32(ctx, s, c"modules");
            if modules >= u32::from(u8::MAX) {
                return Err("structure modules >= u8::MAX".to_owned());
            }
            let player = obj_get_i32(ctx, s, c"player");
            if !(-1..MAX_PLAYERS).contains(&player) {
                return Err(format!("structure player {player} out of range"));
            }
            Ok(Structure {
                name,
                position,
                direction: direction as u16,
                player: player as i8,
                modules: modules as u8,
                id: None,
            })
        })();
        unsafe { qjs::wzqjs_free_value(ctx, s) };
        match res {
            Ok(s) => structures.push(s),
            Err(e) => return unsafe { throw(ctx, &format!("setMapData: {e}")) },
        }
    }

    // Droids.
    let Some(droids_len) = js_array_length(ctx, v_droids) else {
        return unsafe { throw(ctx, "setMapData: droids length unreadable") };
    };
    if droids_len > u64::from(u16::MAX) {
        return unsafe { throw(ctx, "setMapData: too many droids") };
    }
    let mut droids = Vec::<Droid>::with_capacity(droids_len as usize);
    for i in 0..droids_len as u32 {
        let d = unsafe { qjs::JS_GetPropertyUint32(ctx, v_droids, i) };
        let res = (|| {
            let name = obj_get_str(ctx, d, c"name");
            let position = obj_get_position(ctx, d).unwrap_or_default();
            let direction = obj_get_i32(ctx, d, c"direction");
            if !(0..=i32::from(i16::MAX) * 2 + 1).contains(&direction) {
                return Err("droid direction out of u16 range".to_owned());
            }
            let player = obj_get_i32(ctx, d, c"player");
            if !(-1..MAX_PLAYERS).contains(&player) {
                return Err(format!("droid player {player} out of range"));
            }
            Ok(Droid {
                name,
                position,
                direction: direction as u16,
                player: player as i8,
                id: None,
            })
        })();
        unsafe { qjs::wzqjs_free_value(ctx, d) };
        match res {
            Ok(d) => droids.push(d),
            Err(e) => return unsafe { throw(ctx, &format!("setMapData: {e}")) },
        }
    }

    // Features.
    let Some(feats_len) = js_array_length(ctx, v_features) else {
        return unsafe { throw(ctx, "setMapData: features length unreadable") };
    };
    if feats_len > u64::from(u16::MAX) {
        return unsafe { throw(ctx, "setMapData: too many features") };
    }
    let mut features = Vec::<Feature>::with_capacity(feats_len as usize);
    for i in 0..feats_len as u32 {
        let f = unsafe { qjs::JS_GetPropertyUint32(ctx, v_features, i) };
        let res = (|| {
            let name = obj_get_str(ctx, f, c"name");
            let position = obj_get_position(ctx, f).unwrap_or_default();
            let direction = obj_get_i32(ctx, f, c"direction");
            if !(0..=i32::from(i16::MAX) * 2 + 1).contains(&direction) {
                return Err("feature direction out of u16 range".to_owned());
            }
            Ok(Feature {
                name,
                position,
                direction: direction as u16,
                id: None,
                player: None,
            })
        })();
        unsafe { qjs::wzqjs_free_value(ctx, f) };
        match res {
            Ok(f) => features.push(f),
            Err(e) => return unsafe { throw(ctx, &format!("setMapData: {e}")) },
        }
    }

    let Some(state) = (unsafe { state(ctx) }) else {
        return unsafe { throw(ctx, "setMapData: context state missing") };
    };
    state.map.map_data = map_data;
    state.map.structures = structures;
    state.map.droids = droids;
    state.map.features = features;
    state.set_map_data_called = true;
    unsafe { qjs::wzqjs_true() }
}

// ---------- generateFractalValueNoise ----------

#[derive(Clone, Copy)]
struct RiggedRegion {
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    callback: qjs::JSValue,
}

unsafe extern "C" fn generate_fractal_value_noise(
    ctx: *mut qjs::JSContext,
    this_val: qjs::JSValue,
    argc: i32,
    argv: *const qjs::JSValue,
) -> qjs::JSValue {
    if !(5..=8).contains(&argc) {
        return unsafe {
            throw(
                ctx,
                "generateFractalValueNoise: must have 5 to 8 parameters",
            )
        };
    }
    let args: &[qjs::JSValue] = unsafe { core::slice::from_raw_parts(argv, argc as usize) };
    for (i, label) in [
        (0, "width"),
        (1, "height"),
        (2, "range"),
        (3, "crispness"),
        (4, "scale"),
    ] {
        if !js_is_number(args[i]) {
            return unsafe {
                throw(
                    ctx,
                    &format!("generateFractalValueNoise: {label} must be number"),
                )
            };
        }
    }
    let width = js_to_u32(ctx, args[0]);
    let height = js_to_u32(ctx, args[1]);
    let range = js_to_u32(ctx, args[2]);
    let crispness = js_to_u32(ctx, args[3]);
    let scale = js_to_u32(ctx, args[4]);
    if width == 0 || height == 0 || range == 0 || crispness == 0 || scale == 0 {
        return unsafe { throw(ctx, "generateFractalValueNoise: numeric args must be > 0") };
    }
    if width > 256 || height > 256 {
        return unsafe {
            throw(
                ctx,
                "generateFractalValueNoise: width/height must be <= 256",
            )
        };
    }

    let normalize_to_range: u32 = if argc >= 6 {
        if !js_is_number(args[5]) {
            return unsafe { throw(ctx, "normalizeToRange must be number") };
        }
        let mut v: i64 = 0;
        if unsafe { qjs::JS_ToInt64(ctx, &raw mut v, args[5]) } != 0
            || !(0..=i64::from(u32::MAX)).contains(&v)
        {
            return unsafe { throw(ctx, "normalizeToRange must be in [0, u32::MAX]") };
        }
        v as u32
    } else {
        0
    };

    let mut rigged_regions: Vec<RiggedRegion> = Vec::new();
    if argc >= 7 {
        if !js_is_array(ctx, args[6]) {
            return unsafe { throw(ctx, "riggedRegions must be array") };
        }
        let Some(len) = js_array_length(ctx, args[6]) else {
            return unsafe { throw(ctx, "riggedRegions length unreadable") };
        };
        if len > u64::from(u16::MAX) {
            return unsafe { throw(ctx, "too many riggedRegions") };
        }
        for i in 0..len as u32 {
            let r = unsafe { qjs::JS_GetPropertyUint32(ctx, args[6], i) };
            if !js_is_array(ctx, r) || js_array_length(ctx, r) != Some(5) {
                unsafe { qjs::wzqjs_free_value(ctx, r) };
                return unsafe { throw(ctx, "riggedRegion must be 5-element array") };
            }
            let x1 = unsafe { qjs::JS_GetPropertyUint32(ctx, r, 0) };
            let y1 = unsafe { qjs::JS_GetPropertyUint32(ctx, r, 1) };
            let x2 = unsafe { qjs::JS_GetPropertyUint32(ctx, r, 2) };
            let y2 = unsafe { qjs::JS_GetPropertyUint32(ctx, r, 3) };
            let cb = unsafe { qjs::JS_GetPropertyUint32(ctx, r, 4) };
            let ok = js_is_number(x1)
                && js_is_number(y1)
                && js_is_number(x2)
                && js_is_number(y2)
                && js_is_function(ctx, cb);
            if !ok {
                unsafe {
                    qjs::wzqjs_free_value(ctx, x1);
                    qjs::wzqjs_free_value(ctx, y1);
                    qjs::wzqjs_free_value(ctx, x2);
                    qjs::wzqjs_free_value(ctx, y2);
                    qjs::wzqjs_free_value(ctx, cb);
                    qjs::wzqjs_free_value(ctx, r);
                }
                return unsafe { throw(ctx, "riggedRegion has wrong types") };
            }
            rigged_regions.push(RiggedRegion {
                x1: js_to_u32(ctx, x1),
                y1: js_to_u32(ctx, y1),
                x2: js_to_u32(ctx, x2),
                y2: js_to_u32(ctx, y2),
                callback: unsafe { qjs::wzqjs_dup_value(ctx, cb) },
            });
            unsafe {
                qjs::wzqjs_free_value(ctx, x1);
                qjs::wzqjs_free_value(ctx, y1);
                qjs::wzqjs_free_value(ctx, x2);
                qjs::wzqjs_free_value(ctx, y2);
                qjs::wzqjs_free_value(ctx, cb);
                qjs::wzqjs_free_value(ctx, r);
            }
        }
    }

    let row_major: bool = if argc >= 8 {
        let tag = unsafe { qjs::wzqjs_value_get_tag(args[7]) };
        if tag != qjs::JS_TAG_BOOL {
            // Free any region callbacks before throwing.
            for r in &rigged_regions {
                unsafe { qjs::wzqjs_free_value(ctx, r.callback) };
            }
            return unsafe { throw(ctx, "rowMajorOrder must be bool") };
        }
        unsafe { qjs::JS_ToBool(ctx, args[7]) != 0 }
    } else {
        false
    };

    let size = (width as usize) * (height as usize);
    if size > usize::from(u16::MAX) {
        for r in &rigged_regions {
            unsafe { qjs::wzqjs_free_value(ctx, r.callback) };
        }
        return unsafe { throw(ctx, "requested data too large") };
    }

    let result = generate_noise(
        ctx,
        this_val,
        width,
        height,
        range,
        crispness,
        scale,
        normalize_to_range,
        &rigged_regions,
        row_major,
    );

    for r in &rigged_regions {
        unsafe { qjs::wzqjs_free_value(ctx, r.callback) };
    }

    match result {
        Ok(noise) => build_noise_array(ctx, &noise),
        Err(msg) => unsafe { throw(ctx, &msg) },
    }
}

/// Core port of `runMap_generateFractalValueNoise`'s noise-generation loop.
#[expect(
    clippy::too_many_arguments,
    reason = "Mirror of the C function's parameter list."
)]
fn generate_noise(
    ctx: *mut qjs::JSContext,
    this_val: qjs::JSValue,
    width: u32,
    height: u32,
    range: u32,
    crispness: u32,
    scale: u32,
    normalize_to_range: u32,
    rigged_regions: &[RiggedRegion],
    row_major: bool,
) -> Result<Vec<u32>, String> {
    let size = (width as usize) * (height as usize);
    let max_layer_size = ((width + 1) as usize)
        .checked_mul((height + 1) as usize)
        .ok_or_else(|| "integer overflow".to_owned())?;
    let mut noise_data = vec![0u32; size];
    let mut layer_data = vec![0u32; max_layer_size];

    let Some(state_for_rng): Option<&mut ScriptState> = (unsafe {
        let opaque = qjs::JS_GetContextOpaque(ctx);
        if opaque.is_null() {
            None
        } else {
            Some(&mut *opaque.cast::<ScriptState>())
        }
    }) else {
        return Err("context state missing".to_owned());
    };

    let mut layer_scale = scale;
    let mut layer_range = range;
    let mut layer_idx: u32 = u32::MAX; // becomes 0 on first ++.

    loop {
        layer_scale /= 2;
        layer_range = layer_range * crispness / 10;
        if layer_range == 0 {
            break;
        }
        layer_idx = layer_idx.wrapping_add(1);

        let layer_width = width / layer_scale.max(1) + 1;
        let layer_height = height / layer_scale.max(1) + 1;
        let layer_scale_area = layer_scale * layer_scale;

        for x in 0..layer_width {
            for y in 0..layer_height {
                let map_x = x * layer_scale;
                let map_y = y * layer_scale;

                let mut rigged = false;
                for r in rigged_regions {
                    if map_x >= r.x1 && map_y >= r.y1 && map_x <= r.x2 && map_y <= r.y2 {
                        rigged = true;
                        let call_argv = unsafe {
                            [
                                qjs::wzqjs_new_uint32(ctx, map_x),
                                qjs::wzqjs_new_uint32(ctx, map_y),
                                qjs::wzqjs_new_uint32(ctx, layer_idx),
                                qjs::wzqjs_new_uint32(ctx, layer_range),
                            ]
                        };
                        let ret = unsafe {
                            qjs::JS_Call(ctx, r.callback, this_val, 4, call_argv.as_ptr())
                        };
                        for v in call_argv {
                            unsafe { qjs::wzqjs_free_value(ctx, v) };
                        }
                        if unsafe { qjs::wzqjs_is_exception(ret) } != 0 {
                            unsafe { qjs::wzqjs_free_value(ctx, ret) };
                            return Err("riggedRegion callback threw".to_owned());
                        }
                        if !js_is_number(ret) {
                            unsafe { qjs::wzqjs_free_value(ctx, ret) };
                            return Err("riggedRegion callback must return number".to_owned());
                        }
                        let v = js_to_u32(ctx, ret);
                        unsafe { qjs::wzqjs_free_value(ctx, ret) };
                        if v >= layer_range {
                            return Err(
                                "riggedRegion callback returned out-of-range value".to_owned()
                            );
                        }
                        layer_data[(x * layer_height + y) as usize] = v;
                    }
                }

                if !rigged {
                    let num = state_for_rng.rng.next_u32() % layer_range;
                    layer_data[(x * layer_height + y) as usize] = num;
                }
            }
        }

        if layer_scale == 0 {
            break;
        }

        for x in 0..layer_width {
            let map_x = x * layer_scale;
            if map_x >= width {
                continue;
            }
            for y in 0..layer_height {
                let map_y = y * layer_scale;
                if map_y >= height {
                    continue;
                }
                let tl = layer_data[(x * layer_height + y) as usize];
                let tr = layer_data[((x + 1) * layer_height + y) as usize];
                let bl = layer_data[(x * layer_height + (y + 1)) as usize];
                let br = layer_data[((x + 1) * layer_height + (y + 1)) as usize];

                let inner_x_end = layer_scale.min(width - map_x);
                let inner_y_end = layer_scale.min(height - map_y);
                for inner_x in 0..inner_x_end {
                    for inner_y in 0..inner_y_end {
                        let sum = br * inner_x * inner_y
                            + bl * (layer_scale - 1 - inner_x) * inner_y
                            + tr * inner_x * (layer_scale - 1 - inner_y)
                            + tl * (layer_scale - 1 - inner_x) * (layer_scale - 1 - inner_y);
                        let idx = ((map_x + inner_x) * height + (map_y + inner_y)) as usize;
                        noise_data[idx] += sum / layer_scale_area;
                    }
                }
            }
        }

        if layer_scale <= 1 || layer_range <= 1 {
            break;
        }
    }

    if normalize_to_range > 0 && !noise_data.is_empty() {
        let min = *noise_data.iter().min().unwrap();
        let max = *noise_data.iter().max().unwrap();
        match core::num::NonZero::new(max - min) {
            Some(divisor) => {
                for v in &mut noise_data {
                    *v = (*v - min) * normalize_to_range / divisor.get();
                }
            }
            None => noise_data.fill(0),
        }
    }

    if row_major {
        let mut row = vec![0u32; size];
        for x in 0..width {
            for y in 0..height {
                row[(y * width + x) as usize] = noise_data[(x * height + y) as usize];
            }
        }
        noise_data = row;
    }

    Ok(noise_data)
}

fn build_noise_array(ctx: *mut qjs::JSContext, values: &[u32]) -> qjs::JSValue {
    let arr = unsafe { qjs::JS_NewArray(ctx) };
    for (i, &v) in values.iter().enumerate() {
        let elem = unsafe { qjs::wzqjs_new_uint32(ctx, v) };
        unsafe { qjs::JS_SetPropertyUint32(ctx, arr, i as u32, elem) };
    }
    arr
}

// ---------- Registration / globals ----------

/// Register the four host functions on the global object.
pub(super) fn register_all(
    ctx: *mut qjs::JSContext,
    global: qjs::JSValue,
) -> Result<(), ScriptError> {
    unsafe {
        register(ctx, global, c"gameRand", 0, game_rand)?;
        register(ctx, global, c"log", 1, script_log)?;
        register(ctx, global, c"setMapData", 7, set_map_data)?;
        register(
            ctx,
            global,
            c"generateFractalValueNoise",
            6,
            generate_fractal_value_noise,
        )?;
    }
    Ok(())
}

unsafe fn register(
    ctx: *mut qjs::JSContext,
    global: qjs::JSValue,
    name: &CStr,
    arity: i32,
    func: qjs::JSCFunction,
) -> Result<(), ScriptError> {
    let f = unsafe { qjs::wzqjs_new_cfunction(ctx, func, name.as_ptr(), arity) };
    if unsafe { qjs::wzqjs_is_exception(f) } != 0 {
        return Err(ScriptError::Other(format!(
            "failed to create JS function {name:?}"
        )));
    }
    unsafe {
        qjs::JS_DefinePropertyValueStr(ctx, global, name.as_ptr(), f, qjs::JS_PROP_DEFAULT);
    }
    Ok(())
}

/// Define the six numeric globals Warzone exposes (`preview`, `XFLIP`,
/// `YFLIP`, `ROTMASK`, `ROTSHIFT`, `TRIFLIP`).
pub(super) fn define_globals(ctx: *mut qjs::JSContext, global: qjs::JSValue) {
    use crate::constants::{TILE_ROTMASK, TILE_ROTSHIFT, TILE_TRIFLIP, TILE_XFLIP, TILE_YFLIP};
    unsafe {
        define(ctx, global, c"preview", qjs::wzqjs_new_bool(ctx, 0));
        define(
            ctx,
            global,
            c"XFLIP",
            qjs::wzqjs_new_int32(ctx, i32::from(TILE_XFLIP)),
        );
        define(
            ctx,
            global,
            c"YFLIP",
            qjs::wzqjs_new_int32(ctx, i32::from(TILE_YFLIP)),
        );
        define(
            ctx,
            global,
            c"ROTMASK",
            qjs::wzqjs_new_int32(ctx, i32::from(TILE_ROTMASK)),
        );
        define(
            ctx,
            global,
            c"ROTSHIFT",
            qjs::wzqjs_new_int32(ctx, i32::from(TILE_ROTSHIFT)),
        );
        define(
            ctx,
            global,
            c"TRIFLIP",
            qjs::wzqjs_new_int32(ctx, i32::from(TILE_TRIFLIP)),
        );
    }
}

unsafe fn define(ctx: *mut qjs::JSContext, global: qjs::JSValue, name: &CStr, value: qjs::JSValue) {
    unsafe {
        qjs::JS_DefinePropertyValueStr(ctx, global, name.as_ptr(), value, qjs::JS_PROP_DEFAULT);
    }
}
