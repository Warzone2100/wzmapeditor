//! Static-link target and raw FFI bindings for Warzone 2100's `quickjs-wz`.
//!
//! Hand-rolled bindings for the small subset of `QuickJS` used by the
//! script-map runtime: runtime + limited-context lifecycle, value
//! creation and reading, evaluation, and a handful of shims for the
//! macros and `static inline` helpers (see `src/wrapper.c`) that have no
//! link-time symbol.
//!
//! All bindings target the 64-bit non-NaN-boxing `JSValue` layout
//! defined in `quickjs.h`. The crate does not support 32-bit targets.
//!
//! # Safety
//!
//! Every binding in this crate is `unsafe` to call. Callers must
//! uphold `QuickJS`'s reference-counting rules: each function returning a
//! `JSValue` returns an owned reference that must be released exactly
//! once via [`wzqjs_free_value`].
#![no_std]
#![expect(
    non_snake_case,
    reason = "Function names mirror `QuickJS`'s C API verbatim."
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

unsafe extern "C" {
    pub fn JS_NewRuntime() -> *mut JSRuntime;
    pub fn JS_FreeRuntime(rt: *mut JSRuntime);
    pub fn JS_SetMemoryLimit(rt: *mut JSRuntime, limit: usize);
    pub fn JS_SetMaxStackSize(rt: *mut JSRuntime, stack_size: usize);

    pub fn JS_NewLimitedContext(
        rt: *mut JSRuntime,
        options: *const JSLimitedContextOptions,
    ) -> *mut JSContext;
    pub fn JS_FreeContext(ctx: *mut JSContext);

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

    pub fn JS_ToInt32(ctx: *mut JSContext, pres: *mut i32, val: JSValue) -> c_int;

    pub fn wzqjs_free_value(ctx: *mut JSContext, v: JSValue);
    pub fn wzqjs_is_exception(v: JSValue) -> c_int;
    pub fn wzqjs_value_get_tag(v: JSValue) -> c_int;
    pub fn wzqjs_value_get_int(v: JSValue) -> c_int;
    pub fn wzqjs_undefined() -> JSValue;
}
