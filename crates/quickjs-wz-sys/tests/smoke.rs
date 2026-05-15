//! End-to-end smoke test for the hand-rolled FFI bindings.
//!
//! Creates a `QuickJS` runtime, builds a limited context with only the
//! base intrinsics enabled, evaluates `1 + 2`, and checks the result.
//! This exercises the full FFI surface that the host-function port
//! will rely on: `JSValue` ABI (returned by value), runtime/context
//! lifecycle, the WZ limited-context entry point, and value-reading
//! shims.

use quickjs_wz_sys::{
    JS_EVAL_TYPE_GLOBAL, JS_Eval_BypassLimitedContext, JS_FreeContext, JS_FreeRuntime,
    JS_NewLimitedContext, JS_NewRuntime, JS_SetMaxStackSize, JS_SetMemoryLimit, JS_TAG_INT,
    JS_ToInt32, JSLimitedContextOptions, wzqjs_free_value, wzqjs_is_exception, wzqjs_value_get_tag,
};

#[test]
fn limited_context_evaluates_basic_arithmetic() {
    // SAFETY: Every call below follows QuickJS's documented lifecycle:
    // create a runtime, create a context from it, drop both at the end
    // in reverse order. All values returned by `JS_Eval` are owned
    // references that are released via `wzqjs_free_value` exactly once.
    unsafe {
        let rt = JS_NewRuntime();
        assert!(!rt.is_null(), "JS_NewRuntime returned null");

        // Same caps Warzone uses for script-map runs.
        JS_SetMemoryLimit(rt, 100 * 1024 * 1024);
        JS_SetMaxStackSize(rt, 512 * 1024);

        let options = JSLimitedContextOptions {
            baseObjects: 1,
            mapSet: 1,
            ..JSLimitedContextOptions::default()
        };
        let ctx = JS_NewLimitedContext(rt, &raw const options);
        assert!(!ctx.is_null(), "JS_NewLimitedContext returned null");

        let source = c"1 + 2";
        let filename = c"<smoke>";
        let value = JS_Eval_BypassLimitedContext(
            ctx,
            source.as_ptr(),
            source.count_bytes(),
            filename.as_ptr(),
            JS_EVAL_TYPE_GLOBAL,
        );
        assert_eq!(wzqjs_is_exception(value), 0, "eval threw");
        assert_eq!(wzqjs_value_get_tag(value), JS_TAG_INT, "expected int tag");

        let mut out: i32 = 0;
        let rc = JS_ToInt32(ctx, &raw mut out, value);
        assert_eq!(rc, 0, "JS_ToInt32 failed");
        assert_eq!(out, 3);

        wzqjs_free_value(ctx, value);
        JS_FreeContext(ctx);
        JS_FreeRuntime(rt);
    }
}
