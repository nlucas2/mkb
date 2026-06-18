# mdkb desktop shell (Tauri)

A thin [Tauri](https://tauri.app) desktop front-end for mdkb.

It is **not** part of the main cargo workspace (it needs the Tauri toolchain and a system
webview, which the core product doesn't require). It is wired to the same shared crates as
every other mdkb front-end:

- `mdkb-protocol` — talks to a running `mdkbd` daemon.
- `mdkb-view` — turns resolved Markdown into HTML.

Because rendering goes through `mdkb-view`, the desktop UI shows exactly what the web UI
(`mdkb-web`) shows — there is no second rendering path to drift out of sync.

## Architecture

```
┌─────────────┐   Tauri IPC    ┌────────────────────┐   Unix socket   ┌────────┐
│  ui/ (HTML) │ ─────────────▶ │ src-tauri (Rust)   │ ──────────────▶ │ mdkbd  │
│  invoke()   │                │ commands → mdkb-view│                │ Service│
└─────────────┘                └────────────────────┘                 └────────┘
```

The Rust side exposes four commands (`list_pages`, `render_page`, `title_for`, `search`);
all real work happens in the daemon and the shared crates.

## Prerequisites

- Rust (stable)
- The Tauri CLI: `cargo install tauri-cli --version '^2'`
- A running daemon for your vault: `mdkbd --vault /path/to/vault`
  (the shell connects to `$MDKB_VAULT`'s socket, or the default `~/mdkb-vault`).
- App icons under `src-tauri/icons/` (generate with `cargo tauri icon <png>`).

## Run (development)

```sh
# Local daemon:
export MDKB_VAULT=/path/to/your/vault
mdkbd --vault "$MDKB_VAULT" &          # start the daemon
cd app/mdkb-tauri
cargo tauri dev                         # build + launch the desktop window
```

### Connecting to a remote daemon

The desktop app resolves its connection from the environment (via the shared
`mdkb_protocol::Client::from_env`), so it can talk to a daemon deployed in your cluster:

```sh
export MDKB_REMOTE=mdkbd.example:7820   # host:port of the remote mdkbd --listen
export MDKB_TOKEN=<the shared token>    # required for remote (token-gated)
cd app/mdkb-tauri
cargo tauri dev
```

If `MDKB_REMOTE` is unset it falls back to the local socket for `MDKB_VAULT` (or
`~/mdkb-vault`). `MDKB_SOCKET` can override the socket path explicitly.

## Build a bundle

```sh
cd app/mdkb-tauri
cargo tauri build
```

## Note

This directory is excluded from the root `cargo` workspace (see the root `Cargo.toml`
`exclude` list), so `cargo test --workspace` at the repository root does not require the
Tauri toolchain. Build the desktop shell from within this directory as shown above.
