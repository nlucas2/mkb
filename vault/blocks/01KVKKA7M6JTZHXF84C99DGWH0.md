---
title: "Skill: consolidating a duplicate"
tags: [skill, dedup]
---

## Act — execute the approved batch

Work through the items the human approved (plus any exact duplicates they let you handle
autonomously up front). For **each** approved item, merge **by ULID** so no link breaks:

1. **Pick the canonical block** — the one the human chose, or for an exact duplicate either copy
   (the content is identical).
2. **Enrich the canonical first** (near-duplicates only). If the duplicate holds a nuance the
   canonical lacks, edit the canonical to cover it **before** anything is removed — never lose
   content in a merge. Exact duplicates need no enrichment.
3. **Repoint every embedder/reference of the duplicate to the canonical id.** For each backlink of
   the duplicate, read that block, swap the duplicate's id for the canonical id in its body, and
   write it back. Targeting by ULID means the link keeps working.
4. **Confirm the duplicate is now orphaned** — its backlinks should be empty.
5. **Delete the duplicate.** With zero backlinks, deleting it breaks nothing.

Repoint-then-delete (never delete-first) keeps the graph valid at every step. After working the
batch, **if the docs are generated, re-export once** and verify there's no drift, then report what
you changed.
