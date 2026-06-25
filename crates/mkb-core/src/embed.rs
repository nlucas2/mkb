//! Text embedding: turning block text into vectors for semantic search.
//!
//! The [`Embedder`] trait abstracts the model so the real backend (a local ONNX model via
//! `fastembed`, in the `mkb-embed` crate) can be swapped for the deterministic
//! [`HashEmbedder`] used in tests and as an offline fallback. Keeping the trait in core
//! means every consumer embeds identically (see `AGENTS.md`).

use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};

/// An embedding error.
#[derive(Debug)]
pub struct EmbedError(pub String);

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "embed error: {}", self.0)
    }
}

impl Error for EmbedError {}

impl EmbedError {
    /// Wrap any displayable error.
    pub fn new(e: impl fmt::Display) -> EmbedError {
        EmbedError(e.to_string())
    }
}

/// Produces vector embeddings for text.
pub trait Embedder: Send + Sync {
    /// The dimensionality of produced vectors.
    fn dim(&self) -> usize;

    /// Embed a batch of texts. The output has one vector per input, in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// Embed a single text.
    fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut out = self.embed(std::slice::from_ref(&text.to_string()))?;
        out.pop()
            .ok_or_else(|| EmbedError::new("embedder returned no vector"))
    }

    /// A short identifier for the model (used to invalidate cached vectors when the model
    /// changes).
    fn model_id(&self) -> String {
        format!("unknown-{}", self.dim())
    }
}

/// A deterministic, dependency-free embedder.
///
/// It hashes whitespace/punctuation-delimited tokens into a fixed-width bag-of-tokens
/// vector and L2-normalises the result. It is **not** semantic in the neural sense, but it
/// is fully offline and deterministic — texts that share tokens get higher cosine
/// similarity — which makes it ideal for tests and a safe fallback when no model is
/// available.
#[derive(Debug, Clone)]
pub struct HashEmbedder {
    dim: usize,
}

impl HashEmbedder {
    /// Create a hash embedder of the given dimensionality.
    pub fn new(dim: usize) -> Self {
        assert!(dim > 0, "embedding dim must be positive");
        HashEmbedder { dim }
    }
}

impl Default for HashEmbedder {
    fn default() -> Self {
        HashEmbedder::new(256)
    }
}

fn token_hash(token: &str) -> u64 {
    let mut h = DefaultHasher::new();
    token.hash(&mut h);
    h.finish()
}

impl Embedder for HashEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            let mut v = vec![0.0f32; self.dim];
            for token in text
                .split(|c: char| !c.is_alphanumeric())
                .filter(|t| !t.is_empty())
            {
                let token = token.to_lowercase();
                let idx = (token_hash(&token) % self.dim as u64) as usize;
                // Signed contribution decorrelates collisions a little.
                let sign = if token_hash(&token) & 1 == 0 {
                    1.0
                } else {
                    -1.0
                };
                v[idx] += sign;
            }
            normalize(&mut v);
            out.push(v);
        }
        Ok(out)
    }

    fn model_id(&self) -> String {
        format!("hash-{}", self.dim)
    }
}

fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Cosine similarity between two equal-length vectors. Returns 0.0 on a length mismatch or
/// a zero vector.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Serialize a vector to little-endian f32 bytes (for BLOB storage).
pub fn vector_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Deserialize little-endian f32 bytes back into a vector.
pub fn bytes_to_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_embedder_is_deterministic() {
        let e = HashEmbedder::new(64);
        let a = e.embed_one("restart the nginx server").unwrap();
        let b = e.embed_one("restart the nginx server").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn shared_tokens_increase_similarity() {
        let e = HashEmbedder::new(512);
        let q = e.embed_one("restart nginx").unwrap();
        let related = e.embed_one("how to restart the nginx service").unwrap();
        let unrelated = e.embed_one("favourite pizza toppings list").unwrap();
        let s_related = cosine_similarity(&q, &related);
        let s_unrelated = cosine_similarity(&q, &unrelated);
        assert!(
            s_related > s_unrelated,
            "related {s_related} should beat unrelated {s_unrelated}"
        );
    }

    #[test]
    fn normalised_vectors_are_unit_length() {
        let e = HashEmbedder::new(128);
        let v = e.embed_one("some text here").unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn bytes_round_trip() {
        let v = vec![0.5f32, -0.25, 1.0, 0.0];
        assert_eq!(bytes_to_vector(&vector_to_bytes(&v)), v);
    }
}
