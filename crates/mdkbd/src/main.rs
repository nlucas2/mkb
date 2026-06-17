//! `mdkbd` — the mdkb headless daemon (scaffold).
//!
//! Will own the file watcher, the local index, and all writes. For now it is a
//! placeholder that confirms the workspace wires up against `mdkb-core`.

fn main() {
    println!("mdkbd (scaffold) — mdkb-core v{}", mdkb_core::VERSION);
}
