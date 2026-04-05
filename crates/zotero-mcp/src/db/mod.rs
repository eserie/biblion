//! Database access layer — direct SQLite reads on Zotero's databases.
//!
//! # The key insight
//!
//! Zotero stores everything in SQLite. The Python MCP server was slow because
//! it went through BBT JSON-RPC (a JavaScript plugin inside Zotero's Electron
//! app) to read this same data. By reading SQLite directly, we eliminate the
//! dominant bottleneck: ~300ms per BBT RPC call → <1ms per SQLite query.
//!
//! # Two databases
//!
//! - **`zotero.sqlite`** — all item metadata, collections, tags, creators,
//!   attachments, notes. Uses an EAV (Entity-Attribute-Value) schema:
//!   `items → itemData → itemDataValues` with field IDs mapped via `fields`.
//!
//! - **`better-bibtex.migrated`** — citekey assignments. Maps Zotero item keys
//!   (like "9MS26VH5") to citation keys (like "demilloHintsTestData1978").
//!   This file may not exist if BBT isn't installed.
//!
//! Both are opened read-only. We never write to Zotero's databases directly —
//! writes go through the Zotero Web API.

pub mod bbt;
pub mod zotero;

use anyhow::Result;
use std::path::Path;

/// Holds connections to both Zotero databases.
///
/// Created once at startup and shared (by reference) across all tool calls.
/// Both connections are read-only and held open for the process lifetime.
pub struct DbPool {
    pub zotero: Option<zotero::ZoteroDb>,
    pub bbt: Option<bbt::BbtDb>,
}

impl DbPool {
    /// Open both databases. Non-fatal: if a database doesn't exist or can't
    /// be opened, the corresponding field is None and tools that need it
    /// return a clear error.
    pub fn open(zotero_path: &Path, bbt_path: &Path) -> Self {
        let zotero = match zotero::ZoteroDb::open(zotero_path) {
            Ok(db) => Some(db),
            Err(e) => {
                eprintln!("[zotero-mcp] Warning: cannot open zotero.sqlite: {e}");
                None
            }
        };
        let bbt = match bbt::BbtDb::open(bbt_path) {
            Ok(db) => Some(db),
            Err(e) => {
                eprintln!("[zotero-mcp] Warning: cannot open better-bibtex.migrated: {e}");
                None
            }
        };
        Self { zotero, bbt }
    }

    /// Create an empty pool (for testing when no databases are available).
    pub fn empty() -> Self {
        Self {
            zotero: None,
            bbt: None,
        }
    }

    /// Get the Zotero database, returning a clear error if unavailable.
    pub fn zotero(&self) -> Result<&zotero::ZoteroDb> {
        self.zotero.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Zotero database not available. Check ZOTERO_SQLITE_PATH.")
        })
    }

    /// Get the BBT database, returning a clear error if unavailable.
    pub fn bbt(&self) -> Result<&bbt::BbtDb> {
        self.bbt.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Better BibTeX database not available. Check BBT_MIGRATED_PATH.")
        })
    }

    /// Resolve a citekey to an item key, trying all available sources.
    ///
    /// Priority:
    /// 1. `zotero.sqlite` citationKey field (most complete, 99.9% coverage)
    /// 2. `better-bibtex.migrated` (for items not yet synced to Zotero)
    pub fn item_key_for_citekey(&self, citekey: &str) -> Result<Option<String>> {
        // Try zotero.sqlite first (most reliable)
        if let Some(zdb) = &self.zotero
            && let Ok(Some(key)) = bbt::item_key_from_zotero_sqlite(zdb.conn(), citekey)
        {
            return Ok(Some(key));
        }
        // Fallback to BBT migrated
        if let Some(bdb) = &self.bbt
            && let Ok(Some(key)) = bdb.item_key_for_citekey(citekey)
        {
            return Ok(Some(key));
        }
        Ok(None)
    }

    /// Resolve an item key to a citekey, trying all available sources.
    pub fn citekey_for_item_key(&self, item_key: &str) -> Option<String> {
        // Try zotero.sqlite first
        if let Some(zdb) = &self.zotero
            && let Ok(Some(ck)) = bbt::citekey_from_zotero_sqlite(zdb.conn(), item_key)
        {
            return Some(ck);
        }
        // Fallback to BBT migrated
        if let Some(bdb) = &self.bbt
            && let Ok(Some(ck)) = bdb.citekey_for_item_key(item_key)
        {
            return Some(ck);
        }
        None
    }
}
