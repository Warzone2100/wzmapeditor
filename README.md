<p align="center">
  <img src="crates/wzmapeditor/icons/256x256.png" alt="wzmapeditor" width="128" height="128">
</p>

<h1 align="center">wzmapeditor</h1>

<p align="center">A cross-platform map editor for <a href="https://wz2100.net/">Warzone 2100</a>, built in Rust with <a href="https://github.com/emilk/egui">egui</a> and <a href="https://github.com/gfx-rs/wgpu">wgpu</a>.</p>

![wzmapeditor](docs/screenshots/editor.jpg)

---

## Try it online

A browser build runs at **[mapeditor.wz2100.net](https://mapeditor.wz2100.net)**.

---

## Requirements

- A [Warzone 2100](https://wz2100.net/) 4.x installation

For building from source:

- [rustup](https://rustup.rs/) (installs `rustc` + `cargo`, stable 1.95+)

---

## Install

Prebuilt binaries are available for Windows (x64), macOS (Apple Silicon), and Linux (x64). Download the archive for your platform from the [Releases](../../releases) page, unzip, and run the executable.

---

## Configuration

Configuration and cached game data live in:

- Windows: `%APPDATA%\wzmapeditor\`
- Linux/macOS: `~/.config/wzmapeditor/`

---

## Building from source

Requires [Rust](https://rustup.rs/) 1.95 or later (stable toolchain).

```bash
git clone https://github.com/Warzone2100/wzmapeditor
cd wzmapeditor
cargo build --release
```

The latest `main` is deployed at [dev.mapeditor.wz2100.net](https://dev.mapeditor.wz2100.net).

For a debug build with logging:

```bash
RUST_LOG=info cargo run
```

## Running Tests

```bash
cargo test --workspace
```

## Linting

```bash
cargo fmt --check          # Check formatting
cargo clippy --workspace   # Run clippy lints (pedantic + cargo enabled)
```

---

## Related Projects

- [Warzone 2100](https://github.com/Warzone2100/warzone2100)
- [FlaME](https://github.com/Warzone2100/FlaME)
- [wzmaplib](https://github.com/Warzone2100/warzone2100/tree/master/lib/wzmaplib)
- [Maps Database](https://github.com/Warzone2100/maps-database)

---

## Licensing

wzmapeditor is free software; you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation; either version 2 of the License, or (at your option) any later version.

[![SPDX-License-Identifier: GPL-2.0-or-later](https://img.shields.io/static/v1?label=SPDX-License-Identifier&message=GPL-2.0-or-later&color=blue&logo=open-source-initiative&logoColor=white&logoWidth=10&style=flat-square)](COPYING)
