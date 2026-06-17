//! Markdown → [`Document`] parsing.
//!
//! The parser is **line-oriented and fidelity-preserving**: every block records the byte
//! range it occupies in the original source so writers can splice edits (e.g. inject an id
//! marker) without reformatting the rest of the file. It recognises a pragmatic subset of
//! CommonMark sufficient for a knowledge base: YAML frontmatter, ATX headings, fenced code
//! blocks, block quotes, list items, thematic breaks, HTML blocks, and paragraphs.
//!
//! Block ids are encoded via an [`IdCodec`] (default [`NativeIdCodec`]) so the on-disk
//! marker format stays swappable.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

use crate::block::{Block, BlockKind, Tag, TagSource};
use crate::id::{BlockId, IdCodec, NativeIdCodec};

/// Parsed YAML frontmatter (only the bits mdkb cares about).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    /// Byte range of the whole frontmatter block (delimiters included) in the source.
    pub range: Range<usize>,
    /// Tags declared under `tags:`.
    pub tags: Vec<String>,
}

/// A parsed Markdown document: its source plus the blocks it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The original source text, owned and unmodified.
    pub source: String,
    /// Frontmatter, if the document began with a `---` fenced YAML block.
    pub frontmatter: Option<Frontmatter>,
    /// Blocks in source order.
    pub blocks: Vec<Block>,
}

struct Line<'a> {
    start: usize,
    end: usize,
    content_end: usize,
    text: &'a str,
}

fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(#{1,6})(?:\s+(.*?))?\s*#*\s*$").expect("heading re"))
}

fn list_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\s*)([-*+]|\d{1,9}[.)])\s+\S").expect("list re"))
}

fn thematic_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:-\s*){3,}$|^\s*(?:\*\s*){3,}$|^\s*(?:_\s*){3,}$").unwrap()
    })
}

fn inline_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|[^\w&/])#([A-Za-z][\w/-]*)").expect("inline tag re"))
}

fn leading_ws(s: &str) -> usize {
    s.len() - s.trim_start().len()
}

fn split_lines(s: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < s.len() {
        if bytes[i] == b'\n' {
            let mut ce = i;
            if ce > start && bytes[ce - 1] == b'\r' {
                ce -= 1;
            }
            out.push(Line {
                start,
                end: i + 1,
                content_end: ce,
                text: &s[start..ce],
            });
            start = i + 1;
            i = start;
        } else {
            i += 1;
        }
    }
    if start < s.len() {
        out.push(Line {
            start,
            end: s.len(),
            content_end: s.len(),
            text: &s[start..],
        });
    }
    out
}

fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

fn fence_marker(text: &str) -> Option<(char, usize, String)> {
    let t = text.trim_start();
    let first = t.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = t.chars().take_while(|&c| c == first).count();
    if count < 3 {
        return None;
    }
    let info = t[count..].trim().to_string();
    Some((first, count, info))
}

/// Returns the id if `text` is a line consisting solely of an id marker (plus whitespace).
fn marker_only(codec: &dyn IdCodec, text: &str) -> Option<BlockId> {
    let m = codec.find_first(text)?;
    if text[..m.start].trim().is_empty() && text[m.end..].trim().is_empty() {
        Some(m.id)
    } else {
        None
    }
}

/// True if `text` begins a block construct that interrupts an open paragraph.
fn interrupts_paragraph(codec: &dyn IdCodec, text: &str) -> bool {
    if is_blank(text) {
        return true;
    }
    if marker_only(codec, text).is_some() {
        return true;
    }
    if fence_marker(text).is_some() {
        return true;
    }
    if heading_re().is_match(text) {
        return true;
    }
    if thematic_re().is_match(text) {
        return true;
    }
    if list_re().is_match(text) {
        return true;
    }
    let trimmed = text.trim_start();
    trimmed.starts_with('>')
}

impl Document {
    /// Parse using the native id codec.
    pub fn parse(source: impl Into<String>) -> Document {
        Document::parse_with(&NativeIdCodec, source)
    }

    /// Parse using a specific [`IdCodec`] for marker recognition.
    pub fn parse_with(codec: &dyn IdCodec, source: impl Into<String>) -> Document {
        let source = source.into();
        let lines = split_lines(&source);
        let mut idx = 0usize;

        let frontmatter = parse_frontmatter(&lines, &mut idx);
        let fm_tags = frontmatter
            .as_ref()
            .map(|f| f.tags.clone())
            .unwrap_or_default();

        let mut blocks = Vec::new();
        let mut lineage_stack: Vec<(u8, String)> = Vec::new();
        let mut pending: Option<(BlockId, Range<usize>)> = None;

        while idx < lines.len() {
            let line = &lines[idx];
            if is_blank(line.text) {
                idx += 1;
                continue;
            }
            if let Some(id) = marker_only(codec, line.text) {
                pending = Some((id, line.start..line.end));
                idx += 1;
                continue;
            }

            let (kind, lang, end_line) = classify(codec, &lines, idx);
            let content_start = lines[idx].start;
            let content_end = lines[end_line - 1].content_end;
            let content = source[content_start..content_end].to_string();

            // Lineage: pop siblings/descendants before recording a heading's own ancestry.
            if let BlockKind::Heading { level } = kind {
                while lineage_stack.last().is_some_and(|(l, _)| *l >= level) {
                    lineage_stack.pop();
                }
            }
            let lineage: Vec<String> = lineage_stack.iter().map(|(_, t)| t.clone()).collect();

            let (id, marker_range) = match pending.take() {
                Some((id, r)) => (id, Some(r)),
                None => (BlockId::generate(), None),
            };

            let tags = collect_tags(&fm_tags, &kind, lang.as_deref(), &content);

            if let BlockKind::Heading { level } = kind {
                let htext = heading_text(line.text);
                lineage_stack.push((level, htext));
            }

            blocks.push(Block {
                id,
                kind,
                lang,
                lineage,
                tags,
                content,
                content_range: content_start..content_end,
                marker_range,
            });
            idx = end_line;
        }

        Document {
            source,
            frontmatter,
            blocks,
        }
    }

    /// Produce a new source string with id markers inserted for every block that lacks a
    /// persisted one. Returns `None` if no block needed an id (the document is already
    /// fully addressable on disk).
    ///
    /// Only marker lines are inserted; all existing content is preserved byte-for-byte.
    pub fn with_assigned_ids(&self, codec: &dyn IdCodec) -> Option<String> {
        let inserts: Vec<(usize, String)> = self
            .blocks
            .iter()
            .filter(|b| b.marker_range.is_none())
            .map(|b| {
                let indent = &self.source
                    [b.content_range.start..b.content_range.start + leading_ws(&b.content)];
                (
                    b.content_range.start,
                    format!("{indent}{}\n", codec.encode(&b.id)),
                )
            })
            .collect();
        if inserts.is_empty() {
            return None;
        }
        let mut out = self.source.clone();
        // Splice from the end so earlier offsets stay valid.
        for (pos, text) in inserts.into_iter().rev() {
            out.insert_str(pos, &text);
        }
        Some(out)
    }

    /// Look up a block by id.
    pub fn block(&self, id: &BlockId) -> Option<&Block> {
        self.blocks.iter().find(|b| &b.id == id)
    }
}

fn heading_text(line: &str) -> String {
    heading_re()
        .captures(line)
        .and_then(|c| c.get(2))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_default()
}

fn parse_frontmatter(lines: &[Line<'_>], idx: &mut usize) -> Option<Frontmatter> {
    if lines.is_empty() || lines[0].text.trim() != "---" {
        return None;
    }
    let mut j = 1;
    while j < lines.len() {
        let t = lines[j].text.trim();
        if t == "---" || t == "..." {
            let range = 0..lines[j].end;
            let tags = parse_fm_tags(&lines[1..j]);
            *idx = j + 1;
            return Some(Frontmatter { range, tags });
        }
        j += 1;
    }
    None
}

fn parse_fm_tags(body: &[Line<'_>]) -> Vec<String> {
    let mut tags = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let line = body[i].text;
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("tags:") {
            let rest = rest.trim();
            if rest.starts_with('[') {
                let inner = rest.trim_start_matches('[').trim_end_matches(']');
                for part in inner.split(',') {
                    push_tag(&mut tags, part);
                }
            } else if !rest.is_empty() {
                for part in rest.split(|c: char| c.is_whitespace() || c == ',') {
                    push_tag(&mut tags, part);
                }
            } else {
                // Block list form: subsequent `  - tag` lines.
                let mut j = i + 1;
                while j < body.len() {
                    let item = body[j].text.trim_start();
                    if let Some(t) = item.strip_prefix("- ") {
                        push_tag(&mut tags, t);
                        j += 1;
                    } else if item.is_empty() {
                        j += 1;
                    } else {
                        break;
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    tags
}

fn push_tag(tags: &mut Vec<String>, raw: &str) {
    let t = raw.trim().trim_matches(['"', '\'', '#']).trim();
    if !t.is_empty() && !tags.iter().any(|x| x == t) {
        tags.push(t.to_string());
    }
}

fn collect_tags(
    fm_tags: &[String],
    kind: &BlockKind,
    lang: Option<&str>,
    content: &str,
) -> Vec<Tag> {
    let mut tags: Vec<Tag> = Vec::new();
    let mut add = |name: &str, source: TagSource| {
        if !name.is_empty() && !tags.iter().any(|t| t.name == name && t.source == source) {
            tags.push(Tag::new(name, source));
        }
    };
    for t in fm_tags {
        add(t, TagSource::Frontmatter);
    }
    if matches!(kind, BlockKind::CodeFence) {
        if let Some(l) = lang {
            if !l.is_empty() {
                add(l, TagSource::Lang);
            }
        }
    }
    // Inline #tags are not meaningful inside code; skip code fences for inline scanning.
    if !matches!(kind, BlockKind::CodeFence) {
        for caps in inline_tag_re().captures_iter(content) {
            if let Some(m) = caps.get(1) {
                add(m.as_str(), TagSource::Inline);
            }
        }
    }
    tags
}

/// Determine the kind of the block starting at `lines[start]` and the exclusive end line.
fn classify(
    codec: &dyn IdCodec,
    lines: &[Line<'_>],
    start: usize,
) -> (BlockKind, Option<String>, usize) {
    let text = lines[start].text;

    if let Some((fence_char, fence_len, info)) = fence_marker(text) {
        let mut j = start + 1;
        while j < lines.len() {
            if let Some((c, len, rest)) = fence_marker(lines[j].text) {
                if c == fence_char && len >= fence_len && rest.is_empty() {
                    j += 1;
                    break;
                }
            }
            j += 1;
        }
        let lang = info.split_whitespace().next().map(|s| s.to_string());
        return (BlockKind::CodeFence, lang.filter(|s| !s.is_empty()), j);
    }

    if let Some(caps) = heading_re().captures(text) {
        let level = caps.get(1).unwrap().as_str().len() as u8;
        return (BlockKind::Heading { level }, None, start + 1);
    }

    if thematic_re().is_match(text) {
        return (BlockKind::ThematicBreak, None, start + 1);
    }

    if list_re().is_match(text) {
        let indent = leading_ws(text);
        let mut j = start + 1;
        while j < lines.len() {
            let lt = lines[j].text;
            if is_blank(lt) {
                break;
            }
            let lindent = leading_ws(lt);
            if list_re().is_match(lt) && lindent <= indent {
                break;
            }
            if lindent > indent {
                j += 1;
                continue;
            }
            break;
        }
        return (BlockKind::ListItem, None, j);
    }

    if text.trim_start().starts_with('>') {
        let mut j = start + 1;
        while j < lines.len()
            && !is_blank(lines[j].text)
            && lines[j].text.trim_start().starts_with('>')
        {
            j += 1;
        }
        return (BlockKind::Quote, None, j);
    }

    if text.trim_start().starts_with('<') {
        let mut j = start + 1;
        while j < lines.len()
            && !is_blank(lines[j].text)
            && marker_only(codec, lines[j].text).is_none()
        {
            j += 1;
        }
        return (BlockKind::Html, None, j);
    }

    // Paragraph: consume until a blank line or an interrupting construct.
    let mut j = start + 1;
    while j < lines.len() && !interrupts_paragraph(codec, lines[j].text) {
        j += 1;
    }
    (BlockKind::Paragraph, None, j)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(doc: &Document) -> Vec<&BlockKind> {
        doc.blocks.iter().map(|b| &b.kind).collect()
    }

    #[test]
    fn parses_basic_block_kinds() {
        let src = "# Title\n\nA paragraph here.\n\n- item one\n- item two\n\n> a quote\n\n---\n";
        let doc = Document::parse(src);
        assert_eq!(
            kinds(&doc),
            vec![
                &BlockKind::Heading { level: 1 },
                &BlockKind::Paragraph,
                &BlockKind::ListItem,
                &BlockKind::ListItem,
                &BlockKind::Quote,
                &BlockKind::ThematicBreak,
            ]
        );
    }

    #[test]
    fn code_fence_captures_lang_and_is_one_block() {
        let src = "```kusto\nStormEvents\n| take 10\n```\n";
        let doc = Document::parse(src);
        assert_eq!(doc.blocks.len(), 1);
        let b = &doc.blocks[0];
        assert_eq!(b.kind, BlockKind::CodeFence);
        assert_eq!(b.lang.as_deref(), Some("kusto"));
        assert_eq!(b.content, "```kusto\nStormEvents\n| take 10\n```");
        // The fence language becomes a lang-sourced tag (enables "all kusto queries").
        assert!(b
            .tags
            .iter()
            .any(|t| t.name == "kusto" && t.source == TagSource::Lang));
    }

    #[test]
    fn code_fence_does_not_split_on_blank_lines() {
        let src = "```\nline 1\n\nline 3\n```\n";
        let doc = Document::parse(src);
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].kind, BlockKind::CodeFence);
    }

    #[test]
    fn lineage_tracks_heading_hierarchy() {
        let src = "# A\n\n## B\n\ntext under B\n\n# C\n\ntext under C\n";
        let doc = Document::parse(src);
        let para_b = doc
            .blocks
            .iter()
            .find(|b| b.content == "text under B")
            .unwrap();
        assert_eq!(para_b.lineage, vec!["A".to_string(), "B".to_string()]);
        let para_c = doc
            .blocks
            .iter()
            .find(|b| b.content == "text under C")
            .unwrap();
        assert_eq!(para_c.lineage, vec!["C".to_string()]);
        // A heading's own lineage excludes itself.
        let hb = doc
            .blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Heading { level: 2 }))
            .unwrap();
        assert_eq!(hb.lineage, vec!["A".to_string()]);
    }

    #[test]
    fn contextual_text_prepends_lineage() {
        let src = "# Server\n\n## Nginx\n\nrestart it\n";
        let doc = Document::parse(src);
        let b = doc
            .blocks
            .iter()
            .find(|b| b.content == "restart it")
            .unwrap();
        assert_eq!(b.contextual_text(), "Server > Nginx\n\nrestart it");
    }

    #[test]
    fn frontmatter_tags_apply_to_blocks() {
        let src = "---\ntitle: x\ntags: [kusto, ops]\n---\n\nsome note\n";
        let doc = Document::parse(src);
        assert!(doc.frontmatter.is_some());
        let b = &doc.blocks[0];
        assert!(b
            .tags
            .iter()
            .any(|t| t.name == "kusto" && t.source == TagSource::Frontmatter));
        assert!(b.tags.iter().any(|t| t.name == "ops"));
    }

    #[test]
    fn frontmatter_block_list_tags() {
        let src = "---\ntags:\n  - alpha\n  - beta\n---\n\nbody\n";
        let doc = Document::parse(src);
        let names: Vec<_> = doc.blocks[0].tag_names();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn inline_tags_detected_outside_code() {
        let src = "A note about #nginx and #kql/ops here.\n";
        let doc = Document::parse(src);
        let names = doc.blocks[0].tag_names();
        assert!(names.contains(&"nginx"));
        assert!(names.contains(&"kql/ops"));
    }

    #[test]
    fn heading_hash_is_not_a_tag() {
        let src = "## Heading Title\n";
        let doc = Document::parse(src);
        assert!(doc.blocks[0].tags.is_empty());
    }

    #[test]
    fn assign_ids_is_idempotent_and_minimal() {
        let src = "# Title\n\nbody paragraph\n";
        let doc = Document::parse(src);
        let assigned = doc
            .with_assigned_ids(&NativeIdCodec)
            .expect("ids to assign");
        // Re-parsing finds persisted ids and assigns nothing further.
        let doc2 = Document::parse(&assigned);
        assert!(doc2.blocks.iter().all(|b| b.has_persisted_id()));
        assert!(doc2.with_assigned_ids(&NativeIdCodec).is_none());
        // Ids are stable across the round trip.
        let ids1: Vec<_> = doc.blocks.iter().map(|b| b.id.clone()).collect();
        let ids2: Vec<_> = doc2.blocks.iter().map(|b| b.id.clone()).collect();
        assert_eq!(ids1, ids2);
        // Only marker lines were added; stripping them recovers the original content lines.
        let stripped = NativeIdCodec.strip(&assigned);
        let recovered: Vec<&str> = stripped.lines().filter(|l| !l.trim().is_empty()).collect();
        let original: Vec<&str> = src.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(recovered, original);
    }

    #[test]
    fn assign_preserves_content_exactly_between_markers() {
        let src = "first para\n\nsecond para\n";
        let doc = Document::parse(src);
        let assigned = doc.with_assigned_ids(&NativeIdCodec).unwrap();
        let doc2 = Document::parse(&assigned);
        assert_eq!(doc2.blocks[0].content, "first para");
        assert_eq!(doc2.blocks[1].content, "second para");
    }

    #[test]
    fn existing_marker_binds_to_following_block() {
        let id = BlockId::generate();
        let src = format!("{}\nthe paragraph\n", NativeIdCodec.encode(&id));
        let doc = Document::parse(&src);
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].id, id);
        assert_eq!(doc.blocks[0].content, "the paragraph");
        assert!(doc.blocks[0].has_persisted_id());
    }

    #[test]
    fn marker_terminates_preceding_paragraph() {
        let id = BlockId::generate();
        let src = format!("para one\n{}\npara two\n", NativeIdCodec.encode(&id));
        let doc = Document::parse(&src);
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(doc.blocks[0].content, "para one");
        assert_eq!(doc.blocks[1].content, "para two");
        assert_eq!(doc.blocks[1].id, id);
    }

    #[test]
    fn empty_document_has_no_blocks() {
        let doc = Document::parse("");
        assert!(doc.blocks.is_empty());
        assert!(doc.with_assigned_ids(&NativeIdCodec).is_none());
    }
}
