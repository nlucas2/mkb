//! The in-memory block model.
//!
//! A [`Block`] is the atomic addressable unit of a document: a heading, paragraph,
//! fenced code block, block quote, list item, etc. Parsing lives in [`crate::document`];
//! this module just defines the data.

use std::ops::Range;

use crate::id::BlockId;

/// The structural kind of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    /// An ATX heading (`#`..`######`) with its level (1-6).
    Heading { level: u8 },
    /// A plain paragraph (one or more non-blank lines).
    Paragraph,
    /// A fenced code block. The fence language, if any, is on [`Block::lang`].
    CodeFence,
    /// A block quote (`>`-prefixed lines).
    Quote,
    /// A single list item (and any of its continuation/nested lines).
    ListItem,
    /// A standalone HTML block that is not an mdkb id marker.
    Html,
    /// A thematic break (`---`, `***`, `___`).
    ThematicBreak,
}

impl BlockKind {
    /// The structural kind as a short stable string (for indexing/filtering).
    pub fn kind_str(&self) -> &'static str {
        match self {
            BlockKind::Heading { .. } => "heading",
            BlockKind::Paragraph => "paragraph",
            BlockKind::CodeFence => "code",
            BlockKind::Quote => "quote",
            BlockKind::ListItem => "list",
            BlockKind::Html => "html",
            BlockKind::ThematicBreak => "break",
        }
    }

    /// The heading level, if this is a heading.
    pub fn heading_level(&self) -> Option<u8> {
        match self {
            BlockKind::Heading { level } => Some(*level),
            _ => None,
        }
    }
}

/// Where a tag came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagSource {
    /// An inline `#tag` written in the block text.
    Inline,
    /// Inherited from the page's YAML frontmatter `tags:`.
    Frontmatter,
    /// Implied by a fenced code block's language (e.g. ```` ```kusto ````).
    Lang,
}

/// A tag attached to a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// The tag text, without a leading `#`.
    pub name: String,
    /// How the tag was derived.
    pub source: TagSource,
}

impl Tag {
    /// Convenience constructor.
    pub fn new(name: impl Into<String>, source: TagSource) -> Self {
        Tag {
            name: name.into(),
            source,
        }
    }
}

/// A parsed block.
///
/// `content` is the exact source text of the block (excluding its id marker line).
/// `content_range` and `marker_range` index into the owning [`crate::document::Document`]'s
/// source string and exist so writers can splice edits in without disturbing the rest of
/// the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Stable identity.
    pub id: BlockId,
    /// Structural kind.
    pub kind: BlockKind,
    /// Fence language for [`BlockKind::CodeFence`], else `None`.
    pub lang: Option<String>,
    /// Breadcrumb of ancestor heading texts, nearest-last.
    pub lineage: Vec<String>,
    /// Tags attached to this block.
    pub tags: Vec<Tag>,
    /// Exact block source text, without the id marker line.
    pub content: String,
    /// Byte range of `content` within the document source.
    pub content_range: Range<usize>,
    /// Byte range of the existing id marker line within the document source, if the block
    /// already carried one on disk. `None` means the id was assigned in memory and is not
    /// yet persisted.
    pub marker_range: Option<Range<usize>>,
}

impl Block {
    /// Whether this block's id was already present on disk.
    pub fn has_persisted_id(&self) -> bool {
        self.marker_range.is_some()
    }

    /// The text used for embedding/search: lineage breadcrumb prepended to the content,
    /// which fixes "context starvation" (a bare block is meaningless without its heading
    /// path).
    pub fn contextual_text(&self) -> String {
        if self.lineage.is_empty() {
            self.content.clone()
        } else {
            format!("{}\n\n{}", self.lineage.join(" > "), self.content)
        }
    }

    /// All tag names (deduplicated, order-preserving) regardless of source.
    pub fn tag_names(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for t in &self.tags {
            if !seen.contains(&t.name.as_str()) {
                seen.push(t.name.as_str());
            }
        }
        seen
    }
}
