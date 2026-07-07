# mkb task runner. Install `just` (https://github.com/casey/just), then run e.g. `just`,
# `just install`, or `just build`. These recipes are the canonical build steps — the same
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

# Build and install the WHOLE product from source: the desktop app plus the bundled daemon, CLI,
# and MCP server (macOS → /Applications, Linux → .deb or a ~/.local AppImage, Windows → the NSIS
# installer). The command-line tools (`mkb`, `mkb-mcp`) ship *inside* the app and are exposed on
# PATH by the app itself — on Windows the installer's PATH hook does it; on macOS/Linux launch the
# app once and use the "Install command-line tools" prompt (or Settings → Command-line tools).
# There is no separate CLI-only install: the app is the one product, sharing one vault with its daemon.
# Install mkb from source — the desktop app plus its bundled daemon, CLI, and MCP server.
[unix]
install: app
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
        echo "Done. Launch mkb from /Applications; the app can add mkb + mkb-mcp to your PATH." ;;
      Linux)
        deb=$(ls "$bundle"/deb/*.deb 2>/dev/null | head -1 || true)
        appimage=$(ls "$bundle"/appimage/*.AppImage 2>/dev/null | head -1 || true)
        if [ -n "$deb" ] && command -v dpkg >/dev/null 2>&1; then
          echo "Installing $deb (sudo dpkg -i)"
          sudo dpkg -i "$deb"
          echo "Done. Launch mkb; the app can add mkb + mkb-mcp to your PATH."
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

# Windows: build the app (which bundles the CLI + daemon and, via the NSIS PATH hook, puts them on
# PATH), then run the exact installer we just produced — pinned to the current version from
# tauri.conf.json (arch via glob; newest first if several), never a blind "first match". No separate
# CLI step: the installer is the whole product, exactly like the downloaded release.
[windows]
install: app
    #!powershell
    $ErrorActionPreference = 'Stop'
    $nsis = '{{tauri}}/target/release/bundle/nsis'
    $ver = (Get-Content '{{tauri}}/tauri.conf.json' -Raw | ConvertFrom-Json).version
    $setup = Get-ChildItem -Path $nsis -Filter "mkb_${ver}_*-setup.exe" -ErrorAction SilentlyContinue |
             Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($setup) {
      Write-Host "Running $($setup.Name) (silent)"
      Start-Process -FilePath $setup.FullName -ArgumentList '/S' -Wait
      Write-Host 'Installed mkb (Start menu; mkb + mkb-mcp on your PATH — open a new terminal).'
    } else {
      Write-Host "No mkb_${ver}_*-setup.exe under $nsis — build may have failed."
      exit 1
    }

# Undo the OLD `just install-cli` (removed): delete the daemon/CLI/MCP copies it installed onto
# ~/.cargo/bin, via `cargo uninstall` — which removes ONLY what cargo recorded installing (tracked
# in ~/.cargo/.crates.toml) and never touches anything else in ~/.cargo/bin. The app now owns
# CLI-on-PATH, so these cargo-bin copies are just a stale shadow that can win over the app's; this
# clears them. Harmless no-op for each tool you never installed that way.
#
# Guard: an MCP client config (or its launcher) that HARDCODES the ~/.cargo/bin copy we're about to
# remove would break silently once it's gone. A bare `mkb-mcp` (PATH-resolved) or a path into the
# app bundle is fine — only an absolute ~/.cargo/bin/mkb* reference is the hazard. We scan the
# common config locations (best-effort; clients vary) and ABORT on a hit so nothing breaks — re-run
# with FORCE=1 to remove anyway.
# Remove the stale ~/.cargo/bin daemon/CLI/MCP copies left by the old `install-cli` (cargo uninstall).
[unix]
clean-cargo-installs:
    #!/usr/bin/env bash
    set -uo pipefail
    shopt -s nullglob
    scan=(
      ./.mcp.json ./.vscode/mcp.json
      "$HOME/.copilot/mcp-config.json"
      "$HOME/.copilot/bin/"*-launcher
      "$HOME/.claude.json"
      "$HOME/Library/Application Support/Claude/claude_desktop_config.json"
      "$HOME/.config/Claude/claude_desktop_config.json"
      "$HOME/.cursor/mcp.json"
    )
    hits=()
    for f in "${scan[@]}"; do
      [ -f "$f" ] || continue
      grep -qE '\.cargo/bin/mkb' "$f" 2>/dev/null && hits+=("$f")
    done
    if [ "${#hits[@]}" -gt 0 ]; then
      echo "WARNING: these MCP configs hardcode a ~/.cargo/bin/mkb* path we're about to remove:" >&2
      for f in "${hits[@]}"; do
        echo "    $f" >&2
        grep -nE '\.cargo/bin/mkb' "$f" | sed 's/^/      /' >&2
      done
      echo "  Point them at a bare 'mkb-mcp' (now on PATH via the app) or the app's bundled copy, then re-run." >&2
      if [ "${FORCE:-}" != "1" ]; then
        echo "  Aborted — no binaries removed. Re-run with FORCE=1 to remove them anyway." >&2
        exit 1
      fi
      echo "  FORCE=1 set — continuing despite the above." >&2
    fi
    for pkg in mkb-cli mkb-mcp mkbd; do
      if cargo uninstall "$pkg" >/dev/null 2>&1; then
        echo "removed $pkg from ~/.cargo/bin"
      else
        echo "$pkg not cargo-installed — skipped"
      fi
    done

# Windows locks a running .exe, so stop any cargo-bin copies (auto-started daemon/MCP) before
# `cargo uninstall` can delete them; a client respawns from the app's copy on next use. Same
# hardcoded-path guard as the unix recipe (abort on a hit unless FORCE=1).
[windows]
clean-cargo-installs:
    #!powershell
    $ErrorActionPreference = 'Continue'
    $scan = @(
      './.mcp.json', './.vscode/mcp.json',
      (Join-Path $env:USERPROFILE '.copilot\mcp-config.json'),
      (Join-Path $env:USERPROFILE '.claude.json'),
      (Join-Path $env:APPDATA 'Claude\claude_desktop_config.json'),
      (Join-Path $env:USERPROFILE '.cursor\mcp.json')
    )
    $launchers = @(Get-ChildItem (Join-Path $env:USERPROFILE '.copilot\bin') -Filter '*-launcher*' -ErrorAction SilentlyContinue |
        ForEach-Object { $_.FullName })
    $scan += $launchers
    $hits = @()
    foreach ($f in $scan) {
      if ((Test-Path $f) -and (Select-String -Path $f -Pattern '\.cargo[\\/]+bin[\\/]+mkb' -Quiet)) { $hits += $f }
    }
    if ($hits.Count -gt 0) {
      Write-Host "WARNING: these MCP configs hardcode a .cargo\bin\mkb* path we're about to remove:"
      foreach ($f in $hits) {
        Write-Host "    $f"
        Select-String -Path $f -Pattern '\.cargo[\\/]+bin[\\/]+mkb' |
            ForEach-Object { Write-Host "      $($_.LineNumber): $($_.Line.Trim())" }
      }
      Write-Host "  Point them at a bare 'mkb-mcp' (on PATH via the app) or the app's bundled copy, then re-run."
      if ($env:FORCE -ne '1') {
        Write-Host "  Aborted - no binaries removed. Re-run with FORCE=1 to remove them anyway."
        exit 1
      }
      Write-Host "  FORCE=1 set - continuing despite the above."
    }
    $bin = Join-Path $env:USERPROFILE '.cargo\bin'
    Get-Process mkbd, mkb-mcp, mkb -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and $_.Path.StartsWith($bin, [System.StringComparison]::OrdinalIgnoreCase) } |
        Stop-Process -Force
    Start-Sleep -Milliseconds 500
    foreach ($pkg in 'mkb-cli', 'mkb-mcp', 'mkbd') {
      cargo uninstall $pkg 2>$null
      if ($LASTEXITCODE -eq 0) { Write-Host "removed $pkg from ~/.cargo/bin" }
      else { Write-Host "$pkg not cargo-installed - skipped" }
    }

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
    # Release binaries the app bundles as resources (auto-start daemon + CLI-on-PATH install).
    cargo build --release -p mkbd -p mkb-cli -p mkb-mcp -p mkb-web
    rm -rf {{tauri}}/bin && mkdir -p {{tauri}}/bin
    # Stage under real command names — the whole bin/ dir is bundled and exposed on PATH as-is.
    cp target/release/mkbd    {{tauri}}/bin/mkbd
    cp target/release/mkb-mcp {{tauri}}/bin/mkb-mcp
    cp target/release/mkb     {{tauri}}/bin/mkb
    cp target/release/mkb-web {{tauri}}/bin/mkb-web
    cd {{tauri}} && cargo tauri build

[windows]
app: icons
    #!powershell
    # Release binaries the app bundles as resources; the NSIS hook adds the whole bin/ dir to PATH.
    $ErrorActionPreference = 'Stop'
    cargo build --release -p mkbd -p mkb-cli -p mkb-mcp -p mkb-web
    Remove-Item -Recurse -Force '{{tauri}}/bin' -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path '{{tauri}}/bin' | Out-Null
    Copy-Item 'target/release/mkbd.exe'    '{{tauri}}/bin/mkbd.exe'    -Force
    Copy-Item 'target/release/mkb-mcp.exe' '{{tauri}}/bin/mkb-mcp.exe' -Force
    # Stage under the real command name so PATH exposes `mkb`, not `mkb-cli`.
    Copy-Item 'target/release/mkb.exe'     '{{tauri}}/bin/mkb.exe'     -Force
    Copy-Item 'target/release/mkb-web.exe' '{{tauri}}/bin/mkb-web.exe' -Force
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

# Launch the mkb MCP server for THIS repo's vault, for an AI client pointed at it (see the repo's
# `.mcp.json`). Prefers an installed `mkb-mcp` (instant, no rebuild); falls back to `cargo run` when
# it isn't on PATH (works from a fresh source checkout with no install). `just` runs this from the
# justfile's directory, so `--vault vault` resolves to this repo's vault from any subdir/worktree.
[unix]
[doc('Run the mkb MCP server for THIS repo vault (installed mkb-mcp, else cargo run)')]
mcp:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v mkb-mcp >/dev/null 2>&1; then
        exec mkb-mcp --vault vault
    else
        exec cargo run -q -p mkb-mcp -- --vault vault
    fi

[windows]
[doc('Run the mkb MCP server for THIS repo vault (installed mkb-mcp, else cargo run)')]
mcp:
    #!powershell
    $ErrorActionPreference = 'Stop'
    if (Get-Command mkb-mcp -ErrorAction SilentlyContinue) {
        mkb-mcp --vault vault
    } else {
        cargo run -q -p mkb-mcp -- --vault vault
    }

# One-time per clone: point git at the tracked hooks in `.githooks/`, installing the docs-as-data
# pre-commit hook (regenerates & re-stages generated docs when you change vault blocks, so they
# never drift in a commit). `git config` is cross-platform, so one recipe covers every OS.
[doc('Install the docs-as-data pre-commit hook (once per clone; sets core.hooksPath)')]
hooks:
    git config core.hooksPath .githooks
    @echo "Installed .githooks (pre-commit keeps docs-as-data in sync). Undo: git config --unset core.hooksPath"

# Remove build artifacts (both workspaces).
clean:
    cargo clean
    cargo clean --manifest-path {{tauri}}/Cargo.toml
