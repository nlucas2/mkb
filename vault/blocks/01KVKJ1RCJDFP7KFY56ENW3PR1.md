---
title: "README: Running the daemon manually"
tags: [doc/readme]
updated: 2026-07-07T05:34:04Z
---

### Running the daemon manually

You normally never do this — every client auto-starts and reuses the daemon. Run it yourself
only to keep a vault warm, expose it over the network, or run it as a service:

```sh
mkbd --vault ~/my-vault            # serves ~/my-vault's daemon

# from another shell, clients connect to (or would auto-start) that vault's daemon
mkb ping  --vault ~/my-vault
mkb stats --vault ~/my-vault
mkb search --vault ~/my-vault "restart the web server"
```
