# Changelog

All notable changes to this project will be documented in this file.

## [0.2.1] — 2026-04-26

### Maintenance

- Frozen-at-v0.2.1 maintenance posture. See `MAINTENANCE.md` for the
  contract: no new features, security-class fixes only, best-effort.
- Pinned dependency tree, CI green, no API or behavior changes.

## [0.2.0] — 2026-04-06

### Added

- `zotero_attach_pdf` resolves citekeys to Zotero item keys before
  upload (no longer requires the caller to pass an item key).
- Per-source failure reporting in the PDF resolver — failed sources
  surface their HTTP status / error so callers can diagnose offline
  fallbacks.
- FAQ section in the README with an architecture diagram.

### Fixed

- Keyword `model-context-protocol` shortened to `reference` to satisfy
  the crates.io 20-character keyword limit.
- `zotero_attach_pdf` now uses the global Zotero item template endpoint
  (the user-scoped endpoint was never reachable in practice).
- MCP tool descriptions stripped of concrete examples that confused
  some clients into echoing them back as inputs.
- `codecov-action` upgraded to v5 with explicit token plumbing for
  green coverage uploads.

### Documentation

- Custom rustdoc theme aligned with the project palette.
- Standardized install path to `~/.cargo/bin`.
- crates.io badge added; `cargo install biblion` documented as the
  primary install path.

### Tests

- Regression tests for four bugs surfaced during a demo session.

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
