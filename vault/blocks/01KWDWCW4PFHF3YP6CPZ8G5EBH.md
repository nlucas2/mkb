---
title: "Usage: searching"
tags: [doc, usage]
updated: 2026-07-02T04:41:10Z
---

## Searching

`mkb search "<query>"` (and the app's search box) run a **hybrid** of keyword (full-text) and
semantic (vector) ranking, so a natural phrase — *"how do I restart the ingress controller"* —
works as well as exact terms. Search for a *concept*, not a lone keyword, then sharpen with
operators. Each operator has an inline form (typed into the query) and, on the CLI, an equivalent
flag; they combine with AND:

| Inline | CLI flag | Matches |
|---|---|---|
| `tag:NAME` / `#NAME` | `--tag NAME` (repeatable) | blocks where `NAME` is a whole `/`-segment: `NAME`, `NAME/…`, `…/NAME`, or `…/NAME/…` |
| `tag:NAME$` | `--tag-exact NAME` | that exact tag only (no hierarchy) |
| `lang:NAME` / `code:NAME` | `--lang NAME` | blocks with a fenced code block in that language |
| `created:after:DATE` / `created:before:DATE` | `--created-after` / `--created-before` | by creation date (`YYYY-MM-DD` or RFC 3339) |
| `updated:after:DATE` / `updated:before:DATE` | `--updated-after` / `--updated-before` | by last-modified date |
| `has:KEY` | `--has KEY` (repeatable) | blocks that **have** a property key |
| `missing:KEY` | `--missing KEY` (repeatable) | blocks that **lack** a property key |
| `"exact phrase"` | — | those words in sequence |

Each hit also shows where it lives — `[root]` for a top-level page, or `↑ embedded in: <page>` for a
reused block — so a match tells you its page(s) without a separate backlinks lookup (suppress it
with `--no-context`). The metadata filters make tidy-up audits easy: `updated:before:2026-01-01`
finds stale blocks, `missing:source` finds atoms lacking a citation.

Tag matching is **hierarchical**: a query matches wherever it appears as a whole `/`-segment run.
`tag:kusto` matches `kusto`, `kusto/workersystemstats`, `a/kusto`, and `a/kusto/b`; the leaf `tag:df`
matches `kusto/workersystemstats/df`; and a multi-segment `tag:doc/readme` matches that contiguous
run anywhere (`a/doc/readme/b`). Boundaries are anchored on `/` (or the ends), so nothing leaks into
an unrelated `kustonomicon`. Append `$` (or use `--tag-exact`) for an exact-tag match instead.

Nest tags with `/` right in the frontmatter — one path tag is enough, because a search for any
ancestor finds it:

```markdown
---
title: Worker system stats — disk-free query
tags: [kusto/workersystemstats/df]
---
```

That block is returned by `tag:kusto`, `tag:kusto/workersystemstats`, the full
`tag:kusto/workersystemstats/df`, and even the bare leaf `tag:df` — so you no longer add a redundant
flat `kusto` tag alongside the path just to keep it findable.

