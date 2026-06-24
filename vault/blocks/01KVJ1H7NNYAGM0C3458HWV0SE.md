---
title: "Skill: MCP surface"
tags: [skill, surface, mcp]
---

## The MCP tool surface

**Read:** `search` (filters: `tags`, `lang`, `limit`, and `created_*`/`updated_*` date ranges —
the freshness/staleness audit), `get_block` (returns title, tags, properties, and timestamps —
`created` is free from the id, `updated` is the last-write time), `render_block`,
`list_blocks`, `list_roots`, `graph`, `list_tags`, `backlinks`, `links_from`.

**Write:** `create_block` (title?, body), `update_block` (id, title?, body - overwrites the
whole body), `set_tags` (id, tags - replaces managed tags), `set_props` (id, props - **add or
update** the named `key`/`value` properties, preserving the rest; open-ended metadata like
`source`/`verified`/`confidence`, whose values are searchable), `unset_props` (id, keys - remove
the named properties, preserving the rest), `carve_block` (parent_id, body),
`flatten_block` (parent_id, child_id - inverse of carve: inline a single-use embed and delete the
child; errors unless the child is referenced exactly once), `link_blocks` (source_id, target_id,
embed - embed=true is `![[...]]`), `delete_block`.

**Maintain:** `stats`, `conflicts`, `rebuild`.
