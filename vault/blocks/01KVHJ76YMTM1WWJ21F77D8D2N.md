---
title: Run mdkb with Docker Compose
tags: [mdkb, run, docker]
---

# Run mdkb with Docker Compose

A reproducible local stack.

![[01KVHJ76YHDWY5PMB7GY9B0WP6]]

```yaml
services:
  mdkbd:
    image: ghcr.io/example/mdkb:latest
    command: ["mdkbd", "--vault", "/vault", "--listen", "0.0.0.0:7820", "--token", "${MDKB_TOKEN}"]
    volumes:
      - ./my-vault:/vault
    ports:
      - "7820:7820"
```
