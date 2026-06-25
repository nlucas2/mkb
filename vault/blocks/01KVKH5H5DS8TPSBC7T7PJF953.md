---
title: Pre-commit checklist
tags: [doc, contributing]
updated: 2026-06-25T10:14:29Z
---

1. `cargo fmt --all` — code is formatted.
2. `cargo clippy --workspace --all-targets -- -D warnings` — no lints.
3. `cargo test --workspace` — **all green**.
4. `mdkb export --vault vault --check` passes; generated docs were regenerated (edit the block, not the
   file). Any not-yet-generated docs updated to match the current state.
5. New/changed logic has tests committed alongside it.
6. If the change completes or materially advances a **roadmap** item, reconcile that entry in the
   same change — mark it done or narrow it to the true remaining gap, so the roadmap reflects
   reality (the roadmap is data too, like the docs and tests).
7. No duplicated logic; shared behavior lives in `mdkb-core`.
8. Commit message explains the *why*, not just the *what*.

Only commit when **all** boxes are satisfied.
