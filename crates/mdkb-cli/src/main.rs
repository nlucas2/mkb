//! `mdkb` — the mdkb CLI.
//!
//! A **thin client**: all behavior lives in `mdkb-core` / `mdkb-index`. This binary only
//! parses arguments and prints results. See `AGENTS.md`.

use std::process::ExitCode;

use mdkb_core::{render_page, Index, SearchQuery, Vault};
use mdkb_index::SqliteIndex;

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
        Some("search") => cmd_search(&args[1..]),
        Some("stats") => cmd_stats(&args[1..]),
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
  mdkb render <vault-dir> <page>            render a page with transclusions resolved
  mdkb assign-ids <vault-dir>               assign ids to all un-id'd blocks (writes files)
  mdkb list <vault-dir>                     list pages in the vault
  mdkb search <vault-dir> <query> [flags]   keyword search across the vault
       flags: --lang=<lang> --tag=<tag> (repeatable) --page=<path> --limit=<n>
  mdkb stats <vault-dir>                    index statistics
  mdkb --version                            print version";

fn print_help() {
    println!(
        "mdkb {} — Markdown knowledge base CLI\n\n{USAGE}",
        mdkb_core::VERSION
    );
}

fn load_vault(dir: &str) -> Result<Vault, String> {
    Vault::from_dir(dir).map_err(|e| format!("loading vault: {e}"))
}

fn build_index(vault: &Vault) -> Result<SqliteIndex, String> {
    let mut idx = SqliteIndex::open_in_memory().map_err(|e| e.to_string())?;
    idx.rebuild(vault).map_err(|e| e.to_string())?;
    Ok(idx)
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let page = args.get(1).ok_or("missing <page>")?;
    let vault = load_vault(dir)?;
    let out = render_page(&vault, page).ok_or_else(|| format!("page not found: {page}"))?;
    println!("{out}");
    Ok(())
}

fn cmd_assign_ids(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let mut vault = load_vault(dir)?;
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
    let vault = load_vault(dir)?;
    let mut paths: Vec<&str> = vault.pages().iter().map(|p| p.path.as_str()).collect();
    paths.sort_unstable();
    for p in paths {
        println!("{p}");
    }
    Ok(())
}

fn cmd_search(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let mut query = SearchQuery::default();
    let mut text_parts = Vec::new();
    for arg in &args[1..] {
        if let Some(v) = arg.strip_prefix("--lang=") {
            query.lang = Some(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--tag=") {
            query.tags.push(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--page=") {
            query.page = Some(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--limit=") {
            query.limit = v.parse().map_err(|_| "invalid --limit")?;
        } else {
            text_parts.push(arg.clone());
        }
    }
    if !text_parts.is_empty() {
        query.text = Some(text_parts.join(" "));
    }

    let vault = load_vault(dir)?;
    let idx = build_index(&vault)?;
    let hits = idx.search(&query).map_err(|e| e.to_string())?;
    if hits.is_empty() {
        println!("(no matches)");
        return Ok(());
    }
    for hit in hits {
        let b = &hit.block;
        let lineage = if b.lineage.is_empty() {
            String::new()
        } else {
            format!("  [{}]", b.lineage.join(" > "))
        };
        let preview = b.content.replace('\n', " ");
        let preview = preview.chars().take(80).collect::<String>();
        println!("{}  {}{}\n    {}", b.page_path, b.id, lineage, preview);
    }
    Ok(())
}

fn cmd_stats(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let vault = load_vault(dir)?;
    let idx = build_index(&vault)?;
    let s = idx.stats().map_err(|e| e.to_string())?;
    println!("pages:    {}", s.pages);
    println!("blocks:   {}", s.blocks);
    println!("embedded: {}", s.embedded);
    Ok(())
}
