//! `mdkb-core` — the shared engine for mdkb.
//!
//! All behavior that touches blocks, transclusion, indexing, search, parsing, or
//! writes lives here. The daemon, MCP server, CLI, and Tauri UI are **thin clients**
//! of this crate so a bug fixed once is fixed everywhere. See `AGENTS.md`.

pub mod block;
pub mod document;
pub mod id;
pub mod link;
pub mod render;
pub mod vault;

pub use block::{Block, BlockKind, Tag, TagSource};
pub use document::{Document, Frontmatter};
pub use id::{BlockId, IdCodec, IdError, MarkerMatch, NativeIdCodec};
pub use link::{extract_references, Anchor, LinkTarget, Reference};
pub use render::{render_block, render_page};
pub use vault::{Page, Vault};

/// Crate version, surfaced to clients for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
