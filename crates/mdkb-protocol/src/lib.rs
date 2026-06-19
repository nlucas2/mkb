//! The mdkb wire protocol: request/response types, a blocking client, and the shared
//! request dispatcher.
//!
//! Both ends speak newline-delimited JSON over a transport (a Unix socket today). The
//! [`dispatch`] function maps a [`Request`] onto a [`mdkb_core::Service`] and is the single
//! place request handling lives — the daemon is just transport glue around it, and any
//! other host (tests, an embedded server) reuses the exact same logic. This upholds the
//! "no divergence" rule in `AGENTS.md`.

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub mod paths;
pub mod transport;
pub use paths::DaemonPaths;

use mdkb_core::{
    BlockId, BlockRecord, Index, IndexStats, LinkRow, RequestContext, SearchHit, SearchQuery,
    Service,
};
use serde::{Deserialize, Serialize};

/// A request to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Liveness check.
    Ping,
    /// Index statistics.
    Stats,
    /// List page paths.
    ListPages,
    /// Search (keyword + semantic).
    Search {
        /// The query.
        query: SearchQuery,
    },
    /// Fetch a block by id.
    GetBlock {
        /// Block id.
        id: BlockId,
    },
    /// Raw Markdown source of a page.
    GetPageSource {
        /// Page path.
        page: String,
    },
    /// Render a page with transclusions resolved.
    RenderPage {
        /// Page path.
        page: String,
    },
    /// Outgoing links from a block.
    LinksFrom {
        /// Block id.
        id: BlockId,
    },
    /// Incoming references / transclusions of a block.
    Backlinks {
        /// Block id.
        id: BlockId,
    },
    /// Upsert a block (update by id, or append a new block to a page).
    UpsertBlock {
        /// Existing block id to update, or `None` to create.
        id: Option<BlockId>,
        /// The block text.
        text: String,
        /// Target page (required when creating).
        page: Option<String>,
    },
    /// Save (create/overwrite) a whole page.
    SavePage {
        /// Page path.
        page: String,
        /// Markdown source.
        source: String,
    },
    /// Delete a page.
    DeletePage {
        /// Page path.
        page: String,
    },
    /// Create a link/transclusion from one block to a target.
    LinkBlocks {
        /// Source block id.
        source_id: BlockId,
        /// Target page.
        target_page: Option<String>,
        /// Target block id.
        target_id: Option<BlockId>,
        /// Target heading anchor.
        target_anchor: Option<String>,
        /// `true` to embed (`![[...]]`), `false` to link (`[[...]]`).
        embed: bool,
    },
    /// Reconcile the vault directory with the index.
    Reconcile,
    /// Rebuild the entire index from the vault files (clear + re-ingest).
    Rebuild,
    /// List cloud-sync conflict files detected at the last reconcile.
    Conflicts,
    /// Authenticate this connection with a shared token (network transport).
    Authenticate {
        /// The shared token.
        token: String,
    },
}

/// A response from the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum Response {
    /// Reply to [`Request::Ping`].
    Pong,
    /// Index statistics.
    Stats(IndexStats),
    /// Page paths.
    Pages(Vec<String>),
    /// Search results.
    Hits(Vec<SearchHit>),
    /// A single (optional) block.
    Block(Option<BlockRecord>),
    /// Optional text (page source / rendered page).
    Text(Option<String>),
    /// Link rows.
    Links(Vec<LinkRow>),
    /// An affected block id (e.g. after upsert).
    BlockId(BlockId),
    /// Success with no payload.
    Ok,
    /// An error with a human-readable message.
    Error {
        /// The message.
        message: String,
    },
}

impl Response {
    /// Convenience: extract an error message if this is an [`Response::Error`].
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Response::Error { message } => Some(message),
            _ => None,
        }
    }
}

/// Map a request onto a service and produce a response. The single shared request handler.
pub fn dispatch<I: Index>(
    service: &mut Service<I>,
    ctx: &RequestContext,
    request: Request,
) -> Response {
    match handle(service, ctx, request) {
        Ok(resp) => resp,
        Err(message) => Response::Error { message },
    }
}

fn handle<I: Index>(
    service: &mut Service<I>,
    ctx: &RequestContext,
    request: Request,
) -> Result<Response, String> {
    let to_str = |e: mdkb_core::IndexError| e.to_string();
    Ok(match request {
        Request::Ping => Response::Pong,
        Request::Stats => Response::Stats(service.stats(ctx).map_err(to_str)?),
        Request::ListPages => Response::Pages(service.list_pages(ctx).map_err(to_str)?),
        Request::Search { query } => Response::Hits(service.search(ctx, query).map_err(to_str)?),
        Request::GetBlock { id } => Response::Block(service.get_block(ctx, &id).map_err(to_str)?),
        Request::GetPageSource { page } => {
            Response::Text(service.get_page_source(ctx, &page).map_err(to_str)?)
        }
        Request::RenderPage { page } => {
            Response::Text(service.render_page(ctx, &page).map_err(to_str)?)
        }
        Request::LinksFrom { id } => Response::Links(service.links_from(ctx, &id).map_err(to_str)?),
        Request::Backlinks { id } => Response::Links(service.backlinks(ctx, &id).map_err(to_str)?),
        Request::UpsertBlock { id, text, page } => Response::BlockId(
            service
                .upsert_block(ctx, id, &text, page.as_deref())
                .map_err(to_str)?,
        ),
        Request::SavePage { page, source } => {
            service.save_page(ctx, &page, &source).map_err(to_str)?;
            Response::Ok
        }
        Request::DeletePage { page } => {
            service.delete_page(ctx, &page).map_err(to_str)?;
            Response::Ok
        }
        Request::LinkBlocks {
            source_id,
            target_page,
            target_id,
            target_anchor,
            embed,
        } => {
            service
                .link_blocks(
                    ctx,
                    &source_id,
                    target_page.as_deref(),
                    target_id.as_ref(),
                    target_anchor.as_deref(),
                    embed,
                )
                .map_err(to_str)?;
            Response::Ok
        }
        Request::Reconcile => {
            service.reconcile(ctx).map_err(to_str)?;
            Response::Ok
        }
        Request::Rebuild => {
            service.rebuild(ctx).map_err(to_str)?;
            Response::Ok
        }
        Request::Conflicts => Response::Pages(service.conflicts(ctx).map_err(to_str)?),
        // Authentication is handled by the transport layer (the server upgrades the
        // connection's context); reaching dispatch means it was used out of place.
        Request::Authenticate { .. } => Response::Error {
            message: "authenticate is handled by the transport layer".to_string(),
        },
    })
}

/// Encode a request as a single newline-terminated JSON line.
pub fn encode_request(req: &Request) -> serde_json::Result<String> {
    Ok(format!("{}\n", serde_json::to_string(req)?))
}

/// Encode a response as a single newline-terminated JSON line.
pub fn encode_response(resp: &Response) -> serde_json::Result<String> {
    Ok(format!("{}\n", serde_json::to_string(resp)?))
}

/// Decode a request from a JSON line.
pub fn decode_request(line: &str) -> serde_json::Result<Request> {
    serde_json::from_str(line.trim())
}

/// Decode a response from a JSON line.
pub fn decode_response(line: &str) -> serde_json::Result<Response> {
    serde_json::from_str(line.trim())
}

/// Write a request (newline-terminated) to a stream and flush.
fn send(writer: &mut impl Write, request: &Request) -> io::Result<()> {
    writer.write_all(encode_request(request)?.as_bytes())?;
    writer.flush()
}

/// Read exactly one newline-delimited response from a buffered reader.
fn read_one(reader: &mut impl BufRead) -> io::Result<Response> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty response from daemon",
        ));
    }
    Ok(serde_json::from_str(line.trim_end())?)
}

/// The transport a [`Client`] uses to reach the daemon.
#[derive(Debug, Clone)]
enum Transport {
    /// Local socket (trusted): a Unix-domain socket on Unix, a named pipe on Windows.
    Local(PathBuf),
    /// TCP with a shared auth token presented on each connection.
    Tcp { addr: String, token: String },
}

/// A blocking client over a [`Transport`].
///
/// One request per connection (simple and robust). Programs against the `Transport` enum so
/// the same surface works locally (Unix socket) or over the network (TCP + token).
#[derive(Debug, Clone)]
pub struct Client {
    transport: Transport,
}

impl Client {
    /// Point a client at a daemon local socket path (Unix socket / Windows named pipe).
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Client {
            transport: Transport::Local(socket.into()),
        }
    }

    /// Point a client at a daemon TCP address, authenticating with `token`.
    pub fn tcp(addr: impl Into<String>, token: impl Into<String>) -> Self {
        Client {
            transport: Transport::Tcp {
                addr: addr.into(),
                token: token.into(),
            },
        }
    }

    /// The socket path, if this is a local-socket client.
    pub fn socket(&self) -> &Path {
        match &self.transport {
            Transport::Local(p) => p,
            Transport::Tcp { .. } => Path::new(""),
        }
    }

    /// A short human description of where this client connects (for logs/UI).
    pub fn endpoint(&self) -> String {
        match &self.transport {
            Transport::Local(p) => format!("local:{}", p.display()),
            Transport::Tcp { addr, .. } => format!("tcp:{addr}"),
        }
    }

    /// Resolve a client from the environment, so every UI connects the same way:
    ///
    /// - `MDKB_REMOTE=host:port` (+ `MDKB_TOKEN`) → a **remote TCP** client (token required).
    /// - else `MDKB_SOCKET=/path` → that Unix socket.
    /// - else the local socket for `MDKB_VAULT` (or the default vault).
    ///
    /// This is the single connection-resolution path shared by the desktop app, the web UI,
    /// and any other client, so they cannot drift apart.
    pub fn from_env() -> Result<Client, String> {
        if let Some(remote) = std::env::var_os("MDKB_REMOTE") {
            let remote = remote.to_string_lossy().trim().to_string();
            if !remote.is_empty() {
                let token = std::env::var("MDKB_TOKEN").unwrap_or_default();
                if token.is_empty() {
                    return Err(
                        "MDKB_REMOTE is set but MDKB_TOKEN is empty; a remote daemon requires a token"
                            .to_string(),
                    );
                }
                return Ok(Client::tcp(remote, token));
            }
        }
        if let Some(sock) = std::env::var_os("MDKB_SOCKET") {
            let sock = sock.to_string_lossy().to_string();
            if !sock.trim().is_empty() {
                return Ok(Client::new(PathBuf::from(sock)));
            }
        }
        Ok(Client::new(DaemonPaths::for_default_vault().socket))
    }

    /// Send one request and read one response.
    pub fn call(&self, request: &Request) -> io::Result<Response> {
        match &self.transport {
            Transport::Local(socket) => {
                let stream = transport::connect_local(socket)?;
                let mut writer = &stream;
                let mut reader = BufReader::new(&stream);
                send(&mut writer, request)?;
                read_one(&mut reader)
            }
            Transport::Tcp { addr, token } => {
                let stream = std::net::TcpStream::connect(addr)?;
                let mut writer = stream.try_clone()?;
                // A single reader for the whole connection so no response bytes are lost
                // between the auth handshake and the actual request.
                let mut reader = BufReader::new(stream);
                send(
                    &mut writer,
                    &Request::Authenticate {
                        token: token.clone(),
                    },
                )?;
                if let Response::Error { message } = read_one(&mut reader)? {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, message));
                }
                send(&mut writer, request)?;
                read_one(&mut reader)
            }
        }
    }

    /// Convenience: search.
    pub fn search(&self, query: SearchQuery) -> io::Result<Vec<SearchHit>> {
        match self.call(&Request::Search { query })? {
            Response::Hits(h) => Ok(h),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: stats.
    pub fn stats(&self) -> io::Result<IndexStats> {
        match self.call(&Request::Stats)? {
            Response::Stats(s) => Ok(s),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: render a page.
    pub fn render_page(&self, page: &str) -> io::Result<Option<String>> {
        match self.call(&Request::RenderPage {
            page: page.to_string(),
        })? {
            Response::Text(t) => Ok(t),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: ping; returns true if the daemon answered.
    pub fn ping(&self) -> bool {
        matches!(self.call(&Request::Ping), Ok(Response::Pong))
    }
}

fn unexpected(resp: Response) -> io::Error {
    let msg = resp
        .error_message()
        .map(|m| m.to_string())
        .unwrap_or_else(|| format!("unexpected response: {resp:?}"));
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdkb_core::SyncEngine;
    use mdkb_index::SqliteIndex;

    fn service() -> (tempfile::TempDir, Service<SqliteIndex>) {
        let dir = tempfile::tempdir().unwrap();
        let engine = SyncEngine::new(dir.path(), SqliteIndex::open_in_memory().unwrap());
        (dir, Service::new(engine))
    }

    #[test]
    fn request_response_json_round_trip() {
        let req = Request::UpsertBlock {
            id: None,
            text: "hello".into(),
            page: Some("a.md".into()),
        };
        let line = encode_request(&req).unwrap();
        let decoded: Request = serde_json::from_str(line.trim_end()).unwrap();
        assert!(matches!(decoded, Request::UpsertBlock { .. }));
    }

    #[test]
    fn every_response_variant_serializes_and_round_trips() {
        // Guards against tagged-enum serialization pitfalls (e.g. sequences in newtype
        // variants) that would otherwise only surface as a runtime hang.
        let id = mdkb_core::BlockId::generate();
        let variants = vec![
            Response::Pong,
            Response::Stats(IndexStats::default()),
            Response::Pages(vec!["a.md".into(), "b.md".into()]),
            Response::Hits(vec![]),
            Response::Block(None),
            Response::Text(Some("hi".into())),
            Response::Links(vec![]),
            Response::BlockId(id),
            Response::Ok,
            Response::Error {
                message: "boom".into(),
            },
        ];
        for v in variants {
            let line = encode_response(&v).expect("response must serialize");
            let back = decode_response(&line).expect("response must deserialize");
            assert_eq!(
                std::mem::discriminant(&v),
                std::mem::discriminant(&back),
                "variant changed across round trip: {v:?}"
            );
        }
    }

    #[test]
    fn search_request_accepts_partial_query() {
        // A client should be able to send only the fields it cares about.
        let line = r#"{"op":"search","query":{"text":"index"}}"#;
        let req = decode_request(line).expect("partial query must decode");
        match req {
            Request::Search { query } => {
                assert_eq!(query.text.as_deref(), Some("index"));
                assert!(query.tags.is_empty());
                assert!(query.lang.is_none());
            }
            other => panic!("expected search, got {other:?}"),
        }
    }

    #[test]
    fn from_env_resolves_remote_socket_and_default() {
        // This test mutates process env, so keep all cases in one test to avoid races.
        for k in ["MDKB_REMOTE", "MDKB_TOKEN", "MDKB_SOCKET", "MDKB_VAULT"] {
            std::env::remove_var(k);
        }
        // Default: local socket.
        let c = Client::from_env().unwrap();
        assert!(c.endpoint().starts_with("local:"));

        // Explicit socket override.
        std::env::set_var("MDKB_SOCKET", "/tmp/custom.sock");
        let c = Client::from_env().unwrap();
        assert_eq!(c.endpoint(), "local:/tmp/custom.sock");

        // Remote requires a token: set remote without token → error.
        std::env::set_var("MDKB_REMOTE", "host.example:7820");
        assert!(Client::from_env().is_err());

        // Remote + token → TCP, and takes precedence over the socket.
        std::env::set_var("MDKB_TOKEN", "secret");
        let c = Client::from_env().unwrap();
        assert_eq!(c.endpoint(), "tcp:host.example:7820");

        for k in ["MDKB_REMOTE", "MDKB_TOKEN", "MDKB_SOCKET", "MDKB_VAULT"] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn dispatch_executes_writes_and_reads() {
        let (_dir, mut svc) = service();
        let ctx = RequestContext::local();

        // Create a block.
        let id = match dispatch(
            &mut svc,
            &ctx,
            Request::UpsertBlock {
                id: None,
                text: "StormEvents | take 10".into(),
                page: Some("queries.md".into()),
            },
        ) {
            Response::BlockId(id) => id,
            other => panic!("expected block id, got {other:?}"),
        };

        // Read it back via dispatch.
        match dispatch(&mut svc, &ctx, Request::GetBlock { id: id.clone() }) {
            Response::Block(Some(b)) => assert_eq!(b.content, "StormEvents | take 10"),
            other => panic!("expected block, got {other:?}"),
        }

        // Stats reflect one page / one block.
        match dispatch(&mut svc, &ctx, Request::Stats) {
            Response::Stats(s) => {
                assert_eq!(s.pages, 1);
                assert_eq!(s.blocks, 1);
            }
            other => panic!("expected stats, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_reports_errors_as_response() {
        let (_dir, mut svc) = service();
        let ctx = RequestContext::local();
        let resp = dispatch(
            &mut svc,
            &ctx,
            Request::UpsertBlock {
                id: None,
                text: "x".into(),
                page: None, // missing page → error
            },
        );
        assert!(resp.error_message().is_some());
    }
}
