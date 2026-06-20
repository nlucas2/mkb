---
title: Run mdkb with Docker
tags: [mdkb, run, docker]
---

# Run mdkb with Docker

Run the daemon in a container, mounting your vault.

![[01KVHJ76YHDWY5PMB7GY9B0WP6]]

```sh
docker run --rm \
  -v "$PWD/my-vault:/vault" \
  -p 7820:7820 \
  ghcr.io/example/mdkb:latest \
  mdkbd --vault /vault --listen 0.0.0.0:7820 --token "$MDKB_TOKEN"
```
