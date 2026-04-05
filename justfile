# Biblion — task runner
# Install just: cargo install just

# Default: show available recipes
default:
    @just --list

# Build release binary
build:
    cargo build --release

# Run all tests
test:
    cargo test --workspace

# Run clippy (strict, CI-equivalent)
lint:
    cargo clippy --workspace -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Full CI check (local)
ci: fmt lint test
    cargo doc --workspace --no-deps

# Install binary to ~/.local/bin
install: build
    install -m 755 target/release/biblion ~/.local/bin/biblion

# Run diagnostics
check:
    biblion check

# Run coverage (requires cargo-tarpaulin)
coverage:
    cargo tarpaulin --workspace --out Stdout

# Run mutation testing (requires cargo-mutants)
mutants package="paper-resolver":
    cargo mutants --package {{package}} -- --release

# Start SSE server (for Claude Desktop)
serve:
    ZOTERO_MCP_TRANSPORT=sse biblion

# Load/reload LaunchAgent (SSE daemon)
daemon-start:
    launchctl load ~/Library/LaunchAgents/com.eserie.biblion.plist

daemon-stop:
    launchctl unload ~/Library/LaunchAgents/com.eserie.biblion.plist

daemon-restart: daemon-stop daemon-start
