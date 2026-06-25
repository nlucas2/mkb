//! `mdkb-mcp` — the mdkb MCP server.
//!
//! A **thin client**: it speaks MCP (JSON-RPC 2.0 over stdio) and forwards every tool call
//! to the daemon, which owns the one shared `Service`. It implements no knowledge-base
//! behavior of its own. See `AGENTS.md`.

mod rpc;
mod tools;

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use mdkb_protocol::{resolve_client, ClientInputs};
use rpc::{handle_message, Outcome};

/// MCP server CLI. Pin it to a specific vault (`--vault`, or `$MDKB_VAULT`) so an agent only ever
/// sees that one knowledge base; otherwise it falls back to the configured registry default. A
/// remote daemon (`--remote`/`--socket`) is also supported for split agent/daemon setups.
#[derive(Parser)]
#[command(
    name = "mdkb-mcp",
    version,
    about = "MCP server for mdkb (stdio) — a thin client that forwards tool calls to the daemon",
    long_about = "Speaks MCP over stdin/stdout and forwards tool calls to mdkbd (auto-started for \
                  a local vault). Pin the vault with --vault (or $MDKB_VAULT) so an agent only \
                  acts on that knowledge base; otherwise the configured registry default is used."
)]
struct Cli {
    /// Vault directory to serve (supports a leading `~`). Overrides $MDKB_VAULT and the registry
    /// default — pin this so an agent can't act on the wrong vault.
    #[arg(long, value_name = "DIR")]
    vault: Option<PathBuf>,
    /// Connect to a remote daemon `host:port` over TCP instead of a local vault.
    #[arg(long, value_name = "HOST:PORT")]
    remote: Option<String>,
    /// Token to present to a remote daemon.
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
    /// Dial this explicit local socket instead of deriving one from the vault.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mdkb-mcp: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let inputs = ClientInputs {
        vault: cli.vault,
        remote: cli.remote,
        token: cli.token,
        socket: cli.socket,
    };
    // Resolve + connect via the shared layer (flag > env > registry default > builtin), which
    // auto-starts a local daemon if needed — same path every client uses.
    let client = resolve_client(&inputs, None)?;
    eprintln!("mdkb-mcp: connected to daemon on {}", client.endpoint());

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                // Malformed JSON: emit a parse error with null id, per JSON-RPC.
                let err = serde_json::json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": format!("parse error: {e}")}
                });
                write_line(&mut out, &err)?;
                continue;
            }
        };
        if let Outcome::Reply(reply) = handle_message(&client, &msg) {
            write_line(&mut out, &reply)?;
        }
    }
    Ok(())
}

fn write_line(out: &mut impl Write, value: &serde_json::Value) -> Result<(), String> {
    let s = serde_json::to_string(value).map_err(|e| e.to_string())?;
    out.write_all(s.as_bytes()).map_err(|e| e.to_string())?;
    out.write_all(b"\n").map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    Ok(())
}
