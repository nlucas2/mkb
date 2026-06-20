# mdkb — Markdown Knowledge Base

A **personal Markdown knowledge base that is a tool, not an agent.**

mdkb stores knowledge as plain Markdown files (the single source of truth) and serves two
equal consumers:

- **You**, the human, via a desktop app that renders the knowledge cleanly with live
  transclusion (edit a block once, every block that embeds it reflects the change).
- **AI clients** (e.g. GitHub Copilot) via an MCP server exposing deterministic
  search / write / link tools.

The store works fully with all AI turned off. It is decoupled from any agent: agents are
clients, not part of the store.

> **Model: file-per-block.** A *block* is the unit of knowledge and **is a file**
> (`blocks/<ULID>.md`). A block can embed other blocks (`![[id]]` = a live child /
> transclusion) or link to them (`[[id]]` = a reference). A "page" is just a block you open at
> the top — there is no separate page concept. This makes reuse uniform (a reusable chunk is
> its own block, embedded anywhere, edited once) and **sync-friendly** (editing one block
> touches one small file). See **[`docs/architecture.md`](./docs/architecture.md)** for the
> design and **[`docs/SPEC.md`](./docs/SPEC.md)** for the exact on-disk format.

> Status: core re-architected to the file-per-block model (parser, transclusion, index,
> semantic search, daemon, MCP, web + desktop UIs). Versioned `0.0.0` / pre-release.

## Two ways to run it

mdkb supports two deployment paradigms. The Markdown vault is the source of truth in both;
the difference is *where the daemon runs* and *how clients reach it*.

### 1. Local-first (single machine)

Everything runs on your machine. `mdkbd` owns the vault and a local socket (a **Unix-domain
socket** on Linux/macOS, a **named pipe** on Windows); the index lives in `<vault>/.mdkb/`
(local-only, rebuildable). The CLI, MCP server, web UI, and desktop app all connect to that
socket. **At most one daemon owns a vault** — it holds an exclusive lock on `.mdkb/mdkbd.lock`,
so a stray second launch refuses to start and you never get two writers. A daemon the app (or
any client) auto-starts also **self-reaps after 15 minutes idle**, so an unused vault doesn't
leave a process and its embedder RAM resident; the next interaction transparently restarts it. A
daemon you run by hand stays up. To use multiple machines, **sync only the Markdown** (OneDrive,
etc.) — each machine runs its own daemon and keeps its own local index. This is the default and
needs no configuration.

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
- **Block = file = page.** Every block is a file `blocks/<ULID>.md`; the ULID filename is its
  stable identity. Reuse is uniform: `![[id]]` embeds a block (and its subtree) live, `[[id]]`
  links to it. A page is just a root block.
- **One shared core.** The MCP server, CLI, web UI, and desktop UI are thin clients of
  `mdkb-core`; behavior is never implemented twice. See [`AGENTS.md`](./AGENTS.md).

## Workspace layout

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

## The vault (`vault/`)

mdkb's own knowledge lives in [`vault/`](./vault) as interlinked blocks — it is the project's
real knowledge base **and** a self-documenting demo: it explains how to use and run mdkb *using*
mdkb. Opening it *is* the tutorial; the run-guides all embed one shared note, so editing that
block once updates every guide (live transclusion). The human-facing docs in this repo are
**generated** from these blocks (see *Docs are generated* below).

```sh
# point the daemon at it (or set the desktop app's Settings → Local vault to this folder)
cargo run -p mdkbd -- --vault vault
```

### Docs are generated (docs-as-data)

Some human-facing docs in this repo are **generated from blocks in `vault/`** — the vault is the
single source of truth, and `mdkb export` renders chosen blocks to flat Markdown. This keeps the
embed-once/reflect-everywhere property for documentation: shared knowledge (e.g. the
embed-vs-reference rules) lives in one block that several docs transclude.

- `vault/export.toml` maps each generated file to its source block — one `[[doc]]` per file:

  ```toml
  [[doc]]
  path  = "docs/skills/mdkb-knowledge/SKILL.md"
  block = "MCP skill page"
  ```
- Generated files carry a `<!-- @generated by mdkb … -->` banner — **don't hand-edit them**;
  edit the source block in `vault/` and re-run export.
- Regenerate everything, or check for drift (CI/pre-commit gate, non-zero exit on drift):

```sh
cargo run -p mdkb-cli -- export vault            # regenerate the mapped docs
cargo run -p mdkb-cli -- export vault --check    # verify they're current
```

There are three ways to select what to export:

- **A manifest** (`vault/export.toml`, or `--manifest=<path>`) — a TOML list of `[[doc]]` entries,
  each pairing an output `path` with the `block` (ULID or title) that fills it, for docs that need a
  specific location (e.g. the skills under `docs/skills/…`). An optional `[defaults]` table sets the
  banner policy for every doc (`raw = true|false`); a per-entry `raw` overrides it. A manifest entry
  is the on-disk twin of the options an export command computes — *one command ≈ one `[[doc]]`*. A
  `--manifest=<path>.json` file with the same schema is also accepted.
- **`--tag=<name>`** — export the **root** blocks carrying that tag to `<slug>.md` (under `--root`,
  default `docs-export/`). A page is just a root, so `--tag=doc` exports the pages tagged `doc`
  without also emitting their transcluded section blocks. Add `--include-non-root` to export
  *every* block with the tag.
- **No selector** — dump every root block to `<slug>.md` (the whole-KB case).

`--raw` omits the `@generated` banner (for portable, off-repo output); `--check` writes nothing and
exits non-zero on drift (the CI/pre-commit gate).

Within a single export, the chosen docs **cross-link**: a `[[reference]]` from one exported doc to
another renders as a real relative Markdown link between their files (instead of inert plain text).
A `[[reference]]` to a block that is **not** in the export degrades to plain text and prints a
`warning:` naming the dropped link — unless **`--follow-links`** is given, which pulls every
explicitly-linked block into the export (transitively) so nothing is left dangling. (`--follow-links`
applies to the `--tag`/whole-KB modes; a manifest's paths are explicit, so it warns instead.)

The CLI/MCP skills were the first docs generated this way: the `mdkb-cli` and `mdkb-knowledge`
skills share the same knowledge blocks and differ only in their per-transport surface, so editing
a shared block once updates both. **[`docs/SPEC.md`](./docs/SPEC.md)** is also generated — from a
`SPEC page` block that transcludes one block per section — so the on-disk-format reference is itself
kept in the vault it describes.

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
- `mdkb-embed` — `Embedder` backends: the offline deterministic `HashEmbedder` and a bundled
  INT8 ONNX model (no runtime download); configurable per vault.
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
  `update`, `set-tags`, `link`, `carve`, `delete`. Maintenance: `rebuild`, `export`.
- `mdkb-mcp` — an MCP server (JSON-RPC 2.0 over stdio) exposing the knowledge base as tools
  (`search`, `get_block`, `render_block`, `list_blocks`, `list_roots`, `graph`, `list_tags`,
  `backlinks`, `links_from`, `create_block`, `update_block`, `set_tags`, `delete_block`,
  `carve_block`, `link_blocks`, `stats`, `rebuild`, `conflicts`). A thin client that forwards
  every call to the daemon and auto-starts `mdkbd` if needed.

## Usage (current)

```sh
# Every CLI command takes a vault dir and auto-starts (then reuses) that vault's daemon.
# The daemon owns the one warm index and is the single writer.

# reads
cargo run -p mdkb-cli -- list ./my-vault                     # root blocks: id  title
cargo run -p mdkb-cli -- search ./my-vault "how do I restart nginx"
cargo run -p mdkb-cli -- search ./my-vault kusto --lang=kusto
cargo run -p mdkb-cli -- search ./my-vault "ops" --tag=ops --limit=10
cargo run -p mdkb-cli -- render ./my-vault <block-id>        # children inlined
cargo run -p mdkb-cli -- tags ./my-vault
cargo run -p mdkb-cli -- stats ./my-vault

# writes (body via stdin where shown)
echo "# Note" | cargo run -p mdkb-cli -- create ./my-vault --title="Note"   # prints new id
cargo run -p mdkb-cli -- set-tags ./my-vault <id> ops kusto
cargo run -p mdkb-cli -- link ./my-vault <src> <dst> --embed
```

### Running the daemon

`mdkbd` owns the index and a file watcher, and serves a local socket (Unix socket / Windows named pipe). Markdown is the
source of truth; the index lives in `<vault>/.mdkb/` and is local-only (exclude it from
cloud sync).

```sh
# start the daemon (defaults: vault ~/mdkb-vault, socket <vault>/.mdkb/mdkbd.sock)
cargo run -p mdkbd -- --vault ./my-vault

# from another shell, the CLI auto-connects to (and would auto-start) that vault's daemon
cargo run -p mdkb-cli -- ping ./my-vault
cargo run -p mdkb-cli -- stats ./my-vault
cargo run -p mdkb-cli -- search ./my-vault "restart the web server"
```

By default the offline hash embedder is used (deterministic, no downloads). For real
semantic embeddings via a local ONNX model, run the **daemon** with the `onnx` feature (the
daemon owns embedding; the CLI is a thin client and needs no embedder):

```sh
cargo run -p mdkbd --features onnx -- --vault ./my-vault
```

#### Choosing an embedder (`config.json`)

The embedder backend is configurable per vault via an optional `<vault>/.mdkb/config.json`.
The model is **never downloaded at runtime** — local models are loaded from disk, and the
shipped container image bakes the model in. The `embedder` block selects the source:

```jsonc
// 1. default / file absent → bundled vendored model (the container ships one); falls back
//    to the offline hash embedder if no bundled model is present (e.g. a plain `cargo run`).
{ "embedder": { "kind": "bundled" } }

// 2. a different ONNX model directory on disk (ONNX weights + tokenizer files)
{ "embedder": { "kind": "local", "path": "/models/bge-large", "dim": 1024 } }

// 3. a remote OpenAI-compatible /v1/embeddings endpoint (vLLM, LM Studio, llama.cpp, TEI,
//    OpenAI). Build the daemon with the `remote` feature. The API key, if any, is read from
//    the named environment variable so it never lives in config.json.
{ "embedder": { "kind": "remote", "url": "http://vllm:8000/v1/embeddings",
                "model": "bge-m3", "api_key_env": "MDKB_EMBED_KEY", "dim": 1024 } }

// 4. force the offline, dependency-free hash embedder (no model)
{ "embedder": { "kind": "hash" } }
```

The bundled model directory is resolved from `$MDKB_BUNDLED_MODEL_DIR`, else a `model/`
directory beside the binary. Any misconfiguration (missing model, unreachable endpoint)
logs a warning and falls back to the hash embedder, so the tool always keeps working.
The `onnx` (local models) and `remote` (HTTP endpoint) backends are opt-in cargo features;
default builds pull neither and stay fully offline.

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

For guidance on using mdkb *well* as an AI client — the DRY/transclusion principle, the process
for adding knowledge, and effective search patterns — see the example skill at
[`docs/skills/mdkb-knowledge/SKILL.md`](./docs/skills/mdkb-knowledge/SKILL.md).

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

- **Desktop shell** (`app/mdkb-tauri`) — a Tauri app over the same crates. It is a full
  **editor and graph browser**, not just a viewer:
  - **Read / Edit** per block — read with children (transclusions) resolved into embed cards
    and references shown as chips, or edit the block's title + Markdown body.
  - **Inline editing** — click a block's rendered content (or an embed card's content) to edit
    that block in place; type `[[` for a **link/embed picker** that searches blocks by title
    (Enter inserts a `[[ref]]`, Tab toggles to a `![[embed]]`; no match offers to create one).
  - **New block / Add block / Carve / Delete** — create a top-level block, **add** a child to
    the current block, **carve** a text selection out of a block into its own reusable block
    (replaced in place by an embed, non-destructive), or delete a block.
  - **Knowledge graph** — a force-directed block graph (nodes = blocks sized by link degree,
    roots highlighted; edges = `[[refs]]` / `![[transclusions]]`); click a node to open it,
    hover to highlight neighbours. Computed in `mdkb-core` (`link_graph`), rendered with a
    vendored offline `force-graph`.
  - **Linked references** — every block lists the blocks that link to or embed it.
  - **Settings** (no env vars) — choose a **Local vault** (the app auto-starts a background
    `mdkbd` for that folder and reuses it; the daemon keeps running after you quit) or a
    **Remote daemon** (`host:port` + token). Saved to a per-user config file and applied
    without restarting the app. The connection status dot in the sidebar opens Settings.

  Lives in its own workspace (needs the Tauri toolchain); see
  [`app/mdkb-tauri/README.md`](./app/mdkb-tauri/README.md).

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
  protocol, `mdkbd` with a local-socket server and `notify` file watcher.
- **Phase 5 — MCP server** *(done)*: `mdkb-mcp` exposes search / get / render / upsert /
  link / stats as MCP tools over stdio; thin client of the daemon.
- **Phase 6 — Frontends** *(done)*: shared `mdkb-view` (Markdown→HTML), runnable
  `mdkb-web` local UI, and a `app/mdkb-tauri` desktop shell over the same view layer.
- **Phase 7 — Sync UX & packaging** *(done)*: cloud-sync conflict detection (surfaced, not
  indexed), index `rebuild`, token-gated TCP transport for cluster deploy, Dockerfile + k8s
  manifest + example MCP config (`deploy/`).

### Follow-ups / known gaps

- **Windows desktop app — observability** *(planned)*: the Tauri shell runs in the Windows
  `windows` subsystem (no console), and its diagnostics are best-effort stderr writes that go
  nowhere in a GUI launch. `tauri::Builder::run()` still ends in `.expect(...)`, so a genuine
  WebView2 init failure would panic **silently and undiagnosably**. Add structured logging to
  a rolling file in the app-data dir (`tracing` + `tracing-subscriber` + `tracing-appender`),
  install a panic hook that records to the same log, and replace the `.expect` with a logged
  graceful exit.
  - *Investigation note:* a "window flashes then disappears" symptom on Windows was reproduced
    only by a pathological harness (force-killing the app + daemon + webview every ~2s, which
    races the shared WebView2 profile lock at `%LOCALAPPDATA%\dev.mdkb.desktop\EBWebView`).
    Normal launches, and relaunch-after-crash, were reliable in testing. The file log above is
    what would let us confirm/deny this in the wild rather than theorize.
- **Knowledge graph — distinguish transclusions from references** *(planned)*: the graph
  currently collapses `[[refs]]` and `![[transclusions]]` into one undifferentiated edge type.
  Tag each edge with its kind in `mdkb-core` (`link_graph`) so the two are distinguishable in
  the data, then render them differently in the UI (e.g. solid edges for `![[transclusions]]`,
  dashed for `[[refs]]`) so a reused/embedded block reads visibly different from a plain link.

## Deployment

See [`deploy/README.md`](./deploy/README.md). In short: run `mdkbd --vault <dir>` locally,
or deploy the daemon to k3s/Kubernetes as a single writer (`replicas: 1`) serving a
token-gated TCP API (`deploy/k8s.yaml`, `Dockerfile`). Sync only the Markdown vault across
machines; each daemon keeps its own local, rebuildable index.

## License

Dual-licensed under MIT or Apache-2.0.
