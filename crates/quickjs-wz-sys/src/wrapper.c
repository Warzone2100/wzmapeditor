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

void wzqjs_free_value(JSContext *ctx, JSValue v) {
    JS_FreeValue(ctx, v);
}

int wzqjs_is_exception(JSValue v) {
    return JS_IsException(v);
}

int wzqjs_value_get_tag(JSValue v) {
    return JS_VALUE_GET_TAG(v);
}

int wzqjs_value_get_int(JSValue v) {
    return JS_VALUE_GET_INT(v);
}

JSValue wzqjs_undefined(void) {
    return JS_UNDEFINED;
}
