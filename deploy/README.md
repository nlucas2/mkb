# Deploying mkb

## Local (single machine)

Run the daemon against your vault; it owns the index and a file watcher:

```sh
mkbd --vault ~/mkb-vault
```

Then use any client:

- **AI agent (MCP):** point your client at `deploy/mcp-config.example.json` (it runs
  `mkb-mcp`, which auto-starts the daemon).
- **CLI:** `mkb search --vault ~/mkb-vault "…"`

The Markdown vault is the source of truth and is the only thing you should sync across
machines (OneDrive, etc.). The index is machine-local and rebuildable — it lives **outside**
the vault (under the OS local-data dir, keyed by a hash of the vault path), so a synced vault
never drags the live index along. Never sync the index.

## In the cluster (k3s/Kubernetes)

The daemon can serve over TCP for in-cluster clients. It stays a **single writer**
(`replicas: 1`, `Recreate` strategy) over one vault PVC, and the network listener is
**token-gated and fail-closed**.

```sh
# 1. build & push the image
docker build -t <registry>/mkb:latest .
docker push <registry>/mkb:latest

# 2. create the shared token secret
kubectl -n mkb create secret generic mkb-token \
  --from-literal=token=$(openssl rand -hex 24)

# 3. apply (edit image / storageClass first)
kubectl apply -f deploy/k8s.yaml
```

Clients connect with `mkbd`'s TCP transport and the token:

- A networked client authenticates first (`authenticate { token }`), then issues requests.
- Without a valid token, every data request is rejected.

> **Embedding model is compiled in (no runtime download).** The daemon binary has an
> int8-quantized BGE-small-en-v1.5 ONNX model (~32 MB) compiled directly into it (the default
> `vendored-model` build), so semantic search runs **fully offline** — no egress to
> `huggingface.co`, no slow first start, nothing to mount. To use a different model, mount one and
> point `config.json` at it (`{"embedder":{"kind":"local","path":"…"}}`) or target a remote
> endpoint (`{"embedder":{"kind":"remote","url":"…"}}`); see the README's "Choosing an embedder"
> section. If a configured embedder can't load, the daemon logs a warning and degrades to the
> offline hash embedder (keyword search still works; semantic ranking is weaker). The readiness
> probe is a plain TCP check, so the pod only reports Ready once the daemon is actually
> listening.

### Connecting a UI to the deployed daemon

The `mkbd` Service is a `LoadBalancer`, so it gets an address reachable from outside the
cluster. Point the desktop app (or the CLI/MCP) at it via `--remote` / the env vars:

```sh
# Find the daemon's external address:
kubectl -n mkb get svc mkbd          # note EXTERNAL-IP

# Desktop app (Tauri) — environment-driven, or via Settings → Remote daemon:
export MKB_REMOTE=<EXTERNAL-IP>:7820
export MKB_TOKEN=<token>
cargo tauri dev        # from app/mkb-tauri

# CLI / MCP — flags or the same env:
mkb search --remote <EXTERNAL-IP>:7820 --token <token> "…"
```

If your cluster has no LoadBalancer provider, switch the Service to `ClusterIP` and reach it
for a quick test with `kubectl -n mkb port-forward svc/mkbd 7820:7820`, then point
`MKB_REMOTE` at `127.0.0.1:7820`.

### Why single-writer

One `mkbd` owns the index and serializes writes, which preserves consistency and avoids
the cloud-sync corruption that plagues multi-writer database files. Scale *clients*, never
the daemon.

## Conflict files

If a synced vault produces conflict copies (e.g. `note-DESKTOP-AB12.md`), the daemon
**does not index them** — they are surfaced via the `conflicts` tool / `mkb conflicts
--vault <dir>` so you can resolve them in plain text. The Markdown stays authoritative.

## Continuous build & releases

`.forgejo/workflows/build.yaml` runs on every push to `main` (and version tags):

- **Every push to `main`** — runs `cargo test --workspace` (the Dockerfile `tester` stage),
  builds and pushes the multi-arch daemon image to `$REGISTRY/containers/mkb:latest` and
  `:<short-sha>` (amd64 + arm64 manifests), and publishes the daemon + client binaries
  (`mkbd`, `mkb`, `mkb-mcp`, per-arch tarballs — the embedding model is compiled
  into `mkbd`, so nothing extra ships alongside — plus checksums) as **downloadable workflow
  artifacts** on the run.
- **A version tag `vX.Y.Z`** — does all of the above tagged with the version, **and** cuts a
  Forgejo release with the same binaries attached.

Required Forgejo Actions configuration:

| Name | Kind | Used for |
|------|------|----------|
| `REGISTRY` | variable | Container registry host (e.g. `registry.example`); used for `docker login` and as the image-ref base. |
| `REGISTRY_ORG` | variable | Registry namespace/org (e.g. `containers`); the image ref is `$REGISTRY/$REGISTRY_ORG/mkb`. |
| `REGISTRY_TOKEN` | secret | `docker login $REGISTRY` to push images |
| `RELEASE_TOKEN` | secret | Forgejo API token to create the release + upload assets (tags only) |

The Forgejo API host is read from `github.server_url` (the instance's own URL), so it is
never hardcoded.

### Native release binaries via GitHub (tags only)

This repo push-mirrors to GitHub. `.github/workflows/release.yml` runs **only on GitHub-hosted
runners** (guarded by `github.server_url`, so Forgejo ignores it) and **only on `v*` tags**. When
a tag rides the mirror up, GitHub builds **native** binaries — Linux amd64, macOS arm64, and
Windows x64 — each with the ONNX embedder and the model compiled in, and attaches them to a GitHub
Release via the built-in `GITHUB_TOKEN` (no PAT). This covers the platforms the Forgejo Linux
runner can't produce: macOS (Apple SDK licensing) and Windows-with-onnx. (Linux arm64 and an
Intel-mac leg are present but commented out — free GitHub arm64 runners and cheap macOS minutes
are public-repo only; Linux arm64 is already covered by the Forgejo image.)

Cutting a release:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

The embedding model is compiled into the daemon by default, so the image always ships with
semantic search built in (no model files to mount, no runtime download). Building an image
*without* the embedded model isn't a supported build-arg; it would require editing the
Dockerfile's `cargo build` to pass `--no-default-features` (the daemon then falls back to the
offline hash embedder unless `$MKB_BUNDLED_MODEL_DIR` points at a model on disk).
