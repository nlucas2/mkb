//! mkb web front-end.
//!
//! A **thin client**, exactly like the desktop shell: it serves the *same* `ui/` and forwards each
//! UI command to the daemon through [`mkb_app_core`], rendering through the shared `mkb-view` layer.
//! No knowledge-base behavior lives here — this is transport (HTTP/SSE) + connection glue only, the
//! browser-side twin of `app/mkb-tauri` (see `AGENTS.md`: one shared core, thin clients).
//!
//! Run it on your own machine and open `http://127.0.0.1:<port>` — "the app, in a browser tab."
//!
//! ## Per-tab vaults
//!
//! Unlike the desktop app (one window → one vault), a browser can have many tabs each viewing a
//! *different* vault. So the active vault is **per request**, not global server state: every request
//! carries an `X-MKB-Vault: <name>` header (absent → the registry default); the server keeps one
//! [`VaultSession`] per named vault (a cheap reconnectable client + a change broadcast) and routes to
//! it. There is no global "active vault," so one tab switching never disturbs another.
//!
//! ## Daemon lifetime = live sessions, not the server process
//!
//! A vault's daemon is kept warm only while a tab is *watching* it. Each open SSE stream is a live
//! session: while a vault has ≥1 subscriber, a per-vault thread renews the daemon's interactive lease
//! (and long-polls for changes); when the last tab disconnects, that thread stops, the lease lapses,
//! and the (auto-started, local) daemon idle-reaps as normal. Plain request/response calls hold no
//! lease — like the CLI, they just momentarily keep the daemon busy. A remote/cluster daemon never
//! reaps regardless, so leases only matter for local auto-started daemons.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use mkb_protocol::{connect, Client, ConnectionConfig, DaemonPaths, Registry};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

/// The header a browser tab sends to name its active vault (matches the platform shim).
const VAULT_HEADER: &str = "x-mkb-vault";
/// Interactive-lease heartbeat cadence; ~3× → the lease TTL, so a couple of missed beats are tolerated.
const HEARTBEAT_SECS: u64 = 10;
const LEASE_TTL_MS: u64 = 30_000;
/// Long-poll window the change-watcher requests; the daemon clamps it under its read timeout.
const WAIT_CHANGE_MS: u64 = 60_000;

/// One vault's server-side session: a reconnectable daemon client, the info to rebuild it, the
/// vault root (for image assets), a change broadcast fanned out to that vault's SSE tabs, and the
/// bookkeeping that ties the keep-alive lease to live subscribers.
struct VaultSession {
    client: Arc<Client>,
    cfg: ConnectionConfig,
    /// The vault's folder for resolving relative image assets; `None` for a remote connection.
    root: Option<PathBuf>,
    /// Fan-out of the daemon's content generation to every SSE stream on this vault.
    changes: broadcast::Sender<u64>,
    /// Last generation broadcast, so the watcher seeds its baseline to the current value and only
    /// publishes real advances. `0` = not yet known.
    last_gen: u64,
    /// Open SSE streams on this vault; the keep-alive watcher runs iff this is > 0.
    subscribers: usize,
    /// Whether a watcher/heartbeat thread is currently running for this vault.
    watcher_running: bool,
}

/// Shared server state: the vault sessions (keyed by registry name) plus process-wide bits. There is
/// deliberately **no** "active vault" here — that is a per-request/per-tab concept.
struct AppState {
    mkbd: Option<PathBuf>,
    /// Optional dev override: when `Some`, the UI is served from this directory on disk (live-edit
    /// without recompiling); when `None`, the UI compiled into the binary is served.
    ui_dir: Option<PathBuf>,
    vaults: Mutex<HashMap<String, VaultSession>>,
}

impl AppState {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, VaultSession>> {
        self.vaults.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Resolve the vault a request targets (from the `X-MKB-Vault` header, else the registry
    /// default) to a stable session key, creating the session if it doesn't exist yet. The key is
    /// the vault's registry name, so a header naming the default vault and a header-less request for
    /// it share one session.
    fn ensure_session(&self, header: Option<&str>) -> Result<String, String> {
        let (key, cfg) = resolve_target(header)?;
        let mut map = self.map();
        map.entry(key.clone()).or_insert_with(|| {
            let (changes, _) = broadcast::channel(64);
            VaultSession {
                client: Arc::new(descriptor(&cfg)),
                root: vault_root(&cfg),
                cfg,
                changes,
                last_gen: 0,
                subscribers: 0,
                watcher_running: false,
            }
        });
        Ok(key)
    }

    /// A live client for `key`, transparently reconnecting if the connection is dead (a local daemon
    /// may self-reap when idle, or crash). Ping first; if unreachable, rebuild from the stored config
    /// — which for a local vault respawns a detached daemon — and swap it into the session.
    fn live_client(&self, key: &str) -> Arc<Client> {
        let (current, cfg) = {
            let map = self.map();
            let Some(s) = map.get(key) else {
                return Arc::new(Client::new(DaemonPaths::for_default_vault().socket));
            };
            (s.client.clone(), s.cfg.clone())
        };
        if current.ping() {
            return current;
        }
        match connect(&cfg, self.mkbd.as_deref()) {
            Ok(fresh) => {
                let fresh = Arc::new(fresh.as_app());
                if let Some(s) = self.map().get_mut(key) {
                    s.client = fresh.clone();
                }
                fresh
            }
            Err(_) => current,
        }
    }

    fn root(&self, key: &str) -> Option<PathBuf> {
        self.map().get(key).and_then(|s| s.root.clone())
    }

    fn cfg(&self, key: &str) -> Option<ConnectionConfig> {
        self.map().get(key).map(|s| s.cfg.clone())
    }
}

/// Resolve a target vault to `(session key, connection)`. A named vault must exist in the registry;
/// no name → the registry default (falling back to the built-in default vault for an empty registry).
fn resolve_target(header: Option<&str>) -> Result<(String, ConnectionConfig), String> {
    let reg = Registry::load();
    match header.filter(|h| !h.trim().is_empty()) {
        Some(name) => reg
            .vaults
            .iter()
            .find(|e| e.name == name)
            .map(|e| (name.to_string(), e.connection.clone()))
            .ok_or_else(|| format!("unknown vault {name:?}")),
        None => match reg.default.clone() {
            Some(name) => {
                let conn = reg
                    .vaults
                    .iter()
                    .find(|e| e.name == name)
                    .map(|e| e.connection.clone())
                    .unwrap_or_default();
                Ok((name, conn))
            }
            None => Ok(("\u{0}default".to_string(), reg.default_connection())),
        },
    }
}

/// Build a client *descriptor* (no connection attempt) for a config — the session's initial client.
fn descriptor(cfg: &ConnectionConfig) -> Client {
    match cfg {
        ConnectionConfig::Remote { host, token } => {
            Client::tcp(host.clone(), token.clone()).as_app()
        }
        ConnectionConfig::Local { vault } => {
            Client::new(DaemonPaths::from_vault(vault).socket).as_app()
        }
    }
}

/// The vault's folder for resolving relative image assets; `None` for a remote connection.
fn vault_root(cfg: &ConnectionConfig) -> Option<PathBuf> {
    match cfg {
        ConnectionConfig::Local { vault } => Some(vault.clone()),
        ConnectionConfig::Remote { .. } => None,
    }
}

// ---------- argument + result helpers ----------

/// Extract a named argument from the JSON request body, deserialized to `T`. A missing key is
/// treated as JSON `null` (so `Option<_>` fields deserialize to `None`). Keys are the **camelCase**
/// names the JS `invoke(cmd, {...})` calls send, which the platform shim forwards verbatim.
fn arg<T: serde::de::DeserializeOwned>(body: &Value, key: &str) -> Result<T, String> {
    let v = body.get(key).cloned().unwrap_or(Value::Null);
    serde_json::from_value(v).map_err(|e| format!("invalid argument '{key}': {e}"))
}

/// Serialize a command result to JSON, matching how Tauri hands a command's return value to JS.
fn ok<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

// ---------- command dispatch ----------

/// Handle one `invoke(cmd, args)` call by forwarding to the same [`mkb_app_core`] operation the
/// desktop shell's Tauri command would, against the session `key`'s vault. Runs **synchronously**
/// (the daemon client is blocking); the axum handler wraps it in `spawn_blocking`.
///
/// Kept as one match rather than 40 routes so the mapping mirrors the app's single
/// `invoke_handler!` list and stays trivially auditable against it.
fn dispatch(state: &AppState, key: &str, cmd: &str, body: &Value) -> Result<Value, String> {
    use mkb_app_core as core;
    match cmd {
        // ----- reads -----
        "list_blocks" => ok(core::list_blocks(&state.live_client(key))?),
        "block_index" => ok(core::block_index(&state.live_client(key))?),
        "render_block" => {
            let id: String = arg(body, "id")?;
            let root = state.root(key);
            ok(core::render_block(
                &state.live_client(key),
                root.as_deref(),
                &id,
            )?)
        }
        "block_source" => ok(core::block_source(
            &state.live_client(key),
            &arg::<String>(body, "id")?,
        )?),
        "block_title_of" => ok(core::block_title_of(
            &state.live_client(key),
            &arg::<String>(body, "id")?,
        )?),
        "block_version" => ok(core::block_version(
            &state.live_client(key),
            &arg::<String>(body, "id")?,
        )?),
        "block_locked" => ok(core::block_locked(
            &state.live_client(key),
            &arg::<String>(body, "id")?,
        )?),
        "graph" => ok(core::graph(&state.live_client(key))?),
        "graph_svg" => {
            let scene: mkb_core::GraphScene = arg(body, "scene")?;
            ok(core::graph_svg(&scene)?)
        }
        "group_blocks" => ok(core::group_blocks(
            &state.live_client(key),
            &arg::<String>(body, "axis")?,
        )?),
        "hierarchy" => ok(core::hierarchy(&state.live_client(key))?),
        "backlinks" => ok(core::backlinks(
            &state.live_client(key),
            &arg::<String>(body, "id")?,
        )?),
        "search" => ok(core::search(
            &state.live_client(key),
            &arg::<String>(body, "query")?,
        )?),
        "list_tags" => ok(core::list_tags(&state.live_client(key))?),
        "orphan_assets" => ok(core::orphan_assets(&state.live_client(key))?),

        // ----- writes -----
        "save_block" => {
            let id: String = arg(body, "id")?;
            let title: Option<String> = arg(body, "title")?;
            let bodytext: String = arg(body, "body")?;
            let base: Option<String> = arg(body, "baseVersion")?;
            ok(core::save_block(
                &state.live_client(key),
                &id,
                title.as_deref(),
                &bodytext,
                base.as_deref(),
            )?)
        }
        "create_block" => {
            let title: Option<String> = arg(body, "title")?;
            let bodytext: String = arg(body, "body")?;
            ok(core::create_block(
                &state.live_client(key),
                title.as_deref(),
                &bodytext,
            )?)
        }
        "delete_block" => {
            core::delete_block(&state.live_client(key), &arg::<String>(body, "id")?)?;
            Ok(Value::Null)
        }
        "carve_selection" => {
            let parent: String = arg(body, "parentId")?;
            let start: usize = arg(body, "start")?;
            let end: usize = arg(body, "end")?;
            ok(core::carve_selection(
                &state.live_client(key),
                &parent,
                start,
                end,
            )?)
        }
        "add_asset" => {
            let name: String = arg(body, "name")?;
            let data: Vec<u8> = arg(body, "data")?;
            ok(core::add_asset(&state.live_client(key), &name, &data)?)
        }
        "remove_asset" => {
            core::remove_asset(&state.live_client(key), &arg::<String>(body, "path")?)?;
            Ok(Value::Null)
        }
        "link_blocks" => {
            let source: String = arg(body, "sourceId")?;
            let target: String = arg(body, "targetId")?;
            let embed: bool = arg(body, "embed")?;
            ok(core::link_blocks(
                &state.live_client(key),
                &source,
                &target,
                embed,
            )?)
        }
        "link_block_at" => {
            let source: String = arg(body, "sourceId")?;
            let target: String = arg(body, "targetId")?;
            let embed: bool = arg(body, "embed")?;
            let anchor: String = arg(body, "anchorId")?;
            let after: bool = arg(body, "after")?;
            ok(core::link_block_at(
                &state.live_client(key),
                &source,
                &target,
                embed,
                &anchor,
                after,
            )?)
        }
        "link_block_at_offset" => {
            let source: String = arg(body, "sourceId")?;
            let target: String = arg(body, "targetId")?;
            let embed: bool = arg(body, "embed")?;
            let offset: usize = arg(body, "offset")?;
            ok(core::link_block_at_offset(
                &state.live_client(key),
                &source,
                &target,
                embed,
                offset,
            )?)
        }
        "unlink_blocks" => {
            let source: String = arg(body, "sourceId")?;
            let target: String = arg(body, "targetId")?;
            core::unlink_blocks(&state.live_client(key), &source, &target)?;
            Ok(Value::Null)
        }
        "set_tags" => {
            let id: String = arg(body, "id")?;
            let tags: Vec<String> = arg(body, "tags")?;
            core::set_tags(&state.live_client(key), &id, tags)?;
            Ok(Value::Null)
        }
        "set_props" => {
            let id: String = arg(body, "id")?;
            let props: Vec<(String, String)> = arg(body, "props")?;
            core::set_props(&state.live_client(key), &id, props)?;
            Ok(Value::Null)
        }
        "unset_props" => {
            let id: String = arg(body, "id")?;
            let keys: Vec<String> = arg(body, "keys")?;
            core::unset_props(&state.live_client(key), &id, keys)?;
            Ok(Value::Null)
        }
        "set_lock" => {
            let id: String = arg(body, "id")?;
            let locked: bool = arg(body, "locked")?;
            core::set_lock(&state.live_client(key), &id, locked)?;
            Ok(Value::Null)
        }

        // ----- vault registry / connection (registry ops are global; "active" = this request's vault) -----
        "get_settings" => ok(core::current_config()),
        "save_settings" => {
            let cfg: ConnectionConfig = arg(body, "config")?;
            cfg.save()?;
            Ok(Value::Null)
        }
        "list_vaults" => {
            let active = state.cfg(key).unwrap_or_default();
            ok(core::list_vaults(&active))
        }
        "discover_vaults" => {
            let active = state.cfg(key).unwrap_or_default();
            ok(core::discover_vaults(&active))
        }
        "connection_status" => {
            let active = state.cfg(key).unwrap_or_default();
            ok(core::connection_status(&state.live_client(key), &active))
        }
        // The browser shim handles switching client-side (per-tab, no server state); if it ever
        // reaches here, just validate the target exists.
        "switch_vault" => {
            core::switch_vault(&arg::<String>(body, "name")?)?;
            Ok(Value::Null)
        }
        "add_vault" => {
            let name: String = arg(body, "name")?;
            let path: String = arg(body, "path")?;
            let activate: bool = arg(body, "activate")?;
            core::add_local_vault(&name, &path, activate)?;
            Ok(Value::Null)
        }
        "add_remote_vault" => {
            let name: String = arg(body, "name")?;
            let host: String = arg(body, "host")?;
            let token: String = arg(body, "token")?;
            let activate: bool = arg(body, "activate")?;
            core::add_remote_vault(&name, &host, &token, activate)?;
            Ok(Value::Null)
        }
        "remove_vault" => {
            let active = state.cfg(key).unwrap_or_default();
            core::remove_vault(&arg::<String>(body, "name")?, &active)?;
            Ok(Value::Null)
        }
        "rename_vault" => {
            let name: String = arg(body, "name")?;
            let new_name: String = arg(body, "newName")?;
            core::rename_vault(&name, &new_name)?;
            Ok(Value::Null)
        }
        "set_default_vault" => {
            core::set_default_vault(&arg::<String>(body, "name")?)?;
            Ok(Value::Null)
        }
        "edit_local_vault" => {
            let name: String = arg(body, "name")?;
            let path: String = arg(body, "path")?;
            let active = state.cfg(key).unwrap_or_default();
            core::edit_local_vault(&name, &path, &active)?;
            Ok(Value::Null)
        }
        "edit_remote_vault" => {
            let name: String = arg(body, "name")?;
            let host: String = arg(body, "host")?;
            let token: String = arg(body, "token")?;
            let active = state.cfg(key).unwrap_or_default();
            core::edit_remote_vault(&name, &host, &token, &active)?;
            Ok(Value::Null)
        }

        // ----- native affordances (server-side where they make sense) -----
        "reveal_vault" => {
            let path = core::local_vault_path(&arg::<String>(body, "name")?)?;
            open_in_file_manager(&path)?;
            Ok(Value::Null)
        }
        "restart_daemon" => {
            restart_daemon(state, key);
            Ok(Value::Null)
        }
        // A browser can't open the OS folder picker; the shim intercepts this with a path prompt().
        // If it still reaches here, report "cancelled" (null).
        "pick_vault" => Ok(Value::Null),
        // The command-line tools are bundled by the desktop *app*; nothing for the web to link.
        "cli_tools_status" => Ok(Value::String("unavailable".to_string())),
        "cli_tools_check" => Ok(Value::String(r#"{"available":false}"#.to_string())),
        "install_cli_tools" => {
            Err("The command-line tools are installed by the desktop app, not the web UI.".into())
        }

        other => Err(format!("unknown command: {other}")),
    }
}

/// Best-effort restart of a vault's local daemon: ask it to shut down, then reconnect (respawning a
/// fresh detached daemon). Mirrors the desktop app's `restart_daemon`.
fn restart_daemon(state: &AppState, key: &str) {
    let current = state.live_client(key);
    let _ = current.shutdown();
    std::thread::sleep(Duration::from_millis(300));
    let cfg = state.cfg(key).unwrap_or_default();
    if let Ok(fresh) = connect(&cfg, state.mkbd.as_deref()) {
        if let Some(s) = state.map().get_mut(key) {
            s.client = Arc::new(fresh.as_app());
        }
    }
}

/// Open a folder in the serving machine's file manager (works when the path exists locally).
fn open_in_file_manager(path: &Path) -> Result<(), String> {
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

// ---------- HTTP handlers ----------

/// `POST /__shutdown` — stop this server. **Loopback-only**: the peer address must be the local
/// machine (127.0.0.0/8 or ::1), so a client reaching the server over the network — e.g. a phone,
/// when it's bound to `0.0.0.0` for LAN access — can never shut it down; only something on the same
/// host (the desktop app's Stop button, or a local `curl`) can. Replies, then exits the process a
/// beat later so the response is flushed first.
async fn shutdown(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<SocketAddr>,
) -> Response {
    if !peer.ip().is_loopback() {
        return (
            StatusCode::FORBIDDEN,
            "shutdown is allowed from localhost only",
        )
            .into_response();
    }
    tokio::spawn(async {
        // Give axum a moment to write the 200 back before the process dies.
        tokio::time::sleep(Duration::from_millis(150)).await;
        std::process::exit(0);
    });
    (StatusCode::OK, "shutting down").into_response()
}

/// Read the target vault name a request declares via the `X-MKB-Vault` header (absent → default).
fn vault_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get(VAULT_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// `POST /api/{cmd}` — one invoke, against the vault named by `X-MKB-Vault` (else default). Runs the
/// blocking dispatch on a blocking task. Success → 200 + JSON; error → 400 + JSON error string
/// (mirroring how Tauri's `invoke` rejects with the `Err` value).
async fn api(
    State(state): State<Arc<AppState>>,
    UrlPath(cmd): UrlPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let key = match state.ensure_session(vault_header(&headers).as_deref()) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(Value::String(e))).into_response(),
    };
    let result = tokio::task::spawn_blocking(move || dispatch(&state, &key, &cmd, &body)).await;
    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)).into_response(),
        Ok(Err(msg)) => (StatusCode::BAD_REQUEST, Json(Value::String(msg))).into_response(),
        Err(join) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(Value::String(format!("task failed: {join}"))),
        )
            .into_response(),
    }
}

/// RAII guard: while an SSE stream is alive it counts as one live subscriber on its vault; on drop
/// (client disconnect) it decrements the count, so the keep-alive lease can lapse once the last tab
/// on a vault goes away.
struct SubGuard {
    state: Arc<AppState>,
    key: String,
}
impl Drop for SubGuard {
    fn drop(&mut self) {
        if let Some(s) = self.state.map().get_mut(&self.key) {
            s.subscribers = s.subscribers.saturating_sub(1);
        }
    }
}

/// `GET /sse?vault=<name>` — the live-refresh stream for one vault. Registers a subscriber (arming
/// the per-vault keep-alive watcher if it's the first), sends the current generation as a baseline
/// so a reconnecting tab can tell whether it missed a change, then streams each subsequent change.
/// The shim delivers these to the UI's existing `event.listen("vault-changed", …)` handler.
async fn sse(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let header = q.get("vault").cloned();
    let key = match state.ensure_session(header.as_deref()) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    // Register this subscriber and, if it's the first, start the watcher. Subscribe to the change
    // broadcast under the lock so no change is missed between here and the stream starting.
    let (rx, need_watcher) = {
        let mut map = state.map();
        let s = map.get_mut(&key).expect("session ensured");
        s.subscribers += 1;
        let need_watcher = !s.watcher_running;
        if need_watcher {
            s.watcher_running = true;
        }
        (s.changes.subscribe(), need_watcher)
    };
    if need_watcher {
        spawn_watcher(state.clone(), key.clone());
    }

    let guard = SubGuard {
        state: state.clone(),
        key,
    };

    // Stream real changes only. A tab that reconnects (e.g. a phone returning from the background)
    // re-syncs via the shim's on-reconnect refresh, so no server-sent baseline is needed — and the
    // watcher seeds its baseline to the *current* generation, so the current value is never
    // broadcast as if it were a change on connect.
    let live = BroadcastStream::new(rx).filter_map(
        |msg| -> Option<Result<Event, std::convert::Infallible>> {
            match msg {
                Ok(generation) => Some(Ok(Event::default()
                    .event("vault-changed")
                    .data(generation.to_string()))),
                // Lagged receiver: skip; the next real change re-syncs, and a reconnect refreshes anyway.
                Err(_) => None,
            }
        },
    );
    // Move the guard into the stream so it drops (→ decrement) when the client disconnects.
    let stream = live.map(move |ev| {
        let _hold = &guard;
        ev
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// `GET /assets?vault=<name>&path=<abs>` — serve a vault-relative image the renderer emitted as
/// `mkb-asset:<abs>`. Strictly scoped to that vault's directory (canonicalize + prefix-check) so it
/// can't read arbitrary files. 404 for a remote vault or any path outside the vault root.
async fn assets(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(requested) = q.get("path") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let key = match state.ensure_session(q.get("vault").map(|s| s.as_str())) {
        Ok(k) => k,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let Some(root) = state.root(&key) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (Ok(root), Ok(file)) = (root.canonicalize(), PathBuf::from(requested).canonicalize())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !file.starts_with(&root) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::task::spawn_blocking(move || std::fs::read(&file).map(|b| (b, file))).await {
        Ok(Ok((bytes, path))) => {
            ([(header::CONTENT_TYPE, content_type(&path))], bytes).into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The UI files, compiled into the binary at build time — exactly how the Tauri app embeds its
/// frontend (`frontendDist` + `generate_context!`) and the daemon embeds its model. So `mkb-web` is
/// self-contained: launch it from anywhere (a repo checkout, or inside the installed app) and it
/// serves the same UI with no files on disk to find. Built from the one shared `app/mkb-tauri/ui/`
/// tree, so the web and desktop UIs are the same source and cannot diverge.
const EMBEDDED_UI: &[(&str, &[u8])] = &[
    (
        "index.html",
        include_bytes!("../../../app/mkb-tauri/ui/index.html"),
    ),
    (
        "platform-shim.js",
        include_bytes!("../../../app/mkb-tauri/ui/platform-shim.js"),
    ),
    (
        "vendor/force-graph.min.js",
        include_bytes!("../../../app/mkb-tauri/ui/vendor/force-graph.min.js"),
    ),
    (
        "vendor/highlight.min.js",
        include_bytes!("../../../app/mkb-tauri/ui/vendor/highlight.min.js"),
    ),
    (
        "vendor/highlight-theme.css",
        include_bytes!("../../../app/mkb-tauri/ui/vendor/highlight-theme.css"),
    ),
    (
        "vendor/hljs-dockerfile.min.js",
        include_bytes!("../../../app/mkb-tauri/ui/vendor/hljs-dockerfile.min.js"),
    ),
    (
        "vendor/hljs-kql.min.js",
        include_bytes!("../../../app/mkb-tauri/ui/vendor/hljs-kql.min.js"),
    ),
    (
        "vendor/hljs-nginx.min.js",
        include_bytes!("../../../app/mkb-tauri/ui/vendor/hljs-nginx.min.js"),
    ),
    (
        "vendor/hljs-powershell.min.js",
        include_bytes!("../../../app/mkb-tauri/ui/vendor/hljs-powershell.min.js"),
    ),
    (
        "vendor/hljs-properties.min.js",
        include_bytes!("../../../app/mkb-tauri/ui/vendor/hljs-properties.min.js"),
    ),
    (
        "vendor/mermaid.min.js",
        include_bytes!("../../../app/mkb-tauri/ui/vendor/mermaid.min.js"),
    ),
];

/// Look up an embedded UI file by its request-relative path (e.g. `vendor/highlight.min.js`).
fn embedded_ui(rel: &str) -> Option<&'static [u8]> {
    EMBEDDED_UI.iter().find(|(p, _)| *p == rel).map(|(_, b)| *b)
}

/// Fallback: serve the static UI (`index.html`, `vendor/*`, the platform shim). By default this is
/// the copy compiled into the binary; with a `--ui-dir` dev override it's read from disk. Rejects
/// any path escaping the UI root; a miss is a real 404 (the UI is one page with explicit assets).
async fn static_files(State(state): State<Arc<AppState>>, uri: axum::http::Uri) -> Response {
    let rel = uri.path().trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.split('/').any(|seg| seg == ".." || seg == ".") {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Dev override: read from disk so UI edits show on reload without recompiling.
    if let Some(dir) = &state.ui_dir {
        let path = dir.join(rel);
        return match tokio::task::spawn_blocking(move || std::fs::read(&path).map(|b| (b, path)))
            .await
        {
            Ok(Ok((bytes, path))) => {
                ([(header::CONTENT_TYPE, content_type(&path))], bytes).into_response()
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        };
    }
    // Default: serve the compiled-in copy.
    match embedded_ui(rel) {
        Some(bytes) => (
            [(header::CONTENT_TYPE, content_type(Path::new(rel)))],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A minimal extension → MIME map for the file types the UI serves (HTML/JS/CSS + image assets).
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

// ---------- per-vault keep-alive watcher ----------

/// Keep one vault's daemon warm and its tabs live, for as long as it has ≥1 SSE subscriber. On a
/// dedicated OS thread (the client is blocking) it renews the interactive lease and long-polls
/// `wait_for_change`, publishing each new generation to the vault's broadcast (→ every SSE tab). When
/// the last subscriber leaves it exits, the lease lapses, and an auto-started local daemon idle-reaps.
///
/// The subscriber-count check and the `watcher_running` flag are both read/written under the map
/// mutex, and the subscribe path flips the flag under the same lock, so "watcher decides to exit" and
/// "subscriber decides to (re)start it" can't race.
fn spawn_watcher(state: Arc<AppState>, key: String) {
    std::thread::spawn(move || {
        let lease = format!("mkb-web-{}-{}", std::process::id(), key);

        // Seed the baseline to the *current* generation (via a heartbeat, which also returns it) so
        // the loop only ever broadcasts a *future* change — never the current value as if it were a
        // change the moment a tab connects (which would cause a spurious refresh on load).
        if let Ok(g) = state.live_client(&key).heartbeat(&lease, LEASE_TTL_MS) {
            if let Some(s) = state.map().get_mut(&key) {
                if s.last_gen < g {
                    s.last_gen = g;
                }
            }
        }

        loop {
            // Decide whether to keep running, and gather what the (unlocked) poll needs.
            let (client, sender, since) = {
                let mut map = state.map();
                let Some(s) = map.get_mut(&key) else { return };
                if s.subscribers == 0 {
                    s.watcher_running = false;
                    return;
                }
                (s.client.clone(), s.changes.clone(), s.last_gen)
            };

            // Renew the lease so an auto-started local daemon won't self-reap while a tab watches.
            let _ = client.heartbeat(&lease, LEASE_TTL_MS);

            match client.wait_for_change(since, WAIT_CHANGE_MS) {
                Ok(Some(generation)) => publish(&state, &key, &sender, generation),
                // Old daemon without the op: fall back to the heartbeat-generation poll.
                Ok(None) => {
                    if let Ok(generation) = client.heartbeat(&lease, LEASE_TTL_MS) {
                        publish(&state, &key, &sender, generation);
                    }
                    std::thread::sleep(Duration::from_secs(HEARTBEAT_SECS));
                }
                // Daemon restarting mid-wait: reconnect (respawns it) and re-baseline next loop.
                Err(_) => {
                    if let Some(cfg) = state.cfg(&key) {
                        if let Ok(fresh) = connect(&cfg, state.mkbd.as_deref()) {
                            if let Some(s) = state.map().get_mut(&key) {
                                s.client = Arc::new(fresh.as_app());
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    });
}

/// Record a new generation for `key` and broadcast it to that vault's SSE tabs, but only when it
/// actually changed (so idle re-arms don't spam a refresh).
fn publish(state: &AppState, key: &str, sender: &broadcast::Sender<u64>, generation: u64) {
    let changed = {
        let mut map = state.map();
        match map.get_mut(key) {
            Some(s) if s.last_gen != generation => {
                s.last_gen = generation;
                true
            }
            _ => false,
        }
    };
    if changed {
        let _ = sender.send(generation);
    }
}

// ---------- entry point ----------

#[derive(clap::Parser)]
#[command(
    name = "mkb-web",
    version,
    about = "Serve the mkb desktop UI in a browser — a thin web client over the mkb daemon"
)]
struct Args {
    /// Address to bind (default: 127.0.0.1:8787 — localhost only).
    #[arg(long, default_value = "127.0.0.1:8787")]
    bind: SocketAddr,
    /// Dev override: serve the UI from this directory on disk instead of the copy compiled into the
    /// binary, so edits to `app/mkb-tauri/ui/` show on reload without recompiling. Omit in normal use.
    #[arg(long, value_name = "DIR")]
    ui_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    let args = <Args as clap::Parser>::parse();

    // The UI is compiled into the binary (self-contained). `--ui-dir` is an optional dev override to
    // serve it live from disk; if given, sanity-check that it actually contains the UI.
    if let Some(dir) = &args.ui_dir {
        if !dir.join("index.html").is_file() {
            eprintln!("mkb-web: --ui-dir {} has no index.html", dir.display());
            std::process::exit(1);
        }
    }

    let state = Arc::new(AppState {
        mkbd: None,
        ui_dir: args.ui_dir,
        vaults: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/api/{cmd}", post(api))
        .route("/sse", get(sse))
        .route("/assets", get(assets))
        .route("/__shutdown", post(shutdown))
        .fallback(static_files)
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(args.bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("mkb-web: could not bind {}: {e}", args.bind);
            std::process::exit(1);
        }
    };
    println!("mkb-web: serving the app at http://{}", args.bind);
    // `into_make_service_with_connect_info` so the shutdown handler can see the peer address and
    // enforce loopback-only.
    let service = app.into_make_service_with_connect_info::<SocketAddr>();
    if let Err(e) = axum::serve(listener, service).await {
        eprintln!("mkb-web: server error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{arg, content_type};
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn arg_extracts_typed_values() {
        let body = json!({ "id": "abc", "embed": true, "offset": 7 });
        assert_eq!(arg::<String>(&body, "id").unwrap(), "abc");
        assert!(arg::<bool>(&body, "embed").unwrap());
        assert_eq!(arg::<usize>(&body, "offset").unwrap(), 7);
    }

    #[test]
    fn arg_missing_key_is_null_so_option_is_none() {
        let body = json!({ "id": "abc" });
        assert_eq!(arg::<Option<String>>(&body, "baseVersion").unwrap(), None);
        assert!(arg::<String>(&body, "body").is_err());
    }

    #[test]
    fn arg_wrong_type_errors() {
        let body = json!({ "start": "not-a-number" });
        assert!(arg::<usize>(&body, "start").is_err());
    }

    #[test]
    fn content_type_maps_known_extensions_and_defaults() {
        assert_eq!(
            content_type(Path::new("index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("vendor/x.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("a/b/c.css")),
            "text/css; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("pic.png")), "image/png");
        assert_eq!(content_type(Path::new("pic.jpeg")), "image/jpeg");
        assert_eq!(
            content_type(Path::new("mystery.xyz")),
            "application/octet-stream"
        );
        assert_eq!(content_type(Path::new("noext")), "application/octet-stream");
    }

    #[test]
    fn embedded_ui_carries_the_shell_and_shim() {
        // The self-contained binary must ship the entry page and the platform shim (which turns the
        // shared Tauri UI into a browser client). Non-empty guards against an emptied source file.
        assert!(super::embedded_ui("index.html").is_some_and(|b| !b.is_empty()));
        assert!(super::embedded_ui("platform-shim.js").is_some_and(|b| !b.is_empty()));
        assert!(super::embedded_ui("does-not-exist.js").is_none());
    }

    #[test]
    fn embedded_graph_experiments_have_expected_defaults() {
        let html = std::str::from_utf8(super::embedded_ui("index.html").unwrap()).unwrap();
        assert!(html.contains("id=\"vHoverEffects\""));
        assert!(html.contains("id=\"vUncollideLabels\""));
        assert!(html.contains("id=\"gExperimentalBody\" hidden"));
        assert!(html.contains("id=\"gLayoutBody\""));
        assert!(html.contains("id=\"gLayoutHeading\""));
        assert!(html.contains("\"mkb.graphLayout\", false"));
        assert!(html.contains(".glegend-col-body[hidden] { display:none; }"));
        assert!(html.contains(".gsection-toggle[hidden] { display:none; }"));
        assert!(html.contains("const mobileLayout = window.matchMedia(\"(max-width: 640px)\")"));
        assert!(html.contains("let graphHoverEffects = true;"));
        assert!(html.contains("let graphUncollideLabels = false;"));
        assert!(html.contains(".nodePointerAreaPaint("));
        assert!(html.contains("ctx.strokeText("));
        assert!(html.contains("graph.d3Force(\"label-spacing\", labelSpacing());"));
        assert!(html.contains("const maxLabelPad = 22 / scale;"));
        assert!(html.contains("const maxPush = 4 / scale * alpha;"));
        assert!(html.contains("const dvx = new Float64Array(ns.length)"));
        assert!(!html.contains("placeGraphLabels"));
        assert!(!html.contains("graph.d3Force(\"aspect\""));
    }

    #[test]
    fn embedded_graph_export_uses_native_png_and_shared_svg() {
        let html = std::str::from_utf8(super::embedded_ui("index.html").unwrap()).unwrap();
        assert!(html.contains("id=\"gExportPng\""));
        assert!(html.contains("id=\"gExportSvg\""));
        assert!(html.contains("id=\"gExportBody\" hidden"));
        assert!(html.contains("output.toBlob("));
        assert!(html.contains("invoke(\"graph_svg\""));
        assert!(html.contains("invoke(\"save_graph_export\""));
    }
}
