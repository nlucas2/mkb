//! The vendored embedding model, compiled into the binary.
//!
//! Under the `vendored-model` feature the five BGE-small files in `assets/model/` are embedded
//! via [`include_bytes!`], so the daemon carries its own model: semantic search works with no
//! files on disk, no download, and no network — the bytes load straight from the binary's
//! read-only data into `fastembed` (see [`crate::FastEmbedder::from_model_bytes`]).
//!
//! The model is BGE-small-en-v1.5 (int8 ONNX), 384-dimensional, vendored from
//! `Xenova/bge-small-en-v1.5` (upstream `BAAI/bge-small-en-v1.5`, MIT licensed). Full source URLs,
//! license, and checksum-pinned hashes are recorded in `assets/model/PROVENANCE.md`.

use mkb_core::Embedder;

use crate::FastEmbedder;

/// Embedding width of the vendored BGE-small model.
const DIM: usize = 384;

/// Tag recorded with vectors this model produces (so only same-model vectors are compared).
const NAME: &str = "vendored-bge-small";

const ONNX: &[u8] = include_bytes!("../assets/model/model_quantized.onnx");
const TOKENIZER: &[u8] = include_bytes!("../assets/model/tokenizer.json");
const CONFIG: &[u8] = include_bytes!("../assets/model/config.json");
const SPECIAL_TOKENS_MAP: &[u8] = include_bytes!("../assets/model/special_tokens_map.json");
const TOKENIZER_CONFIG: &[u8] = include_bytes!("../assets/model/tokenizer_config.json");

/// Build an embedder from the compiled-in model, or `None` if it fails to initialise (in which
/// case the caller degrades to the offline hash embedder, with a warning).
pub fn load() -> Option<Box<dyn Embedder>> {
    match FastEmbedder::from_model_bytes(
        ONNX.to_vec(),
        TOKENIZER.to_vec(),
        CONFIG.to_vec(),
        SPECIAL_TOKENS_MAP.to_vec(),
        TOKENIZER_CONFIG.to_vec(),
        DIM,
        NAME,
    ) {
        Ok(e) => Some(Box::new(e)),
        Err(e) => {
            eprintln!("mkb-embed: vendored model failed to load: {e}; using offline hash embedder");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_bytes_are_present_and_plausible() {
        // The ONNX weights are the large file; the tokenizer/config are small JSON. Guard against
        // an empty/zeroed include (e.g. a missing LFS pointer) by asserting non-trivial sizes.
        assert!(ONNX.len() > 1_000_000, "onnx weights look too small");
        assert!(TOKENIZER.len() > 10_000, "tokenizer.json looks too small");
        assert!(!CONFIG.is_empty());
        assert!(!SPECIAL_TOKENS_MAP.is_empty());
        assert!(!TOKENIZER_CONFIG.is_empty());
    }

    #[test]
    fn vendored_model_embeds_and_is_semantic() {
        let e = load().expect("vendored model should load");
        assert_eq!(e.dim(), DIM);
        let v = e.embed_one("restart the nginx server").unwrap();
        assert_eq!(v.len(), DIM);

        // A real neural model places a paraphrase nearer than an unrelated sentence — the
        // property the hash embedder can't provide. This is the whole point of vendoring it.
        let q = e.embed_one("reboot the web server").unwrap();
        let related = e.embed_one("restart the nginx service").unwrap();
        let unrelated = e.embed_one("a recipe for chocolate cake").unwrap();
        let sim = |a: &[f32], b: &[f32]| -> f32 { a.iter().zip(b).map(|(x, y)| x * y).sum() };
        assert!(
            sim(&q, &related) > sim(&q, &unrelated),
            "paraphrase must score higher than an unrelated sentence"
        );
    }
}
