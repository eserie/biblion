# zotero-mcp

[![CI](https://github.com/eserie/zotero-mcp-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/eserie/zotero-mcp-rs/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-41%25-yellow)](https://github.com/eserie/zotero-mcp-rs)
[![Mutation Score](https://img.shields.io/badge/mutation_score-72%25-green)](https://github.com/eserie/zotero-mcp-rs)
[![Rust](https://img.shields.io/badge/rust-2024_edition-orange?logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MCP](https://img.shields.io/badge/protocol-MCP_2024--11--05-purple)](https://modelcontextprotocol.io)

An MCP server that connects your Zotero library to Claude and other LLMs. Search papers, export BibTeX, generate bibliographies, and find open-access PDFs — all from your AI assistant.

---

## What it does

You ask Claude "find me the BibTeX for that portfolio optimization paper" and it searches your Zotero library, finds the item, and returns the citation — in under a millisecond. No plugins needed, Zotero does not even need to be running. The server reads your local Zotero database directly and exposes 26 tools: search, browse, cite, export, organize, and resolve PDFs from 9 academic sources.

## Quick start

**1. Build and install**

```bash
git clone https://github.com/eserie/zotero-mcp-rs.git
cd zotero-mcp-rs
cargo build --release
cp target/release/zotero-mcp ~/.local/bin/
```

**2. Add to Claude Code** (in `~/.claude.json` under `"mcpServers"`):

```json
{
  "zotero": {
    "command": "zotero-mcp"
  }
}
```

**3. Verify**

```bash
zotero-mcp check
```

That is it. Claude can now search your library, export citations, and format bibliographies.

> For **Claude Desktop**, use SSE mode instead — see [Configuration](#sse-mode) below.

## Features

### Read tools — instant, no network needed

| Tool | What it does |
|------|-------------|
| `zotero_search` | Full-text search across titles, DOIs, abstracts |
| `zotero_get_item` | Full metadata for an item by citation key |
| `zotero_get_recent` | Recently modified items |
| `zotero_get_collections` | List all collections with hierarchy |
| `zotero_get_collection_items` | Items in a specific collection |
| `zotero_get_notes` | Item notes (HTML converted to text) |
| `zotero_get_pdf_path` | Filesystem path to PDF attachments |
| `zotero_list_attachments` | All attachments for an item |
| `zotero_status` | Library statistics |

### Citations and export

| Tool | What it does |
|------|-------------|
| `zotero_get_bibtex` | Export as BibTeX or BibLaTeX |
| `zotero_get_bibliography` | Formatted bibliography (APA, IEEE, and more) |
| `zotero_export_bibtex` | Export an entire collection as BibTeX |

### Paper discovery — search and resolve PDFs beyond your library

| Tool | What it does |
|------|-------------|
| `paper_search` | Search for open-access papers by title or keywords |
| `paper_resolve_pdf` | Find a downloadable PDF by DOI, title, or URL |
| `paper_source_status` | Show configured sources and their status |

PDF resolution queries 9 academic sources concurrently: arXiv, OpenAlex, CORE, Google Scholar, Unpaywall, Crossref, Zenodo, SSRN, and Semantic Scholar.

### Write tools — organize your library from the chat

| Tool | What it does |
|------|-------------|
| `zotero_create_item` | Create a new library item |
| `zotero_update_item` | Update metadata fields |
| `zotero_add_tags` | Add tags (preserves existing) |
| `zotero_add_note` | Add a note to an item |
| `zotero_create_collection` | Create a new collection |
| `zotero_add_to_collection` | Add item to a collection |
| `zotero_remove_from_collection` | Remove item from a collection |
| `zotero_delete_item` | Delete an item permanently |
| `zotero_merge_items` | Merge duplicate items |
| `zotero_attach_pdf` | Download and attach a PDF |
| `zotero_fetch_missing_pdfs` | Bulk-find PDFs for items missing them |

Write tools are **disabled by default**. To enable them, set both `ZOTERO_API_KEY` and `ZOTERO_MCP_ENABLE_WRITES=true`.

## Configuration

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `ZOTERO_SQLITE_PATH` | `~/Zotero/zotero.sqlite` | Path to your Zotero database |
| `ZOTERO_STORAGE_PATH` | `~/Zotero/storage` | Path to PDF storage |
| `ZOTERO_MCP_TRANSPORT` | `stdio` | Transport mode: `stdio` or `sse` |
| `ZOTERO_MCP_HOST` | `127.0.0.1` | SSE listen address |
| `ZOTERO_MCP_PORT` | `23120` | SSE listen port |
| `ZOTERO_API_KEY` | — | Zotero API key (required for write tools) |
| `ZOTERO_LIBRARY_ID` | — | Your Zotero library ID (for write tools) |
| `ZOTERO_MCP_ENABLE_WRITES` | `false` | Explicitly enable write tools |

### TOML config file (optional)

Place a file at `~/.config/zotero-mcp/config.toml` to configure the PDF resolver:

```toml
[resolver]
email = "you@university.edu"       # polite-pool access for Unpaywall/Crossref
timeout_secs = 15

# Enable/disable sources, set priority by order
[[resolver.sources]]
name = "arxiv"
enabled = true

[[resolver.sources]]
name = "openalex"
enabled = true

[[resolver.sources]]
name = "unpaywall"
enabled = true

[[resolver.sources]]
name = "ssrn"
enabled = false                     # disable sources you don't need
```

Override the config path with `ZOTERO_MCP_CONFIG=/path/to/config.toml`.

### SSE mode

For Claude Desktop or other HTTP-based clients:

```bash
ZOTERO_MCP_TRANSPORT=sse zotero-mcp
```

Then in Claude Desktop settings:

```json
{
  "zotero": {
    "type": "sse",
    "url": "http://127.0.0.1:23120/sse"
  }
}
```

## Building from source

```bash
cargo build --release
cargo test
```

Requires Rust 1.85+ (2024 edition). The workspace contains two crates:

- **zotero-mcp** — the MCP server
- **paper-resolver** — standalone library for academic PDF resolution (usable independently)

## License

MIT
