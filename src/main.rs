// Allow dead code for API completeness — fields/methods used in tests or
// reserved for future use (e.g., BbtDb::all_citekeys, ZoteroWebClient::children).
#![allow(dead_code)]

//! Zotero MCP Server — high-performance Rust implementation.
//!
//! # Transports
//!
//! Supports two MCP transports:
//!
//! - **stdio** (default) — pipe-based, launched by Claude Code CLI.
//!   Zero overhead, one process per session.
//!
//! - **SSE** — HTTP-based, persistent daemon. For Claude desktop app,
//!   IDE extensions, or multiple concurrent clients.
//!   Set `ZOTERO_MCP_TRANSPORT=sse` to enable.
//!
//! # Architecture
//!
//! ```text
//! Claude ←stdio/sse→ zotero-mcp (this binary)
//!                         │
//!              ┌──────────┼──────────┐
//!              │          │          │
//!         Read Tools   Write Tools  PDF Resolver
//!         (sync SQLite) (reqwest)   (tokio async)
//!              │          │          │
//!         zotero.sqlite  Zotero    9 sources
//!         bbt.migrated   Web API   concurrent
//! ```
//!
//! # Environment variables
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `ZOTERO_MCP_TRANSPORT` | `stdio` | Transport: `stdio` or `sse` |
//! | `ZOTERO_MCP_HOST` | `127.0.0.1` | SSE listen address |
//! | `ZOTERO_MCP_PORT` | `23120` | SSE listen port |
//! | `ZOTERO_API_KEY` | (none) | Required for write operations |
//! | `ZOTERO_SQLITE_PATH` | `~/Zotero/zotero.sqlite` | Zotero database |
//! | `BBT_MIGRATED_PATH` | `~/Zotero/better-bibtex.migrated` | BBT citekeys |

mod api;
mod config;
mod db;
mod protocol;
mod server;
mod sse;
mod tools;

use anyhow::Result;

fn main() -> Result<()> {
    let config = config::Config::from_env();

    // Open both databases at startup (non-fatal if missing)
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
