---
title: "SPEC: Human-only (locked) blocks"
tags: [spec, doc]
---

## Human-only (locked) blocks

A block may be marked **human-only** by adding `locked: true` to its frontmatter:

```markdown
---
title: My hand-curated notes
locked: true
---
```

A locked block is **fully readable by every client** (search, get, render) but **immutable
through the write API to all of them** — there is no write-through, not even for the app. Any
mutation of a locked block (update, set-tags, delete, carve-from, flatten, link-into) is refused.
To change one, a human **unlocks** it, edits, then optionally re-locks.

Locking is governed by an authorization scope. Capabilities are `Read`, `Write`, and
`ManageLocks` (lock/unlock); a client holds a subset. The CLI and MCP server are **agents**
(`Read` + `Write`) — they can read a locked block but not modify it, and cannot lock or unlock.
Only the **desktop app** holds `ManageLocks`, so lock state is toggled there (or by a human
editing the `locked:` line directly). This is a local-trust guardrail, not a security boundary:
the vault files are the source of truth, so a determined human (or a process with raw file access)
can always edit the Markdown — the lock keeps *tool clients*, especially AI, from casually
changing curated content.
