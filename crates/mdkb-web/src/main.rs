//! `mdkb-web` — a local web UI for mdkb.
//!
//! A **thin client**: it serves HTML built by the shared `mdkb-view` layer and gets all
//! data from the daemon via `mdkb-protocol`. It implements no knowledge-base behavior. This
//! is the "or similar" companion to a future Tauri shell — both render via `mdkb-view`, so
//! they cannot diverge (see `AGENTS.md`).

mod http;
mod routes;

use std::io::Write;
use std::net::TcpListener;
use std::process::ExitCode;

use mdkb_core::SearchQuery;
use mdkb_protocol::{Client, DaemonPaths, Request, Response};
use mdkb_view::ResultRow;

use routes::Backend;

/// Adapts the daemon client to the UI's [`Backend`] needs.
struct DaemonBackend {
    client: Client,
}

impl Backend for DaemonBackend {
    fn list_pages(&self) -> Result<Vec<String>, String> {
        match self
            .client
            .call(&Request::ListPages)
            .map_err(|e| e.to_string())?
        {
            Response::Pages(p) => Ok(p),
            Response::Error { message } => Err(message),
            other => Err(format!("unexpected response: {other:?}")),
        }
    }

    fn render_page(&self, page: &str) -> Result<Option<String>, String> {
        self.client.render_page(page).map_err(|e| e.to_string())
    }

    fn search(&self, query: &str) -> Result<Vec<ResultRow>, String> {
        let q = SearchQuery {
            text: Some(query.to_string()),
            ..Default::default()
        };
        let hits = self.client.search(q).map_err(|e| e.to_string())?;
        Ok(hits
            .into_iter()
            .map(|h| ResultRow {
                page_path: h.block.page_path,
                id: h.block.id.to_string(),
                lineage: h.block.lineage,
                content: h.block.content,
            })
            .collect())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mdkb-web: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "mdkb-web {} — local web UI for mdkb\n\n\
usage:\n  mdkb-web [--socket <path>] [--vault <dir>] [--addr <host:port>]\n\n\
Serves the knowledge base over HTTP, rendering via mdkb-view and reading from a running\n\
mdkbd. Default address: 127.0.0.1:7878.",
        env!("CARGO_PKG_VERSION")
    );
}

fn run(args: &[String]) -> Result<(), String> {
    let mut socket = None;
    let mut vault = None;
    let mut addr = "127.0.0.1:7878".to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => socket = it.next().cloned(),
            "--vault" => vault = it.next().cloned(),
            "--addr" => addr = it.next().cloned().ok_or("--addr requires a value")?,
            other => {
                if let Some(v) = other.strip_prefix("--socket=") {
                    socket = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--vault=") {
                    vault = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--addr=") {
                    addr = v.to_string();
                } else {
                    return Err(format!("unknown argument: {other}"));
                }
            }
        }
    }

    let socket_path = match socket {
        Some(s) => std::path::PathBuf::from(s),
        None => {
            let vault = vault
                .map(std::path::PathBuf::from)
                .unwrap_or_else(DaemonPaths::default_vault);
            DaemonPaths::from_vault(vault).socket
        }
    };

    let client = Client::new(&socket_path);
    if !client.ping() {
        return Err(format!(
            "no daemon reachable on {} — start mdkbd first",
            socket_path.display()
        ));
    }
    let backend = DaemonBackend { client };

    let listener = TcpListener::bind(&addr).map_err(|e| format!("binding {addr}: {e}"))?;
    eprintln!(
        "mdkb-web: serving http://{addr} (daemon: {})",
        socket_path.display()
    );

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("mdkb-web: accept error: {e}");
                continue;
            }
        };
        let response = match http::read_request(&mut stream) {
            Ok(Some(req)) => routes::handle(&backend, &req),
            Ok(None) => continue,
            Err(e) => {
                eprintln!("mdkb-web: read error: {e}");
                continue;
            }
        };
        if let Err(e) = stream.write_all(&response.to_bytes()) {
            eprintln!("mdkb-web: write error: {e}");
        }
    }
    Ok(())
}
