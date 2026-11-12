#!/usr/bin/env bash
# Downloads the on-device pose model and its runtime into the web app's
# vendor folder. They are fetched at build time instead of being committed:
# together they are about 20 MB of third-party binaries.
#
#   MediaPipe Tasks Vision 1.2.3 (Apache-2.0)  - runtime (JS + WASM)
#   pose_landmarker_lite, float16, version 1   - model (Apache-2.0)
#
# The files are self-hosted (not loaded from a CDN) so the service worker
# can cache them, the Content-Security-Policy can stay strict, and the
# mobile apps work without reaching a third-party host.
set -euo pipefail

TASKS_VISION_VERSION="1.2.3"
MODEL_URL="https://storage.googleapis.com/mediapipe-models/pose_landmarker/pose_landmarker_lite/float16/1/pose_landmarker_lite.task"
# SHA-256 of the files above; the build fails if a download differs.
MODEL_SHA256="59929e1d1ee95287735ddd833b19cf4ac46d29bc7afddbbf6753c459690d574a"
RUNTIME_WASM_SHA256="225bc06c0f51a9cf31fea0e9e0b0243e9e01fb5ef332cc1a7fa6656dbdfdaabf"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/crates/web/public/vendor/mediapipe"
mkdir -p "$DEST/wasm"

if [ -f "$DEST/.version" ] && [ "$(cat "$DEST/.version")" = "$TASKS_VISION_VERSION" ] && [ -f "$DEST/pose_landmarker_lite.task" ]; then
  echo "vendor files already present ($TASKS_VISION_VERSION)"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "fetching @mediapipe/tasks-vision@$TASKS_VISION_VERSION"
(cd "$TMP" && npm pack --silent "@mediapipe/tasks-vision@$TASKS_VISION_VERSION" >/dev/null && tar -xzf mediapipe-tasks-vision-*.tgz)
cp "$TMP/package/vision_bundle.mjs" "$DEST/vision_bundle.mjs"
cp "$TMP/package/wasm/vision_wasm_internal.js" "$TMP/package/wasm/vision_wasm_internal.wasm" "$DEST/wasm/"

echo "fetching pose model"
curl -fsSL --retry 3 -o "$DEST/pose_landmarker_lite.task" "$MODEL_URL"

check() {
  local got
  got="$(sha256sum "$1" | cut -d' ' -f1)"
  if [ "$got" != "$2" ]; then
    echo "checksum mismatch for $1: got $got, expected $2" >&2
    exit 1
  fi
}
check "$DEST/pose_landmarker_lite.task" "$MODEL_SHA256"
check "$DEST/wasm/vision_wasm_internal.wasm" "$RUNTIME_WASM_SHA256"

echo "$TASKS_VISION_VERSION" > "$DEST/.version"
echo "vendor files ready in $DEST"
