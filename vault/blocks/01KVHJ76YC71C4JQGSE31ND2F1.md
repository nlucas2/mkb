---
title: Embeds vs references
---

# Embeds vs references

- **Embed** — `![[id]]` inlines the target's *live* content. Use it to compose a
  page out of reusable parts; edit the target once and every embed reflects it.
- **Reference** — `[[id]]` is a navigable link, not inlined.

Only embeds expand at render time, so only embeds can form a cycle. If an embed
would loop, mkb **downgrades it to a reference** automatically.

A target can be a block id **or a title**: for example, this links to [[Blocks are files]] by its
title, and mkb resolves it to that block just as an id would. Ids are stable across renames, so
the picker defaults to them — but a title works too.
