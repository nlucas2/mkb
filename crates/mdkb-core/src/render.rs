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

use std::collections::{HashMap, HashSet};

use crate::block::Block;
use crate::id::BlockId;
use crate::link::extract_references;
use crate::vault::Vault;

/// The set of blocks being exported together, for set-aware flat rendering. When a `[[reference]]`
/// targets a block that is **in the set**, flat export renders a real relative Markdown link to
/// that block's output file instead of inert plain text — so a multi-doc export is navigable.
/// References to blocks outside the set stay plain text (and the caller can warn about them).
pub struct FlatSet<'a> {
    /// Resolved block id → its output path (relative to the export root).
    pub paths: &'a HashMap<BlockId, String>,
    /// Output path of the doc currently being rendered, so links are made relative to it.
    pub self_path: &'a str,
}

/// Build a relative link from the directory of `from` to the file `to` (both export-root-relative,
/// `/`-separated). Used to cross-link co-exported docs that may live in different directories.
pub fn relative_link(from: &str, to: &str) -> String {
    let from_dirs: Vec<&str> = from.split('/').collect();
    let to_parts: Vec<&str> = to.split('/').collect();
    // Directory components of `from` (drop its filename).
    let from_dirs = &from_dirs[..from_dirs.len().saturating_sub(1)];
    let mut common = 0;
    while common < from_dirs.len()
        && common + 1 < to_parts.len()
        && from_dirs[common] == to_parts[common]
    {
        common += 1;
    }
    let ups = from_dirs.len() - common;
    let mut out = String::new();
    for _ in 0..ups {
        out.push_str("../");
    }
    out.push_str(&to_parts[common..].join("/"));
    out
}

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
    Some(render_flat_body(vault, block, &mut visited, None))
}

/// Like [`render_flat`], but **set-aware**: `[[reference]]`s whose target is in `set` render as
/// relative Markdown links to the target's output file (see [`FlatSet`]). Used by the docs-as-data
/// export so co-exported docs cross-link instead of degrading to plain text.
pub fn render_flat_in_set(vault: &Vault, id: &BlockId, set: &FlatSet) -> Option<String> {
    let block = vault.block(id)?;
    let mut visited = HashSet::new();
    visited.insert(id.clone());
    Some(render_flat_body(vault, block, &mut visited, Some(set)))
}

/// The resolved targets of every `[[reference]]` that appears in the flat output of `id` — i.e.
/// references in `id`'s own body **plus** references surfaced through the blocks it transitively
/// embeds (since `![[embeds]]` are dissolved inline). Unresolvable references are skipped; embed
/// recursion is cycle-safe. Order-preserving and de-duplicated. Used by the export layer to (a)
/// warn about links that leave the export set and (b) expand the set with `--follow-links`.
pub fn flat_reference_targets(vault: &Vault, id: &BlockId) -> Vec<BlockId> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    collect_flat_ref_targets(vault, id, &mut seen, &mut out);
    out
}

fn collect_flat_ref_targets(
    vault: &Vault,
    id: &BlockId,
    seen: &mut HashSet<BlockId>,
    out: &mut Vec<BlockId>,
) {
    if !seen.insert(id.clone()) {
        return;
    }
    let Some(block) = vault.block(id) else {
        return;
    };
    for r in extract_references(&block.body) {
        let Some(target) = vault.resolve(&r.target) else {
            continue;
        };
        if r.embed {
            // Walk the embedded subtree for the references it contributes to the flat output.
            collect_flat_ref_targets(vault, &target, seen, out);
        } else if !out.contains(&target) {
            out.push(target);
        }
    }
}

fn render_flat_body(
    vault: &Vault,
    block: &Block,
    visited: &mut HashSet<BlockId>,
    set: Option<&FlatSet>,
) -> String {
    let refs = extract_references(&block.body);
    if refs.is_empty() {
        return block.body.clone();
    }
    let mut out = String::new();
    let mut cursor = 0usize;
    for r in refs {
        out.push_str(&block.body[cursor..r.span.start]);
        if r.embed {
            // Indentation is a property of the *call site*: dissolve the child, then re-indent
            // its continuation lines to the column where `![[` starts, so a multi-line block
            // dropped into an indented context (a YAML scalar, a list item) stays well-formed.
            // At column 0 (a block on its own line — the common case) this is a no-op.
            let col = current_line_width(&out);
            let dissolved = dissolve_embed(vault, &r.target, r.label(), visited, set);
            out.push_str(&reindent_continuation(&dissolved, col));
        } else {
            out.push_str(&flat_reference(vault, &r.target, r.label(), set));
        }
        cursor = r.span.end;
    }
    out.push_str(&block.body[cursor..]);
    out
}

/// Width (in chars) of the current, in-progress last line of `s` — i.e. how far indented the
/// next text appended to `s` will start. Used as the re-indent column for a transcluded block.
pub(crate) fn current_line_width(s: &str) -> usize {
    match s.rfind('\n') {
        Some(i) => s[i + 1..].chars().count(),
        None => s.chars().count(),
    }
}

/// Prefix every line **after the first** with `col` spaces (the first line is already positioned
/// by the text preceding the directive). Blank lines are left empty (no trailing whitespace).
/// `col == 0` returns the text unchanged.
///
/// Indentation is always derived from the call site (the directive's column); there is
/// **deliberately no per-directive override** (e.g. a "flush-left" / "verbatim" flag). This is a
/// YAGNI decision, not a structural limitation: an override is perfectly buildable, but it would
/// add syntax/surface to maintain and a second way to think about `![[]]`. The call-site default
/// is correct for every case we actually have (YAML scalars, list items, prose), so we keep the
/// model simple — "everything is a block, reuse is `![[]]`" — and would only add an override if a
/// concrete need appears.
pub(crate) fn reindent_continuation(text: &str, col: usize) -> String {
    if col == 0 {
        return text.to_string();
    }
    let pad = " ".repeat(col);
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(&pad);
            }
        }
        out.push_str(line);
    }
    out
}

/// A `[[reference]]` in flat output. By default the target's display title (or the author's
/// alias) as **plain text** — a standalone `.md` doc can't resolve the `mdkb:` scheme. When a
/// `set` is provided and the target is **in it**, render a real relative Markdown link to the
/// target's output file instead, so co-exported docs cross-link.
fn flat_reference(vault: &Vault, target: &str, label: &str, set: Option<&FlatSet>) -> String {
    let Some(id) = vault.resolve(target) else {
        // Dangling target: nothing to resolve — show the label as written.
        return label.to_string();
    };
    let text = if label == target {
        vault
            .block(&id)
            .map(|b| b.display_title())
            .unwrap_or_else(|| label.to_string())
    } else {
        label.to_string()
    };
    if let Some(set) = set {
        if let Some(path) = set.paths.get(&id) {
            return format!("[{text}]({})", relative_link(set.self_path, path));
        }
    }
    text
}

/// A `![[embed]]` in flat output → the target's resolved body, dissolved inline (recursively).
/// Cycles and dangling targets degrade to an invisible HTML comment so the doc stays intact.
fn dissolve_embed(
    vault: &Vault,
    target: &str,
    label: &str,
    visited: &mut HashSet<BlockId>,
    set: Option<&FlatSet>,
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
    let body = render_flat_body(vault, block, visited, set);
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
    fn relative_link_same_dir_is_bare_filename() {
        assert_eq!(relative_link("a.md", "b.md"), "b.md");
        assert_eq!(relative_link("docs/a.md", "docs/b.md"), "b.md");
    }

    #[test]
    fn relative_link_walks_up_and_down() {
        assert_eq!(
            relative_link("docs/skills/cli/SKILL.md", "docs/SPEC.md"),
            "../../SPEC.md"
        );
        assert_eq!(relative_link("a.md", "sub/b.md"), "sub/b.md");
        assert_eq!(relative_link("x/a.md", "y/b.md"), "../y/b.md");
    }

    #[test]
    fn flat_reference_in_set_becomes_relative_link() {
        let mut v = Vault::new();
        let target = BlockId::generate();
        let src = BlockId::generate();
        v.insert_source(target.clone(), "---\ntitle: Spec\n---\nbody\n");
        v.insert_source(src.clone(), &format!("see [[{target}]] now\n"));

        let mut paths = HashMap::new();
        paths.insert(target.clone(), "spec.md".to_string());
        let set = FlatSet {
            paths: &paths,
            self_path: "guide.md",
        };
        let out = render_flat_in_set(&v, &src, &set).unwrap();
        assert_eq!(out, "see [Spec](spec.md) now\n");
    }

    #[test]
    fn flat_reference_out_of_set_stays_plain_text() {
        let mut v = Vault::new();
        let target = BlockId::generate();
        let src = BlockId::generate();
        v.insert_source(target.clone(), "---\ntitle: Spec\n---\nbody\n");
        v.insert_source(src.clone(), &format!("see [[{target}]] now\n"));

        // Empty set → the target isn't being exported → plain text, as without a set.
        let paths = HashMap::new();
        let set = FlatSet {
            paths: &paths,
            self_path: "guide.md",
        };
        let out = render_flat_in_set(&v, &src, &set).unwrap();
        assert_eq!(out, "see Spec now\n");
    }

    #[test]
    fn flat_reference_targets_includes_links_through_embeds() {
        let mut v = Vault::new();
        let linked = BlockId::generate();
        let child = BlockId::generate();
        let page = BlockId::generate();
        v.insert_source(linked.clone(), "---\ntitle: Linked\n---\nbody\n");
        // The child (embedded into the page) carries the only reference.
        v.insert_source(child.clone(), &format!("child links [[{linked}]]\n"));
        v.insert_source(page.clone(), &format!("# Page\n\n![[{child}]]\n"));

        let targets = flat_reference_targets(&v, &page);
        assert_eq!(targets, vec![linked], "should surface refs through embeds");
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

    #[test]
    fn flat_embed_at_column_zero_is_unchanged() {
        // The common case: a directive on its own line. Continuation lines must NOT gain indent.
        let mut v = Vault::new();
        let child = BlockId::generate();
        let host = BlockId::generate();
        v.insert_source(child.clone(), "line one\nline two\nline three\n");
        v.insert_source(host.clone(), &format!("# Doc\n\n![[{child}]]\n"));
        let out = render_flat(&v, &host).unwrap();
        assert!(out.contains("# Doc"));
        assert!(
            out.contains("\nline one\nline two\nline three"),
            "no indent at col 0:\n{out}"
        );
    }

    #[test]
    fn flat_embed_reindents_continuation_lines_in_indented_context() {
        // The real skill case: the block's OWN title frontmatter is consumed, leaving a second
        // (skill) frontmatter block in the body whose `desc:` scalar embeds a multi-line block at
        // a 4-space indent. Every continuation line must gain that indent so the YAML stays valid.
        let mut v = Vault::new();
        let child = BlockId::generate();
        let host = BlockId::generate();
        v.insert_source(child.clone(), "line one\nline two\nline three\n");
        v.insert_source(
            host.clone(),
            &format!("---\ntitle: Page\n---\n\n---\ndesc: >-\n    ![[{child}]]\n---\nbody\n"),
        );
        let out = render_flat(&v, &host).unwrap();
        assert!(
            out.contains("    line one\n    line two\n    line three"),
            "got:\n{out}"
        );
        assert!(
            !out.contains("\nline two"),
            "continuation not re-indented:\n{out}"
        );
    }

    #[test]
    fn flat_embed_reindents_under_list_item() {
        let mut v = Vault::new();
        let child = BlockId::generate();
        let host = BlockId::generate();
        v.insert_source(child.clone(), "first\nsecond\n");
        v.insert_source(host.clone(), &format!("- ![[{child}]]\n"));
        let out = render_flat(&v, &host).unwrap();
        // "- " is 2 columns, so the continuation aligns under the item content.
        assert!(out.contains("- first\n  second"), "got:\n{out}");
    }

    #[test]
    fn flat_inline_single_line_embed_splices_mid_sentence() {
        let mut v = Vault::new();
        let tag = BlockId::generate();
        let host = BlockId::generate();
        v.insert_source(tag.clone(), "the fast knowledge base");
        v.insert_source(
            host.clone(),
            &format!("mdkb is ![[{tag}]] for developers.\n"),
        );
        let out = render_flat(&v, &host).unwrap();
        assert_eq!(out, "mdkb is the fast knowledge base for developers.\n");
    }

    #[test]
    fn flat_list_embedded_in_list_item_nests() {
        // `- ![[sublist]]` must keep the embedded list's items nested under the outer item, not
        // let them escape to the outer level. The re-indent (call-site column 2) achieves this;
        // a real CommonMark renderer reads it as an outer item containing a nested <ul>.
        let mut v = Vault::new();
        let sub = BlockId::generate();
        let host = BlockId::generate();
        v.insert_source(sub.clone(), "- item a\n- item b\n- item c\n");
        v.insert_source(host.clone(), &format!("- before\n- ![[{sub}]]\n- after\n"));
        let out = render_flat(&v, &host).unwrap();
        assert!(
            out.contains("- - item a\n  - item b\n  - item c"),
            "got:\n{out}"
        );
        // continuation items must NOT be flush-left (which would un-nest them)
        assert!(
            !out.contains("\n- item b"),
            "sub-item escaped the nesting:\n{out}"
        );
    }
}
