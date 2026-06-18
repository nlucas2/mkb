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

> Status: **feature-complete** across all planned phases (parser, transclusion, index,
> semantic search, daemon, MCP, web + desktop UIs, cluster deploy). Versioned `0.0.0` /
> pre-release. See "Roadmap" for what each phase delivered.

## Two ways to run it

mdkb supports two deployment paradigms. The Markdown vault is the source of truth in both;
the difference is *where the daemon runs* and *how clients reach it*.

### 1. Local-first (single machine)

Everything runs on your machine. `mdkbd` owns the vault and a local **Unix socket**; the
index lives in `<vault>/.mdkb/` (local-only, rebuildable). The CLI, MCP server, web UI, and
desktop app all connect to that socket. To use multiple machines, **sync only the Markdown**
(OneDrive, etc.) — each machine runs its own daemon and keeps its own local index. This is
the default and needs no configuration.

```sh
mdkbd --vault ~/mdkb-vault          # serves ~/mdkb-vault/.mdkb/mdkbd.sock
```

### 2. Remote / shared daemon (e.g. in a cluster)

One `mdkbd` runs centrally (e.g. a `replicas: 1` Deployment in k3s) and serves a
**token-gated TCP** API. Thin clients — desktop app, web UI, CLI, MCP — connect over the
network by setting `MDKB_REMOTE=host:port` and `MDKB_TOKEN=<token>` (resolved by the shared
`Client::from_env`). The daemon stays a single writer; you scale *clients*, not the daemon.
Network access is opt-in and fails closed without a valid token.

```sh
mdkbd --vault /vault --listen 0.0.0.0:7820 --token "$MDKB_TOKEN"   # on the server
export MDKB_REMOTE=mdkbd.example:7820 MDKB_TOKEN=…                 # on each client
```

See [`deploy/README.md`](./deploy/README.md) for the Kubernetes manifest and end-to-end
remote setup.

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
| `crates/mdkb-embed` | lib | Embedder backends: offline hash embedder + optional local ONNX (`fastembed`). |
| `crates/mdkb-protocol` | lib | Wire protocol: request/response types, blocking client, shared dispatcher. |
| `crates/mdkbd` | bin | Headless daemon: owns the watcher, index, and writes; serves a local Unix socket. |
| `crates/mdkb-mcp` | bin (`mdkb-mcp`) | MCP server (stdio); thin client that forwards tool calls to the daemon. |
| `crates/mdkb-cli` | bin (`mdkb`) | CLI for scripting/manual ops, thin client. |
| `crates/mdkb-view` | lib | Shared presentation: Markdown→HTML rendering + page templating for any UI. |
| `crates/mdkb-web` | bin (`mdkb-web`) | Local web UI: thin HTTP server over the daemon + `mdkb-view`. |
| `app/mdkb-tauri` | app | Desktop shell (Tauri); thin client over `mdkb-view` + daemon. *(separate workspace)* |

## Requirements

- Rust (stable). The workspace pins `rust-version = 1.80`.

## Build, test, run

```sh
# build everything
cargo build --workspace

# run the test suite (must be green before every commit — see AGENTS.md)
cargo test --workspace

# see CLI usage
cargo run -p mdkb-cli -- --help
```

## Components

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
  filters, vector storage, brute-force cosine, hybrid keyword+vector fusion, backlinks,
  stats). Bundled SQLite, no system dependency.
- `mdkb-embed` — `Embedder` backends: the offline deterministic `HashEmbedder` (default)
  and an optional local ONNX model via `fastembed` (build with `--features onnx`).
- `mdkb-core::service` — the shared `Service` API (search / get / render / upsert / link /
  delete / reconcile) with a `RequestContext` + capability gate on every call. Every client
  goes through this; behavior is never reimplemented per client.
- `mdkb-protocol` — newline-delimited JSON wire types, a blocking Unix-socket `Client`, and
  the shared `dispatch` request handler.
- `mdkbd` — the headless daemon: owns a `SyncEngine` over SQLite + the vault, a `notify`
  file watcher that auto-reconciles external edits, and a local Unix-socket server. Can also
  serve a **token-gated TCP** listener (opt-in via `--listen`, fail-closed) for remote/cluster
  clients.
- `mdkb` CLI commands: `render`, `assign-ids`, `list`, `search` (keyword + semantic),
  `stats`, and `daemon` (talk to a running `mdkbd`: `ping`/`stats`/`list`/`search`/`render`/
  `rebuild`/`conflicts`).
- `mdkb-mcp` — an MCP server (JSON-RPC 2.0 over stdio) exposing the knowledge base as tools
  (`search`, `get_block`, `get_page`, `render_page`, `list_pages`, `backlinks`,
  `links_from`, `upsert_block`, `save_page`, `delete_page`, `link_blocks`, `stats`). It is a
  thin client that forwards every call to the daemon and auto-starts `mdkbd` if needed.

## Usage (current)

```sh
# assign stable ids to every block (writes invisible markers into your .md files)
cargo run -p mdkb-cli -- assign-ids ./my-vault

# render a page with all transclusions resolved
cargo run -p mdkb-cli -- render ./my-vault useful-queries

# search — combines keyword (FTS) and semantic (vector) ranking, with optional filters
cargo run -p mdkb-cli -- search ./my-vault "how do I restart nginx"
cargo run -p mdkb-cli -- search ./my-vault --lang=kusto
cargo run -p mdkb-cli -- search ./my-vault --tag=ops --limit=10

# list pages / index stats
cargo run -p mdkb-cli -- list ./my-vault
cargo run -p mdkb-cli -- stats ./my-vault
```

### Running the daemon

`mdkbd` owns the index and a file watcher, and serves a local Unix socket. Markdown is the
source of truth; the index lives in `<vault>/.mdkb/` and is local-only (exclude it from
cloud sync).

```sh
# start the daemon (defaults: vault ~/mdkb-vault, socket <vault>/.mdkb/mdkbd.sock)
cargo run -p mdkbd -- --vault ./my-vault

# from another shell, talk to it
cargo run -p mdkb-cli -- daemon ./my-vault/.mdkb/mdkbd.sock ping
cargo run -p mdkb-cli -- daemon ./my-vault/.mdkb/mdkbd.sock stats
cargo run -p mdkb-cli -- daemon ./my-vault/.mdkb/mdkbd.sock search "restart the web server"
```

By default the offline hash embedder is used (deterministic, no downloads). For real
semantic embeddings via a local ONNX model, build with the `onnx` feature:

```sh
cargo run -p mdkb-cli --features onnx -- search ./my-vault "restart the web server"
```

### Using mdkb from an AI agent (MCP)

`mdkb-mcp` speaks MCP over stdio and exposes the knowledge base as tools. Point any MCP
client at it; it auto-starts the daemon for the given vault.

```jsonc
// example MCP client config entry
{
  "command": "mdkb-mcp",
  "args": ["--vault", "/path/to/my-vault"]
}
```

### Browsing in a UI

Two front-ends share the same `mdkb-view` rendering layer (so they can't drift apart), and
both connect using the two paradigms above — a **local** socket or a **remote** TCP daemon:

- **Local web UI** (`mdkb-web`):

  ```sh
  # local daemon:
  mdkbd --vault ./my-vault &
  cargo run -p mdkb-web -- --vault ./my-vault          # http://127.0.0.1:7878

  # remote daemon:
  cargo run -p mdkb-web -- --remote mdkbd.example:7820 --token "$MDKB_TOKEN"
  ```

- **Desktop shell** (`app/mdkb-tauri`) — a Tauri app over the same crates; environment-driven
  (`MDKB_REMOTE` + `MDKB_TOKEN` for a remote daemon, else the local socket). Lives in its own
  workspace (needs the Tauri toolchain); see [`app/mdkb-tauri/README.md`](./app/mdkb-tauri/README.md).

## Roadmap

- **Phase 0 — Scaffold** *(done)*: workspace, crates, governance docs.
- **Phase 1 — Core SSOT (no AI)** *(done)*: Markdown parser, block model (incl. code-fence
  `lang`), eager block-id assignment, transclusion/reference resolver, `#tag` + frontmatter
  extraction, CLI render.
- **Phase 2 — Index + watcher** *(done)*: SQLite (FTS5) index, keyword + tag/lang-filtered
  search, `SyncEngine` reconcile. *(Live `notify` event loop lands with the daemon.)*
- **Phase 3 — Semantic search** *(done)*: local embeddings (`mdkb-embed`: offline hash +
  optional `fastembed` ONNX), vector storage, hybrid keyword+vector ranking.
- **Phase 4 — Daemon + API** *(done)*: shared `Service` API + `RequestContext`, JSON wire
  protocol, `mdkbd` with a local Unix-socket server and `notify` file watcher.
- **Phase 5 — MCP server** *(done)*: `mdkb-mcp` exposes search / get / render / upsert /
  link / stats as MCP tools over stdio; thin client of the daemon.
- **Phase 6 — Frontends** *(done)*: shared `mdkb-view` (Markdown→HTML), runnable
  `mdkb-web` local UI, and a `app/mdkb-tauri` desktop shell over the same view layer.
- **Phase 7 — Sync UX & packaging** *(done)*: cloud-sync conflict detection (surfaced, not
  indexed), index `rebuild`, token-gated TCP transport for cluster deploy, Dockerfile + k8s
  manifest + example MCP config (`deploy/`).

## Deployment

See [`deploy/README.md`](./deploy/README.md). In short: run `mdkbd --vault <dir>` locally,
or deploy the daemon to k3s/Kubernetes as a single writer (`replicas: 1`) serving a
token-gated TCP API (`deploy/k8s.yaml`, `Dockerfile`). Sync only the Markdown vault across
machines; each daemon keeps its own local, rebuildable index.

## License

Dual-licensed under MIT or Apache-2.0.
