---
title: "README: Running the daemon"
tags: [doc, readme]
---

### Running the daemon

```sh
# start the daemon (defaults: vault ~/mdkb-vault, socket <vault>/.mdkb/mdkbd.sock)
cargo run -p mdkbd -- --vault ./my-vault

# from another shell, the CLI auto-connects to (and would auto-start) that vault's daemon
cargo run -p mdkb-cli -- ping ./my-vault
cargo run -p mdkb-cli -- stats ./my-vault
cargo run -p mdkb-cli -- search ./my-vault "restart the web server"
```

By default the offline hash embedder is used (deterministic, no downloads). For real
semantic embeddings via a local ONNX model, run the **daemon** with the `onnx` feature (the
daemon owns embedding; the CLI is a thin client and needs no embedder):

```sh
cargo run -p mdkbd --features onnx -- --vault ./my-vault
```
