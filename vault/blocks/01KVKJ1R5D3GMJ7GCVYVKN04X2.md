---
title: "README: Install — from source"
tags: [doc, readme]
---

### Install: with Rust (`cargo install`)

**Use this if you have Rust and just want the headless tools** (CLI/MCP/daemon) without an
installer. Requires Rust (stable); the workspace pins `rust-version = 1.80`. This compiles locally,
so on macOS the result is **not** quarantined and Gatekeeper never blocks it:

```sh
cargo install --git https://github.com/<you>/mdkb mdkbd mdkb-cli mdkb-mcp
# installs: mdkbd (daemon), mdkb (CLI), mdkb-mcp — onto ~/.cargo/bin (put it on PATH)
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

### Build everything from a checkout (`just`)

Want the **whole product** — the desktop app *and* the daemon/CLI/MCP — from source? Clone the
repo and use the [`just`](https://github.com/casey/just) recipes (this is also how arm64 Linux,
which has no prebuilt desktop release, gets the app):

```sh
just install        # headless tools onto PATH + build & install the desktop app
just install-cli    # only the headless tools (daemon + CLI + MCP)
just app            # just build the desktop app bundle
just --list         # all recipes (build, test, check, docs, …)
```

`just install` needs the Tauri toolchain (`cargo install tauri-cli` + your platform's webkit/GTK
dev libs) — see [`app/mdkb-tauri/README.md`](./app/mdkb-tauri/README.md). On macOS, building the
app from source yields one that opens without the Gatekeeper "damaged" prompt (unlike the
downloaded `.dmg`, which is unsigned and needs a one-time `xattr` unquarantine).

Prefer raw cargo? Each interface runs straight from the tree — each auto-starts the daemon:

```sh
cargo build --workspace                                # build everything
cargo run -p mdkb-cli -- list --vault ./my-vault       # CLI
cargo run -p mdkb-mcp -- --vault ./my-vault            # MCP server (stdio)

cargo test --workspace                                 # the suite (green before every commit)
```

Working *inside* the repo against the repo's own `vault/`? The daemon's index is keyed by the
vault's absolute path and lives outside the vault, so it won't pollute your checkout. If you also
want `cargo`'s build output elsewhere (e.g. to avoid a huge in-tree `target/`), set
`CARGO_TARGET_DIR=~/.cache/mdkb-target` (or any path) before building.
