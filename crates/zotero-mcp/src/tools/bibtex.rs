//! Native BibTeX/BibLaTeX export — no BBT dependency.
//!
//! # Why native?
//!
//! BBT's JSON-RPC export takes ~300ms per call (JavaScript inside Electron).
//! We can generate equivalent BibTeX in <0.1ms from SQLite data.
//!
//! # BibTeX format
//!
//! ```bibtex
//! @article{demilloHintsTestData1978,
//!   author    = {DeMillo, Richard A. and Lipton, Richard J. and Sayward, Fred G.},
//!   title     = {Hints on Test Data Selection: Help for the Practicing Programmer},
//!   journal   = {Computer},
//!   year      = {1978},
//!   volume    = {11},
//!   number    = {4},
//!   pages     = {34--41},
//!   doi       = {10.1109/C-M.1978.218136},
//! }
//! ```
//!
//! # Item type mapping
//!
//! | Zotero type | BibTeX type | BibLaTeX type |
//! |-------------|-------------|---------------|
//! | journalArticle | @article | @article |
//! | book | @book | @book |
//! | bookSection | @incollection | @incollection |
//! | conferencePaper | @inproceedings | @inproceedings |
//! | thesis | @phdthesis | @thesis |
//! | report | @techreport | @report |
//! | webpage | @misc | @online |
//! | * (other) | @misc | @misc |

use std::collections::HashMap;

use crate::db::zotero::{Creator, ZoteroItem};
use super::format::extract_year;

/// Map Zotero item types to BibTeX entry types.
fn bibtex_type(zotero_type: &str) -> &'static str {
    match zotero_type {
        "journalArticle" => "article",
        "book" => "book",
        "bookSection" => "incollection",
        "conferencePaper" => "inproceedings",
        "thesis" => "phdthesis",
        "report" => "techreport",
        "webpage" | "blogPost" | "forumPost" => "misc",
        "presentation" => "misc",
        "patent" => "patent",
        "letter" | "email" => "misc",
        _ => "misc",
    }
}

/// Map Zotero item types to BibLaTeX entry types (more granular).
fn biblatex_type(zotero_type: &str) -> &'static str {
    match zotero_type {
        "journalArticle" => "article",
        "book" => "book",
        "bookSection" => "incollection",
        "conferencePaper" => "inproceedings",
        "thesis" => "thesis",
        "report" => "report",
        "webpage" | "blogPost" => "online",
        "presentation" => "unpublished",
        _ => "misc",
    }
}

/// Format creators as BibTeX `author` field.
///
/// BibTeX author format: `Last1, First1 and Last2, First2`
fn format_authors(creators: &[Creator]) -> String {
    creators
        .iter()
        .filter(|c| c.creator_type == "author")
        .map(|c| match &c.first_name {
            Some(first) if !first.is_empty() => format!("{}, {}", c.last_name, first),
            _ => c.last_name.clone(),
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

/// Format creators as BibTeX `editor` field.
fn format_editors(creators: &[Creator]) -> String {
    creators
        .iter()
        .filter(|c| c.creator_type == "editor")
        .map(|c| match &c.first_name {
            Some(first) if !first.is_empty() => format!("{}, {}", c.last_name, first),
            _ => c.last_name.clone(),
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

/// Escape special BibTeX characters in a string value.
fn escape_bibtex(s: &str) -> String {
    s.replace('&', r"\&")
        .replace('%', r"\%")
        .replace('#', r"\#")
        .replace('_', r"\_")
        .replace('{', r"\{")
        .replace('}', r"\}")
        .replace('~', r"\textasciitilde{}")
        .replace('^', r"\^{}")
}

/// Generate a BibTeX entry for a single item.
///
/// `format`: "bibtex" or "biblatex"
pub fn item_to_bibtex(
    item: &ZoteroItem,
    citekey: &str,
    metadata: &HashMap<String, String>,
    format: &str,
) -> String {
    let entry_type = if format == "biblatex" {
        biblatex_type(&item.item_type)
    } else {
        bibtex_type(&item.item_type)
    };

    let mut fields: Vec<(String, String)> = Vec::new();

    // Authors
    let authors = format_authors(&item.creators);
    if !authors.is_empty() {
        fields.push(("author".into(), authors));
    }

    // Editors
    let editors = format_editors(&item.creators);
    if !editors.is_empty() {
        fields.push(("editor".into(), editors));
    }

    // Title (double braces to preserve capitalization, special chars escaped)
    fields.push(("title".into(), escape_bibtex(&item.title)));

    // Standard fields from metadata
    let field_map = [
        ("publicationTitle", "journal"),
        ("bookTitle", "booktitle"),
        ("volume", "volume"),
        ("issue", "number"),
        ("pages", "pages"),
        ("publisher", "publisher"),
        ("place", "address"),
        ("university", "school"),
        ("conferenceName", "booktitle"),
        ("series", "series"),
        ("seriesNumber", "number"),
        ("ISSN", "issn"),
        ("ISBN", "isbn"),
        ("language", "language"),
        ("abstractNote", "abstract"),
    ];

    for (zotero_field, bibtex_field) in &field_map {
        if let Some(value) = metadata.get(*zotero_field)
            && !value.is_empty() {
                // Don't duplicate booktitle from conferenceName if bookTitle exists
                if *bibtex_field == "booktitle"
                    && *zotero_field == "conferenceName"
                    && metadata.contains_key("bookTitle")
                {
                    continue;
                }
                fields.push((bibtex_field.to_string(), escape_bibtex(value)));
            }
    }

    // Year (extract from date field)
    if let Some(date) = &item.date {
        if let Some(year) = extract_year(date) {
            fields.push(("year".into(), year));
        }
        if format == "biblatex" {
            fields.push(("date".into(), date.clone()));
        }
    }

    // DOI
    if let Some(doi) = &item.doi {
        fields.push(("doi".into(), doi.clone()));
    }

    // URL
    if let Some(url) = &item.url {
        fields.push(("url".into(), url.clone()));
    }

    // Format the entry
    let mut output = format!("@{entry_type}{{{citekey},\n");
    for (key, value) in &fields {
        if key == "title" {
            // Double braces preserve capitalization: title = {{My Title}},
            output.push_str(&format!("  {key} = {{{{{value}}}}},\n"));
        } else {
            output.push_str(&format!("  {key} = {{{value}}},\n"));
        }
    }
    output.push('}');
    output
}

// extract_year is imported from format.rs

/// Generate BibTeX for multiple items.
pub fn items_to_bibtex(
    items: &[(ZoteroItem, String, HashMap<String, String>)], // (item, citekey, metadata)
    format: &str,
) -> String {
    items
        .iter()
        .map(|(item, citekey, metadata)| item_to_bibtex(item, citekey, metadata, format))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_item() -> ZoteroItem {
        ZoteroItem {
            item_id: 1,
            item_key: "ABC12345".into(),
            item_type: "journalArticle".into(),
            title: "Hints on Test Data Selection".into(),
            date: Some("1978".into()),
            doi: Some("10.1109/C-M.1978.218136".into()),
            url: None,
            abstract_note: None,
            creators: vec![
                Creator { creator_type: "author".into(), first_name: Some("Richard A.".into()), last_name: "DeMillo".into(), order: 0 },
                Creator { creator_type: "author".into(), first_name: Some("Richard J.".into()), last_name: "Lipton".into(), order: 1 },
            ],
            tags: vec![],
            date_added: "2024-01-01".into(),
            date_modified: "2024-06-15".into(),
        }
    }

    fn test_metadata() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("publicationTitle".into(), "Computer".into());
        m.insert("volume".into(), "11".into());
        m.insert("issue".into(), "4".into());
        m.insert("pages".into(), "34-41".into());
        m
    }

    #[test]
    fn bibtex_entry_format() {
        let item = test_item();
        let metadata = test_metadata();
        let bib = item_to_bibtex(&item, "demilloHintsTestData1978", &metadata, "bibtex");

        assert!(bib.starts_with("@article{demilloHintsTestData1978,"));
        assert!(bib.contains("author = {DeMillo, Richard A. and Lipton, Richard J.}"));
        assert!(bib.contains("title = {{Hints on Test Data Selection}},"), "Got: {bib}");
        assert!(bib.contains("journal = {Computer}"));
        assert!(bib.contains("year = {1978}"));
        assert!(bib.contains("volume = {11}"));
        assert!(bib.contains("doi = {10.1109/C-M.1978.218136}"));
        assert!(bib.ends_with('}'));
    }

    #[test]
    fn biblatex_uses_different_entry_types() {
        let mut item = test_item();
        item.item_type = "conferencePaper".into();
        let bib = item_to_bibtex(&item, "test2024", &HashMap::new(), "biblatex");
        assert!(bib.starts_with("@inproceedings{test2024,"));
    }

    #[test]
    fn biblatex_thesis_type() {
        let mut item = test_item();
        item.item_type = "thesis".into();
        let bibtex = item_to_bibtex(&item, "t", &HashMap::new(), "bibtex");
        let biblatex = item_to_bibtex(&item, "t", &HashMap::new(), "biblatex");
        assert!(bibtex.starts_with("@phdthesis{t,"));
        assert!(biblatex.starts_with("@thesis{t,"));
    }

    #[test]
    fn biblatex_includes_date_field() {
        let item = test_item();
        let bib = item_to_bibtex(&item, "test", &HashMap::new(), "biblatex");
        assert!(bib.contains("date = {1978}"));
    }

    #[test]
    fn special_characters_escaped() {
        let mut item = test_item();
        item.title = "Testing & Verification: A 100% Approach".into();
        let bib = item_to_bibtex(&item, "test", &HashMap::new(), "bibtex");
        assert!(bib.contains(r"Testing \& Verification: A 100\% Approach"));
    }

    #[test]
    fn extract_year_simple() {
        assert_eq!(extract_year("2024"), Some("2024".into()));
    }

    #[test]
    fn extract_year_full_date() {
        assert_eq!(extract_year("2024-01-15"), Some("2024".into()));
    }

    #[test]
    fn extract_year_zotero_format() {
        // Zotero sometimes stores "2011-00-00 2011"
        assert_eq!(extract_year("2011-00-00 2011"), Some("2011".into()));
    }

    #[test]
    fn extract_year_none() {
        assert_eq!(extract_year("no date"), None);
    }

    #[test]
    fn editors_formatted_separately() {
        let item = ZoteroItem {
            item_id: 1,
            item_key: "X".into(),
            item_type: "bookSection".into(),
            title: "Chapter".into(),
            date: None, doi: None, url: None, abstract_note: None,
            creators: vec![
                Creator { creator_type: "author".into(), first_name: Some("Alice".into()), last_name: "Author".into(), order: 0 },
                Creator { creator_type: "editor".into(), first_name: Some("Bob".into()), last_name: "Editor".into(), order: 1 },
            ],
            tags: vec![],
            date_added: "2024-01-01".into(),
            date_modified: "2024-01-01".into(),
        };
        let bib = item_to_bibtex(&item, "test", &HashMap::new(), "bibtex");
        assert!(bib.contains("author = {Author, Alice}"));
        assert!(bib.contains("editor = {Editor, Bob}"));
    }

    #[test]
    fn multiple_items_separated_by_blank_line() {
        let item = test_item();
        let entries = vec![
            (item.clone(), "key1".into(), HashMap::new()),
            (item, "key2".into(), HashMap::new()),
        ];
        let bib = items_to_bibtex(&entries, "bibtex");
        assert!(bib.contains("}\n\n@"));
    }
}
