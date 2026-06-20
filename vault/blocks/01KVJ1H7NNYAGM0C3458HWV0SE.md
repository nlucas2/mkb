---
title: "Skill: MCP surface"
tags: [skill, surface, mcp]
---

## The MCP tool surface

**Read:** `search` (filters: `tags`, `lang`, `limit`), `get_block`, `render_block`,
`list_blocks`, `list_roots`, `graph`, `list_tags`, `backlinks`, `links_from`.

**Write:** `create_block` (title?, body), `update_block` (id, title?, body - overwrites the
whole body), `set_tags` (id, tags - replaces managed tags), `carve_block` (parent_id, body),
`flatten_block` (parent_id, child_id - inverse of carve: inline a single-use embed and delete the
child; errors unless the child is referenced exactly once), `link_blocks` (source_id, target_id,
embed - embed=true is `![[...]]`), `delete_block`.

**Maintain:** `stats`, `conflicts`, `rebuild`.
