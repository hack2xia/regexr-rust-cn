#!/usr/bin/env bash
# Build macOS release binaries locally.
#
# With rustup: builds x86_64 + aarch64 and a Universal binary (lipo).
# Without rustup (e.g. MacPorts rust): builds the host arch only — the
# rust-std for the other arch is a rustup-managed component — and prints
# a note. Either way, the GitHub Release workflow (release.yml) builds
# both arches in the cloud.
# Usage: scripts/build-macos.sh [--smoke]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/server"

HOST="$(rustc -vV | sed -n 's/^host: //p')"
TARGETS=("$HOST")

if command -v rustup >/dev/null 2>&1; then
  TARGETS=(x86_64-apple-darwin aarch64-apple-darwin)
else
  echo "==> note: rustup not found ($(rustc --version)); building host arch only."
  echo "    install rustup for both arches + universal locally; the Release"
  echo "    workflow builds both regardless."
fi

for t in "${TARGETS[@]}"; do
  echo "==> build $t"
  if command -v rustup >/dev/null 2>&1; then
    rustup target add "$t"
  fi
  cargo build --release --target "$t"
done

if [ "${#TARGETS[@]}" -eq 2 ]; then
  echo "==> universal binary (lipo)"
  mkdir -p target/universal
  lipo -create -output target/universal/regexr-server \
    target/x86_64-apple-darwin/release/regexr-server \
    target/aarch64-apple-darwin/release/regexr-server
  file target/universal/regexr-server
fi

if [ "${1:-}" = "--smoke" ]; then
  "$ROOT/scripts/smoke.sh" "target/$HOST/release/regexr-server"
fi
