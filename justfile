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

# Install binary to ~/.cargo/bin
install: build
    install -m 755 target/release/biblion ~/.cargo/bin/biblion

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

# Tag and release (triggers GitHub Actions → build binaries + publish crates.io)
release version:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "=== Releasing v{{version}} ==="
    echo ""
    # Guard: clean working tree
    if [ -n "$(git status --porcelain)" ]; then
        echo "ERROR: working tree is dirty. Commit or stash first."
        exit 1
    fi
    # Guard: on main branch
    BRANCH=$(git branch --show-current)
    if [ "$BRANCH" != "main" ]; then
        echo "ERROR: not on main branch (on $BRANCH)"
        exit 1
    fi
    # Preflight checks
    echo "→ Running tests..."
    cargo test --workspace --quiet
    echo "→ Running clippy..."
    cargo clippy --workspace -- -D warnings 2>&1 | tail -1
    echo "→ Checking format..."
    cargo fmt --all -- --check
    echo ""
    echo "→ Bumping versions to {{version}}..."
    # Update version in Cargo.toml files
    sed -i '' 's/^version = ".*"/version = "{{version}}"/' crates/biblion/Cargo.toml
    sed -i '' 's/^version = ".*"/version = "{{version}}"/' crates/paper-resolver/Cargo.toml
    sed -i '' 's/paper-resolver = { version = "[^"]*"/paper-resolver = { version = "{{version}}"/' crates/biblion/Cargo.toml
    # Verify it still builds
    cargo check --workspace --quiet
    echo ""
    echo "→ Committing and tagging..."
    git add -A
    # Commit only if there are staged changes (versions may already match)
    git diff --cached --quiet || git commit -m "release: v{{version}}"
    git tag -a "v{{version}}" -m "Release v{{version}}"
    echo ""
    echo "→ Pushing to origin..."
    git push && git push --tags
    echo ""
    echo "✓ Released v{{version}}"
    echo "  → GitHub Actions will build binaries for Linux + macOS"
    echo "  → GitHub Actions will publish to crates.io"
    echo "  → Check: https://github.com/eserie/biblion/releases"
