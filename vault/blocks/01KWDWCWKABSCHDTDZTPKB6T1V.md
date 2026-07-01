---
title: "Usage: browsing & organizing"
tags: [doc, usage]
updated: 2026-07-01T03:43:39Z
---

## Browsing & organizing

Blocks are a flat pool of files; you impose structure by **viewing** them different ways rather
than by moving files around, so the same block can appear under several groupings at once.

In the **desktop app**, the sidebar's *Group by* selector re-shapes the block list:

- **Hierarchy** (default) — the composition tree: root blocks at the top, each expanding into the
  blocks it embeds or links, in authored order.
- **Flat** — every block, ungrouped.
- **Tags** — a `/`-nested tree of your tags.
- **Path** (or any property) — grouped by a property value, also `/`-nested.

From the **CLI**, the same two engines are:

- `mkb hierarchy` — the composition tree (roots → embeds/links) as an indented outline.
- `mkb group-by tags`, `mkb group-by path`, or `mkb group-by <property>` — a `/`-nested tree by tag
  or by any property key.

`/`-**nesting** is a convention, not a separate feature: a tag or property value that contains
slashes — `ops/networking/dns`, or `path: docs/skills/mkb-cli` — nests in these trees, so a flat
namespace reads like folders. Blocks that don't carry the grouping value collect under an
**Unfiled** node you can still open and browse.
