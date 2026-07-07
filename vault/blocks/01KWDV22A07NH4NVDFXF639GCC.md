---
title: "Docs-as-data: a block can own its output"
tags: [skill/docs-as-data]
updated: 2026-07-07T06:08:59Z
---

### Add a new generated doc

Give a block a `path` property (the output directory) and optionally `filename` (the output file
name; defaults to the title slug + `.md`) — `mkb export` automatically finds and exports it.
Remove `path` to stop generating a doc; the file becomes hand-maintained again.

For the full mechanics — the worked example, the `export.toml` override manifest, cross-document
link rewriting, and ad-hoc export flags — see [[Exporting and publishing]] rather than duplicating
them here.