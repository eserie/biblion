//! PDF resolver — find open-access PDF URLs from 9 academic sources.
//!
//! # Architecture
//!
//! This is the one module that uses `tokio` for async HTTP. All 9 sources
//! are queried concurrently via `tokio::join!`, and the best result
//! (downloadable, highest priority) wins.
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
//! ```ignore
//! let result = resolve_pdf(Some("10.1109/TSE.2010.62"), None, Some("mutation testing"));
//! // result: Some(ResolvedPdf { url: "https://...", source: "openalex", downloadable: true })
//! ```

use regex::Regex;

/// Result of PDF URL resolution.
#[derive(Debug, Clone)]
pub struct ResolvedPdf {
    pub url: String,
    pub source: String,
    pub downloadable: bool,
}

/// Domains known to block programmatic downloads.
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

/// Resolve a PDF URL using all available sources.
///
/// This creates a temporary tokio runtime for the concurrent HTTP calls.
/// Called from the synchronous tool dispatch — the runtime is dropped after.
pub fn resolve_pdf(
    doi: Option<&str>,
    url: Option<&str>,
    title: Option<&str>,
) -> Option<ResolvedPdf> {
    // 1. arXiv — instant, no network
    if let Some(doi) = doi {
        if let Some(id) = doi_to_arxiv_id(doi) {
            return Some(ResolvedPdf {
                url: format!("https://arxiv.org/pdf/{id}.pdf"),
                source: "arxiv".into(),
                downloadable: true,
            });
        }
    }
    if let Some(url) = url {
        if let Some(id) = url_to_arxiv_id(url) {
            return Some(ResolvedPdf {
                url: format!("https://arxiv.org/pdf/{id}.pdf"),
                source: "arxiv".into(),
                downloadable: true,
            });
        }
    }

    // 2-9. Concurrent HTTP queries via tokio
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return None,
    };

    rt.block_on(resolve_pdf_async(doi, title))
}

async fn resolve_pdf_async(
    doi: Option<&str>,
    title: Option<&str>,
) -> Option<ResolvedPdf> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .ok()?;

    // Fire all sources concurrently
    let (openalex, core, scholar, unpaywall, crossref, zenodo, ssrn, semantic) = tokio::join!(
        try_openalex(&client, doi, title),
        try_core(&client, doi, title),
        try_google_scholar(&client, title),
        try_unpaywall(&client, doi),
        try_crossref(&client, doi),
        try_zenodo(&client, title),
        try_ssrn(&client, title),
        try_semantic_scholar(&client, doi, title),
    );

    // Collect results with priorities
    let mut candidates: Vec<(u8, ResolvedPdf)> = Vec::new();
    if let Some(r) = openalex { candidates.push((2, r)); }
    if let Some(r) = core { candidates.push((3, r)); }
    if let Some(r) = scholar { candidates.push((4, r)); }
    if let Some(r) = unpaywall { candidates.push((5, r)); }
    if let Some(r) = crossref { candidates.push((6, r)); }
    if let Some(r) = zenodo { candidates.push((7, r)); }
    if let Some(r) = ssrn { candidates.push((8, r)); }
    if let Some(r) = semantic { candidates.push((9, r)); }

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
    let re = Regex::new(r"arxiv\.org/(?:abs|pdf)/(\d{4}\.\d{4,5}(?:v\d+)?)").ok()?;
    re.captures(url).map(|c| c[1].to_string())
}

// ---------------------------------------------------------------------------
// OpenAlex
// ---------------------------------------------------------------------------

async fn try_openalex(
    client: &reqwest::Client,
    doi: Option<&str>,
    title: Option<&str>,
) -> Option<ResolvedPdf> {
    let resp = if let Some(doi) = doi {
        client
            .get(format!("https://api.openalex.org/works/doi:{doi}"))
            .query(&[("select", "open_access,locations,best_oa_location")])
            .send().await.ok()?
    } else {
        let title = title?;
        client
            .get("https://api.openalex.org/works")
            .query(&[("search", title), ("per_page", "1"), ("select", "open_access,locations,best_oa_location")])
            .send().await.ok()?
    };

    if !resp.status().is_success() { return None; }
    let data: serde_json::Value = resp.json().await.ok()?;

    let work = if let Some(results) = data.get("results").and_then(|v| v.as_array()) {
        results.first()?
    } else {
        &data
    };

    // Try best_oa_location.pdf_url → open_access.oa_url → locations[].pdf_url
    if let Some(url) = work.pointer("/best_oa_location/pdf_url").and_then(|v| v.as_str()) {
        return Some(ResolvedPdf { url: url.into(), source: "openalex".into(), downloadable: is_downloadable(url) });
    }
    if let Some(url) = work.pointer("/open_access/oa_url").and_then(|v| v.as_str()) {
        if url.ends_with(".pdf") {
            return Some(ResolvedPdf { url: url.into(), source: "openalex".into(), downloadable: is_downloadable(url) });
        }
    }
    for loc in work.get("locations").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
        if let Some(url) = loc.get("pdf_url").and_then(|v| v.as_str()) {
            return Some(ResolvedPdf { url: url.into(), source: "openalex".into(), downloadable: is_downloadable(url) });
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
) -> Option<ResolvedPdf> {
    let query = if let Some(doi) = doi {
        format!(r#"doi:"{doi}""#)
    } else {
        let title = title?;
        format!(r#"title:"{title}""#)
    };

    let resp = client
        .get("https://api.core.ac.uk/v3/search/works")
        .query(&[("q", &query), ("limit", &"1".to_string())])
        .send().await.ok()?;

    if !resp.status().is_success() { return None; }
    let data: serde_json::Value = resp.json().await.ok()?;
    let work = data.get("results")?.as_array()?.first()?;

    if let Some(url) = work.get("downloadUrl").and_then(|v| v.as_str()) {
        return Some(ResolvedPdf { url: url.into(), source: "core".into(), downloadable: is_downloadable(url) });
    }
    None
}

// ---------------------------------------------------------------------------
// Google Scholar
// ---------------------------------------------------------------------------

async fn try_google_scholar(
    client: &reqwest::Client,
    title: Option<&str>,
) -> Option<ResolvedPdf> {
    let title = title?;
    let resp = client
        .get("https://scholar.google.com/scholar")
        .query(&[("q", &format!("\"{title}\"")), ("num", &"5".to_string())])
        .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept", "text/html")
        .send().await.ok()?;

    if !resp.status().is_success() { return None; }
    let html = resp.text().await.ok()?;

    let re = Regex::new(r#"href="(https?://[^"]+\.pdf)""#).ok()?;
    let academic_hosts = [".edu", ".ac.uk", "research.google", "hal.science", "eprint.iacr.org"];

    // Prefer academic hosts
    for cap in re.captures_iter(&html) {
        let url = &cap[1];
        if academic_hosts.iter().any(|h| url.contains(h)) {
            return Some(ResolvedPdf { url: url.into(), source: "google_scholar".into(), downloadable: true });
        }
    }
    // Fallback: any downloadable PDF
    for cap in re.captures_iter(&html) {
        let url = &cap[1];
        if is_downloadable(url) {
            return Some(ResolvedPdf { url: url.into(), source: "google_scholar".into(), downloadable: true });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Unpaywall
// ---------------------------------------------------------------------------

async fn try_unpaywall(client: &reqwest::Client, doi: Option<&str>) -> Option<ResolvedPdf> {
    let doi = doi?;
    let resp = client
        .get(format!("https://api.unpaywall.org/v2/{doi}"))
        .query(&[("email", "zotero-mcp@example.com")])
        .send().await.ok()?;

    if !resp.status().is_success() { return None; }
    let data: serde_json::Value = resp.json().await.ok()?;

    if let Some(url) = data.pointer("/best_oa_location/url_for_pdf").and_then(|v| v.as_str()) {
        return Some(ResolvedPdf { url: url.into(), source: "unpaywall".into(), downloadable: is_downloadable(url) });
    }
    for loc in data.get("oa_locations").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
        if let Some(url) = loc.get("url_for_pdf").and_then(|v| v.as_str()) {
            return Some(ResolvedPdf { url: url.into(), source: "unpaywall".into(), downloadable: is_downloadable(url) });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Crossref
// ---------------------------------------------------------------------------

async fn try_crossref(client: &reqwest::Client, doi: Option<&str>) -> Option<ResolvedPdf> {
    let doi = doi?;
    let resp = client
        .get(format!("https://api.crossref.org/works/{doi}"))
        .header("User-Agent", "ZoteroMCP/0.1 (mailto:zotero-mcp@example.com)")
        .send().await.ok()?;

    if !resp.status().is_success() { return None; }
    let data: serde_json::Value = resp.json().await.ok()?;
    let msg = data.get("message")?;

    // Check resource.primary.URL
    if let Some(url) = msg.pointer("/resource/primary/URL").and_then(|v| v.as_str()) {
        if url.to_lowercase().ends_with(".pdf") {
            return Some(ResolvedPdf { url: url.into(), source: "crossref".into(), downloadable: is_downloadable(url) });
        }
    }
    // Check link[] array
    for link in msg.get("link").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
        let ct = link.get("content-type").and_then(|v| v.as_str()).unwrap_or("");
        if ct.contains("pdf") {
            if let Some(url) = link.get("URL").and_then(|v| v.as_str()) {
                return Some(ResolvedPdf { url: url.into(), source: "crossref".into(), downloadable: is_downloadable(url) });
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Zenodo
// ---------------------------------------------------------------------------

async fn try_zenodo(client: &reqwest::Client, title: Option<&str>) -> Option<ResolvedPdf> {
    let title = title?;
    let resp = client
        .get("https://zenodo.org/api/records")
        .query(&[("q", title), ("size", "3"), ("type", "publication")])
        .send().await.ok()?;

    if !resp.status().is_success() { return None; }
    let data: serde_json::Value = resp.json().await.ok()?;

    for hit in data.pointer("/hits/hits").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
        for file in hit.get("files").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
            if file.get("key").and_then(|v| v.as_str()).unwrap_or("").to_lowercase().ends_with(".pdf") {
                if let Some(url) = file.pointer("/links/self").and_then(|v| v.as_str()) {
                    return Some(ResolvedPdf { url: url.into(), source: "zenodo".into(), downloadable: true });
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// SSRN
// ---------------------------------------------------------------------------

async fn try_ssrn(client: &reqwest::Client, title: Option<&str>) -> Option<ResolvedPdf> {
    let title = title?;
    let resp = client
        .get("https://papers.ssrn.com/sol3/results.cfm")
        .query(&[("txtKey_Words", title), ("npage", "1")])
        .header("User-Agent", "Mozilla/5.0")
        .header("Accept", "text/html")
        .send().await.ok()?;

    if !resp.status().is_success() { return None; }
    let html = resp.text().await.ok()?;

    let re = Regex::new(r#"href="(https?://papers\.ssrn\.com/sol3/papers\.cfm\?abstract_id=\d+)""#).ok()?;
    if let Some(cap) = re.captures(&html) {
        return Some(ResolvedPdf { url: cap[1].to_string(), source: "ssrn".into(), downloadable: false });
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
) -> Option<ResolvedPdf> {
    let resp = if let Some(doi) = doi {
        client
            .get(format!("https://api.semanticscholar.org/graph/v1/paper/DOI:{doi}"))
            .query(&[("fields", "openAccessPdf")])
            .send().await.ok()?
    } else {
        let title = title?;
        client
            .get("https://api.semanticscholar.org/graph/v1/paper/search")
            .query(&[("query", title), ("limit", "1"), ("fields", "openAccessPdf")])
            .send().await.ok()?
    };

    if !resp.status().is_success() { return None; }
    let data: serde_json::Value = resp.json().await.ok()?;

    let work = if let Some(items) = data.get("data").and_then(|v| v.as_array()) {
        items.first()?
    } else {
        &data
    };

    let oa = work.get("openAccessPdf")?;
    if let Some(url) = oa.get("url").and_then(|v| v.as_str()) {
        return Some(ResolvedPdf { url: url.into(), source: "semantic_scholar".into(), downloadable: is_downloadable(url) });
    }
    // Disclaimer fallback
    if let Some(disclaimer) = oa.get("disclaimer").and_then(|v| v.as_str()) {
        let re = Regex::new(r"https?://[^\s,)]+").ok()?;
        for m in re.find_iter(disclaimer) {
            let url = m.as_str();
            if url.contains("arxiv.org/abs/") {
                let pdf_url = url.replace("/abs/", "/pdf/");
                return Some(ResolvedPdf { url: format!("{pdf_url}.pdf"), source: "semantic_scholar".into(), downloadable: true });
            }
            if !url.contains("arxiv.org") || url.contains("/pdf/") {
                return Some(ResolvedPdf { url: url.into(), source: "semantic_scholar".into(), downloadable: is_downloadable(url) });
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
        assert_eq!(doi_to_arxiv_id("10.48550/arXiv.2105.15183"), Some("2105.15183".into()));
    }

    #[test]
    fn doi_to_arxiv_id_invalid() {
        assert_eq!(doi_to_arxiv_id("10.1234/other"), None);
    }

    #[test]
    fn url_to_arxiv_id_abs() {
        assert_eq!(url_to_arxiv_id("https://arxiv.org/abs/2105.15183"), Some("2105.15183".into()));
    }

    #[test]
    fn url_to_arxiv_id_pdf_versioned() {
        assert_eq!(url_to_arxiv_id("https://arxiv.org/pdf/2105.15183v2"), Some("2105.15183v2".into()));
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
        assert!(!is_downloadable("https://www.sciencedirect.com/article.pdf"));
    }

    #[test]
    fn is_downloadable_ok() {
        assert!(is_downloadable("https://arxiv.org/pdf/2105.15183.pdf"));
        assert!(is_downloadable("https://example.edu/paper.pdf"));
    }
}
