---
title: "README: Install — from source"
tags: [doc, readme]
---

### Install: with Rust (`cargo install`)

**Use this if you have Rust and just want the binaries** (CLI/MCP/web/daemon) without an installer.
Requires Rust (stable); the workspace pins `rust-version = 1.80`. This compiles locally, so on
macOS the result is **not** quarantined and Gatekeeper never blocks it:

```sh
cargo install --git https://github.com/<you>/mdkb mdkbd mdkb-cli mdkb-mcp mdkb-web
# installs: mdkbd (daemon), mdkb (CLI), mdkb-mcp, mdkb-web — onto ~/.cargo/bin (put it on PATH)
```

Zero-to-running from there:

```sh
echo "# First note" | mdkb create --vault ~/notes --title "First note"   # auto-starts the daemon
mdkb search --vault ~/notes "first note"
```

Semantic search is built in: the neural model (BGE-small) is compiled into the daemon, so
`cargo install` "just works" fully offline — real semantic embeddings, no model files, no download.
The first command may take a few seconds while the daemon starts and indexes; later ones are warm.
(Advanced: build with `--no-default-features` to leave the embedded model out and fall back to the
offline hash embedder.)

### Build from a checkout (contributors)

Clone the repo, then run any interface straight from the source tree — each command auto-starts the
daemon:

```sh
cargo build --workspace                                # build everything
cargo run -p mdkb-cli -- list --vault ./my-vault       # CLI
cargo run -p mdkb-web -- --vault ./my-vault            # web UI → http://127.0.0.1:7878
cargo run -p mdkb-mcp -- --vault ./my-vault            # MCP server (stdio)

cargo test --workspace                                 # the suite (green before every commit)
```

Working *inside* the repo against the repo's own `vault/`? The daemon's index is keyed by the
vault's absolute path and lives outside the vault, so it won't pollute your checkout. If you also
want `cargo`'s build output elsewhere (e.g. to avoid a huge in-tree `target/`), set
`CARGO_TARGET_DIR=~/.cache/mdkb-target` (or any path) before building.

The desktop app lives in its own workspace and needs the Tauri toolchain — see
[`app/mdkb-tauri/README.md`](./app/mdkb-tauri/README.md). On macOS, building it from source with
`cargo tauri build` likewise yields an app that opens without the Gatekeeper "damaged" prompt; the
downloaded `.dmg` from a Release is unsigned and needs a one-time `xattr` unquarantine (documented
in that app README).
