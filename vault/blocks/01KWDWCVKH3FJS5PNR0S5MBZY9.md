---
title: USAGE page
updated: 2026-07-07T05:55:51Z
---

# Using mkb

A tour of the day-to-day features beyond create / read / edit: how to **search**, **browse and
organize**, protect **human-only** blocks, and **export** any slice of the vault to Markdown.

Everything here lives in `mkb-core`, so it behaves identically from the CLI (`mkb …`), the desktop
app, and an AI client through the MCP tools — pick whichever interface fits the task. CLI examples
below drop the `--vault <dir>` flag; add it (or set `$MKB_VAULT`, or a registry default) to target
a specific vault.

![[01KWK2J9912X51Q97XMYETE4FH]]

![[01KWDWCW4PFHF3YP6CPZ8G5EBH]]

![[01KWDWCWKABSCHDTDZTPKB6T1V]]

![[01KWXJB2NRM8G5QN8WZC8G0GHW]]

![[01KVP1DEBEDM8G5BXCB3JHC6VM]]

## Exporting & publishing

`mkb export` renders blocks (with their embeds resolved inline) to flat Markdown files, so a slice
of the vault becomes ordinary documents anyone can read without mkb. See the 
[[01KWXGS7X4EQR76GADKD8VBT0J|Export Guide]] for the full mechanics of `path` properties, manifests,
and cross-document link resolution.
