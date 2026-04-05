# ADR-001: Read SQLite directly, not BBT JSON-RPC

## Status
Accepted

## Context

Zotero stores all data in a local SQLite database (`zotero.sqlite`). The standard way to access this data programmatically is through Better BibTeX (BBT), a Zotero plugin that exposes a JSON-RPC API on `localhost:23119`. BBT provides citation key resolution, CSL-JSON export, and bibliography formatting.

The problem: every BBT JSON-RPC call takes 200-500ms because it wakes JavaScript inside Zotero's Electron process. An MCP tool call that chains search → get_item → get_bibtex takes 1-2 seconds through BBT.

## Decision

Read `zotero.sqlite` directly in read-only mode. Resolve citation keys from the `citationKey` field in Zotero's EAV schema (99.9% coverage) and from `better-bibtex.migrated` as fallback.

BBT is only used as a fallback for exotic CSL bibliography styles (Chicago, Vancouver, etc.) that require the CSL engine. APA and IEEE are implemented natively.

## Consequences

- Read latency: <1ms (was 200-500ms per BBT call)
- Zotero does not need to be running for read operations
- No dependency on the BBT plugin for reads
- We must understand and maintain queries against Zotero's EAV schema
- Schema changes in Zotero updates could break queries (mitigated by read-only access and tests against realistic fixtures)
