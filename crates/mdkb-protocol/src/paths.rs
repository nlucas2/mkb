//! Shared resolution of the daemon's on-disk paths.
//!
//! Both `mdkbd` (which creates them) and clients like `mdkb-mcp` (which must find the
//! socket, and may auto-start the daemon) need the same vault → db/socket mapping. Keeping
//! it here means the layout is defined once, never duplicated (see `AGENTS.md`).

use std::path::{Path, PathBuf};

/// The standard locations derived from a vault directory.
///
/// The index db, socket, lock, log, and embedder config are **machine-local** and must never live
/// inside a (possibly cloud-synced) vault — only the Markdown should sync. So by default they live
/// in a per-vault directory under the OS's local-data location (e.g. `%LOCALAPPDATA%\mdkb\<id>\`,
/// `~/Library/Application Support/mdkb/<id>/`, `~/.local/state/mdkb/<id>/`), keyed by a stable hash
/// of the vault's absolute path. `$MDKB_INDEX_DIR` overrides the base; with no resolvable home (a
/// minimal container) it falls back to the legacy in-vault `<vault>/.mdkb/`. The same resolver is
/// used by the daemon and by clients' auto-start, so they always agree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonPaths {
    /// Vault root (directory of Markdown files — the only thing meant to sync).
    pub vault: PathBuf,
    /// SQLite index database (machine-local).
    pub db: PathBuf,
    /// Local socket the daemon listens on (machine-local).
    pub socket: PathBuf,
}

impl DaemonPaths {
    /// Derive the standard paths from a vault directory. The index/socket/etc. land in the
    /// per-vault machine-local directory (see [`DaemonPaths`]), never inside the vault by default.
    pub fn from_vault(vault: impl Into<PathBuf>) -> Self {
        let vault = vault.into();
        let dir = index_dir_for(&vault);
        DaemonPaths {
            db: dir.join("index.db"),
            socket: dir.join("mdkbd.sock"),
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

    /// The machine-local directory holding db/socket/lock/log/embedder-config (parent of db/socket).
    pub fn mdkb_dir(&self) -> &Path {
        self.socket.parent().unwrap_or(&self.vault)
    }

    /// Create the vault and the machine-local index directory if missing. The index directory holds
    /// the trusted local socket and the index, so on Unix it is restricted to the owner.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.vault)?;
        let mdkb_dir = self.mdkb_dir();
        std::fs::create_dir_all(mdkb_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(mdkb_dir, std::fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }
}

impl Default for DaemonPaths {
    fn default() -> Self {
        DaemonPaths::for_default_vault()
    }
}

/// The per-vault machine-local directory for index/socket/lock/log/config (see [`DaemonPaths`]).
fn index_dir_for(vault: &Path) -> PathBuf {
    index_dir_in(vault, resolved_index_base())
}

/// Resolve the per-vault dir given a base: `base/<vault-id>` when there is a base, else the legacy
/// in-vault `<vault>/.mdkb` (the fallback when no OS local-data dir is resolvable). Split out from
/// [`index_dir_for`] so the (env-free) path logic is unit-testable.
fn index_dir_in(vault: &Path, base: Option<PathBuf>) -> PathBuf {
    match base {
        Some(base) => base.join(vault_id(vault)),
        None => vault.join(".mdkb"),
    }
}

/// The base directory under which per-vault index dirs live: `$MDKB_INDEX_DIR` if set, else the
/// OS local-data dir with an `mdkb` segment, else `None` (→ legacy in-vault fallback).
fn resolved_index_base() -> Option<PathBuf> {
    if let Some(b) = std::env::var_os("MDKB_INDEX_DIR") {
        return Some(PathBuf::from(b));
    }
    os_local_data_dir().map(|d| d.join("mdkb"))
}

/// The OS's per-user local-data directory, or `None` if it can't be resolved (no home env).
fn os_local_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(x) = std::env::var_os("XDG_STATE_HOME") {
            return Some(PathBuf::from(x));
        }
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state"))
    }
}

/// A short, stable id for a vault: a 64-bit FNV-1a hash of its absolute (best-effort canonical)
/// path, hex-encoded. Stable across runs on one machine; deliberately *different* per machine for a
/// synced vault, since each machine keeps its own local index.
fn vault_id(vault: &Path) -> String {
    let abs = std::fs::canonicalize(vault).unwrap_or_else(|_| absolute_lossy(vault));
    format!("{:016x}", fnv1a64(abs.to_string_lossy().as_bytes()))
}

/// Best-effort absolute path without requiring the path to exist (unlike `canonicalize`).
fn absolute_lossy(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(p),
        Err(_) => p.to_path_buf(),
    }
}

/// FNV-1a 64-bit — a tiny, dependency-free, *stable* hash (unlike `DefaultHasher`, whose output is
/// not guaranteed stable across builds), so a vault always maps to the same local index dir.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_dir_uses_base_and_a_stable_vault_id() {
        let base = PathBuf::from("/data/mdkb");
        let dir = index_dir_in(Path::new("/home/me/vault"), Some(base.clone()));
        // <base>/<vault-id>, and the id is a 16-hex-char FNV hash.
        assert_eq!(dir.parent().unwrap(), base);
        let id = dir.file_name().unwrap().to_str().unwrap();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        // Same vault → same id; different vault → different id.
        assert_eq!(
            index_dir_in(Path::new("/home/me/vault"), Some(base.clone())),
            dir
        );
        assert_ne!(
            index_dir_in(Path::new("/home/me/other"), Some(base.clone())),
            dir
        );
    }

    #[test]
    fn index_dir_falls_back_to_in_vault_when_no_base() {
        // No resolvable OS dir (e.g. a minimal container) → legacy <vault>/.mdkb.
        assert_eq!(
            index_dir_in(Path::new("/srv/vault"), None),
            PathBuf::from("/srv/vault/.mdkb")
        );
    }

    #[test]
    fn db_and_socket_share_one_dir() {
        // Whatever the resolved location, db and socket are siblings and mdkb_dir is their parent.
        let p = DaemonPaths::from_vault("/tmp/v");
        assert_eq!(p.db.file_name().unwrap(), "index.db");
        assert_eq!(p.socket.file_name().unwrap(), "mdkbd.sock");
        assert_eq!(p.db.parent(), p.socket.parent());
        assert_eq!(p.mdkb_dir(), p.db.parent().unwrap());
        // The index must NOT live inside the vault (unless we hit the no-home fallback).
        if super::resolved_index_base().is_some() {
            assert!(
                !p.db.starts_with("/tmp/v"),
                "index leaked into the vault: {:?}",
                p.db
            );
        }
    }

    #[test]
    fn fnv_is_stable_and_distinguishes() {
        assert_eq!(fnv1a64(b"hello"), fnv1a64(b"hello"));
        assert_ne!(fnv1a64(b"hello"), fnv1a64(b"world"));
    }
}
