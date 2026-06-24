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
use mdkb_protocol::{Client, DaemonPaths};
use mdkb_view::{block_title, NavEntry, ResultRow};

use routes::Backend;

/// Adapts the daemon client to the UI's [`Backend`] needs.
struct DaemonBackend {
    client: Client,
}

impl Backend for DaemonBackend {
    fn nav_entries(&self) -> Result<Vec<NavEntry>, String> {
        let roots = self.client.list_roots().map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for id in roots {
            let title = self
                .client
                .get_block(id.clone())
                .map_err(|e| e.to_string())?
                .map(|b| block_title(b.title.as_deref(), &b.content))
                .unwrap_or_else(|| id.to_string());
            entries.push(NavEntry {
                id: id.to_string(),
                title,
            });
        }
        Ok(entries)
    }

    fn render_block(&self, id: &str) -> Result<Option<(String, String)>, String> {
        let bid = match mdkb_core::BlockId::parse(id) {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        let rendered = self.client.rendered_block(bid).map_err(|e| e.to_string())?;
        Ok(rendered.map(|rb| (rb.title, rb.rendered)))
    }

    fn search(&self, query: &str) -> Result<Vec<ResultRow>, String> {
        // Parse the same inline operators (tag:/#tag/lang:/created:/updated:) the CLI and app use,
        // so the web search box behaves identically rather than treating operators as plain text.
        let q = SearchQuery::parse(query);
        let hits = self.client.search(q).map_err(|e| e.to_string())?;
        Ok(hits
            .into_iter()
            .map(|h| ResultRow {
                id: h.block.id.to_string(),
                title: block_title(h.block.title.as_deref(), &h.block.content),
                tags: h.block.tags,
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
usage:\n  mdkb-web [--socket <path>] [--vault <dir>] [--addr <host:port>]\n  \
mdkb-web --remote <host:port> --token <tok> [--addr <host:port>]\n\n\
Serves the knowledge base over HTTP, rendering via mdkb-view and reading from a running\n\
mdkbd — either a local Unix socket or a remote TCP daemon (--remote/$MDKB_REMOTE +\n\
--token/$MDKB_TOKEN). Default listen address: 127.0.0.1:7878.",
        env!("CARGO_PKG_VERSION")
    );
}

fn run(args: &[String]) -> Result<(), String> {
    let mut socket = None;
    let mut vault = None;
    let mut remote = None;
    let mut token = None;
    let mut addr = "127.0.0.1:7878".to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => socket = it.next().cloned(),
            "--vault" => vault = it.next().cloned(),
            "--remote" => remote = it.next().cloned(),
            "--token" => token = it.next().cloned(),
            "--addr" => addr = it.next().cloned().ok_or("--addr requires a value")?,
            other => {
                if let Some(v) = other.strip_prefix("--socket=") {
                    socket = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--vault=") {
                    vault = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--remote=") {
                    remote = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--token=") {
                    token = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--addr=") {
                    addr = v.to_string();
                } else {
                    return Err(format!("unknown argument: {other}"));
                }
            }
        }
    }

    // Connect to a remote TCP daemon (--remote/$MDKB_REMOTE, token-gated) or a local socket.
    let remote = remote.or_else(|| std::env::var("MDKB_REMOTE").ok().filter(|s| !s.is_empty()));
    let client = if let Some(remote) = remote {
        let token = token
            .or_else(|| std::env::var("MDKB_TOKEN").ok())
            .filter(|s| !s.is_empty())
            .ok_or("--remote requires a token (--token or $MDKB_TOKEN)")?;
        Client::tcp(remote, token)
    } else {
        let socket_path = match socket {
            Some(s) => std::path::PathBuf::from(s),
            None => {
                let vault = vault
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(DaemonPaths::default_vault);
                DaemonPaths::from_vault(vault).socket
            }
        };
        Client::new(&socket_path)
    };

    if !client.ping() {
        return Err(format!(
            "no daemon reachable at {} — start mdkbd first",
            client.endpoint()
        ));
    }
    let endpoint = client.endpoint();
    let backend = DaemonBackend { client };

    let listener = TcpListener::bind(&addr).map_err(|e| format!("binding {addr}: {e}"))?;
    eprintln!("mdkb-web: serving http://{addr} (daemon: {endpoint})");

    let backend = std::sync::Arc::new(backend);
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("mdkb-web: accept error: {e}");
                continue;
            }
        };
        // Bound how long a connection may stall so one slow/half-open socket (browsers open
        // speculative ones) cannot wedge the server.
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(15)));
        let backend = std::sync::Arc::clone(&backend);
        std::thread::spawn(move || {
            let response = match http::read_request(&mut stream) {
                Ok(Some(req)) => routes::handle(backend.as_ref(), &req),
                Ok(None) => return,
                Err(e) => {
                    eprintln!("mdkb-web: read error: {e}");
                    return;
                }
            };
            if let Err(e) = stream.write_all(&response.to_bytes()) {
                eprintln!("mdkb-web: write error: {e}");
            }
        });
    }
    Ok(())
}
