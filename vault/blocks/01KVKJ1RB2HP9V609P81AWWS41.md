---
title: "README: Usage (current)"
tags: [doc, readme]
---

## Usage (current)

```sh
# Every CLI command takes a vault dir and auto-starts (then reuses) that vault's daemon.
# The daemon owns the one warm index and is the single writer.

# reads
cargo run -p mdkb-cli -- list ./my-vault                     # root blocks: id  title
cargo run -p mdkb-cli -- search ./my-vault "how do I restart nginx"
cargo run -p mdkb-cli -- search ./my-vault kusto --lang=kusto
cargo run -p mdkb-cli -- search ./my-vault "ops" --tag=ops --limit=10
cargo run -p mdkb-cli -- render ./my-vault <block-id>        # children inlined
cargo run -p mdkb-cli -- tags ./my-vault
cargo run -p mdkb-cli -- stats ./my-vault

# writes (body via stdin where shown)
echo "# Note" | cargo run -p mdkb-cli -- create ./my-vault --title="Note"   # prints new id
cargo run -p mdkb-cli -- set-tags ./my-vault <id> ops kusto
cargo run -p mdkb-cli -- link ./my-vault <src> <dst> --embed
```
