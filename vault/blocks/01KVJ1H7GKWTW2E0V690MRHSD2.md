---
title: Skill: DRY search-first
tags: [skill, shared]
---

## DRY — search before you write

A fact lives in **exactly one block**. Before creating any new block, **search first**:
the most common failure is writing a fresh block for knowledge that already exists in
slightly different words. Edit the one block and every place that embeds it updates at once.

Follow this in order:

1. **Search (the DRY check)** in 2-3 phrasings before concluding something is absent -
   search is hybrid (keyword + semantic), so paraphrases rank too.
2. **If it exists, reuse - don't fork:** embed it (`![[id]]`) to inline it live, reference
   it (`[[id]]`) to point at it, or edit that block if it's stale.
3. **If a reusable chunk is buried in a bigger block, carve it** into its own block and
   embed it back in place.
4. **Only if nothing covers it, create** - one atomic idea, with a clear title.
5. **Connect it** - never leave an orphan; add references and/or embeds.
6. **Tag it** for retrieval.
