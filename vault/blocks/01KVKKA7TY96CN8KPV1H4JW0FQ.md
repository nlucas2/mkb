---
title: Dedup skill page (MCP)
tags: [skill, skill-page, dedup, mcp]
updated: 2026-06-30T23:06:31Z
path: docs/skills/mkb-dedup-mcp
filename: SKILL.md
---

---
name: mkb-dedup-mcp
description: >-
    Keep an mkb knowledge base deduplicated and well-connected via the MCP server: audit for
    near-duplicate facts and orphans with search + backlinks, and consolidate duplicates by ULID
    (repoint embedders, then delete) without breaking links. Read when tidying or merging knowledge
    in an mkb vault.
user-invocable: false
---

# Keeping an mkb vault deduplicated (MCP)

A healthy mkb vault states each fact in **exactly one block**. This skill is the *repair* side
of that discipline — finding and consolidating duplication that already exists — using the MCP
tools. **It is human-driven: you audit and propose; you do not delete or merge knowledge on your
own** (see the rule below). Audit instruments: `search` (find near-dupes in 2-3 phrasings),
`backlinks` (find orphans, and check before repointing/deleting), `list_roots` and `stats`
(survey roots vs blocks), `update_block`/`delete_block` (repoint by ULID, then remove the dup —
**only once approved**).

![[01KVJ1H7GKWTW2E0V690MRHSD2]]

![[01KVKKA7CD300KNCX4DMD59DPK]]

![[01KVKKA7FW5Y8EC3BSKCGWZ54E]]

![[01KVKKA7J12ZEZ7GBGCQ7V13SZ]]

![[01KVKKA7M6JTZHXF84C99DGWH0]]

![[01KVJ1H7J1B5J2PQTZ8XPQAF93]]

![[01KVKKA7PGPARG7CP6CRZWDDK1]]
