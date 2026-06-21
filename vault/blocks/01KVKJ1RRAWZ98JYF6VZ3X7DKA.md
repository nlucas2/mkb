---
title: README page
tags: [doc, doc-page, user]
---

# mdkb — Markdown Knowledge Base

![[01KVHJ76YA04MEM71HNDB7RT8G]]

> Status: core re-architected to the file-per-block model (parser, transclusion, index,
> semantic search, daemon, MCP, web + desktop UIs). Versioned `0.0.0` / pre-release. See
> **[`docs/architecture.md`](./docs/architecture.md)** for the design and
> **[`docs/SPEC.md`](./docs/SPEC.md)** for the exact on-disk format.

## Getting started

![[01KVM9NPR2HD2WF05GKFYNMG68]]

![[01KVM9NQ2N1BKBJHS9TJQCW5KB]]

![[01KVM9NQBWBH5NXSPAS1ZA7NHE]]

![[01KVKJ1R5D3GMJ7GCVYVKN04X2]]

![[01KVKJ1R428705BJD9VXGTRSDV]]

## Using mdkb

![[01KVKJ1RB2HP9V609P81AWWS41]]

### From an AI client (MCP)

![[01KVKJ1QYRSVX6DW8B575BW44X]]

For guidance on using mdkb *well* as an AI client — the DRY/transclusion principle, the process
for adding knowledge, and effective search patterns — see the example skill at
[`docs/skills/mdkb-knowledge/SKILL.md`](./docs/skills/mdkb-knowledge/SKILL.md).

![[01KVKJ1RFN39VZ0AXVRJ3VHMFB]]

![[01KVKJ1RE0RG49PD09PMJBGTDF]]

![[01KVKJ1RM8647XQC65WD0G37YN]]

## Under the hood

Implementation details most users never touch — clients auto-start and self-reap the daemon for
you; the vault Markdown is the only thing you manage.

![[01KVKJ1R2KE4ZCAPJM9C25ZBV6]]

![[01KVKJ1RCJDFP7KFY56ENW3PR1]]

![[01KVKBYMCVYWADCHVEJFBGZR8Z]]

![[01KVKBYMDS9STTDCAVZMYDVPHG]]

## Workspace layout

![[01KVKH5GX94BYEDTNEMA5W1NHS]]

### What each crate/module does

![[01KVKJ1R9MMSB6C1T9FBTDFNRG]]

![[01KVKJ1R6TX2CP1QPB3HDVKYP2]]

![[01KVKJ1R8416YVA07D34004351]]

![[01KVKJ1RH7H3T01HXTN3HTQRAT]]

![[01KVM9NQP8GEPRXQRKK062E37R]]

## Contributing

These rules are mandatory; the canonical copy is **[`AGENTS.md`](./AGENTS.md)**, generated from
the **same blocks** embedded below — so editing a rule once updates both.

![[01KVKH5GYBEGZVMQQYG4Q7SE8E]]

![[01KVKH5GZDV9MPQKCZ4VM6GQM9]]

![[01KVKH5H0H88ZTK52FBH60AWJG]]

![[01KVKH5H1R0129ZVGF76MAQZB9]]

![[01KVKH5H2ZH4890TK2G6NRR2EH]]

![[01KVKH5H48K7CNNXG1HH958MJ6]]

### Pre-commit checklist (run top to bottom)

![[01KVKH5H5DS8TPSBC7T7PJF953]]

### Commit hygiene

![[01KVKH5H6NV4178V7WGMQEXAHT]]
