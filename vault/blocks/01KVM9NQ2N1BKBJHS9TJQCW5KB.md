---
title: "README: Install — release binary"
---

### Install: release binary

Download the latest archive for your platform from the **Releases** page and unpack it. Each
archive is the **complete product** — the desktop app, `mdkb` (CLI), `mdkbd` (daemon),
`mdkb-mcp` (MCP server), `mdkb-web` (web UI), and a `model/` directory beside them, so offline
semantic search works out of the box. Or run the native installer (`.dmg` / `…-setup.exe` /
`.deb` / `.AppImage`), which bundles the app together with the CLI tools.

```sh
# Linux / macOS (example: macos-arm64 — also published: linux-amd64)
mkdir -p ~/.local/opt/mdkb
tar -xzf mdkb-<version>-macos-arm64.tar.gz -C ~/.local/opt/mdkb
export PATH="$HOME/.local/opt/mdkb:$PATH"     # keep the binaries beside model/
mdkb --help
```

On Windows, download `mdkb-<version>-windows-amd64.zip` (the full product) or the `…-setup.exe`
desktop installer. Keep the binaries together with the `model/` folder — the daemon looks for
`model/` beside its own executable.

**Prebuilt availability.** The complete archive (with the desktop app) is published for
**Linux amd64**, **macOS** (Apple Silicon), and **Windows x64**. We don't currently publish a
**prebuilt Linux arm64 desktop** binary — arm64 ships a `…-linux-arm64-headless.tar.gz` (daemon
+ CLIs + model) and the multi-arch daemon container image. This is only about prebuilt
*releases*: the desktop app builds and runs fine on arm64 Linux from source (`cargo tauri
build`, see below), and the daemon container image covers running it as a server (see `deploy/`).
