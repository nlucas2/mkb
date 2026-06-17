//! The file watcher: keeps the index reconciled with external edits to the vault.
//!
//! Uses `notify` to observe the vault directory. Events are debounced; after a quiet
//! period the watcher reconciles the whole tree (a hash-skip operation, so unchanged files
//! cost nothing). Reconcile is idempotent — id assignment settles after one pass — so the
//! watcher's own writes don't loop.

use std::path::PathBuf;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use mdkb_core::RequestContext;
use notify::{recommended_watcher, RecursiveMode, Watcher};

use crate::SharedService;

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Spawn the watcher thread. It owns the `notify` watcher for its lifetime.
pub fn spawn(vault: PathBuf, service: SharedService) {
    thread::spawn(move || {
        if let Err(e) = run(vault, service) {
            eprintln!("mdkbd: watcher stopped: {e}");
        }
    });
}

fn run(vault: PathBuf, service: SharedService) -> Result<(), String> {
    let (tx, rx) = channel();
    let mut watcher = recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(&vault, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;

    let ctx = RequestContext::local();
    loop {
        // Block until the first event, then drain a debounce window so a burst of edits
        // collapses into a single reconcile.
        match rx.recv() {
            Ok(_) => {}
            Err(_) => return Ok(()), // sender dropped; watcher gone
        }
        loop {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
        let mut guard = service.lock().unwrap_or_else(|p| p.into_inner());
        match guard.reconcile(&ctx) {
            Ok(report) if !report.is_empty() => {
                eprintln!(
                    "mdkbd: watcher reconcile — {} changed, {} removed",
                    report.changed.len(),
                    report.removed.len()
                );
            }
            Ok(_) => {}
            Err(e) => eprintln!("mdkbd: watcher reconcile error: {e}"),
        }
    }
}
