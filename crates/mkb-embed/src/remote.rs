//! Remote embedding backend: OpenAI-compatible `/v1/embeddings`.
//!
//! Compiled only under the `remote` feature. Speaks the OpenAI embeddings wire format, which
//! vLLM, LM Studio, llama.cpp's server, text-embeddings-inference and OpenAI itself all
//! implement, so one client covers local and hosted servers alike. Blocking I/O (`ureq`)
//! matches the synchronous [`Embedder`] contract.

use mkb_core::{EmbedError, Embedder};
use serde::{Deserialize, Serialize};

/// An embedder backed by a remote OpenAI-compatible endpoint.
pub struct RemoteEmbedder {
    agent: ureq::Agent,
    url: String,
    model: String,
    api_key: Option<String>,
    dim: usize,
}

#[derive(Serialize)]
struct EmbeddingsRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingDatum>,
}

#[derive(Deserialize)]
struct EmbeddingDatum {
    embedding: Vec<f32>,
    #[serde(default)]
    index: usize,
}

impl RemoteEmbedder {
    /// Connect to `url` (a full `/v1/embeddings` URL) for `model`. When `api_key_env` is set,
    /// the bearer token is read from that environment variable (so secrets stay out of
    /// `config.json`). When `dim` is `None`, the dimensionality is probed from the server on
    /// construction — which also validates connectivity and auth before the daemon serves.
    pub fn new(
        url: &str,
        model: &str,
        api_key_env: Option<&str>,
        dim: Option<usize>,
    ) -> Result<Self, EmbedError> {
        let api_key = match api_key_env {
            Some(var) => Some(std::env::var(var).map_err(|_| {
                EmbedError::new(format!("api key environment variable `{var}` is not set"))
            })?),
            None => None,
        };
        let mut embedder = RemoteEmbedder {
            agent: ureq::Agent::new_with_defaults(),
            url: url.to_string(),
            model: model.to_string(),
            api_key,
            dim: dim.unwrap_or(0),
        };
        if embedder.dim == 0 {
            let probe = embedder.request(std::slice::from_ref(&"probe".to_string()))?;
            embedder.dim = probe
                .first()
                .map(Vec::len)
                .filter(|d| *d > 0)
                .ok_or_else(|| {
                    EmbedError::new("remote returned no embedding on dimension probe")
                })?;
        }
        Ok(embedder)
    }

    fn request(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let body = EmbeddingsRequest {
            model: &self.model,
            input: texts,
        };
        let mut req = self
            .agent
            .post(&self.url)
            .header("content-type", "application/json");
        if let Some(key) = &self.api_key {
            req = req.header("authorization", format!("Bearer {key}"));
        }
        // ureq treats non-2xx as an error by default, so a failed call surfaces here.
        let mut resp = req.send_json(&body).map_err(EmbedError::new)?;
        let parsed: EmbeddingsResponse = resp.body_mut().read_json().map_err(EmbedError::new)?;
        let mut data = parsed.data;
        // The spec returns results with an `index`; sort by it rather than trusting order.
        data.sort_by_key(|d| d.index);
        if data.len() != texts.len() {
            return Err(EmbedError::new(format!(
                "remote returned {} embeddings for {} inputs",
                data.len(),
                texts.len()
            )));
        }
        Ok(data.into_iter().map(|d| d.embedding).collect())
    }
}

impl Embedder for RemoteEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.request(texts)
    }

    fn model_id(&self) -> String {
        format!("remote-{}-{}", self.model, self.dim)
    }
}
