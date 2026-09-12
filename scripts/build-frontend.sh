#!/usr/bin/env bash
# Build the regexr frontend and sync artifacts into the Rust server's embedded static dir.
# Requires: Node 18+ (tested on Node 24), npm.
set -euo pipefail

FRONTEND_DIR="$(cd "$(dirname "$0")/../frontend" && pwd)"
STATIC_DIR="$(cd "$(dirname "$0")/../server" && pwd)/static"

cd "$FRONTEND_DIR"

if [ ! -d node_modules ]; then
  echo "==> npm ci (ignore-scripts: no native builds needed)"
  npm ci --ignore-scripts --no-audit --no-fund
fi

echo "==> gulp build (js + sass)"
npx gulp build

echo "==> gulp dev-html"
npx gulp dev-html

echo "==> sync artifacts -> $STATIC_DIR"
mkdir -p "$STATIC_DIR/assets/themes" "$STATIC_DIR/assets/icons"
cp deploy/index.html "$STATIC_DIR/index.html"
cp deploy/regexr.js "$STATIC_DIR/regexr.js"
cp deploy/regexr.css "$STATIC_DIR/regexr.css"
cp assets/themes/*.css "$STATIC_DIR/assets/themes/"
cp assets/icons/* "$STATIC_DIR/assets/icons/"

echo "==> done"
