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
    ///
    /// Note: the template endpoint is global (`/items/new`), not scoped
    /// to a user library. We use the API root, not `self.base_url`.
    pub fn item_template(&self, item_type: &str) -> Result<Value> {
        let api_root = if self.base_url.contains("api.zotero.org") {
            "https://api.zotero.org"
        } else {
            // Testing with custom base URL — use it as-is
            &self.base_url
        };
        let resp = self
            .client
            .get(format!("{api_root}/items/new"))
            .query(&[("itemType", item_type)])
            .headers(self.headers())
            .send()?;
        resp.error_for_status_ref()
            .with_context(|| format!("Failed to get template for itemType '{item_type}'"))?;
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
    /// Implements the full Zotero file upload protocol:
    /// 1. Create attachment item metadata
    /// 2. Get upload authorization (with file hash)
    /// 3. Upload file bytes to S3
    /// 4. Register the upload with Zotero
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

        let file_bytes = std::fs::read(file_path)
            .with_context(|| format!("Failed to read {}", file_path.display()))?;
        let file_md5 = format!("{:x}", md5::compute(&file_bytes));
        let filesize = file_bytes.len();
        let mtime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        // Step 1: Create attachment item
        let template = serde_json::json!({
            "itemType": "attachment",
            "parentItem": parent_key,
            "linkMode": "imported_file",
            "title": title,
            "contentType": "application/pdf",
            "filename": filename,
            "tags": [],
            "relations": {},
        });

        let create_resp = self
            .create_items(&[template])
            .with_context(|| "Failed to create attachment item")?;

        let attachment_key = create_resp
            .pointer("/successful/0/key")
            .or_else(|| create_resp.pointer("/successful/0/data/key"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("No attachment key in API response: {create_resp}"))?
            .to_string();

        let attachment_version = create_resp
            .pointer("/successful/0/version")
            .or_else(|| create_resp.pointer("/successful/0/data/version"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        // Step 2: Get upload authorization
        let auth = match self.get_upload_authorization(
            &attachment_key,
            &file_md5,
            filename,
            filesize,
            mtime,
        ) {
            Ok(a) => a,
            Err(e) => {
                let _ = self.delete_item(&attachment_key, attachment_version);
                return Err(e.context("Failed to get upload authorization"));
            }
        };

        // If file already exists on server, we're done
        if auth.get("exists").and_then(|v| v.as_i64()) == Some(1) {
            return Ok(create_resp);
        }

        // Step 3: Upload to S3
        let url = auth["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' in upload auth response"))?;
        let content_type = auth["contentType"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'contentType' in upload auth response"))?;
        let prefix = auth["prefix"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'prefix' in upload auth response"))?;
        let suffix = auth["suffix"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'suffix' in upload auth response"))?;
        let upload_key = auth["uploadKey"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'uploadKey' in upload auth response"))?;

        if let Err(e) = self.upload_to_s3(url, content_type, prefix, &file_bytes, suffix) {
            let _ = self.delete_item(&attachment_key, attachment_version);
            return Err(e.context("Failed to upload file to storage"));
        }

        // Step 4: Register upload
        if let Err(e) = self.register_upload(&attachment_key, upload_key) {
            let _ = self.delete_item(&attachment_key, attachment_version);
            return Err(e.context("Failed to register upload"));
        }

        Ok(create_resp)
    }

    /// Step 2: Get upload authorization from Zotero.
    fn get_upload_authorization(
        &self,
        key: &str,
        md5: &str,
        filename: &str,
        filesize: usize,
        mtime: u128,
    ) -> Result<Value> {
        let mut headers = self.headers();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        headers.insert("If-None-Match", "*".parse().unwrap());

        let body = format!("md5={md5}&filename={filename}&filesize={filesize}&mtime={mtime}");

        let resp = self
            .client
            .post(format!("{}/items/{key}/file", self.base_url))
            .headers(headers)
            .body(body)
            .send()
            .with_context(|| format!("Upload auth request failed for {key}"))?;
        resp.error_for_status_ref()
            .with_context(|| format!("Upload auth rejected for {key}"))?;
        resp.json().map_err(Into::into)
    }

    /// Step 3: Upload file bytes to S3 pre-signed URL.
    fn upload_to_s3(
        &self,
        url: &str,
        content_type: &str,
        prefix: &str,
        file_bytes: &[u8],
        suffix: &str,
    ) -> Result<()> {
        let mut body = Vec::with_capacity(prefix.len() + file_bytes.len() + suffix.len());
        body.extend_from_slice(prefix.as_bytes());
        body.extend_from_slice(file_bytes);
        body.extend_from_slice(suffix.as_bytes());

        let resp = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .with_context(|| "S3 upload request failed")?;
        resp.error_for_status_ref()
            .with_context(|| "S3 upload rejected")?;
        Ok(())
    }

    /// Step 4: Register a completed upload with Zotero.
    fn register_upload(&self, key: &str, upload_key: &str) -> Result<()> {
        let mut headers = self.headers();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        headers.insert("If-None-Match", "*".parse().unwrap());

        let resp = self
            .client
            .post(format!("{}/items/{key}/file", self.base_url))
            .headers(headers)
            .body(format!("upload={upload_key}"))
            .send()
            .with_context(|| format!("Register upload failed for {key}"))?;
        resp.error_for_status_ref()
            .with_context(|| format!("Register upload rejected for {key}"))?;
        Ok(())
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

#[cfg(test)]
mod upload_tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_test_pdf(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("test.pdf");
        std::fs::write(&path, b"%PDF-1.4 test content").unwrap();
        path
    }

    #[tokio::test]
    async fn upload_flow_success() {
        let server = MockServer::start().await;
        let uri = server.uri();

        // Step 1: Create attachment item
        Mock::given(method("POST"))
            .and(path("/items"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "successful": {
                    "0": { "key": "ATT001", "version": 1, "data": { "key": "ATT001", "version": 1 } }
                },
                "unchanged": {},
                "failed": {}
            })))
            .expect(1)
            .mount(&server)
            .await;

        // Step 2: Upload authorization
        let s3_url = format!("{uri}/s3-upload");
        Mock::given(method("POST"))
            .and(path_regex(r"/items/ATT001/file"))
            .and(body_string_contains("md5="))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": s3_url,
                "contentType": "application/pdf",
                "prefix": "PREFIX",
                "suffix": "SUFFIX",
                "uploadKey": "upload-key-123"
            })))
            .expect(1)
            .mount(&server)
            .await;

        // Step 3: S3 upload
        Mock::given(method("POST"))
            .and(path("/s3-upload"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;

        // Step 4: Register upload
        Mock::given(method("POST"))
            .and(path_regex(r"/items/ATT001/file"))
            .and(body_string_contains("upload="))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let pdf = make_test_pdf(tmp.path());
        let result = tokio::task::spawn_blocking(move || {
            let client = ZoteroWebClient::with_base_url("test-key", &uri);
            client.attach_file("PARENT01", &pdf, "Test Paper")
        })
        .await
        .unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn upload_flow_file_exists() {
        let server = MockServer::start().await;
        let uri = server.uri();

        // Step 1: Create attachment
        Mock::given(method("POST"))
            .and(path("/items"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "successful": {
                    "0": { "key": "ATT002", "version": 1, "data": { "key": "ATT002", "version": 1 } }
                },
                "unchanged": {},
                "failed": {}
            })))
            .mount(&server)
            .await;

        // Step 2: File already exists
        Mock::given(method("POST"))
            .and(path_regex(r"/items/ATT002/file"))
            .and(body_string_contains("md5="))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "exists": 1
            })))
            .mount(&server)
            .await;

        // S3 should NOT be called
        Mock::given(method("POST"))
            .and(path("/s3-upload"))
            .respond_with(ResponseTemplate::new(201))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let pdf = make_test_pdf(tmp.path());
        let result = tokio::task::spawn_blocking(move || {
            let client = ZoteroWebClient::with_base_url("test-key", &uri);
            client.attach_file("PARENT02", &pdf, "Test Paper")
        })
        .await
        .unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn upload_flow_s3_failure_cleans_up() {
        let server = MockServer::start().await;
        let uri = server.uri();

        // Step 1: Create attachment
        Mock::given(method("POST"))
            .and(path("/items"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "successful": {
                    "0": { "key": "ATT003", "version": 1, "data": { "key": "ATT003", "version": 1 } }
                },
                "unchanged": {},
                "failed": {}
            })))
            .mount(&server)
            .await;

        // Step 2: Authorization succeeds
        let s3_url = format!("{uri}/s3-upload");
        Mock::given(method("POST"))
            .and(path_regex(r"/items/ATT003/file"))
            .and(body_string_contains("md5="))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": s3_url,
                "contentType": "application/pdf",
                "prefix": "P",
                "suffix": "S",
                "uploadKey": "uk"
            })))
            .mount(&server)
            .await;

        // Step 3: S3 fails
        Mock::given(method("POST"))
            .and(path("/s3-upload"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        // Cleanup: DELETE the orphan attachment
        Mock::given(method("DELETE"))
            .and(path_regex(r"/items/ATT003"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let pdf = make_test_pdf(tmp.path());
        let result = tokio::task::spawn_blocking(move || {
            let client = ZoteroWebClient::with_base_url("test-key", &uri);
            client.attach_file("PARENT03", &pdf, "Test Paper")
        })
        .await
        .unwrap();
        assert!(result.is_err());
    }
}
