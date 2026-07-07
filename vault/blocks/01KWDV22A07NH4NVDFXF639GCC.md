---
title: "Docs-as-data: a block can own its output"
tags: [skill/docs-as-data]
updated: 2026-07-07T05:28:54Z
---

### Add a new generated doc

To make a block generate a file, give it a `path` property and (optionally) a `filename` property.
`mkb export` automatically sweeps the vault for these blocks.

- `path` — the output **directory** (e.g. `docs/skills/mkb-cli`).
- `filename` — the output **file name** (e.g. `SKILL.md`). Omit it and the file is named from the
  block's title slug (`My Page` → `my-page.md`); an extensionless name gets `.md` appended.

*Example:* A block titled *CLI skill page* with `path: docs/skills/mkb-cli` and `filename: SKILL.md`
generates `docs/skills/mkb-cli/SKILL.md`.

To **stop** generating a doc, remove the `path` property. The file becomes hand-maintained again.

#### Legacy / Override Manifest (`export.toml`)
If a document needs shared banner policies (`raw = true`) or needs explicit override control, it may
be mapped in `vault/export.toml` (`path = ...` and `block = ...`). The manifest stays authoritative:
if `export.toml` already names a block, or already writes an output path, that wins over a
property-derived one.