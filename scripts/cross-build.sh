#!/usr/bin/env bash
# Cross-compile the server as a static Linux x86_64 binary (musl).
# Requires Docker. Output: server/target/x86_64-unknown-linux-musl/release/regexr-server
set -euo pipefail

cd "$(dirname "$0")/../server"

docker run --rm -v "$PWD":/work -w /work rust:1-alpine \
    sh -c "apk add --no-cache musl-dev clang >/dev/null 2>&1 || apk add --no-cache musl-dev; cargo build --release --target x86_64-unknown-linux-musl"

echo "==> binary: $(pwd)/target/x86_64-unknown-linux-musl/release/regexr-server"
