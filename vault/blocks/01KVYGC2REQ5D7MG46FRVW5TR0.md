---
title: "README: Install"
tags: [doc, readme]
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
