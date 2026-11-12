# Versions in use

Checked on 2026-11-11/12. "Verified" says how the version was confirmed.

| Component | Version | Verified |
|---|---|---|
| Rust toolchain | 1.97.0 | installed toolchain; pinned in `rust-toolchain.toml` |
| axum | 0.8.9 | crates.io API; compiled and tested |
| tokio | 1.54 | crates.io API; compiled and tested |
| tower-http | 0.7.1 | crates.io API; compiled and tested |
| sqlx (+ sqlx-cli) | 0.9.0 | crates.io API; compiled; queries checked against PostgreSQL 16 |
| Leptos | 0.8.23 | crates.io API; compiled; exercised in a browser |
| teloxide | 0.17.0 | crates.io API; compiled. **Not run against Telegram** (no bot token here) |
| reqwest | 0.13.5 | crates.io API; compiled; tested against a local fake API |
| argon2 | 0.6.0 | crates.io API; compiled and tested |
| resvg / usvg / tiny-skia | 0.49 / 0.49 / 0.12 | crates.io API; golden-image tests |
| image | 0.25.10 | crates.io API; tested |
| fast_image_resize | 6.1 | crates.io API; tested via golden images |
| object_store | 0.14 | crates.io API; S3 path exercised against MinIO in the compose stack |
| MinIO (development only) | `cgr.dev/chainguard/minio:latest` (RELEASE.2026-11-10) | pulled and run in the compose stack; the tag floats, pin a digest if you need reproducibility |
| Caddy (production) | `caddy:2-alpine` | **not run here** |
| testcontainers | 0.28.0 | crates.io API; integration tests run with it |
| ttf-parser | 0.25 | crates.io API; tested |
| wasm-bindgen (+ CLI) | 0.2.130 | crates.io API; used for the build |
| trunk | 0.22.1 | crates.io API; used for the build |
| PostgreSQL | 16 | local server and `postgres:16-alpine` container |
| MediaPipe Tasks Vision | 1.2.3 | npm registry; API read from its type definitions; run in headless Chromium |
| Pose model | pose_landmarker_lite, float16, v1 | downloaded; SHA-256 pinned in `scripts/fetch-vendor.sh` |
| Capacitor (core, cli, android, ios) | 8.6.0 | npm registry; `cap add android/ios` run. **Native builds not run** (no Android SDK / Xcode here) |
| Capacitor plugins | app 8.1.2, camera 8.2.6, filesystem 8.1.5, haptics 8.0.2, push-notifications 8.1.3, share 8.0.3, splash-screen 8.0.2, status-bar 8.0.4 | npm registry |
| fastlane | 2.241 | rubygems.org API. **Lane not run** |
| Playwright (browser test) | 1.63.0 | installed; test passes locally |
| FASHN API | model `tryon-v1.6` | vendor docs read on 2026-11-11; **no live call made** |
| GitHub runners | ubuntu-24.04, macos-26 (Xcode 26.6 default) | actions/runner-images README |

## Not verified: GitHub Actions versions

github.com release pages were not reachable from the build environment, so
the action versions in `.github/workflows/` are major tags known to exist
(`actions/checkout@v4`, `actions/setup-node@v4`, `actions/setup-java@v4`,
`actions/upload-artifact@v4`, `Swatinem/rust-cache@v2`,
`softprops/action-gh-release@v2`, `docker/*@v3` / `build-push-action@v6`,
`ruby/setup-ruby@v1`, `r0adkll/upload-google-play@v1`,
`EmbarkStudios/cargo-deny-action@v2`, `appleboy/ssh-action@v1`,
`appleboy/scp-action@v1`, `taiki-e/install-action@v2`,
`dtolnay/rust-toolchain@master`). Newer majors may exist. Check each
action's releases page once, and consider pinning to commit SHAs (Dependabot
can keep them current). **None of the four workflows has run yet.**
