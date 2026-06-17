//! Transclusion resolver: render a page with `![[...]]` embeds inlined.
//!
//! This is where "update a block once, every embed reflects it" actually happens — the
//! referencing page stores only the directive, and the resolver pulls the *current* text
//! of the target block at render time. Plain `[[...]]` links are rendered as their label.
//! Cycles are detected and broken with a visible placeholder.

use std::collections::HashSet;

use crate::block::{Block, BlockKind};
use crate::id::BlockId;
use crate::link::{extract_references, Anchor, LinkTarget};
use crate::vault::{Page, Vault};

/// Render a page to Markdown with all transclusions resolved.
///
/// Returns `None` if the page key does not resolve.
pub fn render_page(vault: &Vault, page_key: &str) -> Option<String> {
    let page = vault.page(page_key)?;
    let mut visited = HashSet::new();
    let rendered: Vec<String> = page
        .doc
        .blocks
        .iter()
        .map(|b| render_block(vault, page, b, &mut visited))
        .collect();
    Some(rendered.join("\n\n"))
}

/// Render a single block, resolving any references inside it.
pub fn render_block(
    vault: &Vault,
    page: &Page,
    block: &Block,
    visited: &mut HashSet<BlockId>,
) -> String {
    // Code is rendered verbatim — references inside code are literal text.
    if matches!(block.kind, BlockKind::CodeFence) {
        return block.content.clone();
    }

    let refs = extract_references(&block.content);
    if refs.is_empty() {
        return block.content.clone();
    }

    let mut out = String::new();
    let mut cursor = 0usize;
    for r in refs {
        out.push_str(&block.content[cursor..r.span.start]);
        if r.embed {
            out.push_str(&resolve_embed(vault, page, &r.target, visited));
        } else {
            out.push_str(&r.target.label());
        }
        cursor = r.span.end;
    }
    out.push_str(&block.content[cursor..]);
    out
}

fn resolve_embed(
    vault: &Vault,
    current: &Page,
    target: &LinkTarget,
    visited: &mut HashSet<BlockId>,
) -> String {
    let page = match &target.page {
        Some(name) => match vault.page(name) {
            Some(p) => p,
            None => return missing(target),
        },
        None => current,
    };

    match &target.anchor {
        None => {
            // Whole-page embed: inline all of the page's blocks.
            let parts: Vec<String> = page
                .doc
                .blocks
                .iter()
                .map(|b| guarded_block(vault, page, b, visited))
                .collect();
            parts.join("\n\n")
        }
        Some(anchor) => match find_block(page, anchor) {
            Some(b) => guarded_block(vault, page, b, visited),
            None => missing(target),
        },
    }
}

fn guarded_block(
    vault: &Vault,
    page: &Page,
    block: &Block,
    visited: &mut HashSet<BlockId>,
) -> String {
    if visited.contains(&block.id) {
        return format!("⟲ [transclusion cycle: {}]", block.id);
    }
    visited.insert(block.id.clone());
    let rendered = render_block(vault, page, block, visited);
    visited.remove(&block.id);
    rendered
}

fn find_block<'a>(page: &'a Page, anchor: &Anchor) -> Option<&'a Block> {
    match anchor {
        Anchor::Id(id) => page.doc.block(id),
        Anchor::Heading(text) => page.doc.blocks.iter().find(|b| {
            matches!(b.kind, BlockKind::Heading { .. })
                && heading_label(&b.content).eq_ignore_ascii_case(text.trim())
        }),
    }
}

fn heading_label(content: &str) -> String {
    content
        .trim_start()
        .trim_start_matches('#')
        .trim()
        .trim_end_matches('#')
        .trim()
        .to_string()
}

fn missing(target: &LinkTarget) -> String {
    format!("⚠️ [unresolved: {}]", target.label())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::Vault;

    fn vault_with_ids(pages: &[(&str, &str)]) -> Vault {
        let mut v = Vault::new();
        for (path, src) in pages {
            v.insert(*path, *src);
        }
        v.assign_ids();
        v
    }

    fn block_id(v: &Vault, page: &str, content_contains: &str) -> BlockId {
        v.page(page)
            .unwrap()
            .doc
            .blocks
            .iter()
            .find(|b| b.content.contains(content_contains))
            .unwrap()
            .id
            .clone()
    }

    #[test]
    fn renders_plain_page_unchanged() {
        let v = vault_with_ids(&[("a.md", "# Title\n\njust text\n")]);
        let out = render_page(&v, "a").unwrap();
        assert_eq!(out, "# Title\n\njust text");
    }

    #[test]
    fn update_once_reflects_everywhere() {
        // Master query lives on one page; two other pages embed it by id.
        let mut v =
            vault_with_ids(&[("useful-queries.md", "# Queries\n\nStormEvents | take 10\n")]);
        let qid = block_id(&v, "useful-queries", "StormEvents");
        v.insert(
            "project-x.md",
            format!("# Project X\n\n![[useful-queries#{qid}]]\n"),
        );
        v.insert(
            "project-y.md",
            format!("# Project Y\n\nSee: ![[useful-queries#{qid}]]\n"),
        );
        v.assign_ids();

        // Both pages show the current query text.
        assert!(render_page(&v, "project-x")
            .unwrap()
            .contains("StormEvents | take 10"));
        assert!(render_page(&v, "project-y")
            .unwrap()
            .contains("StormEvents | take 10"));

        // Update the master block once...
        v.update_block(&qid, "StormEvents | where State == 'TEXAS' | take 50")
            .unwrap();

        // ...and every embed reflects it. This is the SSOT guarantee.
        let x = render_page(&v, "project-x").unwrap();
        let y = render_page(&v, "project-y").unwrap();
        assert!(x.contains("where State == 'TEXAS'"));
        assert!(y.contains("where State == 'TEXAS'"));
        assert!(!x.contains("take 10"));
    }

    #[test]
    fn embeds_resolve_by_heading_anchor() {
        let v = vault_with_ids(&[
            ("src.md", "# Kusto Basics\n\nthe basics body\n"),
            ("dst.md", "intro ![[src#Kusto Basics]]\n"),
        ]);
        let out = render_page(&v, "dst").unwrap();
        assert!(out.contains("# Kusto Basics"));
    }

    #[test]
    fn whole_page_embed_inlines_all_blocks() {
        let v = vault_with_ids(&[("part.md", "alpha\n\nbeta\n"), ("host.md", "![[part]]\n")]);
        let out = render_page(&v, "host").unwrap();
        assert!(out.contains("alpha"));
        assert!(out.contains("beta"));
    }

    #[test]
    fn unresolved_embed_is_flagged() {
        let v = vault_with_ids(&[("a.md", "![[does-not-exist]]\n")]);
        let out = render_page(&v, "a").unwrap();
        assert!(out.contains("unresolved"));
    }

    #[test]
    fn cycles_are_broken() {
        // Two blocks embedding each other must not recurse forever.
        let mut v = Vault::new();
        v.insert("a.md", "AAA\n");
        v.insert("b.md", "BBB\n");
        v.assign_ids();
        let aid = block_id(&v, "a", "AAA");
        let bid = block_id(&v, "b", "BBB");
        v.update_block(&aid, &format!("A embeds ![[b#{bid}]]"))
            .unwrap();
        v.update_block(&bid, &format!("B embeds ![[a#{aid}]]"))
            .unwrap();
        let out = render_page(&v, "a").unwrap();
        assert!(out.contains("transclusion cycle"));
    }

    #[test]
    fn plain_links_render_as_label() {
        let v = vault_with_ids(&[("a.md", "see [[Other Page|the docs]] now\n")]);
        let out = render_page(&v, "a").unwrap();
        assert_eq!(out, "see the docs now");
    }
}
