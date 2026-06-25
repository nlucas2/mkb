---
title: "Architecture: non-negotiables"
tags: [doc, architecture]
---

## Non-negotiables

These hold throughout the design:

- **Files on disk are the single source of truth.** The index is a rebuildable cache, never
  authoritative.
- **One shared core.** All block / transclusion / index / search / parsing / write behavior lives
  in `mdkb-core` and is reached through the daemon; the CLI, MCP server, and desktop app
  are thin clients, so a bug fixed once is fixed everywhere. (Enforced by the contributing rules
  in [`AGENTS.md`](../AGENTS.md).)
- **Pluggable seams are traits** (`Index`, `Embedder`, `IdCodec`, transport) — program to the
  trait, not the concrete type, so engines/encodings can be swapped without touching callers.
