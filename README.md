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
- `mdkb-core::id` — `BlockId` (ULID-based) and the `IdCodec` trait with the native
  `<!-- mdkb:<ulid> -->` encoding, plus unit tests.

## Roadmap

- **Phase 0 — Scaffold** *(in progress)*: workspace, crates, governance docs.
- **Phase 1 — Core SSOT (no AI)**: Markdown parser, block model (incl. code-fence `lang`),
  block-id assignment, transclusion/reference resolver, `#tag` + frontmatter extraction.
- **Phase 2 — Index + watcher**: SQLite (`sqlite-vec` + FTS5), file watcher, keyword and
  tag/lang-filtered search.
- **Phase 3 — Semantic search**: local `fastembed` embeddings, hybrid ranking.
- **Phase 4 — Daemon + API**: `mdkbd` owns watcher/index/writes; clients talk to it.
- **Phase 5 — MCP server**: expose search / upsert / link tools.
- **Phase 6 — Tauri frontend**: render Markdown + resolved transclusions.
- **Phase 7 — Sync UX & packaging**: OneDrive conflict surfacing, index rebuild, packaging.

## License

Dual-licensed under MIT or Apache-2.0.
