# CLAUDE.md — Agent Instructions for Biblion

## What is this?

Biblion is a high-performance MCP server that gives LLMs sub-millisecond access to Zotero reference libraries. Written in Rust.

- **Repo**: https://github.com/eserie/biblion
- **Identity**: *"Your library, legible to every intelligence"*
- **Architecture**: *"Academic knowledge made addressable"*

## Workspace structure

```
biblion/
  Cargo.toml              [workspace]
  crates/
    paper-resolver/        [lib] 9-source concurrent PDF resolver (no Zotero dependency)
    biblion/               [bin] MCP server (25 tools, stdio + SSE)
  docs/
    ARCHITECTURE.md        Crate diagram, data flow, extension guides
    adr/                   Architecture Decision Records (001-003)
  .github/workflows/
    ci.yml                 check, test, clippy, fmt, doc, coverage
    mutants.yml            nightly mutation testing
    release.yml            build binaries + publish crates.io on tag
  justfile                 Task runner (just install, just ci, just release)
```

## Quick reference

```bash
just ci            # Full local CI: fmt, lint, test, doc
just test          # cargo test --workspace
just lint          # cargo clippy --workspace -- -D warnings
just fmt           # cargo fmt --all
just install       # Build release + install to ~/.local/bin
just check         # Run biblion diagnostics
just coverage      # Code coverage (cargo-tarpaulin)
just mutants       # Mutation testing (cargo-mutants)
just release X.Y.Z # Tag + push (triggers CI build + publish)
```

## Conventions

### Code style
- **Rust 2024 edition**, MSRV 1.85
- `cargo fmt` before every commit
- `cargo clippy --workspace -- -D warnings` must pass (zero warnings)
- `#![allow(dead_code)]` at crate root for API completeness (fields used in tests or reserved)

### Testing
- 178+ tests (56 paper-resolver + 122 biblion)
- In-memory SQLite fixtures in `crates/biblion/src/test_helpers.rs`
- All DB tests use `ZoteroDb::from_connection()` with realistic EAV schema
- Network-dependent code (PDF resolver sources, Zotero Web API) not mocked yet (see issues #4, #5)

### Architecture (see docs/ARCHITECTURE.md)
- **Read tools** (9): pure SQLite, sub-millisecond. The performance win.
- **Write tools** (11): Zotero Web API via reqwest. Gated behind `ZOTERO_MCP_ENABLE_WRITES`.
- **Content-identity**: Biblion exposes `storage_hash` (MD5), `pdf_path`, `citekey`, `doi` as primitives. It does NOT compute hashes or deduplicate — it reads what Zotero stored and exposes it for external tools.
- **Paper tools** (2): paper-resolver library, tokio async.
- **Export tools** (3): native BibTeX/BibLaTeX + APA/IEEE bibliography.
- **Transports**: stdio (default, for Claude Code) + SSE (for Claude Desktop).
- **Dispatch**: `server::dispatch()` is shared between stdio and SSE.

### Adding a new tool
1. Handler in `crates/biblion/src/tools/{read,write,paper}.rs`
2. Catalog entry in `tools/mod.rs` → `tool_catalog()`
3. Dispatch arm in `tools/mod.rs` → `handle_tool_call()`
4. **Update `docs/MCP_INSTRUCTIONS.md`** — agents see this on connect (compiled into the binary via `include_str!`)

### Adding a new PDF source
1. Async handler `try_my_source()` in `crates/paper-resolver/src/lib.rs`
2. Add name to `SOURCE_NAMES` constant
3. Add match arm in `resolve_pdf_async()`

### Key ADRs
- **ADR-001**: Read SQLite directly, not BBT JSON-RPC (the core insight)
- **ADR-002**: Two crates, not four (pragmatic split)
- **ADR-003**: Write tools gated by default (safety)

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `ZOTERO_SQLITE_PATH` | `~/Zotero/zotero.sqlite` | Zotero database |
| `ZOTERO_STORAGE_PATH` | `~/Zotero/storage` | PDF storage |
| `BBT_MIGRATED_PATH` | `~/Zotero/better-bibtex.migrated` | Citekey database |
| `ZOTERO_API_KEY` | — | Required for write tools |
| `ZOTERO_LIBRARY_ID` | — | Required for write tools |
| `ZOTERO_MCP_ENABLE_WRITES` | `false` | Explicitly enable writes |
| `ZOTERO_MCP_TRANSPORT` | `stdio` | `stdio` or `sse` |
| `ZOTERO_MCP_CONFIG` | `~/.config/biblion/config.toml` | Optional TOML config |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full human-facing guide. Key points for agents:

- Branch from `main`, use conventional commits (`feat:`, `fix:`, `docs:`, `test:`)
- Every new tool needs: handler + catalog entry + dispatch arm + test
- Every new PDF source needs: async handler + SOURCE_NAMES entry + match arm
- Run `just ci` before pushing (fmt + lint + test + doc)
- Read the ADRs before proposing architectural changes

## What NOT to do

- **Never write to zotero.sqlite directly** — all writes go through the Zotero Web API
- **Never hardcode personal data** (library IDs, emails, paths) — use env vars
- **Never commit secrets** — API keys go in env vars or macOS Keychain
- **Never skip `cargo fmt`** — CI will reject unformatted code
- **Never add dependencies without justification** — the binary should stay small (~6 MB)
