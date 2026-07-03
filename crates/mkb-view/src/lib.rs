//! Shared presentation layer for mkb user interfaces.
//!
//! Every mkb UI (the Tauri desktop shell, and any future renderer) renders the *same* way by
//! using this crate: there is exactly one Markdown→HTML path and one page template, so the
//! views can never drift apart (see `AGENTS.md`). UIs supply already-transclusion-resolved
//! Markdown (from `mkb_core::render_page` via the daemon); this crate turns it into HTML
//! and wraps it in a browsable document.

use mkb_core::{IdCodec, NativeIdCodec};
use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};
use std::path::{Path, PathBuf};

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

/// Render Markdown (with mkb id markers) into an HTML fragment for a UI: resolves **vault-local
/// image sources** so they display, and renders **external image sources inert** (never fetched).
///
/// The invisible `<!-- mkb:… -->` id markers are stripped first so they never leak into the
/// rendered output. CommonMark plus tables/strikethrough/task-lists are enabled.
///
/// - A **vault-relative** source (e.g. `![](assets/diagram.png)`) is resolved, when `vault_root`
///   is `Some`, to an absolute path under the vault and emitted as an `mkb-asset:<abs>` URL; the
///   client maps that sentinel to its own asset scheme (the desktop app uses `convertFileSrc`).
/// - An **external** source — anything with a URL scheme (`https:`, `data:`, …) or a
///   protocol-relative `//host` — is replaced with an **inert placeholder** that makes no network
///   request. Blocks are AI-writable, so this prevents a planted remote image from acting as a
///   tracking/exfiltration pixel the moment a human opens the block.
///
/// `vault_root` is `None` for a remote vault (no local files to serve); external images are still
/// blocked. **Security:** raw HTML in the source is **not** passed through — any inline/block HTML
/// event is downgraded to escaped text (so an AI-written `<script>…</script>` renders inert),
/// closing the stored-XSS vector; this holds regardless of `vault_root`.
pub fn markdown_to_html_with_assets(markdown: &str, vault_root: Option<&Path>) -> String {
    render_markdown(markdown, asset_classifier(vault_root), false)
}

/// Like [`markdown_to_html_with_assets`], but stamps each **top-level** rendered element with a
/// sequential `data-bi="N"` (block index) attribute. Paired with [`top_level_block_spans`] over the
/// block's **raw** body — the Nth stamped element corresponds to the Nth raw top-level block — this
/// lets a UI map a rendered-content selection back to source byte offsets (for whole-block carve)
/// without reversing HTML. Desktop-app only; the plain renderers stay attribute-free.
pub fn markdown_to_html_with_assets_indexed(markdown: &str, vault_root: Option<&Path>) -> String {
    render_markdown(markdown, asset_classifier(vault_root), true)
}

/// The image classifier shared by the plain and indexed asset renderers: resolve vault-relative
/// images, render external ones inert.
fn asset_classifier(vault_root: Option<&Path>) -> impl Fn(&str) -> ImageAction + '_ {
    move |dest: &str| {
        if let Some(root) = vault_root {
            if let Some(abs) = vault_asset_path(dest, root) {
                return ImageAction::Rewrite(format!("mkb-asset:{}", asset_url_path(&abs)));
            }
        }
        if is_external_image(dest) {
            ImageAction::Inert(dest.to_string())
        } else {
            ImageAction::Keep
        }
    }
}

/// The source byte spans of every **top-level** block in `md` (paragraphs, headings, lists,
/// blockquotes, code blocks, tables — not standalone rules), in document order. Computed with
/// pulldown's offset iterator, so each span is the exact `md[start..end]` of that block. Used with
/// the raw block body so a UI can carve whole top-level blocks by their source offsets.
pub fn top_level_block_spans(md: &str) -> Vec<(usize, usize)> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let mut spans = Vec::new();
    let mut depth = 0i32;
    for (event, range) in Parser::new_ext(md, opts).into_offset_iter() {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    spans.push((range.start, range.end));
                }
                depth += 1;
            }
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }
    spans
}

/// Resolve a Markdown image source to an absolute path inside `vault_root`, or `None` if the
/// source is an external URL (has a scheme, is protocol-relative `//…`, or a fragment) that a UI
/// should not treat as a vault file. A leading `./` or `/` is treated as vault-relative so a path
/// can never escape the vault by being "absolute"; `..` segments are dropped for the same reason
/// (the desktop app additionally confines loads to the vault via the asset-protocol scope).
pub fn vault_asset_path(dest: &str, vault_root: &Path) -> Option<PathBuf> {
    if dest.is_empty() || dest.starts_with('#') || dest.starts_with("//") || has_url_scheme(dest) {
        return None;
    }
    let mut path = vault_root.to_path_buf();
    for seg in dest.split('/') {
        match seg {
            "" | "." | ".." => continue,
            s => path.push(s),
        }
    }
    (path != vault_root).then_some(path)
}

/// Render an absolute vault path as the payload of an `mkb-asset:` URL. Asset URLs always use `/`
/// regardless of the host's path separator: on Windows [`Path`] joins with `\`, which the Markdown
/// renderer would percent-encode to `%5C`, producing a sentinel the desktop front-end can't map
/// back to a real file. Forcing forward slashes keeps the sentinel format identical on every OS
/// (Windows accepts `/` in file paths, and `convertFileSrc` handles it).
fn asset_url_path(abs: &Path) -> String {
    abs.to_string_lossy().replace('\\', "/")
}

/// Whether an image source points outside the vault (and so must never be auto-fetched): it has a
/// URL scheme (`https:`, `http:`, `data:`, …) or is protocol-relative (`//host/…`). Empty sources
/// and bare fragments are not "external" — they are left as-is.
fn is_external_image(dest: &str) -> bool {
    dest.starts_with("//") || has_url_scheme(dest)
}

/// Whether `s` begins with a URL scheme like `https:` or `data:` (RFC 3986: an ASCII letter
/// followed by letters/digits/`+`/`-`/`.`, then `:`). Windows drive letters (`C:\…`) are not
/// vault-relative image sources, so treating them as "external" (left as-is) is correct here.
fn has_url_scheme(s: &str) -> bool {
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for (i, c) in chars {
        match c {
            ':' => return i > 0,
            c if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') => {}
            _ => return false,
        }
    }
    false
}

/// What to do with a Markdown image, decided per-source by the caller's classifier.
enum ImageAction {
    /// Replace the image source with this URL (e.g. a vault asset URL).
    Rewrite(String),
    /// Drop the `<img>` and render an inert, non-fetching placeholder for this (external) source.
    Inert(String),
    /// Leave the image unchanged.
    Keep,
}

/// Build the inert placeholder shown in place of an external image (no network request is made).
/// The original URL appears only in a hover `title`; the alt text labels it.
fn external_image_placeholder(url: &str, alt: &str) -> String {
    let label = alt.trim();
    let label = if label.is_empty() {
        "external image"
    } else {
        label
    };
    format!(
        "<span class=\"mkb-extern-img\" title=\"external image not loaded: {url}\">\u{1f6ab} {label} (external image, not loaded)</span>",
        url = escape_html(url),
        label = escape_html(label),
    )
}

/// Render Markdown to an HTML fragment, applying `classify` to every image source. Shared by
/// [`markdown_to_html_with_assets`] and its indexed variant so both render identically apart from
/// block-index stamping. Raw HTML is neutralised (escaped) to close the stored-XSS vector; an image
/// the classifier marks [`ImageAction::Inert`] is replaced by a non-fetching placeholder (its inner
/// alt-text events are folded into the placeholder rather than rendered as an `<img>` alt).
fn render_markdown(markdown: &str, classify: impl Fn(&str) -> ImageAction, stamp: bool) -> String {
    let cleaned = NativeIdCodec.strip(markdown);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let mut events: Vec<Event> = Vec::new();
    // While `Some`, we are between the Start and End of an inert (external) image, accumulating its
    // alt text `(url, alt)`; its inner events are swallowed rather than emitted as an `<img>`.
    let mut inert: Option<(String, String)> = None;
    for event in Parser::new_ext(&cleaned, options) {
        match event {
            // Neutralise raw HTML: re-emit it as escaped text instead of live markup.
            Event::Html(h) => events.push(Event::Text(h)),
            Event::InlineHtml(h) => events.push(Event::Text(h)),
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => match classify(&dest_url) {
                ImageAction::Rewrite(url) => events.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url: url.into(),
                    title,
                    id,
                })),
                ImageAction::Keep => events.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url,
                    title,
                    id,
                })),
                ImageAction::Inert(url) => inert = Some((url, String::new())),
            },
            Event::End(TagEnd::Image) => match inert.take() {
                Some((url, alt)) => {
                    events.push(Event::Html(external_image_placeholder(&url, &alt).into()))
                }
                None => events.push(Event::End(TagEnd::Image)),
            },
            Event::Text(t) | Event::Code(t) if inert.is_some() => {
                inert.as_mut().unwrap().1.push_str(&t);
            }
            // Any other inner event of an inert image (e.g. emphasis in the alt) is dropped.
            other if inert.is_some() => {
                let _ = other;
            }
            other => events.push(other),
        }
    }
    if stamp {
        render_events_indexed(events)
    } else {
        let mut out = String::new();
        html::push_html(&mut out, events.into_iter());
        decorate_wiki(out)
    }
}

/// Render an event stream, emitting each **top-level** element as its own decorated fragment with a
/// sequential `data-bi="N"` attribute on its opening tag. Standalone top-level events (e.g. a
/// thematic-break rule, or leftover text) render without an index — matching
/// [`top_level_block_spans`], which likewise counts only proper block elements — so the Nth stamped
/// element aligns with the Nth raw top-level block.
fn render_events_indexed(events: Vec<Event>) -> String {
    let mut out = String::new();
    let mut buf: Vec<Event> = Vec::new();
    let mut depth = 0i32;
    let mut bi = 0usize;
    for ev in events {
        match &ev {
            Event::Start(_) => {
                depth += 1;
                buf.push(ev);
            }
            Event::End(_) => {
                depth -= 1;
                buf.push(ev);
                if depth == 0 {
                    let mut frag = String::new();
                    html::push_html(&mut frag, buf.drain(..));
                    out.push_str(&inject_block_index(&decorate_wiki(frag), bi));
                    bi += 1;
                }
            }
            _ => {
                if depth == 0 {
                    // A standalone top-level event (rule, stray text): render as-is, no index.
                    let mut frag = String::new();
                    html::push_html(&mut frag, std::iter::once(ev));
                    out.push_str(&decorate_wiki(frag));
                } else {
                    buf.push(ev);
                }
            }
        }
    }
    out
}

/// Insert ` data-bi="N"` immediately after the tag name of the first tag in `frag` (the element's
/// opening tag). Robust across `<p>`, `<h2>`, `<ul>`, `<blockquote class="mkb-embed">`, `<pre>`,
/// `<hr />`, etc. — the name ends at the first whitespace, `>`, or `/`.
fn inject_block_index(frag: &str, bi: usize) -> String {
    let Some(lt) = frag.find('<') else {
        return frag.to_string();
    };
    let after = &frag[lt + 1..];
    let name_len = after
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(after.len());
    let at = lt + 1 + name_len;
    let mut s = String::with_capacity(frag.len() + 16);
    s.push_str(&frag[..at]);
    s.push_str(&format!(" data-bi=\"{bi}\""));
    s.push_str(&frag[at..]);
    s
}

/// Post-process rendered HTML to make mkb wiki structure visible and stylable:
///
/// - `mkb:` reference links become `<a class="wikilink" …>` chips (dangling ones also get
///   `unresolved`), so a UI can style and intercept navigation on them;
/// - the embed-card sentinel (`⧉` as the first content of a blockquote, emitted by
///   `mkb_core::render`) tags that blockquote `class="mkb-embed"`, so transclusions render
///   as framed "live mirror" cards rather than ordinary quotes.
///
/// This is a pure string pass keyed on markers the core renderer controls, so both the web
/// and desktop UIs get identical wiki styling from the one shared renderer.
fn decorate_wiki(html: String) -> String {
    html.replace(
        "<a href=\"mkb:?unresolved\"",
        "<a class=\"wikilink unresolved\" href=\"mkb:?unresolved\"",
    )
    .replace("<a href=\"mkb:", "<a class=\"wikilink\" href=\"mkb:")
    .replace(
        "<blockquote>\n<p>⧉",
        "<blockquote class=\"mkb-embed\">\n<p>⧉",
    )
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
            let chips: String = r
                .tags
                .iter()
                .map(|t| {
                    format!(
                        "<span class=\"tag\" data-tag=\"{0}\">#{0}</span>",
                        escape_html(t)
                    )
                })
                .collect();
            format!("<span class=\"crumb\">{chips}</span>")
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

#[cfg(test)]
mod tests {
    use super::*;
    use mkb_core::BlockId;

    /// Plain-render entry for the engine tests below: the live **remote-vault** render path
    /// (`vault_root = None`), which is what production renders for a remote vault. These tests
    /// exercise shared `render_markdown` behavior (wikilinks, embeds, code fences, XSS, headings)
    /// that is independent of image handling, so routing them through the real renderer keeps them
    /// honest without a phantom "plain" API.
    fn render(md: &str) -> String {
        markdown_to_html_with_assets(md, None)
    }

    #[test]
    fn markdown_renders_and_strips_ids() {
        let id = BlockId::generate();
        let md = format!(
            "# Title {}\n\nSome **bold** text.\n",
            NativeIdCodec.encode(&id)
        );
        let html = render(&md);
        assert!(html.contains("<h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        // The id marker must not appear in the output.
        assert!(!html.contains("mkb:"));
        assert!(!html.contains(id.as_str()));
    }

    #[test]
    fn wiki_reference_becomes_chip_link() {
        // Mirrors what mkb_core::render emits for a resolved `[[...]]` reference.
        let html = render("see [ideas](mkb:ideas.md) now");
        assert!(
            html.contains("<a class=\"wikilink\" href=\"mkb:ideas.md\">ideas</a>"),
            "got: {html}"
        );
    }

    #[test]
    fn unresolved_reference_is_marked() {
        let html = render("see [ghost](mkb:?unresolved) now");
        assert!(
            html.contains("class=\"wikilink unresolved\""),
            "got: {html}"
        );
    }

    #[test]
    fn embed_card_blockquote_is_tagged() {
        // Mirrors mkb_core::render's embed card: a blockquote whose first content is `⧉`.
        let html = render("> ⧉ [src](mkb:src.md#01ABC)\n>\n> the body\n");
        assert!(
            html.contains("<blockquote class=\"mkb-embed\">"),
            "got: {html}"
        );
        assert!(html.contains("the body"));
    }

    #[test]
    fn top_level_block_spans_cover_each_block() {
        let md = "First paragraph.\n\n## A heading\n\n- one\n- two\n\n> a quote\n";
        let spans = top_level_block_spans(md);
        let slices: Vec<&str> = spans.iter().map(|&(s, e)| &md[s..e]).collect();
        assert_eq!(slices.len(), 4);
        assert!(slices[0].starts_with("First paragraph."));
        assert!(slices[1].starts_with("## A heading"));
        assert!(slices[2].starts_with("- one"));
        assert!(slices[3].starts_with("> a quote"));
        // Spans are non-overlapping and in order.
        for w in spans.windows(2) {
            assert!(w[0].1 <= w[1].0, "spans overlap: {spans:?}");
        }
    }

    #[test]
    fn indexed_render_stamps_top_level_blocks_in_order() {
        let md = "First para.\n\n## Heading\n\n> ⧉ [c](mkb:c.md#01ABC)\n>\n> child body\n";
        let html = markdown_to_html_with_assets_indexed(md, None);
        // One data-bi per top-level block, sequential from 0.
        assert!(html.contains("<p data-bi=\"0\">"), "got: {html}");
        assert!(html.contains("<h2 data-bi=\"1\">"), "got: {html}");
        // The embed card keeps its class AND gains the index.
        assert!(
            html.contains("<blockquote data-bi=\"2\" class=\"mkb-embed\">"),
            "got: {html}"
        );
        // The count of stamped elements matches the raw block spans (the zip contract).
        let raw = "First para.\n\n## Heading\n\n![[01ABC]]\n";
        assert_eq!(top_level_block_spans(raw).len(), 3);
    }

    #[test]
    fn plain_render_has_no_block_index() {
        // The non-indexed renderers stay attribute-free (other consumers unaffected).
        let html = render("a para\n\n## h\n");
        assert!(!html.contains("data-bi"), "got: {html}");
    }

    #[test]
    fn code_fence_language_becomes_class() {
        let html = render("```kusto\nStormEvents | take 10\n```\n");
        assert!(html.contains("language-kusto"));
        assert!(html.contains("StormEvents"));
    }

    #[test]
    fn raw_html_is_neutralised_not_executed() {
        // Stored-XSS guard: a script/img payload in note content must not survive as live
        // markup. It is escaped to inert text instead.
        let html = render("hello <script>alert('xss')</script> world\n");
        assert!(
            !html.contains("<script>"),
            "raw <script> must not pass through"
        );
        assert!(html.contains("&lt;script&gt;"));
        let img = render("<img src=x onerror=alert(1)>\n");
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
    fn vault_asset_path_resolves_relative_and_skips_external() {
        let root = Path::new("/vault");
        assert_eq!(
            vault_asset_path("assets/x.png", root),
            Some(PathBuf::from("/vault/assets/x.png"))
        );
        assert_eq!(
            vault_asset_path("./assets/x.png", root),
            Some(PathBuf::from("/vault/assets/x.png"))
        );
        // Leading slash / `..` can never escape the vault.
        assert_eq!(
            vault_asset_path("/assets/x.png", root),
            Some(PathBuf::from("/vault/assets/x.png"))
        );
        assert_eq!(
            vault_asset_path("../../etc/passwd", root),
            Some(PathBuf::from("/vault/etc/passwd"))
        );
        // External / scheme / fragment / empty are left for the UI to load as-is.
        for ext in [
            "https://example.com/a.png",
            "http://x/a.png",
            "data:image/png;base64,AAAA",
            "//cdn/a.png",
            "#anchor",
            "",
        ] {
            assert_eq!(vault_asset_path(ext, root), None, "should skip {ext}");
        }
    }

    #[test]
    fn asset_rendering_rewrites_relative_and_blocks_external() {
        let html = markdown_to_html_with_assets(
            "![a](assets/x.png) and ![b](https://h/y.png)\n",
            Some(Path::new("/vault")),
        );
        assert!(
            html.contains("src=\"mkb-asset:/vault/assets/x.png\""),
            "relative image should become an asset URL; got: {html}"
        );
        // The external image is inert: no <img>, no network-loadable src.
        assert!(
            !html.contains("src=\"https://h/y.png\""),
            "external image must not be a live src; got: {html}"
        );
        assert!(
            html.contains("mkb-extern-img"),
            "external image should become an inert placeholder; got: {html}"
        );
        assert!(
            html.contains('b'),
            "alt text should be preserved; got: {html}"
        );
    }

    #[test]
    fn asset_url_always_uses_forward_slashes() {
        // OS-independent guard: a Windows-style absolute path must serialise with `/`, not `\`
        // (a `\` would be percent-encoded to `%5C` and break the desktop front-end's mapping).
        // `to_string_lossy` keeps the literal `\` on every OS, so this also fails on Linux CI if
        // the conversion regresses.
        assert_eq!(
            asset_url_path(Path::new(r"C:\vault\assets\x.png")),
            "C:/vault/assets/x.png"
        );
        assert_eq!(
            asset_url_path(Path::new("/vault/assets/x.png")),
            "/vault/assets/x.png"
        );
        // And the full render must never leak a backslash (raw or percent-encoded) into the URL.
        let html = markdown_to_html_with_assets("![a](assets/x.png)\n", Some(Path::new("/vault")));
        assert!(!html.contains("%5C") && !html.contains('\\'), "got: {html}");
    }

    #[test]
    fn external_image_is_inert_even_without_a_vault_root() {
        // Remote vault (no root): external images are still blocked.
        let html = markdown_to_html_with_assets("![pic](http://x/y.png)\n", None);
        assert!(!html.contains("<img"), "no live img; got: {html}");
        assert!(html.contains("mkb-extern-img"), "got: {html}");
        // A protocol-relative source is external too.
        let pr = markdown_to_html_with_assets("![](//cdn/a.png)\n", None);
        assert!(pr.contains("mkb-extern-img"), "got: {pr}");
    }

    #[test]
    fn block_title_prefers_title_then_first_line() {
        assert_eq!(block_title(Some("Explicit"), "body"), "Explicit");
        assert_eq!(block_title(None, "# Heading\n\nbody"), "Heading");
        assert_eq!(block_title(Some("  "), "first line"), "first line");
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
        // Tags render as clickable chips carrying the tag name.
        assert!(html.contains("<span class=\"tag\" data-tag=\"top\">#top</span>"));
    }

    #[test]
    fn empty_search_says_no_matches() {
        assert!(search_results_html("q", &[]).contains("No matches"));
    }
}
