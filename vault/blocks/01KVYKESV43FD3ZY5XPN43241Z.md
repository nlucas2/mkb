---
title: CONTRIBUTING page
tags: [doc/contributing, page, dev]
updated: 2026-07-07T05:18:34Z
---

# Contributing to mkb

This page collects the developer-facing material — workspace layout, internals, and the roadmap.
Two companions go deeper: the design rationale is in
[[01KVKJM4J0Z8A822SA8GVF8G8B|docs/architecture.md]] and the exact on-disk format in
[[01KVKBYMJS5MGCVSSFR8Z6H4MK|docs/SPEC.md]]. The **mandatory working rules** (tests, the shared-core boundary,
docs-as-data, the pre-commit gate) live in [[01KVKH5H7VGN3SAVZC2YQCK236|AGENTS.md]] — read it before sending a
change.

## Workspace layout

![[01KVKH5GX94BYEDTNEMA5W1NHS]]

### What each crate/module does

![[01KVKJ1R9MMSB6C1T9FBTDFNRG]]

![[01KVKJ1R6TX2CP1QPB3HDVKYP2]]

![[01KVKJ1R8416YVA07D34004351]]

## Under the hood

How clients reach and manage the daemon. You rarely touch this — clients auto-start and self-reap
the daemon for you — but it matters when hacking on the daemon or deploying it.

![[01KVKJ1R2KE4ZCAPJM9C25ZBV6]]

![[01KVKJ1RCJDFP7KFY56ENW3PR1]]

![[01KVKBYMCVYWADCHVEJFBGZR8Z]]

![[01KVKBYMDS9STTDCAVZMYDVPHG]]

![[01KVKJ1RH7H3T01HXTN3HTQRAT]]

## Working rules

These are mandatory; the canonical copy is [[01KVKH5H7VGN3SAVZC2YQCK236|AGENTS.md]], generated from the same
blocks. In short: every behavior change ships with tests, `cargo test --workspace` is green before
every commit, shared behavior lives in `mkb-core` (clients stay thin), and generated docs are
edited at their **source block** then re-exported — never by hand.
