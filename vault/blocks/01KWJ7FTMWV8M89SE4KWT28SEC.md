---
title: Working with the vault
tags: [doc/contributing, dev]
updated: 2026-07-04T20:40:00Z
---

## Working with the vault (mkb's own knowledge)

This repo **dogfoods mkb**: its human-facing docs — `README.md`, this `AGENTS.md`, `docs/SPEC.md`,
`docs/CONFIGURATION.md`, `docs/USAGE.md`, and the `docs/skills/` — are generated from blocks in
`vault/` (docs-as-data; see the *Docs are data* rule below). So to change any of them you read and
write the **vault**, not the generated file. Three ways, in order of preference when available:

- **The mkb MCP tools** — this repo ships a scoped **`.mcp.json`**, so an MCP-capable agent opened
  in it (e.g. the GitHub Copilot CLI) auto-gets an `mkb-repo` server pinned to this checkout's vault
  (`search`, `get_block`, `create_block`, `update_block`, `replace_in_block`, `link_blocks`, …). It's
  named `mkb-repo` (not `mkb`) so it won't collide with a personal `mkb` server you may already run
  for your own knowledge base. It runs `just mcp`, which prefers an installed `mkb-mcp` and falls back
  to `cargo run` from source.
- **The `mkb` CLI** from the repo root: `mkb --vault vault <cmd>`. The `vault` path is relative to
  the working directory, so it resolves correctly in **any clone or git worktree** with no per-repo
  setup.
- **`cargo run` when `mkb` isn't installed** (e.g. a fresh contributor with only the source tree):
  `cargo run -p mkb-cli -- <cmd> --vault vault` — the same arguments go after the `--`.

The `docs/skills/` skills teach the full workflow (`mkb-cli` / `mkb-knowledge` for reading &
writing knowledge, `mkb-docs-as-data` for the generated docs). The short version: **search before
you write** (a fact lives in exactly one block — don't fork a near-duplicate), **edit the source
block, never the generated file**, then run `mkb export --vault vault` and commit the regenerated
file(s) in the **same** change.

To automate that last step, run `just hooks` **once per clone**: it points git at the tracked
`.githooks/`, whose `pre-commit` hook regenerates the docs and re-stages them whenever a commit
touches `vault/` — so a generated file can never drift from its source block. It's a no-op on
pure-code commits, and CI's `mkb export --vault vault --check` remains the hard backstop.