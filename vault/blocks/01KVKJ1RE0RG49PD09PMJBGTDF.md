---
title: "README: Configuration — choosing an embedder"
tags: [doc, readme]
---

## Configuration: choosing an embedder (`config.json`)

The embedder backend is configurable per vault via an optional `<vault>/.mdkb/config.json`.
The model is **never downloaded at runtime** — local models are loaded from disk, and the
shipped container image bakes the model in. The `embedder` block selects the source:

```jsonc
// 1. default / file absent → bundled vendored model (the container ships one); falls back
//    to the offline hash embedder if no bundled model is present (e.g. a plain `cargo run`).
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

The bundled model directory is resolved from `$MDKB_BUNDLED_MODEL_DIR`, else a `model/`
directory beside the binary. Any misconfiguration (missing model, unreachable endpoint)
logs a warning and falls back to the hash embedder, so the tool always keeps working.
The `onnx` (local models) and `remote` (HTTP endpoint) backends are opt-in cargo features;
default builds pull neither and stay fully offline.
