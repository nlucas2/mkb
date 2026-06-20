---
title: "SPEC: Directives (the edges)"
tags: [spec, doc]
---

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
