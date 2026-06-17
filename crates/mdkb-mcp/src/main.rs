//! `mdkb-mcp` — the mdkb MCP server.
//!
//! A **thin client**: it speaks MCP (JSON-RPC 2.0 over stdio) and forwards every tool call
//! to the daemon, which owns the one shared `Service`. It implements no knowledge-base
//! behavior of its own. See `AGENTS.md`.

mod daemon;
mod rpc;
mod tools;

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use rpc::{handle_message, Outcome};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mdkb-mcp: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "mdkb-mcp {} — MCP server for mdkb (stdio)\n\n\
usage:\n  mdkb-mcp [--vault <dir>] [--socket <path>] [--db <path>]\n\n\
Speaks MCP over stdin/stdout and forwards tool calls to mdkbd (auto-started if needed).",
        env!("CARGO_PKG_VERSION")
    );
}

fn run(args: &[String]) -> Result<(), String> {
    let paths = daemon::paths_from_args(args)?;
    paths.ensure_dirs().map_err(|e| e.to_string())?;
    let client = daemon::ensure_daemon(&paths)?;
    eprintln!(
        "mdkb-mcp: connected to daemon on {}",
        paths.socket.display()
    );

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
