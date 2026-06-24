---
title: "README: Install — from source"
tags: [doc, readme]
---

### Install: from source

Requires Rust (stable); the workspace pins `rust-version = 1.80`. Clone the repo, then run any
interface straight from the source tree — each command auto-starts the daemon:

```sh
cargo build --workspace                          # build everything
cargo run -p mdkb-cli -- list ./my-vault         # CLI
cargo run -p mdkb-web -- --vault ./my-vault      # web UI → http://127.0.0.1:7878
cargo run -p mdkb-mcp -- --vault ./my-vault      # MCP server (stdio)

cargo test --workspace                           # the suite (green before every commit)
```

To install the headless binaries onto your `PATH` (instead of running from the tree), use
`cargo install` from the public mirror — this compiles locally, so on macOS the result is **not**
quarantined and Gatekeeper never blocks it:

```sh
cargo install --git https://github.com/<you>/mdkb mdkbd mdkb-cli mdkb-mcp mdkb-web
# installs: mdkbd (daemon), mdkb (CLI), mdkb-mcp, mdkb-web
```

This default build has semantic search built in: the neural model (BGE-small) is compiled into the
daemon, so `cargo build` / `cargo install` "just works" fully offline — real semantic embeddings,
no model files, no download. (Advanced: build with `--no-default-features` to leave the embedded
model out and fall back to the offline hash embedder.)

The desktop app lives in its own workspace and needs the Tauri toolchain — see
[`app/mdkb-tauri/README.md`](./app/mdkb-tauri/README.md). On macOS, building it from source with
`cargo tauri build` likewise yields an app that opens without the Gatekeeper "damaged" prompt; the
downloaded `.dmg` from a Release is unsigned and needs a one-time `xattr` unquarantine (documented
in that app README).
