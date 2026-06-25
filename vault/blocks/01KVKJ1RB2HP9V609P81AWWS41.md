---
title: "README: CLI usage"
tags: [doc, readme]
---

### Command line (`mkb`)

Every `mkb` command targets a vault via the global `--vault <dir>` flag (or `$MKB_VAULT`, or the
configured registry default) and auto-starts (then reuses) that vault's daemon — the daemon owns
the one warm index and is the single writer. With a default configured, you can drop `--vault`
entirely.

```sh
# reads
mkb list --vault ~/my-vault                            # root blocks: id  title
mkb search --vault ~/my-vault "how do I restart nginx"
mkb search --vault ~/my-vault kusto --lang=kusto
mkb search --vault ~/my-vault "ops" --tag=ops --limit=10
mkb search --vault ~/my-vault "updated:before:2026-01-01"   # stale blocks (recency filter)
mkb search --vault ~/my-vault "missing:source"              # blocks lacking a property (metadata gap)
mkb render --vault ~/my-vault <block-id>               # children inlined
mkb tags --vault ~/my-vault
mkb props --vault ~/my-vault <id>                      # a block's properties
mkb info --vault ~/my-vault <id>                       # created, updated, locked, tags, props
mkb stats --vault ~/my-vault

# writes (body via stdin where shown)
echo "# Note" | mkb create --vault ~/my-vault --title="Note"  # prints the new id
mkb set-tags --vault ~/my-vault <id> ops kusto
mkb set-props --vault ~/my-vault <id> source=https://x # add/update properties (preserves others)
mkb unset-props --vault ~/my-vault <id> source         # remove a property
mkb link --vault ~/my-vault <src> <dst> --embed
```

Running from source instead? Use `cargo run -p mkb-cli -- …` in place of `mkb`.
