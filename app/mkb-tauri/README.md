# mkb desktop shell (Tauri)

The desktop app is **the human's window into the vault** — where you read, review, edit, and
reorganize the knowledge mkb holds, including whatever an AI client has written over MCP. It is
a **full editor and knowledge-graph browser**, not just a viewer; mkb is a tool you and the AI
co-manage as equals, and this is your side of it (the AI's side is the MCP server).

Architecturally it stays a **thin client**: all knowledge-base behavior (block parsing,
transclusion, indexing, the link graph, writes) lives in `mkb-core` and is reached over the
wire through `mkb-protocol`; HTML rendering goes through `mkb-view`. There is no second copy of
engine behavior here, so the desktop UI can never drift from the MCP server (see
the repo `AGENTS.md`).

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
  highlight neighbours. The graph is computed by `mkb-core` (`link_graph`); the UI only draws
  it (vendored [`force-graph`](https://github.com/vasturiano/force-graph), no network at runtime).
- **Linked references** — each page lists the pages that link to it (and the pages it links to).
- **New / delete page** from the sidebar.
- **Settings** (no environment variables) — see below.

## Connection (Settings)

The connection is configured in-app and saved to the shared per-user vault registry
(`~/Library/Application Support/mkb/vaults.json` on macOS; the XDG/`APPDATA` equivalent
elsewhere). This is the same registry the CLI and MCP server read, so configuring the vault here
also sets the default for those tools. Click the connection-status dot at the bottom of the
sidebar.

- **Local vault** — pick a folder. The app **auto-starts a background `mkbd`** for that vault
  (the daemon is bundled inside the app) and reuses it next time. The daemon is started
  **detached**, so it **keeps running after you quit the app** — relaunching just reconnects.
- **Remote daemon** — `host:port` of a `mkbd --listen` plus its shared token.

Changes apply immediately (the client reconnects without restarting the app).

## Tauri commands (thin glue)

`list_pages`, `render_page`, `page_blocks`, `page_source`, `graph`, `search`, `title_for`
(reads); `save_block`, `append_block`, `save_page`, `delete_page` (writes); `get_settings`,
`save_settings`, `connection_status`, `pick_vault` (settings). Each just forwards to the
daemon / shared crates.

## Prerequisites

- Rust (stable) + the Tauri CLI: `cargo install tauri-cli --version '^2'`
- App icons: the whole `src-tauri/icons/` tree is **generated** from the single source
  `app-icon.png` and is git-ignored. Generate it (once per checkout, or after editing the
  source) with `cargo tauri icon app-icon.png` — the bundling builds do this automatically.

## Build a bundle

The bundle ships the daemon binary as a resource so local-vault mode works out of the box, and
its icons are generated from the single source. Generate icons + stage the daemon first, then
build:

```sh
# from the repo root: build the daemon and stage it into the app
cargo build --release -p mkbd
mkdir -p app/mkb-tauri/src-tauri/bin
cp target/release/mkbd app/mkb-tauri/src-tauri/bin/mkbd

# generate the icon set from the single source (icons/ is git-ignored build output)
cd app/mkb-tauri
cargo tauri icon app-icon.png

# build the app (produces mkb.app + a .dmg under src-tauri/target/release/bundle/)
cargo tauri build
```

(`src-tauri/bin/` and `src-tauri/icons/` are git-ignored — both are build inputs regenerated
from the workspace: the daemon from `cargo build`, the icons from `app-icon.png`.)

## macOS: Gatekeeper & the "damaged" prompt

The CI-built app is **ad-hoc signed and not notarized** (no Apple Developer ID). macOS attaches a
`com.apple.quarantine` flag to anything downloaded via a browser, and on Apple Silicon an
un-notarized quarantined app is rejected with the misleading **"mkb.app is damaged and can't be
opened."** Two ways around it:

- **Build from source** (recommended on macOS): `cargo tauri build` produces an app you compiled
  locally, so it carries no quarantine flag and opens normally — no notarization needed.
- **Use the downloaded `.dmg`**: clear the quarantine flag once after copying the app out, e.g.

  ```sh
  xattr -dr com.apple.quarantine /Applications/mkb.app
  ```

To ship a `.dmg` that opens cleanly with no workaround, the GitHub release workflow has optional
Developer ID signing + notarization wired in (inert until the `APPLE_*` secrets are configured);
see the comments in `.github/workflows/release.yml`.

## Run (development)

```sh
cd app/mkb-tauri
cargo tauri dev
```

On first launch with no saved config it defaults to the local default vault (`~/mkb-vault`)
and auto-starts a daemon for it. Use **Settings** to point it elsewhere.

## Note

This directory is excluded from the root `cargo` workspace (see the root `Cargo.toml`
`exclude` list), so `cargo test --workspace` at the repository root does not require the
Tauri toolchain.
