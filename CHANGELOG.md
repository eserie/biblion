# Changelog

All notable changes to this project will be documented in this file.

## [0.1.0] — 2026-04-05

### Added
- 25 MCP tools: 9 read, 3 citation export, 2 paper discovery, 11 write
- Direct SQLite access to Zotero database (sub-millisecond reads)
- Native BibTeX/BibLaTeX export (no BBT dependency)
- Native APA and IEEE bibliography formatting
- 9-source concurrent PDF resolver (arXiv, OpenAlex, CORE, Google Scholar, Unpaywall, Crossref, Zenodo, SSRN, Semantic Scholar)
- Dual transport: stdio (Claude Code) and SSE (Claude Desktop)
- Optional TOML configuration for source priority and enable/disable
- Write tools gated behind `ZOTERO_MCP_ENABLE_WRITES` for safety
- `biblion check` diagnostic subcommand
- paper-resolver extracted as standalone publishable crate

### Architecture
- Workspace with 2 crates: `biblion` (server) + `paper-resolver` (library)
- CI: GitHub Actions (check, test, clippy, fmt, doc, coverage, nightly mutation testing)
