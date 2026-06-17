# mdkb — Markdown Knowledge Base

A **personal Markdown knowledge base that is a tool, not an agent.**

mdkb stores knowledge as plain Markdown files (the single source of truth) and serves two
equal consumers:

- **You**, the human, via a desktop app that renders the knowledge cleanly with live
  transclusion (update a block once, every page that embeds it reflects the change).
- **AI clients** (e.g. GitHub Copilot) via an MCP server exposing deterministic
  search / write / link tools.

The store works fully with all AI turned off. It is decoupled from any agent: agents are
clients, not part of the store.

> Status: **early scaffold (Phase 0).** The workspace builds and tests pass, but most
> functionality is not implemented yet. See "Roadmap" below.

## Core principles

- **Markdown files are the source of truth.** Clean, diffable, editable in any editor.
- **The index is a rebuildable cache** (never synced, never authoritative). Delete it and
  re-derive from the Markdown — nothing is lost.
- **Block-level identity + transclusion** — every block has a stable id, encoded as an
  invisible HTML comment (`<!-- mdkb:<ulid> -->`) so files stay clean.
- **One shared core.** The MCP server, CLI, and desktop UI are thin clients of
  `mdkb-core`; behavior is never implemented twice. See [`AGENTS.md`](./AGENTS.md).

## Workspace layout

| Crate | Kind | Role |
|-------|------|------|
| `crates/mdkb-core` | lib | Shared engine: block model, ids, transclusion, indexing, search. |
| `crates/mdkb-index` | lib | SQLite (FTS5 + sqlite-vec) implementation of the core `Index` trait. |
| `crates/mdkbd` | bin | Headless daemon: owns the watcher, index, and writes. *(scaffold)* |
| `crates/mdkb-mcp` | bin (`mdkb-mcp`) | MCP server, thin client of core/daemon. *(scaffold)* |
| `crates/mdkb-cli` | bin (`mdkb`) | CLI for scripting/manual ops, thin client. *(scaffold)* |
| `app/mdkb-tauri` | app | Desktop UI, thin client. *(not started)* |

## Requirements

- Rust (stable). The workspace pins `rust-version = 1.80`.

## Build, test, run

```sh
# build everything
cargo build --workspace

# run the test suite (must be green before every commit — see AGENTS.md)
cargo test --workspace

# run the scaffold binaries
cargo run -p mdkbd
cargo run -p mdkb-mcp
cargo run -p mdkb-cli
```

## Implemented so far

- Workspace scaffold with the four crates above.
- `mdkb-core`:
  - `id` — `BlockId` (ULID) + `IdCodec` trait with the native `<!-- mdkb:<ulid> -->` encoding.
  - `block` / `document` — a fidelity-preserving Markdown parser producing block-level
    nodes (headings, paragraphs, code fences, quotes, list items, thematic breaks, HTML),
    with heading lineage, code-fence `lang`, and tags (inline `#tag`, frontmatter, lang).
    Eager id assignment splices invisible markers in without reformatting the file.
  - `link` — `[[...]]` / `![[...]]` reference parsing (page, id/heading anchor, display).
  - `vault` — in-memory or directory-backed page collection with name/id resolution,
    id assignment, and fidelity-preserving block edits.
  - `render` — transclusion resolver: inlines `![[...]]` embeds (the "update once,
    reflects everywhere" guarantee), renders links, breaks cycles.
  - `index` — storage-agnostic `Index` trait, owned records, search query/hit types, link
    extraction, and pure hybrid ranking (reciprocal rank fusion).
  - `sync` — `SyncEngine`: reconciles a vault directory with an index (hash-skip unchanged
    files, eager id assignment, incremental reindex, deletion detection).
- `mdkb-index` — SQLite + FTS5 implementation of `Index` (keyword search, tag/lang/page
  filters, backlinks, stats). Bundled SQLite, no system dependency.
- `mdkb` CLI commands: `render`, `assign-ids`, `list`, `search`, `stats`.

## Usage (current)

```sh
# assign stable ids to every block (writes invisible markers into your .md files)
cargo run -p mdkb-cli -- assign-ids ./my-vault

# render a page with all transclusions resolved
cargo run -p mdkb-cli -- render ./my-vault useful-queries

# keyword search, with optional filters
cargo run -p mdkb-cli -- search ./my-vault "restart nginx"
cargo run -p mdkb-cli -- search ./my-vault --lang=kusto
cargo run -p mdkb-cli -- search ./my-vault --tag=ops --limit=10

# list pages / index stats
cargo run -p mdkb-cli -- list ./my-vault
cargo run -p mdkb-cli -- stats ./my-vault
```

## Roadmap

- **Phase 0 — Scaffold** *(done)*: workspace, crates, governance docs.
- **Phase 1 — Core SSOT (no AI)** *(done)*: Markdown parser, block model (incl. code-fence
  `lang`), eager block-id assignment, transclusion/reference resolver, `#tag` + frontmatter
  extraction, CLI render.
- **Phase 2 — Index + watcher** *(done)*: SQLite (FTS5) index, keyword + tag/lang-filtered
  search, `SyncEngine` reconcile. *(Live `notify` event loop lands with the daemon.)*
- **Phase 3 — Semantic search** *(next)*: local `fastembed` embeddings, sqlite-vec, hybrid
  ranking.
- **Phase 3 — Semantic search**: local `fastembed` embeddings, hybrid ranking.
- **Phase 4 — Daemon + API**: `mdkbd` owns watcher/index/writes; clients talk to it.
- **Phase 5 — MCP server**: expose search / upsert / link tools.
- **Phase 6 — Tauri frontend**: render Markdown + resolved transclusions.
- **Phase 7 — Sync UX & packaging**: OneDrive conflict surfacing, index rebuild, packaging.

## License

Dual-licensed under MIT or Apache-2.0.
