#!/usr/bin/env bash
# Smoke-test a compiled binary against the full fixture corpus.
# Usage: scripts/smoke.sh [path-to-regexr-server]
#   default: the musl release binary from cross-build.sh; if absent,
#   builds a local debug binary first. Run after every release build.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/server/target/x86_64-unknown-linux-musl/release/regexr-server}"

if [ ! -x "$BIN" ]; then
  echo "==> no binary at $BIN; building local debug binary"
  cargo build --quiet --manifest-path "$ROOT/server/Cargo.toml"
  BIN="$ROOT/server/target/debug/regexr-server"
fi

echo "==> smoke-testing $BIN"
exec python3 "$ROOT/scripts/smoke_test.py" "$BIN"
