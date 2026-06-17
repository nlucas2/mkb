//! MCP tool definitions and their mapping to daemon requests.
//!
//! This is pure translation: a tool call becomes a [`mdkb_protocol::Request`], which the
//! daemon dispatches to the one shared `Service`. No knowledge-base behavior lives here —
//! only schemas and (de)serialization — so the MCP server cannot diverge from the CLI or UI
//! (see `AGENTS.md`).

use mdkb_core::{BlockId, SearchQuery};
use mdkb_protocol::{Request, Response};
use serde_json::{json, Value};

/// A tool exposed over MCP.
pub struct ToolDef {
    /// Tool name.
    pub name: &'static str,
    /// One-line description.
    pub description: &'static str,
    /// JSON Schema for the tool's arguments.
    pub schema: Value,
}

/// All tools the server exposes.
pub fn tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "search",
            description: "Search the knowledge base (keyword + semantic). Optional filters: lang, tags, page, limit.",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Free-text query"},
                    "lang": {"type": "string", "description": "Restrict to a code-fence language (e.g. kusto)"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Require all of these tags"},
                    "page": {"type": "string", "description": "Restrict to a page path"},
                    "limit": {"type": "integer", "description": "Max results (default 50)"}
                }
            }),
        },
        ToolDef {
            name: "get_block",
            description: "Fetch a single block by its id.",
            schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        },
        ToolDef {
            name: "get_page",
            description: "Get the raw Markdown source of a page.",
            schema: json!({
                "type": "object",
                "properties": {"page": {"type": "string"}},
                "required": ["page"]
            }),
        },
        ToolDef {
            name: "render_page",
            description: "Render a page with all transclusions resolved (inlined).",
            schema: json!({
                "type": "object",
                "properties": {"page": {"type": "string"}},
                "required": ["page"]
            }),
        },
        ToolDef {
            name: "list_pages",
            description: "List all page paths in the vault.",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "backlinks",
            description: "List blocks that reference or transclude a given block id.",
            schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        },
        ToolDef {
            name: "links_from",
            description: "List outgoing links/transclusions from a given block id.",
            schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        },
        ToolDef {
            name: "upsert_block",
            description: "Update a block by id, or append a new block to a page when no id is given. Returns the affected block id.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Existing block id to update; omit to create"},
                    "text": {"type": "string", "description": "The block's Markdown text"},
                    "page": {"type": "string", "description": "Target page (required when creating)"}
                },
                "required": ["text"]
            }),
        },
        ToolDef {
            name: "save_page",
            description: "Create or overwrite an entire page from raw Markdown.",
            schema: json!({
                "type": "object",
                "properties": {
                    "page": {"type": "string"},
                    "source": {"type": "string"}
                },
                "required": ["page", "source"]
            }),
        },
        ToolDef {
            name: "delete_page",
            description: "Delete a page (removes the file and its index entries).",
            schema: json!({
                "type": "object",
                "properties": {"page": {"type": "string"}},
                "required": ["page"]
            }),
        },
        ToolDef {
            name: "link_blocks",
            description: "Create a link or transclusion from a source block to a target. Set embed=true for a transclusion (![[...]]), false for a plain link ([[...]]).",
            schema: json!({
                "type": "object",
                "properties": {
                    "source_id": {"type": "string"},
                    "target_page": {"type": "string"},
                    "target_id": {"type": "string"},
                    "target_anchor": {"type": "string"},
                    "embed": {"type": "boolean"}
                },
                "required": ["source_id", "embed"]
            }),
        },
        ToolDef {
            name: "stats",
            description: "Index statistics: page, block, and embedding counts.",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "rebuild",
            description: "Rebuild the entire search index from the vault's Markdown files (the source of truth). Use after corruption or to force a full re-index.",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "conflicts",
            description: "List cloud-sync conflict files (e.g. OneDrive copies) detected in the vault. These are surfaced but not indexed; resolve them in plain text.",
            schema: json!({"type": "object", "properties": {}}),
        },
    ]
}

/// Build a daemon [`Request`] from a tool name and its JSON arguments.
pub fn build_request(name: &str, args: &Value) -> Result<Request, String> {
    let s = |key: &str| {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
    };
    let req_s = |key: &str| s(key).ok_or_else(|| format!("missing required argument: {key}"));
    let id = |key: &str| -> Result<Option<BlockId>, String> {
        match s(key) {
            Some(v) => BlockId::parse(&v)
                .map(Some)
                .map_err(|_| format!("invalid block id for {key}: {v}")),
            None => Ok(None),
        }
    };

    Ok(match name {
        "search" => {
            let tags = args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            Request::Search {
                query: SearchQuery {
                    text: s("query"),
                    vector: None,
                    tags,
                    lang: s("lang"),
                    page: s("page"),
                    limit,
                },
            }
        }
        "get_block" => Request::GetBlock {
            id: id("id")?.ok_or("missing required argument: id")?,
        },
        "get_page" => Request::GetPageSource {
            page: req_s("page")?,
        },
        "render_page" => Request::RenderPage {
            page: req_s("page")?,
        },
        "list_pages" => Request::ListPages,
        "backlinks" => Request::Backlinks {
            id: id("id")?.ok_or("missing required argument: id")?,
        },
        "links_from" => Request::LinksFrom {
            id: id("id")?.ok_or("missing required argument: id")?,
        },
        "upsert_block" => Request::UpsertBlock {
            id: id("id")?,
            text: req_s("text")?,
            page: s("page"),
        },
        "save_page" => Request::SavePage {
            page: req_s("page")?,
            source: req_s("source")?,
        },
        "delete_page" => Request::DeletePage {
            page: req_s("page")?,
        },
        "link_blocks" => Request::LinkBlocks {
            source_id: id("source_id")?.ok_or("missing required argument: source_id")?,
            target_page: s("target_page"),
            target_id: id("target_id")?,
            target_anchor: s("target_anchor"),
            embed: args.get("embed").and_then(|v| v.as_bool()).unwrap_or(false),
        },
        "stats" => Request::Stats,
        "rebuild" => Request::Rebuild,
        "conflicts" => Request::Conflicts,
        other => return Err(format!("unknown tool: {other}")),
    })
}

/// Render a daemon [`Response`] as MCP tool result text (JSON for structured payloads).
pub fn format_response(resp: &Response) -> Result<String, String> {
    match resp {
        Response::Error { message } => Err(message.clone()),
        Response::Pong => Ok("pong".to_string()),
        Response::Ok => Ok("ok".to_string()),
        Response::Text(t) => Ok(t.clone().unwrap_or_else(|| "(not found)".to_string())),
        Response::BlockId(id) => Ok(id.to_string()),
        Response::Pages(p) => to_json(p),
        Response::Hits(h) => to_json(h),
        Response::Block(b) => to_json(b),
        Response::Links(l) => to_json(l),
        Response::Stats(s) => to_json(s),
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<String, String> {
    serde_json::to_string_pretty(v).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tools_have_unique_names_and_object_schemas() {
        let defs = tool_definitions();
        let mut names = std::collections::HashSet::new();
        for d in &defs {
            assert!(names.insert(d.name), "duplicate tool name: {}", d.name);
            assert_eq!(
                d.schema["type"], "object",
                "{} schema must be object",
                d.name
            );
        }
        assert!(defs.len() >= 12);
    }

    #[test]
    fn every_tool_name_builds_a_request() {
        // Provide enough args that each tool's required fields are satisfied.
        let id = BlockId::generate().to_string();
        let args = json!({
            "query": "q", "id": id, "source_id": id, "page": "p.md",
            "text": "t", "source": "s", "embed": true
        });
        for d in tool_definitions() {
            build_request(d.name, &args)
                .unwrap_or_else(|e| panic!("tool {} failed to build: {e}", d.name));
        }
    }

    #[test]
    fn search_maps_all_filters() {
        let args = json!({"query": "x", "lang": "kusto", "tags": ["a", "b"], "limit": 7});
        match build_request("search", &args).unwrap() {
            Request::Search { query } => {
                assert_eq!(query.text.as_deref(), Some("x"));
                assert_eq!(query.lang.as_deref(), Some("kusto"));
                assert_eq!(query.tags, vec!["a", "b"]);
                assert_eq!(query.limit, 7);
            }
            _ => panic!("expected search"),
        }
    }

    #[test]
    fn upsert_without_id_is_create() {
        let args = json!({"text": "hi", "page": "a.md"});
        match build_request("upsert_block", &args).unwrap() {
            Request::UpsertBlock { id, page, .. } => {
                assert!(id.is_none());
                assert_eq!(page.as_deref(), Some("a.md"));
            }
            _ => panic!("expected upsert"),
        }
    }

    #[test]
    fn missing_required_arg_errors() {
        assert!(build_request("get_block", &json!({})).is_err());
        assert!(build_request("save_page", &json!({"page": "a.md"})).is_err());
    }

    #[test]
    fn error_response_becomes_err() {
        let resp = Response::Error {
            message: "boom".into(),
        };
        assert_eq!(format_response(&resp).unwrap_err(), "boom");
    }
}
