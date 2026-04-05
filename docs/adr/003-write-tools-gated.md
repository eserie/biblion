# ADR-003: Write tools gated by default

## Status
Accepted

## Context

Biblion exposes 25 MCP tools. 14 of them modify the Zotero library: create items, delete items, merge duplicates, add tags, attach PDFs. An LLM calling `zotero_delete_item` unsupervised could permanently destroy references.

## Decision

Write tools are disabled by default. Users must explicitly set `ZOTERO_MCP_ENABLE_WRITES=true` to enable them. Additionally, `ZOTERO_API_KEY` must be set (writes go through the Zotero Web API, not direct SQLite writes).

The tool catalog always lists write tools (so the LLM knows they exist), but execution returns a clear error when writes are disabled.

## Consequences

- Safe by default: a new user cannot accidentally lose data
- The LLM sees the full tool catalog and can suggest enabling writes if needed
- Power users opt in with one environment variable
- No write operation ever touches `zotero.sqlite` directly (all writes go through Zotero's Web API to preserve sync integrity)
