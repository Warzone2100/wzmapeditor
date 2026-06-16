// Glue between the Rust/wasm app and the Binomial Basis Universal transcoder
// (vendored as basis_transcoder.{js,wasm}, Apache-2.0). Loaded by Rust through
// `#[wasm_bindgen(module = "/js/basis_glue.js")]`.
//
// basis_transcoder.js is a classic emscripten script that defines a global
// `BASIS` factory; it is not an ES module, so it is injected via a <script>
// tag rather than imported. The .wasm is fetched and handed to the factory as
// `wasmBinary` so no locateFile guesswork is needed.

let _module = null;

// transcoder_texture_format::cTFRGBA32 — uncompressed 8-bit RGBA output. The
// transcodeImage format argument is a plain integer (not the embind enum), and
// this index is fixed by the Basis ABI.
const RGBA32 = 13;

function loadScript(url) {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = url;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error("failed to load " + url));
    document.head.appendChild(script);
  });
}

// Load and initialize the transcoder once. `jsUrl`/`wasmUrl` are resolved
// against the document base so they work under a project-Pages sub-path.
export async function initBasis(jsUrl, wasmUrl) {
  if (_module) return true;
  const jsHref = new URL(jsUrl, document.baseURI).href;
  const wasmHref = new URL(wasmUrl, document.baseURI).href;
  await loadScript(jsHref);
  const response = await fetch(wasmHref);
  if (!response.ok) throw new Error("failed to fetch " + wasmHref);
  const wasmBinary = new Uint8Array(await response.arrayBuffer());
  _module = await globalThis.BASIS({ wasmBinary });
  _module.initializeBasis();
  return true;
}

// Transcode a whole KTX2 file (UASTC, possibly zstd-supercompressed) to RGBA8.
// Returns { width, height, data } or null on any failure. KTX2File decompresses
// zstd internally, so the raw file bytes are passed through unchanged.
export function transcodeKtx2(bytes) {
  if (!_module) return null;
  let ktx2 = null;
  try {
    ktx2 = new _module.KTX2File(bytes);
    if (!ktx2.getLevels() || !ktx2.startTranscoding()) return null;
    const width = ktx2.getWidth();
    const height = ktx2.getHeight();
    const size = ktx2.getImageTranscodedSizeInBytes(0, 0, 0, RGBA32);
    const data = new Uint8Array(size);
    // transcodeImage(dst, level, layer, face, format, getAlphaForOpaque, ch0, ch1)
    const ok = ktx2.transcodeImage(data, 0, 0, 0, RGBA32, 0, -1, -1);
    if (!ok) return null;
    return { width, height, data };
  } finally {
    if (ktx2) {
      ktx2.close();
      ktx2.delete();
    }
  }
}
