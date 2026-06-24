---
title: "SPEC: Layout"
tags: [spec, doc]
---

## Layout

```
<vault>/                  # the ONLY thing meant to sync (cloud sync / git)
  blocks/                 # the knowledge: one file per block
    <ULID>.md
    <ULID>.md
    ...
  SPEC.md                 # this file (in mdkb's own self-documenting vault)
```

The index, socket, lock, and log are **machine-local** and live **outside** the vault, in a
per-vault directory under the OS local-data location — so a cloud-synced vault never syncs the
live index:

```
<base>/mdkb/<vault-id>/   # base: %LOCALAPPDATA% (Win), ~/Library/Application Support (macOS),
    index.db              #   ~/.local/state (Linux); <vault-id> hashes the vault path
    config.json           # embedder configuration (optional)
    mdkbd.sock            # daemon socket (local mode)
    mdkbd.lock            # exclusive lock: at most one daemon per vault
    mdkbd.log             # daemon log (local mode)
```

Set `$MDKB_INDEX_DIR` to override the base. With no resolvable home (a minimal container), it
falls back to the legacy in-vault `<vault>/.mdkb/`. Anything in the index directory is a
rebuildable cache — delete it and the daemon rebuilds from `blocks/` on next start.
