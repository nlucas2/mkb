---
title: "Skill: finding duplicates and orphans"
tags: [skill, dedup]
---

## Collect — sweep the whole vault first

Do a **complete pass before stopping**: gather *every* candidate, then report. Don't interrupt
the human after each find — collect first, present once.

Use search and the link graph as your instruments:

1. **Survey the roots.** List root blocks (the pages) and skim titles for any two that cover the
   same ground. `stats` gives `blocks` vs `roots` vs `embedded` — a low embedded-to-blocks ratio
   hints at orphans.
2. **Search each non-trivial fact in 2-3 phrasings.** Search is hybrid (keyword + semantic), so
   paraphrases of the same fact rank together — two near-identical hits with different ids is a
   duplicate signal. Note whether each pair is **exact** (verbatim-identical) or **near** (same
   fact, different words) — that determines how it can be handled.
3. **Check backlinks to spot orphans.** A block whose backlinks are empty *and* that links to
   nothing is an orphan.

## Itemize — present the whole inventory at once

Report everything you found as **one numbered list**, so the human reviews in bulk instead of
one-at-a-time. For each item give:

- **what it is** — the blocks involved (titles + ids) and whether it's an *exact* dup, a *near*
  dup, or an *orphan*;
- **proposed canonical** — which block you'd keep, and why (usually the most-embedded one);
- **proposed action** — e.g. "repoint 2 embedders to `<id>`, delete the dup", or "no safe action,
  needs a decision".

Then let the human approve in bulk — all of them, a subset ("do 1, 3, 4"), or none. Don't ask
about each one separately.
