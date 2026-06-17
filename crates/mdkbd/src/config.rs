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
    /// Whether `--help` was requested.
    pub help: bool,
}

impl Config {
    /// Usage text.
    pub fn usage() -> &'static str {
        "\
mdkbd — mdkb headless daemon

usage:
  mdkbd [--vault <dir>] [--db <path>] [--socket <path>]

options:
  --vault <dir>     vault directory (default: $MDKB_VAULT or ~/mdkb-vault)
  --db <path>       index database (default: <vault>/.mdkb/index.db)
  --socket <path>   unix socket (default: <vault>/.mdkb/mdkbd.sock)
  --help            show this help

The index and socket directory (<vault>/.mdkb) is local-only and must be excluded
from cloud sync; only the Markdown files are meant to sync."
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
        let mut help = false;

        let mut it = args.peekable();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--help" | "-h" => help = true,
                "--vault" => vault = Some(require_value(&mut it, "--vault")?.into()),
                "--db" => db = Some(require_value(&mut it, "--db")?.into()),
                "--socket" => socket = Some(require_value(&mut it, "--socket")?.into()),
                other => {
                    if let Some(v) = other.strip_prefix("--vault=") {
                        vault = Some(v.into());
                    } else if let Some(v) = other.strip_prefix("--db=") {
                        db = Some(v.into());
                    } else if let Some(v) = other.strip_prefix("--socket=") {
                        socket = Some(v.into());
                    } else {
                        return Err(format!("unknown argument: {other}"));
                    }
                }
            }
        }

        let vault = vault.unwrap_or_else(DaemonPaths::default_vault);
        let mut paths = DaemonPaths::from_vault(vault);
        if let Some(db) = db {
            paths.db = db;
        }
        if let Some(socket) = socket {
            paths.socket = socket;
        }

        Ok(Config { paths, help })
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
}
