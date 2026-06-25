---
title: "README: Install — from source"
tags: [doc, readme]
---

## From source

The one-command path uses [`just`](https://github.com/casey/just), which builds and installs the
**whole product** (desktop app + daemon + CLI + MCP). It's also how arm64 Linux — which has no
prebuilt desktop release — gets the app:

```sh
just install        # everything: desktop app + daemon + CLI + MCP server
just install-cli    # headless tools only (daemon + CLI + MCP), no GUI
just app            # build just the desktop app bundle
just --list         # every recipe (build, test, check, docs, …)
```

Requires the [prerequisites](PREREQS.md) (Rust, `just`, Tauri CLI, system build libraries).
Building from source on macOS also avoids the Gatekeeper "damaged" prompt that a downloaded,
unsigned `.dmg` triggers.

**Without `just`** (the raw commands `just install` runs). The headless tools install with one
`cargo install`; the desktop app is built with `cargo tauri build` right alongside it:

```sh
# 1. headless tools (daemon + CLI + MCP) → ~/.cargo/bin
cargo install --git https://github.com/<you>/mdkb mdkbd mdkb-cli mdkb-mcp

# 2. desktop app — build the bundle from a checkout
git clone https://github.com/<you>/mdkb && cd mdkb
cargo build --release -p mdkbd -p mdkb-cli -p mdkb-mcp        # bins the app bundles
mkdir -p app/mdkb-tauri/src-tauri/bin
cp target/release/mdkbd    app/mdkb-tauri/src-tauri/bin/mdkbd
cp target/release/mdkb-mcp app/mdkb-tauri/src-tauri/bin/mdkb-mcp
cp target/release/mdkb     app/mdkb-tauri/src-tauri/bin/mdkb-cli
cd app/mdkb-tauri && cargo tauri icon app-icon.png           # generate the icon set
cd src-tauri && cargo tauri build                            # bundle → target/release/bundle/
```

Then install the bundle for your OS (macOS → copy `mdkb.app` to `/Applications`; Linux → the
`.deb`/`.AppImage`; Windows → run the `*-setup.exe`). `just install` automates exactly this — these
are its steps spelled out.

Semantic search is built in either way: the BGE-small model is compiled into the daemon, so it
works fully offline — no model files, no download. (Advanced: `--no-default-features` leaves the
embedded model out and falls back to the offline hash embedder.)

**Zero-to-running:**

```sh
echo "# First note" | mdkb create --vault ~/notes --title "First note"   # auto-starts the daemon
mdkb search --vault ~/notes "first note"
```

The first command may take a few seconds while the daemon starts and indexes; later ones are warm.

**Contributors** run the interfaces straight from the tree (`cargo run -p mdkb-cli -- … --vault …`,
`cargo test --workspace`). The index is keyed by the vault's absolute path and lives outside the
vault, so it never pollutes your checkout; set `CARGO_TARGET_DIR=~/.cache/mdkb-target` to move
build output out of the tree too.
