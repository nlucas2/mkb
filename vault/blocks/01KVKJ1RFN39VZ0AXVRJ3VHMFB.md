---
title: "README: Desktop app & web UI"
tags: [doc, readme]
---

### Desktop app & web UI

Two front-ends share the same `mdkb-view` rendering layer (so they can't drift apart), and both
connect either way — a **local** socket or a **remote** TCP daemon.

<table>
  <tr>
    <td align="center"><img src="docs/images/app-block-static.png" alt="Blocks view — each embedded block is a live, self-contained card" width="300"><br><sub>Blocks — embeds become live cards…</sub></td>
    <td align="center"><img src="docs/images/app-block-edit.png" alt="Editing an embedded block inline as a card, in place" width="300"><br><sub>…click any card to edit it inline</sub></td>
    <td align="center"><img src="docs/images/app-edit-picker.png" alt="Edit mode — raw Markdown with the [[ link/embed picker open" width="300"><br><sub>Edit — Markdown + the <code>[[</code> picker</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/images/app-graph.png" alt="Force-directed knowledge graph; node size reflects link degree" width="300"><br><sub>Graph — nodes sized by link degree</sub></td>
    <td align="center"><img src="docs/images/app-codeblocks.png" alt="Syntax-highlighted fenced code blocks across languages" width="300"><br><sub>Code — syntax-highlighted blocks</sub></td>
    <td align="center"><img src="docs/images/app-tag-search.png" alt="Search results filtered by tag and language" width="300"><br><sub>Search — tag &amp; language filters</sub></td>
  </tr>
</table>

- **Desktop app** (`app/mdkb-tauri`) — a Tauri app over the same crates, and a full **editor and
  graph browser**, not just a viewer. It exposes the same three block modes as the rest of mdkb —
  **Read** (the clean document, embeds dissolved inline), **Blocks** (the working view, each embed
  an editable card), and **Edit** (raw Markdown with the `[[` picker and **Carve selection**).

  On top of those it adds **inline editing** (click rendered content to edit that block in place;
  type `[[` for a link/embed picker), a **"references in this block" legend** under the editor
  (the outgoing links/embeds, each resolved to its target with a preview, click-to-open) and
  **hover previews** on rendered wikilink chips, **New / Add / Carve / Delete** block actions, a
  force-directed **knowledge graph** (nodes sized by link degree, computed in `mdkb-core`
  `link_graph`), **linked references** per block, and **Settings** (choose a Local vault or a
  Remote daemon `host:port` + token, no env vars). Point Settings → Local vault at your vault and
  go; see [`app/mdkb-tauri/README.md`](./app/mdkb-tauri/README.md).

- **Local web UI** (`mdkb-web`) — the same views in a browser:

  ```sh
  mdkb-web --vault ~/my-vault                                 # local → http://127.0.0.1:7878
  mdkb-web --remote mdkbd.example:7820 --token "$MDKB_TOKEN"  # a remote daemon
  ```
