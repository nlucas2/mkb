---
title: "Skill: changing what's generated (the manifest)"
tags: [skill, doc]
---

### Add or change a generated doc (the manifest)

`vault/export.toml` maps each generated file to its source block — one `[[doc]]` per file:

```toml
[[doc]]
path  = "docs/example.md"
block = "Example page"      # a ULID or an exact block title
```

- **Add** a generated doc: author the block(s) in the vault, add a `[[doc]]` entry, run export.
- **Stop** generating one: remove its `[[doc]]` entry — the file becomes hand-maintained again.
- A `[[doc]]` may set `raw = true` to omit the `@generated` banner (for verbatim/portable output).

For ad-hoc exports without editing the manifest (by `--tag`, or the whole KB, with `--follow-links`
and cross-linking), see the README's *Docs are generated* section.
