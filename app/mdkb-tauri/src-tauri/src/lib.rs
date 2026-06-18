//! mdkb desktop shell (Tauri).
//!
//! A **thin client**: the Tauri commands fetch data from the daemon via `mdkb-protocol`
//! and render via the shared `mdkb-view` layer — the exact same presentation path as the
//! web UI, so the two front-ends cannot diverge (see `AGENTS.md`). No knowledge-base
//! behavior lives here.

use std::sync::Mutex;

use mdkb_core::SearchQuery;
use mdkb_protocol::{Client, DaemonPaths, Request, Response};
use mdkb_view::{markdown_to_html, page_title, search_results_html, ResultRow};

/// Shared application state: the connection to the daemon.
struct AppState {
    client: Mutex<Client>,
}

#[tauri::command]
fn list_pages(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let client = state.client.lock().map_err(|_| "state poisoned")?;
    match client.call(&Request::ListPages).map_err(|e| e.to_string())? {
        Response::Pages(p) => Ok(p),
        Response::Error { message } => Err(message),
        other => Err(format!("unexpected response: {other:?}")),
    }
}

/// Render a page to HTML (transclusions resolved by the daemon, Markdown→HTML by mdkb-view).
#[tauri::command]
fn render_page(state: tauri::State<'_, AppState>, page: String) -> Result<String, String> {
    let client = state.client.lock().map_err(|_| "state poisoned")?;
    let md = client
        .render_page(&page)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("page not found: {page}"))?;
    Ok(markdown_to_html(&md))
}

/// Page title helper for the front-end.
#[tauri::command]
fn title_for(page: String) -> String {
    page_title(&page)
}

/// Search and return a ready-to-inject HTML fragment.
#[tauri::command]
fn search(state: tauri::State<'_, AppState>, query: String) -> Result<String, String> {
    let client = state.client.lock().map_err(|_| "state poisoned")?;
    let q = SearchQuery {
        text: Some(query.clone()),
        ..Default::default()
    };
    let hits = client.search(q).map_err(|e| e.to_string())?;
    let rows: Vec<ResultRow> = hits
        .into_iter()
        .map(|h| ResultRow {
            page_path: h.block.page_path,
            id: h.block.id.to_string(),
            lineage: h.block.lineage,
            content: h.block.content,
        })
        .collect();
    Ok(search_results_html(&query, &rows))
}

/// Resolve the daemon connection from the environment so the desktop app can talk to a
/// local socket **or** a remote TCP daemon. Set `MDKB_REMOTE=host:port` + `MDKB_TOKEN` to
/// point at a deployed `mdkbd`; otherwise it uses the local vault's socket. Falls back to
/// the default local socket if the environment is misconfigured (logged to stderr).
fn resolve_client() -> Client {
    match Client::from_env() {
        Ok(client) => {
            eprintln!("mdkb: connecting to {}", client.endpoint());
            client
        }
        Err(e) => {
            eprintln!("mdkb: {e}; falling back to the local socket");
            Client::new(DaemonPaths::for_default_vault().socket)
        }
    }
}

/// Entry point used by the generated binary.
pub fn run() {
    let state = AppState {
        client: Mutex::new(resolve_client()),
    };
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            list_pages,
            render_page,
            title_for,
            search
        ])
        .run(tauri::generate_context!())
        .expect("error while running mdkb desktop shell");
}
