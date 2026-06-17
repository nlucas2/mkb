//! `mdkb-mcp` — the mdkb MCP server (scaffold).
//!
//! A **thin client**: it will translate MCP tool calls into core/daemon API calls and
//! contain transport glue only — never a second copy of core behavior. See `AGENTS.md`.

fn main() {
    println!("mdkb-mcp (scaffold) — mdkb-core v{}", mdkb_core::VERSION);
}
