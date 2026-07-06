---
title: "README: Install — from source"
tags: [doc/readme]
updated: 2026-07-06T05:02:42Z
---

## From source

The one-command path uses [`just`](https://github.com/casey/just), which builds and installs the
**whole product** (desktop app + daemon + CLI + MCP). It's also how arm64 Linux — which has no
prebuilt desktop release — gets the app:

```sh
just install        # everything: desktop app + daemon + CLI + MCP server
just app            # build just the desktop app bundle
just --list         # every recipe (build, test, check, docs, …)
```

Requires the [prerequisites](PREREQS.md) (Rust, `just`, Tauri CLI, system build libraries).
Building from source on macOS also avoids the Gatekeeper "damaged" prompt that a downloaded,
unsigned `.dmg` triggers.

**Without `just`** (the raw commands `just install` runs). Build the daemon, CLI, and MCP server,
stage them beside the app so it can bundle them, then build and install the desktop bundle — the
app is what puts `mkb` and `mkb-mcp` on your PATH:

```sh
git clone https://github.com/<you>/mkb && cd mkb
cargo build --release -p mkbd -p mkb-cli -p mkb-mcp        # the bins the app bundles
mkdir -p app/mkb-tauri/src-tauri/bin
cp target/release/mkbd    app/mkb-tauri/src-tauri/bin/mkbd
cp target/release/mkb-mcp app/mkb-tauri/src-tauri/bin/mkb-mcp
cp target/release/mkb     app/mkb-tauri/src-tauri/bin/mkb
cd app/mkb-tauri && cargo tauri icon app-icon.png           # generate the icon set
cd src-tauri && cargo tauri build                            # bundle → target/release/bundle/
```

Then install the bundle for your OS (macOS → copy `mkb.app` to `/Applications`; Linux → the
`.deb`/`.AppImage`; Windows → run the `*-setup.exe`). `just install` automates exactly this — these
are its steps spelled out.

Semantic search is built in either way: the BGE-small model is compiled into the daemon, so it
works fully offline — no model files, no download. (Advanced: `--no-default-features` leaves the
embedded model out and falls back to the offline hash embedder.)

**Zero-to-running:**

```sh
echo "# First note" | mkb create --vault ~/notes --title "First note"   # auto-starts the daemon
mkb search --vault ~/notes "first note"
```

The first command may take a few seconds while the daemon starts and indexes; later ones are warm.

**Contributors** run the interfaces straight from the tree (`cargo run -p mkb-cli -- … --vault …`,
`cargo test --workspace`). The index is keyed by the vault's absolute path and lives outside the
vault, so it never pollutes your checkout; set `CARGO_TARGET_DIR=~/.cache/mkb-target` to move
build output out of the tree too.
