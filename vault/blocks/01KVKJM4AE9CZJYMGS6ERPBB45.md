---
title: "Architecture: the model is a DAG"
tags: [doc, architecture]
---

## The model is a DAG

Blocks form a **directed acyclic graph** (for transclusion) over a directed graph (for
references):

- **Children = the blocks a block embeds** (`![[...]]`), in document order. *Structure* and
  *reuse* are the **same edge** — there is no separate "group", "section", or "nesting" concept;
  a block with children *is* a section, a leaf block has none.
- **References (`[[...]]`)** are a second, independent edge type: navigation links that do **not**
  expand and **may be cyclic**.
- Rendering a block renders its own content, expanding each child embed recursively. Embedding a
  block that itself has children pulls the **whole subtree**, live — edit the one block file and
  every embedder reflects it. This "edit once, reflected everywhere" guarantee is uniform across
  leaf → section → whole page.
