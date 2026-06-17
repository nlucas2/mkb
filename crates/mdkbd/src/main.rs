//! `mdkbd` — the mdkb headless daemon.
//!
//! Owns the single source of truth wiring: a [`SyncEngine`] over a SQLite index and the
//! Markdown vault, a file watcher that keeps the index reconciled, and a Unix-socket server
//! that answers [`mdkb_protocol`] requests by dispatching to the shared
//! [`mdkb_core::Service`]. The daemon binds a **local** Unix socket only — never the network
//! — so it is fail-closed by default (plan Decision #9).

mod config;
mod server;
mod watcher;

use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use mdkb_core::{Service, SyncEngine};
use mdkb_index::SqliteIndex;

use config::Config;

/// A service guarded for shared access by the server and watcher threads.
pub type SharedService = Arc<Mutex<Service<SqliteIndex>>>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mdkbd: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let cfg = Config::from_args(std::env::args().skip(1))?;
    if cfg.help {
        println!("{}", Config::usage());
        return Ok(());
    }

    cfg.ensure_dirs().map_err(|e| e.to_string())?;

    // Refuse to start a second daemon on the same socket.
    if cfg.socket().exists() {
        let probe = mdkb_protocol::Client::new(cfg.socket());
        if probe.ping() {
            return Err(format!(
                "a daemon is already running on {}",
                cfg.socket().display()
            ));
        }
        // Stale socket from a previous run; remove it.
        let _ = std::fs::remove_file(cfg.socket());
    }

    eprintln!(
        "mdkbd: vault={} db={} socket={}",
        cfg.vault().display(),
        cfg.db().display(),
        cfg.socket().display()
    );

    let index = SqliteIndex::open(cfg.db()).map_err(|e| e.to_string())?;
    let engine = SyncEngine::new(cfg.vault(), index).with_embedder(mdkb_embed::recommended());
    let mut service = Service::new(engine);

    // Initial reconcile so the index reflects the vault before serving.
    let ctx = mdkb_core::RequestContext::local();
    let report = service.reconcile(&ctx).map_err(|e| e.to_string())?;
    eprintln!(
        "mdkbd: initial reconcile — {} changed, {} removed",
        report.changed.len(),
        report.removed.len()
    );

    let shared: SharedService = Arc::new(Mutex::new(service));

    // Start the watcher (keeps the index in sync with external edits).
    watcher::spawn(cfg.vault().to_path_buf(), Arc::clone(&shared));

    // Serve requests until interrupted.
    server::serve(cfg.socket(), shared).map_err(|e| e.to_string())?;
    Ok(())
}
