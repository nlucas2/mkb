---
title: "README: Choosing your vault"
tags: [doc, readme]
---

### Choosing your vault

Every client (CLI, MCP, web, desktop app) acts on **one vault at a time**, resolved in this order:

1. an explicit `--vault <dir>` flag (the CLI/MCP/web also accept `--remote`/`--socket`);
2. the `MDKB_VAULT` environment variable;
3. the **default** in your vault registry (see below);
4. the built-in fallback `~/mdkb-vault`.

So you can always be explicit (`mdkb search --vault ~/notes "…"`), or set a default once and drop
the flag entirely (`mdkb search "…"`).

**The registry file (optional).** Clients read a small per-user JSON file listing your vaults and
which is the default. You don't need to create it — if it's absent, the built-in `~/mdkb-vault`
default is used. The desktop app writes it for you when you pick a vault in its settings, and you
can also hand-edit it like any dotfile. It lives at:

- macOS: `~/Library/Application Support/mdkb/vaults.json`
- Linux: `~/.config/mdkb/vaults.json` (or `$XDG_CONFIG_HOME/mdkb/`)
- Windows: `%APPDATA%\mdkb\vaults.json`
- override the directory with `$MDKB_CONFIG_DIR`.

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

A `local` vault is a directory (a leading `~` is expanded, so the same file works across machines);
a `remote` vault is a token-gated `mdkbd --listen` over TCP. If `default` names a vault that isn't
in the list, mdkb falls back to the built-in default rather than guessing.
