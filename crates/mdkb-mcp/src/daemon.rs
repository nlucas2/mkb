//! Connecting to the mdkb daemon, auto-starting it if necessary.
//!
//! The MCP server must talk to a running `mdkbd` (the single index owner). If none is live
//! on the configured socket, we spawn one and wait for it to come up. This keeps the MCP
//! server a pure client while still being convenient to launch standalone.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use mdkb_protocol::{Client, DaemonPaths};

/// Ensure a daemon is reachable on `paths.socket`, spawning `mdkbd` if needed. Returns a
/// connected [`Client`].
pub fn ensure_daemon(paths: &DaemonPaths) -> Result<Client, String> {
    let client = Client::new(&paths.socket);
    if client.ping() {
        return Ok(client);
    }
    spawn_daemon(paths)?;
    wait_until_ready(&client, Duration::from_secs(30))?;
    Ok(client)
}

fn spawn_daemon(paths: &DaemonPaths) -> Result<(), String> {
    let exe = mdkbd_path()?;
    eprintln!(
        "mdkb-mcp: starting daemon: {} --vault {}",
        exe.display(),
        paths.vault.display()
    );
    Command::new(&exe)
        .arg("--vault")
        .arg(&paths.vault)
        .arg("--socket")
        .arg(&paths.socket)
        .arg("--db")
        .arg(&paths.db)
        .spawn()
        .map_err(|e| format!("failed to spawn {}: {e}", exe.display()))?;
    Ok(())
}

/// Locate the `mdkbd` binary: prefer one beside this executable, else rely on `PATH`.
fn mdkbd_path() -> Result<std::path::PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(mdkbd_filename());
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    // Fall back to PATH resolution.
    Ok(std::path::PathBuf::from("mdkbd"))
}

fn mdkbd_filename() -> &'static str {
    if cfg!(windows) {
        "mdkbd.exe"
    } else {
        "mdkbd"
    }
}

fn wait_until_ready(client: &Client, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if client.ping() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "daemon did not become ready within {}s on {}",
        timeout.as_secs(),
        client.socket().display()
    ))
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
        assert_eq!(
            p.socket,
            std::path::PathBuf::from("/tmp/v/.mdkb/mdkbd.sock")
        );
        let p2 =
            paths_from_args(&["--vault=/tmp/v".into(), "--socket=/run/x.sock".into()]).unwrap();
        assert_eq!(p2.socket, std::path::PathBuf::from("/run/x.sock"));
    }

    #[test]
    fn paths_from_args_rejects_unknown() {
        assert!(paths_from_args(&["--bogus".into()]).is_err());
    }
}
