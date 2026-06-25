//! Block identity and its on-disk encoding.
//!
//! Every block carries a stable [`BlockId`] (a ULID). On disk the id is encoded as an
//! invisible, namespaced HTML comment — `<!-- mkb:<ulid> -->` — via [`NativeIdCodec`].
//! All read/write of that marker goes through the [`IdCodec`] trait so an alternate
//! encoding (e.g. a future Obsidian-compatible `^id` codec) can be slotted in without
//! touching callers.

use std::fmt;
use std::sync::OnceLock;

use regex::Regex;

/// A stable, opaque block identifier.
///
/// Generated as a ULID (26-character Crockford base32). Treated as an opaque token by
/// the rest of the system — never parse meaning out of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct BlockId(String);

/// Error produced when a string is not a valid [`BlockId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError(String);

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid block id: {:?}", self.0)
    }
}

impl std::error::Error for IdError {}

impl BlockId {
    /// Mint a fresh, unique id.
    pub fn generate() -> Self {
        BlockId(ulid::Ulid::new().to_string())
    }

    /// Parse an existing id, validating its shape.
    ///
    /// Accepts a 26-character ASCII-alphanumeric token (the ULID shape). This is
    /// deliberately lenient about exact Crockford-alphabet membership; the id is opaque.
    pub fn parse(s: &str) -> Result<Self, IdError> {
        let s = s.trim();
        let is_valid = s.len() == 26 && s.bytes().all(|b| b.is_ascii_alphanumeric());
        if is_valid {
            Ok(BlockId(s.to_string()))
        } else {
            Err(IdError(s.to_string()))
        }
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The creation time encoded in this id's ULID (milliseconds since the Unix epoch), if the id
    /// is a decodable ULID. mkb-minted ids always are; a lenient non-ULID id yields `None`. This
    /// is why `created` is free — every block's birth time rides in its id, with nothing stored.
    pub fn created_ms(&self) -> Option<u64> {
        ulid::Ulid::from_string(&self.0)
            .ok()
            .map(|u| u.timestamp_ms())
    }

    /// The creation time as an RFC 3339 UTC string (decoded from the ULID), if decodable.
    pub fn created_rfc3339(&self) -> Option<String> {
        self.created_ms().and_then(crate::clock::unix_ms_to_rfc3339)
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A located id marker within some text: the id plus the byte range the marker occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerMatch {
    /// The id carried by the marker.
    pub id: BlockId,
    /// Start byte offset of the full marker in the source text.
    pub start: usize,
    /// End byte offset (exclusive) of the full marker in the source text.
    pub end: usize,
}

/// Reads and writes block-id markers in Markdown text.
///
/// Programming against this trait (rather than a concrete codec) is what keeps a future
/// Obsidian-compatible encoding a drop-in swap. See `AGENTS.md` (keep the seams clean).
pub trait IdCodec {
    /// Render an id into its on-disk marker form.
    fn encode(&self, id: &BlockId) -> String;

    /// Find every id marker in `text`, in source order.
    fn find_all(&self, text: &str) -> Vec<MarkerMatch>;

    /// Find the first id marker in `text`, if any.
    fn find_first(&self, text: &str) -> Option<MarkerMatch> {
        self.find_all(text).into_iter().next()
    }

    /// Remove all id markers from `text`, returning the cleaned string.
    ///
    /// Removal is literal: only the marker substrings are deleted, leaving surrounding
    /// text (including any adjacent whitespace) untouched.
    fn strip(&self, text: &str) -> String;
}

/// The native mkb codec: encodes ids as `<!-- mkb:<ulid> -->`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeIdCodec;

fn native_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"<!--\s*mkb:([0-9A-Za-z]{26})\s*-->").expect("valid native id regex")
    })
}

impl IdCodec for NativeIdCodec {
    fn encode(&self, id: &BlockId) -> String {
        format!("<!-- mkb:{} -->", id.as_str())
    }

    fn find_all(&self, text: &str) -> Vec<MarkerMatch> {
        native_re()
            .captures_iter(text)
            .filter_map(|caps| {
                let whole = caps.get(0)?;
                let id = BlockId::parse(caps.get(1)?.as_str()).ok()?;
                Some(MarkerMatch {
                    id,
                    start: whole.start(),
                    end: whole.end(),
                })
            })
            .collect()
    }

    fn strip(&self, text: &str) -> String {
        native_re().replace_all(text, "").into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_unique_26_char_ids() {
        let a = BlockId::generate();
        let b = BlockId::generate();
        assert_eq!(a.as_str().len(), 26);
        assert_eq!(b.as_str().len(), 26);
        assert_ne!(a, b, "two generated ids must differ");
    }

    #[test]
    fn parse_accepts_generated_ids() {
        let id = BlockId::generate();
        let reparsed = BlockId::parse(id.as_str()).expect("generated id must reparse");
        assert_eq!(id, reparsed);
    }

    #[test]
    fn parse_trims_surrounding_whitespace() {
        let id = BlockId::generate();
        let padded = format!("  {}  ", id.as_str());
        assert_eq!(BlockId::parse(&padded).unwrap(), id);
    }

    #[test]
    fn parse_rejects_malformed_ids() {
        assert!(BlockId::parse("too-short").is_err());
        assert!(BlockId::parse("").is_err());
        // 25 and 27 chars are both invalid (must be exactly 26).
        assert!(BlockId::parse(&"a".repeat(25)).is_err());
        assert!(BlockId::parse(&"a".repeat(27)).is_err());
        // Non-alphanumeric inside an otherwise-26-char token.
        assert!(BlockId::parse("01ARZ3NDEKTSV4RRFFQ69G5FA!").is_err());
    }

    #[test]
    fn encode_uses_invisible_html_comment() {
        let id = BlockId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        assert_eq!(
            NativeIdCodec.encode(&id),
            "<!-- mkb:01ARZ3NDEKTSV4RRFFQ69G5FAV -->"
        );
    }

    #[test]
    fn encode_then_find_round_trips() {
        let id = BlockId::generate();
        let line = format!("A note about nginx. {}", NativeIdCodec.encode(&id));
        let found = NativeIdCodec
            .find_first(&line)
            .expect("marker should be found");
        assert_eq!(found.id, id);
        assert_eq!(&line[found.start..found.end], NativeIdCodec.encode(&id));
    }

    #[test]
    fn find_all_returns_markers_in_order() {
        let first = BlockId::generate();
        let second = BlockId::generate();
        let text = format!(
            "Para one {}\n\nPara two {}",
            NativeIdCodec.encode(&first),
            NativeIdCodec.encode(&second)
        );
        let matches = NativeIdCodec.find_all(&text);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].id, first);
        assert_eq!(matches[1].id, second);
        assert!(matches[0].start < matches[1].start);
    }

    #[test]
    fn strip_removes_markers_only() {
        let id = BlockId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let text = format!("keep this {}", NativeIdCodec.encode(&id));
        assert_eq!(NativeIdCodec.strip(&text), "keep this ");
    }

    #[test]
    fn find_ignores_foreign_or_malformed_comments() {
        // Wrong namespace, wrong length, and a non-comment caret form must all be ignored.
        let text = "\
            <!-- note: not an id -->\n\
            <!-- mkb:tooshort -->\n\
            ^blk_01ARZ3NDEKTSV4RRFFQ69G5FAV\n";
        assert!(NativeIdCodec.find_all(text).is_empty());
    }
}
