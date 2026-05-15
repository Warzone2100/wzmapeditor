//! Static-link target for `quickjs-wz`.
//!
//! Phase 1 of the script-map import work: this crate's only job today is to
//! prove that the C library compiles on every platform we ship to (macOS,
//! Linux GNU, Windows MSVC). Raw FFI bindings will be added in a later
//! commit once the build is confirmed green.
#![no_std]
