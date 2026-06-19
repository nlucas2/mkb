//! Shared presentation layer for mdkb user interfaces.
//!
//! Every mdkb UI (the local web UI, a Tauri desktop shell, …) renders the *same* way by
//! using this crate: there is exactly one Markdown→HTML path and one page template, so the
//! views can never drift apart (see `AGENTS.md`). UIs supply already-transclusion-resolved
//! Markdown (from `mdkb_core::render_page` via the daemon); this crate turns it into HTML
//! and wraps it in a browsable document.

use mdkb_core::{IdCodec, NativeIdCodec};
use pulldown_cmark::{html, Event, Options, Parser};

/// HTML-escape a string for safe insertion into element text / attributes.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Convert Markdown (with mdkb id markers) into an HTML fragment.
///
/// The invisible `<!-- mdkb:… -->` id markers are stripped first so they never leak into
/// the rendered output. CommonMark plus tables/strikethrough/task-lists are enabled.
///
/// **Security:** raw HTML in the source is **not** passed through. Any inline/block HTML
/// event is downgraded to escaped text, so a note containing `<script>…</script>` (which an
/// AI agent could be induced to write via the MCP write tools) renders as inert text rather
/// than executing — this closes the stored-XSS vector.
pub fn markdown_to_html(markdown: &str) -> String {
    let cleaned = NativeIdCodec.strip(markdown);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(&cleaned, options).map(|event| match event {
        // Neutralise raw HTML: re-emit it as escaped text instead of live markup.
        Event::Html(h) => Event::Text(h),
        Event::InlineHtml(h) => Event::Text(h),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, parser);
    decorate_wiki(out)
}

/// Post-process rendered HTML to make mdkb wiki structure visible and stylable:
///
/// - `mdkb:` reference links become `<a class="wikilink" …>` chips (dangling ones also get
///   `unresolved`), so a UI can style and intercept navigation on them;
/// - the embed-card sentinel (`⧉` as the first content of a blockquote, emitted by
///   `mdkb_core::render`) tags that blockquote `class="mdkb-embed"`, so transclusions render
///   as framed "live mirror" cards rather than ordinary quotes.
///
/// This is a pure string pass keyed on markers the core renderer controls, so both the web
/// and desktop UIs get identical wiki styling from the one shared renderer.
fn decorate_wiki(html: String) -> String {
    html.replace(
        "<a href=\"mdkb:?unresolved\"",
        "<a class=\"wikilink unresolved\" href=\"mdkb:?unresolved\"",
    )
    .replace("<a href=\"mdkb:", "<a class=\"wikilink\" href=\"mdkb:")
    .replace(
        "<blockquote>\n<p>⧉",
        "<blockquote class=\"mdkb-embed\">\n<p>⧉",
    )
}

/// Rewrite the shared `mdkb:` link scheme onto a concrete navigation base for a client that
/// uses plain hyperlinks (e.g. the web UI's `/page/<path>` routes). `mdkb:<path>#<id>` becomes
/// `<base><path>#<id>`; the unresolved sentinel is left inert (`#`). Clients that intercept
/// clicks in JS (the desktop shell) can ignore this and parse `mdkb:` directly.
pub fn rewrite_mdkb_links(html: &str, base: &str) -> String {
    html.replace(
        "href=\"mdkb:?unresolved\"",
        "href=\"#\" aria-disabled=\"true\"",
    )
    .replace("href=\"mdkb:", &format!("href=\"{base}"))
}

/// Derive a human display title for a block from an optional title and a content snippet.
pub fn block_title(title: Option<&str>, content: &str) -> String {
    if let Some(t) = title {
        if !t.trim().is_empty() {
            return t.trim().to_string();
        }
    }
    for line in content.lines() {
        let t = line.trim().trim_start_matches('#').trim();
        if !t.is_empty() {
            return t.replace(['*', '_', '`'], "").chars().take(80).collect();
        }
    }
    "(untitled)".to_string()
}

/// A single search result row for display.
pub struct ResultRow {
    /// Block id.
    pub id: String,
    /// Block display title.
    pub title: String,
    /// Tag names (shown as chips).
    pub tags: Vec<String>,
    /// Block content (will be escaped, previewed).
    pub content: String,
}

/// Render search results as an HTML fragment.
pub fn search_results_html(query: &str, rows: &[ResultRow]) -> String {
    let mut out = format!(
        "<h1>Search</h1><p class=\"muted\">{} result(s) for <strong>{}</strong></p>",
        rows.len(),
        escape_html(query)
    );
    if rows.is_empty() {
        out.push_str("<p class=\"muted\">No matches.</p>");
        return out;
    }
    out.push_str("<ul class=\"results\">");
    for r in rows {
        let preview: String = r.content.replace('\n', " ").chars().take(160).collect();
        let crumb = if r.tags.is_empty() {
            String::new()
        } else {
            format!(
                "<span class=\"crumb\">{}</span>",
                escape_html(&r.tags.join(" · "))
            )
        };
        out.push_str(&format!(
            "<li><a href=\"/block/{}\">{}</a>{}<div class=\"preview\">{}</div></li>",
            escape_html(&r.id),
            escape_html(&r.title),
            crumb,
            escape_html(&preview)
        ));
    }
    out.push_str("</ul>");
    out
}

/// A sidebar entry: a block id + its display title.
pub struct NavEntry {
    /// Block id.
    pub id: String,
    /// Display title.
    pub title: String,
}

/// Wrap a body fragment in the full mdkb HTML document: a sidebar of blocks plus a search box
/// and the main content. `active` highlights the current block id (empty for none).
pub fn page_document(title: &str, body_html: &str, entries: &[NavEntry], active: &str) -> String {
    let mut nav = String::from(
        "<nav><form action=\"/search\" method=\"get\">\
        <input type=\"search\" name=\"q\" placeholder=\"Search…\" autofocus></form><ul>",
    );
    for e in entries {
        let cls = if e.id == active {
            " class=\"active\""
        } else {
            ""
        };
        nav.push_str(&format!(
            "<li{}><a href=\"/block/{}\">{}</a></li>",
            cls,
            escape_html(&e.id),
            escape_html(&e.title)
        ));
    }
    nav.push_str("</ul></nav>");

    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title><style>{css}</style></head>\
<body><div class=\"layout\">{nav}<main>{body}</main></div></body></html>",
        title = escape_html(title),
        css = STYLE,
        nav = nav,
        body = body_html,
    )
}

/// The single stylesheet shared by every mdkb HTML view.
pub const STYLE: &str = r#"
:root { --bg:#1e1e2e; --fg:#cdd6f4; --muted:#9399b2; --accent:#89b4fa; --panel:#181825; --border:#313244; }
* { box-sizing: border-box; }
body { margin:0; font: 15px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background:var(--bg); color:var(--fg); }
.layout { display:flex; min-height:100vh; }
nav { width:260px; flex:0 0 260px; background:var(--panel); border-right:1px solid var(--border); padding:1rem; overflow-y:auto; }
nav input[type=search] { width:100%; padding:.5rem .6rem; margin-bottom:1rem; background:var(--bg); border:1px solid var(--border); border-radius:6px; color:var(--fg); }
nav ul { list-style:none; margin:0; padding:0; }
nav li a { display:block; padding:.3rem .5rem; border-radius:6px; color:var(--fg); text-decoration:none; }
nav li a:hover { background:var(--border); }
nav li.active a { background:var(--accent); color:var(--panel); font-weight:600; }
main { flex:1; padding:2rem 3rem; max-width:60rem; }
main h1,h2,h3 { line-height:1.25; }
a { color:var(--accent); }
code { background:var(--panel); padding:.1rem .35rem; border-radius:4px; }
pre { background:var(--panel); border:1px solid var(--border); padding:1rem; border-radius:8px; overflow-x:auto; }
pre code { background:none; padding:0; }
blockquote { border-left:3px solid var(--accent); margin:0; padding-left:1rem; color:var(--muted); }
table { border-collapse:collapse; } th,td { border:1px solid var(--border); padding:.4rem .6rem; }
.muted { color:var(--muted); } .crumb { color:var(--muted); margin-left:.5rem; font-size:.85em; }
.results { list-style:none; padding:0; } .results li { padding:.6rem 0; border-bottom:1px solid var(--border); }
.preview { color:var(--muted); font-size:.9em; margin-top:.2rem; }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use mdkb_core::BlockId;

    #[test]
    fn markdown_renders_and_strips_ids() {
        let id = BlockId::generate();
        let md = format!(
            "# Title {}\n\nSome **bold** text.\n",
            NativeIdCodec.encode(&id)
        );
        let html = markdown_to_html(&md);
        assert!(html.contains("<h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        // The id marker must not appear in the output.
        assert!(!html.contains("mdkb:"));
        assert!(!html.contains(id.as_str()));
    }

    #[test]
    fn wiki_reference_becomes_chip_link() {
        // Mirrors what mdkb_core::render emits for a resolved `[[...]]` reference.
        let html = markdown_to_html("see [ideas](mdkb:ideas.md) now");
        assert!(
            html.contains("<a class=\"wikilink\" href=\"mdkb:ideas.md\">ideas</a>"),
            "got: {html}"
        );
    }

    #[test]
    fn unresolved_reference_is_marked() {
        let html = markdown_to_html("see [ghost](mdkb:?unresolved) now");
        assert!(
            html.contains("class=\"wikilink unresolved\""),
            "got: {html}"
        );
    }

    #[test]
    fn embed_card_blockquote_is_tagged() {
        // Mirrors mdkb_core::render's embed card: a blockquote whose first content is `⧉`.
        let html = markdown_to_html("> ⧉ [src](mdkb:src.md#01ABC)\n>\n> the body\n");
        assert!(
            html.contains("<blockquote class=\"mdkb-embed\">"),
            "got: {html}"
        );
        assert!(html.contains("the body"));
    }

    #[test]
    fn rewrite_mdkb_links_targets_web_routes() {
        let html = "<a class=\"wikilink\" href=\"mdkb:ideas.md#01ABC\">ideas</a>";
        let web = rewrite_mdkb_links(html, "/page/");
        assert!(web.contains("href=\"/page/ideas.md#01ABC\""), "got: {web}");
    }

    #[test]
    fn code_fence_language_becomes_class() {
        let html = markdown_to_html("```kusto\nStormEvents | take 10\n```\n");
        assert!(html.contains("language-kusto"));
        assert!(html.contains("StormEvents"));
    }

    #[test]
    fn raw_html_is_neutralised_not_executed() {
        // Stored-XSS guard: a script/img payload in note content must not survive as live
        // markup. It is escaped to inert text instead.
        let html = markdown_to_html("hello <script>alert('xss')</script> world\n");
        assert!(
            !html.contains("<script>"),
            "raw <script> must not pass through"
        );
        assert!(html.contains("&lt;script&gt;"));
        let img = markdown_to_html("<img src=x onerror=alert(1)>\n");
        assert!(!img.contains("<img"), "raw <img> must not pass through");
    }

    #[test]
    fn escape_html_neutralises_markup() {
        assert_eq!(
            escape_html("<script>&\"'"),
            "&lt;script&gt;&amp;&quot;&#39;"
        );
    }

    #[test]
    fn block_title_prefers_title_then_first_line() {
        assert_eq!(block_title(Some("Explicit"), "body"), "Explicit");
        assert_eq!(block_title(None, "# Heading\n\nbody"), "Heading");
        assert_eq!(block_title(Some("  "), "first line"), "first line");
    }

    #[test]
    fn document_includes_nav_and_active_highlight() {
        let entries = vec![
            NavEntry {
                id: "a".into(),
                title: "Alpha".into(),
            },
            NavEntry {
                id: "b".into(),
                title: "Beta".into(),
            },
        ];
        let doc = page_document("T", "<p>hi</p>", &entries, "b");
        assert!(doc.contains("<!DOCTYPE html>"));
        assert!(doc.contains("href=\"/block/a\""));
        assert!(doc.contains("class=\"active\""));
        assert!(doc.contains("<p>hi</p>"));
        assert!(doc.contains("action=\"/search\""));
    }

    #[test]
    fn search_results_render_links_and_escape() {
        let rows = vec![ResultRow {
            id: "x".into(),
            title: "Note".into(),
            tags: vec!["top".into()],
            content: "a <dangerous> line".into(),
        }];
        let html = search_results_html("q", &rows);
        assert!(html.contains("href=\"/block/x\""));
        assert!(html.contains("&lt;dangerous&gt;"));
        assert!(html.contains("top"));
    }

    #[test]
    fn empty_search_says_no_matches() {
        assert!(search_results_html("q", &[]).contains("No matches"));
    }
}
