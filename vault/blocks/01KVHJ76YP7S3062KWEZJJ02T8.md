---
title: Use mdkb from an AI client (MCP)
tags: [mdkb, run, mcp]
---

# Use mdkb from an AI client (MCP)

The MCP server is a thin client of the daemon; it auto-starts a daemon for the vault.

![[01KVHJ76YHDWY5PMB7GY9B0WP6]]

```jsonc
{
  "command": "mdkb-mcp",
  "args": ["--vault", "/path/to/my-vault"]
}
```
