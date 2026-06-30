---
title: "Skill: the edit-block to re-export loop"
tags: [skill, doc]
---

### The loop: edit the block, then re-export

1. **Read the banner** of the generated file to get the source block ULID.
2. **Read the block** (`mkb get --vault vault <id>`, or `get_block` over MCP) so you edit from its current
   content — a write replaces the **whole** body.
3. **Check what it feeds before editing a shared block:** `mkb backlinks --vault vault <id>` (or
   `backlinks`) lists every block that embeds it, so you know which docs your edit will change.
4. **Edit the block** (`mkb update --vault vault <id> < body`, or `update_block`; the UI edits in place).
5. **Re-export:** run `mkb export --vault vault` in the shell — it regenerates every mapped doc. (Export
   is a CLI/maintenance action; there is no MCP export tool, so an MCP-only agent runs it via the
   shell or asks the human.)
6. **Verify:** `mkb export --vault vault --check` writes nothing and exits non-zero on drift. Commit the
   block change and the regenerated file(s) **together**.
