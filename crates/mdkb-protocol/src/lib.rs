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
pub mod env;
pub mod paths;
pub mod transport;
pub use connect::{
    connect, connect_resolved, ensure_daemon, resolve_client, resolve_target, ClientInputs,
    ConnectionConfig, EnvSnapshot, Registry, ResolvedTarget, VaultEntry,
};
pub use paths::DaemonPaths;

use mdkb_core::export::{ExportRequest, PlannedDoc};
use mdkb_core::{
    BlockId, BlockRecord, GraphData, Index, IndexStats, LinkOutcome, LinkRow, PageView,
    RenderedBlock, RequestContext, SearchHit, SearchQuery, Service, TagCount,
};
use serde::{Deserialize, Serialize};

/// Serde default for `expect_count`: a partial edit expects a single match unless told otherwise.
fn one() -> usize {
    1
}

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
    /// Rich, self-contained read of a block: content (raw, rendered, or a line range) plus its
    /// lineage and its incoming/outgoing relationships. Folds together [`GetBlock`],
    /// [`RenderBlock`], [`GetBlockSourceRange`], [`LinksFrom`] and [`Backlinks`].
    ///
    /// [`GetBlock`]: Request::GetBlock
    /// [`RenderBlock`]: Request::RenderBlock
    /// [`GetBlockSourceRange`]: Request::GetBlockSourceRange
    /// [`LinksFrom`]: Request::LinksFrom
    /// [`Backlinks`]: Request::Backlinks
    GetBlockView {
        /// Block id.
        id: BlockId,
        /// Resolve transclusions (inline children) in the returned body.
        #[serde(default)]
        rendered: bool,
        /// First line (1-based, inclusive) when slicing the raw body; `None` = from the start.
        #[serde(default)]
        start: Option<usize>,
        /// Last line (1-based, inclusive) when slicing the raw body; `None` = to the end.
        #[serde(default)]
        end: Option<usize>,
    },
    /// Raw Markdown body of a block (for editing).
    GetBlockSource {
        /// Block id.
        id: BlockId,
    },
    /// Lines `start..=end` (1-based, inclusive) of a block's raw source body.
    GetBlockSourceRange {
        /// Block id.
        id: BlockId,
        /// First line (1-based, inclusive).
        start: usize,
        /// Last line (1-based, inclusive).
        end: usize,
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
    /// Import a binary asset (image, etc.) into the vault's `assets/` directory. The bytes are
    /// base64-encoded so they travel as a single JSON string within the line-length cap.
    AddAsset {
        /// Suggested filename (sanitised + made unique server-side).
        name: String,
        /// Asset bytes, base64 (standard alphabet, no line breaks).
        data_base64: String,
    },
    /// List orphaned assets: files under `assets/` that no block references.
    OrphanAssets,
    /// Delete an asset by its vault-relative `assets/…` path (e.g. an orphan being swept).
    RemoveAsset {
        /// Vault-relative path under `assets/`.
        path: String,
    },
    /// Overwrite a block's title + body. `force` overrides the destructive-update guard (which
    /// refuses an edit that would empty a block or strip most of its content).
    UpdateBlock {
        /// Block id.
        id: BlockId,
        /// Optional title.
        title: Option<String>,
        /// Markdown body.
        body: String,
        /// Bypass the destructive-update guard for an intentional rewrite. Defaults to `false`.
        #[serde(default)]
        force: bool,
    },
    /// Apply an exact, count-checked string replacement to a block's body (the partial-edit op).
    ReplaceInBlock {
        /// Block id.
        id: BlockId,
        /// Exact substring to find (must occur `expect_count` times).
        old: String,
        /// Replacement text (may be empty to delete).
        new: String,
        /// Required number of occurrences; the edit is refused unless it matches exactly.
        #[serde(default = "one")]
        expect_count: usize,
        /// Bypass the destructive-update guard for an intentional rewrite. Defaults to `false`.
        #[serde(default)]
        force: bool,
    },
    /// Append text to a block's body (it starts on a fresh line). Purely additive.
    AppendToBlock {
        /// Block id.
        id: BlockId,
        /// Text to append.
        text: String,
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
    /// **Merge** properties into a block: add or update each given `key`, preserving all other
    /// properties. Open-ended `key: value` metadata in frontmatter. Add/update-only — there is no
    /// replace-the-whole-set op; use [`Request::UnsetProps`] to remove.
    SetProps {
        /// Block id.
        id: BlockId,
        /// The properties to add or update, as ordered `(key, value)` pairs.
        props: Vec<(String, String)>,
    },
    /// Remove the named properties from a block, preserving all others. Unknown keys are ignored.
    UnsetProps {
        /// Block id.
        id: BlockId,
        /// The property keys to remove (case-insensitive).
        keys: Vec<String>,
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
    /// Flatten (uncarve): inline `parent`'s single `![[child]]` embed back into its body and
    /// delete the child. Only valid when the child has exactly one reference in the whole vault.
    FlattenBlock {
        /// Parent block id (holds the single `![[child]]` embed).
        parent_id: BlockId,
        /// Child block id to inline and delete.
        child_id: BlockId,
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
    /// Acquire-or-renew an **interactive lease** that keeps an auto-started daemon alive while a
    /// long-lived client (the desktop app) is open. The `lease` id is chosen by the
    /// client and stable for its lifetime, so this op is idempotent (acquire and renew are one).
    /// The lease expires `ttl_ms` after the last heartbeat, so a crashed client never pins the
    /// daemon. Handled by the daemon's lifecycle layer, not core. Momentary clients (CLI/MCP)
    /// don't need a lease — their request activity already defers the idle timer.
    Heartbeat {
        /// Client-chosen, stable lease id.
        lease: String,
        /// Lease lifetime in milliseconds from now (the daemon may clamp it).
        ttl_ms: u64,
    },
    /// Release an interactive lease (the client is closing cleanly). The daemon then reaps itself
    /// after the normal idle grace if nothing else holds it. Unknown leases are ignored.
    ReleaseLease {
        /// The lease id to drop.
        lease: String,
    },
    /// Lock or unlock a block (toggle its human-only `locked` flag). Requires the `ManageLocks`
    /// capability, which only the desktop app's connection holds — so this is effectively an
    /// app-only op (see [`Request::AnnounceApp`]).
    SetLock {
        /// Block id.
        id: BlockId,
        /// `true` to lock (human-only), `false` to unlock.
        locked: bool,
    },
    /// Declare this connection the **desktop app** — the human surface — upgrading its scope to
    /// include `ManageLocks` (lock/unlock). Per-connection, mirroring [`Request::Authenticate`];
    /// it mutates the daemon-side request context. This is a local-trust guardrail, not a
    /// security boundary: the local Unix socket is already trusted, and this just keeps machine
    /// clients (CLI/MCP) from toggling locks. On a token-gated remote transport it is rejected.
    AnnounceApp,
    /// Ask the daemon to shut down cleanly: remove its socket and exit. **Local connections only**
    /// — a remote/authenticated caller is refused, so a network client can't take down a shared
    /// daemon. Used by the desktop app's "restart daemon" control (shut down, then reconnect,
    /// which auto-starts a fresh daemon — e.g. to pick up an upgraded binary).
    Shutdown,
}

impl Request {
    /// Whether applying this request **mutates vault content** (a block or asset file), so the
    /// daemon should advance its content generation after it succeeds. A write through the daemon
    /// updates the file *and* the index together, so the watcher's later reconcile is a no-op and
    /// can't be relied on to signal it — hence this explicit classification.
    ///
    /// Exhaustive on purpose: a newly added request variant won't compile until it is classified
    /// here as a write (bumps the generation) or a read/lifecycle op (doesn't).
    pub fn mutates(&self) -> bool {
        match self {
            Request::CreateBlock { .. }
            | Request::AddAsset { .. }
            | Request::RemoveAsset { .. }
            | Request::UpdateBlock { .. }
            | Request::ReplaceInBlock { .. }
            | Request::AppendToBlock { .. }
            | Request::DeleteBlock { .. }
            | Request::SetTags { .. }
            | Request::SetProps { .. }
            | Request::UnsetProps { .. }
            | Request::CarveBlock { .. }
            | Request::CarveSelection { .. }
            | Request::FlattenBlock { .. }
            | Request::LinkBlocks { .. }
            | Request::SetLock { .. } => true,
            Request::Ping
            | Request::Stats
            | Request::ListBlocks
            | Request::ListRoots
            | Request::Graph
            | Request::ListTags
            | Request::PlanExports { .. }
            | Request::Search { .. }
            | Request::GetBlock { .. }
            | Request::GetBlockView { .. }
            | Request::GetBlockSource { .. }
            | Request::GetBlockSourceRange { .. }
            | Request::RenderBlock { .. }
            | Request::RenderedBlock { .. }
            | Request::RenderFlat { .. }
            | Request::LinksFrom { .. }
            | Request::Backlinks { .. }
            | Request::Dangling
            | Request::OrphanAssets
            | Request::Reconcile
            | Request::Rebuild
            | Request::Conflicts
            | Request::Authenticate { .. }
            | Request::Heartbeat { .. }
            | Request::ReleaseLease { .. }
            | Request::AnnounceApp
            | Request::Shutdown => false,
        }
    }
}
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
    /// A single (optional) rich page view (block + lineage + relationships).
    Page(Option<PageView>),
    /// Reply to [`Request::Heartbeat`]: the lease is renewed, and `generation` is the daemon's
    /// monotonic vault-content counter — it advances whenever a block changes (any write, or an
    /// external edit the watcher reconciles). A long-lived client compares it across heartbeats
    /// and refreshes when it moves. (Old daemons reply [`Response::Ok`]; clients treat that as
    /// generation `0` — no live-refresh, graceful degradation.)
    Heartbeat {
        /// The daemon's current vault-content generation.
        generation: u64,
    },
    /// A single (optional) rendered block.
    Rendered(Option<RenderedBlock>),
    /// Optional text (block source / rendered block).
    Text(Option<String>),
    /// Link rows.
    Links(Vec<LinkRow>),
    /// An affected block id (e.g. after create/carve).
    BlockId(BlockId),
    /// A vault-relative path (e.g. an imported asset's `assets/<name>`).
    Path(String),
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
        Request::GetBlockView {
            id,
            rendered,
            start,
            end,
        } => Response::Page(
            service
                .page_view(ctx, &id, rendered, start, end)
                .map_err(to_str)?,
        ),
        Request::GetBlockSource { id } => {
            Response::Text(service.get_block_source(ctx, &id).map_err(to_str)?)
        }
        Request::GetBlockSourceRange { id, start, end } => Response::Text(
            service
                .block_source_range(ctx, &id, start, end)
                .map_err(to_str)?,
        ),
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
        Request::AddAsset { name, data_base64 } => {
            let bytes = base64_decode(&data_base64).map_err(|e| e.to_string())?;
            Response::Path(service.add_asset(ctx, &name, &bytes).map_err(to_str)?)
        }
        Request::OrphanAssets => Response::Names(service.orphan_assets(ctx).map_err(to_str)?),
        Request::RemoveAsset { path } => {
            service.remove_asset(ctx, &path).map_err(to_str)?;
            Response::Ok
        }
        Request::UpdateBlock {
            id,
            title,
            body,
            force,
        } => {
            service
                .update_block(ctx, &id, title.as_deref(), &body, force)
                .map_err(to_str)?;
            Response::Ok
        }
        Request::DeleteBlock { id } => {
            service.delete_block(ctx, &id).map_err(to_str)?;
            Response::Ok
        }
        Request::ReplaceInBlock {
            id,
            old,
            new,
            expect_count,
            force,
        } => {
            service
                .replace_in_block(ctx, &id, &old, &new, expect_count, force)
                .map_err(to_str)?;
            Response::Ok
        }
        Request::AppendToBlock { id, text } => {
            service.append_to_block(ctx, &id, &text).map_err(to_str)?;
            Response::Ok
        }
        Request::SetTags { id, tags } => {
            service.set_tags(ctx, &id, &tags).map_err(to_str)?;
            Response::Ok
        }
        Request::SetProps { id, props } => {
            service.set_props(ctx, &id, &props).map_err(to_str)?;
            Response::Ok
        }
        Request::UnsetProps { id, keys } => {
            service.unset_props(ctx, &id, &keys).map_err(to_str)?;
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
        Request::FlattenBlock {
            parent_id,
            child_id,
        } => {
            service
                .flatten_block(ctx, &parent_id, &child_id)
                .map_err(to_str)?;
            Response::Ok
        }
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
        Request::SetLock { id, locked } => {
            service.set_lock(ctx, &id, locked).map_err(to_str)?;
            Response::Ok
        }
        Request::Authenticate { .. } => Response::Error {
            message: "authenticate is handled by the transport layer".to_string(),
        },
        Request::AnnounceApp => Response::Error {
            message: "announce_app is handled by the daemon connection layer".to_string(),
        },
        Request::Shutdown => Response::Error {
            message: "shutdown is handled by the daemon connection layer".to_string(),
        },
        Request::Heartbeat { .. } | Request::ReleaseLease { .. } => Response::Error {
            message: "lease ops are handled by the daemon lifecycle layer".to_string(),
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

/// Base64-encode bytes (standard alphabet, no line breaks) for transport as a JSON string.
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode a base64 (standard alphabet) string back to bytes.
fn base64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s)
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
    /// When set, each **local** request is preceded by an [`Request::AnnounceApp`] handshake so
    /// the connection is granted the app scope (lock management). Only the desktop app sets this;
    /// because every `call` opens a fresh connection, the announce must be re-sent each time
    /// (mirroring how the TCP transport re-authenticates per connection).
    announce_app: bool,
}
impl Client {
    /// Point a client at a daemon local socket path (Unix socket / Windows named pipe).
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Client {
            transport: Transport::Local(socket.into()),
            announce_app: false,
        }
    }

    /// Point a client at a daemon TCP address, authenticating with `token`.
    pub fn tcp(addr: impl Into<String>, token: impl Into<String>) -> Self {
        Client {
            transport: Transport::Tcp {
                addr: addr.into(),
                token: token.into(),
            },
            announce_app: false,
        }
    }

    /// Mark this client as the **desktop app** (the human surface): each local request announces
    /// the app scope, so lock/unlock (`set_lock`) is permitted. A no-op effect on a remote (TCP)
    /// transport, where lock management is not granted. The CLI and MCP server never set this.
    pub fn as_app(mut self) -> Self {
        self.announce_app = true;
        self
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
    /// This is the single connection-resolution path shared by the desktop app
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
                // The desktop app upgrades its scope (lock management) once per connection, before
                // the actual request — mirroring the TCP auth handshake below. This is a privilege
                // *upgrade*, not a gate: it is best-effort, so an older daemon that doesn't know
                // the op (or otherwise declines) simply leaves the connection at the default scope
                // and the request still proceeds. The daemon keeps the connection open after an
                // error, so the next line is the real request either way.
                if self.announce_app {
                    send(&mut writer, &Request::AnnounceApp)?;
                    let _ = read_one(&mut reader)?;
                }
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

    /// Convenience: merge properties into a block (add or update each given key; preserves the
    /// rest).
    pub fn set_props(&self, id: BlockId, props: Vec<(String, String)>) -> io::Result<()> {
        match self.call(&Request::SetProps { id, props })? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: remove the named properties from a block (preserves all others).
    pub fn unset_props(&self, id: BlockId, keys: Vec<String>) -> io::Result<()> {
        match self.call(&Request::UnsetProps { id, keys })? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: lock or unlock a block (human-only flag). Requires the app scope — call this
    /// on a client built with [`Client::as_app`]; an agent client is rejected by the daemon.
    pub fn set_lock(&self, id: BlockId, locked: bool) -> io::Result<()> {
        match self.call(&Request::SetLock { id, locked })? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: ask the daemon to shut down (local connections only). After this returns, the
    /// next call reconnects and auto-starts a fresh daemon. Used by the app's "restart daemon".
    pub fn shutdown(&self) -> io::Result<()> {
        match self.call(&Request::Shutdown)? {
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

    /// Convenience: a rich page view (block + lineage + relationships). `rendered` inlines
    /// children; `start`/`end` (1-based, inclusive) slice the raw body instead.
    pub fn get_block_view(
        &self,
        id: BlockId,
        rendered: bool,
        start: Option<usize>,
        end: Option<usize>,
    ) -> io::Result<Option<PageView>> {
        match self.call(&Request::GetBlockView {
            id,
            rendered,
            start,
            end,
        })? {
            Response::Page(p) => Ok(p),
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

    /// Convenience: lines `start..=end` (1-based, inclusive) of a block's raw source body.
    pub fn get_block_source_range(
        &self,
        id: BlockId,
        start: usize,
        end: usize,
    ) -> io::Result<Option<String>> {
        match self.call(&Request::GetBlockSourceRange { id, start, end })? {
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

    /// Convenience: import a binary asset into the vault's `assets/` directory. Returns the
    /// vault-relative path (e.g. `assets/<name>`) to reference from a block. The bytes are
    /// base64-encoded on the wire.
    pub fn add_asset(&self, name: &str, bytes: &[u8]) -> io::Result<String> {
        match self.call(&Request::AddAsset {
            name: name.to_string(),
            data_base64: base64_encode(bytes),
        })? {
            Response::Path(p) => Ok(p),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: list orphaned assets (files under `assets/` no block references).
    pub fn orphan_assets(&self) -> io::Result<Vec<String>> {
        match self.call(&Request::OrphanAssets)? {
            Response::Names(v) => Ok(v),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: delete an asset by its vault-relative `assets/…` path.
    pub fn remove_asset(&self, path: &str) -> io::Result<()> {
        match self.call(&Request::RemoveAsset {
            path: path.to_string(),
        })? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: update a block. `force` bypasses the destructive-update guard (an edit that
    /// would empty the block or strip most of its content); pass `false` for ordinary edits.
    pub fn update_block(
        &self,
        id: BlockId,
        title: Option<&str>,
        body: &str,
        force: bool,
    ) -> io::Result<()> {
        match self.call(&Request::UpdateBlock {
            id,
            title: title.map(str::to_string),
            body: body.to_string(),
            force,
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

    /// Convenience: apply an exact, count-checked string replacement to a block's body. `old` must
    /// occur exactly `expect_count` times or the call errors and nothing changes.
    pub fn replace_in_block(
        &self,
        id: BlockId,
        old: &str,
        new: &str,
        expect_count: usize,
        force: bool,
    ) -> io::Result<()> {
        match self.call(&Request::ReplaceInBlock {
            id,
            old: old.to_string(),
            new: new.to_string(),
            expect_count,
            force,
        })? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: append text to a block's body (starts on a fresh line; purely additive).
    pub fn append_to_block(&self, id: BlockId, text: &str) -> io::Result<()> {
        match self.call(&Request::AppendToBlock {
            id,
            text: text.to_string(),
        })? {
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

    /// Convenience: flatten (uncarve) a parent's single `![[child]]` embed back inline and delete
    /// the child. Errors unless the child has exactly one reference in the vault.
    pub fn flatten(&self, parent_id: BlockId, child_id: BlockId) -> io::Result<()> {
        match self.call(&Request::FlattenBlock {
            parent_id,
            child_id,
        })? {
            Response::Ok => Ok(()),
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

    /// Convenience: acquire-or-renew an interactive lease (keeps an auto-started daemon alive
    /// while a long-lived client is open). Idempotent in `lease`; call periodically (every
    /// ~`ttl_ms`/3) as a heartbeat. Returns the daemon's current vault-content **generation** —
    /// a monotonic counter that advances whenever a block changes — so a long-lived client can
    /// compare it across beats and refresh when it moves. An older daemon that replies `Ok`
    /// (no generation) is reported as `0`, which never advances → live-refresh simply no-ops.
    pub fn heartbeat(&self, lease: &str, ttl_ms: u64) -> io::Result<u64> {
        match self.call(&Request::Heartbeat {
            lease: lease.to_string(),
            ttl_ms,
        })? {
            Response::Heartbeat { generation } => Ok(generation),
            Response::Ok => Ok(0),
            other => Err(unexpected(other)),
        }
    }

    /// Convenience: release an interactive lease (the client is closing cleanly).
    pub fn release_lease(&self, lease: &str) -> io::Result<()> {
        match self.call(&Request::ReleaseLease {
            lease: lease.to_string(),
        })? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
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
    fn mutates_classifies_writes_and_reads() {
        let id = mdkb_core::BlockId::generate();
        // A representative set of writes — each must advance the generation.
        let writes = [
            Request::CreateBlock {
                title: None,
                body: "b".into(),
            },
            Request::UpdateBlock {
                id: id.clone(),
                title: None,
                body: "b".into(),
                force: false,
            },
            Request::ReplaceInBlock {
                id: id.clone(),
                old: "a".into(),
                new: "b".into(),
                expect_count: 1,
                force: false,
            },
            Request::AppendToBlock {
                id: id.clone(),
                text: "x".into(),
            },
            Request::DeleteBlock { id: id.clone() },
            Request::SetTags {
                id: id.clone(),
                tags: vec![],
            },
            Request::LinkBlocks {
                source_id: id.clone(),
                target_id: id.clone(),
                embed: true,
            },
            Request::SetLock {
                id: id.clone(),
                locked: true,
            },
        ];
        for w in writes {
            assert!(w.mutates(), "{w:?} should be a write");
        }
        // Reads / liveness / lifecycle must NOT advance the generation.
        let reads = [
            Request::Ping,
            Request::Search {
                query: SearchQuery::default(),
            },
            Request::GetBlock { id: id.clone() },
            Request::GetBlockView {
                id: id.clone(),
                rendered: false,
                start: None,
                end: None,
            },
            Request::ListBlocks,
            Request::Backlinks { id: id.clone() },
            Request::Heartbeat {
                lease: "x".into(),
                ttl_ms: 1000,
            },
            Request::Rebuild,
        ];
        for r in reads {
            assert!(!r.mutates(), "{r:?} should not be a write");
        }
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
    fn dispatch_replace_in_block_edits_body() {
        let (_dir, mut svc) = service();
        let ctx = RequestContext::local();
        let id = match dispatch(
            &mut svc,
            &ctx,
            Request::CreateBlock {
                title: Some("T".into()),
                body: "the quick brown fox jumps".into(),
            },
        ) {
            Response::BlockId(id) => id,
            other => panic!("expected block id, got {other:?}"),
        };
        // Exact replace applies and preserves the title.
        match dispatch(
            &mut svc,
            &ctx,
            Request::ReplaceInBlock {
                id: id.clone(),
                old: "brown".into(),
                new: "red".into(),
                expect_count: 1,
                force: false,
            },
        ) {
            Response::Ok => {}
            other => panic!("expected ok, got {other:?}"),
        }
        match dispatch(&mut svc, &ctx, Request::GetBlock { id: id.clone() }) {
            Response::Block(Some(b)) => {
                assert_eq!(b.content, "the quick red fox jumps");
                assert_eq!(b.title.as_deref(), Some("T"));
            }
            other => panic!("expected block, got {other:?}"),
        }
        // A count mismatch is an error, no change.
        match dispatch(
            &mut svc,
            &ctx,
            Request::ReplaceInBlock {
                id: id.clone(),
                old: "fox".into(),
                new: "cat".into(),
                expect_count: 2,
                force: false,
            },
        ) {
            Response::Error { .. } => {}
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_add_asset_writes_file_and_returns_path() {
        let (dir, mut svc) = service();
        let ctx = RequestContext::local();
        let rel = match dispatch(
            &mut svc,
            &ctx,
            Request::AddAsset {
                name: "pic.png".into(),
                data_base64: base64_encode(b"\x89PNG\r\n\x1a\n"),
            },
        ) {
            Response::Path(p) => p,
            other => panic!("expected path, got {other:?}"),
        };
        assert_eq!(rel, "assets/pic.png");
        assert_eq!(
            std::fs::read(dir.path().join(&rel)).unwrap(),
            b"\x89PNG\r\n\x1a\n"
        );
        // Not indexed.
        match dispatch(&mut svc, &ctx, Request::Stats) {
            Response::Stats(s) => assert_eq!(s.blocks, 0),
            other => panic!("expected stats, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_orphan_and_remove_assets() {
        let (dir, mut svc) = service();
        let ctx = RequestContext::local();
        // Two assets; one is referenced by a block, one is not.
        for name in ["used.png", "orphan.png"] {
            dispatch(
                &mut svc,
                &ctx,
                Request::AddAsset {
                    name: name.into(),
                    data_base64: base64_encode(b"x"),
                },
            );
        }
        dispatch(
            &mut svc,
            &ctx,
            Request::CreateBlock {
                title: None,
                body: "![p](assets/used.png)".into(),
            },
        );
        match dispatch(&mut svc, &ctx, Request::OrphanAssets) {
            Response::Names(v) => assert_eq!(v, vec!["assets/orphan.png".to_string()]),
            other => panic!("expected names, got {other:?}"),
        }
        match dispatch(
            &mut svc,
            &ctx,
            Request::RemoveAsset {
                path: "assets/orphan.png".into(),
            },
        ) {
            Response::Ok => {}
            other => panic!("expected ok, got {other:?}"),
        }
        assert!(!dir.path().join("assets/orphan.png").exists());
        assert!(dir.path().join("assets/used.png").exists());
    }

    #[test]
    fn dispatch_add_asset_is_refused_without_write_capability() {
        let (_dir, mut svc) = service();
        let ctx = RequestContext::remote("agent");
        // A read-only remote context (no Write capability) must not be able to write assets.
        let resp = dispatch(
            &mut svc,
            &ctx,
            Request::AddAsset {
                name: "x.png".into(),
                data_base64: base64_encode(b"data"),
            },
        );
        assert!(matches!(resp, Response::Error { .. }), "got {resp:?}");
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
