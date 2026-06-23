---
title: "README: Install — prebuilt"
---

### Install: prebuilt (recommended)

**The installer — easiest.** Download the installer for your OS from the **Releases** page and run
it: `.dmg` (macOS), `…-setup.exe` (Windows), `.deb` or `.AppImage` (Linux). It installs the
desktop app together with the `mdkb` CLI and the `mdkb-mcp` server.

**Portable binaries — no installer, or for servers.** Each platform also ships one archive that is
the **complete product**: the desktop app plus every binary — `mdkb` (CLI), `mdkbd` (daemon),
`mdkb-mcp` (MCP server), `mdkb-web` (web UI) — and a `model/` directory, so offline semantic
search works out of the box. Extract it wherever you keep apps and put that folder on your `PATH`:

```sh
# macOS / Linux (example: macos-arm64 — also: linux-amd64, linux-arm64-headless)
mkdir -p ~/Applications/mdkb
tar -xzf mdkb-<version>-macos-arm64.tar.gz -C ~/Applications/mdkb
# add it to PATH permanently (pick your shell's rc file)
echo 'export PATH="$HOME/Applications/mdkb:$PATH"' >> ~/.zprofile   # or ~/.bashrc / ~/.profile
exec "$SHELL" -l        # reload, then:
mdkb --help
```

On Windows, download `mdkb-<version>-windows-amd64.zip` and extract it; add that folder to your
`PATH` (Settings → *Edit environment variables*) to run `mdkb` from any terminal.

The daemon finds the embedding model in the `model/` folder **beside the binaries** — zero config,
which is why they travel together. To keep the binaries somewhere already on `PATH` (e.g.
`~/.local/bin`) and the model elsewhere, set `MDKB_BUNDLED_MODEL_DIR` to the model directory.

**Prebuilt availability.** The complete archive (with the desktop app) is published for
**Linux amd64**, **macOS** (Apple Silicon), and **Windows x64**. We don't currently publish a
**prebuilt Linux arm64 desktop** binary — arm64 ships a `…-linux-arm64-headless.tar.gz` (daemon
+ CLIs + model) and the multi-arch daemon container image. This is only about prebuilt
*releases*: the desktop app builds and runs fine on arm64 Linux from source (`cargo tauri
build`, see below), and the daemon container image covers running it as a server (see `deploy/`).
