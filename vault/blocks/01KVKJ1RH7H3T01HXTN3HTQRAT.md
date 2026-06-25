---
title: "README: Roadmap"
tags: [doc, readme]
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
