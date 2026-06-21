//! Client-side connection configuration and daemon lifecycle.
//!
//! A *client* (the desktop app, the web UI, the MCP server) needs to know **where** the
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
/// / web UI) holds a heartbeat **lease** that keeps the daemon alive while it's open, so when that
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
    /// Per-user path of the client connection config (distinct from a vault's `.mdkb`):
    /// `$MDKB_CONFIG_DIR`, else the OS app-config dir, else `~/.config/mdkb`.
    pub fn config_path() -> PathBuf {
        client_config_dir().join("connection.json")
    }

    /// Load the connection config, falling back to the default (local default vault) when the
    /// file is missing or unreadable/invalid.
    pub fn load() -> ConnectionConfig {
        let path = Self::config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!(
                    "mdkb: ignoring malformed {}: {e}; using defaults",
                    path.display()
                );
                ConnectionConfig::default()
            }),
            Err(_) => ConnectionConfig::default(),
        }
    }

    /// Persist the connection config, creating the config directory if needed.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))
    }

    /// A short human description of this connection (for UI/logs).
    pub fn describe(&self) -> String {
        match self {
            ConnectionConfig::Local { vault } => format!("local:{}", vault.display()),
            ConnectionConfig::Remote { host, .. } => format!("remote:{host}"),
        }
    }
}

/// The OS-appropriate per-user config directory for mdkb client config.
fn client_config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MDKB_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/dev.mdkb.desktop");
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
    wait_until_ready(&client, Duration::from_secs(30))?;
    Ok(client)
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
        let dir = std::env::temp_dir().join(format!("mdkb-conncfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("MDKB_CONFIG_DIR", &dir);
        let cfg = ConnectionConfig::Remote {
            host: "example:7820".into(),
            token: "secret".into(),
        };
        cfg.save().unwrap();
        assert_eq!(ConnectionConfig::load(), cfg);
        std::env::remove_var("MDKB_CONFIG_DIR");
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
}
