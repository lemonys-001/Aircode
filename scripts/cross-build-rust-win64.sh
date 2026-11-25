#!/usr/bin/env bash
# Cross-compiles dist/AI-Coding-Agent.exe and dist/AI-Coding-Agent-Setup.exe
# from Linux targeting x86_64-pc-windows-gnu (100% Pure Rust + static SQLite3).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "=== [1/3] Running Host Pure-Rust Unit Tests ==="
cargo test

echo "=== [2/3] Cross-Compiling dist/AI-Coding-Agent.exe (x86_64-pc-windows-gnu) ==="
cargo build --release --target x86_64-pc-windows-gnu
mkdir -p dist
cp target/x86_64-pc-windows-gnu/release/AI-Coding-Agent.exe dist/AI-Coding-Agent.exe

echo "=== [3/3] Cross-Compiling dist/AI-Coding-Agent-Setup.exe (x86_64-pc-windows-gnu) ==="
(
  cd installer
  cargo build --release --target x86_64-pc-windows-gnu
  cp target/x86_64-pc-windows-gnu/release/AI-Coding-Agent-Setup.exe ../dist/AI-Coding-Agent-Setup.exe
)

(
  cd dist
  sha256sum AI-Coding-Agent.exe AI-Coding-Agent-Setup.exe > SHA256SUMS.txt
)

ls -lh dist/AI-Coding-Agent.exe dist/AI-Coding-Agent-Setup.exe dist/SHA256SUMS.txt
