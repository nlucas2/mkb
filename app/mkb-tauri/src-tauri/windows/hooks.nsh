; mkb — NSIS installer hooks (wired via bundle.windows.nsis.installerHooks in tauri.conf.json).
;
; The installer IS the whole product: it bundles the CLI tools (mkb.exe, mkb-mcp.exe, mkbd.exe)
; under $INSTDIR\bin, and these hooks put that directory on the user's PATH so the commands work
; from any terminal — the way Docker Desktop's Windows installer adds its resources dir to PATH.
; No separate "install the CLI" step is needed; typing `mkb` just works.
;
; Directory note: Tauri's `bundle.resources: ["bin/"]` lands at $INSTDIR\bin on Windows NSIS —
; there is NO `resources\` segment (that intermediate exists only on macOS, inside the .app). The
; app's Rust side reaches the same files via `resource_dir().join("bin")`, and on Windows
; `resource_dir()` returns $INSTDIR (the exe's own directory), so both agree on $INSTDIR\bin. The
; directory this hook adds to PATH MUST match where the bundle drops the exes, or the PATH entry
; points at nothing and a stale `cargo install` copy wins.
;
; Mechanism: shell out (via nsExec, a CORE NSIS plugin that's always present — unlike EnVar) to
; PowerShell and edit PATH through .NET's [Environment]::(Get|Set)EnvironmentVariable(..., 'User').
; That path handles the registry write, REG_EXPAND_SZ typing, and the environment-change broadcast
; correctly — none of which hand-rolled NSIS string surgery does safely. Tauri's default install
; mode is per-user, so we edit the *user* PATH and need no administrator rights.
;
; The install directory is stable across upgrades, so app updates overwrite the binaries under it
; in place — the CLI on PATH is always the version installed. We **prepend** our directory (and
; de-duplicate any existing occurrence first) so it wins PATH resolution: running the installer is
; the user declaring "make this the active mkb", and prepending is the Windows norm for that
; (rustup's .cargo\bin, Docker Desktop, and VS Code CLI all prepend). Because rustup's .cargo\bin
; lives in the *user* PATH too, prepending to the user PATH reliably beats a stale `cargo install`
; copy. (The one case prepend can't win is a stale mkb on the *system* PATH, which always precedes
; the user PATH — the app's Settings "Check" diagnostic surfaces that.)
;
; NSIS escaping note: `$$` emits a literal `$`, so PowerShell variables ($$d/$$p/$$parts/$$np)
; survive NSIS macro expansion; `$INSTDIR` is a genuine NSIS variable and is expanded by NSIS.
; The whole PowerShell command is wrapped in `...` (backticks) so the embedded double quotes are
; passed through literally. Written to Tauri/NSIS spec; validate the escaping on a real Windows host.

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Adding the mkb command-line tools to the front of your PATH..."
  nsExec::ExecToLog `powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$d = '$INSTDIR\bin'; $$p = [Environment]::GetEnvironmentVariable('Path','User'); if (-not $$p) { $$p = '' }; $$parts = $$p -split ';' | Where-Object { $$_ -and $$_ -ne $$d }; $$np = (@($$d) + $$parts) -join ';'; [Environment]::SetEnvironmentVariable('Path', $$np, 'User')"`
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing the mkb command-line tools from your PATH..."
  nsExec::ExecToLog `powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$d = '$INSTDIR\bin'; $$p = [Environment]::GetEnvironmentVariable('Path','User'); if ($$p) { $$np = ($$p -split ';' | Where-Object { $$_ -and $$_ -ne $$d }) -join ';'; [Environment]::SetEnvironmentVariable('Path', $$np, 'User') }"`
!macroend
