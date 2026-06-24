//! MCP tool definitions and their mapping to daemon requests.
//!
//! Pure translation: a tool call becomes a [`mdkb_protocol::Request`], which the daemon
//! dispatches to the one shared `Service`. No knowledge-base behavior lives here — only
//! schemas and (de)serialization — so the MCP server cannot diverge (see `AGENTS.md`). The
//! unit is the **block** (one file); reuse is by block id (`![[id]]` child / `[[id]]` ref).

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
            description: "Search the knowledge base (keyword + semantic). Optional filters: lang, tags, limit, and created/updated date ranges (the staleness/freshness audit — e.g. updated_before to find blocks not touched since a date). Dates are YYYY-MM-DD or RFC 3339. created comes free from each block's id; updated is the last-write time.",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Free-text query"},
                    "lang": {"type": "string", "description": "Restrict to a code-fence language (e.g. kusto)"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Require all of these tags"},
                    "limit": {"type": "integer", "description": "Max results (default 50)"},
                    "created_after": {"type": "string", "description": "Only blocks created on/after this date (YYYY-MM-DD or RFC 3339)"},
                    "created_before": {"type": "string", "description": "Only blocks created before this date"},
                    "updated_after": {"type": "string", "description": "Only blocks last-modified on/after this date"},
                    "updated_before": {"type": "string", "description": "Only blocks last-modified before this date (find stale blocks)"}
                }
            }),
        },
        ToolDef {
            name: "get_block",
            description: "Fetch a single block by id: its title, tags, properties, timestamps (created — free from the id — and updated), and Markdown body.",
            schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        },
        ToolDef {
            name: "render_block",
            description: "Render a block with all child transclusions (![[id]]) resolved inline.",
            schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        },
        ToolDef {
            name: "list_blocks",
            description: "List all block ids in the vault.",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "list_roots",
            description: "List root block ids (top-level entries that nothing transcludes).",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "graph",
            description: "The block-level knowledge graph (nodes = blocks, edges = references/transclusions).",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "list_tags",
            description: "List all tags in the vault with how many blocks carry each (tag discovery). Use a returned tag with search's `tags` filter to scope a domain.",
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
            name: "create_block",
            description: "Create a new block (optional title + Markdown body). Returns the new block id.",
            schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "body": {"type": "string"}
                },
                "required": ["body"]
            }),
        },
        ToolDef {
            name: "update_block",
            description: "Overwrite a block's title + Markdown body, by id. This replaces the ENTIRE body, so read the current body first (get_block) and send the full revised text — don't send a fragment. An edit that would empty the block or strip most of its content is refused unless force=true (use that only for a deliberate rewrite).",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "force": {"type": "boolean", "description": "Bypass the destructive-update guard for an intentional rewrite (default false)"}
                },
                "required": ["id", "body"]
            }),
        },
        ToolDef {
            name: "delete_block",
            description: "Delete a block (removes its file and index entries).",
            schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        },
        ToolDef {
            name: "set_tags",
            description: "Set a block's managed (frontmatter) tags to exactly the given list (replaces them; pass [] to clear). Inline #hashtags in the body are left untouched. Title and body are preserved.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "The full desired tag set"}
                },
                "required": ["id", "tags"]
            }),
        },
        ToolDef {
            name: "set_props",
            description: "Add or update a block's properties (open-ended key: value metadata, e.g. source/verified/confidence). Each given key is added or updated; ALL OTHER PROPERTIES ARE PRESERVED — this never replaces the whole set, so you can't accidentally drop a property you didn't name. Title, tags, lock state, and body are preserved. Property values are full-text searchable. Use unset_props to remove a property.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "props": {
                        "type": "array",
                        "description": "The properties to add or update (other properties are kept)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "key": {"type": "string"},
                                "value": {"type": "string"}
                            },
                            "required": ["key", "value"]
                        }
                    }
                },
                "required": ["id", "props"]
            }),
        },
        ToolDef {
            name: "unset_props",
            description: "Remove the named properties from a block, preserving all other properties (and title, tags, lock state, body). Keys not present are ignored. This is the only way to remove a property.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "keys": {"type": "array", "items": {"type": "string"}, "description": "The property keys to remove"}
                },
                "required": ["id", "keys"]
            }),
        },
        ToolDef {
            name: "carve_block",
            description: "Carve a new child block out of a parent: the new block gets the given body, and a ![[child]] directive is appended to the parent in place. Returns the new child id.",
            schema: json!({
                "type": "object",
                "properties": {
                    "parent_id": {"type": "string"},
                    "title": {"type": "string"},
                    "body": {"type": "string"}
                },
                "required": ["parent_id", "body"]
            }),
        },
        ToolDef {
            name: "flatten_block",
            description: "Flatten (uncarve): inline a parent's single ![[child]] embed back into the parent body and delete the child block. The inverse of carve_block. Only valid when the child is referenced in exactly one place in the whole vault (a single ![[child]] embed in the given parent, with no other embedders and no [[references]]); errors otherwise and changes nothing. The child's own ![[grandchild]] embeds are preserved.",
            schema: json!({
                "type": "object",
                "properties": {
                    "parent_id": {"type": "string"},
                    "child_id": {"type": "string"}
                },
                "required": ["parent_id", "child_id"]
            }),
        },
        ToolDef {
            name: "link_blocks",
            description: "Link or embed one block into another. embed=true writes a transclusion (![[id]]); false a plain reference ([[id]]). An embed that would create a cycle is auto-downgraded to a reference.",
            schema: json!({
                "type": "object",
                "properties": {
                    "source_id": {"type": "string"},
                    "target_id": {"type": "string"},
                    "embed": {"type": "boolean"}
                },
                "required": ["source_id", "target_id", "embed"]
            }),
        },
        ToolDef {
            name: "stats",
            description: "Index statistics: block, root, and embedding counts.",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "rebuild",
            description: "Rebuild the entire search index from the block files (the source of truth).",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "conflicts",
            description: "List cloud-sync conflict files detected in the vault (surfaced, not indexed).",
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
    let req_id = |key: &str| -> Result<BlockId, String> {
        let v = s(key).ok_or_else(|| format!("missing required argument: {key}"))?;
        BlockId::parse(&v).map_err(|_| format!("invalid block id for {key}: {v}"))
    };

    Ok(match name {
        "search" => {
            // Parse the query for inline operators (tag:/#tag/lang:/created:/updated:) exactly like
            // the CLI/app/web, then overlay the explicit structured arguments on top so both styles
            // work and the agent's explicit filters win.
            let mut q = match s("query") {
                Some(text) => SearchQuery::parse(&text),
                None => SearchQuery::default(),
            };
            if let Some(lang) = s("lang") {
                q.lang = Some(lang);
            }
            if let Some(arr) = args.get("tags").and_then(|v| v.as_array()) {
                for t in arr.iter().filter_map(|x| x.as_str()) {
                    if !q.tags.iter().any(|e| e.eq_ignore_ascii_case(t)) {
                        q.tags.push(t.to_string());
                    }
                }
            }
            if let Some(limit) = args.get("limit").and_then(|v| v.as_u64()) {
                q.limit = limit as usize;
            }
            // Normalize date filters to canonical RFC 3339; an unparsable date errors rather than
            // being silently ignored. An explicit arg overrides the same operator from the query.
            let date = |key: &str| -> Result<Option<String>, String> {
                match s(key) {
                    None => Ok(None),
                    Some(d) => mdkb_core::clock::parse_query_date(&d)
                        .map(Some)
                        .ok_or_else(|| format!("invalid date for {key}: {d}")),
                }
            };
            if let Some(d) = date("created_after")? {
                q.created_after = Some(d);
            }
            if let Some(d) = date("created_before")? {
                q.created_before = Some(d);
            }
            if let Some(d) = date("updated_after")? {
                q.updated_after = Some(d);
            }
            if let Some(d) = date("updated_before")? {
                q.updated_before = Some(d);
            }
            Request::Search { query: q }
        }
        "get_block" => Request::GetBlock { id: req_id("id")? },
        "render_block" => Request::RenderBlock { id: req_id("id")? },
        "list_blocks" => Request::ListBlocks,
        "list_roots" => Request::ListRoots,
        "graph" => Request::Graph,
        "list_tags" => Request::ListTags,
        "backlinks" => Request::Backlinks { id: req_id("id")? },
        "links_from" => Request::LinksFrom { id: req_id("id")? },
        "create_block" => Request::CreateBlock {
            title: s("title"),
            body: req_s("body")?,
        },
        "update_block" => Request::UpdateBlock {
            id: req_id("id")?,
            title: s("title"),
            body: req_s("body")?,
            force: args.get("force").and_then(|v| v.as_bool()).unwrap_or(false),
        },
        "delete_block" => Request::DeleteBlock { id: req_id("id")? },
        "set_tags" => Request::SetTags {
            id: req_id("id")?,
            tags: args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .ok_or_else(|| "missing required argument: tags".to_string())?,
        },
        "set_props" => Request::SetProps {
            id: req_id("id")?,
            props: {
                let arr = args
                    .get("props")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| "missing required argument: props".to_string())?;
                let mut out = Vec::with_capacity(arr.len());
                for (i, item) in arr.iter().enumerate() {
                    // Error (don't silently drop) on a malformed item: set_props REPLACES the whole
                    // set, so dropping one would quietly delete that property instead of failing.
                    let key = item
                        .get("key")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| format!("props[{i}] is missing a string \"key\""))?;
                    let value = item
                        .get("value")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| format!("props[{i}] is missing a string \"value\""))?;
                    out.push((key.to_string(), value.to_string()));
                }
                out
            },
        },
        "unset_props" => Request::UnsetProps {
            id: req_id("id")?,
            keys: args
                .get("keys")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .ok_or_else(|| "missing required argument: keys".to_string())?,
        },
        "carve_block" => Request::CarveBlock {
            parent_id: req_id("parent_id")?,
            title: s("title"),
            body: req_s("body")?,
        },
        "flatten_block" => Request::FlattenBlock {
            parent_id: req_id("parent_id")?,
            child_id: req_id("child_id")?,
        },
        "link_blocks" => Request::LinkBlocks {
            source_id: req_id("source_id")?,
            target_id: req_id("target_id")?,
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
        Response::Linked(o) => Ok(match o {
            mdkb_core::LinkOutcome::Reference => "linked (reference)".to_string(),
            mdkb_core::LinkOutcome::Transclusion => "linked (transclusion)".to_string(),
            mdkb_core::LinkOutcome::DowngradedToReference => {
                "linked as a plain reference (an embed would have created a transclusion cycle)"
                    .to_string()
            }
        }),
        Response::Ids(v) => to_json(v),
        Response::Names(n) => to_json(n),
        Response::Hits(h) => to_json(h),
        Response::Block(b) => to_json(b),
        Response::Rendered(b) => to_json(b),
        Response::Links(l) => to_json(l),
        Response::Stats(s) => to_json(s),
        Response::Graph(g) => to_json(g),
        Response::Tags(t) => to_json(t),
        Response::Exports(d) => to_json(d),
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
        assert!(defs.len() >= 14);
    }

    #[test]
    fn every_tool_name_builds_a_request() {
        let id = BlockId::generate().to_string();
        let args = json!({
            "query": "q", "id": id, "source_id": id, "target_id": id, "parent_id": id,
            "child_id": id, "title": "T", "body": "b", "embed": true, "tags": ["x"],
            "props": [{"key": "source", "value": "git"}], "keys": ["source"]
        });
        for d in tool_definitions() {
            build_request(d.name, &args)
                .unwrap_or_else(|e| panic!("tool {} failed to build: {e}", d.name));
        }
    }

    #[test]
    fn set_props_maps_key_value_objects() {
        let id = BlockId::generate().to_string();
        let args = json!({
            "id": id,
            "props": [
                {"key": "source", "value": "https://example.com/x"},
                {"key": "verified", "value": "2026-06-01"}
            ]
        });
        match build_request("set_props", &args).unwrap() {
            Request::SetProps { props, .. } => {
                assert_eq!(
                    props,
                    vec![
                        ("source".to_string(), "https://example.com/x".to_string()),
                        ("verified".to_string(), "2026-06-01".to_string()),
                    ]
                );
            }
            _ => panic!("expected set_props"),
        }
    }

    #[test]
    fn set_props_errors_on_malformed_item() {
        // Because set_props replaces the whole set, a malformed item must error, not be dropped
        // (silently shrinking the set would delete a property the caller meant to keep).
        let id = BlockId::generate().to_string();
        let args = json!({"id": id, "props": [{"key": "source"}]});
        assert!(build_request("set_props", &args).is_err());
        let args = json!({"id": id, "props": [{"value": "git"}]});
        assert!(build_request("set_props", &args).is_err());
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
    fn search_parses_query_operators_and_merges_explicit_args() {
        // Operators typed into the query (parity with CLI/app/web) are honored, and explicit
        // structured args merge on top — so a date filter works whether sent as an operator or arg.
        let args = json!({
            "query": "deploy #k8s updated:before:2026-01-01",
            "tags": ["ops"],
            "created_after": "2025-01-01"
        });
        match build_request("search", &args).unwrap() {
            Request::Search { query } => {
                assert_eq!(query.text.as_deref(), Some("deploy"));
                assert!(query.tags.iter().any(|t| t == "k8s"), "operator tag kept");
                assert!(query.tags.iter().any(|t| t == "ops"), "explicit tag merged");
                assert_eq!(
                    query.updated_before.as_deref(),
                    Some("2026-01-01T00:00:00Z"),
                    "operator date parsed + normalized"
                );
                assert_eq!(
                    query.created_after.as_deref(),
                    Some("2025-01-01T00:00:00Z"),
                    "explicit date arg applied"
                );
            }
            _ => panic!("expected search"),
        }
    }

    #[test]
    fn search_rejects_bad_date_arg() {
        let args = json!({"query": "x", "updated_before": "last tuesday"});
        assert!(build_request("search", &args).is_err());
    }

    #[test]
    fn create_block_maps_title_and_body() {
        let args = json!({"title": "T", "body": "hi"});
        match build_request("create_block", &args).unwrap() {
            Request::CreateBlock { title, body } => {
                assert_eq!(title.as_deref(), Some("T"));
                assert_eq!(body, "hi");
            }
            _ => panic!("expected create_block"),
        }
    }

    #[test]
    fn missing_required_arg_errors() {
        assert!(build_request("get_block", &json!({})).is_err());
        assert!(build_request("create_block", &json!({})).is_err());
        // set_tags requires a tags array.
        let id = BlockId::generate().to_string();
        assert!(build_request("set_tags", &json!({ "id": id })).is_err());
    }

    #[test]
    fn set_tags_maps_id_and_tags() {
        let id = BlockId::generate().to_string();
        match build_request("set_tags", &json!({ "id": id, "tags": ["a", "b"] })).unwrap() {
            Request::SetTags { tags, .. } => assert_eq!(tags, vec!["a", "b"]),
            _ => panic!("expected set_tags"),
        }
        // An empty list is valid (clears tags).
        match build_request("set_tags", &json!({ "id": id, "tags": [] })).unwrap() {
            Request::SetTags { tags, .. } => assert!(tags.is_empty()),
            _ => panic!("expected set_tags"),
        }
    }

    #[test]
    fn list_tags_maps() {
        assert!(matches!(
            build_request("list_tags", &json!({})).unwrap(),
            Request::ListTags
        ));
    }

    #[test]
    fn error_response_becomes_err() {
        let resp = Response::Error {
            message: "boom".into(),
        };
        assert_eq!(format_response(&resp).unwrap_err(), "boom");
    }
}
