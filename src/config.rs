//! Configuration for the Zotero MCP server.
//!
//! # Design
//!
//! The server needs to know where three things live:
//!
//! 1. **`zotero.sqlite`** — Zotero's main database (71MB, ~7000 refs).
//!    This is where all item metadata, collections, tags, creators, and
//!    attachments are stored. We open it read-only.
//!
//! 2. **`better-bibtex.migrated`** — BBT's citekey mapping database.
//!    Contains a `citationkey` table mapping `(itemID, itemKey) → citekey`.
//!    This lets us resolve "demilloHintsTestData1978" to Zotero item key
//!    "9MS26VH5" without calling the BBT JSON-RPC API.
//!
//! 3. **Zotero storage** — directory where PDFs live, organized as
//!    `storage/{2-char}/{8-char}/filename.pdf`.
//!
//! For write operations, we also need the Zotero Web API key and the
//! BBT JSON-RPC URL (for BibTeX export only).
//!
//! # Environment variables
//!
//! Same variable names as the Python server for drop-in replacement:
//!
//! | Variable | Default |
//! |----------|---------|
//! | `ZOTERO_SQLITE_PATH` | `~/Zotero/zotero.sqlite` |
//! | `ZOTERO_STORAGE_PATH` | `~/Zotero/storage` |
//! | `BBT_MIGRATED_PATH` | `~/Zotero/better-bibtex.migrated` |
//! | `ZOTERO_API_KEY` | (none — writes disabled) |
//! | `ZOTERO_LIBRARY_ID` | `7292316` |
//! | `ZOTERO_LIBRARY_TYPE` | `user` |
//! | `BBT_URL` | `http://localhost:23119/better-bibtex/json-rpc` |
//! | `ZOTERO_MCP_LOG` | `info` |

use std::path::PathBuf;

/// Server configuration, loaded from environment variables.
///
/// All paths have sensible defaults for a standard macOS Zotero installation.
/// The API key is optional — without it, write tools return a clear error.
pub struct Config {
    /// Path to Zotero's main SQLite database.
    pub zotero_sqlite_path: PathBuf,
    /// Path to the Zotero storage directory (where PDFs live).
    pub zotero_storage_path: PathBuf,
    /// Path to BBT's migrated citekey database.
    pub bbt_migrated_path: PathBuf,
    /// Zotero Web API key (required for write operations).
    pub zotero_api_key: Option<String>,
    /// Zotero library ID (default: personal library).
    pub zotero_library_id: String,
    /// Zotero library type ("user" or "group").
    pub zotero_library_type: String,
    /// BBT JSON-RPC URL (only needed for BibTeX/bibliography export).
    pub bbt_url: String,
    /// Log level for stderr diagnostics.
    pub log_level: LogLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Quiet,
    Info,
    Debug,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Reads `.env` file if present (via dotenvy), then environment variables.
    /// All variables have sensible defaults except `ZOTERO_API_KEY`.
    pub fn from_env() -> Self {
        // Load .env file if present (ignore errors — it's optional)
        let _ = dotenvy::dotenv();

        let home = std::env::var("HOME")
            .expect("HOME environment variable must be set");

        Self {
            zotero_sqlite_path: env_path(
                "ZOTERO_SQLITE_PATH",
                &format!("{home}/Zotero/zotero.sqlite"),
            ),
            zotero_storage_path: env_path(
                "ZOTERO_STORAGE_PATH",
                &format!("{home}/Zotero/storage"),
            ),
            bbt_migrated_path: env_path(
                "BBT_MIGRATED_PATH",
                &format!("{home}/Zotero/better-bibtex.migrated"),
            ),
            zotero_api_key: std::env::var("ZOTERO_API_KEY").ok().filter(|s| !s.is_empty()),
            zotero_library_id: std::env::var("ZOTERO_LIBRARY_ID")
                .unwrap_or_else(|_| "7292316".into()),
            zotero_library_type: std::env::var("ZOTERO_LIBRARY_TYPE")
                .unwrap_or_else(|_| "user".into()),
            bbt_url: std::env::var("BBT_URL")
                .unwrap_or_else(|_| "http://localhost:23119/better-bibtex/json-rpc".into()),
            log_level: match std::env::var("ZOTERO_MCP_LOG")
                .unwrap_or_default()
                .as_str()
            {
                "debug" => LogLevel::Debug,
                "quiet" | "silent" => LogLevel::Quiet,
                _ => LogLevel::Info,
            },
        }
    }

    /// Whether write operations are available (API key is configured).
    pub fn has_write_access(&self) -> bool {
        self.zotero_api_key.is_some()
    }
}

fn env_path(var: &str, default: &str) -> PathBuf {
    std::env::var(var)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(default))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sensible_paths() {
        // Don't pollute the actual env — just test the structure
        let config = Config {
            zotero_sqlite_path: PathBuf::from("/Users/test/Zotero/zotero.sqlite"),
            zotero_storage_path: PathBuf::from("/Users/test/Zotero/storage"),
            bbt_migrated_path: PathBuf::from("/Users/test/Zotero/better-bibtex.migrated"),
            zotero_api_key: None,
            zotero_library_id: "7292316".into(),
            zotero_library_type: "user".into(),
            bbt_url: "http://localhost:23119/better-bibtex/json-rpc".into(),
            log_level: LogLevel::Info,
        };
        assert!(!config.has_write_access());
        assert!(config.zotero_sqlite_path.to_str().unwrap().ends_with("zotero.sqlite"));
    }

    #[test]
    fn config_with_api_key_has_write_access() {
        let config = Config {
            zotero_sqlite_path: PathBuf::from("/tmp/z.sqlite"),
            zotero_storage_path: PathBuf::from("/tmp/storage"),
            bbt_migrated_path: PathBuf::from("/tmp/bbt.migrated"),
            zotero_api_key: Some("test-key".into()),
            zotero_library_id: "1".into(),
            zotero_library_type: "user".into(),
            bbt_url: "http://localhost:23119/better-bibtex/json-rpc".into(),
            log_level: LogLevel::Quiet,
        };
        assert!(config.has_write_access());
    }
}
