//! Rendering a block to Markdown, with children (`![[...]]`) expanded and references
//! (`[[...]]`) turned into navigable links.
//!
//! This is where "edit a block once, every embed reflects it" happens — an embedding block
//! stores only the directive; the resolver pulls the *current* content of the target block at
//! render time, recursively (its whole subtree). The output is **Markdown** that makes the
//! wiki structure visible (the whole point of mdkb):
//!
//! - `[[target]]` → a Markdown link `[label](mdkb:<id>)`; `mdkb-view` styles it as a wikilink
//!   chip. Dangling targets link to `mdkb:?unresolved`.
//! - `![[target]]` → a block-quoted **embed card** whose header links to the source block and
//!   whose body is the live resolved content (recursively).
//!
//! Resolution is **total**: it never panics and never loops. A cycle renders up to the repeat
//! and emits a navigable link + note; a missing/dangling target renders an inline note. One
//! bad edge degrades locally; the rest of the block renders fine.

use std::collections::HashSet;

use crate::block::Block;
use crate::id::BlockId;
use crate::link::extract_references;
use crate::vault::Vault;

/// A block rendered for display: id + raw body (for editing) + resolved Markdown (for view).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RenderedBlock {
    /// Stable block id.
    pub id: BlockId,
    /// Display title.
    pub title: String,
    /// All tags on this block (frontmatter + inline `#tags`), for display/search.
    pub tags: Vec<String>,
    /// The **managed** (frontmatter) tags — the subset the tag editor can add/remove. The rest
    /// of `tags` are inline `#hashtag` mentions edited in the body.
    pub fm_tags: Vec<String>,
    /// Original block body, for round-trip editing.
    pub raw: String,
    /// Resolved Markdown for display (references → links, children → embed cards).
    pub rendered: String,
}

/// Render a block (by id) to Markdown with its children expanded. Returns `None` if the id is
/// unknown.
pub fn render_block(vault: &Vault, id: &BlockId) -> Option<String> {
    let block = vault.block(id)?;
    let mut visited = HashSet::new();
    visited.insert(id.clone());
    Some(render_body(vault, block, &mut visited))
}

/// Render a block as a [`RenderedBlock`] (raw + resolved).
pub fn rendered_block(vault: &Vault, id: &BlockId) -> Option<RenderedBlock> {
    let block = vault.block(id)?;
    let mut visited = HashSet::new();
    visited.insert(id.clone());
    Some(RenderedBlock {
        id: id.clone(),
        title: block.display_title(),
        tags: block.tags.clone(),
        fm_tags: block.fm_tags.clone(),
        raw: block.body.clone(),
        rendered: render_body(vault, block, &mut visited),
    })
}

/// Render a block's body: substitute each directive, recursing into children.
fn render_body(vault: &Vault, block: &Block, visited: &mut HashSet<BlockId>) -> String {
    let refs = extract_references(&block.body);
    if refs.is_empty() {
        return block.body.clone();
    }
    let mut out = String::new();
    let mut cursor = 0usize;
    for r in refs {
        out.push_str(&block.body[cursor..r.span.start]);
        if r.embed {
            out.push_str(&resolve_embed(vault, &r.target, r.label(), visited));
        } else {
            out.push_str(&render_reference(vault, &r.target, r.label()));
        }
        cursor = r.span.end;
    }
    out.push_str(&block.body[cursor..]);
    out
}

/// A plain `[[target]]` reference → a Markdown link in the shared `mdkb:` scheme.
///
/// When the author wrote a bare `[[<ulid>]]` (no `|alias`), the link text would otherwise be
/// the opaque id; instead we substitute the resolved block's display title so references read
/// naturally. An explicit alias (`[[id|label]]`) or a title target keeps the author's text.
fn render_reference(vault: &Vault, target: &str, label: &str) -> String {
    match vault.resolve(target) {
        Some(id) => {
            let text = if label == target {
                vault
                    .block(&id)
                    .map(|b| b.display_title())
                    .unwrap_or_else(|| label.to_string())
            } else {
                label.to_string()
            };
            format!("[{}](mdkb:{id})", escape_link_text(&text))
        }
        None => format!("[{}](mdkb:?unresolved)", escape_link_text(label)),
    }
}

/// A `![[target]]` transclusion → an embed card (blockquote) with a source-link header and the
/// live resolved body of the target subtree.
fn resolve_embed(
    vault: &Vault,
    target: &str,
    label: &str,
    visited: &mut HashSet<BlockId>,
) -> String {
    let label = escape_link_text(label);
    let id = match vault.resolve(target) {
        Some(id) => id,
        None => return format!("[⚠ {label}](mdkb:?unresolved) *(unresolved embed)*"),
    };
    if visited.contains(&id) {
        // Cycle: link back to the offending block + a visible note, never recurse.
        return format!("[↻ {label}](mdkb:{id}) *(transclusion cycle)*");
    }
    let block = match vault.block(&id) {
        Some(b) => b,
        None => return format!("[⚠ {label}](mdkb:?unresolved) *(unresolved embed)*"),
    };
    visited.insert(id.clone());
    let body = render_body(vault, block, visited);
    visited.remove(&id);

    let header = format!(
        "⧉ [{}](mdkb:{id})",
        escape_link_text(&block.display_title())
    );
    let mut out = format!("> {header}\n>\n");
    for line in body.lines() {
        if line.is_empty() {
            out.push('>');
        } else {
            out.push_str("> ");
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

fn escape_link_text(s: &str) -> String {
    s.replace('\\', r"\\")
        .replace('[', r"\[")
        .replace(']', r"\]")
}

/// Render a block to **flat, self-contained Markdown** for export (e.g. generating a repo doc
/// from a vault block). Unlike [`render_block`], which produces `mdkb:`-linked embed *cards*,
/// this **dissolves** every `![[embed]]` inline — the child's content (recursively) flows as
/// part of the document — and renders each `[[reference]]` as its plain display title (a flat
/// `.md` file has nowhere to link a `mdkb:` scheme). Total: cycles and dangling targets become
/// invisible HTML-comment markers instead of breaking the document. Returns `None` if unknown.
pub fn render_flat(vault: &Vault, id: &BlockId) -> Option<String> {
    let block = vault.block(id)?;
    let mut visited = HashSet::new();
    visited.insert(id.clone());
    Some(render_flat_body(vault, block, &mut visited))
}

fn render_flat_body(vault: &Vault, block: &Block, visited: &mut HashSet<BlockId>) -> String {
    let refs = extract_references(&block.body);
    if refs.is_empty() {
        return block.body.clone();
    }
    let mut out = String::new();
    let mut cursor = 0usize;
    for r in refs {
        out.push_str(&block.body[cursor..r.span.start]);
        if r.embed {
            out.push_str(&dissolve_embed(vault, &r.target, r.label(), visited));
        } else {
            out.push_str(&flat_reference(vault, &r.target, r.label()));
        }
        cursor = r.span.end;
    }
    out.push_str(&block.body[cursor..]);
    out
}

/// A `[[reference]]` in flat output → the target's display title (or the author's alias) as
/// plain text. No link: a standalone `.md` doc can't resolve the `mdkb:` scheme.
fn flat_reference(vault: &Vault, target: &str, label: &str) -> String {
    match vault.resolve(target) {
        Some(id) if label == target => vault
            .block(&id)
            .map(|b| b.display_title())
            .unwrap_or_else(|| label.to_string()),
        _ => label.to_string(),
    }
}

/// A `![[embed]]` in flat output → the target's resolved body, dissolved inline (recursively).
/// Cycles and dangling targets degrade to an invisible HTML comment so the doc stays intact.
fn dissolve_embed(
    vault: &Vault,
    target: &str,
    label: &str,
    visited: &mut HashSet<BlockId>,
) -> String {
    let id = match vault.resolve(target) {
        Some(id) => id,
        None => return format!("<!-- mdkb: unresolved embed: {target} -->"),
    };
    if visited.contains(&id) {
        return format!("<!-- mdkb: transclusion cycle at {label} -->");
    }
    let block = match vault.block(&id) {
        Some(b) => b,
        None => return format!("<!-- mdkb: unresolved embed: {target} -->"),
    };
    visited.insert(id.clone());
    let body = render_flat_body(vault, block, visited);
    visited.remove(&id);
    // Trim surrounding blank lines so a dissolved child doesn't accumulate extra spacing; the
    // parent's own newlines around the directive provide separation.
    body.trim_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> (Vault, BlockId, BlockId) {
        let mut v = Vault::new();
        let child = BlockId::generate();
        let parent = BlockId::generate();
        v.insert_source(child.clone(), "---\ntitle: Child\n---\nchild content\n");
        v.insert_source(
            parent.clone(),
            &format!("intro\n\n![[{child}]]\n\nlink [[Child]]\n"),
        );
        (v, parent, child)
    }

    #[test]
    fn embed_renders_card_with_live_content() {
        let (v, parent, _child) = vault();
        let out = render_block(&v, &parent).unwrap();
        assert!(out.contains("child content"), "got: {out}");
        assert!(out.contains("> ⧉ ["), "embed card header missing: {out}");
    }

    #[test]
    fn reference_renders_mdkb_link() {
        let (v, parent, child) = vault();
        let out = render_block(&v, &parent).unwrap();
        assert!(out.contains(&format!("(mdkb:{child})")), "got: {out}");
    }

    #[test]
    fn bare_id_reference_uses_block_title_as_label() {
        // A `[[<ulid>]]` with no alias renders the target's title, not the opaque id.
        let mut v = Vault::new();
        let target = BlockId::generate();
        let src = BlockId::generate();
        v.insert_source(target.clone(), "---\ntitle: Deployment Guide\n---\nbody\n");
        v.insert_source(src.clone(), &format!("see [[{target}]] now\n"));
        let out = render_block(&v, &src).unwrap();
        assert_eq!(out, format!("see [Deployment Guide](mdkb:{target}) now\n"));
    }

    #[test]
    fn explicit_alias_is_preserved() {
        let mut v = Vault::new();
        let target = BlockId::generate();
        let src = BlockId::generate();
        v.insert_source(target.clone(), "---\ntitle: Long Title\n---\nbody\n");
        v.insert_source(src.clone(), &format!("see [[{target}|the docs]]\n"));
        let out = render_block(&v, &src).unwrap();
        assert!(
            out.contains(&format!("[the docs](mdkb:{target})")),
            "got: {out}"
        );
    }

    #[test]
    fn edit_once_reflects_everywhere() {
        let mut v = Vault::new();
        let q = BlockId::generate();
        let x = BlockId::generate();
        let y = BlockId::generate();
        v.insert_source(q.clone(), "---\ntitle: Q\n---\nStormEvents | take 10\n");
        v.insert_source(x.clone(), &format!("![[{q}]]\n"));
        v.insert_source(y.clone(), &format!("see ![[{q}]]\n"));
        assert!(render_block(&v, &x).unwrap().contains("take 10"));
        assert!(render_block(&v, &y).unwrap().contains("take 10"));
        v.insert_source(q.clone(), "---\ntitle: Q\n---\nStormEvents | take 50\n");
        assert!(render_block(&v, &x).unwrap().contains("take 50"));
        assert!(!render_block(&v, &x).unwrap().contains("take 10"));
    }

    #[test]
    fn cycle_is_broken_with_a_note() {
        let mut v = Vault::new();
        let a = BlockId::generate();
        let b = BlockId::generate();
        v.insert_source(a.clone(), &format!("A ![[{b}]]\n"));
        v.insert_source(b.clone(), &format!("B ![[{a}]]\n"));
        let out = render_block(&v, &a).unwrap();
        assert!(out.contains("transclusion cycle"), "got: {out}");
        assert!(out.contains("](mdkb:"), "cycle note should link: {out}");
    }

    #[test]
    fn dangling_embed_degrades_locally() {
        let mut v = Vault::new();
        let a = BlockId::generate();
        v.insert_source(
            a.clone(),
            "before\n\n![[01JJJJJJJJJJJJJJJJJJJJJJJJ]]\n\nafter\n",
        );
        let out = render_block(&v, &a).unwrap();
        assert!(out.contains("before"));
        assert!(out.contains("after"));
        assert!(out.contains("unresolved"));
    }

    #[test]
    fn rendered_block_carries_raw_and_resolved() {
        let (v, parent, _child) = vault();
        let rb = rendered_block(&v, &parent).unwrap();
        assert!(rb.raw.contains("![["));
        assert!(rb.rendered.contains("> ⧉ ["));
        assert!(!rb.rendered.contains("![["));
    }

    #[test]
    fn flat_dissolves_embeds_inline_no_card() {
        let (v, parent, _child) = vault();
        let out = render_flat(&v, &parent).unwrap();
        // Child content is inlined, with no embed-card blockquote and no mdkb: links.
        assert!(out.contains("child content"), "got: {out}");
        assert!(!out.contains("> ⧉"), "should not be a card: {out}");
        assert!(
            !out.contains("mdkb:"),
            "no mdkb links in flat output: {out}"
        );
        assert!(
            !out.contains("![["),
            "embed directive should be gone: {out}"
        );
    }

    #[test]
    fn flat_reference_becomes_plain_title() {
        let mut v = Vault::new();
        let target = BlockId::generate();
        let src = BlockId::generate();
        v.insert_source(target.clone(), "---\ntitle: Deployment Guide\n---\nbody\n");
        v.insert_source(src.clone(), &format!("see [[{target}]] now\n"));
        let out = render_flat(&v, &src).unwrap();
        assert_eq!(out, "see Deployment Guide now\n");
    }

    #[test]
    fn flat_nested_embeds_dissolve_recursively() {
        let mut v = Vault::new();
        let leaf = BlockId::generate();
        let mid = BlockId::generate();
        let root = BlockId::generate();
        v.insert_source(leaf.clone(), "leaf text\n");
        v.insert_source(
            mid.clone(),
            &format!("mid before\n\n![[{leaf}]]\n\nmid after\n"),
        );
        v.insert_source(root.clone(), &format!("# Doc\n\n![[{mid}]]\n"));
        let out = render_flat(&v, &root).unwrap();
        assert!(out.contains("# Doc"));
        assert!(out.contains("mid before"));
        assert!(out.contains("leaf text"));
        assert!(out.contains("mid after"));
        assert!(!out.contains("![["));
    }

    #[test]
    fn flat_cycle_and_dangling_become_comments() {
        let mut v = Vault::new();
        let a = BlockId::generate();
        let b = BlockId::generate();
        v.insert_source(a.clone(), &format!("A\n\n![[{b}]]\n"));
        v.insert_source(b.clone(), &format!("B\n\n![[{a}]]\n"));
        let out = render_flat(&v, &a).unwrap();
        assert!(out.contains("A"));
        assert!(out.contains("B"));
        assert!(out.contains("<!-- mdkb: transclusion cycle"), "got: {out}");

        let mut v2 = Vault::new();
        let c = BlockId::generate();
        v2.insert_source(c.clone(), "x\n\n![[01JJJJJJJJJJJJJJJJJJJJJJJJ]]\n\ny\n");
        let out2 = render_flat(&v2, &c).unwrap();
        assert!(out2.contains("<!-- mdkb: unresolved embed"), "got: {out2}");
    }
}
