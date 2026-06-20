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
///
/// Directives inside **code** — inline code spans (`` `...` ``) and fenced code blocks
/// (```` ``` ````/`~~~`) — are ignored, so literal `![[id]]` / `[[id]]` shown as code examples
/// are not parsed as real links. This mirrors how inline `#tags` already skip code.
pub fn extract_references(content: &str) -> Vec<Reference> {
    let mask = code_mask(content);
    ref_re()
        .captures_iter(content)
        .filter_map(|caps| {
            let whole = caps.get(0)?;
            // Skip directives that begin inside a code span/fence.
            if mask.get(whole.start()).copied().unwrap_or(false) {
                return None;
            }
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

/// A byte-indexed mask where `true` marks content that lives inside code (a fenced block or an
/// inline code span) and must not be scanned for directives.
fn code_mask(content: &str) -> Vec<bool> {
    let mut mask = vec![false; content.len()];
    let mut in_fence = false;
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if is_fence {
            mark(&mut mask, offset, offset + line.len());
            in_fence = !in_fence;
        } else if in_fence {
            mark(&mut mask, offset, offset + line.len());
        } else {
            mark_inline_code(line, offset, &mut mask);
        }
        offset += line.len();
    }
    mask
}

/// Mark `[start, end)` as code in the mask.
fn mark(mask: &mut [bool], start: usize, end: usize) {
    for b in mask.iter_mut().take(end).skip(start) {
        *b = true;
    }
}

/// Mark inline code spans (matched runs of backticks) within a single line as code, including
/// the backtick delimiters. An unterminated run leaves the rest of the line untouched.
fn mark_inline_code(line: &str, base: usize, mask: &mut [bool]) {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`' {
            let start = i;
            while i < b.len() && b[i] == b'`' {
                i += 1;
            }
            let run = i - start;
            // Find a closing run of the same length.
            let mut j = i;
            let mut closed = None;
            while j < b.len() {
                if b[j] == b'`' {
                    let rs = j;
                    while j < b.len() && b[j] == b'`' {
                        j += 1;
                    }
                    if j - rs == run {
                        closed = Some(j);
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            match closed {
                Some(end) => {
                    mark(mask, base + start, base + end);
                    i = end;
                }
                None => break,
            }
        } else {
            i += 1;
        }
    }
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

    #[test]
    fn ignores_directives_inside_inline_code() {
        // Literal syntax shown as inline code must not be parsed as a real directive.
        let refs = extract_references("Embed with `![[id]]` and reference with `[[id]]`.");
        assert!(refs.is_empty(), "got: {refs:?}");
    }

    #[test]
    fn ignores_directives_inside_fenced_code() {
        let content = "Example:\n\n```\n![[01ARZ3NDEKTSV4RRFFQ69G5FAV]]\n[[some-title]]\n```\n";
        assert!(extract_references(content).is_empty());
    }

    #[test]
    fn finds_real_directives_alongside_code_examples() {
        // A live reference plus a code example of the syntax: only the live one is extracted.
        let content =
            "See [[alpha]].\n\n```md\n![[not-a-real-embed]]\n```\n\nAlso `[[inline-example]]`.";
        let refs = extract_references(content);
        assert_eq!(refs.len(), 1, "got: {refs:?}");
        assert_eq!(refs[0].target, "alpha");
        assert!(!refs[0].embed);
    }
}
