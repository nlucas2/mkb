---
title: "README: Where your vault lives"
tags: [doc/readme]
updated: 2026-07-07T04:37:25Z
---

### Where your vault lives — local or synced

Your vault is just a directory of Markdown files, and **the Markdown is the only thing you ever
need to sync**. How you store it is independent of how you installed mkb.

- **Local (single machine).** Point a client at any folder (`--vault ~/notes`) and go. Nothing
  else to set up.
- **Synced across machines (OneDrive, Syncthing, Dropbox, iCloud…).** Put the vault folder inside
  your synced location and use it from each machine. Because the synced path is usually under your
  home directory, a `~`-relative vault entry in the registry (e.g. `~/OneDrive/notes`) resolves
  correctly everywhere, so one config can be shared.

This is safe because mkb's search index lives **outside** the vault, on each machine — so syncing
the Markdown never drags a live database between machines, and every machine rebuilds its own index
from the same notes. **Sync only the Markdown, never the index.** (If your sync tool leaves a
conflict copy, mkb won't index it — `mkb conflicts` surfaces it to merge in plain text.)
