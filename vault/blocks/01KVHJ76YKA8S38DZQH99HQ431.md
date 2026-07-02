---
title: Run mkb with Docker
tags: [run/docker]
updated: 2026-07-02T06:45:39Z
---

# Run mkb with Docker

Run the daemon in a container, mounting your vault.

![[01KVHJ76YHDWY5PMB7GY9B0WP6]]

```sh
docker run --rm \
  -v "$PWD/my-vault:/vault" \
  -p 7820:7820 \
  ghcr.io/example/mkb:latest \
  mkbd --vault /vault --listen 0.0.0.0:7820 --token "$MKB_TOKEN"
```
