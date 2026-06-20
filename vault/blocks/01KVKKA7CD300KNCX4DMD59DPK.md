---
title: "Skill: audit is human-driven (don't delete on your own)"
tags: [skill, dedup]
---

## Audit is human-driven — collect, itemize, act

Deduplication **deletes and rewrites knowledge**, so it is **human-in-the-loop by default**. But
don't hold the human hostage to one-at-a-time approvals — work in three phases: **collect** every
candidate in a full sweep, **itemize** them as one batch of proposals, then **act** on what the
human approves. Review once, in bulk; execute without re-prompting per item.

**Ask up front.** When you begin, ask one question: *"May I autonomously consolidate **exact**
duplicates (verbatim-identical content in two places), or should I confirm every change?"* Respect
the answer for the rest of the session.

- **Exact duplicates** — content **byte-for-byte identical** in two blocks (e.g. the same section
  copy-pasted into a README and a Welcome page). Mechanical, not a judgment call, so it is **safe
  to consolidate autonomously *if* the human granted that up front** — no need to make them
  re-approve work they could trivially rubber-stamp. Even then: repoint by ULID, keep one
  canonical block, never alter the wording.
- **Everything else is confirmation-gated.** Near-duplicates (the same fact in different words),
  drifted blocks, or any merge where you'd choose which nuance to keep need the human's approval in
  the itemized review — these take judgment, and judgment is the human's.

Always-true guardrails, regardless of the up-front answer:

- **Default to read-only.** Searching, listing, reading blocks, and checking backlinks are always
  safe — do those freely, and exhaustively, during the collect phase.
- **Itemize before acting** (for anything beyond pre-approved exact-dedup). Present the full list
  of candidates with a proposed canonical + action for each, and let the human approve all, a
  subset, or none — in one pass.
- **Never drop information to "tidy up".** If two blocks each hold a nuance, that is content to
  **preserve**, not a reason to delete one. When unsure whether two blocks are the same fact, or
  whether a duplicate is truly exact, **ask** — don't merge on a guess.
