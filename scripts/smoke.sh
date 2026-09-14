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
  # rust-embed embeds server/static/ at compile time (gitignored build
  # output) — the frontend build must precede cargo on a fresh checkout.
  if [ ! -f "$ROOT/server/static/index.html" ]; then
    ./scripts/build-frontend.sh
  fi
  cargo build --quiet --locked --manifest-path "$ROOT/server/Cargo.toml"
  BIN="$ROOT/server/target/debug/regexr-server"
fi

echo "==> smoke-testing $BIN"
exec python3 "$ROOT/scripts/smoke_test.py" "$BIN"
