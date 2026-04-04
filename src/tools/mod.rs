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
pub mod write;
pub mod format;
pub mod pdf;

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

    // --- BBT RPC tools (BibTeX/bibliography export) ---
    tools.push(tool("zotero_get_bibtex", "Export items as BibTeX or BibLaTeX by citation keys.", json!({
        "type": "object",
        "properties": {
            "citekeys": { "type": "array", "items": { "type": "string" }, "description": "Citation keys to export" },
            "format": { "type": "string", "default": "Better BibTeX", "description": "Export format: 'Better BibTeX' or 'Better BibLaTeX'" }
        },
        "required": ["citekeys"]
    })));

    tools.push(tool("zotero_get_bibliography", "Generate formatted bibliography for citation keys.", json!({
        "type": "object",
        "properties": {
            "citekeys": { "type": "array", "items": { "type": "string" }, "description": "Citation keys" },
            "style": { "type": "string", "default": "http://www.zotero.org/styles/apa", "description": "CSL style URL" }
        },
        "required": ["citekeys"]
    })));

    tools.push(tool("zotero_export_bibtex", "Export a collection or item list as BibTeX/BibLaTeX.", json!({
        "type": "object",
        "properties": {
            "collection_key": { "type": "string", "description": "Collection key to export" },
            "item_keys": { "type": "array", "items": { "type": "string" }, "description": "Item keys to export" },
            "format": { "type": "string", "default": "Better BibLaTeX" }
        }
    })));

    // --- Write tools (Zotero Web API) ---
    tools.push(tool("zotero_create_item", "Create a new Zotero item.", json!({
        "type": "object",
        "properties": {
            "item_type": { "type": "string", "description": "Item type (journalArticle, book, etc.)" },
            "title": { "type": "string" },
            "creators": { "type": "array", "items": { "type": "object" } },
            "fields": { "type": "object", "description": "Additional fields (date, DOI, etc.)" },
            "collection_keys": { "type": "array", "items": { "type": "string" } },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["item_type", "title"]
    })));

    tools.push(tool("zotero_update_item", "Update metadata fields of an existing item.", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string" },
            "fields": { "type": "object" },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["citekey"]
    })));

    tools.push(tool("zotero_add_tags", "Add tags to an item (preserves existing tags).", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["citekey", "tags"]
    })));

    tools.push(tool("zotero_add_note", "Add a note to an item.", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string" },
            "content": { "type": "string", "description": "Note content (markdown or HTML)" },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["citekey", "content"]
    })));

    tools.push(tool("zotero_create_collection", "Create a new collection.", json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "parent_key": { "type": "string", "description": "Parent collection key (for sub-collections)" }
        },
        "required": ["name"]
    })));

    tools.push(tool("zotero_add_to_collection", "Add an item to a collection.", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string" },
            "item_key": { "type": "string" },
            "collection_key": { "type": "string" }
        },
        "required": ["collection_key"]
    })));

    tools.push(tool("zotero_remove_from_collection", "Remove an item from a collection.", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string" },
            "item_key": { "type": "string" },
            "collection_key": { "type": "string" }
        },
        "required": ["collection_key"]
    })));

    tools.push(tool("zotero_delete_item", "Delete an item permanently.", json!({
        "type": "object",
        "properties": {
            "citekey": { "type": "string" },
            "item_key": { "type": "string" }
        }
    })));

    tools.push(tool("zotero_merge_items", "Merge two duplicate items (keeps one, deletes the other).", json!({
        "type": "object",
        "properties": {
            "keep_citekey": { "type": "string", "description": "Citekey of item to keep" },
            "delete_citekey": { "type": "string", "description": "Citekey of item to delete" }
        },
        "required": ["keep_citekey", "delete_citekey"]
    })));

    tools.push(tool("zotero_attach_pdf", "Download a PDF and attach it to an item.", json!({
        "type": "object",
        "properties": {
            "item_key": { "type": "string" },
            "pdf_url": { "type": "string" },
            "title": { "type": "string" }
        },
        "required": ["item_key", "pdf_url"]
    })));

    tools.push(tool("zotero_fetch_missing_pdfs", "Find and attach PDFs for items missing them (9-source resolver).", json!({
        "type": "object",
        "properties": {
            "collection_key": { "type": "string" },
            "limit": { "type": "integer", "default": 50 },
            "dry_run": { "type": "boolean", "default": false }
        }
    })));

    tools.push(tool("zotero_archive_report", "Archive an internal report (create item + attach PDF).", json!({
        "type": "object",
        "properties": {
            "pdf_path": { "type": "string" },
            "title": { "type": "string" },
            "authors": { "type": "array", "items": { "type": "string" } },
            "report_number": { "type": "string" }
        },
        "required": ["pdf_path", "title", "authors", "report_number"]
    })));

    tools.push(tool("zotero_archive_webpage", "Archive a web page as PDF and attach to Zotero.", json!({
        "type": "object",
        "properties": {
            "url": { "type": "string" },
            "item_key": { "type": "string" },
            "title": { "type": "string" }
        },
        "required": ["url"]
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
///
/// Read tools are pure SQLite (sub-millisecond). BBT and write tools
/// make network calls (same latency as Python server).
pub fn handle_tool_call(name: &str, args: &Value, ctx: &ServerContext) -> ToolCallResult {
    match name {
        // Read tools (pure SQLite, <1ms)
        "zotero_status" => read::zotero_status(ctx),
        "zotero_search" => read::zotero_search(args, ctx),
        "zotero_get_item" => read::zotero_get_item(args, ctx),
        "zotero_get_notes" => read::zotero_get_notes(args, ctx),
        "zotero_get_pdf_path" => read::zotero_get_pdf_path(args, ctx),
        "zotero_list_attachments" => read::zotero_list_attachments(args, ctx),
        "zotero_get_collections" => read::zotero_get_collections(ctx),
        "zotero_get_collection_items" => read::zotero_get_collection_items(args, ctx),
        "zotero_get_recent" => read::zotero_get_recent(args, ctx),

        // BBT RPC tools (BibTeX/bibliography, requires Zotero running)
        "zotero_get_bibtex" => write::zotero_get_bibtex(args, ctx),
        "zotero_get_bibliography" => write::zotero_get_bibliography(args, ctx),
        "zotero_export_bibtex" => write::zotero_export_bibtex(args, ctx),

        // Write tools (Zotero Web API, requires API key)
        "zotero_create_item" => write::zotero_create_item(args, ctx),
        "zotero_update_item" => write::zotero_update_item(args, ctx),
        "zotero_add_tags" => write::zotero_add_tags(args, ctx),
        "zotero_add_note" => write::zotero_add_note(args, ctx),
        "zotero_create_collection" => write::zotero_create_collection(args, ctx),
        "zotero_add_to_collection" | "zotero_add_item_to_collection" => write::zotero_add_to_collection(args, ctx),
        "zotero_remove_from_collection" => write::zotero_remove_from_collection(args, ctx),
        "zotero_delete_item" => write::zotero_delete_item(args, ctx),
        "zotero_merge_items" => write::zotero_merge_items(args, ctx),
        "zotero_attach_pdf" => write::zotero_attach_pdf(args, ctx),
        "zotero_fetch_missing_pdfs" => write::zotero_fetch_missing_pdfs(args, ctx),
        "zotero_archive_report" => write::zotero_archive_report(args, ctx),
        "zotero_archive_webpage" => write::zotero_archive_webpage(args, ctx),

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
