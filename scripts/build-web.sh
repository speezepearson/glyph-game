#!/usr/bin/env bash
# Build the WASM bundle and stage it next to web/index.html.
#
# Usage:
#   scripts/build-web.sh           # release build
#   scripts/build-web.sh --debug   # faster compile, larger binary
#
# After this runs, serve `web/` with any static HTTP server, e.g.:
#   python3 -m http.server --directory web 8080
# then open http://localhost:8080/

set -euo pipefail

cd "$(dirname "$0")/.."

PROFILE=release
PROFILE_DIR=release
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE=dev
  PROFILE_DIR=debug
fi

# Ensure the WASM target is installed (cheap no-op if already present).
rustup target add wasm32-unknown-unknown >/dev/null

cargo build --profile "$PROFILE" --target wasm32-unknown-unknown

cp "target/wasm32-unknown-unknown/$PROFILE_DIR/glyph-game.wasm" \
   "web/glyph_game.wasm"

echo "Built web/glyph_game.wasm"
echo "Serve with: python3 -m http.server --directory web 8080"
