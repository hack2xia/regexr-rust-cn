#!/usr/bin/env bash
# Cross-compile the server as a static Linux x86_64 binary (musl).
# Requires Docker. Output: server/target/x86_64-unknown-linux-musl/release/regexr-server
#
# NOTE: image tag is pinned (not `rust:1-alpine`) so a surprise base-image
# update can never break a build that worked before. Bump deliberately.
set -euo pipefail

cd "$(dirname "$0")/../server"

docker run --rm -v "$PWD":/work -w /work rust:1.98.1-alpine3.21 \
    sh -c "apk add --no-cache musl-dev clang >/dev/null 2>&1 || apk add --no-cache musl-dev; cargo build --release --target x86_64-unknown-linux-musl"

echo "==> binary: $(pwd)/target/x86_64-unknown-linux-musl/release/regexr-server"
