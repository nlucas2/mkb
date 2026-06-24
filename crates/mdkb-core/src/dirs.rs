//! Shared OS directory resolution.
//!
//! A single, dependency-free place that knows where per-user machine-local data lives, so the
//! path layout (the index dir in `mdkb-protocol`) and the bundled-model dir (in `mdkb-embed`)
//! agree instead of each rolling their own.

use std::path::PathBuf;

/// The OS's per-user local-data directory, or `None` if it can't be resolved (no home env — e.g.
/// a minimal container). Callers append their own `mdkb/...` subpath.
///
/// - Windows: `%LOCALAPPDATA%` (else `%APPDATA%`)
/// - macOS: `~/Library/Application Support`
/// - other Unix: `$XDG_STATE_HOME` (else `~/.local/state`)
pub fn local_data_dir() -> Option<PathBuf> {
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
