//! Wiki-style references: `[[target]]` (link) and `![[target]]` (embed / transclusion).
//!
//! A target is `page#anchor|display`, where every part is optional:
//! - `[[Page]]` — whole page.
//! - `[[Page#01ARZ...]]` — a specific block by id.
//! - `[[Page#Some Heading]]` — a block by heading text.
//! - `[[#01ARZ...]]` — a block in the *same* page.
//! - `[[Page|label]]` — with custom display text.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

use crate::id::BlockId;

/// What a reference points at within a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// A block addressed by its stable id.
    Id(BlockId),
    /// A block addressed by heading text (case-insensitive match).
    Heading(String),
}

/// The parsed target of a reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    /// Page name or path (without `.md`). `None` means "the current page".
    pub page: Option<String>,
    /// Block anchor within the page, if any.
    pub anchor: Option<Anchor>,
    /// Custom display text following a `|`.
    pub display: Option<String>,
}

/// A reference located within some block content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// `true` for `![[...]]` (embed/transclude), `false` for `[[...]]` (link).
    pub embed: bool,
    /// Where it points.
    pub target: LinkTarget,
    /// Byte range of the full `[[...]]` / `![[...]]` token within the content.
    pub span: Range<usize>,
}

fn ref_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(!?)\[\[([^\]\n]+)\]\]").expect("reference re"))
}

impl LinkTarget {
    /// Parse the inside of a `[[...]]` (the part between the brackets).
    pub fn parse_inner(inner: &str) -> LinkTarget {
        let (link_part, display) = match inner.split_once('|') {
            Some((l, d)) => (l.trim(), Some(d.trim().to_string())),
            None => (inner.trim(), None),
        };
        let (page, anchor_str) = match link_part.split_once('#') {
            Some((p, a)) => {
                let page = p.trim();
                let page = if page.is_empty() {
                    None
                } else {
                    Some(page.to_string())
                };
                (page, Some(a.trim()))
            }
            None => (Some(link_part.to_string()), None),
        };
        let anchor = anchor_str
            .filter(|s| !s.is_empty())
            .map(|a| match BlockId::parse(a) {
                Ok(id) => Anchor::Id(id),
                Err(_) => Anchor::Heading(a.to_string()),
            });
        LinkTarget {
            page,
            anchor,
            display,
        }
    }

    /// A human-friendly label for this target (used when rendering plain links).
    pub fn label(&self) -> String {
        if let Some(d) = &self.display {
            return d.clone();
        }
        match (&self.page, &self.anchor) {
            (Some(p), Some(Anchor::Heading(h))) => format!("{p} › {h}"),
            (Some(p), _) => p.clone(),
            (None, Some(Anchor::Heading(h))) => h.clone(),
            (None, Some(Anchor::Id(id))) => id.to_string(),
            (None, None) => String::new(),
        }
    }
}

/// Extract every reference from block content, in source order.
pub fn extract_references(content: &str) -> Vec<Reference> {
    ref_re()
        .captures_iter(content)
        .filter_map(|caps| {
            let whole = caps.get(0)?;
            let embed = !caps.get(1)?.as_str().is_empty();
            let inner = caps.get(2)?.as_str();
            Some(Reference {
                embed,
                target: LinkTarget::parse_inner(inner),
                span: whole.start()..whole.end(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_page_only() {
        let t = LinkTarget::parse_inner("Useful Queries");
        assert_eq!(t.page.as_deref(), Some("Useful Queries"));
        assert!(t.anchor.is_none());
    }

    #[test]
    fn parses_page_and_id_anchor() {
        let id = BlockId::generate();
        let t = LinkTarget::parse_inner(&format!("Queries#{id}"));
        assert_eq!(t.page.as_deref(), Some("Queries"));
        assert_eq!(t.anchor, Some(Anchor::Id(id)));
    }

    #[test]
    fn parses_heading_anchor() {
        let t = LinkTarget::parse_inner("Queries#Kusto Basics");
        assert_eq!(t.anchor, Some(Anchor::Heading("Kusto Basics".to_string())));
    }

    #[test]
    fn parses_same_page_anchor() {
        let t = LinkTarget::parse_inner("#Section");
        assert!(t.page.is_none());
        assert_eq!(t.anchor, Some(Anchor::Heading("Section".to_string())));
    }

    #[test]
    fn parses_display_override() {
        let t = LinkTarget::parse_inner("Page#Sec|click here");
        assert_eq!(t.display.as_deref(), Some("click here"));
        assert_eq!(t.label(), "click here");
    }

    #[test]
    fn extracts_links_and_embeds() {
        let content = "See [[A]] and embed ![[B#01ARZ3NDEKTSV4RRFFQ69G5FAV]] here.";
        let refs = extract_references(content);
        assert_eq!(refs.len(), 2);
        assert!(!refs[0].embed);
        assert_eq!(refs[0].target.page.as_deref(), Some("A"));
        assert!(refs[1].embed);
        assert_eq!(refs[1].target.page.as_deref(), Some("B"));
    }

    #[test]
    fn ignores_single_brackets() {
        assert!(extract_references("a [normal](link) and [single] brackets").is_empty());
    }
}
