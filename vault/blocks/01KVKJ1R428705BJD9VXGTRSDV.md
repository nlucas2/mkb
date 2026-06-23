---
title: "README: Core principles"
tags: [doc, readme]
---

## Core principles

The design follows from a few rules, each stated once in its own block and reused everywhere:
**Markdown files are the source of truth** and the index is a **rebuildable cache** (above);
**block = file = page** with `![[embed]]` for live reuse and `[[ref]]` for links (the intro and
`docs/SPEC.md`); and **one shared core** behind thin clients (the Contributing rules below).

You and the AI **co-manage the same vault** — you in the desktop app (or the web UI for a
headless/remote daemon), the AI over MCP — and you decide what it may change: toggle a block
**🔒 human-only** from the app and AI clients can read it but never modify it.
