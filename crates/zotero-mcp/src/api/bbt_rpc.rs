//! Better BibTeX JSON-RPC client.
//!
//! # When is this used?
//!
//! Only for 3 tools that need BBT's CSL formatting engine:
//! - `zotero_get_bibtex` — export items as BibTeX/BibLaTeX
//! - `zotero_get_bibliography` — formatted bibliography (APA, IEEE, etc.)
//! - `zotero_export_bibtex` — export a collection as BibTeX file
//!
//! All other read operations bypass BBT entirely via direct SQLite access.
//!
//! # Protocol
//!
//! BBT exposes a JSON-RPC 2.0 API on `http://localhost:23119/better-bibtex/json-rpc`.
//! It runs inside Zotero's Electron process, so it requires Zotero to be open.

use anyhow::{Context, Result};
use serde_json::{json, Value};

/// Blocking client for BBT JSON-RPC API.
///
/// Uses a persistent `reqwest::blocking::Client` to reuse TCP connections.
/// Only instantiated when a BibTeX/bibliography tool is called.
pub struct BbtRpcClient {
    client: reqwest::blocking::Client,
    url: String,
    next_id: std::cell::Cell<u64>,
}

impl BbtRpcClient {
    pub fn new(url: &str) -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
            url: url.into(),
            next_id: std::cell::Cell::new(1),
        }
    }

    /// Make a JSON-RPC call to BBT.
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);

        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let resp = self
            .client
            .post(&self.url)
            .json(&payload)
            .send()
            .with_context(|| {
                "Cannot connect to Zotero. Ensure Zotero is running with Better BibTeX installed."
            })?;

        let body: Value = resp.json().with_context(|| "Invalid JSON response from BBT")?;

        if let Some(error) = body.get("error") {
            let msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown BBT error");
            anyhow::bail!("BBT RPC error: {msg}");
        }

        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Export items as BibTeX/BibLaTeX.
    ///
    /// `translator`: "Better BibTeX", "Better BibLaTeX", "Better CSL JSON"
    pub fn export(&self, citekeys: &[&str], translator: &str) -> Result<String> {
        let result = self.call("item.export", json!([citekeys, translator]))?;
        match result {
            Value::String(s) => Ok(s),
            _ => Ok(result.to_string()),
        }
    }

    /// Generate formatted bibliography.
    ///
    /// `style`: CSL style URL, e.g. "http://www.zotero.org/styles/apa"
    pub fn bibliography(&self, citekeys: &[&str], style: &str) -> Result<String> {
        let result = self.call("item.bibliography", json!([citekeys, {"id": style}]))?;
        match result {
            Value::String(s) => Ok(s),
            _ => Ok(result.to_string()),
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creates_with_default_url() {
        let client = BbtRpcClient::new("http://localhost:23119/better-bibtex/json-rpc");
        assert_eq!(client.url, "http://localhost:23119/better-bibtex/json-rpc");
    }

    #[test]
    fn call_to_unreachable_server_returns_error() {
        let client = BbtRpcClient::new("http://127.0.0.1:1/nonexistent");
        let result = client.export(&["test2024"], "Better BibTeX");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Cannot connect") || err.contains("error"),
            "Unexpected error: {err}"
        );
    }
}
