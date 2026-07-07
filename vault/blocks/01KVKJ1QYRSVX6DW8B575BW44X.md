---
title: Connecting an MCP client
tags: [doc/concept]
updated: 2026-07-07T04:17:50Z
---

Point any MCP client at the `mkb-mcp` server, giving it the vault to work on.

```jsonc
// example MCP client config entry
{
  "command": "mkb-mcp",
  "args": ["--vault", "/path/to/my-vault"]
}
```
