---
title: "README: Install — container"
---

## Container / Kubernetes

Run the daemon as a networked, token-gated service — the daemon has the embedding model
compiled in, so semantic search works offline. Thin clients reach it over TCP with `MDKB_REMOTE` +
`MDKB_TOKEN`.

```sh
# on the host (set a real token; replace <registry>)
docker run -d --name mdkb -p 127.0.0.1:7820:7820 \
  -v ~/mdkb-vault:/vault \
  <registry>/mdkb:latest --vault /vault --listen 0.0.0.0:7820 --token "$MDKB_TOKEN"

# from a client — point the desktop app (Settings → Remote daemon) or the CLI/MCP at it
mdkb search --remote 127.0.0.1:7820 --token "$MDKB_TOKEN" "…"
```

See [`deploy/README.md`](./deploy/README.md) for the Kubernetes manifest and full cluster setup.
