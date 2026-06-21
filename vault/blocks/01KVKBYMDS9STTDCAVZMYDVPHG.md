---
title: "SPEC: Daemon lifetime: idle self-shutdown"
tags: [spec, doc]
---

## Daemon lifetime: idle self-shutdown

A daemon that a **client auto-starts** is given an idle timeout (`--idle-timeout <secs>`, a short
grace by default) and **reaps itself** once it has been idle that long **and no interactive client
is attached** — freeing its process and embedder RAM so an unused vault doesn't leak a daemon. Any
request (including a liveness ping) defers the timer.

Long-lived interactive clients (the desktop app, the web UI) hold a renewable **lease**: they
heartbeat the daemon periodically, and it will not reap while any lease is active. A lease carries
a TTL and lapses if its client stops heartbeating, so a crashed or closed client can never pin the
daemon open — the lease expires and the idle grace then applies. Momentary clients (the CLI, MCP)
need no lease; their request activity defers the timer as usual.

On reap it removes its socket so the next start is clean; the OS releases the lock on exit. A
daemon a user runs **manually**, or the **remote/shared** daemon, gets no idle timeout and runs
forever. Clients self-heal: if a local daemon has idled out (or crashed), the next interaction
transparently respawns it — at most a brief cold start.
