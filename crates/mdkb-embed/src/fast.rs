//! Local ONNX embedding backend via `fastembed`.
//!
//! Compiled only under the `onnx` feature. The model weights are downloaded once on first
//! use (to fastembed's cache) and then run fully in-process/offline.

use std::sync::Mutex;

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use mdkb_core::{EmbedError, Embedder};

/// A local sentence-embedding model.
///
/// `fastembed`'s `embed` takes `&mut self`, so the model is held behind a [`Mutex`] to
/// satisfy the `&self` [`Embedder`] contract. Embedding is CPU-bound and batched, so the
/// lock is held only briefly per call.
pub struct FastEmbedder {
    model: Mutex<TextEmbedding>,
    dim: usize,
    name: String,
}

impl FastEmbedder {
    /// Initialise the default model (BGE-small-en-v1.5, 384-dim) — a strong, small,
    /// CPU-friendly default.
    pub fn new() -> Result<Self, EmbedError> {
        Self::with_model(EmbeddingModel::BGESmallENV15, 384, "BGESmallENV15")
    }

    /// Initialise a specific fastembed model. `dim` must match the model's output size.
    pub fn with_model(model: EmbeddingModel, dim: usize, name: &str) -> Result<Self, EmbedError> {
        let model = TextEmbedding::try_new(TextInitOptions::new(model)).map_err(EmbedError::new)?;
        Ok(FastEmbedder {
            model: Mutex::new(model),
            dim,
            name: name.to_string(),
        })
    }
}

impl Embedder for FastEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut guard = self
            .model
            .lock()
            .map_err(|_| EmbedError::new("embedder mutex poisoned"))?;
        guard.embed(texts.to_vec(), None).map_err(EmbedError::new)
    }

    fn model_id(&self) -> String {
        format!("fastembed-{}-{}", self.name, self.dim)
    }
}
