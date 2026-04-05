# Architecture

> *Academic knowledge made addressable.*

Biblion takes knowledge that exists (in Zotero's SQLite database) and makes it addressable — findable and retrievable by any process that speaks MCP or invokes the CLI.

## Crate structure

```
biblion/                          [workspace]
├── crates/
│   ├── paper-resolver/           [lib] 9-source PDF resolver
│   │   └── src/lib.rs            No Zotero dependency, publishable standalone
│   └── biblion/                  [bin] MCP server
│       └── src/
│           ├── main.rs           Entry point, transport selection, check subcommand
│           ├── config.rs         Config from env vars + optional TOML
│           ├── protocol.rs       MCP JSON-RPC types
│           ├── server.rs         stdio transport + shared dispatch
│           ├── sse.rs            SSE transport (axum)
│           ├── db/               SQLite readers (zotero.sqlite + bbt.migrated)
│           ├── api/              HTTP clients (BBT RPC, Zotero Web API)
│           └── tools/            25 MCP tool handlers
│               ├── read.rs       9 read tools (pure SQLite)
│               ├── write.rs      11 write tools (Zotero Web API)
│               ├── paper.rs      2 paper discovery tools
│               ├── bibtex.rs     Native BibTeX/BibLaTeX generation
│               ├── bibliography.rs  Native APA/IEEE formatting
│               └── format.rs     Item formatting, HTML→text, extract_year
```

## Data flow

```
Claude → stdio/SSE → JSON-RPC parse → dispatch
                                         │
                         ┌───────────────┼───────────────┐
                         │               │               │
                    Read tools      Write tools     Paper tools
                    (sync)          (blocking HTTP)  (tokio async)
                         │               │               │
                    zotero.sqlite   Zotero Web API  9 academic APIs
                    bbt.migrated    (reqwest)       (concurrent)
                         │
                    <1ms response
```

1. Client sends JSON-RPC request via stdio pipe or SSE HTTP POST
2. `server::dispatch()` routes to the appropriate tool handler
3. Read tools query SQLite directly (sub-millisecond)
4. Write tools call the Zotero Web API via reqwest (~200-500ms)
5. Paper tools use paper-resolver with tokio for concurrent HTTP
6. Response is serialized and sent back via the same transport

## How to add a new tool

1. **Add handler** in `tools/read.rs` (or `write.rs` / `paper.rs`):
   ```rust
   pub fn zotero_my_tool(args: &Value, ctx: &ServerContext) -> ToolCallResult {
       // ... implementation
       ToolCallResult::text("result".into())
   }
   ```

2. **Add to catalog** in `tools/mod.rs` (`tool_catalog()` function):
   ```rust
   tools.push(tool("zotero_my_tool", "Description.", json!({
       "type": "object",
       "properties": { "param": { "type": "string" } },
       "required": ["param"]
   })));
   ```

3. **Add to dispatch** in `tools/mod.rs` (`handle_tool_call()` function):
   ```rust
   "zotero_my_tool" => read::zotero_my_tool(args, ctx),
   ```

## How to add a new PDF source

1. **Add async handler** in `crates/paper-resolver/src/lib.rs`:
   ```rust
   async fn try_my_source(client: &reqwest::Client, doi: Option<&str>, title: Option<&str>) -> Option<ResolvedPdf> {
       // Query the API, return ResolvedPdf if found
   }
   ```

2. **Add source name** to `SOURCE_NAMES` constant.

3. **Add to dispatch loop** in `resolve_pdf_async()`:
   ```rust
   "my_source" => futures.push(Box::pin(async move {
       try_my_source(c, doi, title).await.map(|r| (pri, r))
   })),
   ```

The source will automatically be configurable via TOML and shown in `paper_source_status`.
