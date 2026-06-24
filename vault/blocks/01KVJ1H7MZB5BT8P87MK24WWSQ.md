---
title: "Skill: CLI surface"
tags: [skill, surface, cli]
---

## The CLI surface

This is your tool surface: there is no auto-injected schema, so invoke these commands exactly
as shown. **Every command is `mdkb <command> <vault-dir> [args]`** — the vault directory is always
the first positional after the command, and you repeat it on every call. The CLI auto-starts (and
reuses) that vault's daemon. `<id>` is a ULID. Bodies are read from **stdin**, so pipe them in
(`echo`, a heredoc, or `<file`). To target a remote daemon instead of a local socket, set
`MDKB_REMOTE=host:port` (and `MDKB_TOKEN`) in the environment.

*FYI: if you only have the source tree (no installed `mdkb`), run any command as `cargo run -p mdkb-cli -- <command> <vault> …` — same arguments after the `--`.*

Two commands **print an id on stdout you must capture** — `create` (new block id) and `carve`
(child id) — because every later command needs that id.

### Reads

```sh
mdkb list <vault>                          # root blocks, "id  title" per line
mdkb get <vault> <id>                       # raw Markdown body (use before any update)
mdkb render <vault> <id>                     # body with embeds resolved
mdkb render <vault> <id> --flat              # published form: embeds dissolved, refs as titles
mdkb search <vault> "how do I restart nginx" # hybrid keyword+semantic; prefer a natural phrase
mdkb search <vault> "ingress" --tag=ops --lang=yaml --limit=10   # filters (--tag repeatable)
mdkb search <vault> "tag:ops #k8s lang:rust deploy"              # same filters as inline operators
mdkb search <vault> "updated:before:2026-01-01"  # stale blocks (also --updated-before=DATE, and
                                                 # created/updated :after:/:before:, YYYY-MM-DD)
mdkb search <vault> "missing:source"             # metadata-gap audit: blocks lacking a property
                                                 # (also has:<key>, and --has=/--missing= flags)
mdkb tags <vault>                            # every tag with its block count
mdkb props <vault> <id>                      # a block's properties, "key<TAB>value" per line
mdkb info <vault> <id>                       # a block's metadata: created, updated, locked, tags, props
mdkb backlinks <vault> <id>                  # blocks that reference/embed <id> (check before edits)
mdkb links <vault> <id>                      # outgoing links/embeds from <id>
mdkb stats <vault>                           # index statistics
mdkb conflicts <vault>                       # cloud-sync conflict files, if any
mdkb ping <vault>                            # confirm the daemon is reachable
```

### Writes (body via stdin where shown)

```sh
# create — prints the new id; CAPTURE it
id=$(printf '# Title\n\nbody…\n' | mdkb create <vault> --title="Title")

# update — overwrites the WHOLE body+title; read-modify-write (see write safety)
mdkb get <vault> "$id" > /tmp/b.md      # 1. read
#   …edit /tmp/b.md, preserving everything you want to keep…
mdkb update <vault> "$id" --title="Title" < /tmp/b.md   # 2. write the complete new body

mdkb set-tags <vault> <id> ops k8s      # set managed (frontmatter) tags; no args clears them
mdkb set-props <vault> <id> source=https://x verified=2026-06-01   # add/update key=value
                                        # properties (open-ended metadata; preserves other props);
                                        # values are searchable
mdkb unset-props <vault> <id> verified  # remove the named properties (preserves the rest)
mdkb link <vault> <src> <dst>           # add a reference  [[dst]] in src
mdkb link <vault> <src> <dst> --embed   # add a live embed ![[dst]] in src

# carve a reusable chunk out of <parent> into its own child block — prints the child id
child=$(printf 'shared chunk…\n' | mdkb carve <vault> <parent> --title="Shared chunk")

mdkb flatten <vault> <parent> <child>   # inverse of carve: inline parent's one ![[child]] and
                                        # delete child (errors unless child is referenced once)
mdkb delete <vault> <id>                # delete a block
```

### Maintenance

```sh
mdkb rebuild <vault>                     # rebuild the index from blocks/ (after external edits)
```

When a command needs an id, get it from a prior `search`/`list` (or the id `create`/`carve`
printed) — never guess. Titles also resolve (case-insensitively), but prefer the ULID: it is
stable and unambiguous.
