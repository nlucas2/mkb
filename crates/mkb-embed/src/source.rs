//! Configurable embedder selection.
//!
//! [`EmbedderSource`] is the config knob (read from `config.json`): it picks which backend
//! [`from_source`] builds. The factory is infallible — if a configured backend can't be
//! initialised it logs to stderr and falls back to the offline [`HashEmbedder`], so the tool
//! always keeps working. Keeping the factory here (not in the daemon/CLI) means every client
//! constructs embedders identically (see `AGENTS.md`).

use std::path::{Path, PathBuf};

use mkb_core::{Embedder, HashEmbedder};
use serde::{Deserialize, Serialize};

use crate::FALLBACK_DIM;

/// Where the embedding model comes from.
///
/// Serialised in `config.json` under an `embedder` key with a `kind` tag, e.g.
/// `{"kind":"local","path":"/models/bge","dim":384}`. An absent/unset embedder block means
/// [`EmbedderSource::Bundled`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EmbedderSource {
    /// Use the model that ships with the build (default). Resolved in order: a model directory
    /// on disk (`$MKB_BUNDLED_MODEL_DIR`, else a `model/` dir beside the executable), then the
    /// compiled-in vendored model (the `vendored-model` feature), then the offline hash embedder
    /// as a last resort.
    #[default]
    Bundled,

    /// Load a local ONNX model directory from `path` (requires the `onnx` feature).
    Local {
        /// Directory containing the ONNX model and tokenizer files.
        path: PathBuf,
        /// Embedding width; defaults to 384 (BGE-small) when omitted.
        #[serde(default)]
        dim: Option<usize>,
    },

    /// Call a remote OpenAI-compatible `/v1/embeddings` endpoint (requires the `remote`
    /// feature). The API key, if any, is read from the named environment variable so secrets
    /// never live in `config.json`.
    Remote {
        /// Full endpoint URL, e.g. `http://vllm:8000/v1/embeddings`.
        url: String,
        /// Model name to request from the server.
        model: String,
        /// Environment variable holding the bearer token (optional for keyless local servers).
        #[serde(default)]
        api_key_env: Option<String>,
        /// Embedding width; probed from the server on first use when omitted.
        #[serde(default)]
        dim: Option<usize>,
    },

    /// Force the offline, dependency-free hash embedder (no model).
    Hash,
}

/// The on-disk `config.json` schema (in `<vault>/.mkb/`). Only the embedder block exists
/// today; the struct is the extension point for future daemon/CLI settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileConfig {
    /// Which embedder backend to use. Absent → [`EmbedderSource::Bundled`].
    #[serde(default)]
    pub embedder: EmbedderSource,
}

impl FileConfig {
    /// Load `config.json` from the given `.mkb` directory. A missing file yields defaults
    /// (so the config file is entirely optional); a present-but-malformed file logs a warning
    /// and also falls back to defaults rather than failing the daemon.
    pub fn load(mkb_dir: impl AsRef<Path>) -> FileConfig {
        let path = mkb_dir.as_ref().join("config.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!(
                    "mkb-embed: ignoring malformed {}: {e}; using defaults",
                    path.display()
                );
                FileConfig::default()
            }),
            Err(_) => FileConfig::default(),
        }
    }
}

/// Build the embedder for a given source, falling back to the hash embedder (with a stderr
/// warning) if the configured backend can't be initialised.
pub fn from_source(source: &EmbedderSource) -> Box<dyn Embedder> {
    match source {
        EmbedderSource::Hash => Box::new(HashEmbedder::new(FALLBACK_DIM)),
        EmbedderSource::Bundled => bundled_embedder(),
        EmbedderSource::Local { path, dim } => {
            load_local(path, dim.unwrap_or(FALLBACK_DIM), "local")
        }
        EmbedderSource::Remote {
            url,
            model,
            api_key_env,
            dim,
        } => load_remote(url, model, api_key_env.as_deref(), *dim),
    }
}

/// Resolve the default ("bundled") embedder, in priority order:
///
/// 1. A model **directory on disk** — `$MKB_BUNDLED_MODEL_DIR`, else a `model/` dir beside the
///    executable (the release layout). This lets an operator point at a newer/different model.
/// 2. The **compiled-in** model (the `vendored-model` feature), loaded straight from the binary
///    with no disk or network access — the default for a normal build.
/// 3. The offline **hash embedder** — when neither is available (e.g. a build that opted out of
///    the vendored model and has no model on disk).
fn bundled_embedder() -> Box<dyn Embedder> {
    if let Some(dir) = bundled_model_dir() {
        return load_local(&dir, FALLBACK_DIM, "bundled");
    }
    #[cfg(feature = "vendored-model")]
    if let Some(e) = crate::embedded::load() {
        return e;
    }
    eprintln!(
        "mkb-embed: no embedding model available; semantic search is degraded to the offline \
         hash embedder. Point MKB_BUNDLED_MODEL_DIR at a model directory, or build with the \
         (default) `vendored-model` feature."
    );
    Box::new(HashEmbedder::new(FALLBACK_DIM))
}

/// Resolve a model **directory** placed on disk: `$MKB_BUNDLED_MODEL_DIR`, else `<exe_dir>/model`
/// (the vendored release layout). `None` when neither exists — callers then fall back to the
/// compiled-in model or the hash embedder.
fn bundled_model_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MKB_BUNDLED_MODEL_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("model");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(feature = "onnx")]
fn load_local(path: &Path, dim: usize, label: &str) -> Box<dyn Embedder> {
    match crate::FastEmbedder::from_model_dir(path, dim, label) {
        Ok(e) => Box::new(e),
        Err(e) => {
            eprintln!(
                "mkb-embed: failed to load {label} model from {}: {e}; using offline hash embedder",
                path.display()
            );
            Box::new(HashEmbedder::new(FALLBACK_DIM))
        }
    }
}

#[cfg(not(feature = "onnx"))]
fn load_local(path: &Path, _dim: usize, label: &str) -> Box<dyn Embedder> {
    eprintln!(
        "mkb-embed: {label} model requested ({}) but this build lacks the `onnx` feature; \
         using offline hash embedder",
        path.display()
    );
    Box::new(HashEmbedder::new(FALLBACK_DIM))
}

#[cfg(feature = "remote")]
fn load_remote(
    url: &str,
    model: &str,
    api_key_env: Option<&str>,
    dim: Option<usize>,
) -> Box<dyn Embedder> {
    match crate::RemoteEmbedder::new(url, model, api_key_env, dim) {
        Ok(e) => Box::new(e),
        Err(e) => {
            eprintln!(
                "mkb-embed: failed to init remote embedder at {url}: {e}; using offline hash embedder"
            );
            Box::new(HashEmbedder::new(FALLBACK_DIM))
        }
    }
}

#[cfg(not(feature = "remote"))]
fn load_remote(
    url: &str,
    _model: &str,
    _api_key_env: Option<&str>,
    _dim: Option<usize>,
) -> Box<dyn Embedder> {
    eprintln!(
        "mkb-embed: remote embedder requested ({url}) but this build lacks the `remote` \
         feature; using offline hash embedder"
    );
    Box::new(HashEmbedder::new(FALLBACK_DIM))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_source_is_bundled() {
        assert_eq!(EmbedderSource::default(), EmbedderSource::Bundled);
    }

    #[test]
    fn deserializes_each_kind() {
        let bundled: EmbedderSource = serde_json::from_str(r#"{"kind":"bundled"}"#).unwrap();
        assert_eq!(bundled, EmbedderSource::Bundled);

        let local: EmbedderSource =
            serde_json::from_str(r#"{"kind":"local","path":"/m/bge","dim":384}"#).unwrap();
        assert_eq!(
            local,
            EmbedderSource::Local {
                path: "/m/bge".into(),
                dim: Some(384)
            }
        );

        let local_no_dim: EmbedderSource =
            serde_json::from_str(r#"{"kind":"local","path":"/m/bge"}"#).unwrap();
        assert_eq!(
            local_no_dim,
            EmbedderSource::Local {
                path: "/m/bge".into(),
                dim: None
            }
        );

        let remote: EmbedderSource = serde_json::from_str(
            r#"{"kind":"remote","url":"http://h/v1/embeddings","model":"bge-m3","api_key_env":"K"}"#,
        )
        .unwrap();
        assert_eq!(
            remote,
            EmbedderSource::Remote {
                url: "http://h/v1/embeddings".into(),
                model: "bge-m3".into(),
                api_key_env: Some("K".into()),
                dim: None,
            }
        );

        let hash: EmbedderSource = serde_json::from_str(r#"{"kind":"hash"}"#).unwrap();
        assert_eq!(hash, EmbedderSource::Hash);
    }

    #[test]
    fn hash_source_builds_a_working_embedder() {
        let e = from_source(&EmbedderSource::Hash);
        assert_eq!(e.dim(), FALLBACK_DIM);
        assert!(!e.embed_one("hello").unwrap().is_empty());
    }

    #[test]
    fn bundled_without_model_falls_back_to_hash() {
        // No MKB_BUNDLED_MODEL_DIR and no model beside the test binary → hash fallback,
        // which must still produce vectors so the tool keeps working.
        std::env::remove_var("MKB_BUNDLED_MODEL_DIR");
        let e = from_source(&EmbedderSource::Bundled);
        assert!(!e.embed_one("hello world").unwrap().is_empty());
    }

    #[test]
    fn file_config_missing_is_default_bundled() {
        // A directory with no config.json loads defaults.
        let dir = std::env::temp_dir().join("mkb-embed-noconfig-test");
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("config.json"));
        let cfg = FileConfig::load(&dir);
        assert_eq!(cfg.embedder, EmbedderSource::Bundled);
    }

    #[test]
    fn file_config_parses_embedder_block() {
        let dir = std::env::temp_dir().join("mkb-embed-config-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            r#"{"embedder":{"kind":"local","path":"/m/bge","dim":384}}"#,
        )
        .unwrap();
        let cfg = FileConfig::load(&dir);
        assert_eq!(
            cfg.embedder,
            EmbedderSource::Local {
                path: "/m/bge".into(),
                dim: Some(384)
            }
        );
        let _ = std::fs::remove_file(dir.join("config.json"));
    }

    #[test]
    fn file_config_malformed_falls_back_to_default() {
        let dir = std::env::temp_dir().join("mkb-embed-badconfig-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), "{ not valid json").unwrap();
        let cfg = FileConfig::load(&dir);
        assert_eq!(cfg.embedder, EmbedderSource::Bundled);
        let _ = std::fs::remove_file(dir.join("config.json"));
    }
}
