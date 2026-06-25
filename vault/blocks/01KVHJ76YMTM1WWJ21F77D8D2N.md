---
title: Run mkb with Docker Compose
tags: [mkb, run, docker]
---

# Run mkb with Docker Compose

A reproducible local stack.

![[01KVHJ76YHDWY5PMB7GY9B0WP6]]

```yaml
services:
  mkbd:
    image: ghcr.io/example/mkb:latest
    command: ["mkbd", "--vault", "/vault", "--listen", "0.0.0.0:7820", "--token", "${MKB_TOKEN}"]
    volumes:
      - ./my-vault:/vault
    ports:
      - "7820:7820"
```
