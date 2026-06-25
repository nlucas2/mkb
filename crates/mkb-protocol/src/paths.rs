//! Shared resolution of the daemon's on-disk paths.
//!
//! Both `mkbd` (which creates them) and clients like `mkb-mcp` (which must find the
//! socket, and may auto-start the daemon) need the same vault → db/socket mapping. Keeping
//! it here means the layout is defined once, never duplicated (see `AGENTS.md`).

use std::path::{Path, PathBuf};

/// The standard locations derived from a vault directory.
///
/// The index db, socket, lock, log, and embedder config are **machine-local** and must never live
/// inside a (possibly cloud-synced) vault — only the Markdown should sync. So by default they live
/// in a per-vault directory under the OS's local-data location (e.g. `%LOCALAPPDATA%\mkb\<id>\`,
/// `~/Library/Application Support/mkb/<id>/`, `~/.local/state/mkb/<id>/`), keyed by a stable hash
/// of the vault's absolute path. `$MKB_INDEX_DIR` overrides the base; with no resolvable home (a
/// minimal container) it falls back to the legacy in-vault `<vault>/.mkb/`. The same resolver is
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
        let vault = expand_user(vault.into());
        let dir = index_dir_for(&vault);
        DaemonPaths {
            db: dir.join("index.db"),
            socket: dir.join("mkbd.sock"),
            vault,
        }
    }

    /// The default vault directory: `$MKB_VAULT`, else `~/mkb-vault`, else `./mkb-vault`.
    pub fn default_vault() -> PathBuf {
        if let Some(v) = std::env::var_os("MKB_VAULT") {
            return expand_user(PathBuf::from(v));
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("mkb-vault");
        }
        PathBuf::from("mkb-vault")
    }

    /// Paths for the default vault.
    pub fn for_default_vault() -> Self {
        DaemonPaths::from_vault(Self::default_vault())
    }

    /// The machine-local directory holding db/socket/lock/log/embedder-config (parent of db/socket).
    pub fn mkb_dir(&self) -> &Path {
        self.socket.parent().unwrap_or(&self.vault)
    }

    /// Create the vault and the machine-local index directory if missing. The index directory holds
    /// the trusted local socket and the index, so on Unix it is restricted to the owner.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.vault)?;
        let mkb_dir = self.mkb_dir();
        std::fs::create_dir_all(mkb_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(mkb_dir, std::fs::Permissions::from_mode(0o700));
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
/// in-vault `<vault>/.mkb` (the fallback when no OS local-data dir is resolvable). Split out from
/// [`index_dir_for`] so the (env-free) path logic is unit-testable.
fn index_dir_in(vault: &Path, base: Option<PathBuf>) -> PathBuf {
    match base {
        Some(base) => base.join(vault_id(vault)),
        None => vault.join(".mkb"),
    }
}

/// The base directory under which per-vault index dirs live: `$MKB_INDEX_DIR` if set, else the
/// OS local-data dir with an `mkb` segment, else `None` (→ legacy in-vault fallback).
fn resolved_index_base() -> Option<PathBuf> {
    if let Some(b) = std::env::var_os("MKB_INDEX_DIR") {
        return Some(PathBuf::from(b));
    }
    mkb_core::dirs::local_data_dir().map(|d| d.join("mkb"))
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

/// Expand a leading `~` to the user's home directory, so a config or flag may *optionally* use a
/// home-relative path (e.g. `~/OneDrive/notes`) that resolves correctly on any machine — which is
/// what makes a synced `vaults.json` portable. This is **support, not a requirement**: an absolute
/// or relative path is returned unchanged. A bare `~` becomes `$HOME`; `~/x` becomes `$HOME/x`.
/// A `~user` form is *not* expanded (left literal). If `$HOME` is unset, the `~` path is returned
/// unchanged rather than erroring.
pub fn expand_user(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    let s = match path.to_str() {
        Some(s) => s,
        None => return path, // non-UTF8 path: nothing to expand, pass through
    };
    let rest = if s == "~" {
        ""
    } else if let Some(r) = s.strip_prefix("~/") {
        r
    } else {
        // Not a home-relative path (absolute, relative, or `~user`) → unchanged.
        return path;
    };
    match home_dir() {
        Some(home) if rest.is_empty() => home,
        Some(home) => home.join(rest),
        None => path, // no home resolvable → leave the `~` path literal rather than erroring
    }
}

/// The user's home directory: `$HOME` (Unix/macOS) or `%USERPROFILE%` (Windows), if set.
fn home_dir() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("HOME") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    #[cfg(windows)]
    if let Some(h) = std::env::var_os("USERPROFILE") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    None
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
        let base = PathBuf::from("/data/mkb");
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
        // No resolvable OS dir (e.g. a minimal container) → legacy <vault>/.mkb.
        assert_eq!(
            index_dir_in(Path::new("/srv/vault"), None),
            PathBuf::from("/srv/vault/.mkb")
        );
    }

    #[test]
    fn db_and_socket_share_one_dir() {
        // Whatever the resolved location, db and socket are siblings and mkb_dir is their parent.
        let p = DaemonPaths::from_vault("/tmp/v");
        assert_eq!(p.db.file_name().unwrap(), "index.db");
        assert_eq!(p.socket.file_name().unwrap(), "mkbd.sock");
        assert_eq!(p.db.parent(), p.socket.parent());
        assert_eq!(p.mkb_dir(), p.db.parent().unwrap());
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

    #[test]
    fn expand_user_handles_tilde_absolute_and_relative() {
        // A `~/x` path expands against HOME; absolute/relative paths are untouched.
        let home = std::path::PathBuf::from("/home/tester");
        // Drive HOME deterministically for this test.
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);

        assert_eq!(expand_user("~/notes"), home.join("notes"));
        assert_eq!(expand_user("~"), home);
        // Absolute path: unchanged.
        assert_eq!(expand_user("/srv/vault"), PathBuf::from("/srv/vault"));
        // Relative path: unchanged (no implicit cwd join here).
        assert_eq!(expand_user("notes/sub"), PathBuf::from("notes/sub"));
        // `~user` form is NOT expanded.
        assert_eq!(expand_user("~bob/x"), PathBuf::from("~bob/x"));

        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
