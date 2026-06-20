---
title: Blocks are files
---

# Blocks are files

Each block lives in `blocks/<id>.md` — clean Markdown with an optional title in
frontmatter. The filename is a stable id, so you can rename a block's title freely
without breaking any link to it.

Compose by writing two kinds of directive in a block's body:

- `![[<id>]]` pulls in another block's content (a transclusion / child).
- `[[<id>]]` links to another block (a reference).

In the desktop app you never type an id by hand: in the editor, type `[[` and a
picker finds blocks by title (you can also hand-type a title, or paste a ULID).
Enter links; Tab switches to an embed. The id is the durable target — a title
resolves too, but ids survive renames and never collide.
