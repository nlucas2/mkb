---
title: "README: Configuration — choosing an embedder"
tags: [doc, readme]
---

## Configuration: choosing an embedder (`config.json`)

The embedder backend is configurable per vault via an optional `config.json` in the vault's
machine-local index directory (the same dir as the index/socket — see the SPEC layout; set
`$MDKB_INDEX_DIR` to relocate it).
The model is **never downloaded at runtime** — the default neural model is compiled into the
daemon, and any other local model is loaded from disk. The `embedder` block selects the source:

```jsonc
// 1. default / file absent → the model compiled into the daemon (real neural semantic search,
//    offline, zero config); falls back to the offline hash embedder only when the daemon was
//    built without the embedded model (`--no-default-features`).
{ "embedder": { "kind": "bundled" } }

// 2. a different ONNX model directory on disk (ONNX weights + tokenizer files)
{ "embedder": { "kind": "local", "path": "/models/bge-large", "dim": 1024 } }

// 3. a remote OpenAI-compatible /v1/embeddings endpoint (vLLM, LM Studio, llama.cpp, TEI,
//    OpenAI). Build the daemon with the `remote` feature. The API key, if any, is read from
//    the named environment variable so it never lives in config.json.
{ "embedder": { "kind": "remote", "url": "http://vllm:8000/v1/embeddings",
                "model": "bge-m3", "api_key_env": "MDKB_EMBED_KEY", "dim": 1024 } }

// 4. force the offline, dependency-free hash embedder (no model)
{ "embedder": { "kind": "hash" } }
```

For `bundled`, a model directory on disk **overrides** the compiled-in one: set
`$MDKB_BUNDLED_MODEL_DIR` (or place a `model/` directory beside the binary) to point at a
different/newer model. Any misconfiguration (missing model, unreachable endpoint) logs a warning
and falls back to the hash embedder, so the tool always keeps working. The local neural model
(ONNX engine + vendored weights) is built in by default; the `remote` (HTTP endpoint) backend is
an opt-in cargo feature that adds a TLS client.
