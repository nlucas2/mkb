---
title: "README: Deployment"
tags: [doc/readme]
updated: 2026-07-07T05:32:09Z
---

## Deployment

See [[01KWXGZGYKM072QF2D79W86B4C|deploy/README.md]]. In short: run `mkbd --vault <dir>` locally,
or deploy the daemon to k3s/Kubernetes as a single writer (`replicas: 1`) serving a
token-gated TCP API (`deploy/k8s.yaml`, `Dockerfile`). Sync only the Markdown vault across
machines; each daemon keeps its own local, rebuildable index.
