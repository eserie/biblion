//! Native bibliography formatting — APA and IEEE styles.
//!
//! # Why native?
//!
//! The full CSL (Citation Style Language) spec supports 9000+ styles, but
//! we only use 2-3 regularly. Implementing those natively eliminates the
//! last BBT dependency for common usage. BBT remains as fallback for
//! exotic styles.
//!
//! # Reference implementation
//!
//! These formatters were built from the official CSL style definitions:
//! - APA 7th edition: <https://www.zotero.org/styles/apa>
//! - IEEE: <https://www.zotero.org/styles/ieee>
//!
//! The Python MCP server delegated this to BBT's CSL engine via JSON-RPC
//! (`item.bibliography` method at `bbt_client.py:99`). That engine uses
//! `citeproc-js` internally. Our native implementation handles the common
//! cases; edge cases (non-Latin scripts, legal citations, etc.) fall back
//! to BBT.
//!
//! # Verification
//!
//! To verify output matches BBT's CSL engine, compare:
//! ```bash
//! # Native (Rust):
//! echo '...tools/call zotero_get_bibliography...' | biblion-rs
//!
//! # BBT reference (Python, requires Zotero running):
//! curl -X POST http://localhost:23119/better-bibtex/json-rpc \
//!   -d '{"jsonrpc":"2.0","id":1,"method":"item.bibliography",
//!        "params":[["citekey"],{"id":"http://www.zotero.org/styles/apa"}]}'
//! ```

use super::format::extract_year;
use crate::db::zotero::{Creator, ZoteroItem};
use std::collections::HashMap;

/// Supported native styles. Anything else falls back to BBT.
pub fn is_native_style(style: &str) -> bool {
    let s = style.to_lowercase();
    s.contains("apa") || s.contains("ieee")
}

/// Format a bibliography entry in the requested style.
pub fn format_bibliography(
    item: &ZoteroItem,
    metadata: &HashMap<String, String>,
    style: &str,
) -> String {
    let s = style.to_lowercase();
    if s.contains("ieee") {
        format_ieee(item, metadata)
    } else {
        // Default to APA
        format_apa(item, metadata)
    }
}

/// Format multiple items as a numbered bibliography.
pub fn format_bibliography_list(
    items: &[(ZoteroItem, HashMap<String, String>)],
    style: &str,
) -> String {
    items
        .iter()
        .enumerate()
        .map(|(i, (item, metadata))| {
            let entry = format_bibliography(item, metadata, style);
            if style.to_lowercase().contains("ieee") {
                format!("[{}] {}", i + 1, entry)
            } else {
                entry
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ---------------------------------------------------------------------------
// APA 7th edition
// ---------------------------------------------------------------------------
//
// Reference: https://apastyle.apa.org/style-grammar-guidelines/references
//
// Journal article pattern:
//   Author, A. A., Author, B. B., & Author, C. C. (Year). Title of article.
//   *Title of Periodical*, *Volume*(Issue), Pages. https://doi.org/xxxxx
//
// Book pattern:
//   Author, A. A. (Year). *Title of work*. Publisher.
//
// Conference paper pattern:
//   Author, A. A. (Year). Title of paper. In *Proceedings of Conference*
//   (pp. Pages). Publisher. https://doi.org/xxxxx

fn format_apa(item: &ZoteroItem, metadata: &HashMap<String, String>) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Authors: "Last, F. I., Last2, F. I., & Last3, F. I."
    let authors = format_apa_authors(&item.creators);
    if !authors.is_empty() {
        parts.push(authors);
    }

    // Year: "(2024)."
    if let Some(year) = item.date.as_ref().and_then(|d| extract_year(d)) {
        parts.push(format!("({year})."));
    }

    // Title (italicized for books, plain for articles)
    match item.item_type.as_str() {
        "book" => parts.push(format!("*{}*.", item.title)),
        _ => parts.push(format!("{}.", item.title)),
    }

    // Journal/conference name (italicized)
    if let Some(journal) = metadata.get("publicationTitle") {
        let volume = metadata.get("volume");
        let issue = metadata.get("issue");
        let pages = metadata.get("pages");

        let mut journal_part = format!("*{journal}*");
        if let Some(vol) = volume {
            journal_part.push_str(&format!(", *{vol}*"));
            if let Some(iss) = issue {
                journal_part.push_str(&format!("({iss})"));
            }
        }
        if let Some(pp) = pages {
            journal_part.push_str(&format!(", {pp}"));
        }
        journal_part.push('.');
        parts.push(journal_part);
    } else if let Some(conf) = metadata.get("conferenceName") {
        let mut conf_part = format!("In *{conf}*");
        if let Some(pp) = metadata.get("pages") {
            conf_part.push_str(&format!(" (pp. {pp})"));
        }
        conf_part.push('.');
        parts.push(conf_part);
    } else if let Some(publisher) = metadata.get("publisher") {
        parts.push(format!("{publisher}."));
    }

    // DOI
    if let Some(doi) = &item.doi {
        parts.push(format!("https://doi.org/{doi}"));
    }

    parts.join(" ")
}

/// Format authors in APA style: "Last, F. I., Last2, F. I., & Last3, F. I."
fn format_apa_authors(creators: &[Creator]) -> String {
    let authors: Vec<&Creator> = creators
        .iter()
        .filter(|c| c.creator_type == "author")
        .collect();

    if authors.is_empty() {
        return String::new();
    }

    let formatted: Vec<String> = authors
        .iter()
        .map(|c| {
            match &c.first_name {
                Some(first) if !first.is_empty() => {
                    // "DeMillo, R. A." — initials with periods
                    let initials: String = first
                        .split_whitespace()
                        .map(|w| format!("{}.", w.chars().next().unwrap_or(' ')))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("{}, {}", c.last_name, initials)
                }
                _ => c.last_name.clone(),
            }
        })
        .collect();

    match formatted.len() {
        1 => formatted[0].clone(),
        2 => format!("{} & {}", formatted[0], formatted[1]),
        _ => {
            // APA: "A, B, C, ... & Z" (up to 20 authors)
            let last = formatted.last().unwrap();
            let rest = &formatted[..formatted.len() - 1];
            format!("{}, & {}", rest.join(", "), last)
        }
    }
}

// ---------------------------------------------------------------------------
// IEEE
// ---------------------------------------------------------------------------
//
// Reference: https://ieeeauthorcenter.ieee.org/wp-content/uploads/IEEE-Reference-Guide.pdf
//
// Journal article pattern:
//   F. I. Author, F. I. Author, and F. I. Author, "Title of article,"
//   *Title of Journal*, vol. V, no. N, pp. P1–P2, Month Year.
//
// Conference paper pattern:
//   F. I. Author, "Title of paper," in *Proc. Conference*, Year, pp. P1–P2.

fn format_ieee(item: &ZoteroItem, metadata: &HashMap<String, String>) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Authors: "F. I. Last, F. I. Last, and F. I. Last"
    let authors = format_ieee_authors(&item.creators);
    if !authors.is_empty() {
        parts.push(format!("{authors},"));
    }

    // Title in quotes
    parts.push(format!("\"{}\"", item.title));

    // Journal/conference
    if let Some(journal) = metadata.get("publicationTitle") {
        let mut journal_part = format!("*{journal}*");
        if let Some(vol) = metadata.get("volume") {
            journal_part.push_str(&format!(", vol. {vol}"));
        }
        if let Some(iss) = metadata.get("issue") {
            journal_part.push_str(&format!(", no. {iss}"));
        }
        if let Some(pp) = metadata.get("pages") {
            journal_part.push_str(&format!(", pp. {pp}"));
        }
        if let Some(year) = item.date.as_ref().and_then(|d| extract_year(d)) {
            journal_part.push_str(&format!(", {year}"));
        }
        journal_part.push('.');
        parts.push(journal_part);
    } else if let Some(conf) = metadata.get("conferenceName") {
        let mut conf_part = format!("in *{conf}*");
        if let Some(year) = item.date.as_ref().and_then(|d| extract_year(d)) {
            conf_part.push_str(&format!(", {year}"));
        }
        if let Some(pp) = metadata.get("pages") {
            conf_part.push_str(&format!(", pp. {pp}"));
        }
        conf_part.push('.');
        parts.push(conf_part);
    } else if let Some(year) = item.date.as_ref().and_then(|d| extract_year(d)) {
        parts.push(format!("{year}."));
    }

    // DOI
    if let Some(doi) = &item.doi {
        parts.push(format!("doi: {doi}."));
    }

    parts.join(" ")
}

/// Format authors in IEEE style: "F. I. Last, F. I. Last, and F. I. Last"
fn format_ieee_authors(creators: &[Creator]) -> String {
    let authors: Vec<&Creator> = creators
        .iter()
        .filter(|c| c.creator_type == "author")
        .collect();

    if authors.is_empty() {
        return String::new();
    }

    let formatted: Vec<String> = authors
        .iter()
        .map(|c| match &c.first_name {
            Some(first) if !first.is_empty() => {
                let initials: String = first
                    .split_whitespace()
                    .map(|w| format!("{}.", w.chars().next().unwrap_or(' ')))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{} {}", initials, c.last_name)
            }
            _ => c.last_name.clone(),
        })
        .collect();

    match formatted.len() {
        1 => formatted[0].clone(),
        2 => format!("{} and {}", formatted[0], formatted[1]),
        _ => {
            let last = formatted.last().unwrap();
            let rest = &formatted[..formatted.len() - 1];
            format!("{}, and {}", rest.join(", "), last)
        }
    }
}

// extract_year is imported from format.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::zotero::Creator;

    fn demillo_item() -> ZoteroItem {
        ZoteroItem {
            item_id: 1,
            item_key: "ABC12345".into(),
            item_type: "journalArticle".into(),
            title: "Hints on Test Data Selection: Help for the Practicing Programmer".into(),
            date: Some("1978".into()),
            doi: Some("10.1109/C-M.1978.218136".into()),
            url: None,
            abstract_note: None,
            creators: vec![
                Creator {
                    creator_type: "author".into(),
                    first_name: Some("Richard A.".into()),
                    last_name: "DeMillo".into(),
                    order: 0,
                },
                Creator {
                    creator_type: "author".into(),
                    first_name: Some("Richard J.".into()),
                    last_name: "Lipton".into(),
                    order: 1,
                },
                Creator {
                    creator_type: "author".into(),
                    first_name: Some("Fred G.".into()),
                    last_name: "Sayward".into(),
                    order: 2,
                },
            ],
            tags: vec![],
            date_added: "2024-01-01".into(),
            date_modified: "2024-06-15".into(),
        }
    }

    fn demillo_metadata() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("publicationTitle".into(), "Computer".into());
        m.insert("volume".into(), "11".into());
        m.insert("issue".into(), "4".into());
        m.insert("pages".into(), "34-41".into());
        m
    }

    // --- APA ---

    #[test]
    fn apa_journal_article() {
        let bib = format_apa(&demillo_item(), &demillo_metadata());
        // APA: "DeMillo, R. A., Lipton, R. J., & Sayward, F. G. (1978). Hints on..."
        assert!(
            bib.contains("DeMillo, R. A., Lipton, R. J., & Sayward, F. G."),
            "Got: {bib}"
        );
        assert!(bib.contains("(1978)."));
        assert!(bib.contains("Hints on Test Data Selection"));
        assert!(bib.contains("*Computer*, *11*(4), 34-41."));
        assert!(bib.contains("https://doi.org/10.1109/C-M.1978.218136"));
    }

    #[test]
    fn apa_two_authors() {
        let mut item = demillo_item();
        item.creators = vec![
            Creator {
                creator_type: "author".into(),
                first_name: Some("Yue".into()),
                last_name: "Jia".into(),
                order: 0,
            },
            Creator {
                creator_type: "author".into(),
                first_name: Some("Mark".into()),
                last_name: "Harman".into(),
                order: 1,
            },
        ];
        let bib = format_apa(&item, &HashMap::new());
        assert!(bib.contains("Jia, Y. & Harman, M."), "Got: {bib}");
    }

    #[test]
    fn apa_single_author() {
        let mut item = demillo_item();
        item.creators = vec![Creator {
            creator_type: "author".into(),
            first_name: Some("Yue".into()),
            last_name: "Jia".into(),
            order: 0,
        }];
        let bib = format_apa(&item, &HashMap::new());
        assert!(bib.starts_with("Jia, Y."), "Got: {bib}");
    }

    #[test]
    fn apa_book() {
        let mut item = demillo_item();
        item.item_type = "book".into();
        item.title = "The Art of Software Testing".into();
        let mut meta = HashMap::new();
        meta.insert("publisher".into(), "Wiley".into());
        let bib = format_apa(&item, &meta);
        assert!(bib.contains("*The Art of Software Testing*."), "Got: {bib}");
        assert!(bib.contains("Wiley."));
    }

    // --- IEEE ---

    #[test]
    fn ieee_journal_article() {
        let bib = format_ieee(&demillo_item(), &demillo_metadata());
        // IEEE: "R. A. DeMillo, R. J. Lipton, and F. G. Sayward, "Hints on..."
        assert!(
            bib.contains("R. A. DeMillo, R. J. Lipton, and F. G. Sayward"),
            "Got: {bib}"
        );
        assert!(bib.contains("\"Hints on Test Data Selection"));
        assert!(bib.contains("*Computer*"));
        assert!(bib.contains("vol. 11"));
        assert!(bib.contains("no. 4"));
        assert!(bib.contains("pp. 34-41"));
        assert!(bib.contains("doi: 10.1109/C-M.1978.218136"));
    }

    #[test]
    fn ieee_two_authors() {
        let mut item = demillo_item();
        item.creators = vec![
            Creator {
                creator_type: "author".into(),
                first_name: Some("Goran".into()),
                last_name: "Petrovic".into(),
                order: 0,
            },
            Creator {
                creator_type: "author".into(),
                first_name: Some("Marko".into()),
                last_name: "Ivankovic".into(),
                order: 1,
            },
        ];
        let bib = format_ieee(&item, &HashMap::new());
        assert!(bib.contains("G. Petrovic and M. Ivankovic"), "Got: {bib}");
    }

    // --- Style detection ---

    #[test]
    fn native_style_detection() {
        assert!(is_native_style("http://www.zotero.org/styles/apa"));
        assert!(is_native_style("apa"));
        assert!(is_native_style("APA"));
        assert!(is_native_style("http://www.zotero.org/styles/ieee"));
        assert!(is_native_style("IEEE"));
        assert!(!is_native_style(
            "http://www.zotero.org/styles/chicago-author-date"
        ));
        assert!(!is_native_style("vancouver"));
    }

    // --- List formatting ---

    #[test]
    fn ieee_list_is_numbered() {
        let items = vec![(demillo_item(), demillo_metadata())];
        let list = format_bibliography_list(&items, "ieee");
        assert!(list.starts_with("[1]"), "Got: {list}");
    }

    #[test]
    fn apa_list_is_not_numbered() {
        let items = vec![(demillo_item(), demillo_metadata())];
        let list = format_bibliography_list(&items, "apa");
        assert!(!list.starts_with("[1]"));
        assert!(list.starts_with("DeMillo"), "Got: {list}");
    }
}
