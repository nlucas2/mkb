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

use clap::{Args, Parser};

use mdkb_core::SearchQuery;
use mdkb_protocol::{
    resolve_target, Client, ClientInputs, DaemonPaths, EnvSnapshot, Registry, ResolvedTarget,
};
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

/// Connection options (shared seam with the other clients): which daemon to read from. These are
/// the *explicit* inputs; `$MDKB_VAULT`/`$MDKB_REMOTE`/`$MDKB_TOKEN`/`$MDKB_SOCKET` and the
/// registry default are applied by the shared resolver.
#[derive(Args, Debug, Default)]
struct ConnArgs {
    /// Vault directory whose daemon to read from (supports a leading `~`).
    #[arg(long, value_name = "DIR")]
    vault: Option<String>,
    /// Remote daemon `host:port` to read from instead of a local vault.
    #[arg(long, value_name = "HOST:PORT")]
    remote: Option<String>,
    /// Token to present to a remote daemon.
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
    /// Explicit local socket to dial.
    #[arg(long, value_name = "PATH")]
    socket: Option<String>,
}

#[derive(Parser)]
#[command(
    name = "mdkb-web",
    version,
    about = "Local web UI for mdkb — a thin HTTP server over a running daemon",
    long_about = "Serves the knowledge base over HTTP, rendering via mdkb-view and reading from a \
                  running mdkbd. The daemon is chosen by --vault/--socket/--remote (else \
                  $MDKB_VAULT/$MDKB_SOCKET/$MDKB_REMOTE+$MDKB_TOKEN, else the registry default). \
                  Unlike the CLI, the web UI does not auto-start a daemon — start mdkbd first."
)]
struct Cli {
    #[command(flatten)]
    conn: ConnArgs,

    /// Address to bind the web UI.
    #[arg(long, default_value = "127.0.0.1:7878", value_name = "HOST:PORT")]
    addr: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mdkb-web: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let inputs = ClientInputs {
        vault: cli.conn.vault.map(Into::into),
        remote: cli.conn.remote,
        token: cli.conn.token,
        socket: cli.conn.socket.map(Into::into),
    };
    // Resolve *where* to connect via the shared precedence (flag > env > registry default >
    // builtin), but connect without auto-starting: the web UI is a long-running server that reads
    // from an already-running daemon (start mdkbd first), so a LocalVault target maps to that
    // vault's socket rather than spawning a daemon with an idle timeout.
    let env = EnvSnapshot::read();
    let registry_default = Registry::load().default_connection();
    let target = resolve_target(&inputs, &env, Some(&registry_default))?;
    let client = match target {
        ResolvedTarget::Remote { host, token } => Client::tcp(host, token),
        ResolvedTarget::LocalSocket { socket } => Client::new(socket),
        ResolvedTarget::LocalVault { vault } => Client::new(DaemonPaths::from_vault(vault).socket),
    };

    if !client.ping() {
        return Err(format!(
            "no daemon reachable at {} — start mdkbd first",
            client.endpoint()
        ));
    }
    let endpoint = client.endpoint();
    let backend = DaemonBackend { client };

    let addr = cli.addr;
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
