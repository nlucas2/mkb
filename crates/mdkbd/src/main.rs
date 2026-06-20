//! `mdkbd` — the mdkb headless daemon.
//!
//! Owns the single source of truth wiring: a [`SyncEngine`] over a SQLite index and the
//! Markdown vault, a file watcher that keeps the index reconciled, and a Unix-socket server
//! that answers [`mdkb_protocol`] requests by dispatching to the shared
//! [`mdkb_core::Service`]. The daemon binds a **local** Unix socket only — never the network
//! — so it is fail-closed by default (plan Decision #9).

mod config;
mod lock;
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

    // Acquire the per-vault exclusive lock FIRST. At most one daemon may own a vault; this is
    // held for the whole process lifetime and released by the OS on exit (even crash/kill), so
    // it can't go stale. It also makes the socket checks below authoritative: while we hold the
    // lock, no other live daemon can exist for this vault, so any socket file present is stale.
    let lock_path = cfg.paths.mdkb_dir().join("mdkbd.lock");
    let _vault_lock = match lock::VaultLock::acquire(&lock_path) {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            return Err(format!(
                "a daemon already owns vault {} (lock held on {})",
                cfg.vault().display(),
                lock_path.display()
            ));
        }
        Err(e) => {
            return Err(format!("acquiring vault lock {}: {e}", lock_path.display()));
        }
    };

    // Refuse to start a second daemon on the same socket.
    if cfg.socket().exists() {
        let probe = mdkb_protocol::Client::new(cfg.socket());
        if probe.ping() {
            return Err(format!(
                "a daemon is already running on {}",
                cfg.socket().display()
            ));
        }
        // Stale socket from a previous run (we hold the vault lock, so no live peer exists).
        let _ = std::fs::remove_file(cfg.socket());
    }

    eprintln!(
        "mdkbd: vault={} db={} socket={}",
        cfg.vault().display(),
        cfg.db().display(),
        cfg.socket().display()
    );

    let index = SqliteIndex::open(cfg.db()).map_err(|e| e.to_string())?;
    let source = mdkb_embed::FileConfig::load(cfg.paths.mdkb_dir()).embedder;
    eprintln!("mdkbd: embedder source = {source:?}");
    let engine =
        SyncEngine::new(cfg.vault(), index).with_embedder(mdkb_embed::from_source(&source));
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

    // Optional network listener (opt-in, token-gated, fail-closed).
    let net = cfg.listen.as_ref().map(|addr| server::NetConfig {
        addr: addr.clone(),
        token: cfg.token.clone().unwrap_or_default(),
    });

    // Serve requests until interrupted.
    server::serve(cfg.socket(), net, shared).map_err(|e| e.to_string())?;
    Ok(())
}
