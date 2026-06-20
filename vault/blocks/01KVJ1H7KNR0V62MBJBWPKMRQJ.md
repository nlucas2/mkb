---
title: Skill: write safety
tags: [skill, shared]
---

## Safe writes - don't destroy knowledge

The body-mutating write replaces the **entire** body. Treat it as a deliberate rewrite,
never a quick tweak:

1. Read the block's current body.
2. Modify the full text locally (to *add*, append to what you read).
3. Write back the **complete** new body.

Never send only a new line - that erases everything else. Managed (frontmatter) tags are
set separately and are preserved across body edits. Never create a second block that
restates an existing one; edit the original so every embedder updates at once.
