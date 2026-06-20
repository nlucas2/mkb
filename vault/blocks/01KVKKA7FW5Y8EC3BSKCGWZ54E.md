---
title: "Skill: when to audit for duplicates"
tags: [skill, dedup]
---

## Auditing a vault for duplicates

Search-before-write prevents *new* duplicates; an **audit** finds the ones that already crept in
(a fact written twice in different words, two blocks that drifted apart, an orphan nothing links
to) and **proposes** repairs for a human to approve. Audit when you've added a batch of
knowledge, inherited a vault, or notice the same idea phrased two ways.

What you're hunting for (to report, not to silently fix):

- **Near-duplicate facts** — two blocks that state the same thing. The expensive kind: they
  drift, and a fix lands in one but not the other.
- **Orphans** — a block with no backlinks and no outgoing links: unreachable by the graph,
  effectively lost.
- **Should-have-been-shared** — the same sentence/procedure copy-pasted into two blocks instead
  of one block embedded twice.
