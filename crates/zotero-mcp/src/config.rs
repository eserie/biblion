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
#[derive(Clone)]
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
    /// Whether write tools are enabled (default: false for safety).
    /// Set ZOTERO_MCP_ENABLE_WRITES=true to enable.
    pub writes_enabled: bool,
    /// Paper resolver configuration (sources, timeouts, etc.).
    /// Loaded from TOML config file if present, otherwise defaults.
    pub resolver: paper_resolver::ResolverConfig,
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

        let home = std::env::var("HOME").expect("HOME environment variable must be set");

        Self {
            zotero_sqlite_path: env_path(
                "ZOTERO_SQLITE_PATH",
                &format!("{home}/Zotero/zotero.sqlite"),
            ),
            zotero_storage_path: env_path("ZOTERO_STORAGE_PATH", &format!("{home}/Zotero/storage")),
            bbt_migrated_path: env_path(
                "BBT_MIGRATED_PATH",
                &format!("{home}/Zotero/better-bibtex.migrated"),
            ),
            zotero_api_key: std::env::var("ZOTERO_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            zotero_library_id: std::env::var("ZOTERO_LIBRARY_ID").unwrap_or_default(), // Must be set for write operations
            zotero_library_type: std::env::var("ZOTERO_LIBRARY_TYPE")
                .unwrap_or_else(|_| "user".into()),
            bbt_url: std::env::var("BBT_URL")
                .unwrap_or_else(|_| "http://localhost:23119/better-bibtex/json-rpc".into()),
            log_level: match std::env::var("ZOTERO_MCP_LOG").unwrap_or_default().as_str() {
                "debug" => LogLevel::Debug,
                "quiet" | "silent" => LogLevel::Quiet,
                _ => LogLevel::Info,
            },
            writes_enabled: std::env::var("ZOTERO_MCP_ENABLE_WRITES")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            resolver: load_resolver_config(),
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

/// Load resolver config from TOML file if present, otherwise defaults.
///
/// Looks for config at:
/// 1. `$ZOTERO_MCP_CONFIG` (if set)
/// 2. `~/.config/zotero-mcp/config.toml`
fn load_resolver_config() -> paper_resolver::ResolverConfig {
    let config_path = std::env::var("ZOTERO_MCP_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(format!("{home}/.config/zotero-mcp/config.toml"))
        });

    if !config_path.exists() {
        return paper_resolver::ResolverConfig::default();
    }

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[zotero-mcp] Warning: cannot read config file {}: {e}",
                config_path.display()
            );
            return paper_resolver::ResolverConfig::default();
        }
    };

    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "[zotero-mcp] Warning: invalid TOML in {}: {e}",
                config_path.display()
            );
            return paper_resolver::ResolverConfig::default();
        }
    };

    let mut config = paper_resolver::ResolverConfig::default();

    if let Some(resolver) = table.get("resolver").and_then(|v| v.as_table()) {
        if let Some(email) = resolver.get("email").and_then(|v| v.as_str()) {
            config.email = email.into();
        }
        if let Some(ua) = resolver.get("user_agent").and_then(|v| v.as_str()) {
            config.user_agent = ua.into();
        }
        if let Some(timeout) = resolver.get("timeout_secs").and_then(|v| v.as_integer()) {
            config.timeout_secs = timeout as u64;
        }

        // Source configuration — order in TOML = priority
        if let Some(sources) = resolver.get("sources").and_then(|v| v.as_array()) {
            config.sources = sources
                .iter()
                .filter_map(|s| {
                    let name = s.get("name")?.as_str()?.to_string();
                    let enabled = s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                    Some(paper_resolver::SourceEntry { name, enabled })
                })
                .collect();
        }

        // Extra blocked domains
        if let Some(blocked) = resolver.get("blocked_domains").and_then(|v| v.as_table())
            && let Some(extra) = blocked.get("extra").and_then(|v| v.as_array()) {
                config.extra_blocked_domains = extra
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
            }
    }

    config
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
            writes_enabled: false,
            resolver: paper_resolver::ResolverConfig::default(),
        };
        assert!(!config.has_write_access());
        assert!(
            config
                .zotero_sqlite_path
                .to_str()
                .unwrap()
                .ends_with("zotero.sqlite")
        );
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
            writes_enabled: false,
            resolver: paper_resolver::ResolverConfig::default(),
        };
        assert!(config.has_write_access());
    }
}
