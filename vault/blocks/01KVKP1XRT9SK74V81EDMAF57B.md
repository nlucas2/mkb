---
title: Docs-as-data skill page
tags: [skill, skill-page, doc, dev]
updated: 2026-07-01T03:20:15Z
---

---
name: mkb-docs-as-data
description: >-
    How to maintain docs-as-data in an mkb-backed repo: never hand-edit a generated file — find
    its source block from the @generated banner, edit the block (and the blocks it embeds), then
    re-run `mkb export` and verify with `--check`. Read before changing any generated doc.
user-invocable: false
---

# Maintaining generated docs (docs-as-data)

In an mkb-backed repo, some committed docs are **generated from blocks in the vault** — the vault
is the source of truth and `mkb export` renders chosen blocks to flat Markdown. Editing a
generated file directly is a mistake: it drifts from its source and is **clobbered on the next
export**. This skill is the workflow for changing those docs correctly.

![[01KVKH5H0H88ZTK52FBH60AWJG]]

![[01KVKP1XHGR00BPJBJZ51DR12W]]

![[01KVKP1XM4AZ3DPS25ZVZR8G22]]

![[01KVKP1XPDAWGSDEGAQ3S3KSJZ]]

![[01KWDV22A07NH4NVDFXF639GCC]]
