---
title: "Skill: MCP surface"
tags: [skill, surface, mcp]
updated: 2026-06-25T09:25:20Z
---

## The MCP tool surface

The surface is deliberately lean. By default the server exposes a small **core** set; set
`MKB_MCP_TOOLS=full` to also expose the **advanced** structural/metadata tools. Diagnostics
(`graph`, `stats`, `conflicts`, `rebuild`) and the narrow read primitives are CLI-only - they
don't earn a slot in an agent's per-turn tool budget.

**Read:** `search` (filters: `tags`, `lang`, `limit`, `created_*`/`updated_*` date ranges - the
freshness/staleness audit - and `has`/`missing` property keys - the metadata-gap audit, e.g. atoms
missing a `source`); `get_block` - the one rich read: title, tags, properties, timestamps
(`created` is free from the id, `updated` is the last-write time) and body, **plus** where the
block lives (lineage: its root page(s) and the blocks that embed it) and its relationships
(backlinks in / links out, each tagged embed vs reference). Options: `rendered: true` inlines child
`![[embeds]]`; `start`/`end` (1-based, inclusive) return only a line range of a large body - this
folds in what used to be separate render / line-range / backlinks / links tools. Plus `list_blocks`
(`roots_only: true` for just top-level pages) and `list_tags`.

**Write:** `create_block` (title?, body); `update_block` (id, title?, body - overwrites the whole
body; omit/empty `title` keeps it; refused if it empties or guts the block unless `force`);
`replace_in_block` (id, old, new, expect_count? - targeted partial edit, no need to resend the
body; a stale/ambiguous anchor is a safe no-op); `append_to_block` (id, text - add to the end on a
fresh line); `set_tags` (id, tags - replaces managed tags); `link_blocks` (source_id, target_id,
embed - embed=true is `![[...]]`, auto-downgraded to a reference on a cycle); `delete_block`.

**Advanced (opt-in, `MKB_MCP_TOOLS=full`):** `set_props` (id, props - **add or update** the named
`key`/`value` properties, preserving the rest; open-ended searchable metadata like
`source`/`verified`/`confidence`), `unset_props` (id, keys - remove the named properties,
preserving the rest), `carve_block` (parent_id, body - split a child out), `flatten_block`
(parent_id, child_id - inverse of carve: inline a single-use embed and delete the child; errors
unless the child is referenced exactly once).
