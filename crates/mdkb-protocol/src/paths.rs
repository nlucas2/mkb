//! Shared resolution of the daemon's on-disk paths.
//!
//! Both `mdkbd` (which creates them) and clients like `mdkb-mcp` (which must find the
//! socket, and may auto-start the daemon) need the same vault → db/socket mapping. Keeping
//! it here means the layout is defined once, never duplicated (see `AGENTS.md`).

use std::path::{Path, PathBuf};

/// The standard locations derived from a vault directory.
///
/// The index db and socket live under `<vault>/.mdkb/`, a hidden directory the indexer
/// skips. That directory is **local-only** and must be excluded from cloud sync — only the
/// Markdown is meant to sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonPaths {
    /// Vault root (directory of Markdown files).
    pub vault: PathBuf,
    /// SQLite index database (local only).
    pub db: PathBuf,
    /// Unix socket the daemon listens on (local only).
    pub socket: PathBuf,
}

impl DaemonPaths {
    /// Derive the standard paths from a vault directory.
    pub fn from_vault(vault: impl Into<PathBuf>) -> Self {
        let vault = vault.into();
        let mdkb_dir = vault.join(".mdkb");
        DaemonPaths {
            db: mdkb_dir.join("index.db"),
            socket: mdkb_dir.join("mdkbd.sock"),
            vault,
        }
    }

    /// The default vault directory: `$MDKB_VAULT`, else `~/mdkb-vault`, else `./mdkb-vault`.
    pub fn default_vault() -> PathBuf {
        if let Some(v) = std::env::var_os("MDKB_VAULT") {
            return PathBuf::from(v);
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("mdkb-vault");
        }
        PathBuf::from("mdkb-vault")
    }

    /// Paths for the default vault.
    pub fn for_default_vault() -> Self {
        DaemonPaths::from_vault(Self::default_vault())
    }

    /// The `.mdkb` directory (parent of db/socket).
    pub fn mdkb_dir(&self) -> &Path {
        self.socket.parent().unwrap_or(&self.vault)
    }

    /// Create the vault and `.mdkb` directories if missing.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.vault)?;
        std::fs::create_dir_all(self.mdkb_dir())?;
        Ok(())
    }
}

impl Default for DaemonPaths {
    fn default() -> Self {
        DaemonPaths::for_default_vault()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_db_and_socket_under_mdkb() {
        let p = DaemonPaths::from_vault("/tmp/v");
        assert_eq!(p.db, PathBuf::from("/tmp/v/.mdkb/index.db"));
        assert_eq!(p.socket, PathBuf::from("/tmp/v/.mdkb/mdkbd.sock"));
        assert_eq!(p.mdkb_dir(), Path::new("/tmp/v/.mdkb"));
    }
}
