//! `mdkb-core` — the shared engine for mdkb.
//!
//! All behavior that touches blocks, transclusion, indexing, search, parsing, or
//! writes lives here. The daemon, MCP server, CLI, and Tauri UI are **thin clients**
//! of this crate so a bug fixed once is fixed everywhere. See `AGENTS.md`.

pub mod block;
pub mod conflict;
pub mod document;
pub mod embed;
pub mod id;
pub mod index;
pub mod link;
pub mod render;
pub mod service;
pub mod sync;
pub mod vault;

pub use block::{Block, BlockKind, Tag, TagSource};
pub use conflict::{is_conflict_path, CONFLICT_MARKERS};
pub use document::{Document, Frontmatter};
pub use embed::{
    bytes_to_vector, cosine_similarity, vector_to_bytes, EmbedError, Embedder, HashEmbedder,
};
pub use id::{BlockId, IdCodec, IdError, MarkerMatch, NativeIdCodec};
pub use index::{
    page_links, reciprocal_rank_fusion, BlockRecord, Index, IndexError, IndexStats, LinkKind,
    LinkRow, SearchHit, SearchQuery,
};
pub use link::{extract_references, Anchor, LinkTarget, Reference};
pub use render::{render_block, render_page};
pub use service::{Caller, Capability, RequestContext, Service};
pub use sync::{SyncEngine, SyncReport};
pub use vault::{markdown_files, safe_relative_path, Page, Vault};

/// Crate version, surfaced to clients for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
