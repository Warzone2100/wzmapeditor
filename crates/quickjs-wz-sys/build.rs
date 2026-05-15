//! Build the `quickjs-wz` C library and link it statically.
//!
//! Mirrors the upstream `CMakeLists.txt` at `vendor/quickjs-wz/CMakeLists.txt`:
//! same sources, same defines, same warning suppressions. We use the `cc`
//! crate instead of invoking `CMake` so end users don't need `CMake` on `PATH`.

use std::path::PathBuf;

fn main() {
    let vendor = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/quickjs-wz");
    let qjs = vendor.join("quickjs");
    let ext = vendor.join("quickjs-wz-extensions");

    println!("cargo:rerun-if-changed=vendor/quickjs-wz");

    let version = std::fs::read_to_string(qjs.join("VERSION.txt"))
        .expect("read quickjs VERSION.txt")
        .trim()
        .to_owned();

    let mut build = cc::Build::new();
    build
        .file(qjs.join("cutils.c"))
        .file(qjs.join("dtoa.c"))
        .file(qjs.join("libregexp.c"))
        .file(qjs.join("libunicode.c"))
        .file(qjs.join("quickjs.c"))
        .include(&qjs)
        .include(&ext)
        .define("CONFIG_VERSION", format!("\"{version}\"").as_str())
        .define("QJS_DISABLE_ATOMICS", None);

    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target == "windows" {
        build
            .define("WIN32_LEAN_AND_MEAN", None)
            .define("_WIN32_WINNT", "0x0601");
    } else {
        build.define("_GNU_SOURCE", None);
    }

    // Platforms we ship to all have <sys/time.h>; quickjs uses it for
    // gettimeofday on non-Windows. Skip the autoconf-style probe.
    if target != "windows" {
        build.define("QUICKJS_HAVE_SYS_TIME_H", None);
    }

    if env == "msvc" {
        for flag in [
            "/wd4018", "/wd4061", "/wd4100", "/wd4200", "/wd4242", "/wd4244", "/wd4245", "/wd4267",
            "/wd4388", "/wd4389", "/wd4456", "/wd4457", "/wd4710", "/wd4711", "/wd4820", "/wd4996",
            "/wd5045", "/wd4115", "/wd4127", "/wd4132", "/wd4146", "/wd4295", "/wd4464", "/wd4702",
            "/wd4334",
        ] {
            build.flag_if_supported(flag);
        }
    } else {
        for flag in [
            "-Wno-implicit-fallthrough",
            "-Wno-sign-compare",
            "-Wno-missing-field-initializers",
            "-Wno-unused-parameter",
            "-Wno-unused-result",
            "-Wno-stringop-truncation",
            "-Wno-array-bounds",
            "-Wno-cast-align",
            "-Wno-format-nonliteral",
            "-funsigned-char",
        ] {
            build.flag_if_supported(flag);
        }
    }

    build.compile("quickjs-wz");

    // Threads dependency, on non-Windows targets.
    if target != "windows" {
        println!("cargo:rustc-link-lib=pthread");
    }

    println!("cargo:include={}", qjs.display());
    println!("cargo:ext_include={}", ext.display());
}
