; mkb — NSIS installer hooks (wired via bundle.windows.nsis.installerHooks in tauri.conf.json).
;
; The installer IS the whole product: it bundles the CLI tools (mkb.exe, mkb-mcp.exe, mkbd.exe)
; under $INSTDIR\resources\bin, and these hooks put that directory on the user's PATH so the
; commands work from any terminal — the way Docker Desktop's Windows installer adds its
; resources\bin to PATH. No separate "install the CLI" step is needed; typing `mkb` just works.
;
; Mechanism: shell out (via nsExec, a CORE NSIS plugin that's always present — unlike EnVar) to
; PowerShell and edit PATH through .NET's [Environment]::(Get|Set)EnvironmentVariable(..., 'User').
; That path handles the registry write, REG_EXPAND_SZ typing, and the environment-change broadcast
; correctly — none of which hand-rolled NSIS string surgery does safely. Tauri's default install
; mode is per-user, so we edit the *user* PATH and need no administrator rights.
;
; The install directory is stable across upgrades, so the PATH entry is added once; app updates
; overwrite the binaries under it in place, so the CLI on PATH is always the version installed.
;
; NSIS escaping note: `$$` emits a literal `$`, so PowerShell variables ($$d/$$p/$$parts/$$np)
; survive NSIS macro expansion; `$INSTDIR` is a genuine NSIS variable and is expanded by NSIS.
; The whole PowerShell command is wrapped in `...` (backticks) so the embedded double quotes are
; passed through literally. Written to Tauri/NSIS spec; validate the escaping on a real Windows host.

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Adding the mkb command-line tools to your PATH..."
  nsExec::ExecToLog `powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$d = '$INSTDIR\resources\bin'; $$p = [Environment]::GetEnvironmentVariable('Path','User'); if (-not $$p) { $$p = '' }; if (($$p -split ';') -notcontains $$d) { $$np = if ($$p) { $$p.TrimEnd(';') + ';' + $$d } else { $$d }; [Environment]::SetEnvironmentVariable('Path', $$np, 'User') }"`
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing the mkb command-line tools from your PATH..."
  nsExec::ExecToLog `powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$d = '$INSTDIR\resources\bin'; $$p = [Environment]::GetEnvironmentVariable('Path','User'); if ($$p) { $$np = ($$p -split ';' | Where-Object { $$_ -and $$_ -ne $$d }) -join ';'; [Environment]::SetEnvironmentVariable('Path', $$np, 'User') }"`
!macroend
