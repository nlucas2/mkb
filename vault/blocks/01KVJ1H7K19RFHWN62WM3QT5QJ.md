---
title: "Skill: search operators"
tags: [skill, shared]
updated: 2026-06-25T07:51:05Z
---

## Search - hybrid + operators

Search fuses keyword (bm25) and vector ranking, so prefer a **natural phrase**
("how do I restart the ingress controller") over a single word. Then narrow with filters:

- `tag:NAME` or `#NAME` - require a tag (repeatable, AND).
- `lang:NAME` or `code:NAME` - require a fenced code block in that language.
- `"exact phrase"` - require those words **in sequence** (e.g. a sentence copied from a block);
  Markdown markers in the source don't interfere.
- everything else is free text.

Each result shows where it lives (`[root]`, or `↑ embedded in: <page>`), so a hit on a
reused note tells you its page(s) without a separate backlinks lookup.

Try 2-3 phrasings (the DRY safeguard), then **follow the graph** (backlinks / links)
instead of re-searching.
