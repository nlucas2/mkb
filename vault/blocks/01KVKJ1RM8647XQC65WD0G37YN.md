---
title: "README: Deployment & license"
tags: [doc, readme]
---

## Deployment

See [`deploy/README.md`](./deploy/README.md). In short: run `mdkbd --vault <dir>` locally,
or deploy the daemon to k3s/Kubernetes as a single writer (`replicas: 1`) serving a
token-gated TCP API (`deploy/k8s.yaml`, `Dockerfile`). Sync only the Markdown vault across
machines; each daemon keeps its own local, rebuildable index.

## License

Dual-licensed under MIT or Apache-2.0.
