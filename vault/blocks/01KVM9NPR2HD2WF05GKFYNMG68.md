---
title: "README: Pick your interface"
updated: 2026-07-01T03:44:14Z
---

### One vault, many interfaces

mkb is one knowledge base with three front-ends, and a full install gives you all of them — reach
for whichever fits the task. They all
read and write the same vault *through the daemon*, and **you never start the daemon yourself**:
each client auto-starts it on first use and it self-reaps when idle.

| If you want to… | Use | What it is |
|---|---|---|
| Read, edit, and browse the graph | **Desktop app** | a Markdown editor + knowledge-graph browser |
| Script, search, or pipe from a terminal | **CLI** — `mkb` | `mkb search --vault ~/vault "how do I…"` |
| Give an AI assistant your notes | **MCP server** — `mkb-mcp` | a set of tools your MCP client calls |

Everything works with AI turned off; semantic search runs entirely on a local model built into the
daemon (see the **[configuration guide](docs/CONFIGURATION.md)**).

For a tour of the everyday features — searching, browsing and grouping, human-only blocks, and
exporting — see the **[usage guide](docs/USAGE.md)**.
