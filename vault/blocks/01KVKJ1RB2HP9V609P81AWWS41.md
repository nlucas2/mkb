---
title: "README: CLI usage"
tags: [doc, readme]
---

### Command line (`mdkb`)

Every `mdkb` command targets a vault via the global `--vault <dir>` flag (or `$MDKB_VAULT`, or the
configured registry default) and auto-starts (then reuses) that vault's daemon — the daemon owns
the one warm index and is the single writer. With a default configured, you can drop `--vault`
entirely.

```sh
# reads
mdkb list --vault ~/my-vault                            # root blocks: id  title
mdkb search --vault ~/my-vault "how do I restart nginx"
mdkb search --vault ~/my-vault kusto --lang=kusto
mdkb search --vault ~/my-vault "ops" --tag=ops --limit=10
mdkb search --vault ~/my-vault "updated:before:2026-01-01"   # stale blocks (recency filter)
mdkb search --vault ~/my-vault "missing:source"              # blocks lacking a property (metadata gap)
mdkb render --vault ~/my-vault <block-id>               # children inlined
mdkb tags --vault ~/my-vault
mdkb props --vault ~/my-vault <id>                      # a block's properties
mdkb info --vault ~/my-vault <id>                       # created, updated, locked, tags, props
mdkb stats --vault ~/my-vault

# writes (body via stdin where shown)
echo "# Note" | mdkb create --vault ~/my-vault --title="Note"  # prints the new id
mdkb set-tags --vault ~/my-vault <id> ops kusto
mdkb set-props --vault ~/my-vault <id> source=https://x # add/update properties (preserves others)
mdkb unset-props --vault ~/my-vault <id> source         # remove a property
mdkb link --vault ~/my-vault <src> <dst> --embed
```

Running from source instead? Use `cargo run -p mdkb-cli -- …` in place of `mdkb`.
