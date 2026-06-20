---
title: "Architecture: the index is a rebuildable cache"
tags: [doc, architecture]
---

## The index is a rebuildable cache

A SQLite index (`mdkb-index`, behind the `Index` trait) caches everything for fast reads and is
**always reconstructable** by scanning `blocks/`:

- a row per block (id, title, content, kind metadata, tags, lineage/breadcrumb, embedding);
- the link table (`transcludes | references | child_of`) derived from each file's directives;
- FTS5 (porter + unicode61, bm25) for keyword search;
- a vector store (per-block embedding, tagged with the model id) for semantic search.

Search fuses keyword + semantic via **Reciprocal Rank Fusion (RRF)**. The index is never the
source of truth; on any doubt, rebuild from files.
