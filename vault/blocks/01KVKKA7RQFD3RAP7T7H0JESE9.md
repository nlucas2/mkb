---
title: Dedup skill page (CLI)
tags: [skill, skill-page, dedup, cli]
---

---
name: mdkb-dedup
description: >-
    Keep an mdkb knowledge base deduplicated and well-connected from the CLI: audit for
    near-duplicate facts and orphans with search + backlinks, and consolidate duplicates by ULID
    (repoint embedders, then delete) without breaking links. Read when tidying or merging knowledge
    in an mdkb vault.
user-invocable: false
---

# Keeping an mdkb vault deduplicated (CLI)

A healthy mdkb vault states each fact in **exactly one block**. This skill is the *repair* side
of that discipline — finding and consolidating duplication that already exists — using the `mdkb`
CLI (a thin client of the daemon). **It is human-driven: you audit and propose; you do not delete
or merge knowledge on your own** (see the rule below). Audit instruments: `search` (find
near-dupes in 2-3 phrasings), `backlinks <id>` (find orphans, and check before repointing/
deleting), `list` and `stats` (survey roots vs blocks), `update`/`delete` (repoint by ULID, then
remove the dup — **only once approved**), `export --check` (verify generated docs after a merge).

![[01KVJ1H7GKWTW2E0V690MRHSD2]]

![[01KVKKA7CD300KNCX4DMD59DPK]]

![[01KVKKA7FW5Y8EC3BSKCGWZ54E]]

![[01KVKKA7J12ZEZ7GBGCQ7V13SZ]]

![[01KVKKA7M6JTZHXF84C99DGWH0]]

![[01KVJ1H7J1B5J2PQTZ8XPQAF93]]

![[01KVKKA7PGPARG7CP6CRZWDDK1]]
