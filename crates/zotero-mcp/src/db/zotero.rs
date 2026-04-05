//! Zotero SQLite database reader.
//!
//! # Zotero's EAV schema
//!
//! Zotero uses an Entity-Attribute-Value pattern for item metadata:
//!
//! ```text
//! items (itemID, key, itemTypeID, dateAdded, dateModified)
//!   └── itemData (itemID, fieldID, valueID)
//!         └── itemDataValues (valueID, value)
//!               └── fields (fieldID, fieldName)
//! ```
//!
//! This means getting an item's title requires a 3-table JOIN:
//! `items → itemData → itemDataValues`, filtered by `fieldID` for "title".
//!
//! Similarly, creators, tags, and attachments are stored in separate tables
//! linked by `itemID`.
//!
//! # Performance
//!
//! The database is ~71MB with ~2700 items. All queries use prepared statements
//! (`prepare_cached`) and the entire database fits in the OS page cache after
//! first access. Expected latency: <1ms for single-item lookups, <10ms for
//! full-table scans (search).
//!
//! # Filtering conventions
//!
//! - Always exclude deleted items: `itemID NOT IN (SELECT itemID FROM deletedItems)`
//! - Always filter to personal library: `libraryID = 1`
//! - Exclude attachments and notes when listing "real" items:
//!   `itemTypeID NOT IN (SELECT itemTypeID FROM itemTypes WHERE typeName IN ('attachment', 'note', 'annotation'))`

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// A Zotero library item with all its metadata assembled from the EAV schema.
#[derive(Debug, Clone)]
pub struct ZoteroItem {
    pub item_id: i64,
    pub item_key: String,
    pub item_type: String,
    pub title: String,
    pub date: Option<String>,
    pub doi: Option<String>,
    pub url: Option<String>,
    pub abstract_note: Option<String>,
    pub creators: Vec<Creator>,
    pub tags: Vec<String>,
    pub date_added: String,
    pub date_modified: String,
}

#[derive(Debug, Clone)]
pub struct Creator {
    pub creator_type: String,
    pub first_name: Option<String>,
    pub last_name: String,
    pub order: i32,
}

#[derive(Debug, Clone)]
pub struct Collection {
    pub collection_id: i64,
    pub key: String,
    pub name: String,
    pub parent_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Attachment {
    pub item_key: String,
    pub content_type: String,
    pub path: Option<String>,
    pub title: Option<String>,
}

/// Read-only connection to Zotero's main SQLite database.
pub struct ZoteroDb {
    conn: Connection,
}

impl ZoteroDb {
    /// Access the underlying SQLite connection (for cross-database queries).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Open the Zotero database in read-only mode.
    pub fn open(path: &Path) -> Result<Self> {
        let uri = format!("file:{}?mode=ro", path.display());
        let conn = Connection::open_with_flags(
            &uri,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .with_context(|| format!("Failed to open Zotero database: {}", path.display()))?;
        Ok(Self { conn })
    }

    // -----------------------------------------------------------------------
    // Item queries
    // -----------------------------------------------------------------------

    /// Count non-deleted, substantive items (excludes attachments/notes).
    pub fn item_count(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items i
             WHERE i.libraryID = 1
             AND i.itemID NOT IN (SELECT itemID FROM deletedItems)
             AND i.itemTypeID NOT IN (
                 SELECT itemTypeID FROM itemTypes
                 WHERE typeName IN ('attachment', 'note', 'annotation')
             )",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get an item by its 8-char Zotero key (e.g., "9MS26VH5").
    pub fn item_by_key(&self, key: &str) -> Result<Option<ZoteroItem>> {
        let row = self.conn.query_row(
            "SELECT i.itemID, i.key, it.typeName, i.dateAdded, i.dateModified
             FROM items i
             JOIN itemTypes it ON i.itemTypeID = it.itemTypeID
             WHERE i.key = ?1 AND i.libraryID = 1
             AND i.itemID NOT IN (SELECT itemID FROM deletedItems)",
            [key],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        );
        match row {
            Ok((item_id, item_key, item_type, date_added, date_modified)) => {
                let metadata = self.item_metadata(item_id)?;
                let creators = self.item_creators(item_id)?;
                let tags = self.item_tags(item_id)?;
                Ok(Some(ZoteroItem {
                    item_id,
                    item_key,
                    item_type,
                    title: metadata.get("title").cloned().unwrap_or_default(),
                    date: metadata.get("date").cloned(),
                    doi: metadata.get("DOI").cloned(),
                    url: metadata.get("url").cloned(),
                    abstract_note: metadata.get("abstractNote").cloned(),
                    creators,
                    tags,
                    date_added,
                    date_modified,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Search items by title, DOI, or abstract (LIKE query).
    ///
    /// This is a brute-force scan (~8ms on 2700 items). For better performance,
    /// we could add FTS5, but this is already 100x faster than the BBT RPC path.
    pub fn search_items(&self, query: &str, limit: usize) -> Result<Vec<(i64, String)>> {
        // Escape LIKE wildcards in user input to prevent semantic injection
        let escaped = query.replace('%', "\\%").replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT i.itemID, i.key
             FROM items i
             JOIN itemData id ON i.itemID = id.itemID
             JOIN itemDataValues iv ON id.valueID = iv.valueID
             WHERE iv.value LIKE ?1 ESCAPE '\\'
             AND i.libraryID = 1
             AND i.itemID NOT IN (SELECT itemID FROM deletedItems)
             AND i.itemTypeID NOT IN (
                 SELECT itemTypeID FROM itemTypes
                 WHERE typeName IN ('attachment', 'note', 'annotation')
             )
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![pattern, limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get recently modified items.
    pub fn recent_items(&self, limit: usize) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT i.itemID, i.key
             FROM items i
             WHERE i.libraryID = 1
             AND i.itemID NOT IN (SELECT itemID FROM deletedItems)
             AND i.itemTypeID NOT IN (
                 SELECT itemTypeID FROM itemTypes
                 WHERE typeName IN ('attachment', 'note', 'annotation')
             )
             ORDER BY i.dateModified DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Item metadata (EAV assembly)
    // -----------------------------------------------------------------------

    /// Get all metadata fields for an item as a key-value map.
    ///
    /// Joins through the EAV schema: `itemData → fields + itemDataValues`.
    pub fn item_metadata(&self, item_id: i64) -> Result<HashMap<String, String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.fieldName, iv.value
             FROM itemData id
             JOIN fields f ON id.fieldID = f.fieldID
             JOIN itemDataValues iv ON id.valueID = iv.valueID
             WHERE id.itemID = ?1",
        )?;
        let rows = stmt.query_map([item_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(map)
    }

    /// Get creators (authors/editors) for an item, ordered.
    pub fn item_creators(&self, item_id: i64) -> Result<Vec<Creator>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.firstName, c.lastName, ct.creatorType, ic.orderIndex
             FROM itemCreators ic
             JOIN creators c ON ic.creatorID = c.creatorID
             JOIN creatorTypes ct ON ic.creatorTypeID = ct.creatorTypeID
             WHERE ic.itemID = ?1
             ORDER BY ic.orderIndex",
        )?;
        let rows = stmt.query_map([item_id], |row| {
            Ok(Creator {
                first_name: row.get(0)?,
                last_name: row.get(1)?,
                creator_type: row.get(2)?,
                order: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get tags for an item.
    pub fn item_tags(&self, item_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT t.name FROM tags t
             JOIN itemTags it ON t.tagID = it.tagID
             WHERE it.itemID = ?1",
        )?;
        let rows = stmt.query_map([item_id], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Collections
    // -----------------------------------------------------------------------

    /// List all collections with their hierarchy.
    pub fn collections(&self) -> Result<Vec<Collection>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.collectionID, c.key, c.collectionName,
                    pc.key as parentKey
             FROM collections c
             LEFT JOIN collections pc ON c.parentCollectionID = pc.collectionID
             WHERE c.libraryID = 1",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Collection {
                collection_id: row.get(0)?,
                key: row.get(1)?,
                name: row.get(2)?,
                parent_key: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get item keys in a collection.
    pub fn collection_items(
        &self,
        collection_key: &str,
        limit: usize,
    ) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT i.itemID, i.key
             FROM items i
             JOIN collectionItems ci ON i.itemID = ci.itemID
             JOIN collections c ON ci.collectionID = c.collectionID
             WHERE c.key = ?1 AND i.libraryID = 1
             AND i.itemID NOT IN (SELECT itemID FROM deletedItems)
             AND i.itemTypeID NOT IN (
                 SELECT itemTypeID FROM itemTypes
                 WHERE typeName IN ('attachment', 'note', 'annotation')
             )
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![collection_key, limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Attachments
    // -----------------------------------------------------------------------

    /// Get PDF attachments for an item.
    pub fn item_attachments(&self, item_id: i64) -> Result<Vec<Attachment>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT i.key, ia.contentType, ia.path,
                    (SELECT iv.value FROM itemData id2
                     JOIN itemDataValues iv ON id2.valueID = iv.valueID
                     JOIN fields f ON id2.fieldID = f.fieldID
                     WHERE id2.itemID = ia.itemID AND f.fieldName = 'title') as title
             FROM itemAttachments ia
             JOIN items i ON ia.itemID = i.itemID
             WHERE ia.parentItemID = ?1",
        )?;
        let rows = stmt.query_map([item_id], |row| {
            Ok(Attachment {
                item_key: row.get(0)?,
                content_type: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                path: row.get(2)?,
                title: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Notes
    // -----------------------------------------------------------------------

    /// Get HTML notes for an item.
    pub fn item_notes(&self, item_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT in2.note FROM itemNotes in2
             JOIN items i ON in2.itemID = i.itemID
             WHERE in2.parentItemID = ?1
             AND i.itemID NOT IN (SELECT itemID FROM deletedItems)",
        )?;
        let rows = stmt.query_map([item_id], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Count collections.
    pub fn collection_count(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM collections WHERE libraryID = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create an in-memory Zotero database with realistic test data.
    ///
    /// This mirrors the real Zotero schema closely enough to test our
    /// queries without needing the actual 71MB database file.
    fn test_zotero_db() -> ZoteroDb {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            -- Core tables
            CREATE TABLE libraries (libraryID INTEGER PRIMARY KEY);
            INSERT INTO libraries VALUES (1);

            CREATE TABLE itemTypes (itemTypeID INTEGER PRIMARY KEY, typeName TEXT);
            INSERT INTO itemTypes VALUES (1, 'journalArticle');
            INSERT INTO itemTypes VALUES (2, 'book');
            INSERT INTO itemTypes VALUES (14, 'attachment');
            INSERT INTO itemTypes VALUES (15, 'note');
            INSERT INTO itemTypes VALUES (29, 'annotation');

            CREATE TABLE fields (fieldID INTEGER PRIMARY KEY, fieldName TEXT);
            INSERT INTO fields VALUES (110, 'title');
            INSERT INTO fields VALUES (14, 'date');
            INSERT INTO fields VALUES (26, 'DOI');
            INSERT INTO fields VALUES (1, 'url');
            INSERT INTO fields VALUES (90, 'abstractNote');

            CREATE TABLE items (
                itemID INTEGER PRIMARY KEY, itemTypeID INT, dateAdded TEXT,
                dateModified TEXT, libraryID INT, key TEXT UNIQUE
            );
            INSERT INTO items VALUES (1, 1, '2024-01-01', '2024-06-15', 1, 'ABC12345');
            INSERT INTO items VALUES (2, 2, '2024-02-01', '2024-05-10', 1, 'DEF67890');
            INSERT INTO items VALUES (3, 14, '2024-01-01', '2024-01-01', 1, 'ATT00001');
            INSERT INTO items VALUES (4, 15, '2024-03-01', '2024-03-01', 1, 'NOTE0001');

            CREATE TABLE deletedItems (itemID INTEGER PRIMARY KEY, dateDeleted TEXT);

            CREATE TABLE itemDataValues (valueID INTEGER PRIMARY KEY, value TEXT UNIQUE);
            INSERT INTO itemDataValues VALUES (1, 'Hints on Test Data Selection');
            INSERT INTO itemDataValues VALUES (2, '1978');
            INSERT INTO itemDataValues VALUES (3, '10.1109/C-M.1978.218136');
            INSERT INTO itemDataValues VALUES (4, 'The Art of Testing');
            INSERT INTO itemDataValues VALUES (5, '2020');
            INSERT INTO itemDataValues VALUES (6, 'Abstract about mutation testing');

            CREATE TABLE itemData (itemID INT, fieldID INT, valueID INT, PRIMARY KEY (itemID, fieldID));
            INSERT INTO itemData VALUES (1, 110, 1);  -- title
            INSERT INTO itemData VALUES (1, 14, 2);   -- date
            INSERT INTO itemData VALUES (1, 26, 3);   -- DOI
            INSERT INTO itemData VALUES (1, 90, 6);   -- abstract
            INSERT INTO itemData VALUES (2, 110, 4);  -- title
            INSERT INTO itemData VALUES (2, 14, 5);   -- date

            -- Creators
            CREATE TABLE creatorTypes (creatorTypeID INTEGER PRIMARY KEY, creatorType TEXT);
            INSERT INTO creatorTypes VALUES (1, 'author');
            INSERT INTO creatorTypes VALUES (2, 'editor');

            CREATE TABLE creators (creatorID INTEGER PRIMARY KEY, firstName TEXT, lastName TEXT, fieldMode INT);
            INSERT INTO creators VALUES (1, 'Richard', 'DeMillo', 0);
            INSERT INTO creators VALUES (2, 'Richard', 'Lipton', 0);

            CREATE TABLE itemCreators (itemID INT, creatorID INT, creatorTypeID INT, orderIndex INT);
            INSERT INTO itemCreators VALUES (1, 1, 1, 0);
            INSERT INTO itemCreators VALUES (1, 2, 1, 1);

            -- Tags
            CREATE TABLE tags (tagID INTEGER PRIMARY KEY, name TEXT UNIQUE);
            INSERT INTO tags VALUES (1, 'mutation-testing');
            INSERT INTO tags VALUES (2, 'foundational');

            CREATE TABLE itemTags (itemID INT, tagID INT, type INT, PRIMARY KEY (itemID, tagID));
            INSERT INTO itemTags VALUES (1, 1, 0);
            INSERT INTO itemTags VALUES (1, 2, 0);

            -- Collections
            CREATE TABLE collections (
                collectionID INTEGER PRIMARY KEY, collectionName TEXT,
                parentCollectionID INT, libraryID INT, key TEXT UNIQUE
            );
            INSERT INTO collections VALUES (1, 'Mutation Testing', NULL, 1, 'COL00001');
            INSERT INTO collections VALUES (2, 'Foundational', 1, 1, 'COL00002');

            CREATE TABLE collectionItems (collectionID INT, itemID INT, orderIndex INT);
            INSERT INTO collectionItems VALUES (1, 1, 0);
            INSERT INTO collectionItems VALUES (1, 2, 1);

            -- Attachments
            CREATE TABLE itemAttachments (
                itemID INT PRIMARY KEY, parentItemID INT,
                linkMode INT, contentType TEXT, path TEXT,
                charsetID INT, syncState INT, storageModTime INT,
                storageHash TEXT, lastProcessedModificationTime INT
            );
            INSERT INTO itemAttachments VALUES (3, 1, 1, 'application/pdf', 'storage:DeMillo1978.pdf',
                                                 NULL, 0, NULL, NULL, NULL);

            -- Notes
            CREATE TABLE itemNotes (itemID INT PRIMARY KEY, parentItemID INT, note TEXT, title TEXT);
            INSERT INTO itemNotes VALUES (4, 1, '<p>Great foundational paper on mutation testing.</p>', '');
            ",
        )
        .unwrap();
        ZoteroDb { conn }
    }

    // -----------------------------------------------------------------------
    // Item queries
    // -----------------------------------------------------------------------

    #[test]
    fn item_count_excludes_attachments_and_notes() {
        let db = test_zotero_db();
        let count = db.item_count().unwrap();
        assert_eq!(count, 2); // Only journalArticle + book, not attachment/note
    }

    #[test]
    fn item_by_key_found() {
        let db = test_zotero_db();
        let item = db.item_by_key("ABC12345").unwrap().unwrap();
        assert_eq!(item.title, "Hints on Test Data Selection");
        assert_eq!(item.item_type, "journalArticle");
        assert_eq!(item.doi, Some("10.1109/C-M.1978.218136".into()));
        assert_eq!(item.date, Some("1978".into()));
        assert_eq!(
            item.abstract_note,
            Some("Abstract about mutation testing".into())
        );
    }

    #[test]
    fn item_by_key_not_found() {
        let db = test_zotero_db();
        assert!(db.item_by_key("ZZZZZZZZ").unwrap().is_none());
    }

    #[test]
    fn item_by_key_creators_ordered() {
        let db = test_zotero_db();
        let item = db.item_by_key("ABC12345").unwrap().unwrap();
        assert_eq!(item.creators.len(), 2);
        assert_eq!(item.creators[0].last_name, "DeMillo");
        assert_eq!(item.creators[1].last_name, "Lipton");
        assert_eq!(item.creators[0].order, 0);
        assert_eq!(item.creators[1].order, 1);
    }

    #[test]
    fn item_by_key_tags() {
        let db = test_zotero_db();
        let item = db.item_by_key("ABC12345").unwrap().unwrap();
        assert_eq!(item.tags.len(), 2);
        assert!(item.tags.contains(&"mutation-testing".to_string()));
        assert!(item.tags.contains(&"foundational".to_string()));
    }

    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    #[test]
    fn search_by_title() {
        let db = test_zotero_db();
        let results = db.search_items("Hints", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, "ABC12345");
    }

    #[test]
    fn search_by_doi() {
        let db = test_zotero_db();
        let results = db.search_items("10.1109", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_by_abstract() {
        let db = test_zotero_db();
        let results = db.search_items("mutation testing", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_no_results() {
        let db = test_zotero_db();
        let results = db.search_items("quantum computing", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_respects_limit() {
        let db = test_zotero_db();
        let results = db.search_items("t", 1).unwrap(); // matches both items
        assert_eq!(results.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Recent items
    // -----------------------------------------------------------------------

    #[test]
    fn recent_items_ordered_by_date_modified() {
        let db = test_zotero_db();
        let results = db.recent_items(10).unwrap();
        assert_eq!(results.len(), 2);
        // ABC12345 modified 2024-06-15, DEF67890 modified 2024-05-10
        assert_eq!(results[0].1, "ABC12345");
        assert_eq!(results[1].1, "DEF67890");
    }

    // -----------------------------------------------------------------------
    // Collections
    // -----------------------------------------------------------------------

    #[test]
    fn collections_with_hierarchy() {
        let db = test_zotero_db();
        let colls = db.collections().unwrap();
        assert_eq!(colls.len(), 2);
        let parent = colls.iter().find(|c| c.name == "Mutation Testing").unwrap();
        assert!(parent.parent_key.is_none());
        let child = colls.iter().find(|c| c.name == "Foundational").unwrap();
        assert_eq!(child.parent_key, Some("COL00001".into()));
    }

    #[test]
    fn collection_items_found() {
        let db = test_zotero_db();
        let items = db.collection_items("COL00001", 10).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn collection_items_not_found() {
        let db = test_zotero_db();
        let items = db.collection_items("ZZZZZZZZ", 10).unwrap();
        assert!(items.is_empty());
    }

    // -----------------------------------------------------------------------
    // Attachments
    // -----------------------------------------------------------------------

    #[test]
    fn item_attachments_found() {
        let db = test_zotero_db();
        let attachments = db.item_attachments(1).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].content_type, "application/pdf");
        assert_eq!(attachments[0].path, Some("storage:DeMillo1978.pdf".into()));
    }

    #[test]
    fn item_attachments_empty() {
        let db = test_zotero_db();
        let attachments = db.item_attachments(2).unwrap();
        assert!(attachments.is_empty());
    }

    // -----------------------------------------------------------------------
    // Notes
    // -----------------------------------------------------------------------

    #[test]
    fn item_notes_found() {
        let db = test_zotero_db();
        let notes = db.item_notes(1).unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("foundational paper"));
    }

    #[test]
    fn item_notes_empty() {
        let db = test_zotero_db();
        let notes = db.item_notes(2).unwrap();
        assert!(notes.is_empty());
    }

    // -----------------------------------------------------------------------
    // Collection count
    // -----------------------------------------------------------------------

    #[test]
    fn collection_count() {
        let db = test_zotero_db();
        assert_eq!(db.collection_count().unwrap(), 2);
    }
}
