---
title: "SPEC: Layout"
tags: [spec, doc]
---

## Layout

```
<vault>/
  blocks/                 # the knowledge: one file per block
    <ULID>.md
    <ULID>.md
    ...
  .mdkb/                  # rebuildable cache + local config (safe to delete)
    index.db              # SQLite index (FTS5 + vectors); rebuilt from blocks/
    config.json           # embedder configuration (optional)
    mdkbd.sock            # daemon socket (local mode)
    mdkbd.lock            # exclusive lock: at most one daemon per vault
    mdkbd.log             # daemon log (local mode)
  SPEC.md                 # this file
```

Anything under `.mdkb/` can be deleted; the daemon rebuilds it from `blocks/` on next start.
