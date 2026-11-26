#!/usr/bin/env bash
# Cross-compiles dist/aircode.exe and dist/aircode-setup.exe
# from Linux targeting x86_64-pc-windows-gnu (100% Pure Rust + static SQLite3).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "=== [1/3] Running Host Pure-Rust Unit Tests ==="
cargo test

echo "=== [2/3] Cross-Compiling dist/aircode.exe (x86_64-pc-windows-gnu) ==="
cargo build --release --target x86_64-pc-windows-gnu
mkdir -p dist
cp target/x86_64-pc-windows-gnu/release/aircode.exe dist/aircode.exe

echo "=== [3/3] Cross-Compiling dist/aircode-setup.exe (x86_64-pc-windows-gnu) ==="
(
  cd installer
  cargo build --release --target x86_64-pc-windows-gnu
  cp target/x86_64-pc-windows-gnu/release/aircode-setup.exe ../dist/aircode-setup.exe
)

(
  cd dist
  sha256sum aircode.exe aircode-setup.exe > SHA256SUMS.txt
)

ls -lh dist/aircode.exe dist/aircode-setup.exe dist/SHA256SUMS.txt
