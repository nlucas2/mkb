# mdkb task runner. Install `just` (https://github.com/casey/just), then run e.g. `just`,
# `just install`, or `just install-cli`. These recipes are the canonical build steps — the same
# icon → stage → bundle sequence the release CI runs — so "build it from source" is one command on
# any platform (notably arm64 Linux, which has no prebuilt desktop release).

app_dir := "app/mdkb-tauri"
tauri   := app_dir / "src-tauri"

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

# Builds the app from source and installs it (macOS → /Applications, Linux → .deb or a ~/.local
# AppImage, Windows → run the generated installer). The app bundles the daemon + CLIs, so it's the
# Installs the WHOLE product: the headless tools (daemon, CLI, MCP server, web) onto ~/.cargo/bin,
# AND the desktop app (macOS → /Applications, Linux → .deb or a ~/.local AppImage, Windows → the
# NSIS installer, silent). mdkb is one product — the app, the daemon it drives, and the MCP server
# an AI client uses all share one vault — so the default install gives you all of it. (Daemon-only
# is the container deployment, not this.)
# Install mdkb — the desktop app, the daemon, the CLI, and the MCP server.
install: install-cli app
    #!/usr/bin/env bash
    set -euo pipefail
    bundle="{{tauri}}/target/release/bundle"
    case "$(uname -s)" in
      Darwin)
        echo "Installing mdkb.app → /Applications"
        rm -rf /Applications/mdkb.app
        cp -R "$bundle/macos/mdkb.app" /Applications/mdkb.app
        echo "Done. Launch mdkb from /Applications (or Spotlight)." ;;
      Linux)
        deb=$(ls "$bundle"/deb/*.deb 2>/dev/null | head -1 || true)
        appimage=$(ls "$bundle"/appimage/*.AppImage 2>/dev/null | head -1 || true)
        if [ -n "$deb" ] && command -v dpkg >/dev/null 2>&1; then
          echo "Installing $deb (sudo dpkg -i)"
          sudo dpkg -i "$deb"
        elif [ -n "$appimage" ]; then
          mkdir -p "$HOME/.local/bin"
          cp "$appimage" "$HOME/.local/bin/mdkb.AppImage"
          chmod +x "$HOME/.local/bin/mdkb.AppImage"
          echo "Installed → ~/.local/bin/mdkb.AppImage (ensure ~/.local/bin is on PATH)."
        else
          echo "Built bundles are under $bundle — install the .deb or .AppImage manually."
        fi ;;
      MINGW*|MSYS*|CYGWIN*|Windows_NT)
        setup=$(ls "$bundle"/nsis/*-setup.exe 2>/dev/null | head -1 || true)
        if [ -n "$setup" ]; then
          echo "Running $setup (silent)"
          "$setup" /S
          echo "Installed mdkb (Start menu)."
        else
          echo "Built installer is under $bundle\\nsis — run the *-setup.exe to install."
        fi ;;
      *)
        echo "Built installer is under $bundle — run it to install." ;;
    esac

# Semantic search is compiled in, so this works offline. The desktop app is added by `just install`.
# Install the headless tools (daemon + CLI + MCP) onto ~/.cargo/bin.
install-cli:
    cargo install --path crates/mdkbd
    cargo install --path crates/mdkb-cli
    cargo install --path crates/mdkb-mcp

# icons/ is git-ignored build output that `tauri::generate_context!` needs to compile.
# Generate the desktop app's icon set from the tracked source app-icon.png.
icons:
    cd {{app_dir}} && cargo tauri icon app-icon.png

# Builds the headless release binaries, generates icons, stages the daemon + CLIs as bundled
# resources, then bundles. Output lands under `{{tauri}}/target/release/bundle/`. Requires the
# Tauri toolchain (`cargo install tauri-cli` + the platform's webkit/GTK dev libs).
# Build the desktop app (Tauri) from source for the host platform.
app: icons
    # Release binaries the app bundles as resources (auto-start daemon + in-app "install CLI tools").
    cargo build --release -p mdkbd -p mdkb-cli -p mdkb-mcp
    mkdir -p {{tauri}}/bin
    # The CLI is staged as `mdkb-cli` so the bundle glob is unambiguous; it installs as `mdkb`.
    cp target/release/mdkbd   {{tauri}}/bin/mdkbd
    cp target/release/mdkb-mcp {{tauri}}/bin/mdkb-mcp
    cp target/release/mdkb     {{tauri}}/bin/mdkb-cli
    cd {{tauri}} && cargo tauri build

# Run the desktop app in dev mode (hot-reload shell) against your configured vault.
app-dev: icons
    cd {{tauri}} && cargo tauri dev

# Regenerate the docs that are generated from vault blocks (docs-as-data), then verify no drift.
docs:
    cargo run -p mdkb-cli -- export --vault vault
    cargo run -p mdkb-cli -- export --vault vault --check

# Remove build artifacts (both workspaces).
clean:
    cargo clean
    cd {{tauri}} && cargo clean
