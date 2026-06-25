---
title: "README: Choosing your vault"
tags: [doc, readme]
updated: 2026-06-25T09:53:55Z
---

### Choosing your vault

Every client (CLI, MCP, desktop app) acts on **one vault at a time**, resolved in this order:

1. an explicit `--vault <dir>` flag (the CLI/MCP also accept `--remote`/`--socket`);
2. the `MDKB_VAULT` environment variable;
3. the **default** in your vault registry;
4. the built-in fallback `~/mdkb-vault`.

So you can always be explicit (`mdkb search --vault ~/notes "…"`), or set a default once and drop
the flag entirely (`mdkb search "…"`). Naming several vaults and choosing the default lives in the
**[configuration guide](docs/CONFIGURATION.md)**.