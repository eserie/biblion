// Allow dead code for API completeness — fields/methods used in tests or
// reserved for future use (e.g., BbtDb::all_citekeys, ZoteroWebClient::children).
#![allow(dead_code)]

//! Zotero MCP Server — high-performance Rust implementation.
//!
//! # Transports
//!
//! - **stdio** (default) — pipe-based, launched by Claude Code CLI.
//! - **SSE** — HTTP daemon for Claude Desktop. Set `ZOTERO_MCP_TRANSPORT=sse`.
//!
//! # Subcommands
//!
//! - `zotero-mcp` — run MCP server (default)
//! - `zotero-mcp check` — print diagnostics and exit

mod api;
mod config;
mod db;
mod protocol;
mod server;
mod sse;
mod tools;

use anyhow::Result;

fn main() -> Result<()> {
    // Check for "check" subcommand
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "check" {
        return run_check();
    }

    let config = config::Config::from_env();
    let db = db::DbPool::open(&config.zotero_sqlite_path, &config.bbt_migrated_path);

    let transport = std::env::var("ZOTERO_MCP_TRANSPORT")
        .unwrap_or_else(|_| "stdio".into());

    match transport.as_str() {
        "sse" => {
            let host = std::env::var("ZOTERO_MCP_HOST")
                .unwrap_or_else(|_| "127.0.0.1".into());
            let port: u16 = std::env::var("ZOTERO_MCP_PORT")
                .unwrap_or_else(|_| "23120".into())
                .parse()
                .unwrap_or(23120);
            let ctx = server::ServerContext { db, config };
            sse::run_sse(ctx, &host, port)
        }
        _ => {
            let ctx = server::ServerContext { db, config };
            server::run_stdio(&ctx)
        }
    }
}

/// Diagnostic subcommand: print library info and exit.
fn run_check() -> Result<()> {
    let config = config::Config::from_env();
    let db = db::DbPool::open(&config.zotero_sqlite_path, &config.bbt_migrated_path);

    println!("zotero-mcp v{}", env!("CARGO_PKG_VERSION"));
    println!();

    // Zotero database
    match &db.zotero {
        Some(zdb) => {
            let size = std::fs::metadata(&config.zotero_sqlite_path)
                .map(|m| m.len() / 1_048_576)
                .unwrap_or(0);
            let items = zdb.item_count().unwrap_or(0);
            let colls = zdb.collection_count().unwrap_or(0);
            println!(
                "Zotero database: {} ({} MB, {} items, {} collections)",
                config.zotero_sqlite_path.display(),
                size,
                items,
                colls
            );
        }
        None => {
            println!(
                "Zotero database: NOT FOUND at {}",
                config.zotero_sqlite_path.display()
            );
        }
    }

    // BBT database
    match &db.bbt {
        Some(bbt) => {
            let count = bbt.all_citekeys().map(|m| m.len()).unwrap_or(0);
            println!(
                "BBT database:    {} ({} citekeys)",
                config.bbt_migrated_path.display(),
                count
            );
        }
        None => {
            println!(
                "BBT database:    NOT FOUND at {}",
                config.bbt_migrated_path.display()
            );
        }
    }

    // Write access
    println!();
    if config.writes_enabled && config.has_write_access() {
        println!("Write access:    enabled (API key set, writes enabled)");
    } else if config.has_write_access() {
        println!("Write access:    disabled (API key set, but ZOTERO_MCP_ENABLE_WRITES not set)");
    } else {
        println!("Write access:    disabled (no ZOTERO_API_KEY)");
    }

    // Transport
    let transport = std::env::var("ZOTERO_MCP_TRANSPORT").unwrap_or_else(|_| "stdio".into());
    println!("Transport:       {transport}");

    Ok(())
}
