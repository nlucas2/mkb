//! Transport-neutral operations behind every mkb UI command.
//!
//! Each function here is the **logic** a front-end command runs — fetching from the daemon over a
//! [`mkb_protocol::Client`], rendering through [`mkb_view`], and shaping a serializable view for the
//! UI. It deliberately depends on **no presentation shell** (no Tauri, no HTTP framework) so every
//! front-end — the Tauri desktop app today, a web server later — invokes the *same* implementation
//! and cannot diverge (the "one shared core" rule in `AGENTS.md`, applied at the app layer).
//!
//! A front-end stays a thin adapter: it owns transport (IPC / HTTP), the connection lifecycle, and
//! genuinely-native affordances (a folder dialog, revealing a folder in the OS, respawning a local
//! daemon), then calls into these functions for everything else. Errors are `String` so they cross
//! any transport unchanged.

use std::path::{Path, PathBuf};

use mkb_core::{
    BlockId, GraphData, GroupAxis, GroupTree, HierTree, LinkOutcome, SearchQuery, UpdateOutcome,
};
use mkb_protocol::{discover_running, Client, ConnectionConfig, Registry};
use mkb_view::{
    block_title, markdown_to_html_with_assets_indexed, search_results_html, top_level_block_spans,
    ResultRow,
};
use serde::Serialize;

// ----- view DTOs (the wire shape every front-end returns; defined once so they can't diverge) -----

/// A block reduced to `{id, title}` for the sidebar / link picker.
#[derive(Serialize)]
pub struct NavBlock {
    id: String,
    title: String,
}

/// A block prepared for the front-end: stable id, display title, raw Markdown (for editing),
/// and rendered HTML (children expanded, references as chips). HTML is produced by the shared
/// `mkb-view` renderer so any UI renders identically.
#[derive(Serialize)]
pub struct BlockView {
    id: String,
    title: String,
    tags: Vec<String>,
    fm_tags: Vec<String>,
    props: Vec<(String, String)>,
    content: String,
    html: String,
    locked: bool,
    /// Source byte spans (`[start, end]`) of the raw body's top-level blocks, in document order.
    /// The Nth span aligns with the `data-bi="N"` element in `html`, so the UI can carve a run of
    /// whole rendered blocks by mapping to these raw offsets.
    outline: Vec<[usize; 2]>,
}

/// The result of a [`save_block`] under optimistic concurrency, serialised for the front-end.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SaveOutcome {
    /// The save was applied; `version` is the block's new content version (the editor adopts it as
    /// its new base so the user can keep editing without re-reading).
    Applied { version: String },
    /// The save was rejected because the block changed since the editor opened it. Carries the
    /// current state so the UI can offer reload/merge. Nothing was written.
    Conflict {
        current_title: Option<String>,
        current_body: String,
        version: String,
    },
}

/// A tag with its block count, for the tag browser.
#[derive(Serialize)]
pub struct TagCountView {
    tag: String,
    count: usize,
}

/// One row of the vault switcher/manager: a registry entry plus whether it's the active connection
/// and whether it's the launch default.
#[derive(Serialize)]
pub struct VaultRow {
    /// The entry's stable name (used to switch/rename/remove it).
    name: String,
    /// `"local"` or `"remote"` — drives the icon/label in the UI.
    kind: String,
    /// Human target: the local vault path, or the remote `host:port`.
    target: String,
    /// True for the vault the app is currently connected to.
    active: bool,
    /// True for the registry's launch default (the vault opened on next start).
    default: bool,
}

/// A local vault with a **currently running** daemon that isn't in the registry yet — surfaced so
/// the user can add it with one click (e.g. a repo vault an agent spun up).
#[derive(Serialize)]
pub struct DiscoveredRow {
    /// Suggested name (the vault folder's basename).
    name: String,
    /// Absolute vault path the running daemon reports.
    path: String,
    /// Block count (a cheap "is this the vault I mean?" hint).
    blocks: usize,
}

/// The connection indicator: a friendly `label`, the full `endpoint` for a tooltip, and whether the
/// daemon is currently reachable.
#[derive(Serialize)]
pub struct ConnStatus {
    label: String,
    endpoint: String,
    connected: bool,
}

// ----- reads -----

/// Root blocks as `{id, title}` for the sidebar.
pub fn list_blocks(client: &Client) -> Result<Vec<NavBlock>, String> {
    let roots = client.list_roots().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for id in roots {
        let title = client
            .get_block(id.clone())
            .map_err(|e| e.to_string())?
            .map(|b| block_title(b.title.as_deref(), &b.content))
            .unwrap_or_else(|| id.to_string());
        out.push(NavBlock {
            id: id.to_string(),
            title,
        });
    }
    Ok(out)
}

/// Every block as `{id, title}` — powers the `[[` link/embed picker. Reuses the search path
/// (an empty query returns all block records), so there is no second listing path.
pub fn block_index(client: &Client) -> Result<Vec<NavBlock>, String> {
    let all = client
        .search(SearchQuery {
            limit: 10_000,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    Ok(all
        .into_iter()
        .map(|h| NavBlock {
            id: h.block.id.to_string(),
            title: block_title(h.block.title.as_deref(), &h.block.content),
        })
        .collect())
}

/// Render a block to HTML (children resolved by the daemon, Markdown→HTML by mkb-view).
///
/// `vault_root` is `Some(path)` for a local vault, so relative image sources (`![](assets/x.png)`)
/// resolve against it for the WebView asset protocol; `None` for a remote vault (no local files to
/// serve — images render only if external URLs).
pub fn render_block(
    client: &Client,
    vault_root: Option<&Path>,
    id: &str,
) -> Result<BlockView, String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    let rb = client
        .rendered_block(bid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("block not found: {id}"))?;
    let html = markdown_to_html_with_assets_indexed(&rb.rendered, vault_root);
    // Outline of the RAW body's top-level blocks; the Nth span aligns with the Nth data-bi element.
    let outline = top_level_block_spans(&rb.raw)
        .into_iter()
        .map(|(s, e)| [s, e])
        .collect();
    Ok(BlockView {
        html,
        content: rb.raw,
        title: rb.title,
        tags: rb.tags,
        fm_tags: rb.fm_tags,
        props: rb.props,
        id: rb.id.to_string(),
        locked: rb.locked,
        outline,
    })
}

/// Raw Markdown body of a block (for the editor).
pub fn block_source(client: &Client, id: &str) -> Result<String, String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    Ok(client
        .get_block_source(bid)
        .map_err(|e| e.to_string())?
        .unwrap_or_default())
}

/// The block's title (if any).
pub fn block_title_of(client: &Client, id: &str) -> Result<Option<String>, String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    Ok(client
        .get_block(bid)
        .map_err(|e| e.to_string())?
        .and_then(|b| b.title))
}

/// The whole block-level knowledge graph.
pub fn graph(client: &Client) -> Result<GraphData, String> {
    client.graph().map_err(|e| e.to_string())
}

/// Group blocks by an axis into a `/`-nested tree for the sidebar group-by view. `axis` is
/// `"tags"` or `"prop:<key>"` (e.g. `"prop:path"`); anything else is treated as a property key.
pub fn group_blocks(client: &Client, axis: &str) -> Result<GroupTree, String> {
    let axis = match axis.strip_prefix("prop:") {
        Some(key) => GroupAxis::Property(key.to_string()),
        None if axis == "tags" => GroupAxis::Tags,
        None => GroupAxis::Property(axis.to_string()),
    };
    client.group_blocks_by(axis).map_err(|e| e.to_string())
}

/// The composition hierarchy (roots → embeds/links) as an expandable tree.
pub fn hierarchy(client: &Client) -> Result<HierTree, String> {
    client.hierarchy().map_err(|e| e.to_string())
}

/// Backlinks (blocks that reference or transclude `id`), as nav blocks.
pub fn backlinks(client: &Client, id: &str) -> Result<Vec<NavBlock>, String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    let rows = client.backlinks(bid).map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for r in rows {
        if !seen.insert(r.source_id.clone()) {
            continue;
        }
        let title = client
            .get_block(r.source_id.clone())
            .map_err(|e| e.to_string())?
            .map(|b| block_title(b.title.as_deref(), &b.content))
            .unwrap_or_else(|| r.source_id.to_string());
        out.push(NavBlock {
            id: r.source_id.to_string(),
            title,
        });
    }
    Ok(out)
}

/// Search and return a ready-to-inject HTML fragment. The query string supports inline
/// operators (`tag:`, `#tag`, `lang:`/`code:`) parsed by the shared `SearchQuery::parse` so the
/// app, CLI and MCP all understand the same syntax.
pub fn search(client: &Client, query: &str) -> Result<String, String> {
    let q = SearchQuery::parse(query);
    let hits = client.search(q).map_err(|e| e.to_string())?;
    let rows: Vec<ResultRow> = hits
        .into_iter()
        .map(|h| ResultRow {
            id: h.block.id.to_string(),
            title: block_title(h.block.title.as_deref(), &h.block.content),
            tags: h.block.tags,
            content: h.block.content,
        })
        .collect();
    Ok(search_results_html(query, &rows))
}

/// The current content version of a block (optimistic-concurrency token the editor captures on
/// open and passes back on save).
pub fn block_version(client: &Client, id: &str) -> Result<Option<String>, String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    client.block_version(bid).map_err(|e| e.to_string())
}

/// Whether a block is locked (human-only). Cheap lookup used by the Blocks view to show a clear
/// "locked" cue instead of silently failing an edit.
pub fn block_locked(client: &Client, id: &str) -> Result<bool, String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    Ok(client
        .get_block(bid)
        .map_err(|e| e.to_string())?
        .map(|b| b.locked)
        .unwrap_or(false))
}

/// All tags in the vault with per-tag block counts, for the tag browser.
pub fn list_tags(client: &Client) -> Result<Vec<TagCountView>, String> {
    let tags = client.list_tags().map_err(|e| e.to_string())?;
    Ok(tags
        .into_iter()
        .map(|t| TagCountView {
            tag: t.tag,
            count: t.count,
        })
        .collect())
}

/// List orphaned assets (files under `assets/` referenced by no block) for the cleanup UI.
pub fn orphan_assets(client: &Client) -> Result<Vec<String>, String> {
    client.orphan_assets().map_err(|e| e.to_string())
}

// ----- writes -----

/// Update a block's title + body in place, under optimistic concurrency.
///
/// The desktop app is the human surface — the editor shows the full body being saved, so the
/// destructive-update guard (which protects against blind agent truncation) is bypassed with
/// `force=true`. The optimistic-concurrency guard is separate: when the editor passes the version
/// it opened against, a concurrent change is reported as a [`SaveOutcome::Conflict`] for the UI to
/// reconcile, rather than silently clobbered.
pub fn save_block(
    client: &Client,
    id: &str,
    title: Option<&str>,
    body: &str,
    base_version: Option<&str>,
) -> Result<SaveOutcome, String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    let outcome = client
        .update_block_checked(bid, title, body, true, base_version)
        .map_err(|e| e.to_string())?;
    Ok(match outcome {
        UpdateOutcome::Applied { version } => SaveOutcome::Applied { version },
        UpdateOutcome::Conflict {
            current_title,
            current_body,
            version,
        } => SaveOutcome::Conflict {
            current_title,
            current_body,
            version,
        },
    })
}

/// Create a new top-level block. Returns the new id.
pub fn create_block(client: &Client, title: Option<&str>, body: &str) -> Result<String, String> {
    client
        .create_block(title, body)
        .map(|id| id.to_string())
        .map_err(|e| e.to_string())
}

/// Largest asset a UI should import in one go. Kept comfortably under the daemon's 8 MiB
/// request-line cap once base64-expanded (~33%), so an oversized drop fails fast with a clear
/// message instead of a wire error.
pub const MAX_ASSET_BYTES: usize = 5 * 1024 * 1024;

/// Import an image (or other file) into the vault's `assets/` directory via the daemon, returning
/// the vault-relative path (`assets/<name>`) to insert into a block. Enforces [`MAX_ASSET_BYTES`].
pub fn add_asset(client: &Client, name: &str, data: &[u8]) -> Result<String, String> {
    if data.is_empty() {
        return Err("empty file".to_string());
    }
    if data.len() > MAX_ASSET_BYTES {
        return Err(format!(
            "file is too large ({:.1} MB); the limit is {} MB",
            data.len() as f64 / (1024.0 * 1024.0),
            MAX_ASSET_BYTES / (1024 * 1024)
        ));
    }
    client.add_asset(name, data).map_err(|e| e.to_string())
}

/// Delete an asset by its vault-relative `assets/…` path (orphan-sweep cleanup).
pub fn remove_asset(client: &Client, path: &str) -> Result<(), String> {
    client.remove_asset(path).map_err(|e| e.to_string())
}

/// Carve the selected byte range of a parent's body into a new child (replace in place).
/// Returns the new child id.
pub fn carve_selection(
    client: &Client,
    parent_id: &str,
    start: usize,
    end: usize,
) -> Result<String, String> {
    let pid = BlockId::parse(parent_id).map_err(|e| e.to_string())?;
    client
        .carve_selection(pid, start, end)
        .map(|id| id.to_string())
        .map_err(|e| e.to_string())
}

/// Delete a block.
pub fn delete_block(client: &Client, id: &str) -> Result<(), String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    client.delete_block(bid).map_err(|e| e.to_string())
}

/// Map a [`LinkOutcome`] to the short string the UI shows.
fn link_outcome_label(outcome: LinkOutcome) -> String {
    match outcome {
        LinkOutcome::Reference => "reference".to_string(),
        LinkOutcome::Transclusion => "transclusion".to_string(),
        LinkOutcome::DowngradedToReference => "downgraded".to_string(),
    }
}

/// Link or embed one block into another. Returns the outcome label (may report a downgrade).
pub fn link_blocks(
    client: &Client,
    source_id: &str,
    target_id: &str,
    embed: bool,
) -> Result<String, String> {
    let s = BlockId::parse(source_id).map_err(|e| e.to_string())?;
    let t = BlockId::parse(target_id).map_err(|e| e.to_string())?;
    let outcome = client.link(s, t, embed).map_err(|e| e.to_string())?;
    Ok(link_outcome_label(outcome))
}

/// Link/embed `target` at a position: before or after the existing directive that targets
/// `anchor_id` in `source`'s body (the "insert a block above/below this one" action).
pub fn link_block_at(
    client: &Client,
    source_id: &str,
    target_id: &str,
    embed: bool,
    anchor_id: &str,
    after: bool,
) -> Result<String, String> {
    let s = BlockId::parse(source_id).map_err(|e| e.to_string())?;
    let t = BlockId::parse(target_id).map_err(|e| e.to_string())?;
    let a = BlockId::parse(anchor_id).map_err(|e| e.to_string())?;
    let outcome = client
        .link_at(s, t, embed, a, after)
        .map_err(|e| e.to_string())?;
    Ok(link_outcome_label(outcome))
}

/// Link/embed `target` at a source byte `offset` in `source`'s body (snapped to paragraph
/// boundaries) — inserting a block above/below any rendered block via its outline offset.
pub fn link_block_at_offset(
    client: &Client,
    source_id: &str,
    target_id: &str,
    embed: bool,
    offset: usize,
) -> Result<String, String> {
    let s = BlockId::parse(source_id).map_err(|e| e.to_string())?;
    let t = BlockId::parse(target_id).map_err(|e| e.to_string())?;
    let outcome = client
        .link_at_offset(s, t, embed, offset)
        .map_err(|e| e.to_string())?;
    Ok(link_outcome_label(outcome))
}

/// Remove an embed/reference from a page: drop every directive targeting `target_id` from
/// `source_id`'s body (the block itself is untouched). The "remove from page" action.
pub fn unlink_blocks(client: &Client, source_id: &str, target_id: &str) -> Result<(), String> {
    let s = BlockId::parse(source_id).map_err(|e| e.to_string())?;
    let t = BlockId::parse(target_id).map_err(|e| e.to_string())?;
    client.unlink(s, t).map_err(|e| e.to_string())
}

/// Set a block's managed (frontmatter) tags to exactly `tags` (replaces them; empty clears).
pub fn set_tags(client: &Client, id: &str, tags: Vec<String>) -> Result<(), String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    client.set_tags(bid, tags).map_err(|e| e.to_string())
}

/// Add/update a block's properties (other properties are preserved).
pub fn set_props(client: &Client, id: &str, props: Vec<(String, String)>) -> Result<(), String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    client.set_props(bid, props).map_err(|e| e.to_string())
}

/// Remove named properties from a block (others preserved).
pub fn unset_props(client: &Client, id: &str, keys: Vec<String>) -> Result<(), String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    client.unset_props(bid, keys).map_err(|e| e.to_string())
}

/// Lock or unlock a block (the human-only flag). Only the desktop app is granted this scope; a
/// locked block is read-only to AI clients until a human unlocks it here.
pub fn set_lock(client: &Client, id: &str, locked: bool) -> Result<(), String> {
    let bid = BlockId::parse(id).map_err(|e| e.to_string())?;
    client.set_lock(bid, locked).map_err(|e| e.to_string())
}

// ----- vault registry (reads) -----

/// The active connection config: the saved registry default if present, else the built-in default
/// vault. Front-ends use this at startup and for the Settings page.
pub fn current_config() -> ConnectionConfig {
    if ConnectionConfig::config_path().exists() {
        ConnectionConfig::load()
    } else {
        ConnectionConfig::default()
    }
}

/// The known vaults from the registry (`vaults.json`), marking the active + default ones (given the
/// front-end's current `active` connection). This is the switcher/manager's data source. Block
/// counts are intentionally omitted: computing them would auto-start every vault's daemon just to
/// render the list.
pub fn list_vaults(active: &ConnectionConfig) -> Vec<VaultRow> {
    let reg = Registry::load();
    let default = reg.default.clone();
    reg.vaults
        .into_iter()
        .map(|e| {
            let (kind, target) = match &e.connection {
                ConnectionConfig::Local { vault } => ("local", vault.display().to_string()),
                ConnectionConfig::Remote { host, .. } => ("remote", host.clone()),
            };
            VaultRow {
                default: default.as_deref() == Some(e.name.as_str()),
                active: e.connection == *active,
                name: e.name,
                kind: kind.to_string(),
                target,
            }
        })
        .collect()
}

/// The connection indicator for the active connection `cfg`, reachable over `client`. The `label`
/// is the active vault's **registry name** (falling back to a target-derived name), suffixed with
/// its location kind, e.g. `personal (local)` or `team (remote)`.
pub fn connection_status(client: &Client, cfg: &ConnectionConfig) -> ConnStatus {
    let connected = client.ping();
    let endpoint = client.endpoint();
    let kind = match cfg {
        ConnectionConfig::Local { .. } => "local",
        ConnectionConfig::Remote { .. } => "remote",
    };
    let name = Registry::load()
        .vaults
        .into_iter()
        .find(|e| e.connection == *cfg)
        .map(|e| e.name)
        .unwrap_or_else(|| match cfg {
            ConnectionConfig::Local { vault } => vault
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| vault.display().to_string()),
            ConnectionConfig::Remote { host, .. } => host.clone(),
        });
    ConnStatus {
        label: format!("{name} ({kind})"),
        endpoint,
        connected,
    }
}

/// Canonicalize a `Local` connection's vault path (expanding a leading `~`) so a `~`- or
/// relatively-stored registry entry still matches a daemon's canonical path; `None` for remote.
fn canonical_local(conn: &ConnectionConfig) -> Option<PathBuf> {
    match conn {
        ConnectionConfig::Local { vault } => {
            let real = mkb_protocol::paths::expand_user(vault);
            Some(std::fs::canonicalize(&real).unwrap_or(real))
        }
        ConnectionConfig::Remote { .. } => None,
    }
}

/// Discover vaults with a running daemon that are **not already** in the registry (nor the active
/// vault `active`), so the Settings page can offer to add them. Registry entries are canonicalized
/// for the comparison so a `~`- or relatively-stored path still matches the daemon's canonical path.
pub fn discover_vaults(active: &ConnectionConfig) -> Vec<DiscoveredRow> {
    let mut known: Vec<PathBuf> = Registry::load()
        .vaults
        .iter()
        .filter_map(|e| canonical_local(&e.connection))
        .collect();
    known.extend(canonical_local(active));
    known.sort();
    known.dedup();
    discover_running()
        .into_iter()
        .filter(|d| !known.contains(&d.vault))
        .map(|d| DiscoveredRow {
            name: d.name_hint,
            path: d.vault.display().to_string(),
            blocks: d.blocks,
        })
        .collect()
}

// ----- vault registry (mutations) -----
//
// These mutate the registry and return the `ConnectionConfig` the front-end should **apply** (i.e.
// reconnect to), when the change affects the active connection. Applying it — reconnecting the live
// client, re-whitelisting the vault's assets for a WebView — is the front-end's job (its transport
// and platform), so it stays out of core.

/// Switch the **active** connection to an existing registry entry: return its connection for the
/// front-end to apply (reconnect to). This is deliberately *ephemeral* — it does **not** touch the
/// launch `default`, which is a separate, deliberate user choice changed only via
/// [`set_default_vault`]. (A per-window app or per-tab web client relaunches on the default; a mid-
/// session switch shouldn't silently repin it.)
pub fn switch_vault(name: &str) -> Result<ConnectionConfig, String> {
    Registry::load()
        .vaults
        .into_iter()
        .find(|e| e.name == name)
        .map(|e| e.connection)
        .ok_or_else(|| format!("no vault named {name:?}"))
}

/// Add (or create) a local vault: register it under `name` pointing at `path`, ensuring the folder
/// exists (so this both *adds an existing* vault and *creates a new empty* one — the daemon
/// initializes an empty folder on first connect). A blank `name` defaults to the folder basename.
/// Returns `Some(connection)` to apply when `activate` is set, else `None`.
pub fn add_local_vault(
    name: &str,
    path: &str,
    activate: bool,
) -> Result<Option<ConnectionConfig>, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("choose a folder for the vault".to_string());
    }
    // Ensure the folder exists (a no-op for an existing vault; creates a brand-new one). Expand a
    // leading `~` for the filesystem call, but store the path as typed so a `~`-relative entry stays
    // portable across machines in `vaults.json`.
    let real = mkb_protocol::paths::expand_user(path);
    std::fs::create_dir_all(&real).map_err(|e| format!("creating {}: {e}", real.display()))?;
    let conn = ConnectionConfig::Local {
        vault: PathBuf::from(path),
    };
    // Fall back to the folder basename when no name was given, so "Add" always yields a sensible
    // label even if the user typed a path without naming it.
    let name = if name.trim().is_empty() {
        conn.suggested_name()
    } else {
        name.to_string()
    };
    let mut reg = Registry::load();
    reg.add_vault(&name, conn.clone())?;
    if activate {
        reg.set_default(&name)?;
    }
    reg.save()?;
    Ok(activate.then_some(conn))
}

/// Add a **remote** daemon as a named registry entry (the remote counterpart of [`add_local_vault`]).
/// Returns `Some(connection)` to apply when `activate` is set, else `None`.
pub fn add_remote_vault(
    name: &str,
    host: &str,
    token: &str,
    activate: bool,
) -> Result<Option<ConnectionConfig>, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("enter the remote host (host:port)".to_string());
    }
    let conn = ConnectionConfig::Remote {
        host: host.to_string(),
        token: token.to_string(),
    };
    let mut reg = Registry::load();
    reg.add_vault(name, conn.clone())?;
    if activate {
        reg.set_default(name)?;
    }
    reg.save()?;
    Ok(activate.then_some(conn))
}

/// Remove a vault from the registry (does **not** delete any files on disk). Given the current
/// `active` connection, returns `Some(new_default_connection)` to apply when the removed vault was
/// the active one (so the front-end reconnects to the new default), else `None`.
pub fn remove_vault(
    name: &str,
    active: &ConnectionConfig,
) -> Result<Option<ConnectionConfig>, String> {
    let mut reg = Registry::load();
    let was_active = reg
        .vaults
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.connection == *active)
        .unwrap_or(false);
    reg.remove_vault(name)?;
    reg.save()?;
    Ok(was_active.then(|| reg.default_connection()))
}

/// Rename a vault in the registry (manager-only). Names are how the registry references a vault, so
/// this just relabels; the live connection (tracked by target, not name) is unaffected.
pub fn rename_vault(name: &str, new_name: &str) -> Result<(), String> {
    let mut reg = Registry::load();
    reg.rename_vault(name, new_name)?;
    reg.save()
}

/// Mark a vault as the **launch default** without switching to it (manager-only). Distinct from
/// [`switch_vault`], which also returns a connection to reconnect — this only changes which vault
/// opens on next start.
pub fn set_default_vault(name: &str) -> Result<(), String> {
    let mut reg = Registry::load();
    reg.set_default(name)?;
    reg.save()
}

/// Edit a **local** vault's folder (manager-only), e.g. after moving it on disk. Given the current
/// `active` connection, returns `Some(connection)` to apply when it's the active vault, else `None`.
pub fn edit_local_vault(
    name: &str,
    path: &str,
    active: &ConnectionConfig,
) -> Result<Option<ConnectionConfig>, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("choose a folder for the vault".to_string());
    }
    let real = mkb_protocol::paths::expand_user(path);
    std::fs::create_dir_all(&real).map_err(|e| format!("creating {}: {e}", real.display()))?;
    let conn = ConnectionConfig::Local {
        vault: PathBuf::from(path),
    };
    edit_connection(name, conn, active)
}

/// Edit a **remote** vault's host/token (manager-only). A blank token keeps the stored one, so a
/// user can fix the host without re-entering the secret. Given the current `active` connection,
/// returns `Some(connection)` to apply when it's the active vault, else `None`.
pub fn edit_remote_vault(
    name: &str,
    host: &str,
    token: &str,
    active: &ConnectionConfig,
) -> Result<Option<ConnectionConfig>, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("enter the remote host (host:port)".to_string());
    }
    // A blank token means "keep the existing one" — look it up rather than clobbering the secret.
    let token = if token.is_empty() {
        Registry::load()
            .vaults
            .into_iter()
            .find(|e| e.name == name)
            .and_then(|e| match e.connection {
                ConnectionConfig::Remote { token, .. } => Some(token),
                _ => None,
            })
            .unwrap_or_default()
    } else {
        token.to_string()
    };
    let conn = ConnectionConfig::Remote {
        host: host.to_string(),
        token,
    };
    edit_connection(name, conn, active)
}

/// Shared registry half of the two edit operations: persist the new connection for `name`, and
/// return `Some(conn)` to apply when that entry is the active one.
fn edit_connection(
    name: &str,
    conn: ConnectionConfig,
    active: &ConnectionConfig,
) -> Result<Option<ConnectionConfig>, String> {
    let mut reg = Registry::load();
    let was_active = reg
        .vaults
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.connection == *active)
        .unwrap_or(false);
    reg.update_connection(name, conn.clone())?;
    reg.save()?;
    Ok(was_active.then_some(conn))
}

/// Resolve a **local** vault's folder from the registry, for a front-end that reveals it in the OS
/// file manager. Errors for a remote vault (nothing local to reveal).
pub fn local_vault_path(name: &str) -> Result<PathBuf, String> {
    let entry = Registry::load()
        .vaults
        .into_iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("no vault named {name:?}"))?;
    match entry.connection {
        ConnectionConfig::Local { vault } => Ok(mkb_protocol::paths::expand_user(&vault)),
        ConnectionConfig::Remote { .. } => {
            Err("this is a remote vault — nothing local to reveal".to_string())
        }
    }
}
