# zotero-mcp

[![Rust](https://img.shields.io/badge/rust-2024_edition-orange?logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-97_passing-brightgreen)]()
[![MCP](https://img.shields.io/badge/protocol-MCP_2024--11--05-purple)](https://modelcontextprotocol.io)
[![Binary](https://img.shields.io/badge/binary-5.8MB-informational)]()

High-performance MCP server that gives LLMs sub-millisecond access to your Zotero library. Rust. Direct SQLite. No plugins required.

---

You ask Claude a question about a paper in your Zotero library. The MCP server
looks it up, finds the BibTeX, locates the PDF. How long should that take?

The existing Python MCP server for Zotero takes 500ms-2s per query. Most of that
time is spent round-tripping through Better BibTeX's JSON-RPC interface --
JavaScript running inside Zotero's Electron process. **zotero-mcp** skips all of
that. It reads Zotero's SQLite database directly, because it turns out citation
keys are already stored there. The result: reads that took 500ms now complete in
under a millisecond. A single 5.8 MB binary. 97 tests. 25 tools that give your
LLM full access to your reference library -- search, cite, export BibTeX, format
bibliographies, resolve PDFs from 9 academic sources.

Two transports: stdio for Claude Code (CLI), SSE for Claude Desktop. Install it,
point it at your Zotero database, and your LLM can cite papers as fast as you can
think of them.

## Quick start

```bash
# Build from source
git clone https://github.com/eserie/zotero-mcp-rs.git
cd zotero-mcp-rs
cargo build --release

# Install
cp target/release/zotero-mcp ~/.local/bin/

# Configure Claude Code (stdio)
# Add to ~/.claude.json under "mcpServers":
{
  "zotero": {
    "command": "/path/to/zotero-mcp",
    "env": {
      "ZOTERO_API_KEY": "your-api-key-here"
    }
  }
}

# Verify
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | zotero-mcp
```

## Available tools

### Search & Browse

| Tool | Description |
|------|-------------|
| `zotero_search` | Full-text search across titles, DOIs, abstracts |
| `zotero_get_item` | Get full metadata for an item by citation key |
| `zotero_get_recent` | Recently modified items |
| `zotero_get_collections` | List all collections with hierarchy |
| `zotero_get_collection_items` | Items in a specific collection |
| `zotero_status` | Library statistics |

### Citations & Export

| Tool | Description |
|------|-------------|
| `zotero_get_bibtex` | Export as BibTeX or BibLaTeX (native, no BBT needed) |
| `zotero_get_bibliography` | Formatted bibliography (APA, IEEE native; others via BBT fallback) |
| `zotero_export_bibtex` | Export entire collection as BibTeX |

### Attachments & Notes

| Tool | Description |
|------|-------------|
| `zotero_get_pdf_path` | Filesystem paths to PDF attachments |
| `zotero_list_attachments` | All attachments for an item |
| `zotero_get_notes` | Item notes (HTML to text) |

### Write Operations (requires API key)

| Tool | Description |
|------|-------------|
| `zotero_create_item` | Create a new library item |
| `zotero_update_item` | Update metadata fields |
| `zotero_add_tags` | Add tags (preserves existing) |
| `zotero_add_note` | Add a note to an item |
| `zotero_create_collection` | Create a new collection |
| `zotero_add_to_collection` | Add item to collection |
| `zotero_remove_from_collection` | Remove item from collection |
| `zotero_delete_item` | Delete permanently |
| `zotero_merge_items` | Merge duplicates |
| `zotero_attach_pdf` | Download and attach a PDF |
| `zotero_fetch_missing_pdfs` | Bulk PDF resolution from 9 academic sources |

### PDF Resolver Sources

The PDF resolver queries 9 sources concurrently and returns the best
downloadable result:

1. **arXiv** -- instant regex match (no network)
2. **OpenAlex** -- 250M+ works
3. **CORE** -- 300M+ open-access works
4. **Google Scholar** -- university mirrors, author pages
5. **Unpaywall** -- 30M+ OA articles
6. **Crossref** -- publisher PDF links
7. **Zenodo** -- cross-disciplinary preprints (CERN)
8. **SSRN** -- finance/economics preprints
9. **Semantic Scholar** -- OA PDFs + disclaimer field parsing

## Comparison

|                          | **zotero-mcp** (Rust) | zotero-mcp (Python) | BBT JSON-RPC |
|--------------------------|:---------------------:|:-------------------:|:------------:|
| Read latency             | **<1ms** (warm cache) | 500ms-2s            | 500ms-2s     |
| Requires Zotero running  | **No**                | Yes (BBT)           | Yes          |
| Requires BBT plugin      | **No**                | Yes                 | Yes          |
| MCP protocol             | Yes                   | Yes                 | No           |
| Transport                | **stdio + SSE**       | stdio               | JSON-RPC     |
| BibTeX generation        | **Native**            | Via BBT             | Yes          |
| Bibliography (APA/IEEE)  | **Native**            | Via BBT             | Yes          |
| PDF resolver             | **9 concurrent**      | 9 concurrent        | None         |
| Binary size              | **5.8 MB**            | ~50 MB (venv)       | N/A          |
| Runtime dependencies     | **None**              | Python 3.10+        | Zotero + BBT |

## Why Rust?

The honest answer: **performance was the problem, and Rust solved it.**

The Python MCP server works. But every query passes through Better BibTeX's
JSON-RPC interface -- JavaScript executing inside Zotero's Electron process.
That adds 500ms-2s of latency per tool call. A performance audit showed that
95% of the time was spent in the BBT bridge, not in the actual database read.

The fix was to read SQLite directly. And once you are doing direct SQLite reads,
Rust gives you three things for free:

1. **No runtime overhead.** No GC pauses, no interpreter startup, no venv.
   The binary is 5.8 MB and starts in microseconds.
2. **Fearless concurrency.** The 9-source PDF resolver fires all sources in
   parallel via `tokio::join!`.
3. **Single binary distribution.** Download and run. No dependency hell.

Could we have gotten 80% of the speed improvement by reading SQLite from
Python? Yes. But the remaining 20% -- startup time, memory footprint,
concurrent PDF resolution, and distributing a single file instead of a
virtualenv -- made Rust the right call.

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `ZOTERO_MCP_TRANSPORT` | `stdio` | Transport: `stdio` or `sse` |
| `ZOTERO_MCP_HOST` | `127.0.0.1` | SSE listen address |
| `ZOTERO_MCP_PORT` | `23120` | SSE listen port |
| `ZOTERO_API_KEY` | -- | Required for write operations |
| `ZOTERO_LIBRARY_ID` | -- | Your Zotero library ID (for writes) |
| `ZOTERO_LIBRARY_TYPE` | `user` | Library type (`user` or `group`) |
| `ZOTERO_SQLITE_PATH` | `~/Zotero/zotero.sqlite` | Path to Zotero database |
| `ZOTERO_STORAGE_PATH` | `~/Zotero/storage` | Path to PDF storage |
| `BBT_MIGRATED_PATH` | `~/Zotero/better-bibtex.migrated` | BBT citekey database |
| `BBT_URL` | `http://localhost:23119/better-bibtex/json-rpc` | BBT RPC (bibliography fallback) |

### SSE mode (for Claude Desktop)

```bash
# Start as daemon
ZOTERO_MCP_TRANSPORT=sse ZOTERO_MCP_PORT=23120 zotero-mcp

# Configure Claude Desktop
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

Requires Rust 1.85+ (edition 2024).

## Architecture

```
Claude <-stdio/sse-> zotero-mcp
                         |
              +----------+----------+
              |          |          |
         Read Tools   Write Tools  PDF Resolver
         (sync SQLite) (reqwest)   (tokio async)
              |          |          |
         zotero.sqlite  Zotero    9 sources
         bbt.migrated   Web API   concurrent
```

- **Read tools** (9): pure SQLite, sub-millisecond. The performance win.
- **BibTeX/bibliography** (3): native formatting for APA/IEEE, BBT fallback for exotic styles.
- **Write tools** (14): Zotero Web API via reqwest.
- **PDF resolver**: 9 sources queried concurrently via tokio.

## Performance notes

- Read latency: <1ms for single-item lookups on a warm OS page cache.
  First query after startup may take 5-50ms depending on disk cache state.
  Full-text search across ~2700 items: ~5-10ms.
- The speed improvement comes from eliminating the BBT JSON-RPC bottleneck
  (JavaScript in Electron), not from Rust being faster than Python at SQLite reads.
- Write operations go through the Zotero Web API at the same speed as the Python server (~200-500ms).
- Unit tests cover read path, protocol, and formatting. Write tools and network
  resolvers are tested manually against live Zotero.

## Contributing

Contributions welcome. Please run `cargo test` and `cargo clippy` before submitting.

## License

MIT
