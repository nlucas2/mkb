---
title: "SPEC: A block file"
tags: [spec, doc]
---

## A block file

Each file in `blocks/` is **one block**. Its name is the block's identity: a
[ULID](https://github.com/ulid/spec) (26 Crockford-base32 chars) plus the `.md` extension,
e.g. `blocks/01J8Z3K9P0QH7W2RE5N6T4XYAB.md`. The ULID **is** the id; nothing references a
file by path or title, so files can be moved/renamed freely (only the stem matters).

A block file is clean Markdown with optional YAML frontmatter:

```markdown
---
title: Deploying to k3s
tags: [k3s, ops]
---

# Deploying to k3s

Our cluster runs k3s on the Pi rack.

![[01J8Z3M1A2B3C4D5E6F7G8H9JK]]

Push to Forgejo and the pipeline applies the manifests.
```

- **Frontmatter** (optional): `title:` (a human title) and `tags:` (flow `[a, b]` or a block
  list). Unknown keys are ignored. A file with no frontmatter is just Markdown.
- **Body**: arbitrary Markdown. Inline `#tags` are also collected (outside code fences).
  Fenced-code-block languages are recorded for language-filtered search.
