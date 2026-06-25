//! Embedder backends for mkb.
//!
//! The [`mkb_core::Embedder`] trait is the contract. This crate provides the concrete
//! backends and a config-driven [`from_source`] factory:
//!
//! - Always available: [`mkb_core::HashEmbedder`] — deterministic, offline, dependency-free.
//! - With the `onnx` feature: [`FastEmbedder`] — a local ONNX model via `fastembed`, loaded
//!   from vendored/local files with **no network access**.
//! - With the `remote` feature: [`RemoteEmbedder`] — an OpenAI-compatible `/v1/embeddings`
//!   endpoint (vLLM, LM Studio, llama.cpp, TEI, OpenAI).
//!
//! Which backend is used is chosen by [`EmbedderSource`] (read from `config.json`). Default
//! builds pull none of the optional backends, keeping the standard test path light and
//! offline; the daemon enables `onnx` (and optionally `remote`).

pub use mkb_core::{EmbedError, Embedder, HashEmbedder};

#[cfg(feature = "onnx")]
mod fast;
#[cfg(feature = "onnx")]
pub use fast::FastEmbedder;

#[cfg(feature = "vendored-model")]
mod embedded;

#[cfg(feature = "remote")]
mod remote;
#[cfg(feature = "remote")]
pub use remote::RemoteEmbedder;

mod source;
pub use source::{from_source, EmbedderSource, FileConfig};

/// The embedder dimensionality used by the offline fallback.
pub const FALLBACK_DIM: usize = 384;

/// Build the recommended embedder for this build — equivalent to [`from_source`] with the
/// default [`EmbedderSource::Bundled`]: the vendored local model when present, otherwise the
/// offline hash embedder.
pub fn recommended() -> Box<dyn Embedder> {
    from_source(&EmbedderSource::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommended_returns_a_working_embedder() {
        // Without a bundled model (and without the `onnx` feature) this is the hash embedder;
        // either way it must embed.
        let e = recommended();
        let v = e.embed_one("hello world").unwrap();
        assert!(!v.is_empty());
    }
}
