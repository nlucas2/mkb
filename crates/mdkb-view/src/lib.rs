//! Shared presentation layer for mdkb user interfaces.
//!
//! Every mdkb UI (the local web UI, a Tauri desktop shell, …) renders the *same* way by
//! using this crate: there is exactly one Markdown→HTML path and one page template, so the
//! views can never drift apart (see `AGENTS.md`). UIs supply already-transclusion-resolved
//! Markdown (from `mdkb_core::render_page` via the daemon); this crate turns it into HTML
//! and wraps it in a browsable document.

use mdkb_core::{IdCodec, NativeIdCodec};
use pulldown_cmark::{html, Options, Parser};

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
pub fn markdown_to_html(markdown: &str) -> String {
    let cleaned = NativeIdCodec.strip(markdown);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(&cleaned, options);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Derive a human page title from a vault-relative path (file stem, dashes→spaces).
pub fn page_title(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.strip_suffix(".md").unwrap_or(file);
    stem.replace(['-', '_'], " ")
}

/// A single search result row for display.
pub struct ResultRow {
    /// Page path the block lives on.
    pub page_path: String,
    /// Block id.
    pub id: String,
    /// Lineage breadcrumb (heading path).
    pub lineage: Vec<String>,
    /// Raw block content (will be escaped).
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
        let crumb = if r.lineage.is_empty() {
            String::new()
        } else {
            format!(
                "<span class=\"crumb\">{}</span>",
                escape_html(&r.lineage.join(" › "))
            )
        };
        out.push_str(&format!(
            "<li><a href=\"/page/{}\">{}</a>{}<div class=\"preview\">{}</div></li>",
            escape_html(&r.page_path),
            escape_html(&page_title(&r.page_path)),
            crumb,
            escape_html(&preview)
        ));
    }
    out.push_str("</ul>");
    out
}

/// Wrap a body fragment in the full mdkb HTML document: a sidebar of pages plus a search
/// box and the main content. `active` highlights the current page (empty for none).
pub fn page_document(title: &str, body_html: &str, pages: &[String], active: &str) -> String {
    let mut nav = String::from(
        "<nav><form action=\"/search\" method=\"get\">\
        <input type=\"search\" name=\"q\" placeholder=\"Search…\" autofocus></form><ul>",
    );
    for p in pages {
        let cls = if p == active { " class=\"active\"" } else { "" };
        nav.push_str(&format!(
            "<li{}><a href=\"/page/{}\">{}</a></li>",
            cls,
            escape_html(p),
            escape_html(&page_title(p))
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
    fn code_fence_language_becomes_class() {
        let html = markdown_to_html("```kusto\nStormEvents | take 10\n```\n");
        assert!(html.contains("language-kusto"));
        assert!(html.contains("StormEvents"));
    }

    #[test]
    fn escape_html_neutralises_markup() {
        assert_eq!(
            escape_html("<script>&\"'"),
            "&lt;script&gt;&amp;&quot;&#39;"
        );
    }

    #[test]
    fn page_title_humanises_path() {
        assert_eq!(page_title("topic/useful-queries.md"), "useful queries");
        assert_eq!(page_title("notes/arch_design.md"), "arch design");
    }

    #[test]
    fn document_includes_nav_and_active_highlight() {
        let pages = vec!["a.md".to_string(), "b.md".to_string()];
        let doc = page_document("T", "<p>hi</p>", &pages, "b.md");
        assert!(doc.contains("<!DOCTYPE html>"));
        assert!(doc.contains("href=\"/page/a.md\""));
        assert!(doc.contains("class=\"active\""));
        assert!(doc.contains("<p>hi</p>"));
        assert!(doc.contains("action=\"/search\""));
    }

    #[test]
    fn search_results_render_links_and_escape() {
        let rows = vec![ResultRow {
            page_path: "n.md".into(),
            id: "x".into(),
            lineage: vec!["Top".into()],
            content: "a <dangerous> line".into(),
        }];
        let html = search_results_html("q", &rows);
        assert!(html.contains("href=\"/page/n.md\""));
        assert!(html.contains("&lt;dangerous&gt;"));
        assert!(html.contains("Top"));
    }

    #[test]
    fn empty_search_says_no_matches() {
        assert!(search_results_html("q", &[]).contains("No matches"));
    }
}
