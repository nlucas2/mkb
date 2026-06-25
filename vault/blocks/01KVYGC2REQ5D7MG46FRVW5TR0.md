---
title: "README: Install"
tags: [doc, readme]
updated: 2026-06-25T10:18:10Z
---

### Install

The fastest complete install is one command from a checkout, via
[`just`](https://github.com/casey/just) — it builds and installs the **whole product**: the
desktop app, the daemon, the CLI, and the MCP server.

```sh
just install        # everything: desktop app + daemon + CLI + MCP server
just install-cli    # headless only (daemon + CLI + MCP), no GUI
```

Prefer not to build? Grab a **prebuilt release** (installer or portable archive) from the
**Releases** page, run it as a **container**, or `cargo install` just the headless tools — all
detailed in the **[install guide](docs/INSTALL.md)**.

> **Heads-up:** the prebuilt **release pipeline is still a work in progress** — published
> artifacts can lag behind `main` or miss a platform. Until it stabilises, **`just install` from a
> fresh checkout is the most reliable way to get the latest version.**
