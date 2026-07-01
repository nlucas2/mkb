---
title: "Usage: exporting & publishing"
tags: [doc, usage]
updated: 2026-07-01T03:43:40Z
---

## Exporting & publishing

`mkb export` renders blocks — embeds resolved inline — to flat Markdown files, so a slice of the
vault becomes ordinary documents anyone can read without mkb. It has a few modes:

- **Docs-as-data (default)** — with a `vault/export.toml` present, `mkb export` regenerates each
  mapped doc (plus every block that routes itself via `path` / `filename` properties). `--check`
  verifies without writing — non-zero exit on drift — which is the CI gate. See the
  [docs-as-data skill](skills/mkb-docs-as-data/SKILL.md).
- **By tag** — `mkb export --tag ops` dumps every **root** tagged `ops` to `<slug>.md` under
  `docs-export/`; add `--include-non-root` to include tagged non-root blocks too.
- **Whole vault** — with no manifest and no `--tag`, `mkb export` dumps every root to
  `docs-export/`.
- **A custom manifest** — `mkb export --manifest my.toml` (or a `.json` file) uses your own
  path → block map instead of the vault's `export.toml`.

Two modifiers apply to the tag and whole-vault dumps:

- `--follow-links` — pull blocks linked *outside* the export into it, so links resolve instead of
  degrading to plain text.
- `--raw` — omit the `<!-- @generated … -->` banner, for publishing to non-mkb readers.

`--root DIR` overrides the output directory (defaults: `docs-export/` for a dump, the current
directory for a manifest).
