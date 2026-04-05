//! Paper search MCP tools — expose paper-resolver as standalone tools.
//!
//! These tools let Claude find open-access PDFs without needing a specific
//! Zotero item. Useful for: "find me the PDF for this DOI", "is this paper
//! available open-access?", "what sources are configured?"

use serde_json::{Value, json};

use crate::protocol::ToolCallResult;
use crate::server::ServerContext;

/// Find an open-access PDF URL for a paper.
///
/// Queries all enabled sources concurrently and returns the best result.
pub fn paper_resolve_pdf(args: &Value, ctx: &ServerContext) -> ToolCallResult {
    let doi = args.get("doi").and_then(|v| v.as_str());
    let title = args.get("title").and_then(|v| v.as_str());
    let url = args.get("url").and_then(|v| v.as_str());

    if doi.is_none() && title.is_none() && url.is_none() {
        return ToolCallResult::error("Provide at least one of: doi, title, url".into());
    }

    let result = paper_resolver::resolve_pdf_with_config(doi, url, title, &ctx.config.resolver);

    match result {
        Some(pdf) => ToolCallResult::text(
            serde_json::to_string_pretty(&json!({
                "url": pdf.url,
                "source": pdf.source,
                "downloadable": pdf.downloadable,
            }))
            .unwrap_or_default(),
        ),
        None => ToolCallResult::text("No PDF found from available sources.".into()),
    }
}

/// Show the current paper resolver configuration.
///
/// Lists enabled/disabled sources, their priority order, timeout, and email.
pub fn paper_source_status(ctx: &ServerContext) -> ToolCallResult {
    let config = &ctx.config.resolver;
    let mut output = String::from("Paper Resolver Configuration\n\n");

    output.push_str(&format!("Email: {}\n", config.email));
    output.push_str(&format!("User-Agent: {}\n", config.user_agent));
    output.push_str(&format!("Timeout: {}s\n", config.timeout_secs));
    output.push_str(&format!(
        "Extra blocked domains: {}\n\n",
        if config.extra_blocked_domains.is_empty() {
            "(none)".into()
        } else {
            config.extra_blocked_domains.join(", ")
        }
    ));

    output.push_str("Sources (priority order):\n");
    for (i, source) in config.sources.iter().enumerate() {
        let status = if source.enabled {
            "enabled"
        } else {
            "disabled"
        };
        output.push_str(&format!("  {}. {} [{}]\n", i + 1, source.name, status));
    }

    ToolCallResult::text(output)
}
