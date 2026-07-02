---
title: Commit hygiene
tags: [doc/contributing]
updated: 2026-07-02T06:45:42Z
---

- Small, focused commits. One logical change each.
- Never commit secrets, the machine-local index, or `/target`.
- The vault's Markdown is the source of truth; the index is always rebuildable — never treat
  the index as authoritative in code.
