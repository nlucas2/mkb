---
title: "Docs-as-data: a block can own its output"
tags: [doc, dev]
updated: 2026-07-01T03:20:15Z
---

### A block can own its output (`path` / `filename`)

A block doesn't have to be listed in `vault/export.toml` — it can declare where it generates to
with two properties, then be rendered with `mkb export --from-props`:

- `path` — the output **directory** (e.g. `docs/skills/mkb-cli`).
- `filename` — the output **file name** (e.g. `SKILL.md`). Omit it and the file is named from the
  block's title slug (`My Page` → `my-page.md`); an extensionless name gets `.md` appended.

So a block titled *CLI skill page* with `path: docs/skills/mkb-cli` and `filename: SKILL.md`
generates `docs/skills/mkb-cli/SKILL.md` — keeping a readable title instead of having to name the
block `SKILL.md`.

The manifest stays **authoritative**: if `export.toml` already names a block, or already writes an
output path, that wins over a property-derived one. The default `mkb export` (using the vault's
`export.toml`) *also* derives prop-routed docs, so manifest entries and self-routing blocks
coexist — letting `export.toml` shrink to only the docs that need explicit control.
