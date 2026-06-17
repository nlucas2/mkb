//! Embedder backends for mdkb.
//!
//! The [`mdkb_core::Embedder`] trait is the contract. This crate provides the concrete
//! backends and a [`recommended`] factory:
//!
//! - Always available: [`mdkb_core::HashEmbedder`] — deterministic, offline, dependency-free.
//! - With the `onnx` feature: [`FastEmbedder`] — a real local model via `fastembed`
//!   (downloads the model weights once, then runs fully offline/in-process).
//!
//! Default builds do **not** pull `fastembed`, keeping the standard test path light and
//! offline. The daemon enables `onnx` to get semantic embeddings.

pub use mdkb_core::{EmbedError, Embedder, HashEmbedder};

#[cfg(feature = "onnx")]
mod fast;
#[cfg(feature = "onnx")]
pub use fast::FastEmbedder;

/// The embedder dimensionality used by the offline fallback.
pub const FALLBACK_DIM: usize = 384;

/// Build the recommended embedder for this build.
///
/// With the `onnx` feature this tries to initialise a local model and falls back to the
/// hash embedder (logging to stderr) if the model cannot be loaded — so the tool always
/// keeps working, online or off.
pub fn recommended() -> Box<dyn Embedder> {
    #[cfg(feature = "onnx")]
    {
        match FastEmbedder::new() {
            Ok(e) => return Box::new(e),
            Err(e) => eprintln!("mdkb-embed: falling back to offline hash embedder ({e})"),
        }
    }
    Box::new(HashEmbedder::new(FALLBACK_DIM))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommended_returns_a_working_embedder() {
        // Without the `onnx` feature this is the hash embedder; with it, fastembed or the
        // hash fallback. Either way it must embed.
        let e = recommended();
        let v = e.embed_one("hello world").unwrap();
        assert!(!v.is_empty());
    }
}
