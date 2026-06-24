---
title: "README: CLI usage"
tags: [doc, readme]
---

### Command line (`mdkb`)

Every `mdkb` command takes a vault directory and auto-starts (then reuses) that vault's daemon —
the daemon owns the one warm index and is the single writer.

```sh
# reads
mdkb list ~/my-vault                                   # root blocks: id  title
mdkb search ~/my-vault "how do I restart nginx"
mdkb search ~/my-vault kusto --lang=kusto
mdkb search ~/my-vault "ops" --tag=ops --limit=10
mdkb search ~/my-vault "updated:before:2026-01-01"     # stale blocks (recency filter)
mdkb search ~/my-vault "missing:source"                # blocks lacking a property (metadata gap)
mdkb render ~/my-vault <block-id>                      # children inlined
mdkb tags ~/my-vault
mdkb props ~/my-vault <id>                             # a block's properties
mdkb info ~/my-vault <id>                              # created, updated, locked, tags, props
mdkb stats ~/my-vault

# writes (body via stdin where shown)
echo "# Note" | mdkb create ~/my-vault --title="Note"  # prints the new id
mdkb set-tags ~/my-vault <id> ops kusto
mdkb set-props ~/my-vault <id> source=https://x        # add/update properties (preserves others)
mdkb unset-props ~/my-vault <id> source               # remove a property
mdkb link ~/my-vault <src> <dst> --embed
```

Running from source instead? Use `cargo run -p mdkb-cli -- …` in place of `mdkb`.
