//! Per-vault exclusive lock so **at most one daemon ever owns a vault**.
//!
//! The socket guard in [`crate::run`] handles the friendly "already running" fast-path and
//! cleans up a stale socket left by a previous run. But that guard keys off the socket *file*
//! existing, so it can be defeated: if the live socket file is deleted out from under a running
//! daemon, a newly launched daemon sees no socket and would happily bind a **second** daemon on
//! the same vault — two watchers, two writers, corruption risk.
//!
//! An advisory file lock closes that hole. The lock is held for the daemon's entire lifetime and
//! released **by the OS automatically on exit** — even on crash or `kill` — so there is no
//! stale-PID-file problem to clean up. Combined with the socket guard, it makes a same-vault
//! double-spawn impossible regardless of socket-file state.

use std::fs::{File, OpenOptions};
use std::path::Path;

/// An exclusive lock on a vault, held for as long as this value lives (the underlying file
/// descriptor stays open, so the kernel keeps the lock). Dropping it — or the process exiting —
/// releases the lock.
#[derive(Debug)]
pub struct VaultLock {
    // Kept solely to hold the descriptor (and thus the lock) open. Never read.
    _file: File,
}

impl VaultLock {
    /// Try to acquire the exclusive lock at `path` (e.g. `<vault>/.mkb/mkbd.lock`).
    ///
    /// Returns `Ok(Some(lock))` when acquired, `Ok(None)` when another live process already
    /// holds it, and `Err` only on an unexpected I/O failure.
    pub fn acquire(path: &Path) -> std::io::Result<Option<VaultLock>> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        if try_lock_exclusive(&file)? {
            Ok(Some(VaultLock { _file: file }))
        } else {
            Ok(None)
        }
    }
}

#[cfg(unix)]
fn try_lock_exclusive(file: &File) -> std::io::Result<bool> {
    use std::os::unix::io::AsRawFd;
    // LOCK_EX | LOCK_NB: take an exclusive lock, but never block — fail fast if someone holds it.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    // EWOULDBLOCK (== EAGAIN) means another open file description holds the lock — that is a
    // "not acquired", not a hard error.
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        return Ok(false);
    }
    Err(err)
}

#[cfg(not(unix))]
fn try_lock_exclusive(_file: &File) -> std::io::Result<bool> {
    // Non-Unix best-effort: the socket guard in `run` remains the primary protection. Treat the
    // lock as acquired so the daemon still starts.
    Ok(true)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_is_blocked_until_first_is_dropped() {
        let dir = std::env::temp_dir().join(format!("mkb-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mkbd.lock");

        let first = VaultLock::acquire(&path).unwrap();
        assert!(first.is_some(), "first acquire should succeed");

        let second = VaultLock::acquire(&path).unwrap();
        assert!(second.is_none(), "second acquire must fail while held");

        drop(first);
        let third = VaultLock::acquire(&path).unwrap();
        assert!(third.is_some(), "acquire should succeed after release");

        drop(third);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
