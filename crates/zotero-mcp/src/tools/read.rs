//! Read-only MCP tools — pure SQLite, sub-millisecond.
//!
//! These are the tools that benefit from the Rust rewrite. Each one
//! reads directly from `zotero.sqlite` and `better-bibtex.migrated`,
//! bypassing the BBT JSON-RPC bottleneck entirely.
//!
//! # Citekey resolution pattern
//!
//! Most tools accept a `citekey` parameter. Resolution:
//! 1. Look up citekey in `bbt.migrated` → get item_key
//! 2. Look up item_key in `zotero.sqlite` → get full item
//!
//! Both are SQLite reads, total ~0.1ms.

use serde_json::Value;
use std::path::PathBuf;

use crate::protocol::ToolCallResult;
use crate::server::ServerContext;
use crate::tools::format::{format_item_summary, html_to_text};

use super::resolve_citekey;

// ---------------------------------------------------------------------------
// zotero_status
// ---------------------------------------------------------------------------

pub fn zotero_status(ctx: &ServerContext) -> ToolCallResult {
    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    let items = zdb.item_count().unwrap_or(0);
    let collections = zdb.collection_count().unwrap_or(0);
    let bbt_status = if ctx.db.bbt.is_some() {
        "connected"
    } else {
        "unavailable"
    };
    let write_status = if ctx.config.has_write_access() {
        "enabled"
    } else {
        "disabled (no API key)"
    };

    ToolCallResult::text(format!(
        "Zotero MCP (Rust)\n\
         Items: {items}\n\
         Collections: {collections}\n\
         BBT database: {bbt_status}\n\
         Write access: {write_status}\n\
         Version: {}",
        env!("CARGO_PKG_VERSION")
    ))
}

// ---------------------------------------------------------------------------
// zotero_search
// ---------------------------------------------------------------------------

pub fn zotero_search(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let query = match args.get("query").and_then(|v| v.as_str()) {
        Some(q) => q,
        None => return ToolCallResult::error("Missing required parameter: query".into()),
    };
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    let results = match zdb.search_items(query, limit) {
        Ok(r) => r,
        Err(e) => return ToolCallResult::error(format!("Search failed: {e}")),
    };

    if results.is_empty() {
        return ToolCallResult::text(format!("No items found for query: {query}"));
    }

    let mut output = format!("Found {} item(s) for '{query}':\n", results.len());

    for (_item_id, item_key) in &results {
        if let Ok(Some(item)) = zdb.item_by_key(item_key) {
            let citekey = ctx.db.citekey_for_item_key(item_key);
            output.push('\n');
            output.push_str(&format_item_summary(&item, citekey.as_deref()));
            output.push_str("\n\n---\n");
        }
    }

    ToolCallResult::text(output)
}

// ---------------------------------------------------------------------------
// zotero_get_item
// ---------------------------------------------------------------------------

pub fn zotero_get_item(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let citekey = match args.get("citekey").and_then(|v| v.as_str()) {
        Some(ck) => ck,
        None => return ToolCallResult::error("Missing required parameter: citekey".into()),
    };

    let item_key = match resolve_citekey(ctx, citekey) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(e),
    };

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    match zdb.item_by_key(&item_key) {
        Ok(Some(item)) => {
            let mut output = format_item_summary(&item, Some(citekey));
            if let Some(abs) = &item.abstract_note {
                output.push_str(&format!("\n\nAbstract: {abs}"));
            }
            if !item.tags.is_empty() {
                output.push_str(&format!("\nTags: {}", item.tags.join(", ")));
            }
            ToolCallResult::text(output)
        }
        Ok(None) => ToolCallResult::error(format!("Item not found: {item_key}")),
        Err(e) => ToolCallResult::error(format!("Database error: {e}")),
    }
}

// ---------------------------------------------------------------------------
// zotero_get_notes
// ---------------------------------------------------------------------------

pub fn zotero_get_notes(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let citekey = match args.get("citekey").and_then(|v| v.as_str()) {
        Some(ck) => ck,
        None => return ToolCallResult::error("Missing required parameter: citekey".into()),
    };

    let item_key = match resolve_citekey(ctx, citekey) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(e),
    };

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    // Get item_id from item_key
    let item = match zdb.item_by_key(&item_key) {
        Ok(Some(item)) => item,
        Ok(None) => return ToolCallResult::error(format!("Item not found: {item_key}")),
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    match zdb.item_notes(item.item_id) {
        Ok(notes) if notes.is_empty() => ToolCallResult::text(format!("No notes for {citekey}")),
        Ok(notes) => {
            let mut output = format!("{} note(s) for {citekey}:\n\n", notes.len());
            for (i, note) in notes.iter().enumerate() {
                output.push_str(&format!(
                    "--- Note {} ---\n{}\n\n",
                    i + 1,
                    html_to_text(note)
                ));
            }
            ToolCallResult::text(output)
        }
        Err(e) => ToolCallResult::error(format!("Error reading notes: {e}")),
    }
}

// ---------------------------------------------------------------------------
// zotero_get_pdf_path
// ---------------------------------------------------------------------------

pub fn zotero_get_pdf_path(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let citekey = match args.get("citekey").and_then(|v| v.as_str()) {
        Some(ck) => ck,
        None => return ToolCallResult::error("Missing required parameter: citekey".into()),
    };

    let item_key = match resolve_citekey(ctx, citekey) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(e),
    };

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    let item = match zdb.item_by_key(&item_key) {
        Ok(Some(item)) => item,
        Ok(None) => return ToolCallResult::error(format!("Item not found: {item_key}")),
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    let attachments = match zdb.item_attachments(item.item_id) {
        Ok(a) => a,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    let pdf_paths: Vec<String> = attachments
        .iter()
        .filter(|a| a.content_type == "application/pdf")
        .filter_map(|a| {
            a.path.as_ref().map(|p| {
                if let Some(filename) = p.strip_prefix("storage:") {
                    // Resolve relative storage path
                    // Zotero stores PDFs as storage/{parent_item_key}/{filename}
                    let full_path: PathBuf = [
                        ctx.config.zotero_storage_path.to_str().unwrap_or(""),
                        &a.item_key,
                        filename,
                    ]
                    .iter()
                    .collect();
                    full_path.to_string_lossy().to_string()
                } else {
                    p.clone()
                }
            })
        })
        .collect();

    if pdf_paths.is_empty() {
        ToolCallResult::text(format!("No PDF attachments for {citekey}"))
    } else {
        ToolCallResult::text(pdf_paths.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// zotero_list_attachments
// ---------------------------------------------------------------------------

pub fn zotero_list_attachments(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let citekey = match args.get("citekey").and_then(|v| v.as_str()) {
        Some(ck) => ck,
        None => return ToolCallResult::error("Missing required parameter: citekey".into()),
    };

    let item_key = match resolve_citekey(ctx, citekey) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(e),
    };

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    let item = match zdb.item_by_key(&item_key) {
        Ok(Some(item)) => item,
        Ok(None) => return ToolCallResult::error(format!("Item not found: {item_key}")),
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    match zdb.item_attachments(item.item_id) {
        Ok(attachments) if attachments.is_empty() => {
            ToolCallResult::text(format!("No attachments for {citekey}"))
        }
        Ok(attachments) => {
            let mut output = format!("{} attachment(s) for {citekey}:\n\n", attachments.len());
            for att in &attachments {
                let title = att.title.as_deref().unwrap_or("(untitled)");
                let path = att.path.as_deref().unwrap_or("(no path)");
                output.push_str(&format!("- [{title}] {}\n  {path}\n", att.content_type));
            }
            ToolCallResult::text(output)
        }
        Err(e) => ToolCallResult::error(format!("Error listing attachments: {e}")),
    }
}

// ---------------------------------------------------------------------------
// zotero_get_collections
// ---------------------------------------------------------------------------

pub fn zotero_get_collections(ctx: &ServerContext) -> ToolCallResult {
    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    match zdb.collections() {
        Ok(collections) => {
            let mut output = format!("{} collection(s):\n\n", collections.len());
            for coll in &collections {
                let parent = coll.parent_key.as_deref().unwrap_or("-");
                output.push_str(&format!(
                    "- {} (key: {}, parent: {})\n",
                    coll.name, coll.key, parent
                ));
            }
            ToolCallResult::text(output)
        }
        Err(e) => ToolCallResult::error(format!("Error listing collections: {e}")),
    }
}

// ---------------------------------------------------------------------------
// zotero_get_collection_items
// ---------------------------------------------------------------------------

pub fn zotero_get_collection_items(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let collection_key = match args.get("collection_key").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => return ToolCallResult::error("Missing required parameter: collection_key".into()),
    };
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    let items = match zdb.collection_items(collection_key, limit) {
        Ok(i) => i,
        Err(e) => return ToolCallResult::error(format!("Error: {e}")),
    };

    if items.is_empty() {
        return ToolCallResult::text(format!("No items in collection {collection_key}"));
    }

    let mut output = format!("Collection {} ({} item(s)):\n", collection_key, items.len());
    for (_item_id, item_key) in &items {
        if let Ok(Some(item)) = zdb.item_by_key(item_key) {
            let citekey = ctx.db.citekey_for_item_key(item_key);
            output.push('\n');
            output.push_str(&format_item_summary(&item, citekey.as_deref()));
            output.push_str("\n\n---\n");
        }
    }

    ToolCallResult::text(output)
}

// ---------------------------------------------------------------------------
// zotero_get_recent
// ---------------------------------------------------------------------------

pub fn zotero_get_recent(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    let items = match zdb.recent_items(limit) {
        Ok(i) => i,
        Err(e) => return ToolCallResult::error(format!("Error: {e}")),
    };

    if items.is_empty() {
        return ToolCallResult::text("No recent items".into());
    }

    let mut output = format!("{} recent item(s):\n", items.len());
    for (_item_id, item_key) in &items {
        if let Ok(Some(item)) = zdb.item_by_key(item_key) {
            let citekey = ctx.db.citekey_for_item_key(item_key);
            output.push('\n');
            output.push_str(&format_item_summary(&item, citekey.as_deref()));
            output.push_str("\n\n---\n");
        }
    }

    ToolCallResult::text(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::test_ctx;
    use serde_json::json;

    #[test]
    fn status_returns_item_count() {
        let ctx = test_ctx();
        let result = zotero_status(&ctx);
        assert!(result.is_error.is_none());
        let text = &result.content[0].text;
        assert!(text.contains("Items: 2"), "Got: {text}");
        assert!(text.contains("Collections: 1"), "Got: {text}");
    }

    #[test]
    fn search_finds_by_title() {
        let ctx = test_ctx();
        let result = zotero_search(&json!({"query": "Hints", "limit": 10}), &ctx);
        let text = &result.content[0].text;
        assert!(text.contains("demilloHintsTestData1978"), "Got: {text}");
    }

    #[test]
    fn search_no_results() {
        let ctx = test_ctx();
        let result = zotero_search(&json!({"query": "quantum computing", "limit": 10}), &ctx);
        let text = &result.content[0].text;
        assert!(text.contains("No items found"), "Got: {text}");
    }

    #[test]
    fn search_missing_query_returns_error() {
        let ctx = test_ctx();
        let result = zotero_search(&json!({}), &ctx);
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn get_item_by_citekey() {
        let ctx = test_ctx();
        let result = zotero_get_item(&json!({"citekey": "demilloHintsTestData1978"}), &ctx);
        let text = &result.content[0].text;
        assert!(text.contains("Hints on Test Data Selection"), "Got: {text}");
        assert!(text.contains("DeMillo"), "Got: {text}");
    }

    #[test]
    fn get_item_unknown_citekey() {
        let ctx = test_ctx();
        let result = zotero_get_item(&json!({"citekey": "nonexistent2099"}), &ctx);
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn get_notes_found() {
        let ctx = test_ctx();
        let result = zotero_get_notes(&json!({"citekey": "demilloHintsTestData1978"}), &ctx);
        let text = &result.content[0].text;
        assert!(text.contains("foundational paper"), "Got: {text}");
    }

    #[test]
    fn get_collections_lists_all() {
        let ctx = test_ctx();
        let result = zotero_get_collections(&ctx);
        let text = &result.content[0].text;
        assert!(text.contains("Mutation Testing"), "Got: {text}");
    }

    #[test]
    fn get_collection_items_found() {
        let ctx = test_ctx();
        let result =
            zotero_get_collection_items(&json!({"collection_key": "COL00001", "limit": 10}), &ctx);
        let text = &result.content[0].text;
        assert!(text.contains("demilloHintsTestData1978"), "Got: {text}");
    }

    #[test]
    fn get_recent_returns_items() {
        let ctx = test_ctx();
        let result = zotero_get_recent(&json!({"limit": 5}), &ctx);
        let text = &result.content[0].text;
        assert!(text.contains("demilloHintsTestData1978"), "Got: {text}");
    }

    #[test]
    fn get_pdf_path_found() {
        let ctx = test_ctx();
        let result = zotero_get_pdf_path(&json!({"citekey": "demilloHintsTestData1978"}), &ctx);
        let text = &result.content[0].text;
        assert!(text.contains("DeMillo1978.pdf"), "Got: {text}");
    }

    #[test]
    fn list_attachments_empty() {
        let ctx = test_ctx();
        let result = zotero_list_attachments(&json!({"citekey": "artTesting2020"}), &ctx);
        let text = &result.content[0].text;
        assert!(text.contains("No attachments"), "Got: {text}");
    }
}
