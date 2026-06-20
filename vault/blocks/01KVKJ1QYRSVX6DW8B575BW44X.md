---
title: Connecting an MCP client
tags: [doc, concept]
---

The MCP server (`mdkb-mcp`) is a thin client of the daemon; point any MCP client at it and it
auto-starts a daemon for the given vault.

```jsonc
// example MCP client config entry
{
  "command": "mdkb-mcp",
  "args": ["--vault", "/path/to/my-vault"]
}
```
