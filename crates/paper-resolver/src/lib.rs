//! Concurrent PDF resolver for academic papers — 9 open-access sources.
//!
//! # What it does
//!
//! Given a DOI, URL, or title, queries 9 academic sources in parallel and
//! returns the best downloadable PDF URL. No Zotero, no reference manager
//! dependency — just `(doi, url, title) → Option<ResolvedPdf>`.
//!
//! # Sources (by priority)
//!
//! 1. **arXiv** — instant DOI/URL pattern matching (no network)
//! 2. **OpenAlex** — 250M+ works, structured OA location data
//! 3. **CORE** — 300M+ OA works from institutional repos
//! 4. **Google Scholar** — widest coverage, university mirrors (risk of rate-limit)
//! 5. **Unpaywall** — 30M+ OA articles via DOI
//! 6. **Crossref** — publisher PDF links via DOI
//! 7. **Zenodo** — cross-disciplinary preprints (CERN)
//! 8. **SSRN** — finance/economics preprints
//! 9. **Semantic Scholar** — OA PDFs + disclaimer field parsing
//!
//! # Usage
//!
//! ```no_run
//! // Sync (creates its own tokio runtime):
//! let result = paper_resolver::resolve_pdf(
//!     Some("10.1109/TSE.2010.62"), None, Some("mutation testing"),
//! );
//! ```

use regex::Regex;
use std::sync::LazyLock;

static ARXIV_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"arxiv\.org/(?:abs|pdf)/(\d{4}\.\d{4,5}(?:v\d+)?)").unwrap());
static PDF_HREF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"href="(https?://[^"]+\.pdf)""#).unwrap());
static SSRN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"href="(https?://papers\.ssrn\.com/sol3/papers\.cfm\?abstract_id=\d+)""#).unwrap()
});
static URL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s,)]+").unwrap());

/// Shared tokio runtime for PDF resolution. Created once, reused across calls.
///
/// Uses `new_multi_thread` (not `new_current_thread`) because in SSE mode,
/// multiple concurrent requests may call `resolve_pdf()` from different
/// `spawn_blocking` threads. A current-thread runtime would deadlock or panic
/// when a second `block_on()` is called on the same runtime.
static PDF_RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

fn pdf_runtime() -> &'static tokio::runtime::Runtime {
    PDF_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to build PDF tokio runtime")
    })
}

/// Result of PDF URL resolution.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolvedPdf {
    pub url: String,
    pub source: String,
    pub downloadable: bool,
}

/// A source entry — name + enabled flag.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SourceEntry {
    pub name: String,
    pub enabled: bool,
}

impl SourceEntry {
    pub fn new(name: impl Into<String>, enabled: bool) -> Self {
        Self {
            name: name.into(),
            enabled,
        }
    }
}

/// Base URLs for each source — overridable for testing.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Endpoints {
    pub openalex: String,
    pub core: String,
    pub google_scholar: String,
    pub unpaywall: String,
    pub crossref: String,
    pub zenodo: String,
    pub ssrn: String,
    pub semantic_scholar: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            openalex: "https://api.openalex.org".into(),
            core: "https://api.core.ac.uk/v3".into(),
            google_scholar: "https://scholar.google.com".into(),
            unpaywall: "https://api.unpaywall.org/v2".into(),
            crossref: "https://api.crossref.org".into(),
            zenodo: "https://zenodo.org/api".into(),
            ssrn: "https://papers.ssrn.com".into(),
            semantic_scholar: "https://api.semanticscholar.org/graph/v1".into(),
        }
    }
}

/// Configuration for the paper resolver.
///
/// Controls which sources are queried, their priority (order in the vec),
/// timeouts, and API identification. Callers construct this from their
/// own config files (TOML, env vars, etc.) — paper-resolver has no
/// file I/O dependency.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolverConfig {
    /// Email for Unpaywall/Crossref polite pool (required by their ToS).
    pub email: String,
    /// User-Agent string for HTTP requests.
    pub user_agent: String,
    /// HTTP request timeout in seconds.
    pub timeout_secs: u64,
    /// Ordered list of sources. Position = priority (first = highest).
    /// Disabled sources are skipped.
    pub sources: Vec<SourceEntry>,
    /// Extra domains to treat as non-downloadable (appended to defaults).
    pub extra_blocked_domains: Vec<String>,
    /// Base URLs for each source — override for testing with mock servers.
    pub endpoints: Endpoints,
}

/// All available source names.
pub const SOURCE_NAMES: &[&str] = &[
    "arxiv",
    "openalex",
    "core",
    "google_scholar",
    "unpaywall",
    "crossref",
    "zenodo",
    "ssrn",
    "semantic_scholar",
];

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            email: "biblion@example.com".into(),
            user_agent: "biblion/0.1".into(),
            timeout_secs: 20,
            sources: SOURCE_NAMES
                .iter()
                .map(|&name| SourceEntry {
                    name: name.into(),
                    enabled: true,
                })
                .collect(),
            extra_blocked_domains: vec![],
            endpoints: Endpoints::default(),
        }
    }
}

impl ResolverConfig {
    /// Check if a source is enabled by name.
    pub fn is_enabled(&self, name: &str) -> bool {
        self.sources.iter().any(|s| s.name == name && s.enabled)
    }

    /// Get the priority (position index) for a source.
    pub fn priority(&self, name: &str) -> u8 {
        self.sources
            .iter()
            .position(|s| s.name == name)
            .map(|p| (p + 1) as u8)
            .unwrap_or(99)
    }
}

/// Default domains known to block programmatic downloads.
const BLOCKED_DOMAINS: &[&str] = &[
    "academic.oup.com",
    "wiley.com",
    "www.sciencedirect.com",
    "link.springer.com",
    "www.nature.com",
    "www.tandfonline.com",
    "ieeexplore.ieee.org",
    "journals.sagepub.com",
    "silverchair.com",
];

fn is_downloadable(url: &str) -> bool {
    !BLOCKED_DOMAINS.iter().any(|d| url.contains(d))
}

/// Config-aware version that also checks extra_blocked_domains.
fn is_downloadable_cfg(url: &str, config: &ResolverConfig) -> bool {
    if !is_downloadable(url) {
        return false;
    }
    !config
        .extra_blocked_domains
        .iter()
        .any(|d| url.contains(d.as_str()))
}

/// Resolve a PDF URL using all available sources (default config).
///
/// Convenience wrapper that uses [`ResolverConfig::default()`].
/// For custom configuration, use [`resolve_pdf_with_config`].
pub fn resolve_pdf(
    doi: Option<&str>,
    url: Option<&str>,
    title: Option<&str>,
) -> Option<ResolvedPdf> {
    resolve_pdf_with_config(doi, url, title, &ResolverConfig::default())
}

/// Resolve a PDF URL with custom configuration.
///
/// Sync version — creates a tokio runtime internally.
/// For async callers, use [`resolve_pdf_async`].
pub fn resolve_pdf_with_config(
    doi: Option<&str>,
    url: Option<&str>,
    title: Option<&str>,
    config: &ResolverConfig,
) -> Option<ResolvedPdf> {
    // 1. arXiv — instant, no network
    if config.is_enabled("arxiv") {
        if let Some(doi) = doi
            && let Some(id) = doi_to_arxiv_id(doi)
        {
            return Some(ResolvedPdf {
                url: format!("https://arxiv.org/pdf/{id}.pdf"),
                source: "arxiv".into(),
                downloadable: true,
            });
        }
        if let Some(url) = url
            && let Some(id) = url_to_arxiv_id(url)
        {
            return Some(ResolvedPdf {
                url: format!("https://arxiv.org/pdf/{id}.pdf"),
                source: "arxiv".into(),
                downloadable: true,
            });
        }
    }

    // 2-9. Concurrent HTTP queries via tokio (shared runtime)
    pdf_runtime().block_on(resolve_pdf_async(doi, url, title, config))
}

/// Async version with configuration — caller owns the tokio runtime.
///
/// All enabled sources are queried concurrently. Disabled sources are skipped.
/// Source priority is determined by position in `config.sources` (first = highest).
pub async fn resolve_pdf_async(
    doi: Option<&str>,
    url: Option<&str>,
    title: Option<&str>,
    config: &ResolverConfig,
) -> Option<ResolvedPdf> {
    // arXiv from URL (same as sync path)
    if config.is_enabled("arxiv")
        && let Some(url) = url
        && let Some(id) = url_to_arxiv_id(url)
    {
        return Some(ResolvedPdf {
            url: format!("https://arxiv.org/pdf/{id}.pdf"),
            source: "arxiv".into(),
            downloadable: true,
        });
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .ok()?;

    // Fire enabled sources concurrently via join_all pattern
    type PdfFuture<'a> =
        std::pin::Pin<Box<dyn std::future::Future<Output = Option<(u8, ResolvedPdf)>> + Send + 'a>>;
    let mut futures: Vec<PdfFuture<'_>> = Vec::new();

    let ep = &config.endpoints;
    for source in &config.sources {
        if !source.enabled {
            continue;
        }
        let pri = config.priority(&source.name);
        let c = &client;
        match source.name.as_str() {
            "arxiv" => {} // Already handled synchronously above
            "openalex" => futures.push(Box::pin(async move {
                try_openalex(c, doi, title, ep).await.map(|r| (pri, r))
            })),
            "core" => futures.push(Box::pin(async move {
                try_core(c, doi, title, ep).await.map(|r| (pri, r))
            })),
            "google_scholar" => futures.push(Box::pin(async move {
                try_google_scholar(c, title, ep).await.map(|r| (pri, r))
            })),
            "unpaywall" => {
                let email = config.email.clone();
                futures.push(Box::pin(async move {
                    try_unpaywall(c, doi, &email, ep).await.map(|r| (pri, r))
                }))
            }
            "crossref" => {
                let email = config.email.clone();
                let ua = config.user_agent.clone();
                futures.push(Box::pin(async move {
                    try_crossref(c, doi, &email, &ua, ep)
                        .await
                        .map(|r| (pri, r))
                }))
            }
            "zenodo" => futures.push(Box::pin(async move {
                try_zenodo(c, title, ep).await.map(|r| (pri, r))
            })),
            "ssrn" => futures.push(Box::pin(async move {
                try_ssrn(c, title, ep).await.map(|r| (pri, r))
            })),
            "semantic_scholar" => futures.push(Box::pin(async move {
                try_semantic_scholar(c, doi, title, ep)
                    .await
                    .map(|r| (pri, r))
            })),
            _ => {} // Unknown source name, skip
        }
    }

    let results = futures::future::join_all(futures).await;

    // Collect successful results and apply config-aware downloadability check
    let mut candidates: Vec<(u8, ResolvedPdf)> = results
        .into_iter()
        .flatten()
        .map(|(pri, mut r)| {
            // Re-check downloadability with extra_blocked_domains from config
            if r.downloadable {
                r.downloadable = is_downloadable_cfg(&r.url, config);
            }
            (pri, r)
        })
        .collect();

    // Prefer downloadable, then highest priority (lowest number)
    candidates.sort_by_key(|(pri, r)| (!r.downloadable, *pri));
    candidates.into_iter().next().map(|(_, r)| r)
}

// ---------------------------------------------------------------------------
// arXiv helpers
// ---------------------------------------------------------------------------

fn doi_to_arxiv_id(doi: &str) -> Option<String> {
    doi.strip_prefix("10.48550/arXiv.").map(String::from)
}

fn url_to_arxiv_id(url: &str) -> Option<String> {
    ARXIV_RE.captures(url).map(|c| c[1].to_string())
}

// ---------------------------------------------------------------------------
// OpenAlex
// ---------------------------------------------------------------------------

async fn try_openalex(
    client: &reqwest::Client,
    doi: Option<&str>,
    title: Option<&str>,
    endpoints: &Endpoints,
) -> Option<ResolvedPdf> {
    let resp = if let Some(doi) = doi {
        client
            .get(format!("{}/works/doi:{doi}", endpoints.openalex))
            .query(&[("select", "open_access,locations,best_oa_location")])
            .send()
            .await
            .ok()?
    } else {
        let title = title?;
        client
            .get(format!("{}/works", endpoints.openalex))
            .query(&[
                ("search", title),
                ("per_page", "1"),
                ("select", "open_access,locations,best_oa_location"),
            ])
            .send()
            .await
            .ok()?
    };

    if !resp.status().is_success() {
        return None;
    }
    let data: serde_json::Value = resp.json().await.ok()?;

    let work = if let Some(results) = data.get("results").and_then(|v| v.as_array()) {
        results.first()?
    } else {
        &data
    };

    // Try best_oa_location.pdf_url → open_access.oa_url → locations[].pdf_url
    if let Some(url) = work
        .pointer("/best_oa_location/pdf_url")
        .and_then(|v| v.as_str())
    {
        return Some(ResolvedPdf {
            url: url.into(),
            source: "openalex".into(),
            downloadable: is_downloadable(url),
        });
    }
    if let Some(url) = work.pointer("/open_access/oa_url").and_then(|v| v.as_str())
        && url.ends_with(".pdf")
    {
        return Some(ResolvedPdf {
            url: url.into(),
            source: "openalex".into(),
            downloadable: is_downloadable(url),
        });
    }
    for loc in work
        .get("locations")
        .and_then(|v| v.as_array())
        .unwrap_or(&vec![])
    {
        if let Some(url) = loc.get("pdf_url").and_then(|v| v.as_str()) {
            return Some(ResolvedPdf {
                url: url.into(),
                source: "openalex".into(),
                downloadable: is_downloadable(url),
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// CORE
// ---------------------------------------------------------------------------

async fn try_core(
    client: &reqwest::Client,
    doi: Option<&str>,
    title: Option<&str>,
    endpoints: &Endpoints,
) -> Option<ResolvedPdf> {
    let query = if let Some(doi) = doi {
        format!(r#"doi:"{doi}""#)
    } else {
        let title = title?;
        format!(r#"title:"{title}""#)
    };

    let resp = client
        .get(format!("{}/search/works", endpoints.core))
        .query(&[("q", &query), ("limit", &"1".to_string())])
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let data: serde_json::Value = resp.json().await.ok()?;
    let work = data.get("results")?.as_array()?.first()?;

    if let Some(url) = work.get("downloadUrl").and_then(|v| v.as_str()) {
        return Some(ResolvedPdf {
            url: url.into(),
            source: "core".into(),
            downloadable: is_downloadable(url),
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Google Scholar
// ---------------------------------------------------------------------------

async fn try_google_scholar(
    client: &reqwest::Client,
    title: Option<&str>,
    endpoints: &Endpoints,
) -> Option<ResolvedPdf> {
    let title = title?;
    let resp = client
        .get(format!("{}/scholar", endpoints.google_scholar))
        .query(&[("q", &format!("\"{title}\"")), ("num", &"5".to_string())])
        .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept", "text/html")
        .send().await.ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let html = resp.text().await.ok()?;

    let academic_hosts = [
        ".edu",
        ".ac.uk",
        "research.google",
        "hal.science",
        "eprint.iacr.org",
    ];

    // Prefer academic hosts
    for cap in PDF_HREF_RE.captures_iter(&html) {
        let url = &cap[1];
        if academic_hosts.iter().any(|h| url.contains(h)) {
            return Some(ResolvedPdf {
                url: url.into(),
                source: "google_scholar".into(),
                downloadable: true,
            });
        }
    }
    // Fallback: any downloadable PDF
    for cap in PDF_HREF_RE.captures_iter(&html) {
        let url = &cap[1];
        if is_downloadable(url) {
            return Some(ResolvedPdf {
                url: url.into(),
                source: "google_scholar".into(),
                downloadable: true,
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Unpaywall
// ---------------------------------------------------------------------------

async fn try_unpaywall(
    client: &reqwest::Client,
    doi: Option<&str>,
    email: &str,
    endpoints: &Endpoints,
) -> Option<ResolvedPdf> {
    let doi = doi?;
    let resp = client
        .get(format!("{}/{doi}", endpoints.unpaywall))
        .query(&[("email", email)])
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let data: serde_json::Value = resp.json().await.ok()?;

    if let Some(url) = data
        .pointer("/best_oa_location/url_for_pdf")
        .and_then(|v| v.as_str())
    {
        return Some(ResolvedPdf {
            url: url.into(),
            source: "unpaywall".into(),
            downloadable: is_downloadable(url),
        });
    }
    for loc in data
        .get("oa_locations")
        .and_then(|v| v.as_array())
        .unwrap_or(&vec![])
    {
        if let Some(url) = loc.get("url_for_pdf").and_then(|v| v.as_str()) {
            return Some(ResolvedPdf {
                url: url.into(),
                source: "unpaywall".into(),
                downloadable: is_downloadable(url),
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Crossref
// ---------------------------------------------------------------------------

async fn try_crossref(
    client: &reqwest::Client,
    doi: Option<&str>,
    email: &str,
    user_agent: &str,
    endpoints: &Endpoints,
) -> Option<ResolvedPdf> {
    let doi = doi?;
    let resp = client
        .get(format!("{}/works/{doi}", endpoints.crossref))
        .header("User-Agent", format!("{user_agent} (mailto:{email})"))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let data: serde_json::Value = resp.json().await.ok()?;
    let msg = data.get("message")?;

    // Check resource.primary.URL
    if let Some(url) = msg
        .pointer("/resource/primary/URL")
        .and_then(|v| v.as_str())
        && url.to_lowercase().ends_with(".pdf")
    {
        return Some(ResolvedPdf {
            url: url.into(),
            source: "crossref".into(),
            downloadable: is_downloadable(url),
        });
    }
    // Check link[] array
    for link in msg
        .get("link")
        .and_then(|v| v.as_array())
        .unwrap_or(&vec![])
    {
        let ct = link
            .get("content-type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if ct.contains("pdf")
            && let Some(url) = link.get("URL").and_then(|v| v.as_str())
        {
            return Some(ResolvedPdf {
                url: url.into(),
                source: "crossref".into(),
                downloadable: is_downloadable(url),
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Zenodo
// ---------------------------------------------------------------------------

async fn try_zenodo(
    client: &reqwest::Client,
    title: Option<&str>,
    endpoints: &Endpoints,
) -> Option<ResolvedPdf> {
    let title = title?;
    let resp = client
        .get(format!("{}/records", endpoints.zenodo))
        .query(&[("q", title), ("size", "3"), ("type", "publication")])
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let data: serde_json::Value = resp.json().await.ok()?;

    for hit in data
        .pointer("/hits/hits")
        .and_then(|v| v.as_array())
        .unwrap_or(&vec![])
    {
        for file in hit
            .get("files")
            .and_then(|v| v.as_array())
            .unwrap_or(&vec![])
        {
            if file
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase()
                .ends_with(".pdf")
                && let Some(url) = file.pointer("/links/self").and_then(|v| v.as_str())
            {
                return Some(ResolvedPdf {
                    url: url.into(),
                    source: "zenodo".into(),
                    downloadable: true,
                });
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// SSRN
// ---------------------------------------------------------------------------

async fn try_ssrn(
    client: &reqwest::Client,
    title: Option<&str>,
    endpoints: &Endpoints,
) -> Option<ResolvedPdf> {
    let title = title?;
    let resp = client
        .get(format!("{}/sol3/results.cfm", endpoints.ssrn))
        .query(&[("txtKey_Words", title), ("npage", "1")])
        .header("User-Agent", "Mozilla/5.0")
        .header("Accept", "text/html")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let html = resp.text().await.ok()?;

    if let Some(cap) = SSRN_RE.captures(&html) {
        return Some(ResolvedPdf {
            url: cap[1].to_string(),
            source: "ssrn".into(),
            downloadable: false,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Semantic Scholar
// ---------------------------------------------------------------------------

async fn try_semantic_scholar(
    client: &reqwest::Client,
    doi: Option<&str>,
    title: Option<&str>,
    endpoints: &Endpoints,
) -> Option<ResolvedPdf> {
    let resp = if let Some(doi) = doi {
        client
            .get(format!("{}/paper/DOI:{doi}", endpoints.semantic_scholar))
            .query(&[("fields", "openAccessPdf")])
            .send()
            .await
            .ok()?
    } else {
        let title = title?;
        client
            .get(format!("{}/paper/search", endpoints.semantic_scholar))
            .query(&[
                ("query", title),
                ("limit", "1"),
                ("fields", "openAccessPdf"),
            ])
            .send()
            .await
            .ok()?
    };

    if !resp.status().is_success() {
        return None;
    }
    let data: serde_json::Value = resp.json().await.ok()?;

    let work = if let Some(items) = data.get("data").and_then(|v| v.as_array()) {
        items.first()?
    } else {
        &data
    };

    let oa = work.get("openAccessPdf")?;
    if let Some(url) = oa.get("url").and_then(|v| v.as_str()) {
        return Some(ResolvedPdf {
            url: url.into(),
            source: "semantic_scholar".into(),
            downloadable: is_downloadable(url),
        });
    }
    // Disclaimer fallback
    if let Some(disclaimer) = oa.get("disclaimer").and_then(|v| v.as_str()) {
        for m in URL_RE.find_iter(disclaimer) {
            let url = m.as_str();
            if url.contains("arxiv.org/abs/") {
                let pdf_url = url.replace("/abs/", "/pdf/");
                return Some(ResolvedPdf {
                    url: format!("{pdf_url}.pdf"),
                    source: "semantic_scholar".into(),
                    downloadable: true,
                });
            }
            if !url.contains("arxiv.org") || url.contains("/pdf/") {
                return Some(ResolvedPdf {
                    url: url.into(),
                    source: "semantic_scholar".into(),
                    downloadable: is_downloadable(url),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doi_to_arxiv_id_valid() {
        assert_eq!(
            doi_to_arxiv_id("10.48550/arXiv.2105.15183"),
            Some("2105.15183".into())
        );
    }

    #[test]
    fn doi_to_arxiv_id_invalid() {
        assert_eq!(doi_to_arxiv_id("10.1234/other"), None);
    }

    #[test]
    fn url_to_arxiv_id_abs() {
        assert_eq!(
            url_to_arxiv_id("https://arxiv.org/abs/2105.15183"),
            Some("2105.15183".into())
        );
    }

    #[test]
    fn url_to_arxiv_id_pdf_versioned() {
        assert_eq!(
            url_to_arxiv_id("https://arxiv.org/pdf/2105.15183v2"),
            Some("2105.15183v2".into())
        );
    }

    #[test]
    fn url_to_arxiv_id_non_arxiv() {
        assert_eq!(url_to_arxiv_id("https://example.com/paper"), None);
    }

    #[test]
    fn resolve_arxiv_doi_instant() {
        let result = resolve_pdf(Some("10.48550/arXiv.2105.15183"), None, None);
        let r = result.unwrap();
        assert_eq!(r.source, "arxiv");
        assert_eq!(r.url, "https://arxiv.org/pdf/2105.15183.pdf");
        assert!(r.downloadable);
    }

    #[test]
    fn resolve_arxiv_url_instant() {
        let result = resolve_pdf(None, Some("https://arxiv.org/abs/2301.01234"), None);
        let r = result.unwrap();
        assert_eq!(r.source, "arxiv");
        assert!(r.url.contains("2301.01234"));
    }

    #[test]
    fn is_downloadable_blocked() {
        assert!(!is_downloadable("https://ieeexplore.ieee.org/doc/123.pdf"));
        assert!(!is_downloadable(
            "https://www.sciencedirect.com/article.pdf"
        ));
    }

    #[test]
    fn is_downloadable_ok() {
        assert!(is_downloadable("https://arxiv.org/pdf/2105.15183.pdf"));
        assert!(is_downloadable("https://example.edu/paper.pdf"));
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn default_config_has_all_sources_enabled() {
        let config = ResolverConfig::default();
        assert_eq!(config.sources.len(), 9);
        for source in &config.sources {
            assert!(source.enabled, "Source {} should be enabled", source.name);
        }
    }

    #[test]
    fn is_enabled_true_for_enabled_source() {
        let config = ResolverConfig::default();
        assert!(config.is_enabled("arxiv"));
        assert!(config.is_enabled("openalex"));
        assert!(config.is_enabled("semantic_scholar"));
    }

    #[test]
    fn is_enabled_false_for_disabled_source() {
        let mut config = ResolverConfig::default();
        config.sources[1].enabled = false; // disable openalex
        assert!(!config.is_enabled("openalex"));
        assert!(config.is_enabled("arxiv")); // others still enabled
    }

    #[test]
    fn is_enabled_false_for_unknown_source() {
        let config = ResolverConfig::default();
        assert!(!config.is_enabled("nonexistent"));
    }

    #[test]
    fn priority_reflects_position() {
        let config = ResolverConfig::default();
        assert_eq!(config.priority("arxiv"), 1);
        assert_eq!(config.priority("openalex"), 2);
        assert_eq!(config.priority("semantic_scholar"), 9);
    }

    #[test]
    fn priority_returns_99_for_unknown() {
        let config = ResolverConfig::default();
        assert_eq!(config.priority("nonexistent"), 99);
    }

    #[test]
    fn custom_source_order_changes_priority() {
        let config = ResolverConfig {
            sources: vec![
                SourceEntry {
                    name: "unpaywall".into(),
                    enabled: true,
                },
                SourceEntry {
                    name: "arxiv".into(),
                    enabled: true,
                },
            ],
            ..Default::default()
        };
        assert_eq!(config.priority("unpaywall"), 1);
        assert_eq!(config.priority("arxiv"), 2);
    }

    #[test]
    fn resolve_with_arxiv_disabled_skips_arxiv() {
        let mut config = ResolverConfig::default();
        // Disable arxiv
        config.sources[0].enabled = false;
        // This DOI would normally resolve instantly via arxiv
        let result =
            resolve_pdf_with_config(Some("10.48550/arXiv.2105.15183"), None, None, &config);
        // With arxiv disabled and no network, should return None
        // (or a result from another source if network available)
        match result {
            None => {} // Expected without network
            Some(r) => assert_ne!(r.source, "arxiv", "Should not use disabled arxiv"),
        }
    }

    #[test]
    fn resolve_with_config_uses_arxiv_when_enabled() {
        let config = ResolverConfig::default();
        let result =
            resolve_pdf_with_config(Some("10.48550/arXiv.2105.15183"), None, None, &config);
        let r = result.unwrap();
        assert_eq!(r.source, "arxiv");
        assert!(r.downloadable);
    }
}

#[cfg(test)]
mod mock_tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a config that enables only the given source, pointing all endpoints
    /// at the mock server.
    fn single_source_config(source_name: &str, base_uri: &str) -> ResolverConfig {
        let endpoints = Endpoints {
            openalex: base_uri.into(),
            core: base_uri.into(),
            google_scholar: base_uri.into(),
            unpaywall: base_uri.into(),
            crossref: base_uri.into(),
            zenodo: base_uri.into(),
            ssrn: base_uri.into(),
            semantic_scholar: base_uri.into(),
        };
        ResolverConfig {
            sources: vec![SourceEntry::new(source_name, true)],
            endpoints,
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // OpenAlex
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn openalex_doi_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/doi:.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "best_oa_location": {
                    "pdf_url": "https://example.edu/paper.pdf"
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("openalex", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "openalex");
        assert_eq!(r.url, "https://example.edu/paper.pdf");
        assert!(r.downloadable);
    }

    #[tokio::test]
    async fn openalex_title_search_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{
                    "best_oa_location": {
                        "pdf_url": "https://example.edu/search-result.pdf"
                    }
                }]
            })))
            .mount(&server)
            .await;

        let config = single_source_config("openalex", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "openalex");
        assert_eq!(r.url, "https://example.edu/search-result.pdf");
    }

    #[tokio::test]
    async fn openalex_oa_url_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/doi:.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "open_access": {
                    "oa_url": "https://example.edu/open.pdf"
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("openalex", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.url, "https://example.edu/open.pdf");
    }

    #[tokio::test]
    async fn openalex_locations_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/doi:.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "locations": [
                    { "pdf_url": "https://example.edu/loc.pdf" }
                ]
            })))
            .mount(&server)
            .await;

        let config = single_source_config("openalex", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.url, "https://example.edu/loc.pdf");
    }

    #[tokio::test]
    async fn openalex_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/doi:.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let config = single_source_config("openalex", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn openalex_blocked_domain_not_downloadable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/doi:.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "best_oa_location": {
                    "pdf_url": "https://www.sciencedirect.com/paper.pdf"
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("openalex", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert!(!r.downloadable);
    }

    // -----------------------------------------------------------------------
    // CORE
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn core_doi_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/search/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{
                    "downloadUrl": "https://core.ac.uk/download/pdf/123.pdf"
                }]
            })))
            .mount(&server)
            .await;

        let config = single_source_config("core", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "core");
        assert_eq!(r.url, "https://core.ac.uk/download/pdf/123.pdf");
        assert!(r.downloadable);
    }

    #[tokio::test]
    async fn core_title_search_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/search/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{
                    "downloadUrl": "https://core.ac.uk/download/pdf/456.pdf"
                }]
            })))
            .mount(&server)
            .await;

        let config = single_source_config("core", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "core");
    }

    #[tokio::test]
    async fn core_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/search/works"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let config = single_source_config("core", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn core_empty_results_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/search/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": []
            })))
            .mount(&server)
            .await;

        let config = single_source_config("core", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Google Scholar
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn google_scholar_happy_path_academic_host() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/scholar"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<html><body>
                <a href="https://cs.stanford.edu/paper.pdf">[PDF]</a>
                </body></html>"#,
            ))
            .mount(&server)
            .await;

        let config = single_source_config("google_scholar", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "google_scholar");
        assert_eq!(r.url, "https://cs.stanford.edu/paper.pdf");
        assert!(r.downloadable);
    }

    #[tokio::test]
    async fn google_scholar_fallback_non_academic_pdf() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/scholar"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<html><body>
                <a href="https://example.com/paper.pdf">PDF</a>
                </body></html>"#,
            ))
            .mount(&server)
            .await;

        let config = single_source_config("google_scholar", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "google_scholar");
        assert_eq!(r.url, "https://example.com/paper.pdf");
        assert!(r.downloadable);
    }

    #[tokio::test]
    async fn google_scholar_blocked_pdf_skipped() {
        let server = MockServer::start().await;
        // Only blocked-domain PDFs — no downloadable ones
        Mock::given(method("GET"))
            .and(path_regex(r"/scholar"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<html><body>
                <a href="https://www.sciencedirect.com/paper.pdf">PDF</a>
                </body></html>"#,
            ))
            .mount(&server)
            .await;

        let config = single_source_config("google_scholar", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        // The google_scholar handler skips blocked domains internally
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn google_scholar_no_title_returns_none() {
        let config = single_source_config("google_scholar", "http://unused");
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn google_scholar_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/scholar"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let config = single_source_config("google_scholar", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Unpaywall
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn unpaywall_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/10\.1234/test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "best_oa_location": {
                    "url_for_pdf": "https://europepmc.org/paper.pdf"
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("unpaywall", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "unpaywall");
        assert_eq!(r.url, "https://europepmc.org/paper.pdf");
        assert!(r.downloadable);
    }

    #[tokio::test]
    async fn unpaywall_oa_locations_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/10\.1234/test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "oa_locations": [
                    { "url_for_pdf": "https://repo.edu/fallback.pdf" }
                ]
            })))
            .mount(&server)
            .await;

        let config = single_source_config("unpaywall", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.url, "https://repo.edu/fallback.pdf");
    }

    #[tokio::test]
    async fn unpaywall_no_doi_returns_none() {
        let config = single_source_config("unpaywall", "http://unused");
        let result = resolve_pdf_async(None, None, Some("title"), &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn unpaywall_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/10\.1234/test"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let config = single_source_config("unpaywall", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn unpaywall_blocked_domain_not_downloadable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/10\.1234/test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "best_oa_location": {
                    "url_for_pdf": "https://link.springer.com/paper.pdf"
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("unpaywall", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert!(!r.downloadable);
    }

    // -----------------------------------------------------------------------
    // Crossref
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn crossref_primary_url_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/10\.1234/test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "resource": {
                        "primary": {
                            "URL": "https://publisher.org/article.pdf"
                        }
                    }
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("crossref", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "crossref");
        assert_eq!(r.url, "https://publisher.org/article.pdf");
        assert!(r.downloadable);
    }

    #[tokio::test]
    async fn crossref_link_array_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/10\.1234/test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "link": [
                        {
                            "URL": "https://publisher.org/full.pdf",
                            "content-type": "application/pdf"
                        }
                    ]
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("crossref", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "crossref");
        assert_eq!(r.url, "https://publisher.org/full.pdf");
    }

    #[tokio::test]
    async fn crossref_no_doi_returns_none() {
        let config = single_source_config("crossref", "http://unused");
        let result = resolve_pdf_async(None, None, Some("title"), &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn crossref_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/10\.1234/test"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let config = single_source_config("crossref", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Zenodo
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn zenodo_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/records"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hits": {
                    "hits": [{
                        "files": [{
                            "key": "paper.pdf",
                            "links": {
                                "self": "https://zenodo.org/records/123/files/paper.pdf"
                            }
                        }]
                    }]
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("zenodo", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "zenodo");
        assert_eq!(r.url, "https://zenodo.org/records/123/files/paper.pdf");
        assert!(r.downloadable);
    }

    #[tokio::test]
    async fn zenodo_no_pdf_files_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/records"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hits": {
                    "hits": [{
                        "files": [{
                            "key": "data.csv",
                            "links": {
                                "self": "https://zenodo.org/records/123/files/data.csv"
                            }
                        }]
                    }]
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("zenodo", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn zenodo_no_title_returns_none() {
        let config = single_source_config("zenodo", "http://unused");
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn zenodo_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/records"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let config = single_source_config("zenodo", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // SSRN
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn ssrn_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/sol3/results\.cfm"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<html><body>
                <a href="https://papers.ssrn.com/sol3/papers.cfm?abstract_id=1234567">Paper</a>
                </body></html>"#,
            ))
            .mount(&server)
            .await;

        let config = single_source_config("ssrn", &server.uri());
        let result = resolve_pdf_async(None, None, Some("volatility modeling"), &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "ssrn");
        assert_eq!(
            r.url,
            "https://papers.ssrn.com/sol3/papers.cfm?abstract_id=1234567"
        );
        // SSRN never serves direct PDFs
        assert!(!r.downloadable);
    }

    #[tokio::test]
    async fn ssrn_no_match_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/sol3/results\.cfm"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"<html><body>No results found.</body></html>"#),
            )
            .mount(&server)
            .await;

        let config = single_source_config("ssrn", &server.uri());
        let result = resolve_pdf_async(None, None, Some("nonexistent paper"), &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn ssrn_no_title_returns_none() {
        let config = single_source_config("ssrn", "http://unused");
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Semantic Scholar
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn semantic_scholar_doi_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/paper/DOI:.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "openAccessPdf": {
                    "url": "https://example.edu/s2paper.pdf"
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("semantic_scholar", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "semantic_scholar");
        assert_eq!(r.url, "https://example.edu/s2paper.pdf");
        assert!(r.downloadable);
    }

    #[tokio::test]
    async fn semantic_scholar_title_search_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/paper/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "openAccessPdf": {
                        "url": "https://example.edu/s2search.pdf"
                    }
                }]
            })))
            .mount(&server)
            .await;

        let config = single_source_config("semantic_scholar", &server.uri());
        let result = resolve_pdf_async(None, None, Some("mutation testing"), &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "semantic_scholar");
        assert_eq!(r.url, "https://example.edu/s2search.pdf");
    }

    #[tokio::test]
    async fn semantic_scholar_disclaimer_arxiv_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/paper/DOI:.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "openAccessPdf": {
                    "disclaimer": "See https://arxiv.org/abs/2105.15183 for the open access version."
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("semantic_scholar", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "semantic_scholar");
        assert_eq!(r.url, "https://arxiv.org/pdf/2105.15183.pdf");
        assert!(r.downloadable);
    }

    #[tokio::test]
    async fn semantic_scholar_disclaimer_non_arxiv_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/paper/DOI:.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "openAccessPdf": {
                    "disclaimer": "Available at https://example.edu/paper.pdf for download."
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("semantic_scholar", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert_eq!(r.source, "semantic_scholar");
        assert!(r.url.contains("example.edu"));
    }

    #[tokio::test]
    async fn semantic_scholar_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/paper/DOI:.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let config = single_source_config("semantic_scholar", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn semantic_scholar_no_oa_pdf_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/paper/DOI:.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "title": "Some paper"
            })))
            .mount(&server)
            .await;

        let config = single_source_config("semantic_scholar", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn semantic_scholar_blocked_domain_not_downloadable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/paper/DOI:.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "openAccessPdf": {
                    "url": "https://ieeexplore.ieee.org/paper.pdf"
                }
            })))
            .mount(&server)
            .await;

        let config = single_source_config("semantic_scholar", &server.uri());
        let result = resolve_pdf_async(Some("10.1234/test"), None, None, &config).await;
        let r = result.unwrap();
        assert!(!r.downloadable);
    }
}
