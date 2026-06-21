---
title: "README: Browsing in a UI"
tags: [doc, readme]
---

### Browsing in a UI

Two front-ends share the same `mdkb-view` rendering layer (so they can't drift apart), and
both connect using the two paradigms above — a **local** socket or a **remote** TCP daemon.

- **Local web UI** (`mdkb-web`):

  ```sh
  # local daemon:
  mdkbd --vault ./my-vault &
  cargo run -p mdkb-web -- --vault ./my-vault          # http://127.0.0.1:7878

  # remote daemon:
  cargo run -p mdkb-web -- --remote mdkbd.example:7820 --token "$MDKB_TOKEN"
  ```

- **Desktop shell** (`app/mdkb-tauri`) — a Tauri app over the same crates. It is a full
  **editor and graph browser**, not just a viewer. It exposes the same three block modes as the rest of mdkb — **Read** (the clean document, embeds dissolved inline), **Blocks** (the working view, each embed an editable card), and **Edit** (raw Markdown with the `[[` picker and **Carve selection**).

  On top of those it adds **inline editing** (click rendered content to edit that block in
  place; type `[[` for a link/embed picker), a **"references in this block" legend** under the
  editor (the outgoing links/embeds, each resolved to its target with a preview, click-to-open)
  and **hover previews** on rendered wikilink chips, **New / Add / Carve / Delete** block actions,
  a force-directed **knowledge graph** (nodes sized by link degree, computed in `mdkb-core`
  `link_graph`), **linked references** per block, and **Settings** (choose a Local vault or a
  Remote daemon `host:port` + token, no env vars). Lives in its own workspace (needs the Tauri
  toolchain); see [`app/mdkb-tauri/README.md`](./app/mdkb-tauri/README.md).
