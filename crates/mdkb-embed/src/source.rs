//! Configurable embedder selection.
//!
//! [`EmbedderSource`] is the config knob (read from `config.json`): it picks which backend
//! [`from_source`] builds. The factory is infallible — if a configured backend can't be
//! initialised it logs to stderr and falls back to the offline [`HashEmbedder`], so the tool
//! always keeps working. Keeping the factory here (not in the daemon/CLI) means every client
//! constructs embedders identically (see `AGENTS.md`).

use std::path::{Path, PathBuf};

use mdkb_core::{Embedder, HashEmbedder};
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
    /// Use the model vendored with the build (default). Resolved from
    /// `$MDKB_BUNDLED_MODEL_DIR`, else a `model/` directory beside the executable. Falls back
    /// to the hash embedder if no bundled model is present (e.g. a `cargo run` dev build).
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

/// The on-disk `config.json` schema (in `<vault>/.mdkb/`). Only the embedder block exists
/// today; the struct is the extension point for future daemon/CLI settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileConfig {
    /// Which embedder backend to use. Absent → [`EmbedderSource::Bundled`].
    #[serde(default)]
    pub embedder: EmbedderSource,
}

impl FileConfig {
    /// Load `config.json` from the given `.mdkb` directory. A missing file yields defaults
    /// (so the config file is entirely optional); a present-but-malformed file logs a warning
    /// and also falls back to defaults rather than failing the daemon.
    pub fn load(mdkb_dir: impl AsRef<Path>) -> FileConfig {
        let path = mdkb_dir.as_ref().join("config.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!(
                    "mdkb-embed: ignoring malformed {}: {e}; using defaults",
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
        EmbedderSource::Bundled => match bundled_model_dir() {
            Some(dir) => load_local(&dir, FALLBACK_DIM, "bundled"),
            None => {
                eprintln!(
                    "mdkb-embed: no bundled model found (set MDKB_BUNDLED_MODEL_DIR or place a \
                     `model/` dir beside the binary); using offline hash embedder"
                );
                Box::new(HashEmbedder::new(FALLBACK_DIM))
            }
        },
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

/// Resolve the bundled model directory: `$MDKB_BUNDLED_MODEL_DIR`, then `<exe_dir>/model`.
fn bundled_model_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MDKB_BUNDLED_MODEL_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join("model");
    candidate.is_dir().then_some(candidate)
}

#[cfg(feature = "onnx")]
fn load_local(path: &Path, dim: usize, label: &str) -> Box<dyn Embedder> {
    match crate::FastEmbedder::from_model_dir(path, dim, label) {
        Ok(e) => Box::new(e),
        Err(e) => {
            eprintln!(
                "mdkb-embed: failed to load {label} model from {}: {e}; using offline hash embedder",
                path.display()
            );
            Box::new(HashEmbedder::new(FALLBACK_DIM))
        }
    }
}

#[cfg(not(feature = "onnx"))]
fn load_local(path: &Path, _dim: usize, label: &str) -> Box<dyn Embedder> {
    eprintln!(
        "mdkb-embed: {label} model requested ({}) but this build lacks the `onnx` feature; \
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
                "mdkb-embed: failed to init remote embedder at {url}: {e}; using offline hash embedder"
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
        "mdkb-embed: remote embedder requested ({url}) but this build lacks the `remote` \
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
        // No MDKB_BUNDLED_MODEL_DIR and no model beside the test binary → hash fallback,
        // which must still produce vectors so the tool keeps working.
        std::env::remove_var("MDKB_BUNDLED_MODEL_DIR");
        let e = from_source(&EmbedderSource::Bundled);
        assert!(!e.embed_one("hello world").unwrap().is_empty());
    }

    #[test]
    fn file_config_missing_is_default_bundled() {
        // A directory with no config.json loads defaults.
        let dir = std::env::temp_dir().join("mdkb-embed-noconfig-test");
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("config.json"));
        let cfg = FileConfig::load(&dir);
        assert_eq!(cfg.embedder, EmbedderSource::Bundled);
    }

    #[test]
    fn file_config_parses_embedder_block() {
        let dir = std::env::temp_dir().join("mdkb-embed-config-test");
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
        let dir = std::env::temp_dir().join("mdkb-embed-badconfig-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), "{ not valid json").unwrap();
        let cfg = FileConfig::load(&dir);
        assert_eq!(cfg.embedder, EmbedderSource::Bundled);
        let _ = std::fs::remove_file(dir.join("config.json"));
    }
}
