---
title: "SPEC: Rebuilding knowledge from a raw directory"
tags: [spec, doc]
---

## Rebuilding knowledge from a raw directory

Given only `blocks/`:

1. Each `*.md` whose stem is a valid ULID is one block; the stem is its id.
2. Parse YAML frontmatter for `title`/`tags`; the rest is the body.
3. Extract `![[t]]` (child) and `[[t]]` (reference) directives from the body; resolve each `t`
   to an id (ULID, else by title).
4. The blocks + resolved edges are the DAG. Render a block by inlining its children
   recursively (breaking cycles as above).

No other state is required — the index is purely derived.
