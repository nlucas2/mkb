//! The mdkb wire protocol: request/response types, a blocking client, and the shared
//! request dispatcher.
//!
//! Both ends speak newline-delimited JSON over a transport (a local socket or TCP). The
//! [`dispatch`] function maps a [`Request`] onto a [`mdkb_core::Service`] and is the single
//! place request handling lives — the daemon is just transport glue around it. This upholds
//! the "no divergence" rule in `AGENTS.md`. The unit everywhere is the **block** (one file).

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub mod connect;
pub mod paths;
pub mod transport;
pub use connect::{connect, ensure_daemon, ConnectionConfig};
pub use paths::DaemonPaths;

use mdkb_core::export::{ExportRequest, PlannedDoc};
use mdkb_core::{
    BlockId, BlockRecord, GraphData, Index, IndexStats, LinkOutcome, LinkRow, RenderedBlock,
    RequestContext, SearchHit, SearchQuery, Service, TagCount,
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
    /// List all block ids.
    ListBlocks,
    /// List root block ids (top-level entries).
    ListRoots,
    /// The block-level knowledge graph.
    Graph,
    /// All tags with per-tag block counts (tag discovery).
    ListTags,
    /// Plan the docs-as-data export against the live vault. The [`ExportRequest`] encodes the
    /// selector (manifest / tag / whole-KB) and options (`follow_links`, `raw`) such that illegal
    /// combinations are unrepresentable.
    PlanExports { request: ExportRequest },
    /// Search (keyword + semantic).
    Search {
        /// The query.
        query: SearchQuery,
    },
    /// Fetch a block record by id.
    GetBlock {
        /// Block id.
        id: BlockId,
    },
    /// Raw Markdown body of a block (for editing).
    GetBlockSource {
        /// Block id.
        id: BlockId,
    },
    /// Render a block with its children (transclusions) resolved.
    RenderBlock {
        /// Block id.
        id: BlockId,
    },
    /// Render a block as raw + resolved Markdown.
    RenderedBlock {
        /// Block id.
        id: BlockId,
    },
    /// Render a block to flat, self-contained Markdown (embeds dissolved; the published form).
    RenderFlat {
        /// Block id.
        id: BlockId,
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
    /// Dangling links (unresolved targets) across the vault — for the health view.
    Dangling,
    /// Create a new block (optional title + body). Returns the new id.
    CreateBlock {
        /// Optional title.
        title: Option<String>,
        /// Markdown body.
        body: String,
    },
    /// Overwrite a block's title + body.
    UpdateBlock {
        /// Block id.
        id: BlockId,
        /// Optional title.
        title: Option<String>,
        /// Markdown body.
        body: String,
    },
    /// Delete a block.
    DeleteBlock {
        /// Block id.
        id: BlockId,
    },
    /// Set a block's managed (frontmatter) tags to exactly this set.
    SetTags {
        /// Block id.
        id: BlockId,
        /// The full desired tag set (replaces existing frontmatter tags).
        tags: Vec<String>,
    },
    /// Carve a new child block out of an existing block. Returns the new child id.
    CarveBlock {
        /// Parent block id.
        parent_id: BlockId,
        /// Optional title for the new child.
        title: Option<String>,
        /// Markdown body for the new child.
        body: String,
    },
    /// Carve a byte range of a parent block's body into a new child, replacing it in place
    /// with an embed. Returns the new child id.
    CarveSelection {
        /// Parent block id.
        parent_id: BlockId,
        /// Start byte offset into the parent's raw body.
        start: usize,
        /// End byte offset (exclusive).
        end: usize,
    },
    /// Link or embed one block to another (embed may downgrade to a reference to avoid a cycle).
    LinkBlocks {
        /// Source block id.
        source_id: BlockId,
        /// Target block id.
        target_id: BlockId,
        /// `true` to embed (`![[...]]`), `false` to reference (`[[...]]`).
        embed: bool,
    },
    /// Reconcile the `blocks/` directory with the index.
    Reconcile,
    /// Rebuild the entire index from the block files.
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
    /// A list of block ids.
    Ids(Vec<BlockId>),
    /// String names (e.g. conflict files).
    Names(Vec<String>),
    /// The block-level knowledge graph.
    Graph(GraphData),
    /// Tags with per-tag block counts.
    Tags(Vec<TagCount>),
    /// Planned export docs (path + content per manifest entry).
    Exports(Vec<PlannedDoc>),
    /// Search results.
    Hits(Vec<SearchHit>),
    /// A single (optional) block record.
    Block(Option<BlockRecord>),
    /// A single (optional) rendered block.
    Rendered(Option<RenderedBlock>),
    /// Optional text (block source / rendered block).
    Text(Option<String>),
    /// Link rows.
    Links(Vec<LinkRow>),
    /// An affected block id (e.g. after create/carve).
    BlockId(BlockId),
    /// The outcome of a link/embed write (may report a cycle-avoiding downgrade).
    Linked(LinkOutcome),
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
        Request::ListBlocks => Response::Ids(service.list_blocks(ctx).map_err(to_str)?),
        Request::ListRoots => Response::Ids(service.list_roots(ctx).map_err(to_str)?),
        Request::Graph => Response::Graph(service.graph(ctx).map_err(to_str)?),
        Request::ListTags => Response::Tags(service.list_tags(ctx).map_err(to_str)?),
        Request::PlanExports { request } => {
            Response::Exports(service.plan_exports(ctx, &request).map_err(to_str)?)
        }
        Request::Search { query } => Response::Hits(service.search(ctx, query).map_err(to_str)?),
        Request::GetBlock { id } => Response::Block(service.get_block(ctx, &id).map_err(to_str)?),
        Request::GetBlockSource { id } => {
            Response::Text(service.get_block_source(ctx, &id).map_err(to_str)?)
        }
        Request::RenderBlock { id } => {
            Response::Text(service.render_block(ctx, &id).map_err(to_str)?)
        }
        Request::RenderedBlock { id } => {
            Response::Rendered(service.rendered_block(ctx, &id).map_err(to_str)?)
        }
        Request::RenderFlat { id } => {
            Response::Text(service.render_flat(ctx, &id).map_err(to_str)?)
        }
        Request::LinksFrom { id } => Response::Links(service.links_from(ctx, &id).map_err(to_str)?),
        Request::Backlinks { id } => Response::Links(service.backlinks(ctx, &id).map_err(to_str)?),
        Request::Dangling => Response::Links(service.dangling_links(ctx).map_err(to_str)?),
        Request::CreateBlock { title, body } => Response::BlockId(
            service
                .create_block(ctx, title.as_deref(), &body)
                .map_err(to_str)?,
        ),
        Request::UpdateBlock { id, title, body } => {
            service
                .update_block(ctx, &id, title.as_deref(), &body)
                .map_err(to_str)?;
            Response::Ok
        }
        Request::DeleteBlock { id } => {
            service.delete_block(ctx, &id).map_err(to_str)?;
            Response::Ok
        }
        Request::SetTags { id, tags } => {
            service.set_tags(ctx, &id, &tags).map_err(to_str)?;
            Response::Ok
        }
        Request::CarveBlock {
            parent_id,
            title,
            body,
        } => Response::BlockId(
            service
                .carve_block(ctx, &parent_id, title.as_deref(), &body)
                .map_err(to_str)?,
        ),
        Request::CarveSelection {
            parent_id,
            start,
            end,
        } => Response::BlockId(
            service
                .carve_selection(ctx, &parent_id, start, end)
                .map_err(to_str)?,
        ),
        Request::LinkBlocks {
            source_id,
            target_id,
            embed,
        } => Response::Linked(
            service
                .link_blocks(ctx, &source_id, &target_id, embed)
                .map_err(to_str)?,
        ),
        Request::Reconcile => {
            service.reconcile(ctx).map_err(to_str)?;
            Response::Ok
        }
        Request::Rebuild => {
            service.rebuild(ctx).map_err(to_str)?;
            Response::Ok
        }
        Request::Conflicts => Response::Names(service.conflicts(ctx).map_err(to_str)?),
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

    /// Convenience: all tags with per-tag block counts.
    pub fn list_tags(&self) -> io::Result<Vec<TagCount>> {
        match self.call(&Request::ListTags)? {
            Response::Tags(t) => Ok(t),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: plan the docs-as-data export for an [`ExportRequest`] (manifest / tag /
    /// whole-KB, with `follow_links`/`raw`). Co-exported docs cross-link; links leaving the set
    /// surface as per-doc [`PlannedDoc::warnings`].
    pub fn plan_exports(&self, request: ExportRequest) -> io::Result<Vec<PlannedDoc>> {
        match self.call(&Request::PlanExports { request })? {
            Response::Exports(d) => Ok(d),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: set a block's managed (frontmatter) tags to exactly `tags`.
    pub fn set_tags(&self, id: BlockId, tags: Vec<String>) -> io::Result<()> {
        match self.call(&Request::SetTags { id, tags })? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: list all block ids.
    pub fn list_blocks(&self) -> io::Result<Vec<BlockId>> {
        match self.call(&Request::ListBlocks)? {
            Response::Ids(v) => Ok(v),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: list root block ids.
    pub fn list_roots(&self) -> io::Result<Vec<BlockId>> {
        match self.call(&Request::ListRoots)? {
            Response::Ids(v) => Ok(v),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: the knowledge graph.
    pub fn graph(&self) -> io::Result<GraphData> {
        match self.call(&Request::Graph)? {
            Response::Graph(g) => Ok(g),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: fetch a block record.
    pub fn get_block(&self, id: BlockId) -> io::Result<Option<BlockRecord>> {
        match self.call(&Request::GetBlock { id })? {
            Response::Block(b) => Ok(b),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: raw Markdown body of a block.
    pub fn get_block_source(&self, id: BlockId) -> io::Result<Option<String>> {
        match self.call(&Request::GetBlockSource { id })? {
            Response::Text(t) => Ok(t),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: render a block (transclusions resolved).
    pub fn render_block(&self, id: BlockId) -> io::Result<Option<String>> {
        match self.call(&Request::RenderBlock { id })? {
            Response::Text(t) => Ok(t),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: render a block to flat, self-contained Markdown (the published form).
    pub fn render_flat(&self, id: BlockId) -> io::Result<Option<String>> {
        match self.call(&Request::RenderFlat { id })? {
            Response::Text(t) => Ok(t),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: render a block as raw + resolved Markdown.
    pub fn rendered_block(&self, id: BlockId) -> io::Result<Option<RenderedBlock>> {
        match self.call(&Request::RenderedBlock { id })? {
            Response::Rendered(b) => Ok(b),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: outgoing links from a block.
    pub fn links_from(&self, id: BlockId) -> io::Result<Vec<LinkRow>> {
        match self.call(&Request::LinksFrom { id })? {
            Response::Links(l) => Ok(l),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: incoming references / transclusions of a block.
    pub fn backlinks(&self, id: BlockId) -> io::Result<Vec<LinkRow>> {
        match self.call(&Request::Backlinks { id })? {
            Response::Links(l) => Ok(l),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: dangling links across the vault.
    pub fn dangling(&self) -> io::Result<Vec<LinkRow>> {
        match self.call(&Request::Dangling)? {
            Response::Links(l) => Ok(l),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: create a block.
    pub fn create_block(&self, title: Option<&str>, body: &str) -> io::Result<BlockId> {
        match self.call(&Request::CreateBlock {
            title: title.map(str::to_string),
            body: body.to_string(),
        })? {
            Response::BlockId(id) => Ok(id),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: update a block.
    pub fn update_block(&self, id: BlockId, title: Option<&str>, body: &str) -> io::Result<()> {
        match self.call(&Request::UpdateBlock {
            id,
            title: title.map(str::to_string),
            body: body.to_string(),
        })? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: delete a block.
    pub fn delete_block(&self, id: BlockId) -> io::Result<()> {
        match self.call(&Request::DeleteBlock { id })? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: carve a child block out of a parent.
    pub fn carve_block(
        &self,
        parent_id: BlockId,
        title: Option<&str>,
        body: &str,
    ) -> io::Result<BlockId> {
        match self.call(&Request::CarveBlock {
            parent_id,
            title: title.map(str::to_string),
            body: body.to_string(),
        })? {
            Response::BlockId(id) => Ok(id),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: carve a byte range of a parent's body into a new child (replace in place).
    pub fn carve_selection(
        &self,
        parent_id: BlockId,
        start: usize,
        end: usize,
    ) -> io::Result<BlockId> {
        match self.call(&Request::CarveSelection {
            parent_id,
            start,
            end,
        })? {
            Response::BlockId(id) => Ok(id),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: link or embed two blocks. Returns the [`LinkOutcome`].
    pub fn link(
        &self,
        source_id: BlockId,
        target_id: BlockId,
        embed: bool,
    ) -> io::Result<LinkOutcome> {
        match self.call(&Request::LinkBlocks {
            source_id,
            target_id,
            embed,
        })? {
            Response::Linked(o) => Ok(o),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: rebuild the index.
    pub fn rebuild(&self) -> io::Result<()> {
        match self.call(&Request::Rebuild)? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: conflict file names.
    pub fn conflicts(&self) -> io::Result<Vec<String>> {
        match self.call(&Request::Conflicts)? {
            Response::Names(n) => Ok(n),
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
        let req = Request::CreateBlock {
            title: Some("T".into()),
            body: "hello".into(),
        };
        let line = encode_request(&req).unwrap();
        let decoded: Request = serde_json::from_str(line.trim_end()).unwrap();
        assert!(matches!(decoded, Request::CreateBlock { .. }));
    }

    #[test]
    fn every_response_variant_serializes_and_round_trips() {
        let id = mdkb_core::BlockId::generate();
        let variants = vec![
            Response::Pong,
            Response::Stats(IndexStats::default()),
            Response::Ids(vec![id.clone()]),
            Response::Names(vec!["a".into()]),
            Response::Graph(GraphData::default()),
            Response::Hits(vec![]),
            Response::Block(None),
            Response::Rendered(None),
            Response::Text(Some("hi".into())),
            Response::Links(vec![]),
            Response::BlockId(id),
            Response::Linked(LinkOutcome::DowngradedToReference),
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
    fn dispatch_executes_writes_and_reads() {
        let (_dir, mut svc) = service();
        let ctx = RequestContext::local();

        let id = match dispatch(
            &mut svc,
            &ctx,
            Request::CreateBlock {
                title: Some("Q".into()),
                body: "StormEvents | take 10".into(),
            },
        ) {
            Response::BlockId(id) => id,
            other => panic!("expected block id, got {other:?}"),
        };

        match dispatch(&mut svc, &ctx, Request::GetBlock { id: id.clone() }) {
            Response::Block(Some(b)) => assert_eq!(b.content, "StormEvents | take 10"),
            other => panic!("expected block, got {other:?}"),
        }

        match dispatch(&mut svc, &ctx, Request::Stats) {
            Response::Stats(s) => assert_eq!(s.blocks, 1),
            other => panic!("expected stats, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_serves_graph_and_link_with_downgrade() {
        let (_dir, mut svc) = service();
        let ctx = RequestContext::local();
        let a = match dispatch(
            &mut svc,
            &ctx,
            Request::CreateBlock {
                title: None,
                body: "A".into(),
            },
        ) {
            Response::BlockId(id) => id,
            o => panic!("{o:?}"),
        };
        let b = match dispatch(
            &mut svc,
            &ctx,
            Request::CreateBlock {
                title: None,
                body: "B".into(),
            },
        ) {
            Response::BlockId(id) => id,
            o => panic!("{o:?}"),
        };
        // A embeds B.
        assert!(matches!(
            dispatch(
                &mut svc,
                &ctx,
                Request::LinkBlocks {
                    source_id: a.clone(),
                    target_id: b.clone(),
                    embed: true
                }
            ),
            Response::Linked(LinkOutcome::Transclusion)
        ));
        // B embedding A would cycle -> downgrade.
        assert!(matches!(
            dispatch(
                &mut svc,
                &ctx,
                Request::LinkBlocks {
                    source_id: b.clone(),
                    target_id: a.clone(),
                    embed: true
                }
            ),
            Response::Linked(LinkOutcome::DowngradedToReference)
        ));
        match dispatch(&mut svc, &ctx, Request::Graph) {
            Response::Graph(g) => assert_eq!(g.nodes.len(), 2),
            o => panic!("expected graph, got {o:?}"),
        }
    }

    #[test]
    fn dispatch_reports_errors_as_response() {
        let (_dir, mut svc) = service();
        let ctx = RequestContext::remote("agent");
        match dispatch(&mut svc, &ctx, Request::ListBlocks) {
            Response::Error { message } => assert!(message.contains("unauthorized")),
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn from_env_resolves_remote_socket_and_default() {
        std::env::remove_var("MDKB_REMOTE");
        std::env::remove_var("MDKB_SOCKET");
        let c = Client::from_env().unwrap();
        assert!(c.endpoint().starts_with("local:"));
    }
}
