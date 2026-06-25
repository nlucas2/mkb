---
title: "README: Install — container"
---

## Container / Kubernetes

Run the daemon as a networked, token-gated service — the daemon has the embedding model
compiled in, so semantic search works offline. Thin clients reach it over TCP with `MKB_REMOTE` +
`MKB_TOKEN`.

```sh
# on the host (set a real token; replace <registry>)
docker run -d --name mkb -p 127.0.0.1:7820:7820 \
  -v ~/mkb-vault:/vault \
  <registry>/mkb:latest --vault /vault --listen 0.0.0.0:7820 --token "$MKB_TOKEN"

# from a client — point the desktop app (Settings → Remote daemon) or the CLI/MCP at it
mkb search --remote 127.0.0.1:7820 --token "$MKB_TOKEN" "…"
```

See [`deploy/README.md`](./deploy/README.md) for the Kubernetes manifest and full cluster setup.
