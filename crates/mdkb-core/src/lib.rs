//! `mdkb-core` — the shared engine for mdkb.
//!
//! All behavior that touches blocks, transclusion, indexing, search, parsing, or writes lives
//! here. The daemon, MCP server, CLI, and Tauri app are **thin clients** of this crate
//! so a bug fixed once is fixed everywhere. See `AGENTS.md`.
//!
//! The model is **file-per-block**: each block is one file (`blocks/<ulid>.md`); `![[id]]`
//! marks a child (transclusion), `[[id]]` a reference. See `docs/architecture.md`.

pub mod block;
pub mod blockfile;
pub mod clock;
pub mod conflict;
pub mod dirs;
pub mod embed;
pub mod export;
pub mod id;
pub mod index;
pub mod link;
pub mod render;
pub mod service;
pub mod sync;
pub mod vault;

pub use block::Block;
pub use blockfile::{parse_block, write_block};
pub use conflict::{is_conflict_path, CONFLICT_MARKERS};
pub use embed::{
    bytes_to_vector, cosine_similarity, vector_to_bytes, EmbedError, Embedder, HashEmbedder,
};
pub use export::{plan_doc, plan_exports, ExportEntry, Manifest, PlannedDoc, GENERATED_MARKER};
pub use id::{BlockId, IdCodec, IdError, MarkerMatch, NativeIdCodec};
pub use index::{
    block_links, link_graph, reciprocal_rank_fusion, transclusion_reaches, BlockRecord, GraphData,
    GraphEdge, GraphNode, Index, IndexError, IndexStats, LinkKind, LinkOutcome, LinkRow, SearchHit,
    SearchQuery, TagCount,
};
pub use link::{extract_references, Reference};
pub use render::{render_block, render_flat, rendered_block, RenderedBlock};
pub use service::{Caller, Capability, RequestContext, Scope, Service};
pub use sync::{append_text, exact_replace, slice_lines, SyncEngine, SyncReport};
pub use vault::{
    block_rel_path, read_block_files, safe_relative_path, sanitize_asset_filename, Crumb, Lineage,
    Vault, ASSETS_DIR, BLOCKS_DIR,
};

/// Crate version, surfaced to clients for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
