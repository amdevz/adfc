# Task entry points for this project. Everything here assumes the flake's
# devShell is active (direnv, or `nix develop`); CI invokes the same recipes so
# a green local run and a green CI run mean the same thing.
#
# `just lint` mirrors the cargo-clippy hook in .pre-commit-config.yaml exactly.
# When the two drift, whichever is weaker silently becomes the real standard.

# Show the available recipes.
default:
    @just --list

# Run everything CI runs; the gate before pushing.
check: fmt-check lint test test-node

# Run the Rust test suite.
test:
    cargo test

# Run the npm shim and packaging tests.
test-node:
    node --test "npm/test/**/*.test.js" "scripts/test/**/*.test.js"

# Lint with clippy at pedantic, warnings denied.
lint:
    cargo clippy --all-targets -- -D warnings -W clippy::pedantic

# Format the tree in place.
format:
    cargo fmt

# Fail if the tree is not formatted; style pinned in rustfmt.toml.
fmt-check:
    cargo fmt --check

# Build the release binary (stripped, LTO'd).
build:
    cargo build --release

# Check dependencies against the RustSec advisory database.
audit:
    cargo audit

# Run the full pre-commit suite, including file-hygiene hooks.
hooks:
    prek run --all-files

# Install the git hooks so they run on commit.
install-hooks:
    prek install --hook-type pre-commit --hook-type commit-msg

# Convert a Markdown file and pretty-print the ADF, to eyeball output.
run FILE:
    cargo run --quiet -- {{FILE}} | jq .

# Show what a release would produce, without building it.
dist-plan:
    dist plan

# Build release archives for every target. Needs cross-compilation toolchains;
# in practice this runs on CI, where dist provides them.
dist-build:
    dist build

# Generate the seven npm package trees from already-built release archives.
npm-packages PLAN="target/distrib/dist-manifest.json":
    node scripts/build-npm-packages.js --plan {{PLAN}}

# Pack, install and run the npm distribution locally, with no registry.
npm-e2e:
    ./scripts/npm-e2e.sh
