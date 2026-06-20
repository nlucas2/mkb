---
title: "README: Requirements, build, test, run"
tags: [doc, readme]
---

## Requirements

- Rust (stable). The workspace pins `rust-version = 1.80`.

## Build, test, run

```sh
# build everything
cargo build --workspace

# run the test suite (must be green before every commit — see Contributing)
cargo test --workspace

# see CLI usage
cargo run -p mdkb-cli -- --help
```
