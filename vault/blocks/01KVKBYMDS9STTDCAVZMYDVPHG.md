---
title: "SPEC: Daemon lifetime: idle self-shutdown"
tags: [spec, doc]
---

## Daemon lifetime: idle self-shutdown

A daemon that a **client auto-starts** is given an idle timeout (`--idle-timeout <secs>`, 15
minutes by default) and **reaps itself** after that long with no requests — freeing its process
and embedder RAM so an unused vault doesn't leak a daemon. Any request (including a liveness
ping) defers the timer. On reap it removes its socket so the next start is clean; the OS releases
the lock on exit. A daemon a user runs **manually**, or the **remote/shared** daemon, gets no
idle timeout and runs forever. Clients self-heal: if a local daemon has idled out (or crashed),
the next interaction transparently respawns it — at most a brief cold start.
