# mkb task runner. Install `just` (https://github.com/casey/just), then run e.g. `just`,
# `just install`, or `just install-cli`. These recipes are the canonical build steps — the same
# icon → stage → bundle sequence the release CI runs — so "build it from source" is one command on
# any platform (notably arm64 Linux, which has no prebuilt desktop release).

app_dir := "app/mkb-tauri"
tauri   := app_dir / "src-tauri"

# `just` runs recipe lines with `sh` on every OS, but Windows has no `sh`. Point Windows at
# PowerShell so the plain `cargo …` recipes run there; the platform-specific recipes below use
# [unix]/[windows] variants with native tooling (osascript/dpkg vs. the NSIS installer).
# NOTE: the Windows variants are best-effort and have NOT yet been validated on a Windows host
# (see the roadmap); the macOS/Linux paths are the tested ones.
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# List the available recipes (default when you just run `just`).
default:
    @just --list

# Build the whole headless workspace (debug).
build:
    cargo build --workspace

# Run the full test suite (must be green before every commit — see AGENTS.md).
test:
    cargo test --workspace

# Format + lint + test: the pre-commit gate.
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

# Installs the WHOLE product: the headless tools (daemon, CLI, MCP server) onto ~/.cargo/bin,
# AND the desktop app (macOS → /Applications, Linux → .deb or a ~/.local AppImage, Windows → the
# NSIS installer, silent). mkb is one product — the app, the daemon it drives, and the MCP server
# an AI client uses all share one vault — so the default install gives you all of it. (Daemon-only
# is the container deployment, not this.)
# Install mkb — the desktop app, the daemon, the CLI, and the MCP server.
[unix]
install: install-cli app
    #!/usr/bin/env bash
    set -euo pipefail
    bundle="{{tauri}}/target/release/bundle"
    case "$(uname -s)" in
      Darwin)
        # Quit a running copy first — macOS blocks overwriting a running .app bundle.
        if osascript -e 'application "mkb" is running' 2>/dev/null | grep -q true; then
          echo "Quitting running mkb…"
          osascript -e 'quit app "mkb"' 2>/dev/null || true
          for _ in $(seq 1 20); do
            osascript -e 'application "mkb" is running' 2>/dev/null | grep -q true || break
            sleep 0.25
          done
        fi
        echo "Installing mkb.app → /Applications"
        rm -rf /Applications/mkb.app
        cp -R "$bundle/macos/mkb.app" /Applications/mkb.app
        echo "Done. Launch mkb from /Applications (or Spotlight)." ;;
      Linux)
        deb=$(ls "$bundle"/deb/*.deb 2>/dev/null | head -1 || true)
        appimage=$(ls "$bundle"/appimage/*.AppImage 2>/dev/null | head -1 || true)
        if [ -n "$deb" ] && command -v dpkg >/dev/null 2>&1; then
          echo "Installing $deb (sudo dpkg -i)"
          sudo dpkg -i "$deb"
        elif [ -n "$appimage" ]; then
          mkdir -p "$HOME/.local/bin"
          cp "$appimage" "$HOME/.local/bin/mkb.AppImage"
          chmod +x "$HOME/.local/bin/mkb.AppImage"
          echo "Installed → ~/.local/bin/mkb.AppImage (ensure ~/.local/bin is on PATH)."
        else
          echo "Built bundles are under $bundle — install the .deb or .AppImage manually."
        fi ;;
      *)
        echo "Built installer is under $bundle — run it to install." ;;
    esac

# Windows variant: run the NSIS installer the `app` recipe produced (best-effort; untested host).
[windows]
install: install-cli app
    #!powershell
    $ErrorActionPreference = 'Stop'
    $bundle = '{{tauri}}/target/release/bundle'
    $setup = Get-ChildItem -Path (Join-Path $bundle 'nsis') -Filter '*-setup.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($setup) {
      Write-Host "Running $($setup.Name) (silent)"
      Start-Process -FilePath $setup.FullName -ArgumentList '/S' -Wait
      Write-Host 'Installed mkb (Start menu).'
    } else {
      Write-Host "Built installer is under $bundle\nsis — run the *-setup.exe to install."
    }

# Semantic search is compiled in, so this works offline. The desktop app is added by `just install`.
# Install the headless tools (daemon + CLI + MCP) onto ~/.cargo/bin.
install-cli:
    cargo install --path crates/mkbd
    cargo install --path crates/mkb-cli
    cargo install --path crates/mkb-mcp

# icons/ is git-ignored build output that `tauri::generate_context!` needs to compile.
# Generate the desktop app's icon set from the tracked source app-icon.png.
[unix]
icons:
    cd {{app_dir}} && cargo tauri icon app-icon.png

[windows]
icons:
    #!powershell
    $ErrorActionPreference = 'Stop'
    Set-Location '{{app_dir}}'
    cargo tauri icon app-icon.png

# Builds the headless release binaries, generates icons, stages the daemon + CLIs as bundled
# resources, then bundles. Output lands under `{{tauri}}/target/release/bundle/`. Requires the
# Tauri toolchain (`cargo install tauri-cli` + the platform's webkit/GTK dev libs).
# Build the desktop app (Tauri) from source for the host platform.
[unix]
app: icons
    # Release binaries the app bundles as resources (auto-start daemon + in-app "install CLI tools").
    cargo build --release -p mkbd -p mkb-cli -p mkb-mcp
    mkdir -p {{tauri}}/bin
    # The CLI is staged as `mkb-cli` so the bundle glob is unambiguous; it installs as `mkb`.
    cp target/release/mkbd   {{tauri}}/bin/mkbd
    cp target/release/mkb-mcp {{tauri}}/bin/mkb-mcp
    cp target/release/mkb     {{tauri}}/bin/mkb-cli
    cd {{tauri}} && cargo tauri build

[windows]
app: icons
    #!powershell
    # Release binaries the app bundles as resources; the bundle globs (bin/mkbd*, …) match the .exe.
    $ErrorActionPreference = 'Stop'
    cargo build --release -p mkbd -p mkb-cli -p mkb-mcp
    New-Item -ItemType Directory -Force -Path '{{tauri}}/bin' | Out-Null
    Copy-Item 'target/release/mkbd.exe'    '{{tauri}}/bin/mkbd.exe'    -Force
    Copy-Item 'target/release/mkb-mcp.exe' '{{tauri}}/bin/mkb-mcp.exe' -Force
    # The CLI is staged as `mkb-cli` so the bundle glob is unambiguous; it installs as `mkb`.
    Copy-Item 'target/release/mkb.exe'     '{{tauri}}/bin/mkb-cli.exe' -Force
    Set-Location '{{tauri}}'
    cargo tauri build

# Run the desktop app in dev mode (hot-reload shell) against your configured vault.
[unix]
app-dev: icons
    cd {{tauri}} && cargo tauri dev

[windows]
app-dev: icons
    #!powershell
    $ErrorActionPreference = 'Stop'
    Set-Location '{{tauri}}'
    cargo tauri dev

# Regenerate the docs that are generated from vault blocks (docs-as-data), then verify no drift.
docs:
    cargo run -p mkb-cli -- export --vault vault
    cargo run -p mkb-cli -- export --vault vault --check

# Remove build artifacts (both workspaces).
clean:
    cargo clean
    cargo clean --manifest-path {{tauri}}/Cargo.toml
