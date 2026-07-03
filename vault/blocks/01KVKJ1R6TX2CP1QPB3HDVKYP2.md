---
title: "README: The vault"
tags: [doc/readme]
updated: 2026-07-02T21:39:27Z
---

## The vault (`vault/`)

mkb's own knowledge lives in [`vault/`](./vault) as interlinked blocks — it is the project's
real knowledge base **and** a self-documenting demo: it explains how to use and run mkb *using*
mkb. Opening it *is* the tutorial; the run-guides all embed one shared note, so editing that
block once updates every guide (live transclusion). The human-facing docs in this repo are
**generated** from these blocks (see *Docs are generated* below).

```sh
# point the daemon at it (or add it in the desktop app's Settings → Vaults)
cargo run -p mkbd -- --vault vault
```
