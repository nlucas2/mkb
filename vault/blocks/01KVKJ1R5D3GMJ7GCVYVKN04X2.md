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

Requires Rust (the workspace pins `rust-version = 1.80`); `just install` additionally needs the
Tauri toolchain (`cargo install tauri-cli` + your platform's webkit/GTK dev libs) — see
[`app/mdkb-tauri/README.md`](./app/mdkb-tauri/README.md). Building from source on macOS also avoids
the Gatekeeper "damaged" prompt that a downloaded, unsigned `.dmg` triggers.

**No `just`, headless only.** Install the daemon + CLI + MCP straight onto `~/.cargo/bin` (no
desktop app):

```sh
cargo install --git https://github.com/<you>/mdkb mdkbd mdkb-cli mdkb-mcp
```

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
