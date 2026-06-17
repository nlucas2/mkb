//! The mdkb wire protocol: request/response types, a blocking client, and the shared
//! request dispatcher.
//!
//! Both ends speak newline-delimited JSON over a transport (a Unix socket today). The
//! [`dispatch`] function maps a [`Request`] onto a [`mdkb_core::Service`] and is the single
//! place request handling lives — the daemon is just transport glue around it, and any
//! other host (tests, an embedded server) reuses the exact same logic. This upholds the
//! "no divergence" rule in `AGENTS.md`.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

pub mod paths;
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

/// A blocking client over a Unix-domain socket.
///
/// One request per connection (simple and robust). A future network transport implements
/// the same surface behind this type (plan Decision #9).
#[derive(Debug, Clone)]
pub struct Client {
    socket: PathBuf,
}

impl Client {
    /// Point a client at a daemon socket path.
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Client {
            socket: socket.into(),
        }
    }

    /// The socket path.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Send one request and read one response.
    pub fn call(&self, request: &Request) -> io::Result<Response> {
        let stream = UnixStream::connect(&self.socket)?;
        let mut writer = stream.try_clone()?;
        writer.write_all(encode_request(request)?.as_bytes())?;
        writer.flush()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "empty response from daemon",
            ));
        }
        let resp: Response = serde_json::from_str(line.trim_end())?;
        Ok(resp)
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
