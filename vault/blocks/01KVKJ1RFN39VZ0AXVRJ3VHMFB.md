---
title: "README: Desktop app"
tags: [doc/readme]
updated: 2026-07-07T05:07:47Z
---

### Desktop app

The desktop app is the human surface — a full **editor and graph browser** (Read / Blocks / Edit
modes, inline block editing, a `[[` link picker, a force-directed knowledge graph, and per-block
linked references). You and the AI co-manage one vault: toggle a block **🔒 human-only** and AI
clients can read it but never modify it. It opens a **local** vault or a **remote** one (`host:port`
+ token); manage vaults from **Settings**.

<table>
  <tr>
    <td align="center"><a href="docs/images/app-block-static.png"><img src="docs/images/app-block-static.png" alt="Blocks view — each embedded block is a live, self-contained card" width="300"></a><br><sub>Blocks — embeds become live cards…</sub></td>
    <td align="center"><a href="docs/images/app-block-edit.png"><img src="docs/images/app-block-edit.png" alt="Editing an embedded block inline as a card, in place" width="300"></a><br><sub>…click any card to edit it inline</sub></td>
    <td align="center"><a href="docs/images/app-edit-picker.png"><img src="docs/images/app-edit-picker.png" alt="Edit mode — raw Markdown with the [[ link/embed picker open" width="300"></a><br><sub>Edit — Markdown + the <code>[[</code> picker</sub></td>
  </tr>
  <tr>
    <td align="center"><a href="docs/images/app-graph.png"><img src="docs/images/app-graph.png" alt="Force-directed knowledge graph; node size reflects link degree" width="300"></a><br><sub>Graph — nodes sized by link degree</sub></td>
    <td align="center"><a href="docs/images/app-codeblocks.png"><img src="docs/images/app-codeblocks.png" alt="Syntax-highlighted fenced code blocks across languages" width="300"></a><br><sub>Code — syntax-highlighted blocks</sub></td>
    <td align="center"><a href="docs/images/app-tag-search.png"><img src="docs/images/app-tag-search.png" alt="Search results filtered by tag and language" width="300"></a><br><sub>Search — tag &amp; language filters</sub></td>
  </tr>
</table>

