//! BBT (Better BibTeX) citekey database reader.
//!
//! # What this does
//!
//! Better BibTeX assigns citation keys to Zotero items. These keys (like
//! "demilloHintsTestData1978") are the primary way users reference items
//! in the MCP tools. We need to map them to Zotero's internal 8-char item
//! keys (like "9MS26VH5").
//!
//! # The `better-bibtex.migrated` file
//!
//! BBT stores its citekey assignments in `~/Zotero/better-bibtex.migrated`,
//! a SQLite database with a `citationkey` table:
//!
//! ```sql
//! CREATE TABLE citationkey (
//!     itemID INTEGER,
//!     itemKey TEXT,
//!     libraryID INTEGER,
//!     citationKey TEXT,
//!     pinned INTEGER
//! );
//! ```
//!
//! This is much faster than calling BBT's JSON-RPC API (~0.01ms vs ~300ms).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Read-only connection to the BBT citekey database.
pub struct BbtDb {
    conn: Connection,
}

impl BbtDb {
    /// Open the BBT database in read-only mode.
    pub fn open(path: &Path) -> Result<Self> {
        let uri = format!("file:{}?mode=ro", path.display());
        let conn = Connection::open_with_flags(
            &uri,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .with_context(|| format!("Failed to open BBT database: {}", path.display()))?;
        Ok(Self { conn })
    }

    /// Look up a citekey by Zotero item key (e.g., "9MS26VH5" → "demilloHintsTestData1978").
    pub fn citekey_for_item_key(&self, item_key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT citationKey FROM citationkey WHERE itemKey = ?1")?;
        let result = stmt
            .query_row([item_key], |row| row.get::<_, String>(0))
            .optional()?;
        Ok(result)
    }

    /// Look up a Zotero item key by citekey (e.g., "demilloHintsTestData1978" → "9MS26VH5").
    pub fn item_key_for_citekey(&self, citekey: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT itemKey FROM citationkey WHERE citationKey = ?1")?;
        let result = stmt
            .query_row([citekey], |row| row.get::<_, String>(0))
            .optional()?;
        Ok(result)
    }

    /// Load the complete citekey→itemKey mapping into memory.
    ///
    /// Returns a HashMap with ~1000 entries (~43KB). Used for batch operations
    /// and for populating the in-memory cache at startup.
    pub fn all_citekeys(&self) -> Result<HashMap<String, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT citationKey, itemKey FROM citationkey WHERE citationKey IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (ck, ik) = row?;
            map.insert(ck, ik);
        }
        Ok(map)
    }
}

// ---------------------------------------------------------------------------
// Zotero-native citekey fallback
// ---------------------------------------------------------------------------

/// Read citekeys directly from `zotero.sqlite` (field name = "citationKey").
///
/// BBT stores citekeys as item metadata fields in Zotero's EAV schema.
/// This is the most reliable source — it covers 99.9% of items, even
/// those not yet indexed in `better-bibtex.migrated`.
pub fn citekey_from_zotero_sqlite(
    conn: &Connection,
    item_key: &str,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT iv.value FROM items i
         JOIN itemData id ON i.itemID = id.itemID
         JOIN fields f ON id.fieldID = f.fieldID
         JOIN itemDataValues iv ON id.valueID = iv.valueID
         WHERE f.fieldName = 'citationKey' AND i.key = ?1",
    )?;
    let result = stmt
        .query_row([item_key], |row| row.get::<_, String>(0))
        .optional()?;
    Ok(result)
}

/// Reverse lookup: find item key by citekey in `zotero.sqlite`.
pub fn item_key_from_zotero_sqlite(
    conn: &Connection,
    citekey: &str,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT i.key FROM items i
         JOIN itemData id ON i.itemID = id.itemID
         JOIN fields f ON id.fieldID = f.fieldID
         JOIN itemDataValues iv ON id.valueID = iv.valueID
         WHERE f.fieldName = 'citationKey' AND iv.value = ?1
         AND i.libraryID = 1
         AND i.itemID NOT IN (SELECT itemID FROM deletedItems)",
    )?;
    let result = stmt
        .query_row([citekey], |row| row.get::<_, String>(0))
        .optional()?;
    Ok(result)
}

/// Load all citekeys from `zotero.sqlite` (most complete source).
pub fn all_citekeys_from_zotero_sqlite(
    conn: &Connection,
) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare(
        "SELECT iv.value, i.key FROM items i
         JOIN itemData id ON i.itemID = id.itemID
         JOIN fields f ON id.fieldID = f.fieldID
         JOIN itemDataValues iv ON id.valueID = iv.valueID
         WHERE f.fieldName = 'citationKey'
         AND i.libraryID = 1
         AND i.itemID NOT IN (SELECT itemID FROM deletedItems)",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (ck, ik) = row?;
        map.insert(ck, ik);
    }
    Ok(map)
}

/// Extension trait for rusqlite::OptionalExtension
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Create an in-memory BBT database for testing.
    fn test_bbt_db() -> BbtDb {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE citationkey (
                itemID INTEGER,
                itemKey TEXT,
                libraryID INTEGER,
                citationKey TEXT,
                pinned INTEGER
            );
            INSERT INTO citationkey VALUES (1, 'ABC12345', 1, 'demilloHintsTestData1978', 0);
            INSERT INTO citationkey VALUES (2, 'DEF67890', 1, 'jiaAnalysisSurvey2011', 1);
            INSERT INTO citationkey VALUES (3, 'GHI11111', 1, NULL, 0);",
        )
        .unwrap();
        BbtDb { conn }
    }

    #[test]
    fn citekey_for_item_key_found() {
        let db = test_bbt_db();
        let ck = db.citekey_for_item_key("ABC12345").unwrap();
        assert_eq!(ck, Some("demilloHintsTestData1978".into()));
    }

    #[test]
    fn citekey_for_item_key_not_found() {
        let db = test_bbt_db();
        let ck = db.citekey_for_item_key("ZZZZZZZZ").unwrap();
        assert_eq!(ck, None);
    }

    #[test]
    fn item_key_for_citekey_found() {
        let db = test_bbt_db();
        let ik = db.item_key_for_citekey("jiaAnalysisSurvey2011").unwrap();
        assert_eq!(ik, Some("DEF67890".into()));
    }

    #[test]
    fn item_key_for_citekey_not_found() {
        let db = test_bbt_db();
        let ik = db.item_key_for_citekey("nonexistent2099").unwrap();
        assert_eq!(ik, None);
    }

    #[test]
    fn all_citekeys_excludes_null() {
        let db = test_bbt_db();
        let map = db.all_citekeys().unwrap();
        assert_eq!(map.len(), 2); // GHI11111 has NULL citationKey, excluded
        assert_eq!(map["demilloHintsTestData1978"], "ABC12345");
        assert_eq!(map["jiaAnalysisSurvey2011"], "DEF67890");
    }
}
