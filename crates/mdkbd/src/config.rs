//! Daemon configuration: where the vault, index db, and socket live.
//!
//! Defaults keep the index and socket under `<vault>/.mdkb/` (a hidden dir the indexer
//! skips). **That directory must be excluded from OneDrive/cloud sync** — only the Markdown
//! is meant to sync; each machine keeps its own local index (plan sync model).

use std::path::PathBuf;

/// Resolved daemon configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Vault root (directory of Markdown files).
    pub vault: PathBuf,
    /// SQLite index database path (local only).
    pub db: PathBuf,
    /// Unix socket path the daemon listens on (local only).
    pub socket: PathBuf,
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

    /// Parse configuration from CLI args (already past the program name) and environment.
    pub fn from_args(args: impl Iterator<Item = String>) -> Result<Config, String> {
        let mut vault: Option<PathBuf> = std::env::var_os("MDKB_VAULT").map(PathBuf::from);
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

        let vault = vault.unwrap_or_else(default_vault);
        let mdkb_dir = vault.join(".mdkb");
        let db = db.unwrap_or_else(|| mdkb_dir.join("index.db"));
        let socket = socket.unwrap_or_else(|| mdkb_dir.join("mdkbd.sock"));

        Ok(Config {
            vault,
            db,
            socket,
            help,
        })
    }

    /// Create the vault and the local `.mdkb` directories if missing.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.vault)?;
        if let Some(parent) = self.db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Some(parent) = self.socket.parent() {
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

fn default_vault() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join("mdkb-vault")
    } else {
        PathBuf::from("mdkb-vault")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_derive_from_vault() {
        let cfg = Config::from_args(["--vault".into(), "/tmp/v".into()].into_iter()).unwrap();
        assert_eq!(cfg.vault, PathBuf::from("/tmp/v"));
        assert_eq!(cfg.db, PathBuf::from("/tmp/v/.mdkb/index.db"));
        assert_eq!(cfg.socket, PathBuf::from("/tmp/v/.mdkb/mdkbd.sock"));
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
        assert_eq!(cfg.db, PathBuf::from("/var/cache/i.db"));
        assert_eq!(cfg.socket, PathBuf::from("/run/m.sock"));
    }

    #[test]
    fn unknown_arg_errors() {
        assert!(Config::from_args(["--nope".into()].into_iter()).is_err());
    }
}
