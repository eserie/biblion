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

# Tag and release (triggers GitHub Actions release workflow)
release version:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Releasing v{{version}}..."
    # Preflight checks
    cargo test --workspace
    cargo clippy --workspace -- -D warnings
    cargo fmt --all -- --check
    # Update version in Cargo.toml files
    sed -i '' 's/^version = ".*"/version = "{{version}}"/' crates/biblion/Cargo.toml
    sed -i '' 's/^version = ".*"/version = "{{version}}"/' crates/paper-resolver/Cargo.toml
    sed -i '' 's/paper-resolver = { version = ".*"/paper-resolver = { version = "{{version}}"/' crates/biblion/Cargo.toml
    # Commit, tag, push
    git add -A
    git commit -m "release: v{{version}}"
    git tag -a "v{{version}}" -m "v{{version}}"
    git push && git push --tags
    echo "✓ Released v{{version}} — GitHub Actions will build binaries"
