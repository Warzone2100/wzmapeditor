/*
 * Extern shims for QuickJS macros and `static inline` helpers.
 *
 * QuickJS exposes a number of value helpers as macros (e.g. `JS_NULL`,
 * `JS_TRUE`, `JS_NewInt32`) or `static inline` functions (e.g.
 * `JS_FreeValue`, `JS_IsException`). These have no link-time symbol, so
 * Rust FFI cannot call them directly. Wrap each one we need as a plain
 * `extern` function with a `wzqjs_` prefix.
 */
#include "quickjs.h"

/* Value lifetime / inspection */
void wzqjs_free_value(JSContext *ctx, JSValue v) { JS_FreeValue(ctx, v); }
JSValue wzqjs_dup_value(JSContext *ctx, JSValue v) { return JS_DupValue(ctx, v); }
int wzqjs_value_get_tag(JSValue v) { return JS_VALUE_GET_TAG(v); }
int wzqjs_value_get_int(JSValue v) { return JS_VALUE_GET_INT(v); }

/* Type predicates */
int wzqjs_is_exception(JSValue v) { return JS_IsException(v); }
int wzqjs_is_undefined(JSValue v) { return JS_IsUndefined(v); }
int wzqjs_is_null(JSValue v) { return JS_IsNull(v); }
int wzqjs_is_number(JSValue v) { return JS_IsNumber(v); }
int wzqjs_is_bool(JSValue v) { return JS_IsBool(v); }
int wzqjs_is_string(JSValue v) { return JS_IsString(v); }
int wzqjs_is_object(JSValue v) { return JS_IsObject(v); }

/* Singleton-tag value constructors (these are macros in quickjs.h). */
JSValue wzqjs_undefined(void) { return JS_UNDEFINED; }
JSValue wzqjs_null(void) { return JS_NULL; }
JSValue wzqjs_true(void) { return JS_TRUE; }
JSValue wzqjs_false(void) { return JS_FALSE; }
JSValue wzqjs_exception(void) { return JS_EXCEPTION; }

/* Primitive constructors (most of these are macros in quickjs.h). */
JSValue wzqjs_new_bool(JSContext *ctx, int v) { return JS_NewBool(ctx, v); }
JSValue wzqjs_new_int32(JSContext *ctx, int32_t v) { return JS_NewInt32(ctx, v); }
JSValue wzqjs_new_uint32(JSContext *ctx, uint32_t v) { return JS_NewUint32(ctx, v); }
JSValue wzqjs_new_float64(JSContext *ctx, double v) { return JS_NewFloat64(ctx, v); }

/* String helpers (JS_NewString / JS_ToCString are static inline). */
JSValue wzqjs_new_string_len(JSContext *ctx, const char *str, size_t len) {
    return JS_NewStringLen(ctx, str, len);
}
const char *wzqjs_to_cstring_len(JSContext *ctx, size_t *plen, JSValue v) {
    return JS_ToCStringLen(ctx, plen, v);
}

/* Numeric coercions (JS_ToUint32 is static inline). */
int wzqjs_to_uint32(JSContext *ctx, uint32_t *pres, JSValue v) {
    return JS_ToUint32(ctx, pres, v);
}

/* Function registration (JS_NewCFunction is static inline, calls
 * JS_NewCFunction2 internally with JS_CFUNC_generic). */
JSValue wzqjs_new_cfunction(JSContext *ctx, JSCFunction *func, const char *name, int length) {
    return JS_NewCFunction(ctx, func, name, length);
}
