---
title: "Architecture: daemon & clients"
tags: [doc, architecture]
---

## Daemon & clients

```
        ┌───────────── thin clients (transport/presentation only) ─────────────┐
        │   mdkb-cli      mdkb-mcp (MCP)     mdkb-web (HTTP)     mdkb-tauri (app) │
        └───────────────────────────────┬──────────────────────────────────────┘
                                         │  mdkb-protocol (JSON over local socket / TCP+token)
                                ┌────────▼────────┐
                                │      mdkbd       │  single writer: owns watcher + index + writes
                                │  mdkb-core::Service (capability-gated dispatch)
                                └────────┬────────┘
                          ┌──────────────┼───────────────┐
                       mdkb-core     mdkb-index        mdkb-embed
                    (block model,    (SQLite+FTS5+    (Embedder trait,
                     DAG, render,     vectors, RRF)    bundled/local/remote)
                     tags, search)
```

- **One daemon = one writer** over a vault. Local mode: a Unix socket (Windows named pipe),
  fail-closed (no network). Remote mode: TCP + shared token, capability-gated (default
  fail-closed).
- Clients **auto-start a detached daemon** for a local vault (it outlives the app) or connect to
  a remote one. Connection config is shared (`ConnectionConfig` / `connect` / `ensure_daemon` in
  `mdkb-protocol`). The single-daemon-per-vault and idle-shutdown guarantees are below.
- **Presentation is shared** via `mdkb-view` (Markdown→HTML, wikilink/embed decoration, XSS
  neutralization), so the web UI and desktop UI render through the exact same path.