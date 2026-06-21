//! Daemon configuration: parses CLI args/env into [`DaemonPaths`] plus flags.
//!
//! Path layout (vault -> db/socket) is resolved by [`mdkb_protocol::DaemonPaths`] so the
//! daemon and clients agree on locations without duplicating the rule.

use std::path::{Path, PathBuf};

use mdkb_protocol::DaemonPaths;

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
    /// Whether `--help` was requested.
    pub help: bool,
}

impl Config {
    /// Usage text.
    pub fn usage() -> &'static str {
        "\
mdkbd — mdkb headless daemon

usage:
  mdkbd [--vault <dir>] [--db <path>] [--socket <path>] [--listen <addr>] [--token <tok>] [--idle-timeout <secs>]

options:
  --vault <dir>     vault directory (default: $MDKB_VAULT or ~/mdkb-vault)
  --db <path>       index database (default: <vault>/.mdkb/index.db)
  --socket <path>   local socket: Unix socket / Windows named pipe (default: <vault>/.mdkb/mdkbd.sock)
  --listen <addr>   ALSO serve over TCP at <addr> (e.g. 0.0.0.0:7820); requires a token
  --token <tok>     shared token network clients must present ($MDKB_TOKEN also accepted)
  --idle-timeout <secs>  self-shutdown after <secs> with no requests AND no interactive lease
                         (0 = never; default: never when run manually)
  --help            show this help

The index and socket directory (<vault>/.mdkb) is local-only and must be excluded
from cloud sync; only the Markdown files are meant to sync.

The network listener is opt-in and fails closed: without a valid token, remote callers
are rejected. The Unix socket remains local-only and trusted."
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

    /// Parse configuration from CLI args (already past the program name) and environment.
    pub fn from_args(args: impl Iterator<Item = String>) -> Result<Config, String> {
        let mut vault: Option<PathBuf> = None;
        let mut db: Option<PathBuf> = None;
        let mut socket: Option<PathBuf> = None;
        let mut listen: Option<String> = None;
        let mut token: Option<String> = std::env::var("MDKB_TOKEN").ok().filter(|s| !s.is_empty());
        let mut idle_secs: Option<u64> = None;
        let mut help = false;

        let mut it = args.peekable();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--help" | "-h" => help = true,
                "--vault" => vault = Some(require_value(&mut it, "--vault")?.into()),
                "--db" => db = Some(require_value(&mut it, "--db")?.into()),
                "--socket" => socket = Some(require_value(&mut it, "--socket")?.into()),
                "--listen" => listen = Some(require_value(&mut it, "--listen")?),
                "--token" => token = Some(require_value(&mut it, "--token")?),
                "--idle-timeout" => {
                    idle_secs = Some(parse_secs(&require_value(&mut it, "--idle-timeout")?)?)
                }
                other => {
                    if let Some(v) = other.strip_prefix("--vault=") {
                        vault = Some(v.into());
                    } else if let Some(v) = other.strip_prefix("--db=") {
                        db = Some(v.into());
                    } else if let Some(v) = other.strip_prefix("--socket=") {
                        socket = Some(v.into());
                    } else if let Some(v) = other.strip_prefix("--listen=") {
                        listen = Some(v.into());
                    } else if let Some(v) = other.strip_prefix("--token=") {
                        token = Some(v.into());
                    } else if let Some(v) = other.strip_prefix("--idle-timeout=") {
                        idle_secs = Some(parse_secs(v)?);
                    } else {
                        return Err(format!("unknown argument: {other}"));
                    }
                }
            }
        }

        if listen.is_some() && token.as_deref().unwrap_or("").is_empty() {
            return Err(
                "--listen requires a token (set --token or $MDKB_TOKEN); refusing to expose \
                 the network listener without auth"
                    .to_string(),
            );
        }

        let vault = vault.unwrap_or_else(DaemonPaths::default_vault);
        let mut paths = DaemonPaths::from_vault(vault);
        if let Some(db) = db {
            paths.db = db;
        }
        if let Some(socket) = socket {
            paths.socket = socket;
        }

        Ok(Config {
            paths,
            listen,
            token,
            idle_timeout: idle_secs
                .filter(|s| *s > 0)
                .map(std::time::Duration::from_secs),
            help,
        })
    }

    /// Create the vault and the local `.mdkb` directories if missing.
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

fn require_value(
    it: &mut std::iter::Peekable<impl Iterator<Item = String>>,
    flag: &str,
) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_secs(s: &str) -> Result<u64, String> {
    s.trim()
        .parse::<u64>()
        .map_err(|_| format!("--idle-timeout expects a whole number of seconds, got {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_derive_from_vault() {
        let cfg = Config::from_args(["--vault".into(), "/tmp/v".into()].into_iter()).unwrap();
        assert_eq!(cfg.vault(), Path::new("/tmp/v"));
        assert_eq!(cfg.db(), Path::new("/tmp/v/.mdkb/index.db"));
        assert_eq!(cfg.socket(), Path::new("/tmp/v/.mdkb/mdkbd.sock"));
    }

    #[test]
    fn explicit_overrides_win() {
        let cfg = Config::from_args(
            [
                "--vault=/tmp/v".into(),
                "--db=/var/cache/i.db".into(),
                "--socket=/run/m.sock".into(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(cfg.db(), Path::new("/var/cache/i.db"));
        assert_eq!(cfg.socket(), Path::new("/run/m.sock"));
    }

    #[test]
    fn unknown_arg_errors() {
        assert!(Config::from_args(["--nope".into()].into_iter()).is_err());
    }

    #[test]
    fn idle_timeout_parses_and_zero_means_never() {
        // Absent → never.
        let cfg = Config::from_args(["--vault=/tmp/v".into()].into_iter()).unwrap();
        assert_eq!(cfg.idle_timeout, None);
        // A positive value → Some(Duration).
        let cfg = Config::from_args(
            [
                "--vault=/tmp/v".into(),
                "--idle-timeout".into(),
                "900".into(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(cfg.idle_timeout, Some(std::time::Duration::from_secs(900)));
        // `=` form works too.
        let cfg =
            Config::from_args(["--vault=/tmp/v".into(), "--idle-timeout=30".into()].into_iter())
                .unwrap();
        assert_eq!(cfg.idle_timeout, Some(std::time::Duration::from_secs(30)));
        // Zero disables it (treated as never).
        let cfg =
            Config::from_args(["--vault=/tmp/v".into(), "--idle-timeout=0".into()].into_iter())
                .unwrap();
        assert_eq!(cfg.idle_timeout, None);
        // Non-numeric is an error.
        assert!(Config::from_args(
            ["--vault=/tmp/v".into(), "--idle-timeout=soon".into()].into_iter()
        )
        .is_err());
    }

    #[test]
    fn listen_requires_a_token() {
        // --listen without a token must fail closed.
        std::env::remove_var("MDKB_TOKEN");
        let err = Config::from_args(["--listen=0.0.0.0:7820".into()].into_iter()).unwrap_err();
        assert!(err.contains("token"));
        // With a token it succeeds.
        let cfg = Config::from_args(
            ["--listen=0.0.0.0:7820".into(), "--token=secret".into()].into_iter(),
        )
        .unwrap();
        assert_eq!(cfg.listen.as_deref(), Some("0.0.0.0:7820"));
        assert_eq!(cfg.token.as_deref(), Some("secret"));
    }
}
