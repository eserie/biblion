//! External API clients — network calls for operations that can't be done
//! via local SQLite alone.
//!
//! # When do we need network?
//!
//! - **BBT RPC** (3 tools): BibTeX/BibLaTeX export and formatted bibliography.
//!   BBT's CSL engine produces properly formatted citations — we don't reimplement it.
//!
//! - **Zotero Web API** (14 tools): All write operations. We never write to
//!   Zotero's SQLite directly — that would break sync.
//!
//! - **PDF resolver** (2 tools): Querying 9 academic APIs to find open-access PDFs.
//!
//! All network calls use `reqwest::blocking::Client` (persistent connection pooling).
//! The PDF resolver uses `tokio` for concurrent requests.

pub mod bbt_rpc;
pub mod zotero_web;
