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

The desktop app lives in its own workspace and needs the Tauri toolchain — see
[`app/mdkb-tauri/README.md`](./app/mdkb-tauri/README.md).
