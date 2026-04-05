//! Shared test helpers — in-memory SQLite fixtures for integration testing.

#![cfg(test)]

use rusqlite::Connection;

use crate::config::{Config, LogLevel};
use crate::db::DbPool;
use crate::db::bbt::BbtDb;
use crate::db::zotero::ZoteroDb;
use crate::server::ServerContext;

/// Create an in-memory Zotero database with realistic test data.
pub fn test_zotero_db() -> ZoteroDb {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "
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
        INSERT INTO fields VALUES (200, 'citationKey');

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
        INSERT INTO itemDataValues VALUES (7, 'demilloHintsTestData1978');
        INSERT INTO itemDataValues VALUES (8, 'artTesting2020');

        CREATE TABLE itemData (itemID INT, fieldID INT, valueID INT, PRIMARY KEY (itemID, fieldID));
        INSERT INTO itemData VALUES (1, 110, 1);
        INSERT INTO itemData VALUES (1, 14, 2);
        INSERT INTO itemData VALUES (1, 26, 3);
        INSERT INTO itemData VALUES (1, 90, 6);
        INSERT INTO itemData VALUES (1, 200, 7);  -- citationKey
        INSERT INTO itemData VALUES (2, 110, 4);
        INSERT INTO itemData VALUES (2, 14, 5);
        INSERT INTO itemData VALUES (2, 200, 8);  -- citationKey

        CREATE TABLE creatorTypes (creatorTypeID INTEGER PRIMARY KEY, creatorType TEXT);
        INSERT INTO creatorTypes VALUES (1, 'author');

        CREATE TABLE creators (creatorID INTEGER PRIMARY KEY, firstName TEXT, lastName TEXT, fieldMode INT);
        INSERT INTO creators VALUES (1, 'Richard A.', 'DeMillo', 0);
        INSERT INTO creators VALUES (2, 'Richard J.', 'Lipton', 0);

        CREATE TABLE itemCreators (itemID INT, creatorID INT, creatorTypeID INT, orderIndex INT);
        INSERT INTO itemCreators VALUES (1, 1, 1, 0);
        INSERT INTO itemCreators VALUES (1, 2, 1, 1);

        CREATE TABLE tags (tagID INTEGER PRIMARY KEY, name TEXT UNIQUE);
        INSERT INTO tags VALUES (1, 'mutation-testing');
        INSERT INTO tags VALUES (2, 'foundational');

        CREATE TABLE itemTags (itemID INT, tagID INT, type INT, PRIMARY KEY (itemID, tagID));
        INSERT INTO itemTags VALUES (1, 1, 0);
        INSERT INTO itemTags VALUES (1, 2, 0);

        CREATE TABLE collections (
            collectionID INTEGER PRIMARY KEY, collectionName TEXT,
            parentCollectionID INT, libraryID INT, key TEXT UNIQUE
        );
        INSERT INTO collections VALUES (1, 'Mutation Testing', NULL, 1, 'COL00001');

        CREATE TABLE collectionItems (collectionID INT, itemID INT, orderIndex INT);
        INSERT INTO collectionItems VALUES (1, 1, 0);

        CREATE TABLE itemAttachments (
            itemID INT PRIMARY KEY, parentItemID INT,
            linkMode INT, contentType TEXT, path TEXT,
            charsetID INT, syncState INT, storageModTime INT,
            storageHash TEXT, lastProcessedModificationTime INT
        );
        INSERT INTO itemAttachments VALUES (3, 1, 1, 'application/pdf', 'storage:DeMillo1978.pdf',
                                             NULL, 0, NULL, NULL, NULL);

        CREATE TABLE itemNotes (itemID INT PRIMARY KEY, parentItemID INT, note TEXT, title TEXT);
        INSERT INTO itemNotes VALUES (4, 1, '<p>Great foundational paper.</p>', '');
        ",
    )
    .unwrap();
    ZoteroDb::from_connection(conn)
}

/// Create a test ServerContext with in-memory databases.
pub fn test_ctx() -> ServerContext {
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
            zotero_api_key: None,
            zotero_library_id: "1".into(),
            zotero_library_type: "user".into(),
            bbt_url: "http://localhost:23119".into(),
            log_level: LogLevel::Quiet,
            writes_enabled: false,
            resolver: paper_resolver::ResolverConfig::default(),
        },
    }
}
