---
title: "README: Install — release binary"
---

### Install: release binary

Download the latest archive for your platform from the **Releases** page and unpack it. Each
archive carries every binary — `mdkb` (CLI), `mdkbd` (daemon), `mdkb-mcp` (MCP server),
`mdkb-web` (web UI) — plus a `model/` directory beside them, so offline semantic search works
out of the box.

```sh
# Linux / macOS (example: macos-arm64 — also published: linux-amd64)
mkdir -p ~/.local/opt/mdkb
tar -xzf mdkb-<version>-macos-arm64.tar.gz -C ~/.local/opt/mdkb
export PATH="$HOME/.local/opt/mdkb:$PATH"     # keep the binaries beside model/
mdkb --help
```

On Windows, download `mdkb-<version>-windows-amd64.zip` (binaries) or the `…-setup.exe`
desktop installer. Keep the binaries together with the `model/` folder — the daemon looks for
`model/` beside its own executable.
