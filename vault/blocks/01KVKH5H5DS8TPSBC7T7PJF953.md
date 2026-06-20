---
title: Pre-commit checklist
tags: [doc, contributing]
---

1. `cargo fmt --all` — code is formatted.
2. `cargo clippy --workspace --all-targets -- -D warnings` — no lints.
3. `cargo test --workspace` — **all green**.
4. `mdkb export vault --check` passes; generated docs were regenerated (edit the block, not the
   file). Any not-yet-generated docs updated to match the current state.
5. New/changed logic has tests committed alongside it.
6. No duplicated logic; shared behavior lives in `mdkb-core`.
7. Commit message explains the *why*, not just the *what*.

Only commit when **all** boxes are satisfied.
