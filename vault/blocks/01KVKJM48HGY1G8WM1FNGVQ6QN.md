---
title: "Architecture: a block is a file"
tags: [doc, architecture]
---

## The core idea: a block is a file

The primitive unit of knowledge is a **block**, and a block is **an author-chosen span of
content** — *not* a Markdown element. It can be a single paragraph, or a paragraph + a code
block + a list, whatever is the right primitive at the time. Markdown is only the *formatting of
the content inside a block*; it carries **no structural meaning**.

> **block = page = file.** One concept. A "page" is just a block you open at the top; a block
> that nothing embeds is a top-level entry. Promoting a chunk to its own page is the same
> operation as carving out a sub-block — they are identical.

Each block is one file `blocks/<ULID>.md`: the ULID filename **is** the identity (rename the
title or move the file freely — references resolve by ULID, never by path or title). The exact
on-disk format — frontmatter, the `![[child]]` / `[[reference]]` directives — is specified in
[`SPEC.md`](./SPEC.md).
