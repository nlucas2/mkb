---
title: "Skill: CLI surface"
tags: [skill, surface, cli]
updated: 2026-06-25T08:13:56Z
---

## The CLI surface

This is your tool surface: there is no auto-injected schema, so invoke these commands exactly
as shown. The vault is selected by a global **`--vault <dir>`** flag (it can go before or after the
command); if you omit it, the CLI uses `$MKB_VAULT`, else the configured registry default, else
`~/mkb-vault`. Pass `--vault <vault>` as shown so a command always targets the intended vault. The
CLI auto-starts (and reuses) that vault's daemon. `<id>` is a ULID. Bodies are read from **stdin**,
so pipe them in (`echo`, a heredoc, or `<file`). To target a remote daemon instead of a local
socket, pass `--remote host:port --token <tok>` (or set `MKB_REMOTE`/`MKB_TOKEN`).

*FYI: if you only have the source tree (no installed `mkb`), run any command as `cargo run -p mkb-cli -- <command> --vault <vault> …` — same arguments after the `--`.*

Two commands **print an id on stdout you must capture** — `create` (new block id) and `carve`
(child id) — because every later command needs that id.

### Reads

```sh
mkb list --vault <vault>                    # root blocks, "id  title" per line
mkb get --vault <vault> <id>                # raw Markdown body (use before any update)
mkb get --vault <vault> <id> --lines 10:20   # just lines 10-20 (1-based), numbered
mkb render --vault <vault> <id>             # body with embeds resolved
mkb render --vault <vault> <id> --flat      # published form: embeds dissolved, refs as titles
mkb search --vault <vault> "how do I restart nginx" # hybrid keyword+semantic; prefer a natural phrase
mkb search --vault <vault> "ingress" --tag=ops --lang=yaml --limit=10   # filters (--tag repeatable)
mkb search --vault <vault> "tag:ops #k8s lang:rust deploy"              # same filters as inline operators
mkb search --vault <vault> "updated:before:2026-01-01"  # stale blocks (also --updated-before=DATE, and
                                                 # created/updated :after:/:before:, YYYY-MM-DD)
mkb search --vault <vault> "missing:source"     # metadata-gap audit: blocks lacking a property
                                                 # (also has:<key>, and --has=/--missing= flags)
mkb tags --vault <vault>                    # every tag with its block count
mkb props --vault <vault> <id>              # a block's properties, "key<TAB>value" per line
mkb info --vault <vault> <id>               # a block's metadata: created, updated, locked, tags, props
mkb backlinks --vault <vault> <id>          # blocks that reference/embed <id> (check before edits)
mkb links --vault <vault> <id>              # outgoing links/embeds from <id>
mkb stats --vault <vault>                   # index statistics
mkb conflicts --vault <vault>               # cloud-sync conflict files, if any
mkb assets --vault <vault>                  # orphaned assets (no block references); --prune to delete
mkb ping --vault <vault>                    # confirm the daemon is reachable
```

### Writes (body via stdin where shown)

```sh
# create — prints the new id; CAPTURE it
id=$(printf '# Title\n\nbody…\n' | mkb create --vault <vault> --title="Title")

# update — overwrites the WHOLE body+title; read-modify-write (see write safety)
mkb get --vault <vault> "$id" > /tmp/b.md      # 1. read
#   …edit /tmp/b.md, preserving everything you want to keep…
mkb update --vault <vault> "$id" --title="Title" < /tmp/b.md   # 2. write the complete new body

# replace — a SMALL targeted edit: swap an exact string (no full read-modify-write)
mkb replace --vault <vault> "$id" --old "old phrase" --new "new phrase"   # fails unless it occurs once
mkb replace --vault <vault> "$id" --old-file old.txt --new-file new.txt   # for shell-hostile text
printf -- '- new bullet' | mkb append --vault <vault> "$id"   # add text to the END (no anchor needed)

mkb set-tags --vault <vault> <id> ops k8s      # set managed (frontmatter) tags; no args clears them
mkb set-props --vault <vault> <id> source=https://x verified=2026-06-01   # add/update key=value
                                        # properties (open-ended metadata; preserves other props);
                                        # values are searchable
mkb unset-props --vault <vault> <id> verified  # remove the named properties (preserves the rest)
mkb link --vault <vault> <src> <dst>           # add a reference  [[dst]] in src
mkb link --vault <vault> <src> <dst> --embed   # add a live embed ![[dst]] in src

# carve a reusable chunk out of <parent> into its own child block — prints the child id
child=$(printf 'shared chunk…\n' | mkb carve --vault <vault> <parent> --title="Shared chunk")

mkb flatten --vault <vault> <parent> <child>   # inverse of carve: inline parent's one ![[child]] and
                                        # delete child (errors unless child is referenced once)
mkb delete --vault <vault> <id>                # delete a block
```

### Maintenance

```sh
mkb rebuild --vault <vault>             # rebuild the index from blocks/ (after external edits)
```

When a command needs an id, get it from a prior `search`/`list` (or the id `create`/`carve`
printed) — never guess. Titles also resolve (case-insensitively), but prefer the ULID: it is
stable and unambiguous.
