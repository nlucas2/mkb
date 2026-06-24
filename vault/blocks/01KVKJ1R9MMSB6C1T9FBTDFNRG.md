---
title: "README: Crate & module breakdown"
tags: [doc, readme]
---

- `mdkb-core`:
  - `id` — `BlockId` (ULID): a block's identity is its filename stem.
  - `blockfile` — parse a block file (`blocks/<ULID>.md`): YAML frontmatter (`title:`, `tags:`)
    + clean Markdown body; collects inline `#tags` and code-fence languages.
  - `block` — the block model (id, title, tags, langs, body) + derived views (children,
    references, contextual text for embeddings).
  - `link` — `[[target]]` / `![[target]]` directive parsing (target = ULID or title, display alias).
  - `vault` — the DAG: a map of `BlockId → Block` loaded from `blocks/`, with id/title
    resolution, children/backlinks, and root detection.
  - `render` — the transclusion resolver: expands `![[id]]` children (the "edit once, reflects
    everywhere" guarantee), renders `[[id]]` as `mdkb:` links, and is **total** — breaks cycles
    and degrades dangling targets locally with a visible note.
  - `index` — storage-agnostic `Index` trait, owned records, search query/hit types, link
    extraction, the knowledge graph, transclusion-reachability, and hybrid ranking (RRF).
  - `sync` — `SyncEngine`: reconciles the `blocks/` directory with an index (hash-skip
    unchanged files, incremental reindex, deletion + conflict-copy detection); owns block
    create/update/delete/carve writes.
- `mdkb-index` — SQLite + FTS5 implementation of `Index` (keyword search, tag/lang filters,
  vector storage, brute-force cosine, hybrid keyword+vector fusion, backlinks, stats). Bundled
  SQLite, no system dependency.
- `mdkb-embed` — `Embedder` backends: the offline deterministic `HashEmbedder` and a local INT8
  ONNX model (BGE-small) compiled into the daemon by default (`include_bytes!`, no runtime
  download); configurable per vault.
- `mdkb-core::service` — the shared `Service` API (search / get / render / create / update /
  delete / carve / link / reconcile) with a `RequestContext` + capability gate on every call.
  Every client goes through this; behavior is never reimplemented per client.
- `mdkb-protocol` — newline-delimited JSON wire types, a blocking `Client` (local socket or
  token-gated TCP), the shared `dispatch` handler, and the shared connection layer
  (`ConnectionConfig` / `connect` / `ensure_daemon` — auto-starts a **detached** daemon).
- `mdkbd` — the headless daemon: owns a `SyncEngine` over SQLite + the vault, a `notify` file
  watcher that auto-reconciles external edits, and a local-socket server (Unix socket / Windows
  named pipe). Can also serve a **token-gated TCP** listener (opt-in via `--listen`,
  fail-closed) for remote/cluster clients.
- `mdkb` CLI — a **thin daemon client** (auto-starts `mdkbd` for the vault, then dispatches over
  the socket; reads *and* writes, a full equivalent of the MCP surface). Reads: `list`, `render`,
  `get`, `search`, `tags`, `backlinks`, `links`, `stats`, `conflicts`, `ping`. Writes: `create`,
  `update`, `set-tags`, `link`, `carve`, `flatten`, `delete`. Maintenance: `rebuild`, `export`.
- `mdkb-mcp` — an MCP server (JSON-RPC 2.0 over stdio) exposing the knowledge base as tools
  (`search`, `get_block`, `render_block`, `list_blocks`, `list_roots`, `graph`, `list_tags`,
  `backlinks`, `links_from`, `create_block`, `update_block`, `set_tags`, `delete_block`,
  `carve_block`, `flatten_block`, `link_blocks`, `stats`, `rebuild`, `conflicts`). A thin client that forwards
  every call to the daemon and auto-starts `mdkbd` if needed.
