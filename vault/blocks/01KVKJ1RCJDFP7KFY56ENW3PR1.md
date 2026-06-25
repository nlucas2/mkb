---
title: "README: Running the daemon manually"
tags: [doc, readme]
---

### Running the daemon manually

You normally never do this — every client auto-starts and reuses the daemon. Run it yourself
only to keep a vault warm, expose it over the network, or run it as a service:

```sh
mkbd --vault ~/my-vault            # serves ~/my-vault's daemon

# from another shell, clients connect to (or would auto-start) that vault's daemon
mkb ping  --vault ~/my-vault
mkb stats --vault ~/my-vault
mkb search --vault ~/my-vault "restart the web server"
```

The **daemon** owns embedding (clients are thin and need no embedder), and semantic search works
out of the box: the neural model is compiled into `mkbd` by default, so a plain
`cargo run -p mkbd` — or any release build — does real semantic embeddings with no model files
and no download. The offline hash embedder is only a fallback, used when the daemon was built
without the embedded model (`--no-default-features`).
