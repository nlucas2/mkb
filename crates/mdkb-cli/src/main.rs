//! `mdkb` — the mdkb CLI.
//!
//! A **thin client**: every command connects to a vault's daemon (auto-starting a detached
//! `mdkbd` if none is running) and dispatches over the socket — exactly like the MCP server and
//! the desktop app. There is no separate in-process engine: the daemon owns the one persistent,
//! warm index and is the single writer, so the CLI never re-parses or re-embeds the vault and
//! never races the daemon. The unit is the **block** (one file). See `AGENTS.md`.
//!
//! Which vault a command acts on is resolved by the shared connection layer
//! ([`mdkb_protocol::resolve_client`]) with the precedence **`--vault` flag > `$MDKB_VAULT` >
//! the registry default (`vaults.json`) > the built-in `~/mdkb-vault`** — so configuring a default
//! once works across every client. `--remote`/`--socket` (or their env vars) connect to an
//! explicit daemon instead.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use mdkb_core::export::{ExportRequest, SlugSelection};
use mdkb_core::{BlockId, SearchQuery};
use mdkb_protocol::{
    connect_resolved, resolve_client, resolve_target, Client, ClientInputs, EnvSnapshot, Registry,
    ResolvedTarget,
};

/// Connection options shared by every subcommand (clap `global`, so they may appear before or
/// after the subcommand). These are the *explicit* inputs only; environment variables and the
/// registry default are applied by [`mdkb_protocol::resolve_client`], keeping the precedence in
/// one shared place.
#[derive(Args, Debug, Default)]
struct GlobalArgs {
    /// Vault directory to act on (supports a leading `~`). Overrides $MDKB_VAULT and the
    /// configured registry default.
    #[arg(long, global = true, value_name = "DIR")]
    vault: Option<PathBuf>,

    /// Connect to a remote daemon `host:port` over TCP instead of a local vault.
    #[arg(long, global = true, value_name = "HOST:PORT")]
    remote: Option<String>,

    /// Token to present to a remote daemon (required with --remote / $MDKB_REMOTE).
    #[arg(long, global = true, value_name = "TOKEN")]
    token: Option<String>,

    /// Dial this explicit local socket instead of deriving one from the vault.
    #[arg(long, global = true, value_name = "PATH")]
    socket: Option<PathBuf>,
}

impl GlobalArgs {
    fn inputs(&self) -> ClientInputs {
        ClientInputs {
            vault: self.vault.clone(),
            remote: self.remote.clone(),
            token: self.token.clone(),
            socket: self.socket.clone(),
        }
    }

    /// Resolve and connect a client (auto-starting a local daemon if needed).
    fn connect(&self) -> Result<Client, String> {
        resolve_client(&self.inputs(), None)
    }
}

#[derive(Parser)]
#[command(
    name = "mdkb",
    version,
    about = "Markdown knowledge base CLI — a thin client over the mdkb daemon",
    long_about = "Markdown knowledge base CLI.\n\nEvery command connects to a vault's daemon \
                  (auto-starting it). The vault is chosen by --vault, else $MDKB_VAULT, else the \
                  configured registry default (vaults.json), else ~/mdkb-vault. Use \
                  --remote/--socket (or $MDKB_REMOTE+$MDKB_TOKEN / $MDKB_SOCKET) for an explicit \
                  daemon."
)]
struct Cli {
    #[command(flatten)]
    global: GlobalArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check the daemon is reachable.
    Ping,
    /// List root blocks (id  title).
    List,
    /// Render a block, children resolved.
    Render {
        /// Block id.
        id: String,
        /// Published form: embeds dissolved inline, refs as plain titles.
        #[arg(long)]
        flat: bool,
    },
    /// Raw Markdown body of a block.
    Get {
        /// Block id.
        id: String,
    },
    /// Search (keyword + semantic). The query also accepts inline operators
    /// (tag:<t> #<t> lang:<l> code:<l> created:before:<date> updated:after:<date> has:<k> missing:<k>).
    Search {
        /// Query text.
        query: String,
        /// Restrict to a code-fence language.
        #[arg(long)]
        lang: Option<String>,
        /// Require a tag (repeatable).
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Maximum number of hits.
        #[arg(long)]
        limit: Option<usize>,
        /// Only blocks created on/after this date (YYYY-MM-DD or RFC3339).
        #[arg(long = "created-after")]
        created_after: Option<String>,
        /// Only blocks created before this date.
        #[arg(long = "created-before")]
        created_before: Option<String>,
        /// Only blocks updated on/after this date.
        #[arg(long = "updated-after")]
        updated_after: Option<String>,
        /// Only blocks updated before this date.
        #[arg(long = "updated-before")]
        updated_before: Option<String>,
        /// Require a property to be present (repeatable).
        #[arg(long = "has")]
        has: Vec<String>,
        /// Require a property to be absent (repeatable).
        #[arg(long = "missing")]
        missing: Vec<String>,
    },
    /// All tags with block counts.
    Tags,
    /// A block's properties (key<TAB>value per line).
    Props {
        /// Block id.
        id: String,
    },
    /// A block's metadata (created, updated, locked, tags, props).
    Info {
        /// Block id.
        id: String,
    },
    /// Blocks that reference/embed a block.
    Backlinks {
        /// Block id.
        id: String,
    },
    /// Outgoing links/embeds from a block.
    Links {
        /// Block id.
        id: String,
    },
    /// Index statistics.
    Stats,
    /// Cloud-sync conflict files.
    Conflicts,
    /// List orphaned assets (files under `assets/` no block references); `--prune` deletes them.
    Assets {
        /// Delete the orphaned assets instead of only listing them.
        #[arg(long)]
        prune: bool,
    },
    /// Create a block (body from stdin); prints the new id.
    Create {
        /// Optional block title.
        #[arg(long)]
        title: Option<String>,
    },
    /// Overwrite a block's title + body (body from stdin).
    Update {
        /// Block id.
        id: String,
        /// New title.
        #[arg(long)]
        title: Option<String>,
        /// Override the guard that refuses an emptying/truncating edit.
        #[arg(long)]
        force: bool,
    },
    /// Set managed (frontmatter) tags ([] clears).
    SetTags {
        /// Block id.
        id: String,
        /// Tags to set (none clears).
        tags: Vec<String>,
    },
    /// Add/update block properties (preserves the rest).
    SetProps {
        /// Block id.
        id: String,
        /// `key=value` pairs.
        #[arg(value_name = "KEY=VALUE")]
        pairs: Vec<String>,
    },
    /// Remove the named block properties (preserves the rest).
    UnsetProps {
        /// Block id.
        id: String,
        /// Property keys to remove (at least one).
        #[arg(num_args = 1.., required = true)]
        keys: Vec<String>,
    },
    /// Reference (or --embed: transclude) dst from src.
    Link {
        /// Source block id.
        src: String,
        /// Target block id.
        dst: String,
        /// Transclude instead of plain reference.
        #[arg(long)]
        embed: bool,
    },
    /// Carve a new child block (body from stdin); prints the child id.
    Carve {
        /// Parent block id.
        parent: String,
        /// Optional child title.
        #[arg(long)]
        title: Option<String>,
    },
    /// Inline parent's single ![[child]] embed and delete it (child must be referenced once).
    Flatten {
        /// Parent block id.
        parent: String,
        /// Child block id.
        child: String,
    },
    /// Delete a block.
    Delete {
        /// Block id.
        id: String,
    },
    /// Rebuild the index from blocks/.
    Rebuild,
    /// Generate flat docs from blocks (docs-as-data).
    Export(ExportArgs),
}

/// `export` flags (its own struct because there are several, with cross-field rules).
#[derive(Args)]
struct ExportArgs {
    /// Use a manifest file (TOML, or JSON by .json suffix) instead of the default export.toml.
    #[arg(long)]
    manifest: Option<String>,
    /// Output root (default: docs-export/ for a slug dump, . for a manifest).
    #[arg(long)]
    root: Option<String>,
    /// Dump roots carrying this tag to <slug>.md.
    #[arg(long)]
    tag: Option<String>,
    /// With --tag: include every tagged block, not only roots.
    #[arg(long = "include-non-root")]
    include_non_root: bool,
    /// Pull linked blocks outside the export into it.
    #[arg(long = "follow-links")]
    follow_links: bool,
    /// Verify only; non-zero exit on drift (writes nothing).
    #[arg(long)]
    check: bool,
    /// Omit the @generated banner.
    #[arg(long)]
    raw: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let g = &cli.global;
    match cli.command {
        Command::Ping => cmd_ping(g),
        Command::List => cmd_list(g),
        Command::Render { id, flat } => cmd_render(g, &id, flat),
        Command::Get { id } => cmd_get(g, &id),
        Command::Search {
            query,
            lang,
            tags,
            limit,
            created_after,
            created_before,
            updated_after,
            updated_before,
            has,
            missing,
        } => cmd_search(
            g,
            &query,
            SearchFlags {
                lang,
                tags,
                limit,
                created_after,
                created_before,
                updated_after,
                updated_before,
                has,
                missing,
            },
        ),
        Command::Tags => cmd_tags(g),
        Command::Props { id } => cmd_props(g, &id),
        Command::Info { id } => cmd_info(g, &id),
        Command::Backlinks { id } => cmd_links(g, &id, true),
        Command::Links { id } => cmd_links(g, &id, false),
        Command::Stats => cmd_stats(g),
        Command::Conflicts => cmd_conflicts(g),
        Command::Assets { prune } => cmd_assets(g, prune),
        Command::Create { title } => cmd_create(g, title.as_deref()),
        Command::Update { id, title, force } => cmd_update(g, &id, title.as_deref(), force),
        Command::SetTags { id, tags } => cmd_set_tags(g, &id, tags),
        Command::SetProps { id, pairs } => cmd_set_props(g, &id, &pairs),
        Command::UnsetProps { id, keys } => cmd_unset_props(g, &id, keys),
        Command::Link { src, dst, embed } => cmd_link(g, &src, &dst, embed),
        Command::Carve { parent, title } => cmd_carve(g, &parent, title.as_deref()),
        Command::Flatten { parent, child } => cmd_flatten(g, &parent, &child),
        Command::Delete { id } => cmd_delete(g, &id),
        Command::Rebuild => cmd_rebuild(g),
        Command::Export(args) => cmd_export(g, &args),
    }
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

// ---------- reads ----------

fn cmd_ping(g: &GlobalArgs) -> Result<(), String> {
    if g.connect()?.ping() {
        println!("ok");
        Ok(())
    } else {
        Err("daemon did not respond".to_string())
    }
}

fn cmd_list(g: &GlobalArgs) -> Result<(), String> {
    let c = g.connect()?;
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

fn cmd_render(g: &GlobalArgs, id: &str, flat: bool) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
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

fn cmd_get(g: &GlobalArgs, id: &str) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
    match c.get_block_source(id).map_err(|e| e.to_string())? {
        Some(src) => {
            print!("{src}");
            Ok(())
        }
        None => Err("block not found".to_string()),
    }
}

/// The optional filters for `search`, grouped so the handler signature stays small.
struct SearchFlags {
    lang: Option<String>,
    tags: Vec<String>,
    limit: Option<usize>,
    created_after: Option<String>,
    created_before: Option<String>,
    updated_after: Option<String>,
    updated_before: Option<String>,
    has: Vec<String>,
    missing: Vec<String>,
}

fn cmd_search(g: &GlobalArgs, query_text: &str, flags: SearchFlags) -> Result<(), String> {
    let c = g.connect()?;
    // The positional query understands the same inline operators as the app/MCP
    // (tag:, #tag, lang:/code:) via the shared parser; the flags add to it.
    let mut q = SearchQuery::parse(query_text);
    if let Some(l) = flags.lang {
        q.lang = Some(l.to_lowercase());
    }
    for t in flags.tags {
        let t = t.to_lowercase();
        if !q.tags.contains(&t) {
            q.tags.push(t);
        }
    }
    if let Some(n) = flags.limit {
        q.limit = n;
    }
    if let Some(d) = flags.created_after {
        q.created_after = Some(parse_date_flag(&d)?);
    }
    if let Some(d) = flags.created_before {
        q.created_before = Some(parse_date_flag(&d)?);
    }
    if let Some(d) = flags.updated_after {
        q.updated_after = Some(parse_date_flag(&d)?);
    }
    if let Some(d) = flags.updated_before {
        q.updated_before = Some(parse_date_flag(&d)?);
    }
    for k in flags.has {
        push_prop_key(&mut q.has_prop, &k);
    }
    for k in flags.missing {
        push_prop_key(&mut q.lacks_prop, &k);
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

fn parse_date_flag(d: &str) -> Result<String, String> {
    mdkb_core::clock::parse_query_date(d)
        .ok_or_else(|| format!("bad date {d:?}: use YYYY-MM-DD or an RFC 3339 timestamp"))
}

fn push_prop_key(keys: &mut Vec<String>, key: &str) {
    let k = key.trim().to_lowercase();
    if !k.is_empty() && !keys.contains(&k) {
        keys.push(k);
    }
}

fn cmd_info(g: &GlobalArgs, id: &str) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
    match c.get_block(id).map_err(|e| e.to_string())? {
        Some(rec) => {
            println!("id       {}", rec.id);
            println!("title    {}", rec.display_title());
            println!("created  {}", rec.created.as_deref().unwrap_or("—"));
            println!("updated  {}", rec.updated.as_deref().unwrap_or("—"));
            println!("locked   {}", rec.locked);
            if !rec.tags.is_empty() {
                println!("tags     {}", rec.tags.join(", "));
            }
            for (k, v) in &rec.props {
                println!("prop     {k} = {v}");
            }
            Ok(())
        }
        None => Err("block not found".to_string()),
    }
}

fn cmd_tags(g: &GlobalArgs) -> Result<(), String> {
    let c = g.connect()?;
    let tags = c.list_tags().map_err(|e| e.to_string())?;
    if tags.is_empty() {
        println!("(no tags)");
    }
    for t in tags {
        println!("{:>4}  #{}", t.count, t.tag);
    }
    Ok(())
}

fn cmd_links(g: &GlobalArgs, id: &str, incoming: bool) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
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

fn cmd_stats(g: &GlobalArgs) -> Result<(), String> {
    let c = g.connect()?;
    let s = c.stats().map_err(|e| e.to_string())?;
    println!("blocks:   {}", s.blocks);
    println!("roots:    {}", s.roots);
    println!("embedded: {}", s.embedded);
    Ok(())
}

fn cmd_conflicts(g: &GlobalArgs) -> Result<(), String> {
    let c = g.connect()?;
    let files = c.conflicts().map_err(|e| e.to_string())?;
    if files.is_empty() {
        println!("(no conflicts)");
    }
    for f in files {
        println!("{f}");
    }
    Ok(())
}

fn cmd_assets(g: &GlobalArgs, prune: bool) -> Result<(), String> {
    let c = g.connect()?;
    let orphans = c.orphan_assets().map_err(|e| e.to_string())?;
    if orphans.is_empty() {
        println!("(no orphaned assets)");
        return Ok(());
    }
    if prune {
        for path in &orphans {
            c.remove_asset(path).map_err(|e| e.to_string())?;
            println!("removed {path}");
        }
        println!("pruned {} orphaned asset(s)", orphans.len());
    } else {
        for path in &orphans {
            println!("{path}");
        }
        eprintln!(
            "{} orphaned asset(s); re-run with --prune to delete them",
            orphans.len()
        );
    }
    Ok(())
}

// ---------- writes ----------

fn cmd_create(g: &GlobalArgs, title: Option<&str>) -> Result<(), String> {
    let c = g.connect()?;
    let body = read_stdin()?;
    let id = c.create_block(title, &body).map_err(|e| e.to_string())?;
    println!("{id}");
    Ok(())
}

fn cmd_update(g: &GlobalArgs, id: &str, title: Option<&str>, force: bool) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
    let body = read_stdin()?;
    c.update_block(id, title, &body, force)
        .map_err(|e| e.to_string())?;
    println!("ok");
    Ok(())
}

fn cmd_set_tags(g: &GlobalArgs, id: &str, tags: Vec<String>) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
    c.set_tags(id, tags).map_err(|e| e.to_string())?;
    println!("ok");
    Ok(())
}

fn cmd_props(g: &GlobalArgs, id: &str) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
    match c.get_block(id).map_err(|e| e.to_string())? {
        Some(rec) => {
            for (k, v) in &rec.props {
                println!("{k}\t{v}");
            }
            Ok(())
        }
        None => Err("block not found".to_string()),
    }
}

fn cmd_set_props(g: &GlobalArgs, id: &str, pairs: &[String]) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
    let mut props: Vec<(String, String)> = Vec::new();
    for pair in pairs {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("expected key=value, got: {pair}"))?;
        props.push((k.to_string(), v.to_string()));
    }
    c.set_props(id, props).map_err(|e| e.to_string())?;
    println!("ok");
    Ok(())
}

fn cmd_unset_props(g: &GlobalArgs, id: &str, keys: Vec<String>) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
    // clap guarantees >= 1 key (num_args = 1.., required), so no empty-check is needed here.
    c.unset_props(id, keys).map_err(|e| e.to_string())?;
    println!("ok");
    Ok(())
}

fn cmd_link(g: &GlobalArgs, src: &str, dst: &str, embed: bool) -> Result<(), String> {
    let c = g.connect()?;
    let src = parse_id(src)?;
    let dst = parse_id(dst)?;
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

fn cmd_carve(g: &GlobalArgs, parent: &str, title: Option<&str>) -> Result<(), String> {
    let c = g.connect()?;
    let parent = parse_id(parent)?;
    let body = read_stdin()?;
    let child = c
        .carve_block(parent, title, &body)
        .map_err(|e| e.to_string())?;
    println!("{child}");
    Ok(())
}

fn cmd_flatten(g: &GlobalArgs, parent: &str, child: &str) -> Result<(), String> {
    let c = g.connect()?;
    let parent = parse_id(parent)?;
    let child = parse_id(child)?;
    c.flatten(parent, child).map_err(|e| e.to_string())?;
    println!("ok");
    Ok(())
}

fn cmd_delete(g: &GlobalArgs, id: &str) -> Result<(), String> {
    let c = g.connect()?;
    let id = parse_id(id)?;
    c.delete_block(id).map_err(|e| e.to_string())?;
    println!("ok");
    Ok(())
}

// ---------- maintenance ----------

fn cmd_rebuild(g: &GlobalArgs) -> Result<(), String> {
    let c = g.connect()?;
    c.rebuild().map_err(|e| e.to_string())?;
    println!("rebuilt");
    Ok(())
}

fn cmd_export(g: &GlobalArgs, args: &ExportArgs) -> Result<(), String> {
    if args.tag.is_some() && args.manifest.is_some() {
        return Err("--tag and --manifest are mutually exclusive selectors".into());
    }
    if args.include_non_root && args.tag.is_none() {
        return Err("--include-non-root only applies with --tag".into());
    }
    if args.manifest.is_some() && (args.follow_links || args.raw) {
        return Err(
            "--follow-links and --raw don't apply to --manifest (its paths and per-entry \
                    banner policy are explicit); use them with --tag or the whole-KB export"
                .into(),
        );
    }

    // Resolve the connection target once: `export` needs both a client AND, for the default
    // manifest lookup, the local vault directory (when the target is a local vault).
    let inputs = g.inputs();
    let env = EnvSnapshot::read();
    let registry_default = Registry::load().default_connection();
    let target = resolve_target(&inputs, &env, Some(&registry_default))?;
    let vault_dir: Option<PathBuf> = match &target {
        ResolvedTarget::LocalVault { vault } => Some(vault.clone()),
        _ => None,
    };

    // Build the export request. Its type makes illegal combinations unrepresentable, so the only
    // job here is to map the parsed flags onto the right variant:
    //   --manifest=PATH          → Manifest(entries)  (parsed here; TOML, or JSON by .json suffix)
    //   --tag=NAME               → Slugs{ Tag } (the default export.toml is NOT consulted)
    //   --follow-links / --raw   → Slugs{ AllRoots } (slug-mode signals; bypass the default
    //                              manifest, whose paths/banners are explicit)
    //   (none, default exists)   → Manifest(entries from default export.toml)
    //   (none, no default)       → Slugs{ AllRoots } (the whole-KB dump)
    // The manifest is parsed client-side (defaults resolved) and shipped as structured entries, so
    // the protocol stays uniformly JSON and the daemon never parses the on-disk format.
    let default_manifest = vault_dir
        .as_ref()
        .map(|d| d.join("export.toml"))
        .filter(|p| p.exists());
    let read_manifest = |p: &str| -> Result<ExportRequest, String> {
        let text = std::fs::read_to_string(p).map_err(|e| format!("reading manifest {p}: {e}"))?;
        let manifest = if p.ends_with(".json") {
            mdkb_core::export::Manifest::parse_json(&text)
        } else {
            mdkb_core::export::Manifest::parse(&text)
        }
        .map_err(|e| format!("{p}: {e}"))?;
        Ok(ExportRequest::Manifest(manifest.entries))
    };
    let request: ExportRequest = if let Some(name) = args.tag.clone() {
        ExportRequest::Slugs {
            selection: SlugSelection::Tag {
                name,
                include_non_root: args.include_non_root,
            },
            follow_links: args.follow_links,
            raw: args.raw,
        }
    } else if let Some(p) = &args.manifest {
        read_manifest(p)?
    } else if let Some(default) = default_manifest.filter(|_| !args.follow_links && !args.raw) {
        read_manifest(&default.to_string_lossy())?
    } else {
        ExportRequest::Slugs {
            selection: SlugSelection::AllRoots,
            follow_links: args.follow_links,
            raw: args.raw,
        }
    };

    // A manifest names exact paths (so it writes relative to cwd); a slug dump emits `<slug>.md`
    // and defaults into `docs-export/` to avoid scattering files in cwd.
    let slug_dump = matches!(request, ExportRequest::Slugs { .. });
    let root = args.root.clone().unwrap_or_else(|| {
        if slug_dump {
            "docs-export".into()
        } else {
            ".".into()
        }
    });

    // The daemon plans the docs against its warm vault (rendering + banner live in core).
    let docs = connect_resolved(target, None)?
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
        if args.check {
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

    if args.check {
        if drifted.is_empty() {
            println!("up to date ({} doc(s))", docs.len());
            Ok(())
        } else {
            for p in &drifted {
                eprintln!("drift: {p}");
            }
            Err(format!(
                "{} doc(s) out of date; run `mdkb export` to regenerate",
                drifted.len()
            ))
        }
    } else {
        println!("exported {} doc(s) ({wrote} written)", docs.len());
        Ok(())
    }
}
