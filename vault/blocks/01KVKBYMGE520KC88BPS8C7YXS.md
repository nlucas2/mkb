---
title: "SPEC: Resilience"
tags: [spec, doc]
---

## Resilience

- **Cycles**: rendering is total — it never loops. A transclusion cycle renders up to the
  repeat, then shows a link + a "↻ cycle" note. Creating an embed that would cycle is
  **downgraded** to a plain reference (the link is still made).
- **Dangling targets**: an `![[...]]`/`[[...]]` whose target doesn't resolve renders an
  inline "unresolved" note; the rest of the block renders normally.
- **Conflict copies**: cloud-sync conflict files (e.g. `… (conflicted copy).md`) are
  surfaced but never indexed; resolve them in plain text.
