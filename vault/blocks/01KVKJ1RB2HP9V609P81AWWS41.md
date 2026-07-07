---
title: "README: CLI usage"
tags: [doc/readme]
updated: 2026-07-07T04:23:34Z
---

### Command line (`mkb`)

`mkb` is the terminal interface — script, search, and pipe against a vault:

```sh
mkb search "how do I restart nginx"   # hybrid keyword + semantic search
```

See the **[usage guide](docs/USAGE.md)** for the CLI in context, and `mkb --help` (or
`mkb <cmd> --help`) for the full command surface. From a source checkout, use
`cargo run -p mkb-cli -- …` in place of `mkb`.
