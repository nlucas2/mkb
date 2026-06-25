---
title: Workspace layout
tags: [doc, concept]
---

| Crate | Kind | Role |
|-------|------|------|
| `crates/mdkb-core` | lib | Shared engine: block model, ids, transclusion, indexing, search. |
| `crates/mdkb-index` | lib | SQLite (FTS5 + sqlite-vec) implementation of the core `Index` trait. |
| `crates/mdkb-embed` | lib | Embedder backends: offline hash embedder + optional local ONNX (`fastembed`). |
| `crates/mdkb-protocol` | lib | Wire protocol: request/response types, blocking client, shared dispatcher. |
| `crates/mdkbd` | bin | Headless daemon: owns the watcher, index, and writes; serves a local socket (Unix socket / Windows named pipe). |
| `crates/mdkb-mcp` | bin (`mdkb-mcp`) | MCP server (stdio); thin client that forwards tool calls to the daemon. |
| `crates/mdkb-cli` | bin (`mdkb`) | CLI for scripting/manual ops, thin client. |
| `crates/mdkb-view` | lib | Shared presentation: Markdown→HTML rendering + page templating for any UI. |
| `app/mdkb-tauri` | app | Desktop shell (Tauri); thin client over `mdkb-view` + daemon. *(separate workspace)* |

If a piece of behavior doesn't clearly belong to transport or presentation, it belongs in
`mdkb-core`.
