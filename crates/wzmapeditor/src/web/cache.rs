//! Thin wrappers over the browser Cache Storage API.
//!
//! Three subsystems persist bytes in Cache Storage (downloaded `.wz` archives,
//! decoded HQ ground layers, generated thumbnails). They share the same
//! open/match/put/enumerate plumbing and the same secure-context guard, which
//! lives here once. Each subsystem keeps only its own bucket name and the
//! synthetic-URL scheme it keys entries under.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// Open (or create) a named cache bucket.
///
/// Returns `None` on insecure origins, where `window.caches` is `undefined`
/// and calling `open()` on it would throw. Callers degrade to no-ops.
pub(crate) async fn open(name: &str) -> Option<web_sys::Cache> {
    let caches = web_sys::window()?.caches().ok()?;
    if caches.is_undefined() {
        return None;
    }
    let value = JsFuture::from(caches.open(name)).await.ok()?;
    value.dyn_into::<web_sys::Cache>().ok()
}

/// Look up a cached response by its synthetic URL.
pub(crate) async fn match_url(cache: &web_sys::Cache, url: &str) -> Option<web_sys::Response> {
    let value = JsFuture::from(cache.match_with_str(url)).await.ok()?;
    if value.is_undefined() {
        return None;
    }
    value.dyn_into::<web_sys::Response>().ok()
}

/// Look up the cached response for an enumerated request.
pub(crate) async fn match_request(
    cache: &web_sys::Cache,
    request: &web_sys::Request,
) -> Option<web_sys::Response> {
    let value = JsFuture::from(cache.match_with_request(request))
        .await
        .ok()?;
    value.dyn_into::<web_sys::Response>().ok()
}

/// Read a response body fully into a byte vector.
pub(crate) async fn read_response_bytes(resp: &web_sys::Response) -> Option<Vec<u8>> {
    let buffer = JsFuture::from(resp.array_buffer().ok()?).await.ok()?;
    Some(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Store raw bytes under `url`. Returns whether the write succeeded.
pub(crate) async fn put_bytes(cache: &web_sys::Cache, url: &str, bytes: Vec<u8>) -> bool {
    let Some(resp) = response_from_bytes(bytes) else {
        return false;
    };
    JsFuture::from(cache.put_with_str(url, &resp)).await.is_ok()
}

/// Store an existing `Response` under `url`. Used to cache a fetch result whose
/// body is consumed separately for progress.
pub(crate) async fn put_response(cache: &web_sys::Cache, url: &str, resp: &web_sys::Response) {
    let _ = JsFuture::from(cache.put_with_str(url, resp)).await;
}

/// Enumerate cached entries whose URL contains `prefix`, returning the decoded
/// trailing key segment and the originating request for each.
///
/// `Request::url()` is absolute, so the prefix is located within it; the
/// trailing segment is percent-decoded to undo [`encode_component`].
pub(crate) async fn keys_with_prefix(
    cache: &web_sys::Cache,
    prefix: &str,
) -> Vec<(String, web_sys::Request)> {
    let mut out = Vec::new();
    let Ok(keys) = JsFuture::from(cache.keys()).await else {
        return out;
    };
    for request in js_sys::Array::from(&keys).iter() {
        let Ok(request) = request.dyn_into::<web_sys::Request>() else {
            continue;
        };
        let url = request.url();
        let Some(pos) = url.find(prefix) else {
            continue;
        };
        let Ok(name) = js_sys::decode_uri_component(&url[pos + prefix.len()..]) else {
            continue;
        };
        out.push((String::from(name), request));
    }
    out
}

/// Delete the entry an enumerated request points at. Returns whether one was
/// removed.
pub(crate) async fn delete_request(cache: &web_sys::Cache, request: &web_sys::Request) -> bool {
    JsFuture::from(cache.delete_with_request(request))
        .await
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Percent-encode a key segment so it survives round-tripping through a URL.
pub(crate) fn encode_component(s: &str) -> String {
    String::from(js_sys::encode_uri_component(s))
}

fn response_from_bytes(bytes: Vec<u8>) -> Option<web_sys::Response> {
    let mut bytes = bytes;
    web_sys::Response::new_with_opt_u8_array(Some(&mut bytes)).ok()
}
