---
title: Deploying mkb
tags: [doc/deploy]
updated: 2026-07-07T05:35:58Z
path: deploy
filename: README.md
---

## Local (single machine)

You don't need to deploy anything: point any client (the desktop app, `mkb-mcp`, or the CLI) at a
local folder and it auto-starts the daemon for you. The index lives outside the vault, so you can
sync the Markdown files safely via OneDrive, Dropbox, etc.

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