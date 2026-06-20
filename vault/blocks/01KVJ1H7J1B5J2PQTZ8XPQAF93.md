---
title: "Skill: embed vs reference"
tags: [skill, shared]
---

## Embed vs. reference - pick deliberately

| Use `![[id]]` (embed) when... | Use `[[id]]` (reference) when... |
|---|---|
| the target's content should appear here, live | you're just pointing at related material |
| you're composing a page from reusable parts | you want a backlink without inlining |
| edits to the target must show up here | a cycle is fine (A-B is allowed) |

Only embeds expand at render time, so only embeds can form a cycle. An embed that would
create a transclusion cycle is **auto-downgraded to a reference**; references are never
restricted.

A target may be a **ULID or a block title** (titles resolve case-insensitively). Prefer the
**ULID** you get back from `create`/`search`: it is stable across title edits and never ambiguous.
A title is fine for hand-authoring, but it can drift or collide, so don't rely on it for a durable
link.
