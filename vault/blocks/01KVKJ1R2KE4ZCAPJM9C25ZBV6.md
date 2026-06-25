---
title: "README: How clients reach the daemon"
tags: [doc, readme]
---

### How clients reach the daemon

The Markdown vault is the source of truth either way; what changes is *where the daemon runs* and
*how clients reach it*.

- **Local (default).** `mdkbd` owns the vault and a local socket — a **Unix-domain socket** on
  Linux/macOS, a **named pipe** on Windows — and the CLI, MCP server, and desktop app all
  connect to it. No configuration; clients auto-start it. To work across machines, **sync only the
  Markdown** (OneDrive, etc.) — each machine runs its own daemon and keeps its own local index.
- **Remote / shared.** One `mdkbd` runs centrally (e.g. a `replicas: 1` Deployment in k3s) and
  serves a **token-gated TCP** API. Thin clients connect by setting `MDKB_REMOTE=host:port` and
  `MDKB_TOKEN=<token>`. The daemon stays the single writer; you scale *clients*, not the daemon.
  Network access is opt-in and fails closed without a valid token.

Either way the daemon is the **single writer**: clients never touch `blocks/` files directly, and
the `.mdkb/` index is a rebuildable cache.
