//! Local ONNX embedding backend via `fastembed`.
//!
//! Compiled only under the `onnx` feature. The model is loaded from local files via
//! [`FastEmbedder::from_model_dir`] with **no network access** — the `hf-hub` download path
//! is not even compiled in (see this crate's `Cargo.toml`).

use std::path::Path;
use std::sync::Mutex;

use fastembed::{
    InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
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
    /// Load a model from a local directory with **zero network access**. The directory must
    /// contain the ONNX weights and the four tokenizer files:
    ///
    /// - one of `model.onnx` / `model_quantized.onnx` / `model_int8.onnx` / `model_optimized.onnx`
    /// - `tokenizer.json`, `config.json`, `tokenizer_config.json`, `special_tokens_map.json`
    ///
    /// Pooling is CLS and quantization is treated as dynamic, matching the int8 BGE-small
    /// export we vendor (`DynamicQuantizeLinear`). `dim` is the embedding width (384 for
    /// BGE-small) and `name` tags the stored vectors via [`Embedder::model_id`].
    pub fn from_model_dir(
        dir: impl AsRef<Path>,
        dim: usize,
        name: &str,
    ) -> Result<Self, EmbedError> {
        let dir = dir.as_ref();
        let read = |file: &str| -> Result<Vec<u8>, EmbedError> {
            std::fs::read(dir.join(file))
                .map_err(|e| EmbedError::new(format!("reading {file} from {}: {e}", dir.display())))
        };
        let onnx = [
            "model.onnx",
            "model_quantized.onnx",
            "model_int8.onnx",
            "model_optimized.onnx",
        ]
        .iter()
        .find_map(|f| std::fs::read(dir.join(f)).ok())
        .ok_or_else(|| EmbedError::new(format!("no ONNX model file found in {}", dir.display())))?;
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json")?,
            tokenizer_config_file: read("tokenizer_config.json")?,
        };
        Self::from_bytes(onnx, tokenizer_files, dim, name)
    }

    /// Load a model from in-memory bytes (the ONNX weights and the four tokenizer files), with
    /// **zero network and zero filesystem access**. This is how the vendored, compile-time
    /// embedded model is loaded (see the `vendored-model` feature): the bytes come straight from
    /// the binary's read-only data, never touching disk. Same pooling/quantization contract as
    /// [`Self::from_model_dir`].
    pub fn from_model_bytes(
        onnx: Vec<u8>,
        tokenizer: Vec<u8>,
        config: Vec<u8>,
        special_tokens_map: Vec<u8>,
        tokenizer_config: Vec<u8>,
        dim: usize,
        name: &str,
    ) -> Result<Self, EmbedError> {
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: tokenizer,
            config_file: config,
            special_tokens_map_file: special_tokens_map,
            tokenizer_config_file: tokenizer_config,
        };
        Self::from_bytes(onnx, tokenizer_files, dim, name)
    }

    /// Shared constructor for [`Self::from_model_dir`] and [`Self::from_model_bytes`]: build the
    /// `fastembed` model from already-loaded bytes. Pooling is CLS and quantization is dynamic,
    /// matching the int8 BGE-small export.
    fn from_bytes(
        onnx: Vec<u8>,
        tokenizer_files: TokenizerFiles,
        dim: usize,
        name: &str,
    ) -> Result<Self, EmbedError> {
        let model = UserDefinedEmbeddingModel::new(onnx, tokenizer_files)
            .with_pooling(Pooling::Cls)
            .with_quantization(QuantizationMode::Dynamic);
        let inner = TextEmbedding::try_new_from_user_defined(model, InitOptionsUserDefined::new())
            .map_err(EmbedError::new)?;
        Ok(FastEmbedder {
            model: Mutex::new(inner),
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
        guard.embed(texts, None).map_err(EmbedError::new)
    }

    fn model_id(&self) -> String {
        format!("fastembed-{}-{}", self.name, self.dim)
    }
}
