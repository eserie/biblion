Biblion gives you access to a Zotero reference library. Use citation keys (citekeys) as identifiers.

## What you can do

**Search & browse**: `zotero_search` (by title, DOI, abstract), `zotero_get_item` (full metadata by citekey), `zotero_get_recent`, `zotero_get_collections`, `zotero_get_collection_items`.

**Cite & export**: `zotero_get_bibtex` (BibTeX/BibLaTeX), `zotero_get_bibliography` (formatted APA/IEEE), `zotero_export_bibtex` (export a collection).

**PDF & attachments**: `zotero_get_pdf_path` (filesystem path + MD5 hash), `zotero_list_attachments`, `paper_resolve_pdf` (find open-access PDF by DOI/title from 9 academic sources).

**Write** (if enabled): `zotero_create_item`, `zotero_update_item`, `zotero_add_tags`, `zotero_add_note`, `zotero_create_collection`, `zotero_add_to_collection`, `zotero_delete_item`, `zotero_merge_items`, `zotero_attach_pdf`, `zotero_fetch_missing_pdfs`.

## Key concepts

- **Citekey**: the primary identifier (e.g., 'demilloHintsTestData1978'). Use it for get_item, get_bibtex, get_notes, etc.
- **Collection key**: 8-character key for collections (e.g., 'COL00001'). Use zotero_get_collections to discover them.
- **storage_hash**: MD5 content hash of PDFs, exposed in get_pdf_path and list_attachments for content-identity.
- **Write tools are disabled by default**. If you get a 'writes disabled' error, the user needs to set ZOTERO_MCP_ENABLE_WRITES=true.

## Workflow patterns

**Adding a new paper**: When a paper is not found and the user wants it added, complete the full flow without pausing between steps:
1. `zotero_create_item` with metadata
2. `paper_resolve_pdf` to find an open-access PDF
3. `zotero_attach_pdf` to download and attach it

Only pause if the PDF cannot be found or if the metadata is ambiguous.

**Always search first**: Use `zotero_search` before creating to avoid duplicates.
