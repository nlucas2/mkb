---
title: "README: The vault"
tags: [doc, readme]
---

## The vault (`vault/`)

mdkb's own knowledge lives in [`vault/`](./vault) as interlinked blocks — it is the project's
real knowledge base **and** a self-documenting demo: it explains how to use and run mdkb *using*
mdkb. Opening it *is* the tutorial; the run-guides all embed one shared note, so editing that
block once updates every guide (live transclusion). The human-facing docs in this repo are
**generated** from these blocks (see *Docs are generated* below).

```sh
# point the daemon at it (or set the desktop app's Settings → Local vault to this folder)
cargo run -p mdkbd -- --vault vault
```
