//! `mdkb` — the mdkb CLI.
//!
//! A **thin client**: all behavior lives in `mdkb-core` / `mdkb-index`. This binary only
//! parses arguments and prints results. The unit is the **block** (one file). See `AGENTS.md`.

use std::process::ExitCode;

use mdkb_core::{export, render_block, BlockId, Index, SearchQuery, SyncEngine};
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
        Some("list") => cmd_list(&args[1..]),
        Some("search") => cmd_search(&args[1..]),
        Some("tags") => cmd_tags(&args[1..]),
        Some("export") => cmd_export(&args[1..]),
        Some("stats") => cmd_stats(&args[1..]),
        Some("daemon") => cmd_daemon(&args[1..]),
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
  mdkb render <vault-dir> <block-id>        render a block with its children resolved
  mdkb list <vault-dir>                     list root blocks (id  title)
  mdkb search <vault-dir> <query> [flags]   search across the vault (keyword + semantic)
       flags: --lang=<lang> --tag=<tag> (repeatable) --limit=<n>
       query also accepts inline operators: tag:<t>  #<t>  lang:<l>  code:<l>
  mdkb tags <vault-dir>                     list all tags with block counts
  mdkb export <vault-dir> [flags]           generate docs from blocks (docs-as-data)
       flags: --manifest=<path> (default <vault>/export.manifest)
              --root=<dir>      output root for relative paths (default: cwd)
              --check           verify files are up to date; non-zero exit on drift
  mdkb stats <vault-dir>                    index statistics
  mdkb daemon <socket> <subcmd> [args]      talk to a running mdkbd over its socket
       subcmds: ping | stats | list | search <query> | render <id> | rebuild | conflicts
  mdkb --version                            print version";

fn print_help() {
    println!(
        "mdkb {} — Markdown knowledge base CLI\n\n{USAGE}",
        mdkb_core::VERSION
    );
}

/// Build a read-only, in-memory engine over the vault: it reconciles and embeds blocks, so
/// both keyword and semantic search work. Reuses the exact ingest/embed/search path the
/// daemon uses — no duplicated logic.
fn readonly_engine(dir: &str) -> Result<SyncEngine<SqliteIndex>, String> {
    let index = SqliteIndex::open_in_memory().map_err(|e| e.to_string())?;
    let mdkb_dir = mdkb_protocol::DaemonPaths::from_vault(dir)
        .mdkb_dir()
        .to_path_buf();
    let source = mdkb_embed::FileConfig::load(&mdkb_dir).embedder;
    let mut engine = SyncEngine::new(dir, index).with_embedder(mdkb_embed::from_source(&source));
    engine.reconcile().map_err(|e| e.to_string())?;
    Ok(engine)
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let id = args.get(1).ok_or("missing <block-id>")?;
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    let engine = readonly_engine(dir)?;
    let out = render_block(engine.vault(), &bid).ok_or_else(|| format!("block not found: {id}"))?;
    println!("{out}");
    Ok(())
}

fn cmd_list(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let engine = readonly_engine(dir)?;
    for id in engine.vault().roots() {
        let title = engine
            .vault()
            .block(&id)
            .map(|b| b.display_title())
            .unwrap_or_default();
        println!("{id}  {title}");
    }
    Ok(())
}

fn cmd_search(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let query_text = args.get(1).ok_or("missing <query>")?;
    // The positional query understands the same inline operators as the app/MCP
    // (tag:, #tag, lang:/code:) via the shared parser; the --tag/--lang flags add to it.
    let mut q = SearchQuery::parse(query_text);
    for flag in &args[2..] {
        if let Some(l) = flag.strip_prefix("--lang=") {
            q.lang = Some(l.to_lowercase());
        } else if let Some(t) = flag.strip_prefix("--tag=") {
            let t = t.to_lowercase();
            if !q.tags.contains(&t) {
                q.tags.push(t);
            }
        } else if let Some(n) = flag.strip_prefix("--limit=") {
            q.limit = n.parse().map_err(|_| format!("bad --limit: {n}"))?;
        } else {
            return Err(format!("unknown flag: {flag}"));
        }
    }
    let engine = readonly_engine(dir)?;
    let hits = engine.search(q).map_err(|e| e.to_string())?;
    if hits.is_empty() {
        println!("(no matches)");
    }
    for h in hits {
        let b = &h.block;
        let preview: String = b.content.replace('\n', " ").chars().take(100).collect();
        println!("{}  {}\n    {}", b.id, b.display_title(), preview);
    }
    Ok(())
}

fn cmd_stats(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let engine = readonly_engine(dir)?;
    let s = engine.index().stats().map_err(|e| e.to_string())?;
    println!("blocks:   {}", s.blocks);
    println!("roots:    {}", engine.vault().roots().len());
    println!("embedded: {}", s.embedded);
    Ok(())
}

fn cmd_tags(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let engine = readonly_engine(dir)?;
    let tags = engine.index().tag_counts().map_err(|e| e.to_string())?;
    if tags.is_empty() {
        println!("(no tags)");
    }
    for t in tags {
        println!("{:>4}  #{}", t.count, t.tag);
    }
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <vault-dir>")?;
    let mut manifest_path = format!("{dir}/export.manifest");
    let mut root = ".".to_string();
    let mut check = false;
    for flag in &args[1..] {
        if let Some(p) = flag.strip_prefix("--manifest=") {
            manifest_path = p.to_string();
        } else if let Some(r) = flag.strip_prefix("--root=") {
            root = r.to_string();
        } else if flag == "--check" {
            check = true;
        } else {
            return Err(format!("unknown flag: {flag}"));
        }
    }

    let manifest_text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("reading manifest {manifest_path}: {e}"))?;
    let manifest = export::Manifest::parse(&manifest_text)?;
    let engine = readonly_engine(dir)?;
    let docs = export::plan_exports(engine.vault(), &manifest)?;

    let root = std::path::Path::new(&root);
    let mut drifted = Vec::new();
    let mut wrote = 0usize;
    for doc in &docs {
        let out = root.join(&doc.path);
        let current = std::fs::read_to_string(&out).ok();
        let up_to_date = current.as_deref() == Some(doc.content.as_str());
        if check {
            if !up_to_date {
                drifted.push(doc.path.clone());
            }
            continue;
        }
        if up_to_date {
            println!("unchanged  {}", doc.path);
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        std::fs::write(&out, &doc.content)
            .map_err(|e| format!("writing {}: {e}", out.display()))?;
        println!(
            "{}  {}",
            if current.is_some() {
                "updated"
            } else {
                "created"
            },
            doc.path
        );
        wrote += 1;
    }

    if check {
        if drifted.is_empty() {
            println!("up to date ({} doc(s))", docs.len());
            Ok(())
        } else {
            for p in &drifted {
                eprintln!("drift: {p}");
            }
            Err(format!(
                "{} doc(s) out of date; run `mdkb export {dir}` to regenerate",
                drifted.len()
            ))
        }
    } else {
        println!("exported {} doc(s) ({wrote} written)", docs.len());
        Ok(())
    }
}

fn cmd_daemon(args: &[String]) -> Result<(), String> {
    let socket = args.first().ok_or("missing <socket>")?;
    let sub = args.get(1).map(String::as_str).ok_or("missing <subcmd>")?;
    let client = mdkb_protocol::Client::new(socket.as_str());
    match sub {
        "ping" => {
            if client.ping() {
                println!("ok");
                Ok(())
            } else {
                Err("daemon did not respond".to_string())
            }
        }
        "stats" => {
            let s = client.stats().map_err(|e| e.to_string())?;
            println!(
                "blocks: {}  roots: {}  embedded: {}",
                s.blocks, s.roots, s.embedded
            );
            Ok(())
        }
        "list" => {
            for id in client.list_roots().map_err(|e| e.to_string())? {
                let title = client
                    .get_block(id.clone())
                    .map_err(|e| e.to_string())?
                    .map(|b| b.display_title())
                    .unwrap_or_default();
                println!("{id}  {title}");
            }
            Ok(())
        }
        "search" => {
            let query = args.get(2).ok_or("missing <query>")?;
            let hits = client
                .search(SearchQuery::text(query))
                .map_err(|e| e.to_string())?;
            for h in hits {
                let preview: String = h
                    .block
                    .content
                    .replace('\n', " ")
                    .chars()
                    .take(100)
                    .collect();
                println!(
                    "{}  {}\n    {}",
                    h.block.id,
                    h.block.display_title(),
                    preview
                );
            }
            Ok(())
        }
        "render" => {
            let id = args.get(2).ok_or("missing <block-id>")?;
            let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
            match client.render_block(bid).map_err(|e| e.to_string())? {
                Some(md) => {
                    println!("{md}");
                    Ok(())
                }
                None => Err(format!("block not found: {id}")),
            }
        }
        "rebuild" => {
            client.rebuild().map_err(|e| e.to_string())?;
            println!("rebuilt");
            Ok(())
        }
        "conflicts" => {
            let files = client.conflicts().map_err(|e| e.to_string())?;
            if files.is_empty() {
                println!("(no conflicts)");
            } else {
                for f in files {
                    println!("{f}");
                }
            }
            Ok(())
        }
        other => Err(format!("unknown daemon subcmd: {other}")),
    }
}
