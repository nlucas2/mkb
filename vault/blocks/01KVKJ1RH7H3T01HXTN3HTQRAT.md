---
title: "README: Roadmap"
tags: [doc, readme]
updated: 2026-06-29T07:00:32Z
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

- **Validate the Windows-native `justfile`** *(planned)*: `just` runs recipe lines with `sh`, which
  Windows lacks, and several recipes used bash-only constructs (`uname`/`case`/`osascript`) and Unix
  coreutils (`mkdir -p`/`cp`). The justfile now sets `windows-shell` to PowerShell for the plain
  `cargo` recipes and ships `[windows]` variants of `install` / `app` / `icons` / `app-dev` (the
  macOS/Linux recipes are unchanged), but that path was **written from macOS and has not run on a
  Windows host**. Verify on Windows that `just install` builds the bundles and launches the NSIS
  `*-setup.exe`, that the staged `bin\*.exe` names match the Tauri `resources` globs (`bin/mkbd*`,
  `bin/mkb-mcp*`, `bin/mkb-cli*`), and that the plain recipes run under `powershell.exe`; fix path
  separators / quoting as needed.
