//! Connecting to the mdkb daemon, auto-starting it if necessary.
//!
//! The MCP server must talk to a running `mdkbd` (the single index owner). The actual
//! "ping-or-spawn-detached" logic lives in [`mdkb_protocol::ensure_daemon`] so every client
//! (this server, the desktop app, the CLI) starts the daemon identically — see the
//! "no divergence" rule in `AGENTS.md`. This module only adds the MCP-specific CLI parsing.

use std::path::Path;

use mdkb_protocol::{Client, DaemonPaths};

/// Ensure a daemon is reachable on `paths.socket`, spawning a detached `mdkbd` if needed.
pub fn ensure_daemon(paths: &DaemonPaths) -> Result<Client, String> {
    mdkb_protocol::ensure_daemon(paths, None)
}

/// Resolve daemon paths from CLI args / env (mirrors `mdkbd`'s flags).
pub fn paths_from_args(args: &[String]) -> Result<DaemonPaths, String> {
    let mut vault = None;
    let mut socket = None;
    let mut db = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--vault" => vault = it.next().cloned(),
            "--socket" => socket = it.next().cloned(),
            "--db" => db = it.next().cloned(),
            other => {
                if let Some(v) = other.strip_prefix("--vault=") {
                    vault = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--socket=") {
                    socket = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--db=") {
                    db = Some(v.to_string());
                } else {
                    return Err(format!("unknown argument: {other}"));
                }
            }
        }
    }
    let vault = vault
        .map(std::path::PathBuf::from)
        .unwrap_or_else(DaemonPaths::default_vault);
    let mut paths = DaemonPaths::from_vault(vault);
    if let Some(s) = socket {
        paths.socket = Path::new(&s).to_path_buf();
    }
    if let Some(d) = db {
        paths.db = Path::new(&d).to_path_buf();
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_from_args_defaults_and_overrides() {
        let p = paths_from_args(&["--vault".into(), "/tmp/v".into()]).unwrap();
        // Default socket lands in the machine-local per-vault dir (resolved by DaemonPaths), not in
        // the vault; an explicit --socket overrides it.
        assert_eq!(p.socket.file_name().unwrap(), "mdkbd.sock");
        let p2 =
            paths_from_args(&["--vault=/tmp/v".into(), "--socket=/run/x.sock".into()]).unwrap();
        assert_eq!(p2.socket, std::path::PathBuf::from("/run/x.sock"));
    }

    #[test]
    fn paths_from_args_rejects_unknown() {
        assert!(paths_from_args(&["--bogus".into()]).is_err());
    }
}
