# Vendored embedding model — provenance

These files are the **BGE-small-en-v1.5** sentence-embedding model (int8 ONNX export,
384-dimensional), vendored into the repository and compiled into the `mdkbd` daemon via
`include_bytes!` (the `vendored-model` feature, on by default). They give mdkb fully offline,
self-contained semantic search — no download, no network, no per-machine provisioning.

## Source

- **Upstream model:** `BAAI/bge-small-en-v1.5` — Beijing Academy of Artificial Intelligence.
  <https://huggingface.co/BAAI/bge-small-en-v1.5>
- **ONNX export vendored here:** `Xenova/bge-small-en-v1.5` (a faithful ONNX/Transformers.js port).
  <https://huggingface.co/Xenova/bge-small-en-v1.5>
- **Revision:** `main` at vendoring time (files are checksum-pinned below, so the exact bytes are
  fixed regardless of later upstream changes).

Downloaded from `https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/<path>`:

| Vendored file              | Upstream path                  |
| -------------------------- | ------------------------------ |
| `model_quantized.onnx`     | `onnx/model_quantized.onnx`    |
| `tokenizer.json`           | `tokenizer.json`               |
| `config.json`              | `config.json`                  |
| `tokenizer_config.json`    | `tokenizer_config.json`        |
| `special_tokens_map.json`  | `special_tokens_map.json`      |

## License

MIT, inherited from the upstream `BAAI/bge-small-en-v1.5`. Free for personal and commercial use
with attribution; the verbatim license text and copyright notice are vendored alongside the model
in [`LICENSE`](./LICENSE) (`Copyright (c) 2022 staoxiao`). The upstream model card explicitly
permits commercial use of the released models free of charge. Canonical source:
<https://github.com/FlagOpen/FlagEmbedding/blob/master/LICENSE> · <https://opensource.org/license/mit/>

## Integrity (SHA-256)

```
6c9c6101a956d62dfb5e7190c538226c0c5bb9cb27b651234b6df063ee7dbfe4  model_quantized.onnx
d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66  tokenizer.json
fa73f90bf92c8cace1fbcb709626306f2bdbc9ea3e5b5f94b440df9b6aa56350  config.json
9261e7d79b44c8195c1cada2b453e55b00aeb81e907a6664974b4d7776172ab3  tokenizer_config.json
b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3  special_tokens_map.json
```

Re-verify at any time:

```sh
shasum -a 256 -c <<'EOF'
6c9c6101a956d62dfb5e7190c538226c0c5bb9cb27b651234b6df063ee7dbfe4  model_quantized.onnx
d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66  tokenizer.json
fa73f90bf92c8cace1fbcb709626306f2bdbc9ea3e5b5f94b440df9b6aa56350  config.json
9261e7d79b44c8195c1cada2b453e55b00aeb81e907a6664974b4d7776172ab3  tokenizer_config.json
b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3  special_tokens_map.json
EOF
```
