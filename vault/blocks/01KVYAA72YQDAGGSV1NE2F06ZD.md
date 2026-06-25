---
title: "README: Where your vault lives"
tags: [doc, readme]
---

### Where your vault lives — local or synced

Your vault is just a directory of Markdown files, and **the Markdown is the only thing you ever
need to sync**. How you store it is independent of how you installed mdkb.

- **Local (single machine).** Point a client at any folder (`--vault ~/notes`) and go. Nothing
  else to set up.
- **Synced across machines (OneDrive, Syncthing, Dropbox, iCloud…).** Put the vault folder inside
  your synced location and use it from each machine. Because the synced path is usually under your
  home directory, a `~`-relative vault entry in the registry (e.g. `~/OneDrive/notes`) resolves
  correctly everywhere, so one config can be shared.

The reason this is safe: the **index is not in the vault**. The SQLite index, socket, lock, and
log are machine-local — they live outside the vault (under the OS local-data dir, keyed by a hash
of the vault's path) and are fully rebuildable from the Markdown. So a synced vault never drags a
live database between machines (the classic cause of cloud-sync corruption); each machine keeps its
own index for the same notes. **Never sync the index** — only the Markdown.

If a sync tool produces a conflict copy (e.g. `note (conflicted copy).md` /
`note-DESKTOP-AB12.md`), mdkb deliberately **doesn't index it** and surfaces it via `mdkb conflicts
--vault <dir>` so you can merge it back in plain text. The Markdown stays authoritative.
