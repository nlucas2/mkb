---
title: "SPEC: Single daemon per vault"
tags: [spec, doc]
---

## Single daemon per vault

A vault is owned by **at most one daemon at a time**. On startup the daemon takes an
**exclusive advisory lock** on `mdkbd.lock` (in the machine-local index directory, held for its
whole lifetime, released by the OS on exit — even on crash/kill, so it never goes stale). A second
daemon launched for the same vault fails to take the lock and exits immediately. This guarantees
there is never more than one writer/watcher for a vault, even if the socket file is removed out
from under a running daemon. Clients (the app, MCP, CLI) reuse a live daemon by pinging its socket
and only spawn one when none answers.
