---
title: Commit hygiene
tags: [doc, contributing]
---

- Small, focused commits. One logical change each.
- Never commit secrets, the local `.mdkb/` index, or `/target`.
- The vault's Markdown is the source of truth; the index is always rebuildable — never treat
  the index as authoritative in code.
