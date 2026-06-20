---
title: "Skill: dedup anti-patterns"
tags: [skill, dedup]
---

## Dedup anti-patterns - don't

- **Don't delete a duplicate before repointing its backlinks** — that strands every block that
  embedded it (dangling links).
- **Don't merge by find-replacing titles** — repoint by **ULID**; titles are not identity and may
  not be unique.
- **Don't force-merge two blocks that only look similar** — different registers (a tutorial note
  vs a precise spec) can describe the same topic and both deserve to exist. Merge identical
  **facts**, not merely related ones.
- **Don't flatten content into the canonical block and lose the nuance** of the one you delete —
  enrich first, then delete.
- **Don't autonomously merge anything but exact (verbatim) duplicates, and only with up-front
  permission.** Near-duplicates and any judgment merge need explicit human approval; when in doubt
  whether two blocks are the same fact — or whether a duplicate is truly exact — ask.
