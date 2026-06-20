---
name: mdkb-knowledge
description: >-
    How to use mdkb (a personal Markdown knowledge base) well as an AI client via
    its MCP server: when to read before writing, the DRY/transclusion principle,
    the process for adding new knowledge, and effective search patterns. Read this
    before storing, refactoring, or retrieving knowledge in an mdkb vault.
user-invocable: false
---

# Using mdkb as a knowledge base

mdkb is **a tool, not an agent**: plain Markdown files are the single source of truth, and
you (an AI client) reach them through the MCP server's tools. Knowledge is stored as **blocks**
— each block is one file (`blocks/<ULID>.md`). A block can **embed** other blocks
(`![[id]]`, a live transclusion) or **reference** them (`[[id]]`, a link). A "page" is just a
block that nothing embeds.

Your job when acting on the KB: **keep it a clean, deduplicated, well-connected graph** — not
a pile of near-duplicate notes.

## The golden rule: DRY — search before you write

Before creating *any* new block, **search first**. The most common mistake is writing a fresh
block for knowledge that already exists in slightly different words.

1. `search` for the concept (2–3 phrasings — keyword + semantic are fused, so paraphrases work).
2. If a block already says it → **reuse it**:
   - need it verbatim somewhere → **embed** it (`link(..., embed=true)` / `![[id]]`).
   - need to point at it → **reference** it (`[[id]]`).
   - it's slightly wrong/stale → **edit that block** (`append` to add, `replace` to correct) so
     every place that embeds it updates at once. Do **not** fork a near-duplicate.
3. Only if nothing covers it → create a new block.

> **Edit once, reflected everywhere.** A fact lives in exactly one block. If two guides need
> the same step, they both `![[that-step]]`; you never copy the text.

## When to make a reusable block (carve)

If you find a chunk *inside* a larger block that is independently reusable (an example query,
a shared procedure, a definition), **carve it into its own block** and embed it back in place.
After carving, other blocks can embed the same chunk. Rule of thumb: **if a chunk would be
useful on its own or is needed in two places, it deserves its own block.**

## Process for adding new knowledge

1. **Search** (DRY check) — confirm it isn't already captured.
2. **Decide granularity** — one atomic idea per block. A block should be the smallest unit you
   would want to reuse or link to on its own.
3. **Create** with a clear `title` (titles are how humans *and* the picker find blocks) and a
   focused body. Use a fenced code block with a **language tag** for code (```` ```kusto ````,
   ```` ```rust ````, ```` ```python ````, ```` ```sh ````, ```` ```csharp ````, … any language
   — the tag enables `lang`-filtered search).
4. **Connect it** — don't leave it orphaned:
   - `[[id]]` to related concepts (references may point anywhere, including cyclically).
   - `![[id]]` to pull in a shared sub-block instead of restating it.
   - If it belongs under an existing entry, embed it there so it's reachable.
5. **Tag** with `#tag`s (inline) or frontmatter `tags:` for faceted retrieval (e.g. `#ops`,
   `#kusto`).

## Embed vs. reference — pick deliberately

| Use `![[id]]` (embed) when… | Use `[[id]]` (reference) when… |
|---|---|
| the target's content should appear here, live | you're just pointing at related material |
| you're composing a page from reusable parts | you want a backlink without inlining |
| edits to the target must show up here | a cycle is fine (A↔B links are allowed) |

Create either with the `link_blocks` tool (`embed=true` for `![[…]]`, `false` for `[[…]]`), or
by writing the directive directly into a block's body.

**Cycles:** only embeds can loop, so the system **auto-downgrades** an embed to a reference if
it would create a transclusion cycle (`link_blocks` reports the downgrade). References are never
restricted.

## Search patterns that work

- **Concept, not keywords:** `search` is hybrid (FTS5 + vector). Prefer a natural phrase
  ("how do I restart the ingress controller") over a single word — semantic ranking handles
  paraphrase; bm25 handles exact terms.
- **Try 2–3 phrasings** before concluding something is absent (this is the DRY safeguard).
- **Filter when you know the shape:** `lang` for code (e.g. `lang=kusto`), `tags` to scope a
  domain. Combine with the query text.
- **Follow the graph, don't re-search:** once you land on a block, use `backlinks` (where it's
  already used — check this before editing a block) and `links_from` (what it points to) to
  discover neighbours. `graph` returns the whole adjacency if you need it.
- **Render to read composed pages:** `render_block` resolves all `![[…]]` children so you see
  the full assembled content; `get_block` returns just the raw body + metadata.

## Write safety (don't destroy knowledge)

- `update_block` **overwrites the whole body** of a block. Treat it as a deliberate rewrite,
  never as a quick tweak: read the block first (`get_block`), then send the *complete* new body.
- To *add* to a block, fetch its current body, append your addition, and write the whole thing
  back — do not send only the new line (that would erase the rest).
- **Never** create a second block that restates an existing one. Edit the original so every
  embedder updates at once.

> Tool surface note: the write tools may be consolidated over time (e.g. an explicit
> `append` vs. guarded `replace`). The principles above hold regardless of the exact names —
> additive edits are safe; full overwrites are deliberate and must carry the complete body.

## Anti-patterns (don't)

- ❌ Writing a new block without searching first (duplicates the graph).
- ❌ Copy-pasting the same procedure into two blocks instead of embedding one.
- ❌ Leaving a new block orphaned (no links in or out) — it becomes unfindable by graph.
- ❌ Over-splitting trivial one-liners into separate blocks, or under-splitting a reusable chunk
  into a giant block.
- ❌ `update_block`-ing a large block with a short body to "simplify" — that destroys content.
