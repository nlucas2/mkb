---
title: "Usage: MCP tool tiers"
tags: [doc, usage]
updated: 2026-07-01T03:43:40Z
---

## MCP tool tiers

The MCP server exposes a **lean core surface** by default — the read tools and everyday writes an
AI assistant needs, and nothing more. Setting `MKB_MCP_TOOLS=full` (or `all`) adds the **advanced
tier**: `set_props`, `unset_props`, `carve_block`, and `flatten_block`, for agents that actively
refactor the graph. Keep the default surface unless an assistant genuinely needs to restructure
blocks — a smaller toolset is easier for a model to use well. See the
[configuration guide](CONFIGURATION.md#environment-variables).
