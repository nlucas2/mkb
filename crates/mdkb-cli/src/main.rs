//! `mdkb` — the mdkb CLI (scaffold).
//!
//! A **thin client** for scripting and manual operations. All real behavior lives in
//! `mdkb-core` / the daemon; this binary is glue only. See `AGENTS.md`.

fn main() {
    println!("mdkb (scaffold) — mdkb-core v{}", mdkb_core::VERSION);
}
