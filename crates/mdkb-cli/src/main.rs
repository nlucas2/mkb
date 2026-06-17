//! `mdkb` — the mdkb CLI.
//!
//! A **thin client**: all behavior lives in `mdkb-core`. This binary only parses arguments
//! and prints results. See `AGENTS.md`.

use std::process::ExitCode;

use mdkb_core::{render_page, Vault};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("render") => cmd_render(&args[1..]),
        Some("assign-ids") => cmd_assign_ids(&args[1..]),
        Some("list") => cmd_list(&args[1..]),
        Some("--version") | Some("-V") => {
            println!("mdkb {}", mdkb_core::VERSION);
            Ok(())
        }
        Some("--help") | Some("-h") | None => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!("unknown command: {other}\n\n{USAGE}")),
    }
}

const USAGE: &str = "\
usage:
  mdkb render <vault-dir> <page>     render a page with transclusions resolved
  mdkb assign-ids <vault-dir>        assign ids to all un-id'd blocks (writes files)
  mdkb list <vault-dir>              list pages in the vault
  mdkb --version                     print version";

fn print_help() {
    println!(
        "mdkb {} — Markdown knowledge base CLI\n\n{USAGE}",
        mdkb_core::VERSION
    );
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let page = args.get(1).ok_or("missing <page>")?;
    let vault = Vault::from_dir(dir).map_err(|e| format!("loading vault: {e}"))?;
    let out = render_page(&vault, page).ok_or_else(|| format!("page not found: {page}"))?;
    println!("{out}");
    Ok(())
}

fn cmd_assign_ids(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let mut vault = Vault::from_dir(dir).map_err(|e| format!("loading vault: {e}"))?;
    let changed = vault.assign_ids();
    if changed.is_empty() {
        println!("all blocks already have ids");
        return Ok(());
    }
    for (path, source) in &changed {
        let full = std::path::Path::new(dir).join(path);
        std::fs::write(&full, source).map_err(|e| format!("writing {path}: {e}"))?;
        println!("updated {path}");
    }
    Ok(())
}

fn cmd_list(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let vault = Vault::from_dir(dir).map_err(|e| format!("loading vault: {e}"))?;
    let mut paths: Vec<&str> = vault.pages().iter().map(|p| p.path.as_str()).collect();
    paths.sort_unstable();
    for p in paths {
        println!("{p}");
    }
    Ok(())
}
