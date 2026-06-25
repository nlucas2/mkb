//! Minimal MCP server: JSON-RPC 2.0 over newline-delimited stdio.
//!
//! Implements just enough of MCP to be a useful tool provider: `initialize`, `tools/list`,
//! `tools/call`, and `ping`. Tool calls are forwarded to the daemon through
//! [`mkb_protocol::Client`], so the MCP server stays a thin client of the one shared
//! `Service`.

use mkb_protocol::Client;
use serde_json::{json, Value};

use crate::tools::{build_request, format_response, tool_definitions};

/// The MCP protocol version this server speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Outcome of handling a single JSON-RPC message.
pub enum Outcome {
    /// Send this JSON-RPC response.
    Reply(Value),
    /// A notification (or otherwise id-less message) — send nothing.
    Silent,
}

/// Handle one parsed JSON-RPC message against the daemon `client`.
pub fn handle_message(client: &Client, msg: &Value) -> Outcome {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Notifications have no id and expect no reply.
    if id.is_none() {
        return Outcome::Silent;
    }
    let id = id.unwrap();

    match method {
        "initialize" => Outcome::Reply(ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mkb-mcp", "version": env!("CARGO_PKG_VERSION")}
            }),
        )),
        "ping" => Outcome::Reply(ok(id, json!({}))),
        "tools/list" => {
            let tools: Vec<Value> = tool_definitions()
                .into_iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.schema
                    })
                })
                .collect();
            Outcome::Reply(ok(id, json!({ "tools": tools })))
        }
        "tools/call" => Outcome::Reply(handle_tool_call(client, id, msg)),
        other => Outcome::Reply(error(id, -32601, &format!("method not found: {other}"))),
    }
}

fn handle_tool_call(client: &Client, id: Value, msg: &Value) -> Value {
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let name = match params.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return error(id, -32602, "tools/call requires a tool name"),
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let request = match build_request(name, &args) {
        Ok(r) => r,
        Err(e) => return tool_error(id, &e),
    };
    let response = match client.call(&request) {
        Ok(r) => r,
        Err(e) => return tool_error(id, &format!("daemon error: {e}")),
    };
    match format_response(&response) {
        Ok(text) => ok(
            id,
            json!({
                "content": [{"type": "text", "text": text}],
                "isError": false
            }),
        ),
        Err(message) => tool_error(id, &message),
    }
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// A tool-level error is reported as a successful result with `isError: true`, per MCP, so
/// the model sees the message rather than a transport failure.
fn tool_error(id: Value, message: &str) -> Value {
    ok(
        id,
        json!({
            "content": [{"type": "text", "text": message}],
            "isError": true
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> Client {
        // A client pointed at a nonexistent socket: fine for tests that don't call tools.
        Client::new("/nonexistent/mkb-test.sock")
    }

    #[test]
    fn initialize_returns_protocol_and_capabilities() {
        let msg = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
        match handle_message(&client(), &msg) {
            Outcome::Reply(r) => {
                assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
                assert!(r["result"]["capabilities"]["tools"].is_object());
                assert_eq!(r["result"]["serverInfo"]["name"], "mkb-mcp");
            }
            Outcome::Silent => panic!("expected a reply"),
        }
    }

    #[test]
    fn tools_list_includes_search_with_schema() {
        let msg = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        match handle_message(&client(), &msg) {
            Outcome::Reply(r) => {
                let tools = r["result"]["tools"].as_array().unwrap();
                let search = tools.iter().find(|t| t["name"] == "search").unwrap();
                assert!(search["inputSchema"]["properties"]["query"].is_object());
            }
            Outcome::Silent => panic!("expected a reply"),
        }
    }

    #[test]
    fn notifications_are_silent() {
        let msg = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(matches!(handle_message(&client(), &msg), Outcome::Silent));
    }

    #[test]
    fn unknown_method_is_jsonrpc_error() {
        let msg = json!({"jsonrpc": "2.0", "id": 9, "method": "bogus"});
        match handle_message(&client(), &msg) {
            Outcome::Reply(r) => assert_eq!(r["error"]["code"], -32601),
            Outcome::Silent => panic!("expected error reply"),
        }
    }

    #[test]
    fn tool_call_with_unknown_tool_is_tool_error() {
        let msg = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "nope", "arguments": {}}
        });
        match handle_message(&client(), &msg) {
            // Unknown tool fails at build time → reported as isError result, not transport error.
            Outcome::Reply(r) => assert_eq!(r["result"]["isError"], true),
            Outcome::Silent => panic!("expected reply"),
        }
    }
}
