//! Route handling for the web UI (block-centric).
//!
//! Routes call a [`Backend`] (the daemon, in production) and render via the shared
//! `mdkb-view` layer, so the UI shares the *exact* presentation path with every other
//! front-end. A block is the unit: `/block/<id>` renders a block with its children expanded.

use mdkb_view::{page_document, search_results_html, NavEntry, ResultRow};

use crate::http::{HttpRequest, HttpResponse};

/// The data operations the web UI needs from the daemon.
pub trait Backend {
    /// Sidebar entries (root blocks): `(id, title)`.
    fn nav_entries(&self) -> Result<Vec<NavEntry>, String>;
    /// Render a block (children resolved) to Markdown; also returns its display title.
    fn render_block(&self, id: &str) -> Result<Option<(String, String)>, String>;
    /// Search, returning display rows.
    fn search(&self, query: &str) -> Result<Vec<ResultRow>, String>;
}

/// Handle a parsed request.
pub fn handle(backend: &dyn Backend, req: &HttpRequest) -> HttpResponse {
    if req.method != "GET" {
        return HttpResponse::not_found(error_page("Only GET is supported"));
    }
    match req.path.as_str() {
        "/" => home(backend),
        "/search" => search(
            backend,
            req.query.get("q").map(String::as_str).unwrap_or(""),
        ),
        p if p.starts_with("/block/") => block(backend, &p["/block/".len()..]),
        _ => HttpResponse::not_found(shell(backend, "Not found", &error_page("Not found"), "")),
    }
}

fn home(backend: &dyn Backend) -> HttpResponse {
    match backend.nav_entries() {
        Ok(entries) if !entries.is_empty() => {
            HttpResponse::redirect(&format!("/block/{}", entries[0].id))
        }
        Ok(_) => HttpResponse::html(shell(
            backend,
            "mdkb",
            "<h1>mdkb</h1><p class=\"muted\">No blocks yet.</p>",
            "",
        )),
        Err(e) => HttpResponse::html(shell(backend, "mdkb", &error_page(&e), "")),
    }
}

fn block(backend: &dyn Backend, id: &str) -> HttpResponse {
    match backend.render_block(id) {
        Ok(Some((title, md))) => {
            // Map the shared `mdkb:` reference scheme onto `/block/` routes so wiki links and
            // embed-card headers navigate.
            let body = mdkb_view::rewrite_mdkb_links(&mdkb_view::markdown_to_html(&md), "/block/");
            HttpResponse::html(shell(backend, &title, &body, id))
        }
        Ok(None) => HttpResponse::not_found(shell(
            backend,
            "Not found",
            &error_page(&format!("Block not found: {id}")),
            "",
        )),
        Err(e) => HttpResponse::html(shell(backend, "Error", &error_page(&e), "")),
    }
}

fn search(backend: &dyn Backend, query: &str) -> HttpResponse {
    if query.trim().is_empty() {
        return HttpResponse::html(shell(
            backend,
            "Search",
            "<h1>Search</h1><p class=\"muted\">Type a query above.</p>",
            "",
        ));
    }
    match backend.search(query) {
        Ok(rows) => {
            let body = search_results_html(query, &rows);
            HttpResponse::html(shell(backend, "Search", &body, ""))
        }
        Err(e) => HttpResponse::html(shell(backend, "Search", &error_page(&e), "")),
    }
}

/// Wrap a body in the full document, fetching the sidebar entries.
fn shell(backend: &dyn Backend, title: &str, body: &str, active: &str) -> String {
    let entries = backend.nav_entries().unwrap_or_default();
    page_document(title, body, &entries, active)
}

fn error_page(msg: &str) -> String {
    format!(
        "<h1>Error</h1><p class=\"muted\">{}</p>",
        mdkb_view::escape_html(msg)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockBackend {
        blocks: Vec<(String, String)>, // (id, title)
    }

    impl Backend for MockBackend {
        fn nav_entries(&self) -> Result<Vec<NavEntry>, String> {
            Ok(self
                .blocks
                .iter()
                .map(|(id, title)| NavEntry {
                    id: id.clone(),
                    title: title.clone(),
                })
                .collect())
        }
        fn render_block(&self, id: &str) -> Result<Option<(String, String)>, String> {
            match self.blocks.iter().find(|(bid, _)| bid == id) {
                Some((_, title)) => Ok(Some((
                    title.clone(),
                    format!("# {title}\n\nbody of {id}\n"),
                ))),
                None => Ok(None),
            }
        }
        fn search(&self, query: &str) -> Result<Vec<ResultRow>, String> {
            Ok(vec![ResultRow {
                id: "x".into(),
                title: "Arch".into(),
                tags: vec!["top".into()],
                content: format!("a block matching {query}"),
            }])
        }
    }

    fn req(path: &str) -> HttpRequest {
        HttpRequest {
            method: "GET".into(),
            path: path.into(),
            query: HashMap::new(),
        }
    }

    fn backend() -> MockBackend {
        MockBackend {
            blocks: vec![
                ("01AAA".into(), "Arch".into()),
                ("01BBB".into(), "Queries".into()),
            ],
        }
    }

    #[test]
    fn home_redirects_to_first_block() {
        let r = handle(&backend(), &req("/"));
        assert_eq!(r.status, 302);
        assert_eq!(r.location.as_deref(), Some("/block/01AAA"));
    }

    #[test]
    fn block_renders_markdown_into_document() {
        let r = handle(&backend(), &req("/block/01BBB"));
        let body = String::from_utf8_lossy(&r.body);
        assert_eq!(r.status, 200);
        assert!(body.contains("<!DOCTYPE html>"));
        assert!(body.contains("body of 01BBB"));
        assert!(body.contains("class=\"active\""));
    }

    #[test]
    fn missing_block_is_404() {
        let r = handle(&backend(), &req("/block/nope"));
        assert_eq!(r.status, 404);
    }

    #[test]
    fn search_route_uses_query_param() {
        let mut request = req("/search");
        request.query.insert("q".into(), "restart".into());
        let r = handle(&backend(), &request);
        let body = String::from_utf8_lossy(&r.body);
        assert!(body.contains("matching restart"));
        assert!(body.contains("href=\"/block/x\""));
    }

    #[test]
    fn non_get_is_rejected() {
        let mut request = req("/");
        request.method = "POST".into();
        assert_eq!(handle(&backend(), &request).status, 404);
    }
}
