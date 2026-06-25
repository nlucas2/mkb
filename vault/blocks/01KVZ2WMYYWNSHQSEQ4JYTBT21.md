---
title: "Config: multiple vaults (the registry)"
tags: [doc, config]
updated: 2026-06-25T09:48:47Z
---

## Multiple vaults (the registry)

Every client acts on **one vault at a time** — `--vault <dir>`, else `$MKB_VAULT`, else the registry default, else the built-in `~/mkb-vault`. To name several vaults and choose which is the default, clients read a small per-user JSON registry. You don't need to create it — if it's absent, the built-in `~/mkb-vault` default is used. The desktop app writes it for you when you pick a vault in its settings, and you can also hand-edit it like any dotfile. It lives at:

- macOS: `~/Library/Application Support/mkb/vaults.json`
- Linux: `~/.config/mkb/vaults.json` (or `$XDG_CONFIG_HOME/mkb/`)
- Windows: `%APPDATA%\mkb\vaults.json`
- override the directory with `$MKB_CONFIG_DIR`.

The format is a list of named vaults plus the `default` to use when no vault is specified:

```json
{
  "vaults": [
    { "name": "notes", "connection": { "mode": "local", "vault": "~/notes" } },
    { "name": "work",  "connection": { "mode": "remote", "host": "10.0.0.5:7820", "token": "…" } }
  ],
  "default": "notes"
}
```

A `local` vault is a directory (a leading `~` is expanded, so the same file works across machines); a `remote` vault is a token-gated `mkbd --listen` over TCP. If `default` names a vault that isn't in the list, mkb falls back to the built-in default rather than guessing.