# Deploying mdkb

## Local (single machine)

Run the daemon against your vault; it owns the index and a file watcher:

```sh
mdkbd --vault ~/mdkb-vault
```

Then use any client:

- **AI agent (MCP):** point your client at `deploy/mcp-config.example.json` (it runs
  `mdkb-mcp`, which auto-starts the daemon).
- **Web UI:** `mdkb-web --vault ~/mdkb-vault` → http://127.0.0.1:7878
- **CLI:** `mdkb daemon ~/mdkb-vault/.mdkb/mdkbd.sock search "…"`

The Markdown vault is the source of truth and is the only thing you should sync across
machines (OneDrive, etc.). The index under `~/mdkb-vault/.mdkb/` is local-only and
rebuildable — never sync it.

## In the cluster (k3s/Kubernetes)

The daemon can serve over TCP for in-cluster clients. It stays a **single writer**
(`replicas: 1`, `Recreate` strategy) over one vault PVC, and the network listener is
**token-gated and fail-closed**.

```sh
# 1. build & push the image
docker build -t <registry>/mdkb:latest .
docker push <registry>/mdkb:latest

# 2. create the shared token secret
kubectl -n mdkb create secret generic mdkb-token \
  --from-literal=token=$(openssl rand -hex 24)

# 3. apply (edit image / storageClass first)
kubectl apply -f deploy/k8s.yaml
```

Clients connect with `mdkbd`'s TCP transport and the token:

- A networked client authenticates first (`authenticate { token }`), then issues requests.
- Without a valid token, every data request is rejected.

### Why single-writer

One `mdkbd` owns the index and serializes writes, which preserves consistency and avoids
the cloud-sync corruption that plagues multi-writer database files. Scale *clients*, never
the daemon.

## Conflict files

If a synced vault produces conflict copies (e.g. `note-DESKTOP-AB12.md`), the daemon
**does not index them** — they are surfaced via the `conflicts` tool / `mdkb daemon …
conflicts` so you can resolve them in plain text. The Markdown stays authoritative.
