//! MCP tool catalog and dispatch.
//!
//! # Tool organization
//!
//! Tools are split into categories matching the Python server:
//!
//! - **Read tools** (9): pure SQLite, sub-millisecond. The performance win.
//! - **BBT tools** (3): BibTeX/bibliography export, still need BBT JSON-RPC.
//! - **Write tools** (14): Zotero Web API, same latency as Python.
//! - **PDF tools** (3): network-bound, tokio async.
//!
//! This module defines the tool catalog (for `tools/list`) and the dispatch
//! function (for `tools/call`).

pub mod read;
pub mod format;

use serde_json::{json, Value};

use crate::protocol::{ToolCallResult, ToolDefinition};
use crate::server::ServerContext;

/// Build the complete tool catalog for `tools/list`.
///
/// Each tool definition includes its name, description, and JSON Schema
/// for input parameters. Claude uses this to know what tools are available.
pub fn tool_catalog() -> Vec<ToolDefinition> {
    let mut tools = Vec::new();

    // --- Read tools (pure SQLite) ---
    tools.push(tool("zotero_status", "Check Zotero library statistics (item count, collection count).", json!({
        "type": "object",
        "properties": {},
    })));

    tools.push(tool("zotero_search", "Search Zotero library items by text query. Returns matching items with citekeys.", json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Search query (matches title, DOI, abstract)" },
            "limit": { "type": "integer", "default": 50, "description": "Maximum number of results" }
        },
        "required": ["query"]
    })));

    tools.push(tool("zotero_get_item", "Get full metadata for an item by its citation key.", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string", "description": "Citation key (e.g., 'demilloHintsTestData1978')" }
        },
        "required": ["citekey"]
    })));

    tools.push(tool("zotero_get_notes", "Get all notes for an item by its citation key.", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string", "description": "Citation key" }
        },
        "required": ["citekey"]
    })));

    tools.push(tool("zotero_get_pdf_path", "Get filesystem path(s) to PDF attachments for an item.", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string", "description": "Citation key" }
        },
        "required": ["citekey"]
    })));

    tools.push(tool("zotero_list_attachments", "List all attachments for an item.", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string", "description": "Citation key" }
        },
        "required": ["citekey"]
    })));

    tools.push(tool("zotero_get_collections", "List all collections in the library with hierarchy.", json!({
        "type": "object",
        "properties": {},
    })));

    tools.push(tool("zotero_get_collection_items", "Get items in a specific collection by its key.", json!({
        "type": "object",
        "properties": {
            "collection_key": { "type": "string", "description": "Collection key (8-char)" },
            "limit": { "type": "integer", "default": 100, "description": "Maximum number of items" }
        },
        "required": ["collection_key"]
    })));

    tools.push(tool("zotero_get_recent", "Get recently modified items.", json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer", "default": 20, "description": "Maximum number of items" }
        },
    })));

    tools
}

fn tool(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema,
    }
}

/// Dispatch a tool call to the appropriate handler.
pub fn handle_tool_call(name: &str, args: &Value, ctx: &ServerContext) -> ToolCallResult {
    match name {
        // Read tools (pure SQLite)
        "zotero_status" => read::zotero_status(ctx),
        "zotero_search" => read::zotero_search(args, ctx),
        "zotero_get_item" => read::zotero_get_item(args, ctx),
        "zotero_get_notes" => read::zotero_get_notes(args, ctx),
        "zotero_get_pdf_path" => read::zotero_get_pdf_path(args, ctx),
        "zotero_list_attachments" => read::zotero_list_attachments(args, ctx),
        "zotero_get_collections" => read::zotero_get_collections(ctx),
        "zotero_get_collection_items" => read::zotero_get_collection_items(args, ctx),
        "zotero_get_recent" => read::zotero_get_recent(args, ctx),

        _ => ToolCallResult::error(format!("Unknown tool: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_expected_tools() {
        let catalog = tool_catalog();
        let names: Vec<&str> = catalog.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"zotero_status"));
        assert!(names.contains(&"zotero_search"));
        assert!(names.contains(&"zotero_get_item"));
        assert!(names.contains(&"zotero_get_collections"));
        assert!(names.contains(&"zotero_get_recent"));
    }

    #[test]
    fn catalog_tools_have_input_schema() {
        let catalog = tool_catalog();
        for tool in &catalog {
            assert!(tool.input_schema.is_object(), "Tool {} missing input_schema", tool.name);
        }
    }

    #[test]
    fn unknown_tool_returns_error() {
        let ctx = ServerContext {
            db: crate::db::DbPool::empty(),
            config: crate::config::Config {
                zotero_sqlite_path: "/tmp/z.sqlite".into(),
                zotero_storage_path: "/tmp/storage".into(),
                bbt_migrated_path: "/tmp/bbt".into(),
                zotero_api_key: None,
                zotero_library_id: "1".into(),
                zotero_library_type: "user".into(),
                bbt_url: "http://localhost:23119".into(),
                log_level: crate::config::LogLevel::Quiet,
            },
        };
        let result = handle_tool_call("nonexistent", &json!({}), &ctx);
        assert_eq!(result.is_error, Some(true));
    }
}
