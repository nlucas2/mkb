---
title: Shared note: vault & connection
tags: [mdkb, config]
---

**Shared note — the vault is the source of truth.** *(Edit this one block; every
run-guide below updates at once.)*

- A vault is a directory of `blocks/<id>.md` files; the index in `.mdkb/` is a
  rebuildable cache.
- The daemon serves a local socket by default. To connect over the network, run it
  with `--listen <host:port> --token <token>` and point clients at that.
- Clients (CLI, MCP, web, desktop) never write files directly — they go through the
  daemon, the single writer.
