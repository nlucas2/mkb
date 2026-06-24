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
updated: 2026-06-24T02:30:00Z
source: https://docs.k3s.io
---

# Deploying to k3s

Our cluster runs k3s on the Pi rack.

![[01J8Z3M1A2B3C4D5E6F7G8H9JK]]

Push to Forgejo and the pipeline applies the manifests.
```

- **Frontmatter** (optional): mdkb manages `title:` (a human title), `tags:` (flow `[a, b]` or a
  block list), and `locked:` (`true` marks the block **human-only** — see below). It also stamps
  `updated:` (RFC 3339 UTC last-modified) on every write; the matching **`created:` time is *not*
  stored** — it is decoded from the block's ULID id, which embeds its creation timestamp. **Any
  other `key: value` is a block *property*** — open-ended scalar metadata (e.g. `source:`,
  `verified:`, `confidence:`) that round-trips and whose value is searchable. A file with no
  frontmatter is just Markdown.
- **Body**: arbitrary Markdown. Inline `#tags` are also collected (outside code fences).
  Fenced-code-block languages are recorded for language-filtered search.
