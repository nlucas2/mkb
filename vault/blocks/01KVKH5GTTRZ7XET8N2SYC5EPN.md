---
title: What mdkb is
tags: [doc, concept]
---

A **personal Markdown knowledge base that is a tool, not an agent.** It serves two equal
consumers: **you**, the human, via a desktop app that renders the knowledge cleanly with live
transclusion (edit a block once, every block that embeds it reflects the change); and **AI
clients** (e.g. GitHub Copilot) via an MCP server exposing deterministic search / write / link
tools. The store works fully with all AI turned off — agents are clients, never part of the
store. Markdown files on disk are the single source of truth; the index is a rebuildable cache,
never authoritative.
