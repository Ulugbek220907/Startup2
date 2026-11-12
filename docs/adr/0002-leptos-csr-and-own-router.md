# ADR 0002: Leptos, client-side rendered, with a hand-written router

**Status:** accepted

## Context
The frontend must be Rust where practical, load fast on 4G, and run
unchanged in a browser, as a Telegram Mini App and inside Capacitor.

## Decision
- **Leptos 0.8 in CSR mode**, built with trunk. Capacitor ships static
  files, so server-side rendering could not be used in the apps anyway; one
  rendering mode everywhere avoids two code paths.
- **No `leptos_router`.** The app has seven routes; `src/router.rs` is about
  100 lines on the History API. This is a deviation from "use the
  framework's router" made for payload size and fewer moving parts.
- **Admin is a second WASM bundle** (`/admin/`), so customers never download
  it. It uses hash routing, which is also what the QR code links to.
- **No runtime code splitting.** trunk cannot split a Leptos bundle. The
  budget is met without it: the first visit downloads about 400 KB gzipped
  (WASM + JS + CSS + HTML; limit 600 KB, enforced by `scripts/build-web.sh`).
  What *is* lazy: print fonts (fetched when first used) and the pose model
  and its runtime (fetched when live try-on starts).

## Consequences
- JavaScript is limited to `shims/platform.js`, `shims/pose.js` and the
  service worker, each with a header explaining why it exists.
- Async tasks that outlive a screen must use the `try_` signal accessors;
  this bit us once (see CHANGELOG) and the browser test now fails on any
  console error.
