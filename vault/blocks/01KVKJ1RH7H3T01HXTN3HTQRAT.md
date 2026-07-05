---
title: "README: Roadmap"
tags: [doc/readme]
updated: 2026-07-05T08:40:00Z
---

## Roadmap

- **Phase 0 — Scaffold** *(done)*: workspace, crates, governance docs.
- **Phase 1 — Core SSOT (no AI)** *(done)*: Markdown parser, block model (incl. code-fence
  `lang`), eager block-id assignment, transclusion/reference resolver, `#tag` + frontmatter
  extraction, CLI render.
- **Phase 2 — Index + watcher** *(done)*: SQLite (FTS5) index, keyword + tag/lang-filtered
  search, `SyncEngine` reconcile. *(Live `notify` event loop lands with the daemon.)*
- **Phase 3 — Semantic search** *(done)*: local embeddings (`mkb-embed`: offline hash +
  optional `fastembed` ONNX), vector storage, hybrid keyword+vector ranking.
- **Phase 4 — Daemon + API** *(done)*: shared `Service` API + `RequestContext`, JSON wire
  protocol, `mkbd` with a local-socket server and `notify` file watcher.
- **Phase 5 — MCP server** *(done)*: `mkb-mcp` exposes search / get / render / upsert /
  link / stats as MCP tools over stdio; thin client of the daemon.
- **Phase 6 — Frontends** *(done)*: shared `mkb-view` (Markdown→HTML) and a `app/mkb-tauri`
  desktop shell over that view layer.
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
    races the shared WebView2 profile lock at `%LOCALAPPDATA%\dev.mkb.desktop\EBWebView`).
    Normal launches, and relaunch-after-crash, were reliable in testing. The file log above is
    what would let us confirm/deny this in the wild rather than theorize.
- **Knowledge graph — distinguish transclusions from references** *(planned)*: the graph
  currently collapses `[[refs]]` and `![[transclusions]]` into one undifferentiated edge type.
  Tag each edge with its kind in `mkb-core` (`link_graph`) so the two are distinguishable in
  the data, then render them differently in the UI (e.g. solid edges for `![[transclusions]]`,
  dashed for `[[refs]]`) so a reused/embedded block reads visibly different from a plain link.
- **Desktop app — light theme** *(planned)*: the app currently ships a single dark theme. Add a
  light theme and a theme toggle (follow the OS appearance by default), so the editor, graph, and
  block cards read well on a light background. Until then, the README screenshots are dark-only.
- **Limited inline HTML rendering** *(planned)*: `mkb-view` currently neutralises **all** raw HTML
  in a block (re-emitting it as escaped text) to close the stored-XSS vector — safe, but it means
  hand-written layout (image grids, captions, `<details>`) shows as literal markup. Move to a
  GitHub-style **sanitize-by-allowlist** model: parse the HTML and keep a vetted set of tags and
  attributes (`table`/`tr`/`td`, `img`, `sub`/`sup`, `details`/`summary`, `a[href]`…) while
  stripping `script`/`style`/`on*=`/`javascript:` — e.g. via the `ammonia` crate. Since blocks are
  AI-writable, the allowlist must stay tight and is a deliberate security-posture change to record
  in `docs/SPEC.md`. Relative `<img src>` in raw HTML would need the same vault-relative→asset
  resolution (and external `<img>` the same inert-placeholder treatment) the Markdown image path
  already applies.
- **Search match provenance** *(planned)*: hybrid search fuses a keyword/phrase (bm25) list and a
  vector (semantic) list via reciprocal-rank fusion, but `reciprocal_rank_fusion` discards *which*
  list each hit came from — only the fused `score` survives on `SearchHit`. Preserve that signal:
  have fusion report per-result membership (keyword-only / vector-only / both) and add it to
  `SearchHit` (e.g. a `MatchSource` flag), then surface it in the clients — so a `"quoted phrase"`
  search visibly distinguishes an exact phrase/keyword hit from a result that only the semantic
  side returned. Useful for trusting precision queries and for debugging ranking.
- **Block-display view — CLI `mkb show`** *(core + MCP done; CLI planned)*: the
  page-as-a-human-sees-it read — breadcrumb lineage upward, rendered children downward, backlinks,
  and metadata in one call — now exists in core as `Service::page_view` and is exactly what the MCP
  `get_block` returns (it absorbed the old separate render / backlinks / links tools). The CLI still
  lacks a single equivalent (it has separate `get` / `render` / `backlinks` / `links`); add `mkb
  show` over the same `page_view` so the human CLI gets the same one-call page read.
- **Partial-edit primitives** *(mostly done)*: `replace_in_block` (exact string swap),
  `append_to_block` (add to the end), and a line-range source view (`get_block_source_range`,
  CLI `get --lines`) have all shipped across core/CLI/MCP. Remaining gap: a line-targeted `insert`
  edit (insert at a given line without an anchor) — lower priority now that replace + append cover
  most edits.
- **Opt-in root-biased search** *(planned)*: `--roots-only` and `--root-bias <w>` as post-fusion
  knobs in the service (never the default, never inside RRF), for navigational queries that want
  the page rather than its embedded fragments.
- **`mkb daemon restart` / `stop` (CLI)** *(planned)*: only the desktop app can currently
  restart the local daemon (Settings -> Restart daemon); from the CLI there is no way to replace a
  running detached daemon, so after rebuilding `mkbd` a stale daemon (e.g. the one bundled in the
  installed app, which owns the vault socket) keeps serving the old binary and new requests fail
  with "unknown variant". Add a `mkb daemon restart`/`stop` command (shut down + let the next
  client respawn) to fix the dev loop without the GUI.

- **Concurrent-edit safety — the app's full-overwrite write is a lost-update vector** *(shipped)*:
  the desktop app reads a block into its editor and saves with a **whole-body overwrite**
  (`save_block`), with no check that the block is unchanged since it was read. If an AI client (or
  the CLI) edits that block via MCP between the app opening it and the human saving, the app's save
  silently **clobbers** the external edits — the classic lost-update problem, made likely by mkb's
  whole premise of a human and an AI co-editing one vault. The rendered body is *not* itself stale
  (the app re-fetches on navigation/reload) and the daemon remains the single writer, so this is a
  *concerning surface, not an active bug*. Close it with **optimistic concurrency**: stamp each read
  with the block's `updated` time (or a content hash) and have the daemon reject a write whose base
  no longer matches, surfacing the clash the way cloud-sync conflicts already are — and/or push
  change events from the daemon's `notify` watcher so clients invalidate their in-memory caches
  (sidebar list, graph, link previews) and
  reflect co-edits live.
  - *Update — live-refresh shipped (Phase 1):* the daemon now advances a monotonic content
    **generation** on every change (a daemon-applied write, or a watcher-reconciled external edit),
    and the desktop app reads it on its existing lease heartbeat; when it moves, the app invalidates
    its caches and re-opens the current block, so an edit from the CLI, an MCP client, or another
    editor shows within a heartbeat.
  - *Update — lost-update prevention shipped (Phase 2):* each block now carries an opaque content
    **version** token; the editor captures it on open and `update_block` rejects a whole-body save
    whose base no longer matches, returning the current state so the app can reconcile. The desktop
    app surfaces this as a side-by-side resolver (keep mine / keep current / a confirm-before-save
    3-way **merge** preview), and live refresh is edit-aware (the sidebar updates mid-edit without
    discarding the draft).
  - *Update — daemon-pushed change events shipped (Phase 3):* the generation now carries a wait
    primitive (a condvar), and clients long-poll `WaitForChange{since}` — parked server-side until
    the vault changes, then woken sub-second, so a co-edit reflects in the app almost immediately
    instead of within the old 10s heartbeat. A restart resets the counter; clients compare with
    `!=` and reconnect on the dropped wait, so it reads as a change and refreshes. Old daemons
    reject the op, so the app falls back to the heartbeat poll. Concurrent-edit safety is complete.

- **Windows-native `justfile`** *(done)*: `just` runs recipe lines with `sh`, which Windows lacks,
  and several recipes used bash-only constructs (`uname`/`case`/`osascript`) and Unix coreutils
  (`mkdir -p`/`cp`). The justfile now sets `windows-shell` to PowerShell for the plain `cargo` recipes
  and ships `[windows]` variants of `install` / `app` / `icons` / `app-dev` (the macOS/Linux recipes
  are unchanged). Exercised on a Windows host: `just install` builds the bundles, the staged `bin\*.exe`
  names match the Tauri `resources` globs (`bin/mkbd*`, `bin/mkb-mcp*`, `bin/mkb-cli*`), and the plain
  recipes run under `powershell.exe`.

- **Approximate-nearest-neighbour (ANN) vector search** *(planned)*: semantic matching is currently
  an exact brute-force cosine scan over every stored embedding (`mkb-index`) — exact, dependency-free,
  and comfortably fast for everything mkb has been used for so far. At large scale the linear scan (and
  reading every embedding per query) will eventually dominate. The `Index` trait already isolates the
  vector engine, so an ANN path can be added behind it without touching callers. The open design choice
  is *how*: (a) an **adaptive** switch that keeps the exact scan below a block-count threshold and
  engages an ANN index above it — brute force staying the correctness oracle — versus (b) a
  config-selectable ANN backend; and *which* engine — an in-memory Rust index (e.g. a quantized/SIMD
  crate such as `turbovec`, rebuilt from the stored vectors and owned by the long-lived daemon) versus a
  persisted `sqlite-vec` (`vec0`) table in the same file. Decide when a real vault actually approaches the
  crossover, not before.
- **Web frontend over the shared app-core** *(possible future)*: the desktop app's UI operation logic
  (reads, writes, vault-registry ops) now lives in the transport-neutral **`mkb-app-core`** crate, with
  the Tauri commands as thin shims over it. That seam means a browser frontend — a small server exposing
  the same `mkb-app-core` operations over the daemon and rendering through the shared `mkb-view` — could
  be added without duplicating any core behaviour. Not built; noted because the boundary is already in
  place if a web UI is ever wanted.
- **Change tracking / audit + restore — git-aware, not git-owning** *(planned)*: mkb's founding promise
  is *auditable, refactorable* memory, but a **standalone** vault (a bare `blocks/` dir — e.g. a
  cluster/NFS vault or `~/mkb-vault`) keeps **no history**: a whole-body write overwrites the file and
  the prior content is gone. (A vault that lives *inside* a git repo — like this repo's own `vault/` —
  already gets history for free from that repo; the gap is standalone vaults.) Guiding principle:
  **mkb never *owns* git.** It must not auto-`git init` or auto-commit — that would nest into / pollute
  an already-in-a-repo vault and fight the user's own commit workflow, and committing on the write path
  would tax the snappy single-writer hot path. Instead:
  - **Commits are owned by whoever owns the repo.** An embedded vault is committed by the user's existing
    workflow; a standalone vault opts into a lightweight external committer (timer/hook) — or, an open
    sub-decision, an mkb-provided **async, best-effort, off-by-default** auto-commit that never blocks a
    write.
  - **mkb is git-*aware* for reads.** Where the vault is a git repo, expose `history` / `diff` as
    **CLI + app diagnostics** — the same tier as `graph`/`stats`/`conflicts`/`rebuild`, deliberately
    **not** in the lean default MCP surface, so an agent's per-turn tool budget stays small. Where the
    vault isn't git-backed, these report "no history."
  - **`restore <block-id> [--from <ref>]` is a per-block forward-write.** Read that one block's file at a
    chosen commit (`git show <ref>:blocks/<id>.md`, a pure read) and write it **forward** as the new
    current state via the normal single-writer path — same ULID, history preserved (**not** a git
    reset/revert), and only that block re-embeds. It also **undeletes** a block whose file was removed.
    File-per-block makes single-block restore the natural granularity; whole-vault rollback stays
    raw-git territory for the human. Restore is a **human surface (CLI + app), kept out of the default
    MCP tools** — the thing writing your memory shouldn't be able to silently rewind it (same spirit as
    human-only locked blocks).
  - **UI: initialize history from the app.** A one-click "enable history for this vault" (git init + an
    initial commit) offered **only when the vault isn't already a git repo**, so a non-git user gets
    audit/restore without a terminal; an already-embedded vault is left untouched.
  - Open sub-decisions: whole-file restore (frontmatter + body as of that version) vs body-only; and
    whether standalone vaults get the opt-in async auto-commit, or mkb stays strictly read-only toward
    git.
- **Duplicate discovery & reconcile-on-write** *(partly shipped; the rest planned)*: mkb's promise is
  "each fact in exactly one block," but nothing helps an agent *find* that a fact is already stated —
  or scattered across several blocks — so duplication accumulates at scale (the existing `mkb-dedup`
  skill names the case but hands the agent no instrument beyond re-querying). The clarifying insight
  (validated against three independent model reviews) is that there are **three distinct operations**,
  not one, split by whether the input is a *query* or the *corpus*:
  - **Lookup** (query → block, read-time): "find X." Lean snippet search is correct here. *(shipped:
    match-snippet search.)*
  - **Write-time absence check** (query → block): "does this fact already exist before I create it?"
    Still search, but the cost of a miss is a *new duplicate*, so it wants an absolute sameness
    signal, not a query-relative rank. *(shipped: `search` surfaces `semantic_similarity` (the raw
    cosine, recovered before RRF fusion discards it) and `keyword_match` per hit, so an agent has
    both provenance and an absolute closeness read, then `get_block`s the top candidates to compare
    before writing.)* Remaining: a documented write-preflight workflow/skill ("search 2–3 phrasings
    of the proposed fact; if the close hits look like the same fact, `get_block` and reconcile
    instead of forking").
  - **Background audit** (corpus → corpus): "where does the vault duplicate *itself*?" There is no
    query, so no search change reaches it — this is the genuinely separate capability. *(planned.)*
    It bifurcates by granularity, which matters because whole-block embeddings can't see a small
    shared passage:
    - **Whole-block near-dupes** — two blocks that are mostly the same fact written twice. Cheap:
      top-k cosine over the vectors that already exist. A `similar_blocks`/dedup surface.
    - **Scattered-passage duplication** — the same paragraph (e.g. a "be DRY" principle) restated
      inside otherwise-different blocks (a C#, a Rust, a C++ guide…). Whole-block cosine **fails** here
      (each vector is dominated by its language-specific bulk; the shared passage washes out). Detect
      it at **passage granularity** — shingling / MinHash over spans (best cost/value; catches
      near-verbatim copy-paste), or passage/chunk embeddings (heavier; catches paraphrase). The precise
      target is **text spans that recur across blocks that are NOT already a shared `![[embed]]`** —
      recurrence that isn't reuse — which maps directly onto mkb's native remedy: carve the span into
      one block and embed it back everywhere.
  - Reconciliation stays **human-approved** (candidates surfaced, not auto-merged): some repetition is
    legitimately contextual, and over-eager merging into tiny generic blocks harms human browsing.
    Note: `search` returns full block bodies by default (body truncation is opt-in via
    `MKB_MCP_SEARCH_SNIPPET`), so a corpus audit sweep already has the text it needs.
