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
    let mut locked = false;
    let mut props = Vec::new();
    if let Some(fm) = frontmatter {
        title = parse_title(fm);
        tags = parse_fm_tags(fm);
        locked = parse_locked(fm);
        props = parse_fm_props(fm);
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
        locked,
        props,
        body: body.to_string(),
    }
}

/// Serialize a block's managed metadata + body back into file text. Frontmatter is emitted only
/// when there is metadata mdkb manages: a `title:`, `tags:`, the `locked:` flag, and/or one or
/// more block `props`. Inline `#hashtag` mentions live in the body and are not re-emitted as
/// frontmatter (they round-trip as prose). `tags` are the managed (frontmatter) tags; pass an
/// empty slice for none. `locked` emits `locked: true` (and is omitted when false, so unlocked
/// blocks stay clean). `props` are arbitrary scalar `key: value` pairs emitted in order after the
/// managed keys; this is what makes open-ended metadata round-trip instead of being dropped on a
/// rewrite. Empty/blank keys are skipped.
pub fn write_block(
    title: Option<&str>,
    tags: &[String],
    locked: bool,
    props: &[(String, String)],
    body: &str,
) -> String {
    let title = title.map(str::trim).filter(|t| !t.is_empty());
    let clean_tags: Vec<&str> = tags
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();
    let clean_props: Vec<(&str, &str)> = props
        .iter()
        .map(|(k, v)| (k.trim(), v.as_str()))
        // Only well-formed, non-managed keys are emitted. A malformed key (newline, `:`,
        // whitespace) would inject extra frontmatter lines; a managed key (`title`/`tags`/`locked`)
        // would collide with mdkb's own metadata — e.g. a smuggled `locked: true`, the human-only
        // access-control flag agents must not set. This makes serialization safe by construction;
        // `set_props` rejects such keys up front so this filter never silently drops valid data.
        .filter(|(k, _)| is_prop_key(k) && !is_managed_key(k))
        .collect();

    if title.is_none() && clean_tags.is_empty() && !locked && clean_props.is_empty() {
        return body.to_string();
    }

    let mut fm = String::from("---\n");
    if let Some(t) = title {
        fm.push_str(&format!("title: {}\n", yaml_scalar(t)));
    }
    if !clean_tags.is_empty() {
        fm.push_str(&format!("tags: [{}]\n", clean_tags.join(", ")));
    }
    if locked {
        fm.push_str("locked: true\n");
    }
    for (k, v) in clean_props {
        fm.push_str(&format!("{}: {}\n", k, yaml_scalar(v)));
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
            let v = parse_scalar(rest);
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Parse the `locked:` frontmatter flag. Truthy values (`true`/`yes`/`on`/`1`, case-insensitive)
/// mark the block human-only; anything else (or absent) is unlocked.
fn parse_locked(fm: &str) -> bool {
    for line in fm.lines() {
        if let Some(rest) = line.trim().strip_prefix("locked:") {
            let v = parse_scalar(rest).to_ascii_lowercase();
            return matches!(v.as_str(), "true" | "yes" | "on" | "1");
        }
    }
    false
}

/// Serialize a string as a YAML scalar that survives a standard YAML parser: a plain scalar
/// when unambiguous, otherwise a double-quoted scalar with `\`, `"`, newlines and tabs escaped.
/// This keeps frontmatter valid YAML even for titles containing `:`, `#`, or quotes — honoring
/// the SPEC promise that a raw vault is recoverable with or without mdkb.
fn yaml_scalar(s: &str) -> String {
    if is_plain_yaml_safe(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Whether `s` can be written as a YAML plain (unquoted) scalar without ambiguity.
fn is_plain_yaml_safe(s: &str) -> bool {
    if s.is_empty() || s != s.trim() {
        return false;
    }
    let first = s.chars().next().expect("non-empty");
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) {
        return false;
    }
    if s.contains(": ") || s.ends_with(':') || s.contains(" #") {
        return false;
    }
    !s.chars().any(char::is_control)
}

/// Parse a YAML scalar value as our writer produces it: a double-quoted string (with `\\`,
/// `\"`, `\n`, `\t` escapes), a single-quoted string (`''` → `'`), or a plain scalar.
fn parse_scalar(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        out
    } else if let Some(inner) = raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        inner.replace("''", "'")
    } else {
        raw.to_string()
    }
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

/// The frontmatter keys mdkb manages directly; everything else becomes a block property.
const MANAGED_KEYS: [&str; 3] = ["title", "tags", "locked"];

/// Whether `key` (case-insensitively) is one mdkb manages itself (`title`/`tags`/`locked`). Such
/// keys are never block properties: they round-trip through their own typed parsers/writers, and
/// `set_props` rejects them so an agent can't, say, smuggle `locked: true` via a property.
pub(crate) fn is_managed_key(key: &str) -> bool {
    MANAGED_KEYS.iter().any(|m| key.eq_ignore_ascii_case(m))
}

/// Parse arbitrary block **properties** from frontmatter: every top-level `key: value` line whose
/// key isn't one mdkb manages (`title`/`tags`/`locked`). Only simple scalar values are captured
/// (the open-ended-metadata primitive is intentionally flat); nested maps or empty-valued keys
/// are skipped. Keys must look like identifiers (`source`, `verified-on`) so prose/structure in
/// frontmatter isn't mistaken for a property. Duplicate keys keep the first occurrence.
fn parse_fm_props(fm: &str) -> Vec<(String, String)> {
    let mut props: Vec<(String, String)> = Vec::new();
    for line in fm.lines() {
        // Top-level only: indented lines are list items / nested maps, not properties.
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        let key = line[..colon].trim();
        if !is_prop_key(key) || is_managed_key(key) {
            continue;
        }
        let value = parse_scalar(&line[colon + 1..]);
        if value.is_empty() {
            continue;
        }
        if props.iter().any(|(k, _)| k.eq_ignore_ascii_case(key)) {
            continue;
        }
        props.push((key.to_string(), value));
    }
    props
}

/// Whether `key` is a well-formed property key: a leading ASCII letter, then letters, digits,
/// `_`, or `-`. Keeps property parsing from swallowing arbitrary `text: like this` prose, and —
/// because the same grammar gates the write path — guarantees a key can never carry a newline,
/// `:`, or whitespace that would inject extra frontmatter lines. Parse and write are symmetric:
/// every key mdkb will write, it will read back identically.
pub(crate) fn is_prop_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
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
        let text = write_block(Some("My Title"), &[], false, &[], "the body\n");
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.title.as_deref(), Some("My Title"));
        assert_eq!(b.body, "the body\n");
    }

    #[test]
    fn write_block_without_title_or_tags_is_pure_body() {
        assert_eq!(write_block(None, &[], false, &[], "hello\n"), "hello\n");
    }

    #[test]
    fn write_block_round_trips_frontmatter_tags() {
        let tags = vec!["k8s".to_string(), "ops".to_string()];
        let text = write_block(Some("Deploy"), &tags, false, &[], "body\n");
        assert!(text.contains("tags: [k8s, ops]"));
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.fm_tags, vec!["k8s", "ops"]);
        assert_eq!(b.tags, vec!["k8s", "ops"]);
        assert_eq!(b.title.as_deref(), Some("Deploy"));
    }

    #[test]
    fn write_block_emits_tags_without_title() {
        let tags = vec!["solo".to_string()];
        let text = write_block(None, &tags, false, &[], "body\n");
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.fm_tags, vec!["solo"]);
        assert_eq!(b.title, None);
    }

    #[test]
    fn write_block_quotes_title_with_colon() {
        // A title containing `: ` would be invalid YAML unquoted; it must be double-quoted.
        let text = write_block(Some("SPEC: A block file"), &[], false, &[], "body\n");
        assert!(
            text.contains("title: \"SPEC: A block file\"\n"),
            "title should be quoted: {text}"
        );
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.title.as_deref(), Some("SPEC: A block file"));
    }

    #[test]
    fn write_block_leaves_plain_title_unquoted() {
        // Common titles stay plain so existing vault files don't churn.
        let text = write_block(Some("Deploying to k3s"), &[], false, &[], "body\n");
        assert!(text.contains("title: Deploying to k3s\n"), "{text}");
    }

    #[test]
    fn write_block_round_trips_title_with_quotes_and_backslash() {
        let title = "He said \"hi\" \\ bye";
        let text = write_block(Some(title), &[], false, &[], "body\n");
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.title.as_deref(), Some(title));
    }

    #[test]
    fn parse_title_unescapes_double_quoted() {
        let b = p("---\ntitle: \"a: b \\\"c\\\"\"\n---\n\nbody\n");
        assert_eq!(b.title.as_deref(), Some("a: b \"c\""));
    }

    #[test]
    fn parses_locked_flag() {
        let b = p("---\ntitle: Pinned\nlocked: true\n---\n\nbody\n");
        assert!(b.locked);
        let unlocked = p("---\ntitle: Free\n---\n\nbody\n");
        assert!(!unlocked.locked);
        // No frontmatter at all → unlocked.
        assert!(!p("# plain\n").locked);
    }

    #[test]
    fn write_block_round_trips_locked() {
        let text = write_block(Some("Pinned"), &[], true, &[], "body\n");
        assert!(text.contains("locked: true\n"), "{text}");
        let b = parse_block(BlockId::generate(), &text);
        assert!(b.locked);
        assert_eq!(b.title.as_deref(), Some("Pinned"));
    }

    #[test]
    fn write_block_emits_locked_without_title_or_tags() {
        // A locked block must still get frontmatter even with no title/tags.
        let text = write_block(None, &[], true, &[], "just body\n");
        assert!(text.starts_with("---\nlocked: true\n---\n"), "{text}");
        assert!(parse_block(BlockId::generate(), &text).locked);
    }

    #[test]
    fn write_block_omits_locked_when_false() {
        let text = write_block(Some("Free"), &[], false, &[], "body\n");
        assert!(!text.contains("locked"), "{text}");
    }

    #[test]
    fn parses_arbitrary_props_skipping_managed_keys() {
        let b = p("---\ntitle: Atom\ntags: [mem]\nlocked: true\nsource: https://example.com/x\nverified: 2026-06-01\nconfidence: 0.8\n---\n\nbody\n");
        assert_eq!(b.title.as_deref(), Some("Atom"));
        assert_eq!(b.fm_tags, vec!["mem"]);
        assert!(b.locked);
        assert_eq!(
            b.props,
            vec![
                ("source".to_string(), "https://example.com/x".to_string()),
                ("verified".to_string(), "2026-06-01".to_string()),
                ("confidence".to_string(), "0.8".to_string()),
            ]
        );
    }

    #[test]
    fn props_ignore_indented_lines_and_tag_list_items() {
        // Block-list tags + an indented line must not be captured as properties.
        let b = p("---\ntags:\n  - alpha\n  - beta\nsource: git\n---\nbody\n");
        assert_eq!(b.tags, vec!["alpha", "beta"]);
        assert_eq!(b.props, vec![("source".to_string(), "git".to_string())]);
    }

    #[test]
    fn write_block_round_trips_props() {
        let props = vec![
            ("source".to_string(), "https://example.com/x".to_string()),
            ("verified".to_string(), "2026-06-01".to_string()),
        ];
        let text = write_block(Some("Atom"), &[], false, &props, "body\n");
        assert!(text.contains("source: https://example.com/x\n"), "{text}");
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.props, props);
    }

    #[test]
    fn write_block_quotes_prop_value_with_colon() {
        // A value containing `: ` would be invalid YAML unquoted; it must be quoted and survive.
        let props = vec![("note".to_string(), "ratio: 2:1 high".to_string())];
        let text = write_block(None, &[], false, &props, "body\n");
        let b = parse_block(BlockId::generate(), &text);
        assert_eq!(b.prop("note"), Some("ratio: 2:1 high"));
    }

    #[test]
    fn write_block_emits_props_without_title_tags_or_lock() {
        let props = vec![("source".to_string(), "git".to_string())];
        let text = write_block(None, &[], false, &props, "just body\n");
        assert!(text.starts_with("---\nsource: git\n---\n"), "{text}");
        assert_eq!(
            parse_block(BlockId::generate(), &text).prop("source"),
            Some("git")
        );
    }

    #[test]
    fn write_block_drops_injection_keys() {
        // A key carrying a newline + a smuggled `locked: true` must NOT reach the frontmatter.
        let props = vec![
            ("evil\nlocked".to_string(), "true".to_string()),
            ("ok".to_string(), "1".to_string()),
        ];
        let text = write_block(Some("V"), &[], false, &props, "body\n");
        assert!(!text.contains("locked"), "injection leaked:\n{text}");
        let b = parse_block(BlockId::generate(), &text);
        assert!(!b.locked, "block must not be locked via injection");
        assert_eq!(b.prop("ok"), Some("1"));
        assert_eq!(b.props.len(), 1);
    }
}
