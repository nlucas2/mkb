---
title: "README: Two ways to run it"
tags: [doc, readme]
---

## Two ways to run it

mdkb supports two deployment paradigms. The Markdown vault is the source of truth in both; the
difference is *where the daemon runs* and *how clients reach it*.

![[01KVHJ76YHDWY5PMB7GY9B0WP6]]

### 1. Local-first (single machine)

Everything runs on your machine: `mdkbd` owns the vault and a local socket (a **Unix-domain
socket** on Linux/macOS, a **named pipe** on Windows), and the CLI, MCP server, web UI, and
desktop app all connect to it. To use multiple machines, **sync only the Markdown** (OneDrive,
etc.) — each machine runs its own daemon and keeps its own local index. This is the default and
needs no configuration.

```sh
mdkbd --vault ~/mdkb-vault          # serves ~/mdkb-vault/.mdkb/mdkbd.sock
```

### 2. Remote / shared daemon (e.g. in a cluster)

One `mdkbd` runs centrally (e.g. a `replicas: 1` Deployment in k3s) and serves a **token-gated
TCP** API. Thin clients — desktop app, web UI, CLI, MCP — connect over the network by setting
`MDKB_REMOTE=host:port` and `MDKB_TOKEN=<token>` (resolved by the shared `Client::from_env`). The
daemon stays a single writer; you scale *clients*, not the daemon. Network access is opt-in and
fails closed without a valid token.

```sh
mdkbd --vault /vault --listen 0.0.0.0:7820 --token "$MDKB_TOKEN"   # on the server
export MDKB_REMOTE=mdkbd.example:7820 MDKB_TOKEN=…                 # on each client
```

See [`deploy/README.md`](./deploy/README.md) for the Kubernetes manifest and end-to-end
remote setup.
