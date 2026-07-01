---
title: "Config: the daemon (mkbd)"
tags: [doc, config]
updated: 2026-07-01T03:18:33Z
---

## The daemon (`mkbd`)

The desktop app and the CLI **auto-start** a background `mkbd` for a local vault and reuse it, so
you rarely run it by hand. When you do — a service unit, a remote host, or tuning — these are its
flags:

| Flag | Purpose |
|------|---------|
| `--vault <DIR>` | Vault to serve (default: `$MKB_VAULT`, else `~/mkb-vault`). |
| `--db <PATH>` | Index database (default: a machine-local per-vault dir). |
| `--socket <PATH>` | Local socket / Windows named pipe (default: beside `--db`). |
| `--listen <ADDR>` | **Also** serve over TCP, e.g. `0.0.0.0:7820` — opt-in, and **fails closed** without a token. |
| `--token <TOKEN>` | Shared token network clients must present (`$MKB_TOKEN` is also accepted). |
| `--idle-timeout <SECS>` | Self-shut down after this many seconds with no requests **and** no interactive lease (`0` = never; the default when run manually). |

The index, socket, lock, and log are machine-local and live **outside** the vault by default —
under the OS local-data dir, keyed by a hash of the vault path — so a cloud-synced vault never
syncs the live index (set `$MKB_INDEX_DIR` to relocate the base). The network listener is off
unless `--listen` is given, and refuses to start without a token, so a local vault is never exposed
by accident.
