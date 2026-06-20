# mdkb demo vault

A small, self-documenting mdkb vault: it explains how to use and run mdkb *using*
mdkb. Open **Welcome to mdkb** and follow the links; the run-guides all embed one
shared note to demonstrate transclusion (edit it once, every guide updates).

Point the daemon — or the desktop app's Settings — at this folder:

```sh
mdkbd --vault examples/demo-vault
```

Only `blocks/*.md` is the source of truth; the index is rebuilt from it.
