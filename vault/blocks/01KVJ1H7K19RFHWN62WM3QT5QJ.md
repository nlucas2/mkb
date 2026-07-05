---
title: "Skill: search operators"
tags: [skill/shared]
updated: 2026-07-05T08:00:00Z
---

## Search - hybrid + operators

Search fuses keyword (bm25) and vector ranking, so prefer a **natural phrase**
("how do I restart the ingress controller") over a single word. Then narrow with filters:

- `tag:NAME` or `#NAME` - require a tag (repeatable, AND).
- `lang:NAME` or `code:NAME` - require a fenced code block in that language.
- `"exact phrase"` - require those words **in sequence** (e.g. a sentence copied from a block);
  Markdown markers in the source don't interfere.
- everything else is free text.

Each result carries a **`score`** (this query's fused keyword+semantic relevance rank) and, when a
semantic match contributed, a **`similarity`** — the raw cosine (~0..1) between your query and the
block. Read them differently: `score` only orders *this* result set, while `similarity` is an
absolute "how close in meaning" gauge — **~0.9+ means essentially the same fact**, the mid-range
means merely related. `similarity` is absent on a keyword/filter-only hit (nothing semantic to
measure). A hit also shows **where it lives** (its root page(s)), so a hit on a reused note tells
you its page(s) without a separate backlinks lookup.

Try 2-3 phrasings (the DRY safeguard), then **follow the graph** (backlinks / links)
instead of re-searching.
