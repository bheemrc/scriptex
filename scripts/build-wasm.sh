#!/bin/bash
set -euo pipefail

# Build ScripTeX as a WASM module for browser use
cd "$(dirname "$0")/.."

echo "Building WASM module..."

# Set RUSTFLAGS to enable bulk-memory (all modern browsers support it)
export RUSTFLAGS="-C target-feature=+bulk-memory,+nontrapping-fptoint"

wasm-pack build \
    --target web \
    --out-dir pkg \
    --no-default-features \
    --features wasm

echo "WASM build complete:"
echo "  Size: $(du -h pkg/scriptex_bg.wasm | cut -f1)"
echo "  Files:"
ls -la pkg/
