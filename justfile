#!/usr/bin/env just --justfile

export RUST_BACKTRACE := "full"

# Format the whole workspace.
fmt:
  @echo "Running cargo fmt..."
  cargo fmt --all

# Type-check the whole workspace (fast, no wasm build).
check:
  @echo "Running cargo check..."
  cargo check --workspace

# Run all contract unit tests.
test:
  @echo "Running cargo test..."
  cargo test --workspace

# Clippy with the workspace deny lints (unwrap/expect/indexing/arithmetic) + -D warnings.
clippy:
  @echo "Running cargo clippy..."
  cargo clippy --workspace --all-targets -- -D warnings

# Auto-fix clippy warnings.
clippy-fix:
  @echo "Running cargo clippy with automatic fixes..."
  cargo clippy --fix --allow-dirty --allow-staged --workspace --all-targets -- -D warnings

# Full lint pass: fmt → clippy-fix → clippy.
lint: fmt clippy-fix clippy

# Build a single contract's artifacts (wasm + metadata → target/ink/<name>).
build name:
  @echo "Building {{name}} contract..."
  cargo contract build --release --manifest-path contracts/{{name}}/Cargo.toml

# Build all contract artifacts (wasm + metadata).
build-all:
  @for c in tusdt-erc20 tusdt-auction tusdt-oracle tusdt-vault-alpha tusdt-treasury tusdt-governance tusdt-election tusdt-lending-pool; do \
    echo "Building $$c..."; \
    cargo contract build --release --manifest-path contracts/$c/Cargo.toml || exit 1; \
  done
