---
title: "Skill: CLI surface"
tags: [skill, surface, cli]
---

## The CLI surface

Every command takes a vault dir and goes through the daemon (auto-started). `<id>` is a
ULID; bodies come from stdin where noted.

**Read:** `list`, `render <id>` (`--flat` for the published form), `get <id>` (raw body),
`search <query>` (operators, or `--tag=`/`--lang=`/`--limit=`), `tags`, `backlinks <id>`,
`links <id>`, `stats`, `conflicts`, `ping`.

**Write:** `create [--title=T] < body` (prints the id), `update <id> [--title=T] < body`,
`set-tags <id> [tag...]`, `link <src> <dst> [--embed]`, `carve <parent> [--title=T] < body`
(prints the child id), `flatten <parent> <child>` (inline a single-use embed back and delete the
child - the inverse of carve; errors unless the child is referenced exactly once), `delete <id>`.

The safe edit is read-modify-write: `mdkb get vault <id>` -> edit -> `mdkb update vault <id> < body`.
