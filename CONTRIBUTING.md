# Contributing to Biblion

Thank you for your interest in contributing! Biblion is a focused project — a high-performance MCP server for Zotero. Contributions that improve reliability, test coverage, and documentation are especially welcome.

## Getting started

```bash
git clone https://github.com/eserie/biblion.git
cd biblion
just ci          # run the full check suite (fmt, lint, test, doc)
```

Requires [Rust](https://www.rust-lang.org/tools/install) 1.85+ and [just](https://github.com/casey/just).

## Development workflow

1. **Fork** the repo and create a branch from `main`
2. **Write code** — see [ARCHITECTURE.md](docs/ARCHITECTURE.md) for the crate structure and extension guides
3. **Test** — `just test` (all tests must pass)
4. **Lint** — `just lint` (zero clippy warnings with `-D warnings`)
5. **Format** — `just fmt` (CI enforces `cargo fmt`)
6. **Commit** — use [conventional commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `docs:`, `test:`, `refactor:`
7. **Open a PR** against `main`

## Adding a new MCP tool

1. Add handler in `crates/biblion/src/tools/{read,write,paper}.rs`
2. Add catalog entry in `tools/mod.rs` → `tool_catalog()`
3. Add dispatch arm in `tools/mod.rs` → `handle_tool_call()`
4. **Update `docs/MCP_INSTRUCTIONS.md`** — this is what agents see on connect. If you add a tool and don't update this file, agents won't know it exists.
5. Add a test (use `test_helpers::test_ctx()` for in-memory SQLite fixtures)

## Adding a new PDF source

1. Add `async fn try_my_source()` in `crates/paper-resolver/src/lib.rs`
2. Add source name to `SOURCE_NAMES` constant
3. Add match arm in `resolve_pdf_async()`
4. The source is automatically configurable via TOML and shown in `paper_source_status`

## Architecture decisions

Key design choices are documented as ADRs in [docs/adr/](docs/adr/):

- [ADR-001](docs/adr/001-sqlite-not-bbt-rpc.md): Read SQLite directly, not BBT JSON-RPC
- [ADR-002](docs/adr/002-two-crates-not-four.md): Two workspace crates, not four
- [ADR-003](docs/adr/003-write-tools-gated.md): Write tools gated by default

Please read these before proposing architectural changes.

## Good first issues

Check [issues labeled `good first issue`](https://github.com/eserie/biblion/labels/good%20first%20issue) for starter tasks.

## Code of conduct

Be respectful, be constructive, focus on the work.

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
