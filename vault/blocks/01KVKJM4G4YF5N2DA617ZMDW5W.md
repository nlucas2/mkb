---
title: "Architecture: recoverability & export"
tags: [doc, architecture]
---

## Recoverability & export

- **`docs/SPEC.md`** documents the on-disk format precisely, so a raw `blocks/` directory can be
  turned back into knowledge with no app: read each file, parse its frontmatter + `![[...]]` /
  `[[...]]` directives, rebuild the DAG.
- **Export (`mkb export`)** flattens chosen blocks (or the whole DAG) into standalone,
  fully-inlined Markdown — every transclusion dissolved — for handing knowledge to something that
  doesn't understand mkb, and for **docs-as-data** (this repo's own README, SPEC, AGENTS, and
  skills are generated this way; see the README's *Docs are generated* section). Not required for
  normal operation; the vault remains the source of truth.
