# ADR-002: Two workspace crates, not four

## Status
Accepted

## Context

The codebase has natural boundaries: MCP protocol types, PDF resolution, Zotero database access, and the server itself. An initial proposal split these into 4 crates: `paper-resolver`, `mcp-protocol`, `biblion` (server), and `paper-mcp` (standalone paper search server).

Three expert reviewers (Torvalds, Feynman, Jobs) independently recommended fewer crates.

## Decision

Two crates:
- **paper-resolver** — standalone library for academic PDF resolution. Zero Zotero dependency, publishable to crates.io independently.
- **biblion** — the MCP server. Contains protocol types, database access, tools, transports.

We do not extract:
- `mcp-protocol` — 269 lines of serde structs. Too small. Will diverge between MCP servers.
- `paper-mcp` — no user today. Extract when someone needs a standalone paper search server.
- Domain types (`ZoteroItem`, etc.) — one consumer. Extract when a second consumer appears.

## Consequences

- Simple workspace, low coordination overhead
- paper-resolver is independently useful and testable
- MCP protocol types are duplicated with ox-mcp (acceptable at 269 lines)
- Adding a third crate later is straightforward (the boundary is already clean)
