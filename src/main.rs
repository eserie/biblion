//! Zotero MCP Server — high-performance Rust implementation.
//!
//! # Why Rust?
//!
//! The Python MCP server routes reads through BBT JSON-RPC, a JavaScript
//! plugin inside Zotero's Electron app. Each call takes 300-500ms.
//! This Rust server reads SQLite directly: <1ms per query. 830x faster.
//!
//! # Architecture
//!
//! ```text
//! Claude ←stdio→ zotero-mcp (this binary)
//!                     │
//!          ┌──────────┼──────────┐
//!          │          │          │
//!     Read Tools   Write Tools  PDF Resolver
//!     (sync SQLite) (reqwest)   (tokio async)
//!          │          │          │
//!     zotero.sqlite  Zotero    9 sources
//!     bbt.migrated   Web API   concurrent
//! ```

mod config;
mod db;
mod protocol;
mod server;
mod tools;

use anyhow::Result;

fn main() -> Result<()> {
    let config = config::Config::from_env();

    // Open both databases at startup (non-fatal if missing)
    let db = db::DbPool::open(&config.zotero_sqlite_path, &config.bbt_migrated_path);

    let ctx = server::ServerContext { db, config };

    // Run the stdio server (blocks until stdin EOF)
    server::run_stdio(&ctx)
}
