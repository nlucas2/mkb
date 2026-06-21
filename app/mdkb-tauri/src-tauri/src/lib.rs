//! mdkb desktop shell (Tauri).
//!
//! A **thin client**: every command fetches data from the daemon via `mdkb-protocol` and
//! renders through the shared `mdkb-view` layer — the same presentation path as the web UI,
//! so the two front-ends cannot diverge (see `AGENTS.md`). No knowledge-base behavior (block
//! parsing, transclusion, indexing, the link graph) lives here; that is all in `mdkb-core`
//! and reached over the wire. This file is connection management + command glue only.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use mdkb_core::{BlockId, GraphData, SearchQuery};
use mdkb_protocol::{connect, Client, ConnectionConfig, DaemonPaths};
use mdkb_view::{block_title, markdown_to_html, search_results_html, ResultRow};
use serde::Serialize;
use tauri::Manager;

/// Best-effort diagnostic line to stderr. A GUI build opts into the Windows "windows"
/// subsystem (no console), which leaves stderr as an invalid handle — and `eprintln!` would
/// **panic** on that failed write, taking the app down during startup. This swallows the
/// error so logging is never load-bearing.
macro_rules! log_line {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

/// How often the desktop app renews its interactive lease with the daemon. The window is open,
/// so the daemon must not self-reap; a heartbeat both renews the lease and counts as activity.
const HEARTBEAT_SECS: u64 = 10;
/// Lease time-to-live. Set to ~3× the heartbeat interval so a couple of missed beats are tolerated,
/// while a crashed/closed app still lets the lease expire promptly (it never pins the daemon open).
const LEASE_TTL_MS: u64 = 30_000;

/// Shared application state: the (reconnectable) connection to the daemon, plus what's needed
/// to transparently re-establish it. A local daemon may self-reap when idle (or crash), so the
/// app must be able to respawn it on the next interaction rather than going dead.
struct AppState {
    client: Mutex<Client>,
    /// The active connection config (local vault or remote), so we can reconnect.
    cfg: Mutex<ConnectionConfig>,
    /// Path to the bundled `mdkbd` (for local-mode auto-start), resolved once at startup.
    mdkbd: Option<PathBuf>,
}

impl AppState {
    /// A live client, transparently reconnecting if the current connection is dead.
    ///
    /// Local daemons self-reap after an idle period (and can crash); when that happens the next
    /// command would otherwise fail. We ping first and, if unreachable, re-resolve the client —
    /// which for a local vault respawns a detached daemon (`connect`/`ensure_daemon`) — so an
    /// idled-out daemon is invisible to the user beyond a brief cold-start on the next action.
    fn connected(&self) -> Result<MutexGuard<'_, Client>, String> {
        let mut guard = self.client.lock().map_err(|_| "state poisoned")?;
        if guard.ping() {
            return Ok(guard);
        }
        let cfg = self.cfg.lock().map_err(|_| "state poisoned")?.clone();
        match connect(&cfg, self.mdkbd.as_deref()) {
            Ok(fresh) => {
                log_line!("mdkb: reconnected ({})", fresh.endpoint());
                *guard = fresh;
            }
            Err(e) => log_line!("mdkb: reconnect failed: {e}"),
        }
        Ok(guard)
    }
}

/// A block prepared for the front-end: stable id, display title, raw Markdown (for editing),
/// and rendered HTML (children expanded, references as chips). HTML is produced by the shared
/// `mdkb-view` renderer so the desktop and web UIs cannot diverge.
#[derive(Serialize)]
struct BlockView {
    id: String,
    title: String,
    tags: Vec<String>,
    fm_tags: Vec<String>,
    content: String,
    html: String,
}

// ----- connection management -----

/// The active connection config: the saved file if present, else the local default vault.
fn current_config() -> ConnectionConfig {
    if ConnectionConfig::config_path().exists() {
        ConnectionConfig::load()
    } else {
        ConnectionConfig::default()
    }
}

/// Path to the `mdkbd` binary bundled inside the app (for local-mode auto-start), if present.
fn bundled_mdkbd(app: &tauri::AppHandle) -> Option<PathBuf> {
    let name = if cfg!(windows) { "mdkbd.exe" } else { "mdkbd" };
    let p = app.path().resource_dir().ok()?.join("bin").join(name);
    p.exists().then_some(p)
}

/// Resolve a [`Client`] for `cfg`. Local mode ensures a **detached** daemon is running
/// (auto-start that outlives the app); remote mode builds a TCP client. Falls back to the
/// default local socket on error so the window still opens (the UI shows the failure).
fn resolve_client(app: &tauri::AppHandle, cfg: &ConnectionConfig) -> Client {
    match connect(cfg, bundled_mdkbd(app).as_deref()) {
        Ok(client) => {
            log_line!("mdkb: connected ({})", client.endpoint());
            client
        }
        Err(e) => {
            log_line!("mdkb: {e}; falling back to the local socket");
            Client::new(DaemonPaths::for_default_vault().socket)
        }
    }
}

// ----- reads -----

/// Sidebar entries: root blocks as `{id, title}`.
#[derive(Serialize)]
struct NavBlock {
    id: String,
    title: String,
}

#[tauri::command]
fn list_blocks(state: tauri::State<'_, AppState>) -> Result<Vec<NavBlock>, String> {
    let client = state.connected()?;
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
#[tauri::command]
fn block_index(state: tauri::State<'_, AppState>) -> Result<Vec<NavBlock>, String> {
    let client = state.connected()?;
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

/// Render a block to HTML (children resolved by the daemon, Markdown→HTML by mdkb-view).
#[tauri::command]
fn render_block(state: tauri::State<'_, AppState>, id: String) -> Result<BlockView, String> {
    let client = state.connected()?;
    let bid = BlockId::parse(&id).map_err(|e| e.to_string())?;
    let rb = client
        .rendered_block(bid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("block not found: {id}"))?;
    Ok(BlockView {
        html: markdown_to_html(&rb.rendered),
        content: rb.raw,
        title: rb.title,
        tags: rb.tags,
        fm_tags: rb.fm_tags,
        id: rb.id.to_string(),
    })
}

/// Raw Markdown body of a block (for the editor).
#[tauri::command]
fn block_source(state: tauri::State<'_, AppState>, id: String) -> Result<String, String> {
    let client = state.connected()?;
    let bid = BlockId::parse(&id).map_err(|e| e.to_string())?;
    Ok(client
        .get_block_source(bid)
        .map_err(|e| e.to_string())?
        .unwrap_or_default())
}

/// The block's title (if any).
#[tauri::command]
fn block_title_of(state: tauri::State<'_, AppState>, id: String) -> Result<Option<String>, String> {
    let client = state.connected()?;
    let bid = BlockId::parse(&id).map_err(|e| e.to_string())?;
    Ok(client
        .get_block(bid)
        .map_err(|e| e.to_string())?
        .and_then(|b| b.title))
}

/// The whole block-level knowledge graph.
#[tauri::command]
fn graph(state: tauri::State<'_, AppState>) -> Result<GraphData, String> {
    let client = state.connected()?;
    client.graph().map_err(|e| e.to_string())
}

/// Backlinks (blocks that reference or transclude `id`), as nav blocks.
#[tauri::command]
fn backlinks(state: tauri::State<'_, AppState>, id: String) -> Result<Vec<NavBlock>, String> {
    let client = state.connected()?;
    let bid = BlockId::parse(&id).map_err(|e| e.to_string())?;
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
#[tauri::command]
fn search(state: tauri::State<'_, AppState>, query: String) -> Result<String, String> {
    let client = state.connected()?;
    let q = SearchQuery::parse(&query);
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
    Ok(search_results_html(&query, &rows))
}

// ----- writes -----

/// Update a block's title + body in place.
#[tauri::command]
fn save_block(
    state: tauri::State<'_, AppState>,
    id: String,
    title: Option<String>,
    body: String,
) -> Result<(), String> {
    let client = state.connected()?;
    let bid = BlockId::parse(&id).map_err(|e| e.to_string())?;
    client
        .update_block(bid, title.as_deref(), &body)
        .map_err(|e| e.to_string())
}

/// Create a new top-level block. Returns the new id.
#[tauri::command]
fn create_block(
    state: tauri::State<'_, AppState>,
    title: Option<String>,
    body: String,
) -> Result<String, String> {
    let client = state.connected()?;
    client
        .create_block(title.as_deref(), &body)
        .map(|id| id.to_string())
        .map_err(|e| e.to_string())
}

/// Carve the selected byte range of a parent's body into a new child (replace in place).
/// Returns the new child id.
#[tauri::command]
fn carve_selection(
    state: tauri::State<'_, AppState>,
    parent_id: String,
    start: usize,
    end: usize,
) -> Result<String, String> {
    let client = state.connected()?;
    let pid = BlockId::parse(&parent_id).map_err(|e| e.to_string())?;
    client
        .carve_selection(pid, start, end)
        .map(|id| id.to_string())
        .map_err(|e| e.to_string())
}

/// Delete a block.
#[tauri::command]
fn delete_block(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let client = state.connected()?;
    let bid = BlockId::parse(&id).map_err(|e| e.to_string())?;
    client.delete_block(bid).map_err(|e| e.to_string())
}

/// Link or embed one block into another. Returns the outcome string (may report a downgrade).
#[tauri::command]
fn link_blocks(
    state: tauri::State<'_, AppState>,
    source_id: String,
    target_id: String,
    embed: bool,
) -> Result<String, String> {
    let client = state.connected()?;
    let s = BlockId::parse(&source_id).map_err(|e| e.to_string())?;
    let t = BlockId::parse(&target_id).map_err(|e| e.to_string())?;
    let outcome = client.link(s, t, embed).map_err(|e| e.to_string())?;
    Ok(match outcome {
        mdkb_core::LinkOutcome::Reference => "reference".to_string(),
        mdkb_core::LinkOutcome::Transclusion => "transclusion".to_string(),
        mdkb_core::LinkOutcome::DowngradedToReference => "downgraded".to_string(),
    })
}

/// Set a block's managed (frontmatter) tags to exactly `tags` (replaces them; empty clears).
#[tauri::command]
fn set_tags(
    state: tauri::State<'_, AppState>,
    id: String,
    tags: Vec<String>,
) -> Result<(), String> {
    let client = state.connected()?;
    let bid = BlockId::parse(&id).map_err(|e| e.to_string())?;
    client.set_tags(bid, tags).map_err(|e| e.to_string())
}

/// All tags in the vault with per-tag block counts, for the tag browser.
#[tauri::command]
fn list_tags(state: tauri::State<'_, AppState>) -> Result<Vec<TagCountView>, String> {
    let client = state.connected()?;
    let tags = client.list_tags().map_err(|e| e.to_string())?;
    Ok(tags
        .into_iter()
        .map(|t| TagCountView {
            tag: t.tag,
            count: t.count,
        })
        .collect())
}

#[derive(Serialize)]
struct TagCountView {
    tag: String,
    count: usize,
}

// ----- settings / connection -----

/// The current connection config (for the Settings page).
#[tauri::command]
fn get_settings() -> ConnectionConfig {
    current_config()
}

/// Persist a new connection config and reconnect the client without restarting the app.
#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: ConnectionConfig,
) -> Result<(), String> {
    config.save()?;
    let client = resolve_client(&app, &config);
    let ok = client.ping();
    *state.client.lock().map_err(|_| "state poisoned")? = client;
    // Keep the stored config in sync so later auto-reconnects target the new vault/host.
    *state.cfg.lock().map_err(|_| "state poisoned")? = config;
    if ok {
        Ok(())
    } else {
        Err("saved, but the daemon is not reachable yet".to_string())
    }
}

/// Whether the current client can reach a daemon (for a connection indicator). Returns a
/// friendly `label` (the vault name for a local vault, or the host for a remote) plus the full
/// `endpoint` for a tooltip.
#[tauri::command]
fn connection_status(state: tauri::State<'_, AppState>) -> Result<ConnStatus, String> {
    let connected = state.client.lock().map_err(|_| "state poisoned")?.ping();
    let endpoint = state
        .client
        .lock()
        .map_err(|_| "state poisoned")?
        .endpoint();
    let cfg = state.cfg.lock().map_err(|_| "state poisoned")?.clone();
    let label = match &cfg {
        ConnectionConfig::Local { vault } => {
            let name = vault
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| vault.display().to_string());
            format!("{name} (local)")
        }
        ConnectionConfig::Remote { host, .. } => host.clone(),
    };
    Ok(ConnStatus {
        label,
        endpoint,
        connected,
    })
}

#[derive(Serialize)]
struct ConnStatus {
    label: String,
    endpoint: String,
    connected: bool,
}

/// Open a native folder picker and return the chosen path (for local-vault selection).
#[tauri::command]
fn pick_vault(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |chosen| {
        let path = chosen
            .and_then(|fp| fp.into_path().ok())
            .map(|p| p.display().to_string());
        let _ = tx.send(path);
    });
    rx.recv().map_err(|e| e.to_string())
}

/// Entry point used by the generated binary.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let cfg = current_config();
            let mdkbd = bundled_mdkbd(app.handle());
            let client = resolve_client(app.handle(), &cfg);
            app.manage(AppState {
                client: Mutex::new(client),
                cfg: Mutex::new(cfg),
                mdkbd,
            });

            // Hold the interactive lease for as long as the window is open. This both renews the
            // lease and registers activity, so the daemon won't self-reap while the app is up; if
            // the daemon was idle-reaped or crashed, `connected()` transparently respawns it. The
            // thread dies with the process on quit, after which the lease expires and the daemon
            // is free to wind down on its own — no explicit teardown needed.
            //
            // The lease id is per-process (the pid): stable for this app instance and distinct
            // from any other interactive client. Leases are keyed for liveness, not security, and
            // pids don't collide across concurrently running processes.
            let lease = format!("mdkb-app-{}", std::process::id());
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                if let Ok(client) = handle.state::<AppState>().connected() {
                    let _ = client.heartbeat(&lease, LEASE_TTL_MS);
                }
                std::thread::sleep(Duration::from_secs(HEARTBEAT_SECS));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_blocks,
            block_index,
            render_block,
            block_source,
            block_title_of,
            graph,
            backlinks,
            search,
            save_block,
            create_block,
            carve_selection,
            delete_block,
            link_blocks,
            set_tags,
            list_tags,
            get_settings,
            save_settings,
            connection_status,
            pick_vault,
        ])
        .run(tauri::generate_context!())
        .expect("error while running mdkb desktop shell");
}
