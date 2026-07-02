---
title: "Architecture: why files"
tags: [doc/architecture]
updated: 2026-07-02T06:45:44Z
---

## Why files

Blocks live in files so the vault is **portable** and **sync-friendly** (OneDrive / Dropbox /
iCloud / git) — sync one small file per edit. Raw-file *human* legibility is explicitly **not**
a goal (an export tool covers that); the only hard requirement is **recoverability** — a raw
`blocks/` directory can be turned back into knowledge by a future reader, which a documented
on-disk format (`docs/SPEC.md`) guarantees.
