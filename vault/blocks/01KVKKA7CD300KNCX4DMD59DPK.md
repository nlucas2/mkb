---
title: "Skill: audit is human-driven (don't delete on your own)"
tags: [skill, dedup]
---

## Audit is human-driven — collect, itemize, act

Deduplication **deletes and rewrites knowledge**, so it is **human-in-the-loop by default** — you
audit and propose; the human approves. Work in three phases so you don't stop for approval after
every find: **collect** every candidate in a full sweep, **itemize** them as one batch of
proposals, then **act** on what the human approves.

**Ask up front.** When you begin, ask one question: *"May I autonomously consolidate **exact**
duplicates (verbatim-identical content in two places), or should I confirm every change?"* Respect
the answer for the rest of the session.

- **Exact duplicates** — content **byte-for-byte identical** in two blocks (e.g. the same section
  copy-pasted into a README and a Welcome page). Mechanical, not a judgment call, so it is **safe
  to consolidate autonomously *if* the human granted that up front**. Even then: repoint by ULID,
  keep one canonical block, never alter the wording.
- **Everything else is confirmation-gated.** Near-duplicates (the same fact in different words),
  drifted blocks, or any merge where you'd choose which nuance to keep need the human's approval in
  the itemized review — these take judgment, and judgment is the human's.

One rule holds regardless of the up-front answer: **never drop information to "tidy up".** If two
blocks each hold a nuance, that is content to **preserve**, not a reason to delete one. When unsure
whether two blocks are the same fact — or whether a duplicate is truly exact — **ask**.
