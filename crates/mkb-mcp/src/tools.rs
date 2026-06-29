//! MCP tool definitions and their mapping to daemon requests.
//!
//! Pure translation: a tool call becomes a [`mkb_protocol::Request`], which the daemon
//! dispatches to the one shared `Service`. No knowledge-base behavior lives here — only
//! schemas and (de)serialization — so the MCP server cannot diverge (see `AGENTS.md`). The
//! unit is the **block** (one file); reuse is by block id (`![[id]]` child / `[[id]]` ref).

use mkb_core::{BlockId, SearchQuery};
use mkb_protocol::{Request, Response};
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

/// Power tools kept out of the default surface to keep the agent's tool list lean. They are
/// advertised only when `MKB_MCP_TOOLS=full` (or `all`); the daemon can still execute them if
/// called. Structural/metadata operations a routine read-write-search loop rarely needs.
const ADVANCED_TOOLS: &[&str] = &["set_props", "unset_props", "carve_block", "flatten_block"];

/// Whether the advanced tier is opted into via `MKB_MCP_TOOLS=full` / `=all`.
fn advanced_enabled() -> bool {
    std::env::var("MKB_MCP_TOOLS")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "full" || v == "all"
        })
        .unwrap_or(false)
}

/// The tools advertised to clients: the lean core surface, plus the advanced tier when
/// `MKB_MCP_TOOLS=full`. Diagnostics (graph/stats/conflicts/rebuild) and the read primitives that
/// [`get_block`](Request::GetBlockView) now folds in (render/line-range/backlinks/links) are
/// CLI-only — they don't earn a slot in an agent's per-turn tool budget.
pub fn tool_definitions() -> Vec<ToolDef> {
    tool_definitions_for(advanced_enabled())
}

/// The tier-filtered surface for a given advanced-mode flag (env-free, for testing).
fn tool_definitions_for(advanced: bool) -> Vec<ToolDef> {
    all_tool_definitions()
        .into_iter()
        .filter(|t| advanced || !ADVANCED_TOOLS.contains(&t.name))
        .collect()
}

/// Every tool the server can build a request for, regardless of tier. Used for request building
/// and tests; clients see the tier-filtered [`tool_definitions`].
pub fn all_tool_definitions() -> Vec<ToolDef> {
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
                    "updated_before": {"type": "string", "description": "Only blocks last-modified before this date (find stale blocks)"},
                    "has": {"type": "array", "items": {"type": "string"}, "description": "Only blocks that HAVE a property with each of these keys (metadata-completeness)"},
                    "missing": {"type": "array", "items": {"type": "string"}, "description": "Only blocks that LACK a property with each of these keys (find metadata gaps, e.g. atoms missing 'source')"}
                }
            }),
        },
        ToolDef {
            name: "get_block",
            description: "Read a block in full by id: its title, tags, properties, timestamps (created — free from the id — and updated) and Markdown body, PLUS where it lives (lineage: its root page(s) and the blocks that embed it) and its relationships (backlinks in / links out, each tagged embed vs reference). One call replaces separate read/render/backlink lookups. Options: rendered=true inlines child ![[embeds]]; start/end (1-based, inclusive) return only that line range of the raw body (for a large block).",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "rendered": {"type": "boolean", "description": "Resolve child ![[embeds]] inline in the returned body (default false)"},
                    "start": {"type": "integer", "description": "First body line to return, 1-based inclusive (default: from the start)"},
                    "end": {"type": "integer", "description": "Last body line to return, 1-based inclusive (default: to the end)"}
                },
                "required": ["id"]
            }),
        },
        ToolDef {
            name: "list_blocks",
            description: "List block ids in the vault. Set roots_only=true for just root blocks (top-level pages that nothing transcludes).",
            schema: json!({
                "type": "object",
                "properties": {
                    "roots_only": {"type": "boolean", "description": "Only root blocks (default false = every block)"}
                }
            }),
        },
        ToolDef {
            name: "list_tags",
            description: "List all tags in the vault with how many blocks carry each (tag discovery). Use a returned tag with search's `tags` filter to scope a domain.",
            schema: json!({"type": "object", "properties": {}}),
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
            description: "Overwrite a block's whole body by id (optionally retitling). Read the current body first (get_block) and send the FULL revised text — never a fragment. Tags, lock state and properties are kept; omit or empty `title` to keep the title, send a non-empty one to change it. An edit that empties or guts the block is refused unless force=true. For a small change prefer replace_in_block; to add at the end, append_to_block.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string", "description": "Omit or leave empty to keep the current title; a non-empty value changes it"},
                    "body": {"type": "string"},
                    "force": {"type": "boolean", "description": "Bypass the destructive-update guard for an intentional rewrite (default false)"}
                },
                "required": ["id", "body"]
            }),
        },
        ToolDef {
            name: "replace_in_block",
            description: "Targeted partial edit: replace the exact substring `old` with `new` in a block's body, without resending the whole body. `old` must occur exactly `expect_count` times (default 1) or nothing changes (a stale/ambiguous anchor is a safe no-op — include enough surrounding text to be unique). `new` may be empty to delete the match. Operates on raw Markdown, so ![[embeds]]/[[refs]] are literal text. Title, tags, lock state and properties are preserved.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "old": {"type": "string", "description": "Exact text to find; must occur expect_count times"},
                    "new": {"type": "string", "description": "Replacement text (empty to delete the match)"},
                    "expect_count": {"type": "integer", "description": "Required number of occurrences (default 1)"},
                    "force": {"type": "boolean", "description": "Bypass the destructive-update guard (default false)"}
                },
                "required": ["id", "old"]
            }),
        },
        ToolDef {
            name: "append_to_block",
            description: "Append `text` to the END of a block's body, on a fresh line — to add a line/paragraph/list-item when there is no anchor to target with replace_in_block. Additive only; never removes content. Lead `text` with a newline for a blank-line gap (e.g. before a heading). Title, tags, lock state and properties are preserved.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "text": {"type": "string", "description": "Text to append (starts on a new line)"}
                },
                "required": ["id", "text"]
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
    // For the update_block title boundary: an agent that includes `"title": ""` (or whitespace)
    // almost always means "I have no title to set", not "erase the existing title". Normalising
    // empty/whitespace to `None` makes that a safe *preserve* rather than a silent wipe — title
    // clearing over MCP is deliberately not the accidental path (set a non-empty title to change it).
    let s_nonblank = |key: &str| s(key).filter(|v| !v.trim().is_empty());
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
                    Some(d) => mkb_core::clock::parse_query_date(&d)
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
            // Property presence/absence keys (merge with any has:/missing: operators from the query).
            let prop_keys = |key: &str| -> Vec<String> {
                args.get(key)
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .map(|s| s.trim().to_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            for k in prop_keys("has") {
                if !q.has_prop.contains(&k) {
                    q.has_prop.push(k);
                }
            }
            for k in prop_keys("missing") {
                if !q.lacks_prop.contains(&k) {
                    q.lacks_prop.push(k);
                }
            }
            Request::Search { query: q }
        }
        "get_block" => Request::GetBlockView {
            id: req_id("id")?,
            rendered: args
                .get("rendered")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            start: args
                .get("start")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize),
            end: args.get("end").and_then(|v| v.as_u64()).map(|n| n as usize),
        },
        "list_blocks" => {
            if args
                .get("roots_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                Request::ListRoots
            } else {
                Request::ListBlocks
            }
        }
        "list_tags" => Request::ListTags,
        "create_block" => Request::CreateBlock {
            title: s("title"),
            body: req_s("body")?,
        },
        "update_block" => Request::UpdateBlock {
            id: req_id("id")?,
            title: s_nonblank("title"),
            body: req_s("body")?,
            force: args.get("force").and_then(|v| v.as_bool()).unwrap_or(false),
            // The MCP path is non-interactive: agents don't hold an editor session to pin a base
            // version against, and use replace_in_block for safe partial edits. The optimistic-
            // concurrency guard is the human editor's feature, so update_block here is unguarded.
            base_version: None,
        },
        "replace_in_block" => Request::ReplaceInBlock {
            id: req_id("id")?,
            old: req_s("old")?,
            new: s("new").unwrap_or_default(),
            expect_count: args
                .get("expect_count")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(1),
            force: args.get("force").and_then(|v| v.as_bool()).unwrap_or(false),
        },
        "append_to_block" => Request::AppendToBlock {
            id: req_id("id")?,
            text: req_s("text")?,
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
        other => return Err(format!("unknown tool: {other}")),
    })
}

/// Render a daemon [`Response`] as MCP tool result text (JSON for structured payloads).
pub fn format_response(resp: &Response) -> Result<String, String> {
    match resp {
        Response::Error { message } => Err(message.clone()),
        Response::Pong => Ok("pong".to_string()),
        Response::Ok => Ok("ok".to_string()),
        // The MCP path sends no base_version, so a guarded update never conflicts here; a plain
        // applied update just reports ok.
        Response::Updated { .. } => Ok("ok".to_string()),
        Response::Conflict { version, .. } => Err(format!(
            "conflict: the block changed (current version {version})"
        )),
        Response::Text(t) => Ok(t.clone().unwrap_or_else(|| "(not found)".to_string())),
        Response::BlockId(id) => Ok(id.to_string()),
        Response::Path(p) => Ok(p.clone()),
        Response::Linked(o) => Ok(match o {
            mkb_core::LinkOutcome::Reference => "linked (reference)".to_string(),
            mkb_core::LinkOutcome::Transclusion => "linked (transclusion)".to_string(),
            mkb_core::LinkOutcome::DowngradedToReference => {
                "linked as a plain reference (an embed would have created a transclusion cycle)"
                    .to_string()
            }
        }),
        Response::Ids(v) => to_json(v),
        Response::Names(n) => to_json(n),
        Response::Hits(h) => to_json(h),
        Response::Block(b) => to_json(b),
        Response::Page(p) => to_json(p),
        // The MCP server never heartbeats, but the match must stay exhaustive.
        Response::Heartbeat { generation } => Ok(generation.to_string()),
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
        let defs = all_tool_definitions();
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
    fn lean_surface_is_default_full_surface_is_opt_in() {
        let lean: std::collections::HashSet<&str> =
            tool_definitions_for(false).iter().map(|t| t.name).collect();
        let full: std::collections::HashSet<&str> =
            tool_definitions_for(true).iter().map(|t| t.name).collect();
        // Advanced power tools are hidden by default, shown with MKB_MCP_TOOLS=full.
        for adv in ADVANCED_TOOLS {
            assert!(!lean.contains(adv), "{adv} must be hidden by default");
            assert!(full.contains(adv), "{adv} must appear in the full surface");
        }
        // Core read/write/search stays in the lean surface.
        for core in [
            "search",
            "get_block",
            "list_blocks",
            "update_block",
            "create_block",
        ] {
            assert!(lean.contains(core), "{core} must be in the lean surface");
        }
        // Diagnostics and folded-in read primitives are CLI-only — gone from MCP entirely.
        for gone in [
            "graph",
            "stats",
            "conflicts",
            "rebuild",
            "render_block",
            "get_block_lines",
            "backlinks",
            "links_from",
            "list_roots",
        ] {
            assert!(!full.contains(gone), "{gone} must not be an MCP tool");
        }
    }

    #[test]
    fn every_tool_name_builds_a_request() {
        let id = BlockId::generate().to_string();
        let args = json!({
            "query": "q", "id": id, "source_id": id, "target_id": id, "parent_id": id,
            "child_id": id, "title": "T", "body": "b", "embed": true, "tags": ["x"],
            "props": [{"key": "source", "value": "git"}], "keys": ["source"],
            "old": "b", "new": "c", "text": "more", "start": 1, "end": 2,
            "rendered": true, "roots_only": true
        });
        for d in all_tool_definitions() {
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
    fn update_block_title_empty_string_preserves_not_clears() {
        // An agent that sends `"title": ""` must NOT wipe the title — it normalises to None
        // (preserve). A non-empty title still changes it; omitting it preserves.
        let id = BlockId::generate().to_string();
        for empty in ["", "   "] {
            let args = json!({"id": id, "title": empty, "body": "new body"});
            match build_request("update_block", &args).unwrap() {
                Request::UpdateBlock { title, .. } => assert_eq!(title, None, "empty {empty:?}"),
                _ => panic!("expected update_block"),
            }
        }
        let args = json!({"id": id, "title": "Real", "body": "b"});
        match build_request("update_block", &args).unwrap() {
            Request::UpdateBlock { title, .. } => assert_eq!(title.as_deref(), Some("Real")),
            _ => panic!("expected update_block"),
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
    fn search_maps_has_missing_from_args_and_operators() {
        // `has:` operator in the query + a `missing` array arg both land on the query.
        let args = json!({"query": "has:Source", "missing": ["verified", "Confidence"]});
        match build_request("search", &args).unwrap() {
            Request::Search { query } => {
                assert_eq!(query.has_prop, vec!["source"]); // operator, lowercased
                assert_eq!(query.lacks_prop, vec!["verified", "confidence"]);
            }
            _ => panic!("expected search"),
        }
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
