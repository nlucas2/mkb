//! `mdkb` — the mdkb CLI.
//!
//! A **thin client**: every command connects to the vault's daemon (auto-starting a detached
//! `mdkbd` if none is running) and dispatches over the socket — exactly like the MCP server and
//! the desktop app. There is no separate in-process engine: the daemon owns the one persistent,
//! warm index and is the single writer, so the CLI never re-parses or re-embeds the vault and
//! never races the daemon. The unit is the **block** (one file). See `AGENTS.md`.

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use mdkb_core::export::{ExportRequest, SlugSelection};
use mdkb_core::{BlockId, SearchQuery};
use mdkb_protocol::{ensure_daemon, Client, DaemonPaths};

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
        // reads
        Some("ping") => cmd_ping(&args[1..]),
        Some("list") => cmd_list(&args[1..]),
        Some("render") => cmd_render(&args[1..]),
        Some("get") => cmd_get(&args[1..]),
        Some("search") => cmd_search(&args[1..]),
        Some("tags") => cmd_tags(&args[1..]),
        Some("backlinks") => cmd_links(&args[1..], true),
        Some("links") => cmd_links(&args[1..], false),
        Some("stats") => cmd_stats(&args[1..]),
        Some("conflicts") => cmd_conflicts(&args[1..]),
        // writes
        Some("create") => cmd_create(&args[1..]),
        Some("update") => cmd_update(&args[1..]),
        Some("set-tags") => cmd_set_tags(&args[1..]),
        Some("link") => cmd_link(&args[1..]),
        Some("carve") => cmd_carve(&args[1..]),
        Some("delete") => cmd_delete(&args[1..]),
        // maintenance
        Some("rebuild") => cmd_rebuild(&args[1..]),
        Some("export") => cmd_export(&args[1..]),
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
usage: mdkb <command> <vault-dir> [args]   (connects to the vault's daemon, auto-starting it)

connection: defaults to a local Unix socket under <vault>/.mdkb/, auto-starting mdkbd.
  Set MDKB_REMOTE=host:port (+ MDKB_TOKEN) to use a TCP daemon (e.g. a loopback high port
  where Unix sockets aren't usable), or MDKB_SOCKET=<path> for an explicit socket.

reads:
  list <vault>                      root blocks (id  title)
  render <vault> <id> [--flat]      render a block, children resolved (--flat = published form:
                                    embeds dissolved inline, refs as plain titles, to stdout)
  get <vault> <id>                  raw Markdown body of a block
  search <vault> <query> [flags]    search (keyword + semantic)
       flags: --lang=<l> --tag=<t> (repeatable) --limit=<n>
       query also accepts operators: tag:<t>  #<t>  lang:<l>  code:<l>
  tags <vault>                      all tags with block counts
  backlinks <vault> <id>            blocks that reference/embed <id>
  links <vault> <id>                outgoing links/embeds from <id>
  stats <vault>                     index statistics
  conflicts <vault>                 cloud-sync conflict files
  ping <vault>                      check the daemon is reachable

writes (body is read from stdin where noted):
  create <vault> [--title=T] < body          create a block; prints the new id
  update <vault> <id> [--title=T] < body     overwrite a block's title + body
  set-tags <vault> <id> [tag ...]            set managed (frontmatter) tags ([] clears)
  link <vault> <src> <dst> [--embed]         reference (or --embed: transclude) dst from src
  carve <vault> <parent> [--title=T] < body  carve a new child block; prints the child id
  delete <vault> <id>                        delete a block

maintenance:
  rebuild <vault>                   rebuild the index from blocks/
  export <vault> [flags]            generate flat docs from blocks (docs-as-data)
       With no selector: dumps every root block to <slug>.md under --root (default docs-export/).
       With a manifest (<vault>/export.manifest or --manifest=<path>): writes each mapped doc.
       With --tag=NAME: dumps roots carrying that tag to <slug>.md (add --include-non-root for
       every tagged block, transcluded ones included).
       Co-exported docs cross-link; a [[link]] to a block outside the export warns (and stays
       plain text) unless --follow-links pulls the linked block into the export.
       flags: --manifest=<path>  --tag=<name>  --include-non-root  --follow-links  --root=<dir>
              --raw (omit the @generated banner)  --check (verify only; non-zero exit on drift)

  --version                         print version";

fn print_help() {
    println!(
        "mdkb {} — Markdown knowledge base CLI\n\n{USAGE}",
        mdkb_core::VERSION
    );
}

/// Connect to the daemon for a vault.
///
/// By default this auto-starts (if needed) a local daemon on a Unix socket under
/// `<vault>/.mdkb/`. For environments where a Unix socket isn't usable (read-only FS, a
/// too-long socket path, an odd network mount) — or to share one daemon — set `MDKB_REMOTE`
/// (`host:port` + `MDKB_TOKEN`) to talk to a daemon over **loopback or remote TCP**, or
/// `MDKB_SOCKET` to point at an explicit socket path. Run such a daemon with
/// `mdkbd --vault <dir> --listen 127.0.0.1:<port> --token <tok>`.
fn client(dir: &str) -> Result<Client, String> {
    if std::env::var_os("MDKB_REMOTE").is_some() || std::env::var_os("MDKB_SOCKET").is_some() {
        return Client::from_env();
    }
    let paths = DaemonPaths::from_vault(dir);
    ensure_daemon(&paths, None)
}

fn req<'a>(args: &'a [String], i: usize, what: &str) -> Result<&'a str, String> {
    args.get(i)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {what}"))
}

fn parse_id(s: &str) -> Result<BlockId, String> {
    BlockId::parse(s).map_err(|_| format!("invalid block id: {s}"))
}

fn read_stdin() -> Result<String, String> {
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .map_err(|e| format!("reading stdin: {e}"))?;
    Ok(s)
}

/// Pull an optional `--title=...` out of the flags, returning it and the remaining flags.
fn take_title(flags: &[String]) -> (Option<String>, Vec<String>) {
    let mut title = None;
    let mut rest = Vec::new();
    for f in flags {
        if let Some(t) = f.strip_prefix("--title=") {
            title = Some(t.to_string());
        } else {
            rest.push(f.clone());
        }
    }
    (title, rest)
}

// ---------- reads ----------

fn cmd_ping(args: &[String]) -> Result<(), String> {
    if client(req(args, 0, "<vault-dir>")?)?.ping() {
        println!("ok");
        Ok(())
    } else {
        Err("daemon did not respond".to_string())
    }
}

fn cmd_list(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    for id in c.list_roots().map_err(|e| e.to_string())? {
        let title = c
            .get_block(id.clone())
            .map_err(|e| e.to_string())?
            .map(|b| b.display_title())
            .unwrap_or_default();
        println!("{id}  {title}");
    }
    Ok(())
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let id = parse_id(req(args, 1, "<block-id>")?)?;
    let mut flat = false;
    for f in &args[2..] {
        if f == "--flat" {
            flat = true;
        } else {
            return Err(format!("unknown flag: {f}"));
        }
    }
    // --flat = the published form (embeds dissolved inline, refs as plain titles); the default
    // is the interactive form (embed cards + mdkb: links).
    let out = if flat {
        c.render_flat(id).map_err(|e| e.to_string())?
    } else {
        c.render_block(id).map_err(|e| e.to_string())?
    };
    match out {
        Some(md) => {
            println!("{md}");
            Ok(())
        }
        None => Err("block not found".to_string()),
    }
}

fn cmd_get(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let id = parse_id(req(args, 1, "<block-id>")?)?;
    match c.get_block_source(id).map_err(|e| e.to_string())? {
        Some(src) => {
            print!("{src}");
            Ok(())
        }
        None => Err("block not found".to_string()),
    }
}

fn cmd_search(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let query_text = req(args, 1, "<query>")?;
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
    let hits = c.search(q).map_err(|e| e.to_string())?;
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

fn cmd_tags(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let tags = c.list_tags().map_err(|e| e.to_string())?;
    if tags.is_empty() {
        println!("(no tags)");
    }
    for t in tags {
        println!("{:>4}  #{}", t.count, t.tag);
    }
    Ok(())
}

fn cmd_links(args: &[String], incoming: bool) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let id = parse_id(req(args, 1, "<block-id>")?)?;
    let rows = if incoming {
        c.backlinks(id).map_err(|e| e.to_string())?
    } else {
        c.links_from(id).map_err(|e| e.to_string())?
    };
    if rows.is_empty() {
        println!("(none)");
    }
    for r in rows {
        let kind = match r.kind {
            mdkb_core::LinkKind::Transcludes => "embed",
            mdkb_core::LinkKind::References => "ref",
        };
        let other = if incoming {
            r.source_id.to_string()
        } else {
            r.target.clone()
        };
        println!("{kind:>5}  {other}");
    }
    Ok(())
}

fn cmd_stats(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let s = c.stats().map_err(|e| e.to_string())?;
    println!("blocks:   {}", s.blocks);
    println!("roots:    {}", s.roots);
    println!("embedded: {}", s.embedded);
    Ok(())
}

fn cmd_conflicts(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let files = c.conflicts().map_err(|e| e.to_string())?;
    if files.is_empty() {
        println!("(no conflicts)");
    }
    for f in files {
        println!("{f}");
    }
    Ok(())
}

// ---------- writes ----------

fn cmd_create(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let (title, rest) = take_title(&args[1..]);
    if let Some(f) = rest.first() {
        return Err(format!("unknown flag: {f}"));
    }
    let body = read_stdin()?;
    let id = c
        .create_block(title.as_deref(), &body)
        .map_err(|e| e.to_string())?;
    println!("{id}");
    Ok(())
}

fn cmd_update(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let id = parse_id(req(args, 1, "<block-id>")?)?;
    let (title, rest) = take_title(&args[2..]);
    if let Some(f) = rest.first() {
        return Err(format!("unknown flag: {f}"));
    }
    let body = read_stdin()?;
    c.update_block(id, title.as_deref(), &body)
        .map_err(|e| e.to_string())?;
    println!("ok");
    Ok(())
}

fn cmd_set_tags(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let id = parse_id(req(args, 1, "<block-id>")?)?;
    let tags: Vec<String> = args[2..].to_vec();
    c.set_tags(id, tags).map_err(|e| e.to_string())?;
    println!("ok");
    Ok(())
}

fn cmd_link(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let src = parse_id(req(args, 1, "<source-id>")?)?;
    let dst = parse_id(req(args, 2, "<target-id>")?)?;
    let mut embed = false;
    for f in &args[3..] {
        if f == "--embed" {
            embed = true;
        } else {
            return Err(format!("unknown flag: {f}"));
        }
    }
    let outcome = c.link(src, dst, embed).map_err(|e| e.to_string())?;
    println!(
        "{}",
        match outcome {
            mdkb_core::LinkOutcome::Reference => "reference",
            mdkb_core::LinkOutcome::Transclusion => "transclusion",
            mdkb_core::LinkOutcome::DowngradedToReference =>
                "downgraded to reference (would cycle)",
        }
    );
    Ok(())
}

fn cmd_carve(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let parent = parse_id(req(args, 1, "<parent-id>")?)?;
    let (title, rest) = take_title(&args[2..]);
    if let Some(f) = rest.first() {
        return Err(format!("unknown flag: {f}"));
    }
    let body = read_stdin()?;
    let child = c
        .carve_block(parent, title.as_deref(), &body)
        .map_err(|e| e.to_string())?;
    println!("{child}");
    Ok(())
}

fn cmd_delete(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    let id = parse_id(req(args, 1, "<block-id>")?)?;
    c.delete_block(id).map_err(|e| e.to_string())?;
    println!("ok");
    Ok(())
}

// ---------- maintenance ----------

fn cmd_rebuild(args: &[String]) -> Result<(), String> {
    let c = client(req(args, 0, "<vault-dir>")?)?;
    c.rebuild().map_err(|e| e.to_string())?;
    println!("rebuilt");
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<(), String> {
    let dir = req(args, 0, "<vault-dir>")?;
    let mut manifest_path: Option<String> = None;
    let mut root: Option<String> = None;
    let mut tag: Option<String> = None;
    let mut include_non_root = false;
    let mut follow_links = false;
    let mut check = false;
    let mut raw = false;
    for flag in &args[1..] {
        if let Some(p) = flag.strip_prefix("--manifest=") {
            manifest_path = Some(p.to_string());
        } else if let Some(r) = flag.strip_prefix("--root=") {
            root = Some(r.to_string());
        } else if let Some(t) = flag.strip_prefix("--tag=") {
            tag = Some(t.to_string());
        } else if flag == "--include-non-root" {
            include_non_root = true;
        } else if flag == "--follow-links" {
            follow_links = true;
        } else if flag == "--check" {
            check = true;
        } else if flag == "--raw" {
            raw = true;
        } else {
            return Err(format!("unknown flag: {flag}"));
        }
    }
    if tag.is_some() && manifest_path.is_some() {
        return Err("--tag and --manifest are mutually exclusive selectors".into());
    }
    if include_non_root && tag.is_none() {
        return Err("--include-non-root only applies with --tag".into());
    }
    if manifest_path.is_some() && (follow_links || raw) {
        return Err(
            "--follow-links and --raw don't apply to --manifest (its paths and per-entry \
                    banner policy are explicit); use them with --tag or the whole-KB export"
                .into(),
        );
    }

    // Build the export request. Its type makes illegal combinations unrepresentable, so the only
    // job here is to map the parsed flags onto the right variant:
    //   --manifest=PATH          → Manifest(text)
    //   --tag=NAME               → Slugs{ Tag } (the default export.manifest is NOT consulted)
    //   --follow-links / --raw   → Slugs{ AllRoots } (slug-mode signals; bypass the default
    //                              manifest, whose paths/banners are explicit)
    //   (none, default exists)   → Manifest(default text)
    //   (none, no default)       → Slugs{ AllRoots } (the whole-KB dump)
    let default_manifest = format!("{dir}/export.manifest");
    let read_manifest = |p: &str| -> Result<ExportRequest, String> {
        std::fs::read_to_string(p)
            .map(ExportRequest::Manifest)
            .map_err(|e| format!("reading manifest {p}: {e}"))
    };
    let request: ExportRequest = if let Some(name) = tag {
        ExportRequest::Slugs {
            selection: SlugSelection::Tag {
                name,
                include_non_root,
            },
            follow_links,
            raw,
        }
    } else if let Some(p) = &manifest_path {
        read_manifest(p)?
    } else if !follow_links && !raw && Path::new(&default_manifest).exists() {
        read_manifest(&default_manifest)?
    } else {
        ExportRequest::Slugs {
            selection: SlugSelection::AllRoots,
            follow_links,
            raw,
        }
    };

    // A manifest names exact paths (so it writes relative to cwd); a slug dump emits `<slug>.md`
    // and defaults into `docs-export/` to avoid scattering files in cwd.
    let slug_dump = matches!(request, ExportRequest::Slugs { .. });
    let root = root.unwrap_or_else(|| {
        if slug_dump {
            "docs-export".into()
        } else {
            ".".into()
        }
    });

    // The daemon plans the docs against its warm vault (rendering + banner live in core).
    let docs = client(dir)?
        .plan_exports(request)
        .map_err(|e| e.to_string())?;

    let root = Path::new(&root);
    let mut drifted = Vec::new();
    let mut warnings: Vec<&String> = Vec::new();
    let mut wrote = 0usize;
    for doc in &docs {
        warnings.extend(doc.warnings.iter());
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

    // Dropped-link warnings are informational (they don't affect drift/exit status): an exported
    // doc linked a real block that isn't in this export, so the link became plain text.
    for w in &warnings {
        eprintln!("warning: {w}");
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
