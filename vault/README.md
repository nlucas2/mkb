# mdkb vault

mdkb's own knowledge base — and a self-documenting demo. It explains how to use and run mdkb
*using* mdkb. Open **Welcome to mdkb** and follow the links; the run-guides all embed one shared
note to demonstrate transclusion (edit it once, every guide updates).

This vault is also the **source of truth for the repo's human-facing docs**: files like the
top-level `README.md` are generated from blocks here via `mdkb export` — edit the block, not the
generated file.

Point the daemon — or the desktop app's Settings — at this folder:

```sh
mdkbd --vault vault
```

Only `blocks/*.md` is the source of truth; the index (`.mdkb/`) is rebuilt from it and is
git-ignored.
