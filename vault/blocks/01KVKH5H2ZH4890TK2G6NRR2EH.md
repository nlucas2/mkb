---
title: "Rule: One shared core"
tags: [doc, contributing, concept]
---

### One shared core — the UI and the MCP server must never diverge ⚠️ critical

- **All** behavior that touches blocks, transclusion, indexing, search, parsing, or writes
  **MUST** live in `mkb-core` and be invoked through the daemon/core API.
- The **MCP server, the Tauri UI, and the CLI are thin clients.** They contain
  presentation and transport glue only — **never** a second copy of core behavior.
- Rationale: if the same surface is implemented twice, a bug can be fixed in one and left
  broken in the other. A bug fixed once must be fixed everywhere. If you find yourself about
  to implement the same thing in two clients, **stop** and put it in core.
- When you add a capability, expose it from core first, then wire the clients to that single
  entry point.
