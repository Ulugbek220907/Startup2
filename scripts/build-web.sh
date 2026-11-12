#!/usr/bin/env bash
# Builds the customer app and the admin page into crates/web/dist.
#
#   scripts/build-web.sh            release build (optimised, what gets deployed)
#   scripts/build-web.sh --dev      faster debug build
#
# Needs: trunk (cargo install trunk --locked), the wasm32 target
# (rustup target add wasm32-unknown-unknown), npm and curl for the vendor step.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODE="--release"
if [ "${1:-}" = "--dev" ]; then MODE=""; fi

"$ROOT/scripts/fetch-vendor.sh"

cd "$ROOT/crates/web"
# The customer app goes first: trunk empties its dist folder, and the admin
# bundle is then added underneath it.
trunk build $MODE
trunk build $MODE --config Trunk.admin.toml

# Report the size of what a first visit downloads (budget: 600 KB gzipped).
initial=0
for f in dist/app-*.js dist/app-*_bg.wasm dist/styles-*.css dist/index.html; do
  size=$(gzip -9 -c "$f" | wc -c)
  initial=$((initial + size))
done
echo "initial payload: $((initial / 1024)) KB gzipped (budget 600 KB)"
if [ -n "$MODE" ] && [ "$initial" -gt 614400 ]; then
  echo "initial payload exceeds the 600 KB budget" >&2
  exit 1
fi
