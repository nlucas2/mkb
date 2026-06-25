---
title: "README: Pick your interface"
---

### Pick your interface

mdkb is one knowledge base with three front-ends — pick whichever fits the moment. They all
read and write the same vault *through the daemon*, and **you never start the daemon yourself**:
each client auto-starts it on first use and it self-reaps when idle.

| If you want to… | Use | What it is |
|---|---|---|
| Read, edit, and browse the graph | **Desktop app** (or the local **web UI**) | a Markdown editor + knowledge-graph browser |
| Script, search, or pipe from a terminal | **CLI** — `mdkb` | `mdkb search --vault ~/vault "how do I…"` |
| Give an AI assistant your notes | **MCP server** — `mdkb-mcp` | a set of tools your MCP client calls |

Everything works with AI turned off; semantic search runs entirely on a local model built into the
daemon (see *Configuration* below).
