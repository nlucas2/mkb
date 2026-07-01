---
title: "Config: environment variables"
tags: [doc, config]
updated: 2026-07-01T03:18:32Z
---

## Environment variables

mkb honours a small set of environment variables — the canonical names the daemon and every
client read (so an override is seen the same everywhere). All are optional.

| Variable | What it does |
|----------|--------------|
| `MKB_VAULT` | Vault directory a client connects to / the daemon serves (a leading `~` is expanded). |
| `MKB_REMOTE` | `host:port` of a remote daemon to connect to over TCP (client side). |
| `MKB_TOKEN` | Shared token — presented by a client to a remote daemon, or required by a `--listen` daemon. |
| `MKB_SOCKET` | Explicit local socket path to dial instead of deriving one from the vault (client side). |
| `MKB_INDEX_DIR` | Base directory for the machine-local per-vault index dirs (overrides the OS local-data dir). |
| `MKB_CONFIG_DIR` | Per-user config directory holding the client's `vaults.json` registry. |
| `MKB_READY_TIMEOUT_SECS` | Seconds a client waits for a freshly auto-started daemon to answer its first ping — raise it on slow, network-backed, or CI storage where the initial reconcile is slow. |
| `MKB_BUNDLED_MODEL_DIR` | Directory holding an on-disk embedding model that overrides the compiled-in one. |
| `MKB_MCP_TOOLS` | `full` (or `all`) opts the MCP server into the advanced tool tier (`set_props`, `unset_props`, `carve_block`, `flatten_block`); the default is the lean core surface. |

The vault-selection precedence is `--vault` (flag) → `$MKB_VAULT` → the registry default → the
built-in `~/mkb-vault`; the connection variables (`MKB_REMOTE`/`MKB_TOKEN`/`MKB_SOCKET`) let a
client target a specific daemon without a registry entry.
