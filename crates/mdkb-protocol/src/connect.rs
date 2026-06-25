//! Client-side connection configuration and daemon lifecycle.
//!
//! A *client* (the desktop app, the MCP server) needs to know **where** the
//! daemon is and, for a local vault, be able to **start** one. This is distinct from the
//! vault's own `.mdkb/config.json` (which configures the daemon's embedder). Keeping the
//! resolution here means every client connects identically — no divergence (see `AGENTS.md`).
//!
//! Two modes:
//! - [`ConnectionConfig::Local`] — talk to (and, if needed, **spawn**) a daemon for a vault
//!   directory on this machine. The daemon is started **detached** so it outlives the client
//!   that launched it (the headless-daemon model: the daemon is the persistent tool, every UI
//!   is a disposable client).
//! - [`ConnectionConfig::Remote`] — connect to a daemon over TCP with a shared token.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{Client, DaemonPaths};

/// How long a **client‑auto‑started** daemon may sit with **no requests and no interactive lease**
/// before it self‑reaps. Only applied to daemons we spawn here; a daemon a user runs manually (or
/// the remote/k3s deployment) gets no `--idle-timeout` and runs forever.
///
/// This is now a short *grace* window, not a long warm‑hold: a long‑lived client (the desktop app
/// / desktop app) holds a heartbeat **lease** that keeps the daemon alive while it's open, so when that
/// client closes the daemon winds down within ~this grace instead of lingering for 15 minutes.
/// Momentary clients (CLI/MCP) keep the daemon warm only through this window — long enough for
/// fast back‑to‑back commands, short enough that walking away reclaims the process (and its
/// embedder RAM) promptly.
const AUTOSTART_IDLE_SECS: u64 = 120;

/// Where a client should connect, persisted in the client's config file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ConnectionConfig {
    /// A local vault directory; the client ensures a daemon is running for it.
    Local {
        /// Vault root (Markdown directory).
        vault: PathBuf,
    },
    /// A remote daemon over TCP, authenticated with a shared token.
    Remote {
        /// `host:port` of the remote `mdkbd --listen`.
        host: String,
        /// Shared token the remote daemon requires.
        #[serde(default)]
        token: String,
    },
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        ConnectionConfig::Local {
            vault: DaemonPaths::default_vault(),
        }
    }
}

impl ConnectionConfig {
    /// Path of the client's registry file (`vaults.json`). Kept as `config_path` for the callers
    /// that only need "does a saved client config exist?".
    pub fn config_path() -> PathBuf {
        Registry::path()
    }

    /// The active connection: the registry's default vault entry (see [`Registry::default_connection`]).
    /// Missing/invalid registry falls back to the built-in default vault.
    pub fn load() -> ConnectionConfig {
        Registry::load().default_connection()
    }

    /// Persist this as the registry's **default** vault entry (creating the registry if needed),
    /// so a client that "configures once" updates the default every other client falls back to.
    pub fn save(&self) -> Result<(), String> {
        let mut reg = Registry::load();
        reg.set_default_connection(self.clone());
        reg.save()
    }

    /// A short human description of this connection (for UI/logs).
    pub fn describe(&self) -> String {
        match self {
            ConnectionConfig::Local { vault } => format!("local:{}", vault.display()),
            ConnectionConfig::Remote { host, .. } => format!("remote:{host}"),
        }
    }
}

/// One named vault in the [`Registry`]. The connection is **nested** (not `#[serde(flatten)]`):
/// flattening an internally-tagged enum relies on a value buffer and interacts badly with
/// `deny_unknown_fields`, so a nested object is the robust schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultEntry {
    /// Stable, human-chosen name (also how `default` references this entry).
    pub name: String,
    /// Where this vault's daemon is (local directory or remote host).
    pub connection: ConnectionConfig,
}

/// The client's **vault registry** (`vaults.json`): the named vaults this user knows about and
/// which one is the default. Every client (CLI/web/MCP) falls back to the default when no vault is
/// given, so configuring it once works everywhere; the list is what a UI uses to offer multi-vault
/// switching. The file is portable across machines when entries use `~`-relative paths (see
/// [`crate::paths::expand_user`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// Known vaults, in display order.
    #[serde(default)]
    pub vaults: Vec<VaultEntry>,
    /// Name of the default entry (the fallback connection). `None` → the first entry, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl Registry {
    /// Path of the registry file: `<client-config-dir>/vaults.json`.
    pub fn path() -> PathBuf {
        client_config_dir().join("vaults.json")
    }

    /// A built-in registry with a single `default` local vault (the `$MDKB_VAULT`/`~/mdkb-vault`
    /// fallback). Used when no `vaults.json` exists yet, so there is always at least one vault and
    /// the default always resolves.
    pub fn builtin() -> Registry {
        Registry {
            vaults: vec![VaultEntry {
                name: "default".to_string(),
                connection: ConnectionConfig::default(),
            }],
            default: Some("default".to_string()),
        }
    }

    /// Load `vaults.json`: an absent file yields the built-in single-vault registry; a present but
    /// malformed file logs loudly and also falls back to the built-in (rather than silently doing
    /// the wrong thing).
    pub fn load() -> Registry {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!(
                    "mdkb: ignoring malformed {}: {e}; using the built-in default vault",
                    path.display()
                );
                Registry::builtin()
            }),
            Err(_) => Registry::builtin(),
        }
    }

    /// Persist the registry, creating the config directory if needed.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))
    }

    /// The default connection used as the fallback when a client specifies no vault:
    /// - `default` names an existing entry → that entry's connection.
    /// - `default` names a **missing** entry → the built-in default vault, with a warning (we do
    ///   **not** silently pick a different configured vault, which could be the wrong one).
    /// - `default` is `None` → the first entry if any, else the built-in default.
    pub fn default_connection(&self) -> ConnectionConfig {
        match &self.default {
            Some(name) => match self.vaults.iter().find(|e| &e.name == name) {
                Some(entry) => entry.connection.clone(),
                None => {
                    eprintln!(
                        "mdkb: default vault {name:?} is not in the registry; using the built-in \
                         default vault"
                    );
                    ConnectionConfig::default()
                }
            },
            None => self
                .vaults
                .first()
                .map(|e| e.connection.clone())
                .unwrap_or_default(),
        }
    }

    /// Set (or replace) the **default** entry's connection, keeping its name; if there is no
    /// default entry yet, append one named `default`. Used by a client that saves "the vault I'm
    /// using" so it becomes the shared fallback.
    pub fn set_default_connection(&mut self, connection: ConnectionConfig) {
        let name = self
            .default
            .clone()
            .unwrap_or_else(|| "default".to_string());
        match self.vaults.iter_mut().find(|e| e.name == name) {
            Some(entry) => entry.connection = connection,
            None => self.vaults.push(VaultEntry {
                name: name.clone(),
                connection,
            }),
        }
        self.default = Some(name);
    }
}

/// The OS-appropriate per-user config directory for mdkb's client config (the `vaults.json`
/// registry). All clients — CLI, web, MCP, and the desktop app — share this one directory, so the
/// segment is the product name `mdkb` on every platform (it is no longer the desktop app's bundle
/// id). Override with `$MDKB_CONFIG_DIR` (e.g. point it at a synced folder).
fn client_config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(crate::env::CONFIG_DIR) {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/mdkb");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("mdkb");
        }
    }
    // Linux / fallback: XDG.
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("mdkb");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config/mdkb");
    }
    PathBuf::from(".mdkb-config")
}

/// The connection a client was *asked* for, by explicit flags. Every field is optional; a `None`
/// field defers to the next layer (env, then the registry default, then the built-in vault). This
/// is the clap-free seam every client (CLI/web/MCP) fills, so the precedence logic lives in one
/// place ([`resolve_target`]) instead of being re-implemented per client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientInputs {
    /// Local vault directory to connect to (its daemon is auto-started). Supports a leading `~`.
    pub vault: Option<PathBuf>,
    /// `host:port` of a remote daemon to dial instead.
    pub remote: Option<String>,
    /// Token to present to a remote daemon.
    pub token: Option<String>,
    /// Explicit local socket path to dial instead of deriving one from the vault.
    pub socket: Option<PathBuf>,
}

/// The same four connection inputs as [`ClientInputs`], but sourced from the environment. Read via
/// [`EnvSnapshot::read`], which treats an **empty** variable as absent (so `MDKB_TOKEN=""` doesn't
/// masquerade as a real value and outrank the registry default).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvSnapshot {
    pub vault: Option<PathBuf>,
    pub remote: Option<String>,
    pub token: Option<String>,
    pub socket: Option<PathBuf>,
}

impl EnvSnapshot {
    /// Read the connection env vars, normalising empty values to `None`.
    pub fn read() -> EnvSnapshot {
        let s = |name: &str| -> Option<String> {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        EnvSnapshot {
            vault: s(crate::env::VAULT).map(PathBuf::from),
            remote: s(crate::env::REMOTE),
            token: s(crate::env::TOKEN),
            socket: s(crate::env::SOCKET).map(PathBuf::from),
        }
    }
}

/// A fully-resolved connection target — the *decision* of where to connect, with no I/O performed
/// yet. [`connect_resolved`] turns it into a live [`Client`]. Splitting the decision (pure,
/// testable) from the side effects (spawn/connect) keeps the precedence logic unit-testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTarget {
    /// Connect to (auto-starting) the daemon for this local vault.
    LocalVault { vault: PathBuf },
    /// Connect to this explicit local socket.
    LocalSocket { socket: PathBuf },
    /// Connect to a remote daemon over TCP with a token.
    Remote { host: String, token: String },
}

/// Resolve the connection target from explicit inputs, the environment, and the registry default,
/// applying the precedence **explicit flag > env > registry default > built-in vault**.
///
/// Within a layer the kinds are tried remote → socket → vault. A token may come from either the
/// inputs or the env (so e.g. only the token can be overridden on the command line); a remote
/// target with no token is an error (a remote daemon always requires one). This function performs
/// **no** I/O — it only decides — so it is exhaustively unit-testable.
pub fn resolve_target(
    inputs: &ClientInputs,
    env: &EnvSnapshot,
    registry_default: Option<&ConnectionConfig>,
) -> Result<ResolvedTarget, String> {
    let token = inputs.token.clone().or_else(|| env.token.clone());

    // 1. explicit inputs
    if let Some(remote) = nonempty(inputs.remote.as_deref()) {
        return remote_target(remote, token);
    }
    if let Some(socket) = inputs.socket.clone() {
        return Ok(ResolvedTarget::LocalSocket { socket });
    }
    if let Some(vault) = inputs.vault.clone() {
        return Ok(ResolvedTarget::LocalVault {
            vault: crate::paths::expand_user(vault),
        });
    }
    // 2. environment
    if let Some(remote) = nonempty(env.remote.as_deref()) {
        return remote_target(remote, token);
    }
    if let Some(socket) = env.socket.clone() {
        return Ok(ResolvedTarget::LocalSocket { socket });
    }
    if let Some(vault) = env.vault.clone() {
        return Ok(ResolvedTarget::LocalVault {
            vault: crate::paths::expand_user(vault),
        });
    }
    // 3. registry default
    if let Some(cfg) = registry_default {
        return match cfg {
            ConnectionConfig::Local { vault } => Ok(ResolvedTarget::LocalVault {
                vault: crate::paths::expand_user(vault.clone()),
            }),
            ConnectionConfig::Remote { host, token } => {
                remote_target(host.clone(), nonempty(Some(token)))
            }
        };
    }
    // 4. built-in default vault
    Ok(ResolvedTarget::LocalVault {
        vault: DaemonPaths::default_vault(),
    })
}

/// Build a remote target, enforcing that a non-empty token is present.
fn remote_target(host: String, token: Option<String>) -> Result<ResolvedTarget, String> {
    match token {
        Some(token) => Ok(ResolvedTarget::Remote { host, token }),
        None => Err(format!(
            "remote daemon {host} requires a token (set --token or ${})",
            crate::env::TOKEN
        )),
    }
}

/// Trim a candidate string, returning `None` if it is absent or empty.
fn nonempty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Turn a [`ResolvedTarget`] into a live [`Client`]: a local vault auto-starts its daemon, an
/// explicit socket / remote host connect directly. The single side-effecting half of the resolver.
pub fn connect_resolved(
    target: ResolvedTarget,
    mdkbd_path: Option<&Path>,
) -> Result<Client, String> {
    match target {
        ResolvedTarget::Remote { host, token } => Ok(Client::tcp(host, token)),
        ResolvedTarget::LocalSocket { socket } => Ok(Client::new(socket)),
        ResolvedTarget::LocalVault { vault } => {
            let paths = DaemonPaths::from_vault(vault);
            ensure_daemon(&paths, mdkbd_path)
        }
    }
}

/// Resolve a client end-to-end from explicit inputs: reads the env + registry default, applies the
/// precedence, and connects. The one entry point every client uses, so they cannot diverge.
pub fn resolve_client(inputs: &ClientInputs, mdkbd_path: Option<&Path>) -> Result<Client, String> {
    let env = EnvSnapshot::read();
    let registry_default = Registry::load().default_connection();
    let target = resolve_target(inputs, &env, Some(&registry_default))?;
    connect_resolved(target, mdkbd_path)
}

/// Resolve a [`ConnectionConfig`] into a connected [`Client`].
///
/// For [`ConnectionConfig::Local`] this **ensures** a daemon is running for the vault,
/// spawning one (detached) if none is reachable. `mdkbd_path` lets a bundled binary be used
/// (e.g. the one shipped inside the desktop app); when `None`, the daemon is looked up beside
/// the current executable, then on `PATH`.
pub fn connect(cfg: &ConnectionConfig, mdkbd_path: Option<&Path>) -> Result<Client, String> {
    match cfg {
        ConnectionConfig::Remote { host, token } => {
            if host.trim().is_empty() {
                return Err("remote host is empty".to_string());
            }
            Ok(Client::tcp(host.clone(), token.clone()))
        }
        ConnectionConfig::Local { vault } => {
            let paths = DaemonPaths::from_vault(vault);
            ensure_daemon(&paths, mdkbd_path)
        }
    }
}

/// Ensure a daemon is reachable on `paths.socket`, spawning a **detached** `mdkbd` if needed.
pub fn ensure_daemon(paths: &DaemonPaths, mdkbd_path: Option<&Path>) -> Result<Client, String> {
    let client = Client::new(&paths.socket);
    if client.ping() {
        return Ok(client);
    }
    paths
        .ensure_dirs()
        .map_err(|e| format!("preparing {}: {e}", paths.vault.display()))?;
    spawn_detached(paths, mdkbd_path)?;
    wait_until_ready(&client, ready_timeout())?;
    Ok(client)
}

/// How long [`ensure_daemon`] waits for a freshly spawned daemon to answer its first ping.
///
/// Defaults to 30s — instant on healthy storage, since a daemon binds its socket in well under a
/// second. The initial reconcile (parse every block + write the index) can be far slower on slow
/// or network-backed storage, though: a cold CI runner on NFS has been seen to take ~20–30s, right
/// at the edge of the default. `MDKB_READY_TIMEOUT_SECS` lets such environments (e.g. the export
/// gate in CI) grant more headroom without affecting interactive clients. A missing/invalid/zero
/// value falls back to the default.
const DEFAULT_READY_TIMEOUT_SECS: u64 = 30;

fn ready_timeout() -> Duration {
    let secs = std::env::var(crate::env::READY_TIMEOUT_SECS)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(DEFAULT_READY_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Spawn `mdkbd` for `paths`, fully detached so it outlives the spawning process.
fn spawn_detached(paths: &DaemonPaths, mdkbd_path: Option<&Path>) -> Result<(), String> {
    let exe = resolve_mdkbd(mdkbd_path)?;
    // Daemon output goes to a log file in the vault's local `.mdkb` dir.
    let log_path = paths.mdkb_dir().join("mdkbd.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("opening {}: {e}", log_path.display()))?;
    let log_err = log
        .try_clone()
        .map_err(|e| format!("cloning log handle: {e}"))?;

    let mut cmd = Command::new(&exe);
    cmd.arg("--vault")
        .arg(&paths.vault)
        .arg("--socket")
        .arg(&paths.socket)
        .arg("--db")
        .arg(&paths.db)
        // Auto-started daemons self-reap when idle so an unused vault doesn't leak a process
        // (and its embedder RAM). A manually-run daemon omits this and runs forever.
        .arg("--idle-timeout")
        .arg(AUTOSTART_IDLE_SECS.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));

    // Detach so the daemon survives the parent (app) exiting.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group → does not receive the parent's terminal/quit signals.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW.
        cmd.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
    }

    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to spawn {}: {e}", exe.display()))
}

/// Locate the `mdkbd` binary: an explicit path, else beside the current executable, else
/// rely on `PATH`.
fn resolve_mdkbd(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        return Err(format!("configured mdkbd path not found: {}", p.display()));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(mdkbd_filename());
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    Ok(PathBuf::from("mdkbd"))
}

fn mdkbd_filename() -> &'static str {
    if cfg!(windows) {
        "mdkbd.exe"
    } else {
        "mdkbd"
    }
}

fn wait_until_ready(client: &Client, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if client.ping() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "daemon did not become ready within {}s on {}",
        timeout.as_secs(),
        client.socket().display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialises tests that mutate process-global env vars (`MDKB_CONFIG_DIR`, etc.), since Rust
    /// runs a crate's tests in parallel and the env is shared.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_is_local_default_vault() {
        assert_eq!(
            ConnectionConfig::default(),
            ConnectionConfig::Local {
                vault: DaemonPaths::default_vault()
            }
        );
    }

    #[test]
    fn serde_round_trips_both_modes() {
        let local = ConnectionConfig::Local {
            vault: "/tmp/v".into(),
        };
        let s = serde_json::to_string(&local).unwrap();
        assert_eq!(serde_json::from_str::<ConnectionConfig>(&s).unwrap(), local);

        let remote = ConnectionConfig::Remote {
            host: "h:7820".into(),
            token: "tok".into(),
        };
        let s = serde_json::to_string(&remote).unwrap();
        assert_eq!(
            serde_json::from_str::<ConnectionConfig>(&s).unwrap(),
            remote
        );
    }

    #[test]
    fn remote_token_defaults_when_absent() {
        let r: ConnectionConfig =
            serde_json::from_str(r#"{"mode":"remote","host":"h:7820"}"#).unwrap();
        assert_eq!(
            r,
            ConnectionConfig::Remote {
                host: "h:7820".into(),
                token: String::new()
            }
        );
    }

    #[test]
    fn load_save_round_trip() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("mdkb-conncfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var(crate::env::CONFIG_DIR, &dir);
        let cfg = ConnectionConfig::Remote {
            host: "example:7820".into(),
            token: "secret".into(),
        };
        // Saving a connection persists it as the registry's default entry; loading returns it.
        cfg.save().unwrap();
        assert_eq!(ConnectionConfig::load(), cfg);
        // It was written to vaults.json as a registry, not a bare connection.
        let written = std::fs::read_to_string(dir.join("vaults.json")).unwrap();
        assert!(written.contains("\"vaults\""), "expected a registry file");
        assert!(written.contains("\"default\""));
        std::env::remove_var(crate::env::CONFIG_DIR);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn connect_remote_builds_tcp_client() {
        let c = connect(
            &ConnectionConfig::Remote {
                host: "h:7820".into(),
                token: "t".into(),
            },
            None,
        )
        .unwrap();
        assert_eq!(c.endpoint(), "tcp:h:7820");
    }

    #[test]
    fn ready_timeout_honors_env_with_safe_fallback() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Default when unset.
        std::env::remove_var(crate::env::READY_TIMEOUT_SECS);
        assert_eq!(
            ready_timeout(),
            Duration::from_secs(DEFAULT_READY_TIMEOUT_SECS)
        );
        // A valid positive override wins.
        std::env::set_var(crate::env::READY_TIMEOUT_SECS, "120");
        assert_eq!(ready_timeout(), Duration::from_secs(120));
        // Zero and garbage fall back to the default (never an instant-timeout footgun).
        std::env::set_var(crate::env::READY_TIMEOUT_SECS, "0");
        assert_eq!(
            ready_timeout(),
            Duration::from_secs(DEFAULT_READY_TIMEOUT_SECS)
        );
        std::env::set_var(crate::env::READY_TIMEOUT_SECS, "not-a-number");
        assert_eq!(
            ready_timeout(),
            Duration::from_secs(DEFAULT_READY_TIMEOUT_SECS)
        );
        std::env::remove_var(crate::env::READY_TIMEOUT_SECS);
    }

    // ---------- registry ----------

    #[test]
    fn registry_nested_connection_round_trips() {
        let reg = Registry {
            vaults: vec![
                VaultEntry {
                    name: "notes".into(),
                    connection: ConnectionConfig::Local {
                        vault: "~/OneDrive/notes".into(),
                    },
                },
                VaultEntry {
                    name: "work".into(),
                    connection: ConnectionConfig::Remote {
                        host: "10.0.0.5:7820".into(),
                        token: "secret".into(),
                    },
                },
            ],
            default: Some("notes".into()),
        };
        let json = serde_json::to_string_pretty(&reg).unwrap();
        // Nested (not flattened): the connection is its own object under each entry.
        assert!(json.contains("\"connection\""));
        assert!(json.contains("\"mode\": \"local\""));
        assert_eq!(serde_json::from_str::<Registry>(&json).unwrap(), reg);
    }

    #[test]
    fn registry_rejects_unknown_fields() {
        // deny_unknown_fields guards against silent typos in a hand-edited file.
        let bad = r#"{"vaults":[],"defualt":"x"}"#; // misspelled "default"
        assert!(serde_json::from_str::<Registry>(bad).is_err());
    }

    #[test]
    fn default_connection_resolves_named_default() {
        let reg = Registry {
            vaults: vec![
                VaultEntry {
                    name: "a".into(),
                    connection: ConnectionConfig::Local { vault: "/a".into() },
                },
                VaultEntry {
                    name: "b".into(),
                    connection: ConnectionConfig::Local { vault: "/b".into() },
                },
            ],
            default: Some("b".into()),
        };
        assert_eq!(
            reg.default_connection(),
            ConnectionConfig::Local { vault: "/b".into() }
        );
    }

    #[test]
    fn default_connection_missing_name_falls_back_to_builtin_not_another_vault() {
        // A dangling default must NOT silently connect to some other configured vault.
        let reg = Registry {
            vaults: vec![VaultEntry {
                name: "a".into(),
                connection: ConnectionConfig::Local { vault: "/a".into() },
            }],
            default: Some("ghost".into()),
        };
        assert_eq!(reg.default_connection(), ConnectionConfig::default());
    }

    #[test]
    fn default_connection_none_uses_first_then_builtin() {
        let with = Registry {
            vaults: vec![VaultEntry {
                name: "a".into(),
                connection: ConnectionConfig::Local { vault: "/a".into() },
            }],
            default: None,
        };
        assert_eq!(
            with.default_connection(),
            ConnectionConfig::Local { vault: "/a".into() }
        );
        let empty = Registry::default();
        assert_eq!(empty.default_connection(), ConnectionConfig::default());
    }

    #[test]
    fn set_default_connection_creates_then_updates_default_entry() {
        let mut reg = Registry::default();
        reg.set_default_connection(ConnectionConfig::Local { vault: "/x".into() });
        assert_eq!(reg.default.as_deref(), Some("default"));
        assert_eq!(reg.vaults.len(), 1);
        // Saving again replaces the default entry in place (no duplicate).
        reg.set_default_connection(ConnectionConfig::Local { vault: "/y".into() });
        assert_eq!(reg.vaults.len(), 1);
        assert_eq!(
            reg.default_connection(),
            ConnectionConfig::Local { vault: "/y".into() }
        );
    }

    // ---------- pure resolver (no env / fs / spawn) ----------

    fn no_env() -> EnvSnapshot {
        EnvSnapshot::default()
    }

    #[test]
    fn resolve_explicit_vault_beats_everything() {
        let inputs = ClientInputs {
            vault: Some("/explicit".into()),
            ..Default::default()
        };
        let env = EnvSnapshot {
            remote: Some("h:1".into()),
            token: Some("t".into()),
            ..Default::default()
        };
        let reg = ConnectionConfig::Remote {
            host: "reg:1".into(),
            token: "t".into(),
        };
        // Explicit --vault wins over env remote and the registry default.
        assert_eq!(
            resolve_target(&inputs, &env, Some(&reg)).unwrap(),
            ResolvedTarget::LocalVault {
                vault: "/explicit".into()
            }
        );
    }

    #[test]
    fn resolve_env_beats_registry_default() {
        let env = EnvSnapshot {
            vault: Some("/from-env".into()),
            ..Default::default()
        };
        let reg = ConnectionConfig::Local {
            vault: "/from-registry".into(),
        };
        assert_eq!(
            resolve_target(&ClientInputs::default(), &env, Some(&reg)).unwrap(),
            ResolvedTarget::LocalVault {
                vault: "/from-env".into()
            }
        );
    }

    #[test]
    fn resolve_registry_default_when_no_flag_or_env() {
        let reg = ConnectionConfig::Local {
            vault: "/from-registry".into(),
        };
        assert_eq!(
            resolve_target(&ClientInputs::default(), &no_env(), Some(&reg)).unwrap(),
            ResolvedTarget::LocalVault {
                vault: "/from-registry".into()
            }
        );
    }

    #[test]
    fn resolve_builtin_when_nothing_configured() {
        assert_eq!(
            resolve_target(&ClientInputs::default(), &no_env(), None).unwrap(),
            ResolvedTarget::LocalVault {
                vault: DaemonPaths::default_vault()
            }
        );
    }

    #[test]
    fn resolve_remote_requires_a_token() {
        let inputs = ClientInputs {
            remote: Some("h:7820".into()),
            ..Default::default()
        };
        // No token anywhere → error.
        assert!(resolve_target(&inputs, &no_env(), None).is_err());
        // Token from env satisfies an explicit --remote.
        let env = EnvSnapshot {
            token: Some("t".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_target(&inputs, &env, None).unwrap(),
            ResolvedTarget::Remote {
                host: "h:7820".into(),
                token: "t".into()
            }
        );
    }

    #[test]
    fn resolve_expands_tilde_in_vault() {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", "/home/tester");
        let inputs = ClientInputs {
            vault: Some("~/notes".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_target(&inputs, &no_env(), None).unwrap(),
            ResolvedTarget::LocalVault {
                vault: "/home/tester/notes".into()
            }
        );
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn resolve_socket_beats_vault_within_a_layer() {
        let inputs = ClientInputs {
            vault: Some("/v".into()),
            socket: Some("/run/x.sock".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_target(&inputs, &no_env(), None).unwrap(),
            ResolvedTarget::LocalSocket {
                socket: "/run/x.sock".into()
            }
        );
    }
}
