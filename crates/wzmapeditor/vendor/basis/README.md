# Vendored Basis Universal transcoder

`basis_transcoder.js` and `basis_transcoder.wasm` are the prebuilt Binomial
Basis Universal transcoder, used by the web build to decode the uploaded
`high.wz` KTX2/UASTC terrain textures in the browser (the native build links
the `basis-universal` C++ crate instead, which cannot target
`wasm32-unknown-unknown`).

- Upstream: <https://github.com/BinomialLLC/basis_universal> (`webgl/transcoder`)
- Obtained from the three.js distribution
  (`examples/jsm/libs/basis/`), which tracks the upstream build.
- License: Apache-2.0 — compatible with this project's GPL-2.0-or-later.

The build is compiled with KTX2 Zstandard support, so `KTX2File` decompresses
zstd-supercompressed levels internally. Loaded on demand via `js/basis_glue.js`.
