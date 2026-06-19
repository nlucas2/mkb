# mdkb desktop shell (Tauri)

A [Tauri](https://tauri.app) desktop app for mdkb. It is a **full editor and knowledge-graph
browser**, not just a viewer — but it stays a **thin client**: all knowledge-base behavior
(block parsing, transclusion, indexing, the link graph, writes) lives in `mdkb-core` and is
reached over the wire through `mdkb-protocol`; HTML rendering goes through `mdkb-view`. There
is no second copy of engine behavior here, so the desktop UI can never drift from the web UI
or the MCP server (see the repo `AGENTS.md`).

It is **not** part of the main cargo workspace (it needs the Tauri toolchain and a system
webview, which the core product doesn't require).

## What it does

- **Read / Blocks / Raw** per page:
  - *Reading* — the page rendered with transclusions resolved (read-only).
  - *Blocks* — an editable block list; click any block to edit its Markdown in place
    (`Cmd/Ctrl+Enter` or blur to save, `Esc` to cancel), and `＋ Add block` to append.
  - *Raw* — edit the whole `.md` source and save.
- **Knowledge graph** — a force-directed page graph (nodes = pages sized by link degree,
  edges = `[[references]]` / `![[transclusions]]`). Click a node to open the page; hover to
  highlight neighbours. The graph is computed by `mdkb-core` (`link_graph`); the UI only draws
  it (vendored [`force-graph`](https://github.com/vasturiano/force-graph), no network at runtime).
- **Linked references** — each page lists the pages that link to it (and the pages it links to).
- **New / delete page** from the sidebar.
- **Settings** (no environment variables) — see below.

## Connection (Settings)

The connection is configured in-app and saved to a per-user file
(`~/Library/Application Support/dev.mdkb.desktop/connection.json` on macOS; the XDG/`APPDATA`
equivalent elsewhere). Click the connection-status dot at the bottom of the sidebar.

- **Local vault** — pick a folder. The app **auto-starts a background `mdkbd`** for that vault
  (the daemon is bundled inside the app) and reuses it next time. The daemon is started
  **detached**, so it **keeps running after you quit the app** — relaunching just reconnects.
- **Remote daemon** — `host:port` of a `mdkbd --listen` plus its shared token.

Changes apply immediately (the client reconnects without restarting the app).

## Tauri commands (thin glue)

`list_pages`, `render_page`, `page_blocks`, `page_source`, `graph`, `search`, `title_for`
(reads); `save_block`, `append_block`, `save_page`, `delete_page` (writes); `get_settings`,
`save_settings`, `connection_status`, `pick_vault` (settings). Each just forwards to the
daemon / shared crates.

## Prerequisites

- Rust (stable) + the Tauri CLI: `cargo install tauri-cli --version '^2'`
- App icons under `src-tauri/icons/` (generate with `cargo tauri icon <png>`).

## Build a bundle

The bundle ships the daemon binary as a resource so local-vault mode works out of the box.
Stage it first, then build:

```sh
# from the repo root: build the daemon and stage it into the app
cargo build --release -p mdkbd
mkdir -p app/mdkb-tauri/src-tauri/bin
cp target/release/mdkbd app/mdkb-tauri/src-tauri/bin/mdkbd

# build the app (produces mdkb.app + a .dmg under src-tauri/target/release/bundle/)
cd app/mdkb-tauri
cargo tauri build
```

(`src-tauri/bin/` is git-ignored — it is a build input, rebuilt from the workspace.)

## Run (development)

```sh
cd app/mdkb-tauri
cargo tauri dev
```

On first launch with no saved config it defaults to the local default vault (`~/mdkb-vault`)
and auto-starts a daemon for it. Use **Settings** to point it elsewhere.

## Note

This directory is excluded from the root `cargo` workspace (see the root `Cargo.toml`
`exclude` list), so `cargo test --workspace` at the repository root does not require the
Tauri toolchain.
