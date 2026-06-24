//! The in-memory block model.
//!
//! In the **file-per-block** model a [`Block`] *is* a file (`blocks/<ulid>.md`): the ULID is
//! the filename stem, the block's content is the file body, and the directives inside that
//! body define its edges — `![[target]]` marks a **child** (transclusion) and `[[target]]` a
//! plain **reference**. Parsing a file into a `Block` lives in [`crate::blockfile`]; this
//! module just defines the data and derived views.

use crate::id::BlockId;
use crate::link::{extract_references, Reference};

/// A single block: one file in the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Stable identity (the filename stem).
    pub id: BlockId,
    /// Optional human title (from frontmatter `title:`).
    pub title: Option<String>,
    /// All tags attached to this block (frontmatter `tags:` + inline `#tag`), deduplicated.
    /// This is the union used for display and search.
    pub tags: Vec<String>,
    /// The **managed** tags — those declared in frontmatter `tags:`. A subset of `tags`; the
    /// rest are inline `#hashtag` "mentions" that live in the prose. The tag editor manages
    /// only this set; inline mentions are changed by editing the body.
    pub fm_tags: Vec<String>,
    /// Fenced-code languages appearing in the body (for language-filtered search).
    pub langs: Vec<String>,
    /// **Human-only** flag (frontmatter `locked: true`). A locked block is fully readable by any
    /// caller but may only be *written* (or unlocked) by a human principal — agent clients (MCP,
    /// CLI) are denied mutations. Enforced at the service write gate; see `RequestContext`.
    pub locked: bool,
    /// **Arbitrary block properties** — every frontmatter `key: value` that isn't one mdkb manages
    /// (`title`/`tags`/`locked`/`created`/`updated`), as ordered **scalar** key/value pairs (e.g.
    /// `source:`, `verified:`, `confidence:`): the open-ended metadata primitive. Scalar properties
    /// are stored and round-tripped exactly, and folded into [`Block::contextual_text`] so values
    /// are full-text searchable. The model is deliberately flat — only scalar values are captured;
    /// non-scalar frontmatter (nested maps, block lists, block scalars) under an unmanaged key is
    /// not represented here, and `set_props` can only write scalars. mdkb imposes no schema on
    /// these — they are not typed or range-queryable (that is a separate, opt-in concern). Keys are
    /// unique (first occurrence wins on parse); order is first-seen.
    pub props: Vec<(String, String)>,
    /// **Last-modified timestamp** (frontmatter `updated:`, RFC 3339 UTC) — system-owned and
    /// stamped on every write; callers cannot set it. `None` for a block never written since this
    /// was introduced (absence is normal and never an error). The complementary **created** time is
    /// not stored here at all — it is decoded from the block's ULID id via [`Block::created`].
    pub updated: Option<String>,
    /// The Markdown body (everything after frontmatter), verbatim.
    pub body: String,
}

impl Block {
    /// All directives in the body, in source order.
    pub fn references(&self) -> Vec<Reference> {
        extract_references(&self.body)
    }

    /// The raw targets of the block's **children** (`![[...]]`), in order. Targets are ULIDs
    /// or titles; resolution to ids is the vault's job.
    pub fn child_targets(&self) -> Vec<String> {
        self.references()
            .into_iter()
            .filter(|r| r.embed)
            .map(|r| r.target)
            .collect()
    }

    /// The raw targets of the block's plain **references** (`[[...]]`), in order.
    pub fn reference_targets(&self) -> Vec<String> {
        self.references()
            .into_iter()
            .filter(|r| !r.embed)
            .map(|r| r.target)
            .collect()
    }

    /// A short display title: the explicit title, else the first non-empty line of the body
    /// (stripped of Markdown heading markers), else the id.
    pub fn display_title(&self) -> String {
        if let Some(t) = &self.title {
            if !t.trim().is_empty() {
                return t.trim().to_string();
            }
        }
        for line in self.body.lines() {
            let t = line.trim().trim_start_matches('#').trim();
            if !t.is_empty() {
                let t = strip_inline_markup(t);
                return truncate_chars(&t, 80);
            }
        }
        self.id.to_string()
    }

    /// The text used for embedding/search: the title (context) prepended to the plain-text
    /// body, with directives reduced to their labels, and any block properties' keys + values
    /// appended so open-ended metadata (e.g. a `source:` URL) is full-text searchable too.
    /// Mirrors the old "lineage-prepended" contextual text — a bare block is meaningless without
    /// its title/context.
    pub fn contextual_text(&self) -> String {
        let plain = directives_to_text(&self.body);
        let mut out = match &self.title {
            Some(t) if !t.trim().is_empty() => format!("{}\n\n{}", t.trim(), plain),
            _ => plain,
        };
        for (k, v) in &self.props {
            out.push('\n');
            out.push_str(k);
            out.push(' ');
            out.push_str(v);
        }
        out
    }

    /// The value of property `key`, if present (case-insensitive on the key).
    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// The block's **creation time** as an RFC 3339 UTC string, decoded from its ULID id (nothing
    /// is stored). `None` only if the id isn't a decodable ULID.
    pub fn created(&self) -> Option<String> {
        self.id.created_rfc3339()
    }

    /// All tag names (deduplicated, order-preserving).
    pub fn tag_names(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for t in &self.tags {
            if !seen.contains(&t.as_str()) {
                seen.push(t.as_str());
            }
        }
        seen
    }
}

/// Replace `[[t]]` / `![[t|label]]` directives with their label text, for plain-text uses
/// (search/embedding context) where the wiki syntax would only add noise.
fn directives_to_text(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut cursor = 0usize;
    for r in extract_references(body) {
        out.push_str(&body[cursor..r.span.start]);
        out.push_str(r.label());
        cursor = r.span.end;
    }
    out.push_str(&body[cursor..]);
    out
}

fn strip_inline_markup(s: &str) -> String {
    s.replace(['*', '_', '`'], "")
}

fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        let cut: String = s.chars().take(n).collect();
        format!("{cut}…")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(body: &str) -> Block {
        Block {
            id: BlockId::generate(),
            title: None,
            tags: vec![],
            fm_tags: vec![],
            langs: vec![],
            locked: false,
            props: vec![],
            updated: None,
            body: body.to_string(),
        }
    }

    #[test]
    fn separates_children_from_references() {
        let b = block("intro ![[01ARZ3NDEKTSV4RRFFQ69G5FAV]] and a [[link-target]] here");
        assert_eq!(b.child_targets(), vec!["01ARZ3NDEKTSV4RRFFQ69G5FAV"]);
        assert_eq!(b.reference_targets(), vec!["link-target"]);
    }

    #[test]
    fn contextual_text_prepends_title_and_flattens_directives() {
        let mut b = block("see [[ideas|the ideas page]] now");
        b.title = Some("Home".into());
        let ctx = b.contextual_text();
        assert!(ctx.starts_with("Home"));
        assert!(ctx.contains("the ideas page"));
        assert!(!ctx.contains("[["));
    }

    #[test]
    fn display_title_prefers_title_then_first_line() {
        let mut b = block("# Heading One\n\nbody");
        assert_eq!(b.display_title(), "Heading One");
        b.title = Some("Explicit".into());
        assert_eq!(b.display_title(), "Explicit");
    }

    #[test]
    fn contextual_text_includes_prop_keys_and_values() {
        let mut b = block("the body");
        b.props = vec![
            ("source".into(), "https://example.com/x".into()),
            ("verified".into(), "2026-06-01".into()),
        ];
        let ctx = b.contextual_text();
        assert!(ctx.contains("source https://example.com/x"));
        assert!(ctx.contains("verified 2026-06-01"));
    }

    #[test]
    fn prop_lookup_is_case_insensitive_on_key() {
        let mut b = block("body");
        b.props = vec![("Source".into(), "git".into())];
        assert_eq!(b.prop("source"), Some("git"));
        assert_eq!(b.prop("missing"), None);
    }
}
