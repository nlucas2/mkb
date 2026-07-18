---
title: "Usage: Core Commands (CLI)"
updated: 2026-07-17T23:28:52Z
---

## Core Commands (CLI)

The CLI offers the full surface of mkb for terminal users and script automation. Here are the core data commands (remember to add `--vault <dir>` if you haven't set a default):

### Reads
- `mkb list` — list all root blocks.
- `mkb get <id>` — print the raw Markdown body of a block.
- `mkb render <id>` — print the block with its `![[embeds]]` resolved inline.
- `mkb search "query"` — hybrid keyword + semantic search.
- `mkb tags` — list every tag with its block count.
- `mkb backlinks <id>` — list blocks that reference or embed `<id>`.
- `mkb info <id>` — print metadata (created, updated, locked status, tags, and properties).
- `mkb graph --format svg --output graph.svg` — export a deterministic headless graph. SVG is the default; JSON emits the positioned scene and DOT delegates layout/rendering to Graphviz. Basic options control dimensions, theme, labels, tags, and transparency. This CLI layout is reproducible rather than a copy of an open UI tab.

### Writes
*Write commands that set a body read from `stdin`.*

- `echo "body" | mkb create --title "Title"` — creates a block and prints the new `<id>`.
- `mkb update <id> < file.md` — completely overwrites the block's body with the input.
- `mkb append <id>` — adds text to the end of a block.
- `mkb replace <id> --old "bad" --new "good"` — surgical targeted edit; fails if the old text isn't a unique match.
- `mkb set-tags <id> foo bar` — sets the block's frontmatter tags.
- `mkb set-props <id> author=Alice` — sets the block's frontmatter properties.
- `mkb delete <id>` — deletes the block.