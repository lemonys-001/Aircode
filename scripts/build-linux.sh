#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "=== [1/2] Running Pure-Rust Unit & Headless Render Tests ==="
cargo test

echo "=== [2/2] Building Native Linux Binary (target/release/aircode) ==="
cargo build --release
ls -lh target/release/aircode
