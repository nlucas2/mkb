//! Route handling for the web UI.
//!
//! Routes call a [`Backend`] (the daemon, in production) and render via the shared
//! `mdkb-view` layer. The `Backend` seam keeps routing testable with a mock and means the
//! UI shares the *exact* presentation path with any other front-end.

use mdkb_view::{page_document, search_results_html, ResultRow};

use crate::http::{HttpRequest, HttpResponse};

/// The data operations the web UI needs from the daemon. Implemented for the real client in
/// `main`; mocked in tests.
pub trait Backend {
    /// List page paths.
    fn list_pages(&self) -> Result<Vec<String>, String>;
    /// Render a page (transclusions resolved) to Markdown.
    fn render_page(&self, page: &str) -> Result<Option<String>, String>;
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
        p if p.starts_with("/page/") => page(backend, &p["/page/".len()..]),
        _ => HttpResponse::not_found(shell(backend, "Not found", &error_page("Not found"), "")),
    }
}

fn home(backend: &dyn Backend) -> HttpResponse {
    match backend.list_pages() {
        Ok(pages) if !pages.is_empty() => HttpResponse::redirect(&format!("/page/{}", pages[0])),
        Ok(_) => HttpResponse::html(shell(
            backend,
            "mdkb",
            "<h1>mdkb</h1><p class=\"muted\">No pages yet.</p>",
            "",
        )),
        Err(e) => HttpResponse::html(shell(backend, "mdkb", &error_page(&e), "")),
    }
}

fn page(backend: &dyn Backend, path: &str) -> HttpResponse {
    match backend.render_page(path) {
        Ok(Some(md)) => {
            let title = mdkb_view::page_title(path);
            let body = mdkb_view::markdown_to_html(&md);
            HttpResponse::html(shell(backend, &title, &body, path))
        }
        Ok(None) => HttpResponse::not_found(shell(
            backend,
            "Not found",
            &error_page(&format!("Page not found: {path}")),
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

/// Wrap a body in the full document, fetching the page list for the sidebar.
fn shell(backend: &dyn Backend, title: &str, body: &str, active: &str) -> String {
    let pages = backend.list_pages().unwrap_or_default();
    page_document(title, body, &pages, active)
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
        pages: Vec<String>,
    }

    impl Backend for MockBackend {
        fn list_pages(&self) -> Result<Vec<String>, String> {
            Ok(self.pages.clone())
        }
        fn render_page(&self, page: &str) -> Result<Option<String>, String> {
            if self.pages.iter().any(|p| p == page) {
                Ok(Some(format!("# {page}\n\nbody of {page}\n")))
            } else {
                Ok(None)
            }
        }
        fn search(&self, query: &str) -> Result<Vec<ResultRow>, String> {
            Ok(vec![ResultRow {
                page_path: "notes/arch.md".into(),
                id: "x".into(),
                lineage: vec!["Top".into()],
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
            pages: vec!["notes/arch.md".into(), "queries.md".into()],
        }
    }

    #[test]
    fn home_redirects_to_first_page() {
        let r = handle(&backend(), &req("/"));
        assert_eq!(r.status, 302);
        assert_eq!(r.location.as_deref(), Some("/page/notes/arch.md"));
    }

    #[test]
    fn page_renders_markdown_into_document() {
        let r = handle(&backend(), &req("/page/queries.md"));
        let body = String::from_utf8_lossy(&r.body);
        assert_eq!(r.status, 200);
        assert!(body.contains("<!DOCTYPE html>"));
        assert!(body.contains("body of queries.md"));
        // Sidebar + active highlight.
        assert!(body.contains("class=\"active\""));
    }

    #[test]
    fn missing_page_is_404() {
        let r = handle(&backend(), &req("/page/nope.md"));
        assert_eq!(r.status, 404);
    }

    #[test]
    fn search_route_uses_query_param() {
        let mut request = req("/search");
        request.query.insert("q".into(), "restart".into());
        let r = handle(&backend(), &request);
        let body = String::from_utf8_lossy(&r.body);
        assert!(body.contains("matching restart"));
        assert!(body.contains("href=\"/page/notes/arch.md\""));
    }

    #[test]
    fn non_get_is_rejected() {
        let mut request = req("/");
        request.method = "POST".into();
        assert_eq!(handle(&backend(), &request).status, 404);
    }
}
