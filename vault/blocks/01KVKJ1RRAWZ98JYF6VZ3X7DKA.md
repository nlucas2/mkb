---
title: README page
tags: [doc/readme, page, user]
updated: 2026-07-07T04:38:25Z
---

# mkb — Modular Knowledge Base

![[01KVHJ76YA04MEM71HNDB7RT8G]]

<p align="center">
  <a href="docs/images/app-read.png"><img src="docs/images/app-read.png" alt="mkb desktop app in Read mode — a block with its embeds dissolved into one clean Markdown document" width="820"></a>
</p>

> **Pre-release** (`0.1.0`). See **[`docs/architecture.md`](./docs/architecture.md)** for the design
> and **[`docs/SPEC.md`](./docs/SPEC.md)** for the on-disk format.

## Getting started

![[01KVYGC2REQ5D7MG46FRVW5TR0]]

Building from source needs Rust, [`just`](https://github.com/casey/just), and your platform's
webview build libraries — see **[Installing the prerequisites](docs/PREREQS.md)**. Prebuilt releases
and the container need none of them.

![[01KVYAA72QVXFFEP1PA6FDZN9V]]

![[01KVYAA72YQDAGGSV1NE2F06ZD]]

## Using mkb

![[01KVM9NPR2HD2WF05GKFYNMG68]]

![[01KVKJ1RB2HP9V609P81AWWS41]]

### From an AI client (MCP)

![[01KVKJ1QYRSVX6DW8B575BW44X]]

![[01KVKJ1RFN39VZ0AXVRJ3VHMFB]]

Optional, advanced setup — choosing a different embedder and managing multiple vaults — lives in the **[configuration guide](docs/CONFIGURATION.md)**.

![[01KVKJ1RM8647XQC65WD0G37YN]]

### Skills to reference

![[01KVZ28PBZFKGXKG2GS3BHZK17]]

## Development

Hacking on mkb? The workspace layout, the daemon/client internals, and the roadmap are in
**[`docs/CONTRIBUTING.md`](./docs/CONTRIBUTING.md)**; the mandatory working rules are in
**[`AGENTS.md`](./AGENTS.md)**, the design in **[`docs/architecture.md`](./docs/architecture.md)**,
and the on-disk format in **[`docs/SPEC.md`](./docs/SPEC.md)**.

![[01KVM9NQP8GEPRXQRKK062E37R]]
