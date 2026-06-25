//! Daemon configuration: parses CLI args/env into [`DaemonPaths`] plus flags.
//!
//! These are **server** options (own/serve a vault), distinct from the client-connection seam: a
//! daemon's `--socket` is the address it *binds*, `--token` the secret it *requires* of callers.
//! Path layout (vault -> db/socket) is resolved by [`mkb_protocol::DaemonPaths`] so the daemon
//! and clients agree on locations without duplicating the rule.

use std::path::{Path, PathBuf};

use clap::Parser;
use mkb_protocol::DaemonPaths;

/// Raw daemon CLI arguments (parsed by clap), resolved into a [`Config`] by [`DaemonArgs::resolve`].
#[derive(Parser, Debug)]
#[command(
    name = "mkbd",
    version,
    about = "mkb headless daemon — owns the watcher, index, and writes for one vault",
    long_about = "mkb headless daemon.\n\nServes one vault over a local socket (and optionally \
                  TCP). The index/socket/lock/log are machine-local and live OUTSIDE the vault by \
                  default — under the OS local-data dir, keyed by a hash of the vault path — so a \
                  cloud-synced vault never syncs the live index. Set $MKB_INDEX_DIR to override \
                  the base. The network listener (--listen) is opt-in and fails closed: without a \
                  valid token, remote callers are rejected."
)]
pub struct DaemonArgs {
    /// Vault directory to serve (supports a leading `~`; default: $MKB_VAULT or ~/mkb-vault).
    #[arg(long, value_name = "DIR")]
    vault: Option<PathBuf>,
    /// Index database (default: a machine-local per-vault dir).
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
    /// Local socket: Unix socket / Windows named pipe (default: beside --db).
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,
    /// ALSO serve over TCP at this address (e.g. 0.0.0.0:7820); requires a token.
    #[arg(long, value_name = "ADDR")]
    listen: Option<String>,
    /// Shared token network clients must present ($MKB_TOKEN also accepted).
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
    /// Self-shutdown after this many seconds with no requests AND no interactive lease
    /// (0 = never; default: never when run manually).
    #[arg(long = "idle-timeout", value_name = "SECS")]
    idle_timeout: Option<u64>,
}

impl DaemonArgs {
    /// Resolve raw args + environment into a [`Config`]: apply the `$MKB_TOKEN` fallback, enforce
    /// that `--listen` has a token (fail closed), and derive the machine-local paths.
    pub fn resolve(self) -> Result<Config, String> {
        let token = self
            .token
            .or_else(|| std::env::var(mkb_protocol::env::TOKEN).ok())
            .filter(|s| !s.is_empty());

        if self.listen.is_some() && token.as_deref().unwrap_or("").is_empty() {
            return Err(
                "--listen requires a token (set --token or $MKB_TOKEN); refusing to expose \
                 the network listener without auth"
                    .to_string(),
            );
        }

        let vault = self.vault.unwrap_or_else(DaemonPaths::default_vault);
        let mut paths = DaemonPaths::from_vault(vault);
        if let Some(db) = self.db {
            paths.db = db;
        }
        if let Some(socket) = self.socket {
            paths.socket = socket;
        }

        Ok(Config {
            paths,
            listen: self.listen,
            token,
            idle_timeout: self
                .idle_timeout
                .filter(|s| *s > 0)
                .map(std::time::Duration::from_secs),
        })
    }
}

/// Resolved daemon configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Resolved paths.
    pub paths: DaemonPaths,
    /// Optional network listen address (e.g. `0.0.0.0:7820`). `None` = local socket only.
    pub listen: Option<String>,
    /// Shared token for network auth (required when `listen` is set).
    pub token: Option<String>,
    /// Self-shutdown after this long with no requests. `None` = run forever (the default for
    /// a manually-run or remote daemon). Clients that auto-start a daemon pass a value so an
    /// unused vault's daemon reaps itself instead of leaking.
    pub idle_timeout: Option<std::time::Duration>,
}

impl Config {
    /// Parse configuration from the process arguments (clap handles `--help`/`--version`/errors).
    pub fn parse() -> Result<Config, String> {
        DaemonArgs::parse().resolve()
    }

    /// Vault directory.
    pub fn vault(&self) -> &Path {
        &self.paths.vault
    }

    /// Index database path.
    pub fn db(&self) -> &Path {
        &self.paths.db
    }

    /// Socket path.
    pub fn socket(&self) -> &Path {
        &self.paths.socket
    }

    /// Create the vault and the local `.mkb` directories if missing.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        self.paths.ensure_dirs()?;
        if let Some(parent) = self.paths.db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Some(parent) = self.paths.socket.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `Config` from argv-style tokens (clap parses, then we resolve).
    fn resolve(argv: &[&str]) -> Result<Config, String> {
        let mut full = vec!["mkbd"];
        full.extend_from_slice(argv);
        let args = DaemonArgs::try_parse_from(full).map_err(|e| e.to_string())?;
        args.resolve()
    }

    #[test]
    fn defaults_derive_from_vault() {
        let cfg = resolve(&["--vault", "/tmp/v"]).unwrap();
        assert_eq!(cfg.vault(), Path::new("/tmp/v"));
        // db/socket default to a machine-local per-vault dir (resolved by DaemonPaths) — exact
        // location depends on the environment, but they're named consistently and share a dir.
        assert_eq!(cfg.db().file_name().unwrap(), "index.db");
        assert_eq!(cfg.socket().file_name().unwrap(), "mkbd.sock");
        assert_eq!(cfg.db().parent(), cfg.socket().parent());
    }

    #[test]
    fn explicit_overrides_win() {
        let cfg = resolve(&[
            "--vault=/tmp/v",
            "--db=/var/cache/i.db",
            "--socket=/run/m.sock",
        ])
        .unwrap();
        assert_eq!(cfg.db(), Path::new("/var/cache/i.db"));
        assert_eq!(cfg.socket(), Path::new("/run/m.sock"));
    }

    #[test]
    fn unknown_arg_errors() {
        assert!(resolve(&["--nope"]).is_err());
    }

    #[test]
    fn idle_timeout_parses_and_zero_means_never() {
        // Absent → never.
        let cfg = resolve(&["--vault=/tmp/v"]).unwrap();
        assert_eq!(cfg.idle_timeout, None);
        // A positive value → Some(Duration).
        let cfg = resolve(&["--vault=/tmp/v", "--idle-timeout", "900"]).unwrap();
        assert_eq!(cfg.idle_timeout, Some(std::time::Duration::from_secs(900)));
        // `=` form works too.
        let cfg = resolve(&["--vault=/tmp/v", "--idle-timeout=30"]).unwrap();
        assert_eq!(cfg.idle_timeout, Some(std::time::Duration::from_secs(30)));
        // Zero disables it (treated as never).
        let cfg = resolve(&["--vault=/tmp/v", "--idle-timeout=0"]).unwrap();
        assert_eq!(cfg.idle_timeout, None);
        // Non-numeric is an error (clap rejects it as an invalid u64).
        assert!(resolve(&["--vault=/tmp/v", "--idle-timeout=soon"]).is_err());
    }

    #[test]
    fn listen_requires_a_token() {
        // --listen without a token must fail closed.
        std::env::remove_var("MKB_TOKEN");
        let err = resolve(&["--listen=0.0.0.0:7820"]).unwrap_err();
        assert!(err.contains("token"));
        // With a token it succeeds.
        let cfg = resolve(&["--listen=0.0.0.0:7820", "--token=secret"]).unwrap();
        assert_eq!(cfg.listen.as_deref(), Some("0.0.0.0:7820"));
        assert_eq!(cfg.token.as_deref(), Some("secret"));
    }
}
