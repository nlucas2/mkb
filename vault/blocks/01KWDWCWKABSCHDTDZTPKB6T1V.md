---
title: "Usage: browsing & organizing"
tags: [doc/usage]
updated: 2026-07-07T05:56:25Z
---

## Browsing & organizing

Blocks are a flat pool of files; you impose structure by **viewing** them different ways rather
than by moving files around, so the same block can appear under several groupings at once.

You can use this UI two ways: the **desktop app**, or the **same UI in a browser** — either start it
from the app's **Settings → Web UI** (a Launch/Stop button, no terminal needed) or run `mkb-web`
yourself, then open `http://127.0.0.1:8787`. It is the app as a web page — handy for reaching your
vault from a phone (the layout adapts to a small screen) or another machine on your network. (If
you run `mkb-web` to reach it from another machine, bind it to all interfaces:
`mkb-web --bind 0.0.0.0:8787`, but be aware this exposes the UI to your local network without
authentication.)

Unlike the CLI, `mkb-web` does not take a `--vault` flag. It reads the vaults you configured in the desktop app, and vault selection happens dynamically in the browser tab.

In either UI, the sidebar's *Group by* selector re-shapes the block list:

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

In the app, any parent node with children in a `/`-nested group tree (the **Tags**, **Path**, or a
property grouping — at any depth) carries a small **≡** button — click it to *flatten* that subtree
into one de-duped list of every block under it, without expanding each child in turn; click again to
return to the nested view.
