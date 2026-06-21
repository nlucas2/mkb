---
title: "README: Running the daemon manually"
tags: [doc, readme]
---

### Running the daemon manually

You normally never do this — every client auto-starts and reuses the daemon. Run it yourself
only to keep a vault warm, expose it over the network, or run it as a service:

```sh
mdkbd --vault ~/my-vault            # serves ~/my-vault/.mdkb/mdkbd.sock

# from another shell, clients connect to (or would auto-start) that vault's daemon
mdkb ping  ~/my-vault
mdkb stats ~/my-vault
mdkb search ~/my-vault "restart the web server"
```

By default the offline hash embedder is used (deterministic, no downloads). For real semantic
embeddings from a local ONNX model, the **daemon** owns embedding (clients are thin and need no
embedder). Release builds already include the `onnx` backend and a bundled model; from source,
enable the feature:

```sh
cargo run -p mdkbd --features onnx -- --vault ~/my-vault
```
