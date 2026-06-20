---
title: Skill: MCP surface
tags: [skill, surface, mcp]
---

## The MCP tool surface

**Read:** `search` (filters: `tags`, `lang`, `limit`), `get_block`, `render_block`,
`list_blocks`, `list_roots`, `graph`, `list_tags`, `backlinks`, `links_from`.

**Write:** `create_block` (title?, body), `update_block` (id, title?, body - overwrites the
whole body), `set_tags` (id, tags - replaces managed tags), `carve_block` (parent_id, body),
`link_blocks` (source_id, target_id, embed - embed=true is `![[...]]`), `delete_block`.

**Maintain:** `stats`, `conflicts`, `rebuild`.
