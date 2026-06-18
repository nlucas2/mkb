//! Detection of cloud-sync conflict files.
//!
//! When a vault is synced across machines (OneDrive, Dropbox, …) and the same file is
//! edited in two places, the sync engine spawns a *conflict copy* with a mangled name
//! rather than merging. mdkb must not silently index these — that would pollute search with
//! duplicate, possibly-stale content. Instead it detects them, skips indexing, and surfaces
//! them so a human can resolve the conflict in plain text (plan sync model).

use std::sync::OnceLock;

use regex::Regex;

/// Substrings that unambiguously mark a path as a cloud-sync conflict copy. These are
/// distinctive enough that a real note title is very unlikely to contain them verbatim.
pub const CONFLICT_MARKERS: &[&str] = &["(conflicted copy", "conflicted copy", ".sync-conflict-"];

/// OneDrive-style device-suffix conflicts: `<name>-<DEVICE>-<token>.md`, where the daemon
/// adds an uppercase device name and an alphanumeric token before the extension. Anchored
/// to that shape (uppercase + trailing token + extension) so ordinary titles like
/// `office-laptop-setup.md` are **not** misclassified.
fn device_suffix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"-(?:DESKTOP|LAPTOP|PC|MACBOOK)-[A-Z0-9]{4,}\.[A-Za-z0-9]+$")
            .expect("valid device-suffix regex")
    })
}

/// Whether a vault-relative path looks like a cloud-sync conflict copy.
pub fn is_conflict_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    let lower = file.to_lowercase();
    if CONFLICT_MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    // Device-suffix check is case-sensitive (real markers are uppercase) and anchored.
    device_suffix_re().is_match(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_onedrive_device_suffix() {
        assert!(is_conflict_path("notes/arch-DESKTOP-AB12CD.md"));
        assert!(is_conflict_path("queries-LAPTOP-99XY.md"));
        assert!(is_conflict_path("plan-MACBOOK-7F3K2.md"));
    }

    #[test]
    fn detects_dropbox_and_syncthing() {
        assert!(is_conflict_path("a (conflicted copy 2026-01-02).md"));
        assert!(is_conflict_path("b.sync-conflict-20260102-foo.md"));
    }

    #[test]
    fn does_not_misclassify_legitimate_titles() {
        // Regression: bare "-laptop-"/"-desktop-" substrings must NOT trigger.
        assert!(!is_conflict_path("office-laptop-setup.md"));
        assert!(!is_conflict_path("sit-stand-desktop-guide.md"));
        assert!(!is_conflict_path("my-pc-build-notes.md"));
        // Lowercase device-like token isn't the OneDrive shape.
        assert!(!is_conflict_path("notes-desktop-tips.md"));
    }

    #[test]
    fn normal_files_are_not_conflicts() {
        assert!(!is_conflict_path("notes/architecture.md"));
        assert!(!is_conflict_path("useful-queries.md"));
        assert!(!is_conflict_path("topic/project-x.md"));
    }
}
