//! `mdkb-core` — the shared engine for mdkb.
//!
//! All behavior that touches blocks, transclusion, indexing, search, parsing, or
//! writes lives here. The daemon, MCP server, CLI, and Tauri UI are **thin clients**
//! of this crate so a bug fixed once is fixed everywhere. See `AGENTS.md`.

pub mod id;

pub use id::{BlockId, IdCodec, IdError, MarkerMatch, NativeIdCodec};

/// Crate version, surfaced to clients for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
