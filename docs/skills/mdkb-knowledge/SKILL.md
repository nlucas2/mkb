---
name: mdkb-knowledge
description: >-
    How to use mdkb (a personal Markdown knowledge base) well as an AI client — via its MCP
    server or its CLI. Covers the exact tool surface, the DRY "search before you write"
    process, block embed-vs-reference, search operators, and safe writes. Read this before
    storing, refactoring, or retrieving knowledge in an mdkb vault.
user-invocable: false
---

# Using mdkb as a knowledge base

mdkb is **a tool, not an agent**: plain Markdown files are the single source of truth, and you
reach them through a small set of tools (the MCP server, or the `mdkb` CLI). Knowledge is stored
as **blocks** — each block is one file, `blocks/<ULID>.md`, with an optional `title:`/`tags:`
frontmatter and a clean Markdown body. A block can **embed** other blocks (`![[id]]`, a live
transclusion) or **reference** them (`[[id]]`, a link). A "page" is just a block that nothing
embeds.

Your job when acting on the KB: **keep it a clean, deduplicated, well-connected graph** — not a
pile of near-duplicate notes.

## The one law: DRY — search before you write

A fact lives in **exactly one block**. Before creating *any* new block, **search first**; the
most common failure is writing a fresh block for knowledge that already exists in slightly
different words. Edit the one block and every place that embeds it updates at once.

## The process (follow in order)

1. **Search (the DRY check).** `search` the concept in **2–3 phrasings** before concluding it's
   absent — search is hybrid (FTS5 keyword + vector semantic), so paraphrases rank too.
2. **If it already exists, reuse — do not fork:**
   - need it verbatim elsewhere → **embed** it (`link_blocks … embed=true`, i.e. `![[id]]`).
   - just pointing at it → **reference** it (`link_blocks … embed=false`, i.e. `[[id]]`).
   - it's stale/wrong → **edit that block** (read it, then `update_block` with the full corrected
     body). Every embedder updates at once.
3. **If a reusable chunk is buried inside a bigger block, carve it.** `carve_block` splits a
   chunk into its own child and leaves a `![[child]]` in the parent. Now both places share it.
   Rule of thumb: *if a chunk is useful on its own or is needed in two places, it deserves its
   own block.*
4. **Only if nothing covers it, create.** `create_block` with a clear `title` (titles are how
   humans and the link picker find blocks) and one atomic idea in the body.
5. **Connect it — never leave an orphan.** Add `[[id]]` references to related concepts and/or
   `![[id]]` embeds of shared sub-blocks. An orphan (no links in or out) is unfindable by graph.
6. **Tag for retrieval.** Set managed tags with `set_tags` (these become frontmatter `tags:`),
   and/or write inline `#hashtags` in the prose. Both are searchable; managed tags are the ones
   tools edit, inline hashtags are content. Tags drive faceted search (below).

## The tool surface

**Discover / read**
- `search` — hybrid keyword+semantic search; the primary entry point. Filters below.
- `list_roots` — top-level entries (blocks nothing embeds). `list_blocks` — every block id.
- `get_block` — one block's title, tags, and **raw** body (for editing).
- `render_block` — the block with all `![[…]]` children resolved (the **assembled** view to
  *read*; use `get_block` when you intend to *edit*).

**Connect / explore the graph**
- `backlinks` — who references/embeds this block. **Check this before editing a block** so you
  know what your edit will affect.
- `links_from` — what this block points to. `graph` — the whole adjacency (only when you truly
  need it; prefer `backlinks`/`links_from` for local moves).
- `link_blocks` — create a link: `embed=true` → `![[id]]`, `embed=false` → `[[id]]`.
- `list_tags` — every tag with its block count (tag discovery; feed a tag into `search`'s
  `tags` filter to scope a domain).

**Write**
- `create_block` (`title?`, `body`) → new id.
- `carve_block` (`parent_id`, `title?`, `body`) → new child id; appends `![[child]]` to parent.
- `update_block` (`id`, `title?`, `body`) — **overwrites the whole body** (see *Safe writes*).
  Managed (frontmatter) tags are preserved across the edit.
- `set_tags` (`id`, `tags`) — set the block's managed (frontmatter) `tags:` to exactly this list
  (`[]` clears). Inline `#hashtags` in the body are left as prose; title and body are preserved.
- `delete_block` (`id`) — removes the file and its index entries.

**Maintain**
- `stats` — block / root / embedding counts. `conflicts` — blocks needing attention.
- `rebuild` — rebuild the index from `blocks/` (the index is a rebuildable cache; the Markdown
  is the source of truth). Rarely needed; the daemon keeps the index reconciled automatically.

## Search: hybrid + operators

Search fuses keyword (bm25) and vector ranking, so **prefer a natural phrase**
("how do I restart the ingress controller") over a single word. Then narrow with filters:

- **Via the MCP `search` tool** — use the structured params: `tags: ["k8s"]` (all required, AND)
  and `lang: "kusto"` (require a fenced code block in that language). Combine with the query
  text and `limit`.
- **Via the CLI or the desktop search box** — the same filters are available as inline operators
  in the query string (parsed by the shared `SearchQuery::parse`, so every surface agrees):
  - `tag:NAME` or `#NAME` — require tag `NAME` (repeatable, AND).
  - `lang:NAME` or `code:NAME` — require a fenced code block in language `NAME`.
  - everything else is free text.
  - e.g. `tag:k8s lang:kusto cluster health` or `#networking retry policy`.

Tips: try 2–3 phrasings (the DRY safeguard); filter by `lang` when you're hunting code (e.g. a
Kusto/KQL or Rust snippet); filter by `tag` to scope a domain; then **follow the graph**
(`backlinks` / `links_from`) instead of re-searching.

## Embed vs. reference — pick deliberately

| Use `![[id]]` (embed) when… | Use `[[id]]` (reference) when… |
|---|---|
| the target's content should appear here, live | you're just pointing at related material |
| you're composing a page from reusable parts | you want a backlink without inlining |
| edits to the target must show up here | a cycle is fine (A↔B is allowed) |

**Cycles:** only embeds can loop, so an embed that would create a transclusion cycle is
**auto-downgraded to a reference** (`link_blocks` reports the downgrade). References are never
restricted.

## Safe writes (don't destroy knowledge)

There is today exactly one body-mutating write: **`update_block`, which replaces the entire
body.** Treat it as a deliberate rewrite, never a quick tweak:

1. `get_block` to fetch the **current** body.
2. Modify the full text locally (to *add*, append your lines to what you fetched).
3. `update_block` with the **complete** new body.

Never send only a new line to `update_block` — that erases everything else. Never create a second
block that restates an existing one; edit the original so every embedder updates at once.

> Roadmap: the write surface may gain an explicit `append` and a guarded `replace` (rejecting an
> empty/near-empty body) so additive edits don't require read-modify-write. Until then, the
> read-modify-write pattern above is the safe path.

## CLI quickstart

The CLI is a **thin daemon client** — every command connects to (and auto-starts) the vault's
daemon, exactly like the MCP server and the app, so reads and writes share one warm index and the
daemon stays the single writer. It is a full equivalent of the MCP surface:

```sh
# reads
mdkb search <vault> "tag:k8s lang:kusto cluster health"  # operators or --tag= --lang= --limit=
mdkb tags   <vault>                                       # all tags with block counts
mdkb list   <vault>                                       # root blocks (id  title)
mdkb render <vault> <id>                                  # assembled (embeds resolved)
mdkb get    <vault> <id>                                  # raw body (for read-modify-write)
mdkb backlinks <vault> <id>   /   mdkb links <vault> <id>
mdkb stats  <vault>   /   mdkb conflicts <vault>   /   mdkb ping <vault>

# writes (body via stdin where shown)
echo "# Title" | mdkb create <vault> --title="Title"     # prints the new id
echo "new body" | mdkb update <vault> <id>               # overwrite title+body
mdkb set-tags <vault> <id> k8s ops                        # managed tags ([] clears)
mdkb link <vault> <src> <dst> [--embed]                   # reference or transclude
echo "carved" | mdkb carve <vault> <parent>               # prints the new child id
mdkb delete <vault> <id>

# maintenance
mdkb rebuild <vault>
mdkb export <vault> [--check]                             # generate repo docs from blocks (docs-as-data)
```

Connection defaults to a local Unix socket; set `MDKB_REMOTE=host:port` (+ `MDKB_TOKEN`) to use a
TCP daemon (e.g. a loopback port where Unix sockets aren't usable), or `MDKB_SOCKET=<path>`.

## Anti-patterns (don't)

- ❌ Writing a new block without searching first (duplicates the graph).
- ❌ Copy-pasting the same procedure into two blocks instead of embedding one.
- ❌ Leaving a new block orphaned (no links in or out) — it becomes unfindable by graph.
- ❌ Over-splitting trivial one-liners, or under-splitting a reusable chunk into a giant block.
- ❌ `update_block`-ing a large block with a short body to "simplify" — that destroys content.
- ❌ Editing a block without checking `backlinks` first — you won't know what you're changing.
