---
title: Embeds vs references
---

# Embeds vs references

- **Embed** — `![[id]]` inlines the target's *live* content. Use it to compose a
  page out of reusable parts; edit the target once and every embed reflects it.
- **Reference** — `[[id]]` is a navigable link, not inlined.

Only embeds expand at render time, so only embeds can form a cycle. If an embed
would loop, mdkb **downgrades it to a reference** automatically.
