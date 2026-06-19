//! Wiki directives inside a block's content: `[[target]]` (reference) and `![[target]]`
//! (transclusion / child).
//!
//! In the file-per-block model a directive's **target** is either a block's ULID (its
//! filename stem) or a human title; resolution to a concrete [`crate::id::BlockId`] happens
//! in the [`crate::vault::Vault`], which knows every block. A `|` introduces an optional
//! display alias: `[[<target>|label]]`.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

/// A wiki directive located within some block content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// `true` for `![[...]]` (transclusion / child), `false` for `[[...]]` (plain reference).
    pub embed: bool,
    /// The raw target token (a ULID or a title), trimmed, before any `|` alias.
    pub target: String,
    /// Optional display alias following a `|`.
    pub display: Option<String>,
    /// Byte range of the full `[[...]]` / `![[...]]` token within the content.
    pub span: Range<usize>,
}

impl Reference {
    /// The label to show for this directive: the alias if present, else the raw target.
    pub fn label(&self) -> &str {
        self.display.as_deref().unwrap_or(&self.target)
    }
}

fn ref_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(!?)\[\[([^\]\n]+)\]\]").expect("reference re"))
}

/// Extract every directive from block content, in source order.
pub fn extract_references(content: &str) -> Vec<Reference> {
    ref_re()
        .captures_iter(content)
        .filter_map(|caps| {
            let whole = caps.get(0)?;
            let embed = !caps.get(1)?.as_str().is_empty();
            let inner = caps.get(2)?.as_str();
            let (target, display) = match inner.split_once('|') {
                Some((t, d)) => (t.trim().to_string(), Some(d.trim().to_string())),
                None => (inner.trim().to_string(), None),
            };
            if target.is_empty() {
                return None;
            }
            Some(Reference {
                embed,
                target,
                display,
                span: whole.start()..whole.end(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_links_and_embeds_in_order() {
        let content = "See [[alpha]] and embed ![[01ARZ3NDEKTSV4RRFFQ69G5FAV]] here.";
        let refs = extract_references(content);
        assert_eq!(refs.len(), 2);
        assert!(!refs[0].embed);
        assert_eq!(refs[0].target, "alpha");
        assert!(refs[1].embed);
        assert_eq!(refs[1].target, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    }

    #[test]
    fn parses_display_alias() {
        let refs = extract_references("[[some-block|click here]]");
        assert_eq!(refs[0].target, "some-block");
        assert_eq!(refs[0].display.as_deref(), Some("click here"));
        assert_eq!(refs[0].label(), "click here");
    }

    #[test]
    fn label_falls_back_to_target() {
        let refs = extract_references("[[the-target]]");
        assert_eq!(refs[0].label(), "the-target");
    }

    #[test]
    fn ignores_single_brackets_and_empty() {
        assert!(extract_references("a [normal](link) and [single] brackets").is_empty());
        assert!(extract_references("[[]]").is_empty());
        assert!(extract_references("[[  ]]").is_empty());
    }
}
