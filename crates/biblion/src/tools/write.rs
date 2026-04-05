//! Write MCP tools — Zotero Web API operations.
//!
//! All write tools go through the Zotero Web API (never direct SQLite writes).
//! They require `ZOTERO_API_KEY` to be set.
//!
//! # Latency
//!
//! Write tools have the same latency as the Python server (~200-500ms)
//! because the bottleneck is the Zotero API, not our code. This is expected
//! and acceptable — writes are rare in MCP usage.

use serde_json::{Value, json};

use crate::api::bbt_rpc::BbtRpcClient;
use crate::api::zotero_web::ZoteroWebClient;
use crate::protocol::ToolCallResult;
use crate::server::ServerContext;

/// Get write client. Checks both write-gate flag and API key.
fn get_write_client(ctx: &ServerContext) -> Result<ZoteroWebClient, String> {
    if !ctx.config.writes_enabled {
        return Err("Write tools disabled. Set ZOTERO_MCP_ENABLE_WRITES=true to enable.".into());
    }
    let api_key =
        ctx.config.zotero_api_key.as_deref().ok_or_else(|| {
            "Write access requires ZOTERO_API_KEY environment variable.".to_string()
        })?;
    if let Some(base_url) = &ctx.config.zotero_api_base_url {
        Ok(ZoteroWebClient::with_base_url(api_key, base_url))
    } else {
        Ok(ZoteroWebClient::new(
            api_key,
            &ctx.config.zotero_library_id,
            &ctx.config.zotero_library_type,
        ))
    }
}

use super::resolve_citekey;

/// Resolve an identifier that may be either a Zotero item key (8-char) or a BBT citekey.
///
/// Zotero item keys are 8 alphanumeric characters (e.g., "TQPUXSC2").
/// Anything longer is treated as a citekey and resolved via BBT.
/// We accept item keys without verifying in SQLite because freshly
/// created items may not have synced to the local database yet.
fn resolve_item_key(ctx: &ServerContext, key: &str) -> Result<String, String> {
    // Zotero item keys are exactly 8 chars, alphanumeric
    if key.len() == 8 && key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Ok(key.to_string());
    }
    // Try as citekey
    resolve_citekey(ctx, key)
}

// ---------------------------------------------------------------------------
// BibTeX / Bibliography
// ---------------------------------------------------------------------------

/// Export items as BibTeX/BibLaTeX — native implementation, no BBT needed.
///
/// This reads directly from SQLite and generates BibTeX in <1ms.
/// The Python server routed this through BBT JSON-RPC (~300ms).
pub fn zotero_get_bibtex(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let citekeys: Vec<&str> = match args.get("citekeys").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        None => match args.get("citekey").and_then(|v| v.as_str()) {
            Some(ck) => vec![ck],
            None => return ToolCallResult::error("Missing parameter: citekeys or citekey".into()),
        },
    };
    let format = match args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("bibtex")
    {
        f if f.to_lowercase().contains("biblatex") => "biblatex",
        _ => "bibtex",
    };

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    let mut entries = Vec::new();
    for citekey in &citekeys {
        let item_key = match resolve_citekey(ctx, citekey) {
            Ok(k) => k,
            Err(e) => return ToolCallResult::error(e),
        };
        let item = match zdb.item_by_key(&item_key) {
            Ok(Some(item)) => item,
            Ok(None) => return ToolCallResult::error(format!("Item not found: {item_key}")),
            Err(e) => return ToolCallResult::error(e.to_string()),
        };
        let metadata = zdb.item_metadata(item.item_id).unwrap_or_default();
        entries.push((item, citekey.to_string(), metadata));
    }

    let result = super::bibtex::items_to_bibtex(&entries, format);
    ToolCallResult::text(result)
}

/// Generate formatted bibliography — native for APA/IEEE, BBT fallback for others.
///
/// # Style resolution
///
/// 1. If style is APA or IEEE → native formatting (sub-millisecond, no Zotero needed)
/// 2. If style is anything else → BBT JSON-RPC fallback (requires Zotero running)
///
/// # Reference
///
/// APA implementation follows: <https://apastyle.apa.org/style-grammar-guidelines/references>
/// IEEE implementation follows: <https://ieeeauthorcenter.ieee.org/wp-content/uploads/IEEE-Reference-Guide.pdf>
/// For verification against the reference CSL engine, compare with BBT's output.
pub fn zotero_get_bibliography(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let citekeys: Vec<&str> = match args.get("citekeys").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        None => return ToolCallResult::error("Missing parameter: citekeys".into()),
    };
    let style = args
        .get("style")
        .and_then(|v| v.as_str())
        .unwrap_or("http://www.zotero.org/styles/apa");

    // Try native formatting for supported styles
    if super::bibliography::is_native_style(style) {
        let zdb = match ctx.db.zotero() {
            Ok(db) => db,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };

        let mut items = Vec::new();
        for citekey in &citekeys {
            let item_key = match resolve_citekey(ctx, citekey) {
                Ok(k) => k,
                Err(e) => return ToolCallResult::error(e),
            };
            let item = match zdb.item_by_key(&item_key) {
                Ok(Some(item)) => item,
                Ok(None) => return ToolCallResult::error(format!("Item not found: {item_key}")),
                Err(e) => return ToolCallResult::error(e.to_string()),
            };
            let metadata = zdb.item_metadata(item.item_id).unwrap_or_default();
            items.push((item, metadata));
        }

        let result = super::bibliography::format_bibliography_list(&items, style);
        return ToolCallResult::text(result);
    }

    // Fallback to BBT for unsupported styles (requires Zotero running)
    let bbt = BbtRpcClient::new(&ctx.config.bbt_url);
    match bbt.bibliography(&citekeys, style) {
        Ok(result) => ToolCallResult::text(result),
        Err(e) => ToolCallResult::error(format!(
            "Style '{style}' not supported natively. BBT fallback failed (is Zotero running?): {e}"
        )),
    }
}

/// Export a collection or item list as BibTeX/BibLaTeX — native, no BBT needed.
pub fn zotero_export_bibtex(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let format = match args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("biblatex")
    {
        f if f.to_lowercase().contains("biblatex") => "biblatex",
        _ => "bibtex",
    };

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    // Get item keys from collection or explicit list
    let item_keys: Vec<(i64, String)> =
        if let Some(collection_key) = args.get("collection_key").and_then(|v| v.as_str()) {
            match zdb.collection_items(collection_key, 1000) {
                Ok(items) => items,
                Err(e) => return ToolCallResult::error(e.to_string()),
            }
        } else if let Some(keys) = args.get("item_keys").and_then(|v| v.as_array()) {
            keys.iter()
                .filter_map(|v| v.as_str())
                .filter_map(|key| {
                    zdb.item_by_key(key)
                        .ok()
                        .flatten()
                        .map(|item| (item.item_id, item.item_key))
                })
                .collect()
        } else {
            return ToolCallResult::error("Provide either collection_key or item_keys".into());
        };

    if item_keys.is_empty() {
        return ToolCallResult::text("No items found to export.".into());
    }

    let mut entries = Vec::new();
    for (_item_id, item_key) in &item_keys {
        if let Ok(Some(item)) = zdb.item_by_key(item_key) {
            let citekey = ctx
                .db
                .citekey_for_item_key(item_key)
                .unwrap_or_else(|| item_key.clone());
            let metadata = zdb.item_metadata(item.item_id).unwrap_or_default();
            entries.push((item, citekey, metadata));
        }
    }

    let result = super::bibtex::items_to_bibtex(&entries, format);
    ToolCallResult::text(result)
}

// ---------------------------------------------------------------------------
// Create / Update / Delete items
// ---------------------------------------------------------------------------

pub fn zotero_create_item(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let item_type = match args.get("item_type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return ToolCallResult::error("Missing parameter: item_type".into()),
    };
    let title = match args.get("title").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return ToolCallResult::error("Missing parameter: title".into()),
    };

    // Get template
    let mut template = match client.item_template(item_type) {
        Ok(t) => t,
        Err(e) => return ToolCallResult::error(format!("Failed to get template: {e}")),
    };

    template["title"] = json!(title);

    // Set creators
    if let Some(creators) = args.get("creators").and_then(|v| v.as_array()) {
        template["creators"] = json!(creators);
    }

    // Set additional fields
    if let Some(fields) = args.get("fields").and_then(|v| v.as_object()) {
        for (k, v) in fields {
            template[k] = v.clone();
        }
    }

    // Set collections
    if let Some(colls) = args.get("collection_keys").and_then(|v| v.as_array()) {
        template["collections"] = json!(colls);
    }

    // Set tags
    if let Some(tags) = args.get("tags").and_then(|v| v.as_array()) {
        let tag_objects: Vec<Value> = tags
            .iter()
            .filter_map(|t| t.as_str())
            .map(|t| json!({"tag": t}))
            .collect();
        template["tags"] = json!(tag_objects);
    }

    match client.create_items(&[template]) {
        Ok(result) => ToolCallResult::text(format!(
            "Item created: {}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        )),
        Err(e) => ToolCallResult::error(format!("Failed to create item: {e}")),
    }
}

pub fn zotero_update_item(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let citekey = match args.get("citekey").and_then(|v| v.as_str()) {
        Some(ck) => ck,
        None => return ToolCallResult::error("Missing parameter: citekey".into()),
    };

    let item_key = match resolve_citekey(ctx, citekey) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(e),
    };

    // Fetch current item to get version
    let item = match client.get_item(&item_key) {
        Ok(i) => i,
        Err(e) => return ToolCallResult::error(format!("Failed to fetch item: {e}")),
    };
    let version = item["version"].as_i64().unwrap_or(0) as i32;
    let mut data = item["data"].clone();

    // Apply field updates
    if let Some(fields) = args.get("fields").and_then(|v| v.as_object()) {
        for (k, v) in fields {
            data[k] = v.clone();
        }
    }

    // Apply tag updates
    if let Some(tags) = args.get("tags").and_then(|v| v.as_array()) {
        let tag_objects: Vec<Value> = tags
            .iter()
            .filter_map(|t| t.as_str())
            .map(|t| json!({"tag": t}))
            .collect();
        data["tags"] = json!(tag_objects);
    }

    match client.update_item(&item_key, &data, version) {
        Ok(()) => ToolCallResult::text(format!("Item {citekey} updated.")),
        Err(e) => ToolCallResult::error(format!("Failed to update: {e}")),
    }
}

pub fn zotero_add_tags(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let citekey = match args.get("citekey").and_then(|v| v.as_str()) {
        Some(ck) => ck,
        None => return ToolCallResult::error("Missing parameter: citekey".into()),
    };
    let new_tags: Vec<&str> = match args.get("tags").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        None => return ToolCallResult::error("Missing parameter: tags".into()),
    };

    let item_key = match resolve_citekey(ctx, citekey) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(e),
    };

    let item = match client.get_item(&item_key) {
        Ok(i) => i,
        Err(e) => return ToolCallResult::error(format!("Failed to fetch item: {e}")),
    };
    let version = item["version"].as_i64().unwrap_or(0) as i32;
    let mut data = item["data"].clone();

    // Merge tags (preserve existing, add new)
    let existing: std::collections::HashSet<String> = data["tags"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|t| t["tag"].as_str().map(String::from))
        .collect();

    let mut tags: Vec<Value> = existing.iter().map(|t| json!({"tag": t})).collect();
    for tag in new_tags {
        if !existing.contains(tag) {
            tags.push(json!({"tag": tag}));
        }
    }
    data["tags"] = json!(tags);

    match client.update_item(&item_key, &data, version) {
        Ok(()) => ToolCallResult::text(format!("Tags added to {citekey}.")),
        Err(e) => ToolCallResult::error(format!("Failed to add tags: {e}")),
    }
}

pub fn zotero_add_note(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let citekey = match args.get("citekey").and_then(|v| v.as_str()) {
        Some(ck) => ck,
        None => return ToolCallResult::error("Missing parameter: citekey".into()),
    };
    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return ToolCallResult::error("Missing parameter: content".into()),
    };

    let item_key = match resolve_citekey(ctx, citekey) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(e),
    };

    // Convert markdown to basic HTML (simple replacement)
    let html = if content.contains('<') {
        content.to_string() // Already HTML
    } else {
        format!("<p>{}</p>", content.replace('\n', "</p><p>"))
    };

    let note = json!({
        "itemType": "note",
        "parentItem": item_key,
        "note": html,
        "tags": args.get("tags").and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|t| t.as_str()).map(|t| json!({"tag": t})).collect::<Vec<_>>())
            .unwrap_or_default(),
    });

    match client.create_items(&[note]) {
        Ok(_) => ToolCallResult::text(format!("Note added to {citekey}.")),
        Err(e) => ToolCallResult::error(format!("Failed to add note: {e}")),
    }
}

pub fn zotero_create_collection(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return ToolCallResult::error("Missing parameter: name".into()),
    };

    // Check if collection already exists (via local SQLite)
    if let Ok(zdb) = ctx.db.zotero()
        && let Ok(colls) = zdb.collections()
        && let Some(existing) = colls.iter().find(|c| c.name == name)
    {
        return ToolCallResult::text(format!(
            "Collection '{}' already exists (key: {}).",
            name, existing.key
        ));
    }

    let mut coll = json!({"name": name});
    if let Some(parent_key) = args.get("parent_key").and_then(|v| v.as_str()) {
        coll["parentCollection"] = json!(parent_key);
    }

    match client.create_collections(&[coll]) {
        Ok(result) => ToolCallResult::text(format!(
            "Collection '{}' created: {}",
            name,
            serde_json::to_string_pretty(&result).unwrap_or_default()
        )),
        Err(e) => ToolCallResult::error(format!("Failed to create collection: {e}")),
    }
}

pub fn zotero_add_to_collection(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let collection_key = match args.get("collection_key").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => return ToolCallResult::error("Missing parameter: collection_key".into()),
    };

    // Accept either citekey or item_key
    let item_key = if let Some(ck) = args.get("citekey").and_then(|v| v.as_str()) {
        match resolve_citekey(ctx, ck) {
            Ok(k) => k,
            Err(e) => return ToolCallResult::error(e),
        }
    } else if let Some(ik) = args.get("item_key").and_then(|v| v.as_str()) {
        ik.to_string()
    } else {
        return ToolCallResult::error("Missing parameter: citekey or item_key".into());
    };

    let item = match client.get_item(&item_key) {
        Ok(i) => i,
        Err(e) => return ToolCallResult::error(format!("Failed to fetch item: {e}")),
    };
    let version = item["version"].as_i64().unwrap_or(0) as i32;
    let mut data = item["data"].clone();

    // Add collection if not already present
    let mut collections: Vec<String> = data["collections"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|c| c.as_str().map(String::from))
        .collect();

    if !collections.contains(&collection_key.to_string()) {
        collections.push(collection_key.to_string());
        data["collections"] = json!(collections);
        match client.update_item(&item_key, &data, version) {
            Ok(()) => ToolCallResult::text(format!(
                "Item {item_key} added to collection {collection_key}."
            )),
            Err(e) => ToolCallResult::error(format!("Failed: {e}")),
        }
    } else {
        ToolCallResult::text(format!(
            "Item {item_key} already in collection {collection_key}."
        ))
    }
}

pub fn zotero_remove_from_collection(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let collection_key = match args.get("collection_key").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => return ToolCallResult::error("Missing parameter: collection_key".into()),
    };

    let item_key = if let Some(ck) = args.get("citekey").and_then(|v| v.as_str()) {
        match resolve_citekey(ctx, ck) {
            Ok(k) => k,
            Err(e) => return ToolCallResult::error(e),
        }
    } else if let Some(ik) = args.get("item_key").and_then(|v| v.as_str()) {
        ik.to_string()
    } else {
        return ToolCallResult::error("Missing parameter: citekey or item_key".into());
    };

    let item = match client.get_item(&item_key) {
        Ok(i) => i,
        Err(e) => return ToolCallResult::error(format!("Failed to fetch item: {e}")),
    };
    let version = item["version"].as_i64().unwrap_or(0) as i32;
    let mut data = item["data"].clone();

    let collections: Vec<String> = data["collections"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|c| c.as_str().map(String::from))
        .filter(|c| c != collection_key)
        .collect();

    data["collections"] = json!(collections);
    match client.update_item(&item_key, &data, version) {
        Ok(()) => ToolCallResult::text(format!("Item removed from collection {collection_key}.")),
        Err(e) => ToolCallResult::error(format!("Failed: {e}")),
    }
}

pub fn zotero_delete_item(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let item_key = if let Some(ck) = args.get("citekey").and_then(|v| v.as_str()) {
        match resolve_citekey(ctx, ck) {
            Ok(k) => k,
            Err(e) => return ToolCallResult::error(e),
        }
    } else if let Some(ik) = args.get("item_key").and_then(|v| v.as_str()) {
        ik.to_string()
    } else {
        return ToolCallResult::error("Missing parameter: citekey or item_key".into());
    };

    let item = match client.get_item(&item_key) {
        Ok(i) => i,
        Err(e) => return ToolCallResult::error(format!("Failed to fetch item: {e}")),
    };
    let version = item["version"].as_i64().unwrap_or(0) as i32;

    match client.delete_item(&item_key, version) {
        Ok(()) => ToolCallResult::text(format!("Item {item_key} deleted permanently.")),
        Err(e) => ToolCallResult::error(format!("Failed to delete: {e}")),
    }
}

pub fn zotero_merge_items(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let keep_ck = match args.get("keep_citekey").and_then(|v| v.as_str()) {
        Some(ck) => ck,
        None => return ToolCallResult::error("Missing parameter: keep_citekey".into()),
    };
    let delete_ck = match args.get("delete_citekey").and_then(|v| v.as_str()) {
        Some(ck) => ck,
        None => return ToolCallResult::error("Missing parameter: delete_citekey".into()),
    };

    let keep_key = match resolve_citekey(ctx, keep_ck) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(e),
    };
    let delete_key = match resolve_citekey(ctx, delete_ck) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(e),
    };

    // Fetch both items
    let keep_item = match client.get_item(&keep_key) {
        Ok(i) => i,
        Err(e) => return ToolCallResult::error(format!("Failed to fetch keep item: {e}")),
    };
    let delete_item = match client.get_item(&delete_key) {
        Ok(i) => i,
        Err(e) => return ToolCallResult::error(format!("Failed to fetch delete item: {e}")),
    };

    let keep_version = keep_item["version"].as_i64().unwrap_or(0) as i32;
    let delete_version = delete_item["version"].as_i64().unwrap_or(0) as i32;
    let mut keep_data = keep_item["data"].clone();

    // Merge tags
    let mut tags: std::collections::HashSet<String> = keep_data["tags"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|t| t["tag"].as_str().map(String::from))
        .collect();
    if let Some(delete_tags) = delete_item["data"]["tags"].as_array() {
        for t in delete_tags {
            if let Some(tag) = t["tag"].as_str() {
                tags.insert(tag.to_string());
            }
        }
    }
    keep_data["tags"] = json!(tags.iter().map(|t| json!({"tag": t})).collect::<Vec<_>>());

    // Merge collections
    let mut colls: std::collections::HashSet<String> = keep_data["collections"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|c| c.as_str().map(String::from))
        .collect();
    if let Some(delete_colls) = delete_item["data"]["collections"].as_array() {
        for c in delete_colls {
            if let Some(coll) = c.as_str() {
                colls.insert(coll.to_string());
            }
        }
    }
    keep_data["collections"] = json!(colls.into_iter().collect::<Vec<_>>());

    // Update keep item
    if let Err(e) = client.update_item(&keep_key, &keep_data, keep_version) {
        return ToolCallResult::error(format!("Failed to update keep item: {e}"));
    }

    // Delete the duplicate
    if let Err(e) = client.delete_item(&delete_key, delete_version) {
        return ToolCallResult::error(format!("Merged but failed to delete duplicate: {e}"));
    }

    ToolCallResult::text(format!(
        "Merged {delete_ck} into {keep_ck}. Deleted {delete_ck}."
    ))
}

pub fn zotero_attach_pdf(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let client = match get_write_client(ctx) {
        Ok(c) => c,
        Err(e) => return ToolCallResult::error(e),
    };

    let raw_key = match args.get("item_key").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => return ToolCallResult::error("Missing parameter: item_key".into()),
    };
    let pdf_url = match args.get("pdf_url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return ToolCallResult::error("Missing parameter: pdf_url".into()),
    };
    let title = args.get("title").and_then(|v| v.as_str());

    // Resolve citekey → item_key if needed
    let item_key = match resolve_item_key(ctx, raw_key) {
        Ok(k) => k,
        Err(e) => return ToolCallResult::error(format!("Cannot resolve '{raw_key}': {e}")),
    };

    // Check if item already has a PDF
    if let Ok(zdb) = ctx.db.zotero()
        && let Ok(Some(item)) = zdb.item_by_key(&item_key)
        && let Ok(atts) = zdb.item_attachments(item.item_id)
        && atts.iter().any(|a| a.content_type == "application/pdf")
    {
        return ToolCallResult::text(format!("Item {item_key} already has a PDF attachment."));
    }

    // Validate item_key is alphanumeric (prevent path traversal)
    if !item_key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return ToolCallResult::error("Invalid item_key: must be alphanumeric".into());
    }

    // Download PDF to temp file
    let tmp_dir = std::env::temp_dir();
    let tmp_file = tmp_dir.join(format!("biblion-{item_key}.pdf"));
    if let Err(e) = client.download_file(pdf_url, &tmp_file) {
        return ToolCallResult::error(format!("Download failed: {e}"));
    }

    let display_title = title.unwrap_or("PDF");
    match client.attach_file(&item_key, &tmp_file, display_title) {
        Ok(_) => {
            let _ = std::fs::remove_file(&tmp_file);
            ToolCallResult::text(format!("PDF attached to {item_key}."))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_file);
            ToolCallResult::error(format!("Failed to attach PDF: {e}"))
        }
    }
}

/// Scan items for missing PDFs and resolve from 9 open-access sources.
pub fn zotero_fetch_missing_pdfs(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
    let collection_key = args.get("collection_key").and_then(|v| v.as_str());

    let zdb = match ctx.db.zotero() {
        Ok(db) => db,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    // Get items that need PDFs
    let items_to_scan: Vec<(i64, String)> = if let Some(ck) = collection_key {
        match zdb.collection_items(ck, limit) {
            Ok(items) => items,
            Err(e) => return ToolCallResult::error(e.to_string()),
        }
    } else {
        match zdb.recent_items(limit) {
            Ok(items) => items,
            Err(e) => return ToolCallResult::error(e.to_string()),
        }
    };

    // Filter to items without PDF attachments
    let mut missing: Vec<(String, Option<String>, Option<String>)> = Vec::new(); // (item_key, doi, title)
    for (item_id, item_key) in &items_to_scan {
        let has_pdf = zdb
            .item_attachments(*item_id)
            .map(|atts| atts.iter().any(|a| a.content_type == "application/pdf"))
            .unwrap_or(false);

        if !has_pdf {
            let metadata = zdb.item_metadata(*item_id).unwrap_or_default();
            missing.push((
                item_key.clone(),
                metadata.get("DOI").cloned(),
                metadata.get("title").cloned(),
            ));
        }
    }

    if missing.is_empty() {
        return ToolCallResult::text(format!(
            "Scanned {} items. All have PDF attachments.",
            items_to_scan.len()
        ));
    }

    let mut output = format!(
        "Scanned {} items, {} missing PDFs.\n\n",
        items_to_scan.len(),
        missing.len()
    );

    let mut resolved = 0;
    let mut attached = 0;

    for (item_key, doi, title) in &missing {
        let result = paper_resolver::resolve_pdf_with_config(
            doi.as_deref(),
            None,
            title.as_deref(),
            &ctx.config.resolver,
        );

        let citekey = ctx
            .db
            .citekey_for_item_key(item_key)
            .unwrap_or_else(|| item_key.clone());

        match result {
            Some(pdf) if pdf.downloadable => {
                resolved += 1;
                if dry_run {
                    output.push_str(&format!(
                        "[would attach] {citekey} — {} (via {})\n",
                        pdf.url, pdf.source
                    ));
                } else {
                    // Actually download and attach
                    let client = match get_write_client(ctx) {
                        Ok(c) => c,
                        Err(e) => {
                            output.push_str(&format!("[error] {citekey} — {e}\n"));
                            continue;
                        }
                    };
                    let tmp = std::env::temp_dir().join(format!("biblion-{item_key}.pdf"));
                    match client.download_file(&pdf.url, &tmp) {
                        Ok(()) => {
                            match client.attach_file(item_key, &tmp, &citekey) {
                                Ok(_) => {
                                    attached += 1;
                                    output.push_str(&format!(
                                        "[attached] {citekey} — {} (via {})\n",
                                        pdf.url, pdf.source
                                    ));
                                }
                                Err(e) => {
                                    output.push_str(&format!(
                                        "[error] {citekey} — attach failed: {e}\n"
                                    ));
                                }
                            }
                            let _ = std::fs::remove_file(&tmp);
                        }
                        Err(e) => {
                            output.push_str(&format!("[error] {citekey} — download failed: {e}\n"));
                        }
                    }
                }
            }
            Some(pdf) => {
                output.push_str(&format!(
                    "[manual] {citekey} — {} (via {}, not downloadable)\n",
                    pdf.url, pdf.source
                ));
            }
            None => {
                output.push_str(&format!("[not found] {citekey}\n"));
            }
        }
    }

    let not_found = missing.len() - resolved - attached;
    let manual = resolved - attached;
    output.push_str(&format!(
        "\nResolved: {resolved}, Attached: {attached}, Manual: {manual}, Not found: {not_found}"
    ));

    ToolCallResult::text(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, LogLevel};
    use crate::db::DbPool;
    use crate::test_helpers::test_zotero_db;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Create a test context with writes enabled and a custom API base URL.
    fn write_ctx_with_base_url(base_url: &str) -> ServerContext {
        let zdb = test_zotero_db();
        ServerContext {
            db: DbPool {
                zotero: Some(zdb),
                bbt: None,
            },
            config: Config {
                zotero_sqlite_path: "/tmp/test.sqlite".into(),
                zotero_storage_path: "/tmp/storage".into(),
                bbt_migrated_path: "/tmp/bbt".into(),
                zotero_api_key: Some("test-api-key".into()),
                zotero_library_id: "12345".into(),
                zotero_library_type: "user".into(),
                bbt_url: "http://localhost:23119".into(),
                log_level: LogLevel::Quiet,
                writes_enabled: true,
                resolver: paper_resolver::ResolverConfig::default(),
                zotero_api_base_url: Some(base_url.into()),
            },
        }
    }

    /// Helper: spin up a tokio runtime and a wiremock MockServer.
    fn start_mock() -> (tokio::runtime::Runtime, MockServer) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        (rt, server)
    }

    /// Standard mock response for GET /items/{key}.
    fn item_response(key: &str, version: i64) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "key": key,
            "version": version,
            "data": {
                "key": key,
                "version": version,
                "itemType": "journalArticle",
                "title": "Hints on Test Data Selection",
                "tags": [{"tag": "mutation-testing"}, {"tag": "foundational"}],
                "collections": ["COL00001"],
                "creators": [],
            }
        }))
    }

    // -----------------------------------------------------------------------
    // zotero_create_item — success
    // -----------------------------------------------------------------------

    #[test]
    fn create_item_success() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("GET"))
                .and(path("/items/new"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "itemType": "journalArticle",
                    "title": "",
                    "creators": [],
                    "tags": [],
                    "collections": [],
                })))
                .mount(&mock_server)
                .await;

            Mock::given(method("POST"))
                .and(path("/items"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "successful": {"0": {"key": "NEWKEY01", "version": 1}},
                    "unchanged": {},
                    "failed": {}
                })))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({
            "item_type": "journalArticle",
            "title": "Test Paper",
        });
        let result = zotero_create_item(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("NEWKEY01"));
    }

    // -----------------------------------------------------------------------
    // zotero_create_item — missing params
    // -----------------------------------------------------------------------

    #[test]
    fn create_item_missing_title() {
        let (rt, mock_server) = start_mock();
        let _ = rt;
        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({"item_type": "journalArticle"});
        let result = zotero_create_item(&args, &ctx);
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("Missing parameter: title"));
    }

    #[test]
    fn create_item_missing_item_type() {
        let (rt, mock_server) = start_mock();
        let _ = rt;
        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({"title": "Test"});
        let result = zotero_create_item(&args, &ctx);
        assert_eq!(result.is_error, Some(true));
        assert!(
            result.content[0]
                .text
                .contains("Missing parameter: item_type")
        );
    }

    // -----------------------------------------------------------------------
    // zotero_update_item — success (resolves citekey via SQLite)
    // -----------------------------------------------------------------------

    #[test]
    fn update_item_success() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("GET"))
                .and(path("/items/ABC12345"))
                .respond_with(item_response("ABC12345", 5))
                .mount(&mock_server)
                .await;

            Mock::given(method("PATCH"))
                .and(path("/items/ABC12345"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({
            "citekey": "demilloHintsTestData1978",
            "fields": {"title": "Updated Title"},
        });
        let result = zotero_update_item(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("updated"));
    }

    // -----------------------------------------------------------------------
    // zotero_delete_item — success
    // -----------------------------------------------------------------------

    #[test]
    fn delete_item_success() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("GET"))
                .and(path("/items/ABC12345"))
                .respond_with(item_response("ABC12345", 5))
                .mount(&mock_server)
                .await;

            Mock::given(method("DELETE"))
                .and(path("/items/ABC12345"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({"citekey": "demilloHintsTestData1978"});
        let result = zotero_delete_item(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("deleted"));
    }

    // -----------------------------------------------------------------------
    // zotero_add_tags — success (merges with existing)
    // -----------------------------------------------------------------------

    #[test]
    fn add_tags_success() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("GET"))
                .and(path("/items/ABC12345"))
                .respond_with(item_response("ABC12345", 5))
                .mount(&mock_server)
                .await;

            Mock::given(method("PATCH"))
                .and(path("/items/ABC12345"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({
            "citekey": "demilloHintsTestData1978",
            "tags": ["new-tag", "mutation-testing"],
        });
        let result = zotero_add_tags(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("Tags added"));
    }

    // -----------------------------------------------------------------------
    // zotero_add_note — success
    // -----------------------------------------------------------------------

    #[test]
    fn add_note_success() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("POST"))
                .and(path("/items"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "successful": {"0": {"key": "NOTE0002", "version": 1}},
                    "unchanged": {},
                    "failed": {}
                })))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({
            "citekey": "demilloHintsTestData1978",
            "content": "This is a test note.",
        });
        let result = zotero_add_note(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("Note added"));
    }

    // -----------------------------------------------------------------------
    // zotero_create_collection — success
    // -----------------------------------------------------------------------

    #[test]
    fn create_collection_success() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("POST"))
                .and(path("/collections"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "successful": {"0": {"key": "NEWCOL01", "version": 1}},
                    "unchanged": {},
                    "failed": {}
                })))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({"name": "New Collection"});
        let result = zotero_create_collection(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("created"));
    }

    // -----------------------------------------------------------------------
    // zotero_create_collection — already exists
    // -----------------------------------------------------------------------

    #[test]
    fn create_collection_already_exists() {
        let (rt, mock_server) = start_mock();
        let _ = rt;

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({"name": "Mutation Testing"});
        let result = zotero_create_collection(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("already exists"));
        assert!(result.content[0].text.contains("COL00001"));
    }

    // -----------------------------------------------------------------------
    // zotero_add_to_collection — success
    // -----------------------------------------------------------------------

    #[test]
    fn add_to_collection_success() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("GET"))
                .and(path("/items/ABC12345"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "key": "ABC12345",
                    "version": 5,
                    "data": {
                        "key": "ABC12345",
                        "version": 5,
                        "itemType": "journalArticle",
                        "title": "Test",
                        "tags": [],
                        "collections": [],
                        "creators": [],
                    }
                })))
                .mount(&mock_server)
                .await;

            Mock::given(method("PATCH"))
                .and(path("/items/ABC12345"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({
            "citekey": "demilloHintsTestData1978",
            "collection_key": "NEWCOL01",
        });
        let result = zotero_add_to_collection(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("added to collection"));
    }

    // -----------------------------------------------------------------------
    // zotero_add_to_collection — already in collection
    // -----------------------------------------------------------------------

    #[test]
    fn add_to_collection_already_member() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("GET"))
                .and(path("/items/ABC12345"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "key": "ABC12345",
                    "version": 5,
                    "data": {
                        "key": "ABC12345",
                        "version": 5,
                        "itemType": "journalArticle",
                        "title": "Test",
                        "tags": [],
                        "collections": ["COL00001"],
                        "creators": [],
                    }
                })))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({
            "citekey": "demilloHintsTestData1978",
            "collection_key": "COL00001",
        });
        let result = zotero_add_to_collection(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("already in collection"));
    }

    // -----------------------------------------------------------------------
    // zotero_remove_from_collection — success
    // -----------------------------------------------------------------------

    #[test]
    fn remove_from_collection_success() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("GET"))
                .and(path("/items/ABC12345"))
                .respond_with(item_response("ABC12345", 5))
                .mount(&mock_server)
                .await;

            Mock::given(method("PATCH"))
                .and(path("/items/ABC12345"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({
            "citekey": "demilloHintsTestData1978",
            "collection_key": "COL00001",
        });
        let result = zotero_remove_from_collection(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("removed from collection"));
    }

    // -----------------------------------------------------------------------
    // zotero_merge_items — success
    // -----------------------------------------------------------------------

    #[test]
    fn merge_items_success() {
        let (rt, mock_server) = start_mock();

        rt.block_on(async {
            Mock::given(method("GET"))
                .and(path("/items/ABC12345"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "key": "ABC12345",
                    "version": 5,
                    "data": {
                        "key": "ABC12345",
                        "itemType": "journalArticle",
                        "title": "Hints on Test Data Selection",
                        "tags": [{"tag": "mutation-testing"}],
                        "collections": ["COL00001"],
                    }
                })))
                .mount(&mock_server)
                .await;

            Mock::given(method("GET"))
                .and(path("/items/DEF67890"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "key": "DEF67890",
                    "version": 3,
                    "data": {
                        "key": "DEF67890",
                        "itemType": "book",
                        "title": "The Art of Testing",
                        "tags": [{"tag": "foundational"}, {"tag": "testing"}],
                        "collections": ["COL00002"],
                    }
                })))
                .mount(&mock_server)
                .await;

            Mock::given(method("PATCH"))
                .and(path("/items/ABC12345"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&mock_server)
                .await;

            Mock::given(method("DELETE"))
                .and(path("/items/DEF67890"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&mock_server)
                .await;
        });

        let ctx = write_ctx_with_base_url(&mock_server.uri());
        let args = json!({
            "keep_citekey": "demilloHintsTestData1978",
            "delete_citekey": "artTesting2020",
        });
        let result = zotero_merge_items(&args, &ctx);
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("Merged"));
        assert!(result.content[0].text.contains("Deleted"));
    }

    // -----------------------------------------------------------------------
    // writes_disabled — get_write_client returns error
    // -----------------------------------------------------------------------

    #[test]
    fn writes_disabled_returns_error() {
        let ctx = crate::test_helpers::test_ctx();
        let result = get_write_client(&ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Write tools disabled"));
    }

    // -----------------------------------------------------------------------
    // missing_api_key — get_write_client returns error
    // -----------------------------------------------------------------------

    #[test]
    fn missing_api_key_returns_error() {
        let zdb = test_zotero_db();
        let ctx = ServerContext {
            db: DbPool {
                zotero: Some(zdb),
                bbt: None,
            },
            config: Config {
                zotero_sqlite_path: "/tmp/test.sqlite".into(),
                zotero_storage_path: "/tmp/storage".into(),
                bbt_migrated_path: "/tmp/bbt".into(),
                zotero_api_key: None,
                zotero_library_id: "12345".into(),
                zotero_library_type: "user".into(),
                bbt_url: "http://localhost:23119".into(),
                log_level: LogLevel::Quiet,
                writes_enabled: true,
                resolver: paper_resolver::ResolverConfig::default(),
                zotero_api_base_url: None,
            },
        };
        let result = get_write_client(&ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ZOTERO_API_KEY"));
    }
}
