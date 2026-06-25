//! Binary entry point for the mkb desktop shell.

// Opt into the Windows "windows" subsystem in release builds so launching the app doesn't
// pop a console window alongside it. Debug builds keep the console for `println!`/panic
// output. (The app's own diagnostics use a panic-safe writer, so an absent stderr here is
// harmless — see `log_line!` in lib.rs.)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mkb_tauri_lib::run();
}
