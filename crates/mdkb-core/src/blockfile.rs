//! Parsing a single block file (`blocks/<ulid>.md`) into a [`Block`].
//!
//! A block file is **clean Markdown** with optional YAML frontmatter. mdkb only reads a few
//! frontmatter keys (`title:`, `tags:`); everything else is preserved untouched as part of
//! the body is *not* — frontmatter is stripped from `body`, but unknown keys are ignored, not
//! dropped from disk (writers re-emit only what they manage; see [`crate::sync`]). The body is
//! kept verbatim so the file round-trips. Block edges (`![[...]]` children, `[[...]]`
//! references) are derived lazily from the body via [`crate::link`].

use std::sync::OnceLock;

use regex::Regex;

use crate::block::Block;
use crate::id::BlockId;

/// Parse a block file's text into a [`Block`] with the given id (its filename stem).
pub fn parse_block(id: BlockId, source: &str) -> Block {
    let (frontmatter, body) = split_frontmatter(source);
    let (mut title, mut tags) = (None, Vec::new());
    if let Some(fm) = frontmatter {
        title = parse_title(fm);
        tags = parse_fm_tags(fm);
    }
    // The frontmatter tags are the managed set; inline #tags are prose mentions merged into the
    // searchable union but not managed by the tag editor.
    let fm_tags = tags.clone();
    // Inline #tags from the body, appended (deduped) after frontmatter tags.
    for t in inline_tags(body) {
        if !tags.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
            tags.push(t);
        }
    }
    let langs = code_langs(body);
    // A title is not required; default to None (display falls back to first line / id).
    if title.as_deref().map(str::trim).is_some_and(str::is_empty) {
        title = None;
    }
    Block {
        id,
        title,
        tags,
        fm_tags,
        langs,
        body: body.to_string(),
    }
}

/// Serialize a block's managed metadata + body back into file text. Frontmatter is emitted only
/// when there is metadata mdkb manages: a `title:` and/or `tags:`. Inline `#hashtag` mentions
/// live in the body and are not re-emitted as frontmatter (they round-trip as prose). `tags`
/// are the managed (frontmatter) tags; pass an empty slice for none.
pub fn write_block(title: Option<&str>, tags: &[String], body: &str) -> String {
    let title = title.map(str::trim).filter(|t| !t.is_empty());
    let clean_tags: Vec<&str> = tags
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();

    if title.is_none() && clean_tags.is_empty() {
        return body.to_string();
    }

    let mut fm = String::from("---\n");
    if let Some(t) = title {
        fm.push_str(&format!("title: {t}\n"));
    }
    if !clean_tags.is_empty() {
        fm.push_str(&format!("tags: [{}]\n", clean_tags.join(", ")));
    }
    fm.push_str("---\n\n");
    fm.push_str(body.trim_start_matches('\n'));
    fm
}

/// Split leading YAML frontmatter (`---` … `---`) from the body. Returns `(frontmatter, body)`
/// where `frontmatter` is the inner YAML text (without the fences) and `body` is everything
/// after the closing fence. If there is no well-formed frontmatter, returns `(None, source)`.
fn split_frontmatter(source: &str) -> (Option<&str>, &str) {
    let s = source.strip_prefix('\u{feff}').unwrap_or(source);
    if !(s.starts_with("---\n") || s.starts_with("---\r\n")) {
        return (None, source);
    }
    let after_open = s.find('\n').map(|i| i + 1).unwrap_or(s.len());
    let rest = &s[after_open..];
    // Find a line that is exactly `---` marking the close.
    let mut offset = after_open;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            let fm = &s[after_open..offset];
            let body_start = offset + line.len();
            let body = &s[body_start.min(s.len())..];
            let body = body
                .strip_prefix('\n')
                .or_else(|| body.strip_prefix("\r\n"))
                .unwrap_or(body);
            return (Some(fm), body);
        }
        offset += line.len();
    }
    (None, source)
}

fn parse_title(fm: &str) -> Option<String> {
    for line in fm.lines() {
        if let Some(rest) = line.trim().strip_prefix("title:") {
            let v = rest.trim().trim_matches(['"', '\'']).trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Parse `tags:` from frontmatter, supporting both flow (`tags: [a, b]`) and block list form
/// (`tags:` then `- a` lines).
fn parse_fm_tags(fm: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let lines: Vec<&str> = fm.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(rest) = line.trim().strip_prefix("tags:") {
            let rest = rest.trim();
            if rest.starts_with('[') {
                // flow: [a, b, c]
                let inner = rest.trim_start_matches('[').trim_end_matches(']');
                for t in inner.split(',') {
                    let t = t.trim().trim_matches(['"', '\'']).trim();
                    if !t.is_empty() {
                        tags.push(t.to_string());
                    }
                }
            } else if rest.is_empty() {
                // block list on following `- item` lines
                let mut j = i + 1;
                while j < lines.len() {
                    let l = lines[j].trim();
                    if let Some(item) = l.strip_prefix('-') {
                        let t = item.trim().trim_matches(['"', '\'']).trim();
                        if !t.is_empty() {
                            tags.push(t.to_string());
                        }
                        j += 1;
                    } else {
                        break;
                    }
                }
                i = j;
                continue;
            } else {
                // single scalar: tags: foo
                let t = rest.trim_matches(['"', '\'']).trim();
                if !t.is_empty() {
                    tags.push(t.to_string());
                }
            }
        }
        i += 1;
    }
    tags
}

fn inline_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|[^\w&/])#([A-Za-z][\w/-]*)").expect("inline tag re"))
}

/// Inline `#tag`s in the body, excluding fenced code blocks (so shell comments aren't tags).
fn inline_tags(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in non_code_lines(body) {
        for caps in inline_tag_re().captures_iter(line) {
            if let Some(m) = caps.get(1) {
                let t = m.as_str().to_string();
                if !out.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                    out.push(t);
                }
            }
        }
    }
    out
}

/// Fenced-code-block languages, in order of appearance.
fn code_langs(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("```").or_else(|| t.strip_prefix("~~~")) {
            if !in_fence {
                let lang = rest.split_whitespace().next().unwrap_or("").to_string();
                if !lang.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&lang)) {
                    out.push(lang);
                }
                in_fence = true;
            } else {
                in_fence = false;
            }
        }
    }
    out
}

/// Iterate the body's lines that are NOT inside a fenced code block.
fn non_code_lines(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(src: &str) -> Block {
        parse_block(BlockId::generate(), src)
    }

    #[test]
    fn parses_frontmatter_title_and_tags() {
        let b = p("---\ntitle: Deploying to k3s\ntags: [k3s, ops]\n---\n\nbody here\n");
        assert_eq!(b.title.as_deref(), Some("Deploying to k3s"));
        assert!(b.tags.contains(&"k3s".to_string()));
        assert!(b.tags.contains(&"ops".to_string()));
        assert_eq!(b.body, "body here\n");
    }

    #[test]
    fn parses_block_list_tags() {
        let b = p("---\ntags:\n  - alpha\n  - beta\n---\nbody\n");
        assert_eq!(b.tags, vec!["alpha", "beta"]);
    }

    #[test]
    fn no_frontmatter_keeps_body_verbatim() {
        let b = p("# Just markdown\n\nno frontmatter\n");
        assert_eq!(b.title, None);
        assert_eq!(b.body, "# Just markdown\n\nno frontmatter\n");
    }

    #[test]
    fn inline_tags_picked_up_outside_code() {
        let b = p("a #real tag\n\n```sh\n# not-a-tag shell comment\n```\n");
        assert!(b.tags.contains(&"real".to_string()));
        assert!(!b.tags.iter().any(|t| t == "not-a-tag"));
    }

    #[test]
    fn captures_code_langs() {
        let b = p("```kusto\nStormEvents\n```\n\n```sh\nls\n```\n");
        assert_eq!(b.langs, vec!["kusto", "sh"]);
    }

    #[test]
    fn write_block_round_trips_through_parse() {
        let text = write_block(Some("My Title"), &[], "the body\n");
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.title.as_deref(), Some("My Title"));
        assert_eq!(b.body, "the body\n");
    }

    #[test]
    fn write_block_without_title_or_tags_is_pure_body() {
        assert_eq!(write_block(None, &[], "hello\n"), "hello\n");
    }

    #[test]
    fn write_block_round_trips_frontmatter_tags() {
        let tags = vec!["k8s".to_string(), "ops".to_string()];
        let text = write_block(Some("Deploy"), &tags, "body\n");
        assert!(text.contains("tags: [k8s, ops]"));
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.fm_tags, vec!["k8s", "ops"]);
        assert_eq!(b.tags, vec!["k8s", "ops"]);
        assert_eq!(b.title.as_deref(), Some("Deploy"));
    }

    #[test]
    fn write_block_emits_tags_without_title() {
        let tags = vec!["solo".to_string()];
        let text = write_block(None, &tags, "body\n");
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.fm_tags, vec!["solo"]);
        assert_eq!(b.title, None);
    }
}
