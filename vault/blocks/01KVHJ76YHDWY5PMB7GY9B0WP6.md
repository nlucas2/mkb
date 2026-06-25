---
title: "Shared note: vault & connection"
tags: [mkb, config]
---

**Shared note — the vault is the source of truth.** *(Edit this one block; everything that
embeds it updates at once.)*

- A vault is a directory of `blocks/<id>.md` files — the **only** thing meant to sync. The
  index/socket/lock/log are a machine-local, rebuildable cache that lives **outside** the vault
  (under the OS local-data dir, or `$MKB_INDEX_DIR`), so a cloud-synced vault never syncs them.
- The daemon serves a local socket by default. To connect over the network, run it
  with `--listen <host:port> --token <token>` and point clients at that.
- Clients (CLI, MCP, desktop app) never write files directly — they go through the
  daemon, the single writer.
