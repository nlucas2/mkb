//! mkb desktop shell (Tauri).
//!
//! A **thin client**: every command fetches data from the daemon via `mkb-protocol` and
//! renders through the shared `mkb-view` layer — the same shared presentation path,
//! so the two front-ends cannot diverge (see `AGENTS.md`). No knowledge-base behavior (block
//! parsing, transclusion, indexing, the link graph) lives here; that is all in `mkb-core`
//! and reached over the wire. This file is connection management + command glue only.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use mkb_core::{GraphData, GroupTree, HierTree};
use mkb_protocol::{connect, Client, ConnectionConfig, DaemonPaths};
use tauri::{Emitter, Manager};

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
/// Long-poll timeout the change-watcher requests; the daemon clamps it under its read timeout. A
/// minute keeps re-arms infrequent while still re-validating the connection regularly.
const WAIT_CHANGE_MS: u64 = 60_000;

/// Shared application state: the (reconnectable) connection to the daemon, plus what's needed
/// to transparently re-establish it. A local daemon may self-reap when idle (or crash), so the
/// app must be able to respawn it on the next interaction rather than going dead.
struct AppState {
    client: Mutex<Client>,
    /// The active connection config (local vault or remote), so we can reconnect.
    cfg: Mutex<ConnectionConfig>,
    /// Path to the bundled `mkbd` (for local-mode auto-start), resolved once at startup.
    mkbd: Option<PathBuf>,
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
        match connect(&cfg, self.mkbd.as_deref()) {
            Ok(fresh) => {
                log_line!("mkb: reconnected ({})", fresh.endpoint());
                *guard = fresh.as_app();
            }
            Err(e) => log_line!("mkb: reconnect failed: {e}"),
        }
        Ok(guard)
    }
}

// ----- connection management -----

/// Path to the `mkbd` binary bundled inside the app (for local-mode auto-start), if present.
fn bundled_mkbd(app: &tauri::AppHandle) -> Option<PathBuf> {
    let name = if cfg!(windows) { "mkbd.exe" } else { "mkbd" };
    let p = app.path().resource_dir().ok()?.join("bin").join(name);
    p.exists().then_some(p)
}

/// Whitelist a **local** vault directory in the WebView asset-protocol scope so rendered image
/// sources under it (`mkb-asset:` URLs from [`markdown_to_html_with_assets`], mapped to the asset
/// protocol by the front-end) are allowed to load. Recursive, so `assets/` and any nested folders
/// are covered. No-op for a remote vault (no local files to serve). Best-effort: a scope failure
/// just means images won't load, never that the app fails.
fn allow_vault_assets(app: &tauri::AppHandle, cfg: &ConnectionConfig) {
    if let ConnectionConfig::Local { vault } = cfg {
        if let Err(e) = app.asset_protocol_scope().allow_directory(vault, true) {
            log_line!(
                "mkb: could not allow vault assets ({}): {e}",
                vault.display()
            );
        }
    }
}

/// Resolve a [`Client`] for `cfg`. Local mode ensures a **detached** daemon is running
/// (auto-start that outlives the app); remote mode builds a TCP client. Falls back to the
/// default local socket on error so the window still opens (the UI shows the failure).
///
/// The returned client is marked [`Client::as_app`]: the desktop app is the human surface, so it
/// announces the app scope (lock management) on each request. Over a remote transport the daemon
/// ignores the announce, so lock/unlock is simply unavailable there.
/// Build a [`Client`] **descriptor** without contacting (or spawning) the daemon. Used at startup
/// so the window appears instantly: the actual connect/auto-start happens lazily on the first
/// `connected()` call, which runs on a background/worker thread rather than blocking the UI. A
/// cold daemon's initial reconcile no longer freezes the window — the UI shows an "Indexing…"
/// placeholder until the first data load returns.
fn client_descriptor(cfg: &ConnectionConfig) -> Client {
    match cfg {
        ConnectionConfig::Remote { host, token } => {
            Client::tcp(host.clone(), token.clone()).as_app()
        }
        ConnectionConfig::Local { vault } => {
            Client::new(DaemonPaths::from_vault(vault).socket).as_app()
        }
    }
}

/// Resolve a [`Client`] for `cfg`, **ensuring** a daemon is reachable (auto-starting a local one
/// or building a TCP client). Used where a connection should be established and validated right
/// away — e.g. the Settings page reconnect — rather than lazily. Startup uses
/// [`client_descriptor`] instead so the window never blocks on a cold daemon.
fn resolve_client(app: &tauri::AppHandle, cfg: &ConnectionConfig) -> Client {
    match connect(cfg, bundled_mkbd(app).as_deref()) {
        Ok(client) => {
            log_line!("mkb: connected ({})", client.endpoint());
            client.as_app()
        }
        Err(e) => {
            log_line!("mkb: {e}; falling back to the local socket");
            Client::new(DaemonPaths::for_default_vault().socket).as_app()
        }
    }
}

// ----- reads -----

#[tauri::command]
fn list_blocks(state: tauri::State<'_, AppState>) -> Result<Vec<mkb_app_core::NavBlock>, String> {
    mkb_app_core::list_blocks(&*state.connected()?)
}

#[tauri::command]
fn block_index(state: tauri::State<'_, AppState>) -> Result<Vec<mkb_app_core::NavBlock>, String> {
    mkb_app_core::block_index(&*state.connected()?)
}

#[tauri::command]
fn render_block(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<mkb_app_core::BlockView, String> {
    let client = state.connected()?;
    // A local vault resolves relative image sources against its folder (WebView asset protocol);
    // a remote vault has no local files to serve.
    let vault_root = match &*state.cfg.lock().map_err(|_| "state poisoned")? {
        ConnectionConfig::Local { vault } => Some(vault.clone()),
        ConnectionConfig::Remote { .. } => None,
    };
    mkb_app_core::render_block(&client, vault_root.as_deref(), &id)
}

#[tauri::command]
fn block_source(state: tauri::State<'_, AppState>, id: String) -> Result<String, String> {
    mkb_app_core::block_source(&*state.connected()?, &id)
}

#[tauri::command]
fn block_title_of(state: tauri::State<'_, AppState>, id: String) -> Result<Option<String>, String> {
    mkb_app_core::block_title_of(&*state.connected()?, &id)
}

#[tauri::command]
fn graph(state: tauri::State<'_, AppState>) -> Result<GraphData, String> {
    mkb_app_core::graph(&*state.connected()?)
}

#[tauri::command]
fn group_blocks(state: tauri::State<'_, AppState>, axis: String) -> Result<GroupTree, String> {
    mkb_app_core::group_blocks(&*state.connected()?, &axis)
}

#[tauri::command]
fn hierarchy(state: tauri::State<'_, AppState>) -> Result<HierTree, String> {
    mkb_app_core::hierarchy(&*state.connected()?)
}

#[tauri::command]
fn backlinks(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Vec<mkb_app_core::NavBlock>, String> {
    mkb_app_core::backlinks(&*state.connected()?, &id)
}

#[tauri::command]
fn search(state: tauri::State<'_, AppState>, query: String) -> Result<String, String> {
    mkb_app_core::search(&*state.connected()?, &query)
}

// ----- writes -----

#[tauri::command]
fn block_version(state: tauri::State<'_, AppState>, id: String) -> Result<Option<String>, String> {
    mkb_app_core::block_version(&*state.connected()?, &id)
}

#[tauri::command]
fn save_block(
    state: tauri::State<'_, AppState>,
    id: String,
    title: Option<String>,
    body: String,
    base_version: Option<String>,
) -> Result<mkb_app_core::SaveOutcome, String> {
    mkb_app_core::save_block(
        &*state.connected()?,
        &id,
        title.as_deref(),
        &body,
        base_version.as_deref(),
    )
}

#[tauri::command]
fn create_block(
    state: tauri::State<'_, AppState>,
    title: Option<String>,
    body: String,
) -> Result<String, String> {
    mkb_app_core::create_block(&*state.connected()?, title.as_deref(), &body)
}

#[tauri::command]
fn add_asset(
    state: tauri::State<'_, AppState>,
    name: String,
    data: Vec<u8>,
) -> Result<String, String> {
    mkb_app_core::add_asset(&*state.connected()?, &name, &data)
}

#[tauri::command]
fn orphan_assets(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    mkb_app_core::orphan_assets(&*state.connected()?)
}

#[tauri::command]
fn remove_asset(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    mkb_app_core::remove_asset(&*state.connected()?, &path)
}

#[tauri::command]
fn carve_selection(
    state: tauri::State<'_, AppState>,
    parent_id: String,
    start: usize,
    end: usize,
) -> Result<String, String> {
    mkb_app_core::carve_selection(&*state.connected()?, &parent_id, start, end)
}

#[tauri::command]
fn delete_block(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    mkb_app_core::delete_block(&*state.connected()?, &id)
}

#[tauri::command]
fn link_blocks(
    state: tauri::State<'_, AppState>,
    source_id: String,
    target_id: String,
    embed: bool,
) -> Result<String, String> {
    mkb_app_core::link_blocks(&*state.connected()?, &source_id, &target_id, embed)
}

#[tauri::command]
fn link_block_at(
    state: tauri::State<'_, AppState>,
    source_id: String,
    target_id: String,
    embed: bool,
    anchor_id: String,
    after: bool,
) -> Result<String, String> {
    mkb_app_core::link_block_at(
        &*state.connected()?,
        &source_id,
        &target_id,
        embed,
        &anchor_id,
        after,
    )
}

#[tauri::command]
fn link_block_at_offset(
    state: tauri::State<'_, AppState>,
    source_id: String,
    target_id: String,
    embed: bool,
    offset: usize,
) -> Result<String, String> {
    mkb_app_core::link_block_at_offset(&*state.connected()?, &source_id, &target_id, embed, offset)
}

#[tauri::command]
fn unlink_blocks(
    state: tauri::State<'_, AppState>,
    source_id: String,
    target_id: String,
) -> Result<(), String> {
    mkb_app_core::unlink_blocks(&*state.connected()?, &source_id, &target_id)
}

#[tauri::command]
fn set_tags(
    state: tauri::State<'_, AppState>,
    id: String,
    tags: Vec<String>,
) -> Result<(), String> {
    mkb_app_core::set_tags(&*state.connected()?, &id, tags)
}

#[tauri::command]
fn set_props(
    state: tauri::State<'_, AppState>,
    id: String,
    props: Vec<(String, String)>,
) -> Result<(), String> {
    mkb_app_core::set_props(&*state.connected()?, &id, props)
}

#[tauri::command]
fn unset_props(
    state: tauri::State<'_, AppState>,
    id: String,
    keys: Vec<String>,
) -> Result<(), String> {
    mkb_app_core::unset_props(&*state.connected()?, &id, keys)
}

#[tauri::command]
fn block_locked(state: tauri::State<'_, AppState>, id: String) -> Result<bool, String> {
    mkb_app_core::block_locked(&*state.connected()?, &id)
}

#[tauri::command]
fn set_lock(state: tauri::State<'_, AppState>, id: String, locked: bool) -> Result<(), String> {
    mkb_app_core::set_lock(&*state.connected()?, &id, locked)
}

#[tauri::command]
fn list_tags(state: tauri::State<'_, AppState>) -> Result<Vec<mkb_app_core::TagCountView>, String> {
    mkb_app_core::list_tags(&*state.connected()?)
}

// ----- settings / connection -----

/// The current connection config (for the Settings page).
#[tauri::command]
fn get_settings() -> ConnectionConfig {
    mkb_app_core::current_config()
}

/// Persist a new connection config and reconnect the client without restarting the app.
#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: ConnectionConfig,
) -> Result<(), String> {
    config.save()?;
    apply_connection(&app, &state, config)
}

/// Point the live client at `config` and reconnect, without restarting the app: resolve a client
/// (auto-starting a local daemon as needed), swap it into shared state, whitelist the vault's assets
/// for the WebView, and remember the config for later auto-reconnects. Shared by every command that
/// changes the active vault (save/switch/create/remove) so the reconnect path exists in exactly one
/// place. Returns `Ok` when the daemon is already reachable, else an `Err` the UI can surface (the
/// config is still applied — a local daemon just cold-starts on the next action).
fn apply_connection(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    config: ConnectionConfig,
) -> Result<(), String> {
    let client = resolve_client(app, &config);
    let ok = client.ping();
    *state.client.lock().map_err(|_| "state poisoned")? = client;
    // Allow image loading from the newly selected local vault before it becomes active.
    allow_vault_assets(app, &config);
    // Keep the stored config in sync so later auto-reconnects target the new vault/host.
    *state.cfg.lock().map_err(|_| "state poisoned")? = config;
    if ok {
        Ok(())
    } else {
        Err("saved, but the daemon is not reachable yet".to_string())
    }
}

/// The known vaults from the registry, marking the active + default ones.
#[tauri::command]
fn list_vaults(state: tauri::State<'_, AppState>) -> Result<Vec<mkb_app_core::VaultRow>, String> {
    let active = state.cfg.lock().map_err(|_| "state poisoned")?.clone();
    Ok(mkb_app_core::list_vaults(&active))
}

/// Rename a vault in the registry (manager-only).
#[tauri::command]
fn rename_vault(name: String, new_name: String) -> Result<(), String> {
    mkb_app_core::rename_vault(&name, &new_name)
}

/// Mark a vault as the launch default without switching to it (manager-only).
#[tauri::command]
fn set_default_vault(name: String) -> Result<(), String> {
    mkb_app_core::set_default_vault(&name)
}

/// Edit a local vault's folder (manager-only). Reconnects if it's the active vault.
#[tauri::command]
fn edit_local_vault(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
    path: String,
) -> Result<(), String> {
    let active = state.cfg.lock().map_err(|_| "state poisoned")?.clone();
    if let Some(conn) = mkb_app_core::edit_local_vault(&name, &path, &active)? {
        apply_connection(&app, &state, conn)?;
    }
    Ok(())
}

/// Edit a remote vault's host/token (manager-only). Reconnects if it's the active vault.
#[tauri::command]
fn edit_remote_vault(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
    host: String,
    token: String,
) -> Result<(), String> {
    let active = state.cfg.lock().map_err(|_| "state poisoned")?.clone();
    if let Some(conn) = mkb_app_core::edit_remote_vault(&name, &host, &token, &active)? {
        apply_connection(&app, &state, conn)?;
    }
    Ok(())
}

/// Reveal a local vault's folder in the OS file manager (manager-only). Native affordance: resolve
/// the path in core, then spawn the platform opener here. No-op-with-error for a remote vault.
#[tauri::command]
fn reveal_vault(name: String) -> Result<(), String> {
    let path = mkb_app_core::local_vault_path(&name)?;
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(cmd)
        .arg(path.as_os_str())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open the folder: {e}"))
}

/// Switch the active vault to an existing registry entry: make it the default and reconnect live.
#[tauri::command]
fn switch_vault(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let conn = mkb_app_core::switch_vault(&name)?;
    apply_connection(&app, &state, conn)
}

/// Add (or create) a local vault; when `activate`, switch to it live.
#[tauri::command]
fn add_vault(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
    path: String,
    activate: bool,
) -> Result<(), String> {
    if let Some(conn) = mkb_app_core::add_local_vault(&name, &path, activate)? {
        apply_connection(&app, &state, conn)?;
    }
    Ok(())
}

/// Remove a vault from the registry (files on disk are kept). Reconnects to the new default if the
/// removed vault was the active one.
#[tauri::command]
fn remove_vault(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let active = state.cfg.lock().map_err(|_| "state poisoned")?.clone();
    if let Some(conn) = mkb_app_core::remove_vault(&name, &active)? {
        apply_connection(&app, &state, conn)?;
    }
    Ok(())
}

/// Add a remote daemon as a named registry entry; when `activate`, connect to it live.
#[tauri::command]
fn add_remote_vault(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
    host: String,
    token: String,
    activate: bool,
) -> Result<(), String> {
    if let Some(conn) = mkb_app_core::add_remote_vault(&name, &host, &token, activate)? {
        apply_connection(&app, &state, conn)?;
    }
    Ok(())
}

/// Discover vaults with a running daemon that aren't in the registry yet.
#[tauri::command]
fn discover_vaults(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<mkb_app_core::DiscoveredRow>, String> {
    let active = state.cfg.lock().map_err(|_| "state poisoned")?.clone();
    Ok(mkb_app_core::discover_vaults(&active))
}

/// Whether the current client can reach a daemon (for the connection indicator), with a friendly
/// label + endpoint.
#[tauri::command]
fn connection_status(
    state: tauri::State<'_, AppState>,
) -> Result<mkb_app_core::ConnStatus, String> {
    let cfg = state.cfg.lock().map_err(|_| "state poisoned")?.clone();
    let client = state.client.lock().map_err(|_| "state poisoned")?;
    Ok(mkb_app_core::connection_status(&client, &cfg))
}

/// Restart the local daemon: ask it to shut down (remove its socket and exit), then reconnect —
/// which transparently auto-starts a fresh detached daemon. Useful after upgrading the app to
/// replace a still-running older daemon. Only meaningful for a local vault; on a remote
/// connection the shutdown is refused by the daemon and we surface that.
#[tauri::command]
fn restart_daemon(state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Best-effort shutdown of the current daemon (ignore "not reachable" — we'll respawn anyway).
    {
        let client = state.client.lock().map_err(|_| "state poisoned")?;
        if let Err(e) = client.shutdown() {
            log_line!("mkb: shutdown before restart returned: {e}");
        }
    }
    // Give the daemon a moment to release its socket and lock before respawning.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let cfg = state.cfg.lock().map_err(|_| "state poisoned")?.clone();
    let fresh = connect(&cfg, state.mkbd.as_deref())?;
    let mut guard = state.client.lock().map_err(|_| "state poisoned")?;
    *guard = fresh.as_app();
    log_line!("mkb: daemon restarted ({})", guard.endpoint());
    Ok(())
}

/// Open a native folder picker and return the chosen path (for local-vault selection).
///
/// This MUST be `async`: Tauri runs async commands off the main thread, so the native folder
/// dialog can be dispatched to the main-thread event loop while this worker thread blocks on the
/// result. A *synchronous* command runs on the main thread itself — blocking it to await the
/// dialog deadlocks, because the event loop can then never pump to show the dialog or deliver its
/// choice (the "browse hangs" bug). `blocking_pick_folder` is the documented pattern here.
#[tauri::command]
async fn pick_vault(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let chosen = app.dialog().file().blocking_pick_folder();
    Ok(chosen
        .and_then(|fp| fp.into_path().ok())
        .map(|p| p.display().to_string()))
}

/// Entry point used by the generated binary.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let cfg = mkb_app_core::current_config();
            let mkbd = bundled_mkbd(app.handle());
            // Build a client *descriptor* only — do NOT connect or spawn the daemon here, or a cold
            // daemon's initial reconcile would freeze the window before it appears. The daemon is
            // auto-started lazily by the first `connected()` call (heartbeat thread / UI command),
            // which runs off the main thread; the UI shows an "Indexing…" placeholder until then.
            let client = client_descriptor(&cfg);
            // Allow the WebView to load images stored in the local vault (asset protocol).
            allow_vault_assets(app.handle(), &cfg);
            app.manage(AppState {
                client: Mutex::new(client),
                cfg: Mutex::new(cfg),
                mkbd,
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
            let lease = format!("mkb-app-{}", std::process::id());
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                // Renew the interactive lease so the daemon won't self-reap while the window is up.
                // Change detection lives in the long-poll thread below; this one is lease-only.
                loop {
                    if let Ok(client) = handle.state::<AppState>().connected() {
                        let _ = client.heartbeat(&lease, LEASE_TTL_MS);
                    }
                    std::thread::sleep(Duration::from_secs(HEARTBEAT_SECS));
                }
            });

            // Live refresh via daemon push: long-poll wait_for_change parks until the vault's
            // generation moves, then we tell the WebView to refresh — sub-second, no tight poll.
            // The first reply sets a baseline; a daemon restart resets the counter, so the value
            // differs (compared with !=) and triggers a refresh. A transport error means the daemon
            // is restarting mid-wait: reconnect (which respawns it) and re-baseline. An old daemon
            // that doesn't support the op returns None, so we fall back to a 10s generation poll.
            let handle2 = app.handle().clone();
            let lease2 = format!("mkb-app-poll-{}", std::process::id());
            std::thread::spawn(move || {
                let mut last_gen: Option<u64> = None;
                loop {
                    // Clone the client and release the lock before parking: the long-poll blocks up
                    // to ~25s, and holding the AppState mutex that whole time would freeze every UI
                    // command. The Client is just a cheap transport descriptor (socket/addr).
                    let client = match handle2.state::<AppState>().connected() {
                        Ok(guard) => guard.clone(),
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(500));
                            continue;
                        }
                    };
                    match client.wait_for_change(last_gen.unwrap_or(0), WAIT_CHANGE_MS) {
                        Ok(Some(generation)) => {
                            if last_gen.is_some_and(|prev| prev != generation) {
                                let _ = handle2.emit("vault-changed", generation);
                            }
                            last_gen = Some(generation);
                        }
                        Ok(None) => {
                            // Old daemon: fall back to the heartbeat-generation poll.
                            if let Ok(generation) = client.heartbeat(&lease2, LEASE_TTL_MS) {
                                if last_gen.is_some_and(|prev| prev != generation) {
                                    let _ = handle2.emit("vault-changed", generation);
                                }
                                last_gen = Some(generation);
                            }
                            std::thread::sleep(Duration::from_secs(HEARTBEAT_SECS));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(500)), // daemon restarting: reconnect + re-baseline
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_blocks,
            block_index,
            render_block,
            block_source,
            block_title_of,
            block_version,
            graph,
            group_blocks,
            hierarchy,
            backlinks,
            search,
            save_block,
            create_block,
            add_asset,
            orphan_assets,
            remove_asset,
            carve_selection,
            delete_block,
            link_blocks,
            link_block_at,
            link_block_at_offset,
            unlink_blocks,
            set_tags,
            set_props,
            unset_props,
            set_lock,
            block_locked,
            list_tags,
            get_settings,
            save_settings,
            list_vaults,
            switch_vault,
            rename_vault,
            set_default_vault,
            edit_local_vault,
            edit_remote_vault,
            reveal_vault,
            add_vault,
            add_remote_vault,
            remove_vault,
            discover_vaults,
            connection_status,
            restart_daemon,
            pick_vault,
        ])
        .run(tauri::generate_context!())
        .expect("error while running mkb desktop shell");
}
