---
title: "Prerequisites (list)"
tags: [doc, install]
---

Building **from source** (`just install`, `cargo install`, or `cargo tauri build`) needs these on
your machine. Prebuilt releases and the container need none of them.

- **Rust** (stable) — the `cargo` toolchain; the workspace pins `rust-version = 1.80`.
- **[`just`](https://github.com/casey/just)** — the task runner the one-command install uses.
- **Tauri CLI** (`cargo install tauri-cli`) — only needed to build the **desktop app**.
- **System build libraries** — a C toolchain plus your platform's webview/GTK dev libraries
  (Linux), Xcode Command Line Tools (macOS), or MSVC Build Tools + WebView2 (Windows).
