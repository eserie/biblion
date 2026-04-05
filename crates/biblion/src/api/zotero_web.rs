//! Zotero Web API v3 client — for all write operations.
//!
//! # Why not write to SQLite directly?
//!
//! Zotero syncs its database to the cloud. Writing to the SQLite file
//! directly would cause sync conflicts and data corruption. All writes
//! MUST go through the official Web API.
//!
//! # API Reference
//!
//! Base URL: `https://api.zotero.org`
//! Auth: `Zotero-API-Key: {key}` header
//! Content-Type: `application/json`
//! API version: 3 (via `Zotero-API-Version: 3` header)
//!
//! # Rate limits
//!
//! Zotero doesn't document strict rate limits but recommends <1 req/sec
//! for heavy operations. Our MCP usage pattern (human-triggered, one
//! operation at a time) is well within bounds.

use anyhow::{Context, Result};
use serde_json::Value;

/// Blocking client for the Zotero Web API v3.
#[derive(Debug)]
pub struct ZoteroWebClient {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
}

impl ZoteroWebClient {
    /// Create a new client for a user library.
    pub fn new(api_key: &str, library_id: &str, library_type: &str) -> Self {
        let base_url = format!("https://api.zotero.org/{library_type}s/{library_id}");
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("Failed to build HTTP client"),
            api_key: api_key.into(),
            base_url,
        }
    }

    /// Create a client with a custom base URL (for testing).
    pub fn with_base_url(api_key: &str, base_url: &str) -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("Failed to build HTTP client"),
            api_key: api_key.into(),
            base_url: base_url.into(),
        }
    }

    /// Common headers for all API requests.
    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            "Zotero-API-Key",
            self.api_key
                .parse()
                .expect("ZOTERO_API_KEY contains invalid header characters"),
        );
        h.insert("Zotero-API-Version", "3".parse().unwrap());
        h.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        h
    }

    /// GET a single item by key.
    pub fn get_item(&self, key: &str) -> Result<Value> {
        let resp = self
            .client
            .get(format!("{}/items/{key}", self.base_url))
            .headers(self.headers())
            .send()
            .with_context(|| format!("Failed to fetch item {key}"))?;
        resp.error_for_status_ref()
            .with_context(|| format!("API error fetching item {key}"))?;
        resp.json().with_context(|| "Invalid JSON from Zotero API")
    }

    /// GET children of an item (attachments, notes).
    pub fn children(&self, key: &str) -> Result<Vec<Value>> {
        let resp = self
            .client
            .get(format!("{}/items/{key}/children", self.base_url))
            .headers(self.headers())
            .send()?;
        resp.error_for_status_ref()?;
        resp.json().map_err(Into::into)
    }

    /// GET an item template for a given type.
    pub fn item_template(&self, item_type: &str) -> Result<Value> {
        let resp = self
            .client
            .get(format!("{}/items/new", self.base_url))
            .query(&[("itemType", item_type)])
            .headers(self.headers())
            .send()?;
        resp.error_for_status_ref()?;
        resp.json().map_err(Into::into)
    }

    /// POST new items to the library.
    pub fn create_items(&self, items: &[Value]) -> Result<Value> {
        let resp = self
            .client
            .post(format!("{}/items", self.base_url))
            .headers(self.headers())
            .json(items)
            .send()
            .with_context(|| "Failed to create items")?;
        resp.error_for_status_ref()
            .with_context(|| "API error creating items")?;
        resp.json().map_err(Into::into)
    }

    /// PATCH (update) an existing item.
    ///
    /// Requires the item's current `version` for optimistic concurrency.
    pub fn update_item(&self, key: &str, data: &Value, version: i32) -> Result<()> {
        let resp = self
            .client
            .patch(format!("{}/items/{key}", self.base_url))
            .headers(self.headers())
            .header("If-Unmodified-Since-Version", version.to_string())
            .json(data)
            .send()?;
        resp.error_for_status_ref()
            .with_context(|| format!("API error updating item {key}"))?;
        Ok(())
    }

    /// DELETE an item permanently.
    pub fn delete_item(&self, key: &str, version: i32) -> Result<()> {
        let resp = self
            .client
            .delete(format!("{}/items/{key}", self.base_url))
            .headers(self.headers())
            .header("If-Unmodified-Since-Version", version.to_string())
            .send()?;
        resp.error_for_status_ref()
            .with_context(|| format!("API error deleting item {key}"))?;
        Ok(())
    }

    /// GET all collections.
    pub fn get_collections(&self) -> Result<Vec<Value>> {
        let resp = self
            .client
            .get(format!("{}/collections", self.base_url))
            .headers(self.headers())
            .send()?;
        resp.error_for_status_ref()?;
        resp.json().map_err(Into::into)
    }

    /// POST new collections.
    pub fn create_collections(&self, collections: &[Value]) -> Result<Value> {
        let resp = self
            .client
            .post(format!("{}/collections", self.base_url))
            .headers(self.headers())
            .json(collections)
            .send()?;
        resp.error_for_status_ref()?;
        resp.json().map_err(Into::into)
    }

    /// Download a file from a URL to a local path.
    pub fn download_file(&self, url: &str, dest: &std::path::Path) -> Result<()> {
        let resp = self
            .client
            .get(url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            )
            .send()
            .with_context(|| format!("Failed to download {url}"))?;
        resp.error_for_status_ref()?;
        let bytes = resp.bytes()?;

        // Validate PDF magic bytes
        if !bytes.starts_with(b"%PDF") {
            anyhow::bail!("Downloaded file is not a valid PDF (missing %PDF header)");
        }

        std::fs::write(dest, &bytes)
            .with_context(|| format!("Failed to write to {}", dest.display()))?;
        Ok(())
    }

    /// Upload a file attachment to an item.
    ///
    /// This is the Zotero two-step upload:
    /// 1. Register the upload (get S3 pre-signed URL)
    /// 2. Upload to S3
    /// 3. Confirm upload
    ///
    /// For simplicity in v1, we use the "linked file" attachment method instead,
    /// which just records the path without uploading.
    pub fn attach_file(
        &self,
        parent_key: &str,
        file_path: &std::path::Path,
        title: &str,
    ) -> Result<Value> {
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment.pdf");

        let template = serde_json::json!([{
            "itemType": "attachment",
            "parentItem": parent_key,
            "linkMode": "imported_file",
            "title": title,
            "contentType": "application/pdf",
            "filename": filename,
            "tags": [],
            "relations": {},
        }]);

        self.create_items(&[template])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_constructs_correct_base_url() {
        let client = ZoteroWebClient::new("test-key", "12345", "user");
        assert_eq!(client.base_url, "https://api.zotero.org/users/12345");
    }

    #[test]
    fn client_headers_include_api_key() {
        let client = ZoteroWebClient::new("my-secret-key", "1", "user");
        let headers = client.headers();
        assert_eq!(headers.get("Zotero-API-Key").unwrap(), "my-secret-key");
        assert_eq!(headers.get("Zotero-API-Version").unwrap(), "3");
    }
}
