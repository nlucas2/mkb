---
title: "README: Install"
tags: [doc/readme]
updated: 2026-07-07T04:37:40Z
---

### Install

The fastest complete install is one command from a checkout, via
[`just`](https://github.com/casey/just) — it builds and installs the **whole product**: the
desktop app (also reachable in a browser via `mkb-web`), the CLI, and the MCP server.

```sh
just install        # everything: desktop app (+ mkb-web) + CLI + MCP server
```

Prefer not to build? Grab a **prebuilt release** (installer or portable archive) from the
**Releases** page or run it as a **container** — both detailed in the
**[install guide](docs/INSTALL.md)**. (As a pre-release, published artifacts can lag behind `main`
or miss a platform; `just install` from a fresh checkout is the most reliable way to get the latest.)
