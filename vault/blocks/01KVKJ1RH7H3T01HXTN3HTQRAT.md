---
title: "README: Roadmap"
tags: [doc, readme]
updated: 2026-06-25T10:10:14Z
---

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
- **Phase 6 — Frontends** *(done)*: shared `mdkb-view` (Markdown→HTML) and a `app/mdkb-tauri`
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
    races the shared WebView2 profile lock at `%LOCALAPPDATA%\dev.mdkb.desktop\EBWebView`).
    Normal launches, and relaunch-after-crash, were reliable in testing. The file log above is
    what would let us confirm/deny this in the wild rather than theorize.
- **Knowledge graph — distinguish transclusions from references** *(planned)*: the graph
  currently collapses `[[refs]]` and `![[transclusions]]` into one undifferentiated edge type.
  Tag each edge with its kind in `mdkb-core` (`link_graph`) so the two are distinguishable in
  the data, then render them differently in the UI (e.g. solid edges for `![[transclusions]]`,
  dashed for `[[refs]]`) so a reused/embedded block reads visibly different from a plain link.
- **Desktop app — light theme** *(planned)*: the app currently ships a single dark theme. Add a
  light theme and a theme toggle (follow the OS appearance by default), so the editor, graph, and
  block cards read well on a light background. Until then, the README screenshots are dark-only.
- **Limited inline HTML rendering** *(planned)*: `mdkb-view` currently neutralises **all** raw HTML
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
- **Block-display view (`mdkb show` / `show_block`)** *(planned)*: a page-as-a-human-sees-it
  read — breadcrumb lineage upward, rendered children downward, backlinks, and metadata in one
  call — distinct from `get` (raw body) and `render` (children only). Builds on the search
  lineage already in `SearchHit`.
- **More partial-edit primitives** *(planned)*: `replace_in_block` (exact string swap) exists,
  but there is still no `append_to_block` (add to the end) or `insert`/line-range edit, nor a
  line-range source view — so adding a line at the end of a block still means anchoring on its
  current last line. Add an append op (the engine already has an internal `append_to_body`) and
  a `get_block_source_range`.
- **Opt-in root-biased search** *(planned)*: `--roots-only` and `--root-bias <w>` as post-fusion
  knobs in the service (never the default, never inside RRF), for navigational queries that want
  the page rather than its embedded fragments.
- **`mdkb daemon restart` / `stop` (CLI)** *(planned)*: only the desktop app can currently
  restart the local daemon (Settings -> Restart daemon); from the CLI there is no way to replace a
  running detached daemon, so after rebuilding `mdkbd` a stale daemon (e.g. the one bundled in the
  installed app, which owns the vault socket) keeps serving the old binary and new requests fail
  with "unknown variant". Add a `mdkb daemon restart`/`stop` command (shut down + let the next
  client respawn) to fix the dev loop without the GUI.

- **Concurrent-edit safety — the app's full-overwrite write is a lost-update vector** *(planned)*:
  the desktop app reads a block into its editor and saves with a **whole-body overwrite**
  (`save_block`), with no check that the block is unchanged since it was read. If an AI client (or
  the CLI) edits that block via MCP between the app opening it and the human saving, the app's save
  silently **clobbers** the external edits — the classic lost-update problem, made likely by mdkb's
  whole premise of a human and an AI co-editing one vault. The rendered body is *not* itself stale
  (the app re-fetches on navigation/reload) and the daemon remains the single writer, so this is a
  *concerning surface, not an active bug*. Close it with **optimistic concurrency**: stamp each read
  with the block's `updated` time (or a content hash) and have the daemon reject a write whose base
  no longer matches, surfacing the clash the way cloud-sync conflicts already are — and/or push
  change events from the daemon's `notify` watcher so clients invalidate their in-memory caches
  (sidebar list, graph, link previews — none of which currently refresh on an out-of-band edit) and
  reflect co-edits live.
