//! Item formatting utilities.
//!
//! Converts internal Zotero types into human-readable text summaries
//! that Claude can parse and present to users. Mirrors the Python
//! `format_item_summary()` function.

use crate::db::zotero::{Creator, ZoteroItem};

/// Format a ZoteroItem as a concise text summary.
///
/// Output format matches the Python MCP server for consistency:
/// ```text
/// **citekey**
///   Title of the Paper
///   Author1, A.; Author2, B.
///   (2024)
///   [journalArticle]
///   DOI: 10.1234/example
/// ```
pub fn format_item_summary(item: &ZoteroItem, citekey: Option<&str>) -> String {
    let mut parts = Vec::new();

    // Header: citekey or item key
    let header = citekey.unwrap_or(&item.item_key);
    parts.push(format!("**{header}**"));

    // Title
    parts.push(format!("  {}", item.title));

    // Creators
    if !item.creators.is_empty() {
        let authors = format_creators(&item.creators);
        parts.push(format!("  {authors}"));
    }

    // Date
    if let Some(date) = &item.date {
        parts.push(format!("  ({date})"));
    }

    // Type
    parts.push(format!("  [{}]", item.item_type));

    // DOI
    if let Some(doi) = &item.doi {
        parts.push(format!("  DOI: {doi}"));
    }

    parts.join("\n")
}

/// Format creators as "LastName, F. I.; LastName2, F."
pub fn format_creators(creators: &[Creator]) -> String {
    creators
        .iter()
        .map(|c| {
            match &c.first_name {
                Some(first) if !first.is_empty() => {
                    // Abbreviate: "Richard A." → "R. A."
                    let initials: String = first
                        .split_whitespace()
                        .map(|w| format!("{}.", w.chars().next().unwrap_or(' ')))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("{}, {initials}", c.last_name)
                }
                _ => c.last_name.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Strip HTML tags from a string and convert block elements to newlines.
///
/// Handles common Zotero note patterns: `<p>`, `<br>`, `<div>`, `<b>`, `<i>`.
/// Block-level closing tags (`</p>`, `</div>`) insert newlines.
pub fn html_to_text(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut tag_buf = String::new();

    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag_buf.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let tag = tag_buf.to_lowercase();
                // Insert newline after block-level closing tags and <br>
                if (tag.starts_with("/p")
                    || tag.starts_with("/div")
                    || tag.starts_with("/h")
                    || tag.starts_with("br"))
                    && !result.ends_with('\n')
                    && !result.is_empty()
                {
                    result.push('\n');
                }
            }
            _ if in_tag => {
                tag_buf.push(ch);
            }
            _ => {
                result.push(ch);
            }
        }
    }
    result.trim().to_string()
}

/// Extract a 4-digit year from various date formats.
///
/// Handles: "2024", "2024-01-15", "2024-00-00 2024", "January 2024", etc.
/// Shared by bibtex.rs and bibliography.rs.
pub fn extract_year(date: &str) -> Option<String> {
    for word in date.split(|c: char| !c.is_ascii_digit()) {
        if word.len() == 4
            && let Ok(y) = word.parse::<u32>()
            && (1800..=2100).contains(&y)
        {
            return Some(word.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_item_with_citekey() {
        let item = ZoteroItem {
            item_id: 1,
            item_key: "ABC12345".into(),
            item_type: "journalArticle".into(),
            title: "Hints on Test Data Selection".into(),
            date: Some("1978".into()),
            doi: Some("10.1109/C-M.1978.218136".into()),
            url: None,
            abstract_note: None,
            creators: vec![
                Creator {
                    creator_type: "author".into(),
                    first_name: Some("Richard".into()),
                    last_name: "DeMillo".into(),
                    order: 0,
                },
                Creator {
                    creator_type: "author".into(),
                    first_name: Some("Richard".into()),
                    last_name: "Lipton".into(),
                    order: 1,
                },
            ],
            tags: vec!["mutation-testing".into()],
            date_added: "2024-01-01".into(),
            date_modified: "2024-06-15".into(),
        };
        let summary = format_item_summary(&item, Some("demilloHintsTestData1978"));
        assert!(summary.contains("**demilloHintsTestData1978**"));
        assert!(summary.contains("Hints on Test Data Selection"));
        assert!(summary.contains("DeMillo, R.; Lipton, R."));
        assert!(summary.contains("(1978)"));
        assert!(summary.contains("[journalArticle]"));
        assert!(summary.contains("DOI: 10.1109/C-M.1978.218136"));
    }

    #[test]
    fn format_item_without_citekey_uses_item_key() {
        let item = ZoteroItem {
            item_id: 1,
            item_key: "ABC12345".into(),
            item_type: "book".into(),
            title: "A Book".into(),
            date: None,
            doi: None,
            url: None,
            abstract_note: None,
            creators: vec![],
            tags: vec![],
            date_added: "2024-01-01".into(),
            date_modified: "2024-01-01".into(),
        };
        let summary = format_item_summary(&item, None);
        assert!(summary.contains("**ABC12345**"));
    }

    #[test]
    fn format_creators_with_initials() {
        let creators = vec![
            Creator {
                creator_type: "author".into(),
                first_name: Some("John".into()),
                last_name: "Doe".into(),
                order: 0,
            },
            Creator {
                creator_type: "author".into(),
                first_name: Some("Jane".into()),
                last_name: "Smith".into(),
                order: 1,
            },
        ];
        assert_eq!(format_creators(&creators), "Doe, J.; Smith, J.");
    }

    #[test]
    fn format_creators_multi_word_first_name() {
        let creators = vec![Creator {
            creator_type: "author".into(),
            first_name: Some("Richard A.".into()),
            last_name: "DeMillo".into(),
            order: 0,
        }];
        assert_eq!(format_creators(&creators), "DeMillo, R. A.");
    }

    #[test]
    fn format_creators_no_first_name() {
        let creators = vec![Creator {
            creator_type: "author".into(),
            first_name: None,
            last_name: "Organization".into(),
            order: 0,
        }];
        assert_eq!(format_creators(&creators), "Organization");
    }

    #[test]
    fn html_to_text_strips_tags() {
        assert_eq!(html_to_text("<p>Hello <b>world</b></p>"), "Hello world");
    }

    #[test]
    fn html_to_text_preserves_plain_text() {
        assert_eq!(html_to_text("No HTML here"), "No HTML here");
    }

    #[test]
    fn html_to_text_inserts_newlines_between_paragraphs() {
        let html = "<p>First paragraph</p><p>Second paragraph</p>";
        let text = html_to_text(html);
        assert!(
            text.contains("First paragraph\nSecond paragraph"),
            "Got: {text}"
        );
    }

    #[test]
    fn html_to_text_handles_br_tags() {
        assert_eq!(html_to_text("Line one<br>Line two"), "Line one\nLine two");
        assert_eq!(html_to_text("Line one<br/>Line two"), "Line one\nLine two");
    }

    #[test]
    fn html_to_text_handles_divs() {
        let html = "<div>Block one</div><div>Block two</div>";
        let text = html_to_text(html);
        assert!(text.contains("Block one\nBlock two"), "Got: {text}");
    }
}
