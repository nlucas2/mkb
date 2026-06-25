---
title: Workspace layout
tags: [doc, concept]
---

| Crate | Kind | Role |
|-------|------|------|
| `crates/mkb-core` | lib | Shared engine: block model, ids, transclusion, indexing, search. |
| `crates/mkb-index` | lib | SQLite (FTS5 + sqlite-vec) implementation of the core `Index` trait. |
| `crates/mkb-embed` | lib | Embedder backends: offline hash embedder + optional local ONNX (`fastembed`). |
| `crates/mkb-protocol` | lib | Wire protocol: request/response types, blocking client, shared dispatcher. |
| `crates/mkbd` | bin | Headless daemon: owns the watcher, index, and writes; serves a local socket (Unix socket / Windows named pipe). |
| `crates/mkb-mcp` | bin (`mkb-mcp`) | MCP server (stdio); thin client that forwards tool calls to the daemon. |
| `crates/mkb-cli` | bin (`mkb`) | CLI for scripting/manual ops, thin client. |
| `crates/mkb-view` | lib | Shared presentation: Markdown→HTML rendering + page templating for any UI. |
| `app/mkb-tauri` | app | Desktop shell (Tauri); thin client over `mkb-view` + daemon. *(separate workspace)* |

If a piece of behavior doesn't clearly belong to transport or presentation, it belongs in
`mkb-core`.
