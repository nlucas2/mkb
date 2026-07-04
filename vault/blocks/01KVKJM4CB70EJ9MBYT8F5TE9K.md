---
title: "Architecture: the index is a rebuildable cache"
tags: [doc/architecture]
updated: 2026-07-04T02:15:00Z
---

## The index is a rebuildable cache

A SQLite index (`mkb-index`, behind the `Index` trait) caches everything for fast reads and is
**always reconstructable** by scanning `blocks/`:

- a row per block (id, title, content, kind metadata, tags, lineage/breadcrumb, embedding);
- the link table (`transcludes | references | child_of`) derived from each file's directives;
- FTS5 (porter + unicode61, bm25) for keyword search;
- a vector store (per-block embedding, tagged with the model id) for semantic search.

Search fuses keyword + semantic via **Reciprocal Rank Fusion (RRF)**. The index is never the
source of truth; on any doubt, rebuild from files.

Semantic matching is currently an **exact brute-force cosine scan** over the stored embeddings —
simple, dependency-free, and exact, which has been more than fast enough for everything mkb has
been used for so far. The `Index` trait is the seam to swap in an approximate-nearest-neighbour
(ANN) engine should a vault ever grow large enough for the linear scan to matter — see the roadmap
in [`CONTRIBUTING.md`](./CONTRIBUTING.md).
