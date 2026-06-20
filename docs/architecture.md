# mdkb architecture — the file-per-block model

> Status: **target architecture** for the Phase‑10 re‑architecture. This supersedes the
> earlier "flat blocks parsed out of a page file" model. Everything below is the design we
> are building toward; where the current code still uses the old model, treat this document
> as the source of intent.

---

## 1. What mdkb is

A **personal Markdown knowledge base that is a tool, not an agent.** Two equal first‑class
consumers:

1. **A human**, via a desktop app (Tauri) that renders the knowledge cleanly and lets them
   read, edit, compose, and navigate it.
2. **AI clients**, via an MCP server, that search, read, write, and *refactor* knowledge
   through deterministic tools.

The store works fully with all AI turned off. It is a **tool**: agentic behavior lives in
clients, never in the store.

**Non‑negotiables**

- **Files on disk are the single source of truth.** The index is a rebuildable cache, never
  authoritative.
- **One shared core.** All behavior that touches blocks, transclusion, indexing, search,
  parsing, or writes lives in `mdkb-core` and is reached through the daemon/core API. The MCP
  server, the Tauri UI, the web UI, and the CLI are **thin clients** — transport/presentation
  glue only. A bug fixed once is fixed everywhere (see `AGENTS.md`).
- **Pluggable seams are traits** (`Index`, `Embedder`, `IdCodec`, transport). Program to the
  trait, not the concrete type.

**Storage rationale (why files at all):** so the vault is portable and **sync‑friendly**
(OneDrive / Dropbox / iCloud / git). Raw‑file *human* legibility is explicitly **not** a
goal — we can add an export tool for that. The only hard requirement is that a raw directory
can be turned back into knowledge by a future reader (recoverability), which a documented
on‑disk format guarantees.

---

## 2. The core idea: **a block is a file**

The primitive unit of knowledge is a **block**. A block is **an author‑chosen span of
content** — *not* a Markdown element. It can be a single paragraph, or a paragraph + a code
block + a list, whatever is the right primitive *at the time*. Markdown is only the
*formatting of the content inside a block*; it carries **no structural meaning**.

> **block = page = file.** One concept. A "page" is just a block you happen to open at the
> top. A block that nothing embeds is a top‑level entry. Promoting a chunk to its own page is
> the same operation as carving out a sub‑block — they are identical.

### 2.1 On disk

```
<vault>/
  blocks/
    01J8Z3K9P0QH7W2RE5N6T4XYAB.md      # one block = one file
    01J8Z3M1A2B3C4D5E6F7G8H9JK.md
    ...
  .mdkb/                                # rebuildable cache + local config (never synced as truth)
    index.db
    config.json                         # embedder config (server-side)
  SPEC.md                               # on-disk format spec (recoverability)
```

A block file is **pure, clean Markdown** for its own content, optional YAML frontmatter for
metadata, and **child‑block embeds inline** wherever a child renders:

```markdown
---
title: Deploying to k3s
---

Our cluster runs k3s on the Pi rack. Deploys go through Forgejo CI.

![[01J8Z3M1A2B3C4D5E6F7G8H9JK]]

Once the manifests are in place, push to Forgejo and the pipeline applies them.
```

- **Identity = the ULID filename** (`blocks/<ulid>.md`). Stable: renaming the title or moving
  nothing — references never break because they resolve by ULID, not by path or title.
- **Title** lives in frontmatter (`title:`), so a file is self‑describing. Optional; a block
  need not be "a page."
- **`![[<ulid>]]`** inline = a **child** (transclusion). Its *position* in the file is the
  child's order within the parent. **`[[<ulid>]]`** = a **reference** (a link, no expansion).
- Flat directory, ULID‑named. No subdirectories (they buy only human browsing, which we don't
  optimize for, and make moves fragile). If `blocks/` ever grows too large for the filesystem
  or the sync client, shard mechanically by ULID prefix (`blocks/01/01J8….md`) — deferred
  until it's a real problem.

### 2.2 The model is a DAG

Blocks form a **directed acyclic graph** (for transclusion) over a directed graph (for
references):

- **Children = the blocks a block embeds** (`![[...]]`), in document order. So *structure*
  and *reuse* are the **same edge**. There is no separate "group", "section", or "nesting
  marker" concept — a block with children *is* a section; a leaf block has no children.
- **References (`[[...]]`)** are a second, independent edge type: navigation links that do
  **not** expand and **may be cyclic** (A links B, B links A is fine).
- Rendering a block = render its own content, expanding each child embed recursively.
- `![[<ulid>]]` of a block that itself has children pulls the **whole subtree**, live. Edit
  the one block file, every embedder reflects it. This is the "edit once, reflected
  everywhere" guarantee, and it is uniform across leaf → section → whole page.

---

## 3. Reference vs transclusion semantics

| Directive | Meaning | Expands at render? | May be cyclic? | Graph edge |
|---|---|---|---|---|
| `[[<ulid>]]` | reference / link | No (renders as a chip link) | **Yes** | `references` |
| `![[<ulid>]]` | transclusion / child | Yes (inlines the target subtree) | **No** | `transcludes` / `child_of` |

- A `[[ref]]` renders to a navigable `mdkb:<ulid>` link (a "wikilink chip" in the UI).
- A `![[embed]]` renders to the live content of the target (optionally framed as an embed
  card with a source‑link header), recursively.
- Optional display alias: `[[<ulid>|custom label]]`.

---

## 4. Resilience: cycles & corruption (hard requirements)

The store must never be brought down by a bad edge, a hand edit, a sync conflict, or a
corrupt file.

1. **Resolution is total.** Rendering never panics and never infinite‑loops. Transclusion is
   resolved by DFS with a visited set.
2. **Cycle → render up to it, then a note.** If an embed would re‑enter a block already on the
   current render stack, emit a **navigable link to the target + a visible `↻ cycle` note**
   instead of recursing. The rest of the page renders normally.
3. **Corruption degrades locally.** A missing / unreadable / malformed / dangling embed
   renders an inline **`⚠ unresolved` note + link**, never a crash; surrounding content is
   unaffected.
4. **Cycle *prevention* at write time is a downgrade, not a refusal.** Creating an embed
   `A ![[B]]` when `B` already transcludes its way back to `A` (or `B == A`) **silently writes
   a plain `[[reference]]` instead** and reports the downgrade (`LinkOutcome`). The link still
   gets made; it just won't recurse. **References are never cycle‑checked** — only embeds
   expand, so only embeds can loop.
5. **Health surface.** Detected cycles and dangling/corrupt refs are reported in a lint/health
   view, never fatal.

Reachability for prevention follows **embed edges only** (`transclusion_reaches`).

---

## 5. The index is a rebuildable cache

A SQLite index (`mdkb-index`, behind the `Index` trait) caches everything for fast reads. It
is **always reconstructable** by scanning `blocks/`:

- a row per block (id, title, content, kind metadata, tags, lineage/breadcrumb, embedding);
- the link table (`transcludes | references | child_of`) derived from each file's directives;
- FTS5 (porter + unicode61, bm25) for keyword search;
- a vector store (per‑block embedding, tagged with the model id) for semantic search.

Search fuses keyword + semantic via **Reciprocal Rank Fusion (RRF)**. The index is never the
source of truth; on any doubt, rebuild from files.

---

## 6. Daemon & clients

```
        ┌───────────── thin clients (transport/presentation only) ─────────────┐
        │   mdkb-cli      mdkb-mcp (MCP)     mdkb-web (HTTP)     mdkb-tauri (app) │
        └───────────────────────────────┬──────────────────────────────────────┘
                                         │  mdkb-protocol (JSON over local socket / TCP+token)
                                ┌────────▼────────┐
                                │      mdkbd       │  single writer: owns watcher + index + writes
                                │  mdkb-core::Service (capability‑gated dispatch)
                                └────────┬────────┘
                          ┌──────────────┼───────────────┐
                       mdkb-core     mdkb-index        mdkb-embed
                    (block model,    (SQLite+FTS5+    (Embedder trait,
                     DAG, render,     vectors, RRF)    bundled/local/remote)
                     tags, search)
```

- **One daemon = one writer** over a vault. Local mode: a Unix socket (Windows named pipe),
  fail‑closed (no network). Remote mode: TCP + shared token, capability‑gated (remote callers
  are read/write only as authorized; default fail‑closed).
- Clients **auto‑start a detached daemon** for a local vault (it outlives the app) or connect
  to a remote one. Connection config is shared (`ConnectionConfig` / `connect` /
  `ensure_daemon` in `mdkb-protocol`). A client reuses a live daemon by pinging its socket and
  only spawns one when none answers.
- **One daemon per vault.** The daemon holds an exclusive advisory lock on `.mdkb/mdkbd.lock`
  for its lifetime (released by the OS on exit, so it can't go stale). A second daemon for the
  same vault refuses to start. This holds even if the socket file is deleted out from under a
  running daemon, so a vault can never have two concurrent writers/watchers.
- **Idle self-shutdown (no leaked daemons).** A client-auto-started daemon self-reaps after an
  idle period (`--idle-timeout`, default 15 min) so an unused vault doesn't leave a process (and
  its embedder RAM) resident; any request defers it. Manually-run and remote daemons run forever.
  Clients self-heal — the next interaction respawns an idled-out/crashed local daemon — so this
  is invisible beyond a brief cold start.
- **Presentation is shared** via `mdkb-view` (Markdown→HTML, wikilink/embed decoration, XSS
  neutralization). The web UI and desktop UI render through the exact same path.

---

## 7. Recoverability & export

- **`SPEC.md`** committed at the vault root documents the on‑disk format precisely, so a raw
  `blocks/` directory can be turned back into knowledge with no app: read each file, parse its
  frontmatter + `![[...]]` / `[[...]]` directives, rebuild the DAG.
- **Roadmap — full KB export:** an optional tool that flattens the whole DAG into standalone,
  fully‑inlined Markdown files (every transclusion expanded), for handing the knowledge to
  something that doesn't understand mdkb. Not required for normal operation.

---

## 8. Feature‑parity checklist (must survive the re‑architecture)

Everything below exists in the pre‑re‑architecture codebase and **must be re‑satisfied** in
the file‑per‑block paradigm. Do not let any of these silently disappear.

### Block model & content
- [ ] Stable per‑block **ULID** identity (now = the filename).
- [ ] **Frontmatter title** per block (self‑describing files).
- [ ] **Tags** (`#tag`) — extraction, storage, indexing, and tag search. *(Easy to forget.)*
- [ ] **Heading lineage / breadcrumb** context for a block — used both for display and as
      embedding/search **contextual_text** (lineage‑prepended text). *(Easy to forget; matters
      for search quality.)*
- [ ] **Fenced‑code language** capture (for highlighting and lang‑filtered search).
- [ ] Block **kind** metadata where still meaningful (heading/paragraph/code/quote/list/html).
      *(Re‑evaluate: in the new model a block is author‑chosen, but kind is still useful for
      rendering hints and search facets.)*

### References & transclusion
- [ ] `[[ref]]` references and `![[embed]]` transclusions, resolved **by ULID**.
- [ ] Optional **display alias** `[[id|label]]`.
- [ ] **Whole‑subtree embed** (a block with children pulls its subtree).
- [ ] **`mdkb:` link scheme** + UI decoration (wikilink chips, embed cards),
      `rewrite_mdkb_links` for the web UI.
- [ ] **Total cycle‑safe resolver** (render‑up‑to‑cycle + note).
- [ ] **Dangling/corrupt embed → local degrade** (unresolved note, page still renders).
- [ ] **Write‑time cycle handling = downgrade to reference** (`LinkOutcome`,
      `transclusion_reaches`), embeds only.
- [ ] **Backlinks / linked‑references** per block.

### Index & search
- [ ] SQLite index as a **rebuildable cache** (scan `blocks/` → reindex).
- [ ] **FTS5** keyword search (porter + unicode61, bm25), **OR‑join** query building.
- [ ] **Tag search** and **lang search**.
- [ ] **Semantic / vector search**, **RRF fusion** of keyword + vector.
- [ ] **Model‑id‑tagged vectors** (never compare across embedding spaces).
- [ ] **IndexStats** (pages/blocks/embedded counts).
- [ ] **Page‑level (now block‑level) knowledge graph** (`link_graph` → `GraphData`).

### Embeddings
- [ ] **Embedder trait** + sources: **bundled / local / remote(OpenAI‑compatible) / hash**.
- [ ] Embedder config at **`<vault>/.mdkb/config.json`**.
- [ ] Bundled **INT8 bge‑small** model, baked in (no runtime download).
- [ ] **contextual_text** (lineage‑prepended) used for embeddings. *(Easy to forget.)*

### Sync engine
- [ ] **reconcile** + **rebuild**; **file watcher** (notify) keeps the index live.
- [ ] **Content‑hash skip‑unchanged**.
- [ ] **id assignment on ingest** (now: assign a ULID filename to a new block).
- [ ] **force_write / save** semantics so edits reach the SSOT (the bug we just fixed —
      keep its regression test's intent).
- [ ] **Cloud‑sync conflict file detection** (surface conflict copies, never index them).
      *(Especially relevant now: file‑per‑block makes OneDrive conflicts rarer but still
      possible.)* *(Easy to forget.)*
- [ ] **Path‑traversal confinement** (`safe_relative_path`) on every fs write/delete.

### Daemon, protocol, service
- [ ] **CLI flags** (`--vault/--db/--socket/--listen/--token`), help/usage.
- [ ] **Local socket / Windows named pipe** server, **fail‑closed** (no network by default).
- [ ] **TCP listen + token auth**; **line‑length cap** + **read timeout** (pre‑auth DoS guard).
- [ ] **RequestContext + Capability gate** (Read/Write; remote fail‑closed).
- [ ] Every **Request/Response** variant + **Client** convenience methods.
- [ ] **ConnectionConfig / connect / ensure_daemon** with **detached** daemon spawn (outlives
      the app); `from_env`; `DaemonPaths`.
- [ ] **Socket 0600 / `.mdkb` dir 0700** perms.

### MCP server
- [ ] Every MCP **tool** (search/get/render/links/stats/write/link/…); maps to Requests.
- [ ] **Daemon auto‑start** for the given vault.

### CLI
- [ ] Subcommands: **render, search, stats**, and the **daemon** subcommand
      (ping/stats/list/search/render/…).

### Web UI
- [ ] Routes (page list, page render, search), shared **mdkb-view** rendering.
- [ ] **HTML neutralization (stored‑XSS guard)**, per‑connection **threads + read timeout +
      header caps**.

### Tauri desktop app
- [ ] **Read / Blocks / Raw** per page; **block‑level editing** (edit a block in place).
- [ ] **append / save / delete**; **new page**.
- [ ] **Knowledge‑graph view** (vendored `force-graph`, offline), click‑to‑navigate, hover
      highlight.
- [ ] **Linked‑references panel**.
- [ ] **Settings** (no env): **Local vault** (auto‑start detached bundled daemon, survives
      quit) / **Remote** (host + token); **folder picker**; reconnect without restart.
- [ ] **Bundled `mdkbd`** resource for local auto‑start.
- [ ] Intercept **`mdkb:` link clicks** for navigation.

### Presentation (mdkb-view)
- [ ] `markdown_to_html` with **raw‑HTML neutralization**.
- [ ] `page_title`, `search_results_html`.
- [ ] **wikilink / embed‑card decoration**, `rewrite_mdkb_links`.

### Security (audited; must persist)
- [ ] Path‑traversal confinement; stored‑XSS neutralization; TCP auth + caps; socket/dir
      perms; conflict heuristic (anchored, no false positives); model‑id vector isolation;
      non‑UTF‑8 file skipping (never lossily rewrite).

---

## 9. New behaviors introduced by this model (build these too)

- **Carve** — split a sub‑block out of a block: the carved content moves to a new
  `blocks/<ulid>.md` and is replaced *in place* by `![[<ulid>]]`. **Non‑destructive**: the
  rendered output is unchanged; the chunk "is just now its own block."
- **Promote / open as page** — any block can be opened as a top‑level page (it already is one,
  structurally). No conversion step.
- **Embed / reference picker** in the UI, cycle‑aware (auto‑downgrade with a note).

---

## 10. Build phasing (Phase 10)

1. **Core model** — `Block` = file; vault = DAG of block‑files; frontmatter; ULID filenames;
   `![[id]]` children + `[[id]]` references parsed from file content.
2. **Total, cycle/corruption‑safe resolver** (carry over render‑up‑to‑cycle + downgrade +
   `transclusion_reaches`).
3. **Index rebuild** from `blocks/` (FTS5, tags, lineage/contextual_text, vectors, RRF, graph).
4. **Sync** (watcher, hash skip, conflict detection, path confinement, id assignment).
5. **Protocol/daemon/service** parity (all Requests/Responses, capability gate, detached
   spawn).
6. **Clients** — CLI, MCP, web, Tauri — re‑wired to the new core; carve/embed affordances.
7. **SPEC.md** + **export tool (roadmap)**.

Every step ships with tests in the same change; `cargo test --workspace` green before every
commit; README updated; no duplicated logic (shared behavior in `mdkb-core`).
