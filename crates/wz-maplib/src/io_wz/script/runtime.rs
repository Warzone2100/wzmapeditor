//! `QuickJS` runtime setup for script-map execution.

use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::ptr;
use std::time::Instant;

use quickjs_wz_sys as qjs;
use rand_mt::Mt;

use crate::io_wz::WzMap;

use super::host_fns;

/// Errors returned by [`run_script_source`].
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("failed to create QuickJS runtime")]
    RuntimeAlloc,
    #[error("failed to create QuickJS limited context")]
    ContextAlloc,
    #[error(
        "script aborted: exceeded {} second runtime budget",
        MAX_RUNTIME_SECONDS
    )]
    Timeout,
    #[error("script syntax / compile error: {0}")]
    Compile(String),
    #[error("uncaught script exception: {0}")]
    Runtime(String),
    #[error("script archive missing required entry: {0}")]
    MissingEntry(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("script error: {0}")]
    Other(String),
}

/// Wall-clock budget for the script run. Matches WZ's
/// `MAX_MAPSCRIPT_RUNTIME_SECONDS` in `map_script.cpp`.
pub const MAX_RUNTIME_SECONDS: u64 = 30;

/// Shared state visible to every host function via `JS_GetContextOpaque`.
pub(super) struct ScriptState {
    pub(super) map: WzMap,
    pub(super) rng: Mt,
    pub(super) set_map_data_called: bool,
}

impl ScriptState {
    fn new(name: &str, seed: u32) -> Self {
        Self {
            map: WzMap::new(name, 0, 0),
            rng: Mt::new(seed),
            set_map_data_called: false,
        }
    }
}

unsafe extern "C" fn interrupt_handler(_rt: *mut qjs::JSRuntime, opaque: *mut c_void) -> i32 {
    // SAFETY: `opaque` was set in `run_script_source` to point at a stack
    // `Instant` whose lifetime exceeds the duration of `JS_EvalFunction`.
    let start = unsafe { &*opaque.cast::<Instant>() };
    i32::from(start.elapsed().as_secs() >= MAX_RUNTIME_SECONDS)
}

/// Run `source` as a Warzone 2100 script map.
///
/// `name` is used both as the resulting map name and as the `QuickJS`
/// `filename` for error messages.
pub fn run_script_source(name: &str, source: &str, seed: u32) -> Result<WzMap, ScriptError> {
    let filename = CString::new(name).map_err(|e| ScriptError::Other(e.to_string()))?;

    let mut state = Box::new(ScriptState::new(name, seed));

    let result = unsafe { run_inner(&filename, source, state.as_mut()) };

    match result {
        Ok(()) => {
            if !state.set_map_data_called {
                return Err(ScriptError::Other(
                    "script finished without calling setMapData".to_owned(),
                ));
            }
            Ok(state.map)
        }
        Err(e) => Err(e),
    }
}

/// # Safety
///
/// All callers must keep `state` alive for the duration of this function
/// (it's referenced by `JS_SetContextOpaque`).
unsafe fn run_inner(
    filename: &CStr,
    source: &str,
    state: &mut ScriptState,
) -> Result<(), ScriptError> {
    let rt = unsafe { qjs::JS_NewRuntime() };
    if rt.is_null() {
        return Err(ScriptError::RuntimeAlloc);
    }
    let runtime_guard = RuntimeGuard { rt };

    let opts = qjs::JSLimitedContextOptions {
        baseObjects: 1,
        mapSet: 1,
        ..qjs::JSLimitedContextOptions::default()
    };
    let ctx = unsafe { qjs::JS_NewLimitedContext(rt, &raw const opts) };
    if ctx.is_null() {
        return Err(ScriptError::ContextAlloc);
    }
    let ctx_guard = ContextGuard { ctx };

    // SAFETY: `state` outlives the context (caller owns the Box; we only
    // dereference it during host-function invocations).
    unsafe { qjs::JS_SetContextOpaque(ctx, ptr::from_mut(state).cast::<c_void>()) };

    let global = unsafe { qjs::JS_GetGlobalObject(ctx) };
    let global_guard = ValueGuard { ctx, val: global };

    host_fns::register_all(ctx, global)?;
    host_fns::define_globals(ctx, global);

    // Match Warzone's runtime caps.
    unsafe {
        qjs::JS_SetMaxStackSize(rt, 512 * 1024);
        qjs::JS_SetMemoryLimit(rt, 100 * 1024 * 1024);
    }

    let start = Instant::now();
    unsafe {
        qjs::JS_SetInterruptHandler(
            rt,
            Some(interrupt_handler),
            ptr::from_ref(&start).cast::<c_void>().cast_mut(),
        );
    }

    let source_c =
        CString::new(source).map_err(|_| ScriptError::Other("source contains NUL".to_owned()))?;
    let source_len = source_c.as_bytes().len();
    let compiled = unsafe {
        qjs::JS_Eval_BypassLimitedContext(
            ctx,
            source_c.as_ptr(),
            source_len,
            filename.as_ptr(),
            qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY,
        )
    };
    if unsafe { qjs::wzqjs_is_exception(compiled) } != 0 {
        let msg = dump_exception(ctx);
        unsafe { qjs::wzqjs_free_value(ctx, compiled) };
        return Err(ScriptError::Compile(msg));
    }

    let result = unsafe { qjs::JS_EvalFunction(ctx, compiled) };
    unsafe { qjs::JS_SetInterruptHandler(rt, None, ptr::null_mut()) };

    let timed_out = start.elapsed().as_secs() >= MAX_RUNTIME_SECONDS;

    if unsafe { qjs::wzqjs_is_exception(result) } != 0 {
        let msg = dump_exception(ctx);
        unsafe { qjs::wzqjs_free_value(ctx, result) };
        if timed_out {
            return Err(ScriptError::Timeout);
        }
        return Err(ScriptError::Runtime(msg));
    }
    unsafe { qjs::wzqjs_free_value(ctx, result) };

    drop(global_guard);
    drop(ctx_guard);
    drop(runtime_guard);

    Ok(())
}

fn dump_exception(ctx: *mut qjs::JSContext) -> String {
    unsafe {
        let exc = qjs::JS_GetException(ctx);
        let msg = jsvalue_to_string(ctx, exc);
        let stack_key = c"stack";
        let stack = qjs::JS_GetPropertyStr(ctx, exc, stack_key.as_ptr());
        let mut full = msg;
        if qjs::wzqjs_is_undefined(stack) == 0 {
            full.push('\n');
            full.push_str(&jsvalue_to_string(ctx, stack));
        }
        qjs::wzqjs_free_value(ctx, stack);
        qjs::wzqjs_free_value(ctx, exc);
        full
    }
}

pub(super) fn jsvalue_to_string(ctx: *mut qjs::JSContext, val: qjs::JSValue) -> String {
    unsafe {
        let mut len: usize = 0;
        let ptr = qjs::wzqjs_to_cstring_len(ctx, &raw mut len, val);
        if ptr.is_null() {
            return String::from("<unprintable>");
        }
        let bytes = core::slice::from_raw_parts(ptr.cast::<u8>(), len);
        let owned = String::from_utf8_lossy(bytes).into_owned();
        qjs::JS_FreeCString(ctx, ptr);
        owned
    }
}

struct RuntimeGuard {
    rt: *mut qjs::JSRuntime,
}
impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        // SAFETY: `rt` came from `JS_NewRuntime` and is freed exactly once.
        unsafe { qjs::JS_FreeRuntime(self.rt) };
    }
}

struct ContextGuard {
    ctx: *mut qjs::JSContext,
}
impl Drop for ContextGuard {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `JS_NewLimitedContext` and is freed once.
        unsafe { qjs::JS_FreeContext(self.ctx) };
    }
}

struct ValueGuard {
    ctx: *mut qjs::JSContext,
    val: qjs::JSValue,
}
impl Drop for ValueGuard {
    fn drop(&mut self) {
        // SAFETY: `val` is an owned reference paired with this guard.
        unsafe { qjs::wzqjs_free_value(self.ctx, self.val) };
    }
}
