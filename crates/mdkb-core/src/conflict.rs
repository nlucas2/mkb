//! Detection of cloud-sync conflict files.
//!
//! When a vault is synced across machines (OneDrive, Dropbox, …) and the same file is
//! edited in two places, the sync engine spawns a *conflict copy* with a mangled name
//! rather than merging. mdkb must not silently index these — that would pollute search with
//! duplicate, possibly-stale content. Instead it detects them, skips indexing, and surfaces
//! them so a human can resolve the conflict in plain text (plan sync model).

/// Substrings that mark a path as a cloud-sync conflict copy.
///
/// Covers the common OneDrive (`-DESKTOP-…`, `-LAPTOP-…`) and Dropbox / generic
/// (`(conflicted copy …)`, `conflicted copy`) conventions. Matching is case-insensitive.
pub const CONFLICT_MARKERS: &[&str] = &[
    "-desktop-",
    "-laptop-",
    "(conflicted copy",
    "conflicted copy",
    ".sync-conflict-",
];

/// Whether a vault-relative path looks like a cloud-sync conflict copy.
pub fn is_conflict_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    CONFLICT_MARKERS.iter().any(|m| file.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_onedrive_device_suffix() {
        assert!(is_conflict_path("notes/arch-DESKTOP-AB12CD.md"));
        assert!(is_conflict_path("queries-LAPTOP-99XY.md"));
    }

    #[test]
    fn detects_dropbox_and_syncthing() {
        assert!(is_conflict_path("a (conflicted copy 2026-01-02).md"));
        assert!(is_conflict_path("b.sync-conflict-20260102-foo.md"));
    }

    #[test]
    fn is_case_insensitive() {
        assert!(is_conflict_path("Notes/Arch-Desktop-Ab12.md"));
    }

    #[test]
    fn normal_files_are_not_conflicts() {
        assert!(!is_conflict_path("notes/architecture.md"));
        assert!(!is_conflict_path("useful-queries.md"));
        assert!(!is_conflict_path("topic/project-x.md"));
    }
}
