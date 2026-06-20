# mdkb vault on-disk format (SPEC)

> This file documents the on-disk format of an mdkb vault so a raw directory can always be
> turned back into knowledge — with or without the mdkb app. The Markdown files are the
> **single source of truth**; everything else (the index) is a rebuildable cache.

## Layout

```
<vault>/
  blocks/                 # the knowledge: one file per block
    <ULID>.md
    <ULID>.md
    ...
  .mdkb/                  # rebuildable cache + local config (safe to delete)
    index.db              # SQLite index (FTS5 + vectors); rebuilt from blocks/
    config.json           # embedder configuration (optional)
    mdkbd.sock            # daemon socket (local mode)
    mdkbd.lock            # exclusive lock: at most one daemon per vault
    mdkbd.log             # daemon log (local mode)
  SPEC.md                 # this file
```

Anything under `.mdkb/` can be deleted; the daemon rebuilds it from `blocks/` on next start.

## Single daemon per vault

A vault is owned by **at most one daemon at a time**. On startup the daemon takes an
**exclusive advisory lock** on `.mdkb/mdkbd.lock` (held for its whole lifetime, released by the
OS on exit — even on crash/kill, so it never goes stale). A second daemon launched for the same
vault fails to take the lock and exits immediately. This guarantees there is never more than one
writer/watcher for a vault, even if the socket file is removed out from under a running daemon.
Clients (UI, MCP, CLI) reuse a live daemon by pinging its socket and only spawn one when none
answers.

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

## Directives (the edges)

Two wiki directives inside a block's body define the graph. Their **target** is a block ULID
(preferred) or an exact, case-insensitive **title** match.

| Directive | Meaning | Renders as | Cyclic allowed? |
|---|---|---|---|
| `![[target]]` | **transclusion / child** — inline the target block's whole subtree, live | an embed card | **No** (auto-downgraded) |
| `[[target]]` | **reference** — a navigable link, not expanded | a link chip | Yes |
| `[[target\|label]]` | reference/embed with a display alias | as above, with `label` | — |

- The **position** of an `![[target]]` in the body is where the child renders, and defines
  child order.
- A block that nothing transcludes is a **root** (a top-level "page"). A page is just a block
  you open at the top — there is no separate page concept.
- Editing a block file updates every place that transcludes it ("edit once, reflected
  everywhere").

## Resilience

- **Cycles**: rendering is total — it never loops. A transclusion cycle renders up to the
  repeat, then shows a link + a "↻ cycle" note. Creating an embed that would cycle is
  **downgraded** to a plain reference (the link is still made).
- **Dangling targets**: an `![[...]]`/`[[...]]` whose target doesn't resolve renders an
  inline "unresolved" note; the rest of the block renders normally.
- **Conflict copies**: cloud-sync conflict files (e.g. `… (conflicted copy).md`) are
  surfaced but never indexed; resolve them in plain text.

## Rebuilding knowledge from a raw directory

Given only `blocks/`:

1. Each `*.md` whose stem is a valid ULID is one block; the stem is its id.
2. Parse YAML frontmatter for `title`/`tags`; the rest is the body.
3. Extract `![[t]]` (child) and `[[t]]` (reference) directives from the body; resolve each `t`
   to an id (ULID, else by title).
4. The blocks + resolved edges are the DAG. Render a block by inlining its children
   recursively (breaking cycles as above).

No other state is required — the index is purely derived.
