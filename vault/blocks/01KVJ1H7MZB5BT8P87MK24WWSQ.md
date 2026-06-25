---
title: "Skill: CLI surface"
tags: [skill, surface, cli]
updated: 2026-06-25T08:13:56Z
---

## The CLI surface

This is your tool surface: there is no auto-injected schema, so invoke these commands exactly
as shown. The vault is selected by a global **`--vault <dir>`** flag (it can go before or after the
command); if you omit it, the CLI uses `$MDKB_VAULT`, else the configured registry default, else
`~/mdkb-vault`. Pass `--vault <vault>` as shown so a command always targets the intended vault. The
CLI auto-starts (and reuses) that vault's daemon. `<id>` is a ULID. Bodies are read from **stdin**,
so pipe them in (`echo`, a heredoc, or `<file`). To target a remote daemon instead of a local
socket, pass `--remote host:port --token <tok>` (or set `MDKB_REMOTE`/`MDKB_TOKEN`).

*FYI: if you only have the source tree (no installed `mdkb`), run any command as `cargo run -p mdkb-cli -- <command> --vault <vault> …` — same arguments after the `--`.*

Two commands **print an id on stdout you must capture** — `create` (new block id) and `carve`
(child id) — because every later command needs that id.

### Reads

```sh
mdkb list --vault <vault>                    # root blocks, "id  title" per line
mdkb get --vault <vault> <id>                # raw Markdown body (use before any update)
mdkb get --vault <vault> <id> --lines 10:20   # just lines 10-20 (1-based), numbered
mdkb render --vault <vault> <id>             # body with embeds resolved
mdkb render --vault <vault> <id> --flat      # published form: embeds dissolved, refs as titles
mdkb search --vault <vault> "how do I restart nginx" # hybrid keyword+semantic; prefer a natural phrase
mdkb search --vault <vault> "ingress" --tag=ops --lang=yaml --limit=10   # filters (--tag repeatable)
mdkb search --vault <vault> "tag:ops #k8s lang:rust deploy"              # same filters as inline operators
mdkb search --vault <vault> "updated:before:2026-01-01"  # stale blocks (also --updated-before=DATE, and
                                                 # created/updated :after:/:before:, YYYY-MM-DD)
mdkb search --vault <vault> "missing:source"     # metadata-gap audit: blocks lacking a property
                                                 # (also has:<key>, and --has=/--missing= flags)
mdkb tags --vault <vault>                    # every tag with its block count
mdkb props --vault <vault> <id>              # a block's properties, "key<TAB>value" per line
mdkb info --vault <vault> <id>               # a block's metadata: created, updated, locked, tags, props
mdkb backlinks --vault <vault> <id>          # blocks that reference/embed <id> (check before edits)
mdkb links --vault <vault> <id>              # outgoing links/embeds from <id>
mdkb stats --vault <vault>                   # index statistics
mdkb conflicts --vault <vault>               # cloud-sync conflict files, if any
mdkb assets --vault <vault>                  # orphaned assets (no block references); --prune to delete
mdkb ping --vault <vault>                    # confirm the daemon is reachable
```

### Writes (body via stdin where shown)

```sh
# create — prints the new id; CAPTURE it
id=$(printf '# Title\n\nbody…\n' | mdkb create --vault <vault> --title="Title")

# update — overwrites the WHOLE body+title; read-modify-write (see write safety)
mdkb get --vault <vault> "$id" > /tmp/b.md      # 1. read
#   …edit /tmp/b.md, preserving everything you want to keep…
mdkb update --vault <vault> "$id" --title="Title" < /tmp/b.md   # 2. write the complete new body

# replace — a SMALL targeted edit: swap an exact string (no full read-modify-write)
mdkb replace --vault <vault> "$id" --old "old phrase" --new "new phrase"   # fails unless it occurs once
mdkb replace --vault <vault> "$id" --old-file old.txt --new-file new.txt   # for shell-hostile text
printf -- '- new bullet' | mdkb append --vault <vault> "$id"   # add text to the END (no anchor needed)

mdkb set-tags --vault <vault> <id> ops k8s      # set managed (frontmatter) tags; no args clears them
mdkb set-props --vault <vault> <id> source=https://x verified=2026-06-01   # add/update key=value
                                        # properties (open-ended metadata; preserves other props);
                                        # values are searchable
mdkb unset-props --vault <vault> <id> verified  # remove the named properties (preserves the rest)
mdkb link --vault <vault> <src> <dst>           # add a reference  [[dst]] in src
mdkb link --vault <vault> <src> <dst> --embed   # add a live embed ![[dst]] in src

# carve a reusable chunk out of <parent> into its own child block — prints the child id
child=$(printf 'shared chunk…\n' | mdkb carve --vault <vault> <parent> --title="Shared chunk")

mdkb flatten --vault <vault> <parent> <child>   # inverse of carve: inline parent's one ![[child]] and
                                        # delete child (errors unless child is referenced once)
mdkb delete --vault <vault> <id>                # delete a block
```

### Maintenance

```sh
mdkb rebuild --vault <vault>             # rebuild the index from blocks/ (after external edits)
```

When a command needs an id, get it from a prior `search`/`list` (or the id `create`/`carve`
printed) — never guess. Titles also resolve (case-insensitively), but prefer the ULID: it is
stable and unambiguous.
