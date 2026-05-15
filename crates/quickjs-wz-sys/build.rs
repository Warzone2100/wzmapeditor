//! Build the `quickjs-wz` C library and link it statically.
//!
//! Mirrors the upstream `CMakeLists.txt` at `vendor/quickjs-wz/CMakeLists.txt`:
//! same sources, same defines, same warning suppressions. We use the `cc`
//! crate instead of invoking `CMake` so end users don't need `CMake` on `PATH`.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vendor = manifest.join("vendor/quickjs-wz");
    let qjs = vendor.join("quickjs");
    let ext = vendor.join("quickjs-wz-extensions");

    println!("cargo:rerun-if-changed=vendor/quickjs-wz");
    println!("cargo:rerun-if-changed=src/wrapper.c");

    let version_path = qjs.join("VERSION.txt");
    if !version_path.exists() {
        try_init_submodule(&manifest, &version_path);
    }
    let version = std::fs::read_to_string(&version_path)
        .unwrap_or_else(|e| {
            panic!(
                "could not read {}: {e}\n\
                 The quickjs-wz submodule is missing and `git submodule update --init` \
                 could not fix it. Run it manually from the repo root:\n\
                 \n    git submodule update --init --recursive\n",
                version_path.display(),
            )
        })
        .trim()
        .to_owned();

    let mut build = cc::Build::new();
    build
        .file(qjs.join("cutils.c"))
        .file(qjs.join("dtoa.c"))
        .file(qjs.join("libregexp.c"))
        .file(qjs.join("libunicode.c"))
        .file(qjs.join("quickjs.c"))
        .file(manifest.join("src/wrapper.c"))
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
            "-Wno-unterminated-string-initialization",
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

/// Run `git submodule update --init --recursive` from the repo root when the
/// vendored quickjs-wz tree hasn't been fetched yet. Silent best-effort: if
/// `git` isn't on PATH or the repo isn't a git checkout (e.g. a downloaded
/// tarball) the caller falls back to a panic with manual-fix instructions.
fn try_init_submodule(manifest: &Path, version_path: &Path) {
    let Some(repo_root) = find_repo_root(manifest) else {
        return;
    };
    eprintln!("quickjs-wz submodule not found; running `git submodule update --init --recursive`");
    let status = std::process::Command::new("git")
        .args(["submodule", "update", "--init", "--recursive"])
        .current_dir(&repo_root)
        .status();
    match status {
        Ok(s) if s.success() && version_path.exists() => {
            eprintln!(
                "quickjs-wz submodule initialised at {}",
                repo_root.display()
            );
        }
        Ok(s) => eprintln!("git submodule update exited with {s}"),
        Err(e) => eprintln!("could not run git: {e}"),
    }
}

/// Walk parents of `start` looking for a `.git` entry (a directory in a
/// regular clone, a file inside a worktree). Returns the directory holding it.
fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start;
    loop {
        if cur.join(".git").exists() {
            return Some(cur.to_path_buf());
        }
        cur = cur.parent()?;
    }
}
