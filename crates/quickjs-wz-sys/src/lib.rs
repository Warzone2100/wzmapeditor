//! Static-link target and raw FFI bindings for Warzone 2100's `quickjs-wz`.
//!
//! Hand-rolled bindings for the small subset of `QuickJS` used by the
//! script-map runtime: runtime + limited-context lifecycle, value
//! creation and reading, evaluation, function registration, and a set
//! of `wzqjs_*` shims for the macros and `static inline` helpers (see
//! `src/wrapper.c`) that have no link-time symbol.
//!
//! All bindings target the 64-bit non-NaN-boxing `JSValue` layout
//! defined in `quickjs.h`. The crate does not support 32-bit targets.
//!
//! # Safety
//!
//! Every binding in this crate is `unsafe` to call. Callers must
//! uphold `QuickJS`'s reference-counting rules: each function returning
//! a `JSValue` returns an owned reference that must be released exactly
//! once via [`wzqjs_free_value`].
#![no_std]
#![expect(
    non_snake_case,
    reason = "Function names mirror QuickJS's C API verbatim."
)]

use core::ffi::{c_char, c_int, c_void};
use core::fmt;

/// Opaque handle to a `QuickJS` runtime.
#[repr(C)]
#[derive(Debug)]
pub struct JSRuntime {
    _private: [u8; 0],
}

/// Opaque handle to a `QuickJS` context.
#[repr(C)]
#[derive(Debug)]
pub struct JSContext {
    _private: [u8; 0],
}

/// Tagged-value payload from `quickjs.h` (`JSValueUnion`).
#[repr(C)]
#[derive(Copy, Clone)]
pub union JSValueUnion {
    pub int32: i32,
    pub float64: f64,
    pub ptr: *mut c_void,
    pub short_big_int: i64,
}

impl fmt::Debug for JSValueUnion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JSValueUnion").finish_non_exhaustive()
    }
}

/// `QuickJS` tagged value (64-bit non-NaN-boxing layout).
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct JSValue {
    pub u: JSValueUnion,
    pub tag: i64,
}

/// Mirror of WZ's `JSLimitedContextOptions` (see
/// `quickjs-wz-extensions/quickjs-limitedcontext.h`).
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct JSLimitedContextOptions {
    pub baseObjects: c_int,
    pub dateObject: c_int,
    pub eval: c_int,
    pub stringNormalize: c_int,
    pub regExp: c_int,
    pub json: c_int,
    pub proxy: c_int,
    pub mapSet: c_int,
    pub typedArrays: c_int,
    pub promise: c_int,
    pub bigInt: c_int,
    pub weakRef: c_int,
}

pub const JS_TAG_INT: i32 = 0;
pub const JS_TAG_BOOL: i32 = 1;
pub const JS_TAG_NULL: i32 = 2;
pub const JS_TAG_UNDEFINED: i32 = 3;
pub const JS_TAG_EXCEPTION: i32 = 6;
pub const JS_TAG_FLOAT64: i32 = 7;
pub const JS_TAG_OBJECT: i32 = -1;
pub const JS_TAG_STRING: i32 = -7;

pub const JS_EVAL_TYPE_GLOBAL: c_int = 0;
pub const JS_EVAL_TYPE_MODULE: c_int = 1;
pub const JS_EVAL_FLAG_COMPILE_ONLY: c_int = 1 << 5;

/// Default flags for `JS_DefinePropertyValue*` — writable + enumerable + configurable
/// is `0x07` per `quickjs.h`. We pass `0` to match Warzone's existing usage in
/// `map_script.cpp` (non-enumerable, non-writable, non-configurable).
pub const JS_PROP_DEFAULT: c_int = 0;

/// Signature of a `JSCFunction` host function exposed to `QuickJS`. Matches
/// `typedef JSValue JSCFunction(JSContext *ctx, JSValueConst this_val, int argc,
/// JSValueConst *argv);` in `quickjs.h`.
pub type JSCFunction = unsafe extern "C" fn(
    ctx: *mut JSContext,
    this_val: JSValue,
    argc: c_int,
    argv: *const JSValue,
) -> JSValue;

/// Signature of a `JS_SetInterruptHandler` callback. Returns non-zero to
/// abort execution.
pub type JSInterruptHandler =
    unsafe extern "C" fn(rt: *mut JSRuntime, opaque: *mut c_void) -> c_int;

unsafe extern "C" {
    /* Runtime lifecycle. */
    pub fn JS_NewRuntime() -> *mut JSRuntime;
    pub fn JS_FreeRuntime(rt: *mut JSRuntime);
    pub fn JS_SetMemoryLimit(rt: *mut JSRuntime, limit: usize);
    pub fn JS_SetMaxStackSize(rt: *mut JSRuntime, stack_size: usize);
    pub fn JS_SetInterruptHandler(
        rt: *mut JSRuntime,
        cb: Option<JSInterruptHandler>,
        opaque: *mut c_void,
    );

    /* Context lifecycle (limited context, eval). */
    pub fn JS_NewLimitedContext(
        rt: *mut JSRuntime,
        options: *const JSLimitedContextOptions,
    ) -> *mut JSContext;
    pub fn JS_FreeContext(ctx: *mut JSContext);
    pub fn JS_SetContextOpaque(ctx: *mut JSContext, opaque: *mut c_void);
    pub fn JS_GetContextOpaque(ctx: *mut JSContext) -> *mut c_void;
    pub fn JS_GetGlobalObject(ctx: *mut JSContext) -> JSValue;

    pub fn JS_Eval(
        ctx: *mut JSContext,
        input: *const c_char,
        input_len: usize,
        filename: *const c_char,
        eval_flags: c_int,
    ) -> JSValue;
    /// `JS_Eval` that works even when the limited context has `eval` disabled.
    pub fn JS_Eval_BypassLimitedContext(
        ctx: *mut JSContext,
        input: *const c_char,
        input_len: usize,
        filename: *const c_char,
        eval_flags: c_int,
    ) -> JSValue;
    pub fn JS_EvalFunction(ctx: *mut JSContext, fun_obj: JSValue) -> JSValue;

    /* Errors. */
    pub fn JS_GetException(ctx: *mut JSContext) -> JSValue;
    pub fn JS_IsError(ctx: *mut JSContext, val: JSValue) -> c_int;
    pub fn JS_Throw(ctx: *mut JSContext, obj: JSValue) -> JSValue;
    pub fn JS_ThrowReferenceError(ctx: *mut JSContext, fmt: *const c_char, ...) -> JSValue;
    pub fn JS_ThrowInternalError(ctx: *mut JSContext, fmt: *const c_char, ...) -> JSValue;
    pub fn JS_ThrowTypeError(ctx: *mut JSContext, fmt: *const c_char, ...) -> JSValue;

    /* Value coercions. */
    pub fn JS_ToInt32(ctx: *mut JSContext, pres: *mut i32, val: JSValue) -> c_int;
    pub fn JS_ToInt64(ctx: *mut JSContext, pres: *mut i64, val: JSValue) -> c_int;
    pub fn JS_ToFloat64(ctx: *mut JSContext, pres: *mut f64, val: JSValue) -> c_int;
    pub fn JS_ToBool(ctx: *mut JSContext, val: JSValue) -> c_int;
    pub fn JS_ToIndex(ctx: *mut JSContext, plen: *mut u64, val: JSValue) -> c_int;
    pub fn JS_FreeCString(ctx: *mut JSContext, ptr: *const c_char);

    /* Strings. */
    pub fn JS_NewStringLen(ctx: *mut JSContext, str_: *const c_char, len: usize) -> JSValue;

    /* Arrays + property access. */
    pub fn JS_NewArray(ctx: *mut JSContext) -> JSValue;
    pub fn JS_IsArray(ctx: *mut JSContext, val: JSValue) -> c_int;
    pub fn JS_IsFunction(ctx: *mut JSContext, val: JSValue) -> c_int;
    pub fn JS_GetPropertyStr(
        ctx: *mut JSContext,
        this_obj: JSValue,
        prop: *const c_char,
    ) -> JSValue;
    pub fn JS_GetPropertyUint32(ctx: *mut JSContext, this_obj: JSValue, idx: u32) -> JSValue;
    pub fn JS_SetPropertyStr(
        ctx: *mut JSContext,
        this_obj: JSValue,
        prop: *const c_char,
        val: JSValue,
    ) -> c_int;
    pub fn JS_SetPropertyUint32(
        ctx: *mut JSContext,
        this_obj: JSValue,
        idx: u32,
        val: JSValue,
    ) -> c_int;
    pub fn JS_DefinePropertyValueStr(
        ctx: *mut JSContext,
        this_obj: JSValue,
        prop: *const c_char,
        val: JSValue,
        flags: c_int,
    ) -> c_int;

    /* Function calls. */
    pub fn JS_Call(
        ctx: *mut JSContext,
        func_obj: JSValue,
        this_obj: JSValue,
        argc: c_int,
        argv: *const JSValue,
    ) -> JSValue;
    pub fn JS_NewCFunction2(
        ctx: *mut JSContext,
        func: JSCFunction,
        name: *const c_char,
        length: c_int,
        cproto: c_int,
        magic: c_int,
    ) -> JSValue;

    /* `wzqjs_*` shims defined in src/wrapper.c. */
    pub fn wzqjs_free_value(ctx: *mut JSContext, v: JSValue);
    pub fn wzqjs_dup_value(ctx: *mut JSContext, v: JSValue) -> JSValue;
    pub fn wzqjs_value_get_tag(v: JSValue) -> c_int;
    pub fn wzqjs_value_get_int(v: JSValue) -> c_int;

    pub fn wzqjs_is_exception(v: JSValue) -> c_int;
    pub fn wzqjs_is_undefined(v: JSValue) -> c_int;
    pub fn wzqjs_is_null(v: JSValue) -> c_int;
    pub fn wzqjs_is_number(v: JSValue) -> c_int;
    pub fn wzqjs_is_bool(v: JSValue) -> c_int;
    pub fn wzqjs_is_string(v: JSValue) -> c_int;
    pub fn wzqjs_is_object(v: JSValue) -> c_int;

    pub fn wzqjs_undefined() -> JSValue;
    pub fn wzqjs_null() -> JSValue;
    pub fn wzqjs_true() -> JSValue;
    pub fn wzqjs_false() -> JSValue;
    pub fn wzqjs_exception() -> JSValue;

    pub fn wzqjs_new_bool(ctx: *mut JSContext, v: c_int) -> JSValue;
    pub fn wzqjs_new_int32(ctx: *mut JSContext, v: i32) -> JSValue;
    pub fn wzqjs_new_uint32(ctx: *mut JSContext, v: u32) -> JSValue;
    pub fn wzqjs_new_float64(ctx: *mut JSContext, v: f64) -> JSValue;

    pub fn wzqjs_new_string_len(ctx: *mut JSContext, str_: *const c_char, len: usize) -> JSValue;
    pub fn wzqjs_to_cstring_len(ctx: *mut JSContext, plen: *mut usize, v: JSValue)
    -> *const c_char;

    pub fn wzqjs_to_uint32(ctx: *mut JSContext, pres: *mut u32, v: JSValue) -> c_int;

    pub fn wzqjs_new_cfunction(
        ctx: *mut JSContext,
        func: JSCFunction,
        name: *const c_char,
        length: c_int,
    ) -> JSValue;
}
